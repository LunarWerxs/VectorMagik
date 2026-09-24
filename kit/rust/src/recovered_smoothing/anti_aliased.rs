//! The anti-aliased half of the contour smoother (presets 3, 4 and 5,
//! `Shared::is_anti_aliased`), recovered from the disassembly:
//!
//! * the node set's neighbour lists (0x4822b0 / 0x482190: an ANN kd-tree
//!   over every contour position, the 50 nearest, those within 5 px kept in
//!   search order), the per-contour orientation check 0x4844f0 (a ray from a
//!   node along 0x47f0c0's direction counted against the other edges by
//!   0x47e300) and the inversion pass 0x484630 (edges crossing their
//!   neighbours by 0x47fff0, pinch points by 0x4811a0) that marks nodes with
//!   flag 0x16 and a three-iteration anti-inversion count in `aux`;
//! * the sub-pixel placement 0x495a30 / 0x495720 from the colour model
//!   0x4a2520 (two regions: the projection of the pixel colour onto the
//!   segment between the region colours; more: a non-negative least squares
//!   with a sum-to-one constraint, 0x4a1f20 / 0x4a2040) followed by the random
//!   perturbation from the engine generator 0x468bc0;
//! * the image measurement (0x4880a0 setup, 0x488710 energy, 0x487fe0
//!   gradient): every contour edge is walked through the pixel grid
//!   (0x485e60), its start, crossings and grid-line ends registered per pixel
//!   as events with a clockwise boundary parameter (0x485d50); a pixel's
//!   events of one contour are closed along the pixel perimeter through its
//!   corners (0x4867d0, 0x486730, 0x4865b0, 0x4864d0), runs shorter than
//!   three dropped (0x4852d0), each loop's signed area weights the contour
//!   colour into the pixel's model colour with the enclosing contour taking
//!   the rest (0x4868f0), and the energy is half the squared difference to
//!   the source pixels over the whole image; the gradient (0x4870b0) is the
//!   derivative of each loop's area through the events' node positions;
//! * the after-iteration hook 0x4939a0: every tenth iteration the whole
//!   node-set pass, otherwise the pass over the contours the measurement
//!   flagged (a winding sum away from 2 pi, negative or incomplete pixel
//!   weights), and a steepest-descent restart with a re-evaluated energy
//!   when anything was marked.
//!
//! Floating point follows the original operation order; the colours and
//! event positions are single precision where the original keeps them so.

use std::collections::BTreeSet;

use super::kd::KdTree;
use super::{native_random, Smoother, SmoothingError};
use crate::recovered_colour_model::ColourModel;

/// The source image and segmentation the anti-aliased smoother reads.
#[derive(Clone, Debug, PartialEq)]
pub struct AaImage {
    pub width: i32,
    pub height: i32,
    /// BGRA bytes, row-major.
    pub pixels: Vec<u8>,
    /// The region index of every pixel (resolved labels), row-major.
    pub labels: Vec<i32>,
    /// The region colours segmentation left (engine+0x70), by region id.
    pub region_colors: Vec<[f32; 4]>,
}

/// 1/255 as the original's single-precision constant (0x8dc494).
const INV255: f32 = 0.003921568859368563;
const TWO_PI: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

/// One registered point of a contour in a pixel (the 32-byte records of the
/// pool at measurement+0x1c).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Event {
    /// Bits: 1 on a grid line, 2 on two (a corner), 4 an edge start, 8 a
    /// synthetic pixel corner, 0x10 an edge end, 0x80 on a horizontal line,
    /// 0x100 on a vertical line.
    code: i32,
    /// The edge index within the contour (-2 for a pixel corner).
    k: i32,
    ci: i32,
    x: f32,
    y: f32,
    /// Clockwise position on the pixel perimeter (0..4), -1 inside.
    param: f32,
    /// The contour's turning so far at registration.
    angle: f32,
}

/// The image measurement object at smoother+0x460.
pub(super) struct Measurement {
    width: usize,
    height: usize,
    /// The evaluation's events in registration order, each with its pixel
    /// (the pool at measurement+0x1c, filled front to back).
    staged: Vec<(u32, Event)>,
    /// Per pixel: how many events the evaluation registered there.
    counts: Vec<u32>,
    /// Per pixel: where its registered events start in `pool`.
    starts: Vec<u32>,
    /// The registered events grouped by pixel, each pixel's in registration
    /// order (`group`).
    pool: Vec<Event>,
    /// Per pixel: its events after `process_cell` (loops closed, short runs
    /// dropped, or emptied by `invalidate_pixel`) as a range of `processed`.
    proc_start: Vec<u32>,
    proc_len: Vec<u32>,
    processed: Vec<Event>,
    pub(super) model: Vec<[f32; 4]>,
    /// Each pixel's squared difference between `model` and the image, kept
    /// current by every writer of `model` (the setup, and the energy for the
    /// pixels it processes) so the energy's sum over the image reads it.
    pixel_sum: Vec<f64>,
    /// Each pixel's enclosing region from its last colour computation
    /// (`f4` as the energy left it), read back by the gradient.
    cell_f4: Vec<i32>,
    /// The cells that hold events since the last clearing, in the order
    /// they first received one.
    touched: Vec<usize>,
    angle: f64,
    /// +0xf8: the contours the last evaluation flagged.
    pub changed: BTreeSet<usize>,
    /// +0xf4: the enclosing region a pixel's weights left room for.
    f4: i32,
    regions: [i32; 12],
    weights: [f64; 12],
    count: usize,
}

pub(super) struct AaState<'a> {
    pub image: &'a AaImage,
    /// Every (contour, position) in order (node set +0x4c).
    pub pairs: Vec<(usize, usize)>,
    /// Per node the pair indices within 5 px, nearest first.
    pub neighbours: Vec<Vec<usize>>,
    pub meas: Measurement,
    pub(super) cm: ColourModel,
    /// smoother+0xb0: the node-set pass every this many iterations.
    pub every: i32,
    /// smoother+0x620..0x638: the two diagonal unit vectors.
    pub units: [f64; 4],
    /// The engine generator's seed (0xa21d2c).
    pub seed: i32,
}

impl Measurement {
    /// Whether the pixel holds events after the last energy evaluation.
    #[cfg(test)]
    pub(super) fn crossed(&self, cell: usize) -> bool {
        self.proc_len[cell] > 0
    }

    /// Every pixel empty (0x4880a0's fresh pool).
    fn clear_all(&mut self) {
        self.counts.fill(0);
        self.proc_len.fill(0);
        self.touched.clear();
        self.staged.clear();
        self.processed.clear();
    }

    /// Empty the pixels the last evaluation filled, ready for the next.
    fn clear_touched(&mut self) {
        for &cell in &self.touched {
            self.counts[cell] = 0;
            self.proc_len[cell] = 0;
        }
        self.touched.clear();
        self.staged.clear();
        self.processed.clear();
    }

    /// The staged events grouped by pixel into `pool`, each pixel's in the
    /// order they were registered (a stable counting sort over `touched`).
    fn group(&mut self) {
        let mut running = 0u32;
        for &cell in &self.touched {
            self.starts[cell] = running;
            running += self.counts[cell];
        }
        self.pool.clear();
        self.pool.resize(running as usize, Event::default());
        for &(cell, event) in &self.staged {
            let slot = &mut self.starts[cell as usize];
            self.pool[*slot as usize] = event;
            *slot += 1;
        }
        for &cell in &self.touched {
            self.starts[cell] -= self.counts[cell];
        }
    }

    /// 0x4867d0 then 0x4852d0 on one pixel: its registered events, closed
    /// and filtered, appended to `processed`.
    fn process(&mut self, contours: &[super::Contour], cell: usize, x: i32, y: i32) {
        let start = self.starts[cell] as usize;
        let count = self.counts[cell] as usize;
        let offset = self.processed.len();
        self.processed
            .extend_from_slice(&self.pool[start..start + count]);
        Smoother::process_cell(contours, &mut self.processed, offset, x, y);
        self.proc_start[cell] = offset as u32;
        self.proc_len[cell] = (self.processed.len() - offset) as u32;
    }
}

/// Difference between model colour and input pixel colour.
fn pixel_diff(model: [f32; 4], px: &[u8]) -> [f32; 4] {
    let mut d = [0.0f32; 4];
    for ch in 0..4 {
        d[ch] = model[ch] - ((px[ch] as f32) * INV255);
    }
    d
}

/// The energy's per-pixel term: the squared difference of `model` and the
/// source pixel, summed channel by channel in the original's order.
fn pixel_sum_of(model: [f32; 4], px: &[u8]) -> f64 {
    let d = pixel_diff(model, px);
    ((d[0] as f64 * d[0] as f64) + (d[1] as f64 * d[1] as f64))
        + (d[2] as f64 * d[2] as f64)
        + (d[3] as f64 * d[3] as f64)
}

impl<'a> AaState<'a> {
    /// Every pixel's energy term for its current model colour; called by
    /// whatever sets `model` outside the energy.
    pub(super) fn refresh_pixel_sums(&mut self) {
        let pixels = &self.image.pixels;
        for (cell, sum) in self.meas.pixel_sum.iter_mut().enumerate() {
            *sum = pixel_sum_of(self.meas.model[cell], &pixels[cell * 4..cell * 4 + 4]);
        }
    }

    pub fn new(image: &'a AaImage, every: i32, units: [f64; 4], seed: i32) -> AaState<'a> {
        let w = image.width as usize;
        let h = image.height as usize;
        AaState {
            image,
            pairs: Vec::new(),
            neighbours: Vec::new(),
            meas: Measurement {
                width: w,
                height: h,
                staged: Vec::new(),
                counts: vec![0; w * h],
                starts: vec![0; w * h],
                pool: Vec::new(),
                proc_start: vec![0; w * h],
                proc_len: vec![0; w * h],
                processed: Vec::new(),
                model: vec![[0.0; 4]; w * h],
                pixel_sum: vec![0.0; w * h],
                cell_f4: vec![-1; w * h],
                touched: Vec::new(),
                angle: 0.0,
                changed: BTreeSet::new(),
                f4: -1,
                regions: [0; 12],
                weights: [0.0; 12],
                count: 0,
            },
            cm: ColourModel::new(),
            every,
            units,
            seed,
        }
    }

    pub fn validate(image: &AaImage, contours: usize) -> Result<(), SmoothingError> {
        let w = image.width;
        let h = image.height;
        if w < 1 || h < 1 || w > 1 << 15 || h > 1 << 15 {
            return Err("Anti-aliased image has an invalid size".into());
        }
        let n = (w as usize) * (h as usize);
        if image.pixels.len() != 4 * n || image.labels.len() != n {
            return Err("Anti-aliased image data does not match its size".into());
        }
        if image
            .labels
            .iter()
            .any(|&l| l < 0 || l as usize >= contours)
        {
            return Err("Label outside the contour records".into());
        }
        Ok(())
    }
}

impl<'s> Smoother<'s> {
    pub(super) fn aa(&self) -> &AaState<'s> {
        self.aa.as_ref().expect("anti-aliased state")
    }
    fn aa_mut(&mut self) -> &mut AaState<'s> {
        self.aa.as_mut().expect("anti-aliased state")
    }

    /// 0x4880a0: the model colours start as every pixel's region colour.
    pub(super) fn measurement_setup(&mut self) {
        let contours = &*self.contours;
        let aa = self.aa.as_mut().expect("anti-aliased state");
        let labels = &aa.image.labels;
        for (i, m) in aa.meas.model.iter_mut().enumerate() {
            let c = contours[labels[i] as usize].color;
            for ch in 0..4 {
                m[ch] = (c[ch] as f32) * INV255;
            }
        }
        aa.meas.clear_all();
        aa.refresh_pixel_sums();
    }

    /// 0x482190 then 0x4822b0: the neighbour lists from the kd-tree over
    /// the current node positions.
    pub(super) fn build_neighbours(&mut self) {
        let _span = crate::profile::span("anti_aliased.build_neighbours");
        let mut pairs = Vec::new();
        let mut pts = Vec::new();
        for (ci, c) in self.contours.iter().enumerate() {
            for (k, &id) in c.nodes.iter().enumerate() {
                pairs.push((ci, k));
                let n = &self.nodes[id as usize];
                pts.push([n.x, n.y]);
            }
        }
        let tree = KdTree::new(pts);
        let count = self.nodes.len();
        let k = if count < 50 { count } else { 50 };
        let mut lists = vec![Vec::new(); count];
        for (i, list) in lists.iter_mut().enumerate() {
            if self.nodes[i].flags & 1 != 0 {
                continue;
            }
            if k > pairs.len() {
                continue;
            }
            let found = tree.nearest([self.nodes[i].x, self.nodes[i].y], k, 1e-12);
            for &(dist, idx) in &found {
                if !(25.0 <= dist) {
                    let (ci, pos) = pairs[idx];
                    if self.contours[ci].nodes[pos] as usize != i {
                        list.push(idx);
                    }
                }
            }
            self.nodes[i].flags |= 1;
        }
        let aa = self.aa_mut();
        aa.pairs = pairs;
        aa.neighbours = lists;
    }

    /// 0x4a1d60: the region id of a pixel, -1 outside.
    fn region_at(&self, x: i32, y: i32) -> i32 {
        let img = self.aa().image;
        if x < 0 || x >= img.width || y < 0 || y >= img.height {
            return -1;
        }
        let label = img.labels[(y * img.width + x) as usize];
        self.contours[label as usize].region
    }

    /// 0x4a1de0: the pixel's region and the distinct regions of its eight
    /// neighbours, at most nine.
    pub(super) fn neighbourhood(&mut self, x: i32, y: i32) -> usize {
        const DX: [i32; 8] = [1, 0, -1, 0, -1, 1, -1, 1];
        const DY: [i32; 8] = [0, 1, 0, -1, 1, -1, -1, 1];
        let first = self.region_at(x, y);
        let mut ids = [0i32; 9];
        let mut n = 0usize;
        ids[0] = first;
        if first >= 0 {
            n = 1;
        }
        for i in 0..8 {
            let id = self.region_at(x + DX[i], y + DY[i]);
            if id < 0 {
                continue;
            }
            if ids[..n].contains(&id) {
                continue;
            }
            ids[n] = id;
            n += 1;
            if n >= 9 {
                break;
            }
        }
        let cm = &mut self.aa_mut().cm;
        cm.ids = ids;
        cm.n = n;
        n
    }

    /// 0x4a2520: the colour model at a pixel (the shared
    /// `recovered_colour_model` over this image's region colours); only the
    /// region weights are used by the caller.
    pub(super) fn colour_model_evaluate(&mut self, x: i32, y: i32) {
        let _span = crate::profile::span("anti_aliased.colour_model_evaluate");
        let n = self.neighbourhood(x, y);
        if n <= 1 {
            return;
        }
        let img = self.aa().image;
        let p = ((y * img.width + x) * 4) as usize;
        let px = [
            img.pixels[p],
            img.pixels[p + 1],
            img.pixels[p + 2],
            img.pixels[p + 3],
        ];
        self.aa_mut()
            .cm
            .evaluate(px, &|id| img.region_colors[id as usize]);
    }

    /// 0x495720: shift one interior node towards the side the colour model
    /// gives more weight over the 2x2 pixels around its grid position.
    fn subpixel_placement(&mut self, i: usize) {
        let img = self.aa().image;
        let x0 = (self.nodes[i].fx as i32) - 1;
        let y0 = (self.nodes[i].fy as i32) - 1;
        let w = img.width;
        let mut block = [0i32; 4];
        for row in 0..2 {
            for col in 0..2 {
                let label = img.labels[((y0 + row) * w + x0 + col) as usize];
                block[(row * 2 + col) as usize] = self.contours[label as usize].region;
            }
        }
        let mut acc = [0.0f64; 4];
        for dy in 0..2 {
            for dx in 0..2 {
                let x = x0 + dx;
                let y = y0 + dy;
                if x < 0 || x >= img.width || y < 0 || y >= img.height {
                    continue;
                }
                self.colour_model_evaluate(x, y);
                let cm = &self.aa().cm;
                for &id in &block {
                    let mut j = 0usize;
                    let mut found = false;
                    while j < cm.n {
                        if cm.ids[j] == id {
                            found = true;
                            break;
                        }
                        j += 1;
                    }
                    if found && j > 0 {
                        acc[(dx + 2 * dy) as usize] += cm.weights[j];
                    }
                }
            }
        }
        let u = self.aa().units;
        let t1 = acc[2] - acc[1];
        let t0 = acc[0] - acc[3];
        let sx = ((t1 * u[2]) + (t0 * u[0])) * 0.5;
        let sy = ((t0 * u[1]) + (t1 * u[3])) * 0.5;
        let clamp = |s: f64| s.clamp(-0.333, 0.333);
        let sx = clamp(sx);
        let sy = clamp(sy);
        let node = &mut self.nodes[i];
        node.x += sx;
        node.y += sy;
    }

    /// 0x495a30: the sub-pixel placement and the random perturbation of
    /// every interior node.
    pub(super) fn perturb(&mut self) {
        let _span = crate::profile::span("anti_aliased.perturb");
        let start = self.canvas.border_total();
        let pr = self.params.perturbation_range;
        for i in start..self.nodes.len() {
            self.subpixel_placement(i);
            let aa = self.aa_mut();
            let r1 = native_random(&mut aa.seed);
            let r2 = native_random(&mut aa.seed);
            let twice = pr + pr;
            let sx = (r1 * twice) - pr;
            let sy = (r2 * twice) - pr;
            let node = &mut self.nodes[i];
            node.x += sx;
            node.y += sy;
        }
    }

    /// 0x485d50: register a point of edge `k` of contour `ci` in a pixel.
    #[allow(
        clippy::too_many_arguments,
        reason = "the original's calling convention: the pixel, the contour, the point, its code and its position"
    )]
    fn register(&mut self, cx: i32, cy: i32, ci: usize, k: usize, code: i32, x: f64, y: f64) {
        let meas = &mut self.aa_mut().meas;
        if cx < 0 || cx >= meas.width as i32 || cy < 0 || cy >= meas.height as i32 {
            return;
        }
        let param = if code & 1 == 0 {
            -1.0f32
        } else if code & 0x100 != 0 {
            if x != cx as f64 {
                ((y - cy as f64) + 1.0) as f32
            } else {
                (4.0 - (y - cy as f64)) as f32
            }
        } else if y != cy as f64 {
            (3.0 - (x - cx as f64)) as f32
        } else {
            (x - cx as f64) as f32
        };
        let event = Event {
            code,
            k: k as i32,
            ci: ci as i32,
            x: x as f32,
            y: y as f32,
            param,
            angle: meas.angle as f32,
        };
        let cell = cy as usize * meas.width + cx as usize;
        if meas.counts[cell] == 0 {
            meas.touched.push(cell);
        }
        meas.counts[cell] += 1;
        meas.staged.push((cell as u32, event));
    }

    /// 0x485e60: walk the edge from position `k` to `next_k` of contour
    /// `ci` through the grid.
    fn rasterize(&mut self, ci: usize, k: usize, next_k: usize) {
        let a = self.nodes[self.contours[ci].nodes[k] as usize];
        let b = self.nodes[self.contours[ci].nodes[next_k] as usize];
        let (ax, ay, bx, by) = (a.x, a.y, b.x, b.y);
        let dx = bx - ax;
        let dy = by - ay;
        let fax = ax.floor();
        let fay = ay.floor();
        let cx0 = fax as i32;
        let cy0 = fay as i32;
        if ax == fax || ay == fay {
            let sx = if 0.0 > dx { -1 } else { 0 };
            let sy = if 0.0 > dy { -1 } else { 0 };
            if ax != fax {
                self.register(cx0, cy0 + sy, ci, k, 0x85, ax, ay);
            } else if ay != fay {
                self.register(cx0 + sx, cy0, ci, k, 0x105, ax, ay);
            } else {
                self.register(cx0 + sx, cy0 + sy, ci, k, 0x187, ax, ay);
            }
        } else {
            self.register(cx0, cy0, ci, k, 4, ax, ay);
        }
        let mut cur_x = ax;
        let mut cur_y = ay;
        loop {
            let (gx, tx) = if 0.0 > dx {
                let g = cur_x.ceil() - 1.0;
                (g, (g - ax) / dx)
            } else if dx > 0.0 {
                let g = cur_x.floor() + 1.0;
                (g, (g - ax) / dx)
            } else {
                (0.0, 2.0)
            };
            let (gy, ty) = if 0.0 > dy {
                let g = cur_y.ceil() - 1.0;
                (g, (g - ay) / dy)
            } else if dy > 0.0 {
                let g = cur_y.floor() + 1.0;
                (g, (g - ay) / dy)
            } else {
                (0.0, 2.0)
            };
            if !(tx < 1.0) && ty >= 1.0 {
                break;
            }
            let (x, y, sx, sy, base) = if !(ty <= tx) {
                (gx, (tx * dy) + ay, -1, 0, 0x100)
            } else {
                ((ty * dx) + ax, gy, 0, -1, 0x80)
            };
            let ix = x.trunc() as i32;
            let iy = y.trunc() as i32;
            if ix as f64 == x && iy as f64 == y {
                self.register(ix, iy, ci, k, 0x183, x, y);
                self.register(ix - 1, iy, ci, k, 0x183, x, y);
                self.register(ix, iy - 1, ci, k, 0x183, x, y);
                self.register(ix - 1, iy - 1, ci, k, 0x183, x, y);
            } else {
                self.register(ix, iy, ci, k, base | 1, x, y);
                self.register(ix + sx, iy + sy, ci, k, base | 1, x, y);
            }
            cur_x = x;
            cur_y = y;
        }
        let fbx = bx.floor();
        let fby = by.floor();
        if bx == fbx || by == fby {
            let sx = if dx > 0.0 { -1 } else { 0 };
            let sy = if dy > 0.0 { -1 } else { 0 };
            let tbx = bx.trunc() as i32;
            let tby = by.trunc() as i32;
            if bx == fbx {
                if by == fby {
                    self.register(tbx + sx, tby + sy, ci, k, 0x197, bx, by);
                } else {
                    self.register(tbx + sx, tby, ci, k, 0x115, bx, by);
                }
            } else if by == fby {
                self.register(tbx, tby + sy, ci, k, 0x95, bx, by);
            }
        }
    }

    /// 0x4864d0: a pixel corner event after `at`, for perimeter step
    /// `step`.
    fn insert_corner(cell: &mut Vec<Event>, at: usize, x: i32, y: i32, step: i32) -> usize {
        let c = (step + 4) % 4;
        let dy = if c == 2 || c == 3 { 1 } else { 0 };
        let dx = if c == 1 || c == 2 { 1 } else { 0 };
        let event = Event {
            code: 0x18b,
            k: -2,
            ci: cell[at].ci,
            x: (x + dx) as f32,
            y: (y + dy) as f32,
            param: c as f32,
            angle: 0.0,
        };
        cell.insert(at + 1, event);
        at + 1
    }

    /// 0x4865b0: the corners between event `a` and the event `b` that
    /// follows it on the same contour, walking the perimeter the way the
    /// contour turns; returns the last event before `b`.
    fn insert_corners(cell: &mut Vec<Event>, a: usize, b: usize, x: i32, y: i32) -> usize {
        let ea = cell[a];
        let eb = cell[b];
        let mut diff = eb.angle as f64 - ea.angle as f64;
        if ea.k > eb.k {
            diff += TWO_PI;
        }
        let mut last = a;
        if !(diff < 0.0) {
            let mut stop = (eb.param as f64).ceil() as i32;
            if ea.param > eb.param {
                stop += 4;
            }
            let mut step = (ea.param as f64 + 1.0).trunc() as i32;
            while step < stop {
                last = Self::insert_corner(cell, last, x, y, step);
                step += 1;
            }
        } else {
            let mut stop = (eb.param as f64).floor() as i32;
            if eb.param > ea.param {
                stop -= 4;
            }
            let mut step = (ea.param as f64 - 5.0).trunc() as i32 + 4;
            while step > stop {
                last = Self::insert_corner(cell, last, x, y, step);
                step -= 1;
            }
        }
        last
    }

    /// 0x486730: close the gap between consecutive events `at` and `at+1`
    /// of one contour along the perimeter when both lie on it and are not
    /// the same point.
    fn close_pair(
        contours: &[super::Contour],
        events: &mut Vec<Event>,
        at: usize,
        x: i32,
        y: i32,
    ) -> usize {
        let a = events[at];
        let b = events[at + 1];
        let count = contours[a.ci as usize].nodes.len() as i32;
        let common = a.code & b.code;
        if common & 1 == 0 {
            return at;
        }
        if a.k == b.k {
            return at;
        }
        if (a.k + 1) % count == b.k && a.param == b.param {
            return at;
        }
        if common & 2 != 0 {
            if a.x == b.x {
                return at;
            }
            if a.y == b.y {
                return at;
            }
        }
        Self::insert_corners(events, at, at + 1, x, y)
    }

    /// 0x4867d0 then 0x4852d0: close every contour's events in a pixel into
    /// loops and drop the runs too short to enclose anything. The pixel's
    /// events are the tail of `events` from `offset`.
    fn process_cell(
        contours: &[super::Contour],
        events: &mut Vec<Event>,
        offset: usize,
        x: i32,
        y: i32,
    ) {
        let mut i = offset;
        loop {
            let len = events.len();
            let mut start = i;
            while start < len {
                if start + 1 < len && events[start].ci == events[start + 1].ci {
                    break;
                }
                start += 1;
            }
            if start >= len {
                break;
            }
            let mut cur = start;
            loop {
                if cur + 1 >= events.len() || events[cur].ci != events[cur + 1].ci {
                    break;
                }
                let last = Self::close_pair(contours, events, cur, x, y);
                cur = last + 1;
            }
            let last = cur;
            let count = contours[events[last].ci as usize].nodes.len() as i32;
            let first = events[start];
            let end = events[last];
            let mut next = last + 1;
            if (end.code & first.code) & 1 != 0
                && !((end.k + 1) % count == first.k && end.param == first.param)
            {
                let inserted = Self::insert_corners(events, last, start, x, y);
                next = inserted + 1;
            }
            i = next;
        }
        // 0x4852d0: runs of one or two events cannot close a loop; the
        // kept runs slide down in place.
        let mut write = offset;
        let mut at = offset;
        while at < events.len() {
            let ci = events[at].ci;
            let mut end = at + 1;
            while end < events.len() && events[end].ci == ci {
                end += 1;
            }
            if end - at >= 3 {
                if write != at {
                    events.copy_within(at..end, write);
                }
                write += end - at;
            }
            at = end;
        }
        events.truncate(write);
    }

    /// 0x4868f0: the pixel's model colour from the signed areas of its
    /// loops; an inconsistent pixel is reset to the source colour and its
    /// contours flagged.
    fn pixel_colour(&mut self, x: i32, y: i32) -> f64 {
        let cell = y as usize * self.aa().meas.width + x as usize;
        let (base, len) = {
            let meas = &mut self.aa_mut().meas;
            meas.f4 = -1;
            let len = meas.proc_len[cell] as usize;
            if len == 0 {
                return 1.0;
            }
            meas.count = 0;
            meas.model[cell] = [0.0; 4];
            (meas.proc_start[cell] as usize, len)
        };
        let mut total = 0.0f64;
        let mut large = false;
        let mut at = 0usize;
        loop {
            let (weight, first_ci, last_ci) = {
                let events = &self.aa().meas.processed[base..base + len];
                let first = events[at];
                let mut sum = 0.0f64;
                let mut cur = at;
                while cur + 1 < events.len() && events[cur + 1].ci == first.ci {
                    let a = events[cur];
                    let b = events[cur + 1];
                    let term = (b.y as f64 * a.x as f64) - (a.y as f64 * b.x as f64);
                    sum += term;
                    cur += 1;
                }
                let last = events[cur];
                let term = (first.y as f64 * last.x as f64) - (last.y as f64 * first.x as f64);
                sum += term;
                at = cur;
                (sum * 0.5, first.ci, last.ci)
            };
            let wf = weight as f32;
            total += weight;
            let c = self.contours[last_ci as usize].color;
            let big = self.contours[last_ci as usize].nodes.len() > 4;
            let meas = &mut self.aa_mut().meas;
            if meas.count == meas.regions.len() {
                // More loops cross this pixel than the record's 12 slots
                // hold (the original would write past them): the pixel is
                // taken as inconsistent, as when its weights do not add up,
                // and every loop's contour is flagged, not only the 12 the
                // slots hold (this one and the rest from event `at` on).
                for event in &meas.processed[base + at..base + len] {
                    meas.changed.insert(event.ci as usize);
                }
                return self.invalidate_pixel(cell);
            }
            for (m, &cv) in meas.model[cell].iter_mut().zip(c.iter()) {
                let wc = wf * ((cv as f32) * INV255);
                *m += wc;
            }
            meas.regions[meas.count] = first_ci;
            meas.weights[meas.count] = weight;
            if !(0.0 <= weight) && big {
                large = true;
            }
            meas.count += 1;
            at += 1;
            if at >= len {
                break;
            }
        }
        let count = self.aa().meas.count;
        if (total - 1.0).abs() <= 0.001 {
            if !large {
                return total;
            }
            return self.invalidate_pixel(cell);
        }
        if large {
            return self.invalidate_pixel(cell);
        }
        for i in 0..count {
            let r = self.aa().meas.regions[i];
            let parent = self.contours[r as usize].parent;
            if parent < 0 {
                continue;
            }
            let meas = &mut self.aa_mut().meas;
            if meas.regions[..meas.count].contains(&parent) {
                continue;
            }
            if meas.f4 < 0 {
                meas.f4 = parent;
            }
        }
        let f4 = self.aa().meas.f4;
        if f4 >= 0 && self.aa().meas.count == self.aa().meas.regions.len() {
            // No slot for the enclosing region: flagged here, as the path
            // below that gives it a slot would flag it.
            self.aa_mut().meas.changed.insert(f4 as usize);
            return self.invalidate_pixel(cell);
        }
        if f4 >= 0 {
            let c = self.contours[f4 as usize].color;
            let meas = &mut self.aa_mut().meas;
            total += 1.0;
            for (m, &cv) in meas.model[cell].iter_mut().zip(c.iter()) {
                let cc = (cv as f32) * INV255;
                *m += cc;
            }
            meas.regions[meas.count] = f4;
            meas.weights[meas.count] = 1.0;
            meas.count += 1;
        }
        for i in 0..count {
            let r = self.aa().meas.regions[i];
            let parent = self.contours[r as usize].parent;
            let found = {
                let meas = &self.aa().meas;
                (0..meas.count).find(|&j| meas.regions[j] == parent)
            };
            if let Some(j) = found {
                let wi = self.aa().meas.weights[i];
                let wf = wi as f32;
                let c = self.contours[parent as usize].color;
                let meas = &mut self.aa_mut().meas;
                for (m, &cv) in meas.model[cell].iter_mut().zip(c.iter()) {
                    let sub = ((cv as f32) * INV255) * wf;
                    *m -= sub;
                }
                meas.weights[j] -= wi;
                total -= wi;
            } else if parent < 0 && f4 >= 0 {
                let wi = self.aa().meas.weights[i];
                let wf = wi as f32;
                let c = self.contours[f4 as usize].color;
                let meas = &mut self.aa_mut().meas;
                for (m, &cv) in meas.model[cell].iter_mut().zip(c.iter()) {
                    let sub = wf * ((cv as f32) * INV255);
                    *m -= sub;
                }
                // The original decrements the slot past the last weight,
                // not the enclosing region's. Nothing reads that slot (the
                // check below stops before it and the next pixel writes
                // every slot before reading it), so the port leaves the
                // write out: at 12 loops it would land past the slots.
                total -= wi;
            }
        }
        for i in 0..self.aa().meas.count {
            if 0.0 > self.aa().meas.weights[i] {
                return self.invalidate_pixel(cell);
            }
        }
        if (total - 1.0).abs() <= 0.001 {
            return total;
        }
        self.invalidate_pixel(cell)
    }

    /// The tail of 0x4868f0 for a pixel whose weights do not add up: its
    /// contours go to the changed set, the events are dropped and the model
    /// colour becomes the source colour.
    fn invalidate_pixel(&mut self, cell: usize) -> f64 {
        let img = self.aa().image;
        let meas = &mut self.aa_mut().meas;
        for i in 0..meas.count {
            meas.changed.insert(meas.regions[i] as usize);
        }
        meas.proc_len[cell] = 0;
        let px = &img.pixels[cell * 4..cell * 4 + 4];
        for (m, &p) in meas.model[cell].iter_mut().zip(px.iter()) {
            *m = (p as f32) * INV255;
        }
        -1.0
    }

    /// 0x488710: the image measurement energy.
    pub(super) fn image_energy(&mut self) -> f64 {
        let _span = crate::profile::span("anti_aliased.image_energy");
        {
            let meas = &mut self.aa_mut().meas;
            meas.changed.clear();
            meas.clear_touched();
        }
        {
            let _raster_span = crate::profile::span("anti_aliased.image_energy.raster");
            for ci in 0..self.contours.len() {
                let n = self.contours[ci].nodes.len();
                let a0 = &self.nodes[self.contours[ci].nodes[0] as usize];
                let a1 = &self.nodes[self.contours[ci].nodes[1] as usize];
                let mut prev = (a1.y - a0.y).atan2(a1.x - a0.x);
                for k in 0..=n {
                    let next_k = (k + 1) % n;
                    if k == 0 {
                        self.aa_mut().meas.angle = 0.0;
                    } else {
                        let a = &self.nodes[self.contours[ci].nodes[k % n] as usize];
                        let b = &self.nodes[self.contours[ci].nodes[next_k] as usize];
                        let angle = (b.y - a.y).atan2(b.x - a.x);
                        let mut diff = angle - prev;
                        prev = angle;
                        if !(diff <= PI) {
                            diff -= TWO_PI;
                        } else if !(-PI <= diff) {
                            diff += TWO_PI;
                        }
                        let meas = &mut self.aa_mut().meas;
                        meas.angle += diff;
                    }
                    if k < n {
                        self.rasterize(ci, k, next_k);
                    }
                }
                let meas = &mut self.aa_mut().meas;
                if !((meas.angle - TWO_PI).abs() <= 0.001) {
                    meas.changed.insert(ci);
                }
            }
        }
        self.aa_mut().meas.group();
        let _cells_span = crate::profile::span("anti_aliased.image_energy.cells");
        let (w, h) = (self.aa().meas.width, self.aa().meas.height);
        let mut e = 0.0f64;
        for y in 0..h {
            for x in 0..w {
                let cell = y * w + x;
                if self.aa().meas.counts[cell] > 0 {
                    {
                        let contours = &*self.contours;
                        let aa = self.aa.as_mut().expect("anti-aliased state");
                        aa.meas.process(contours, cell, x as i32, y as i32);
                    }
                    self.pixel_colour(x as i32, y as i32);
                    let img = self.aa().image;
                    let meas = &mut self.aa_mut().meas;
                    meas.pixel_sum[cell] =
                        pixel_sum_of(meas.model[cell], &img.pixels[cell * 4..cell * 4 + 4]);
                    meas.cell_f4[cell] = meas.f4;
                }
                e += self.aa().meas.pixel_sum[cell];
            }
        }
        e * 0.5
    }

    /// The gradient contribution of one event `item` of a loop, with `other`
    /// the next event of the loop (0x4870b0's two passes).
    fn event_gradient(&mut self, item: Event, other: Event, weight: f64, second: bool) {
        if item.k < 0 {
            return;
        }
        let contour = &self.contours[item.ci as usize];
        let count = contour.nodes.len() as i32;
        let mut n0 = item.k;
        if item.code & 0x10 != 0 {
            n0 = (n0 + 1) % count;
        }
        let n1 = (n0 + 1) % count;
        let id0 = contour.nodes[n0 as usize] as usize;
        let id1 = contour.nodes[n1 as usize] as usize;
        if item.code & 1 == 0 {
            let (gx, gy) = if !second {
                ((other.y as f64) * weight, (-(other.x as f64)) * weight)
            } else {
                ((-(other.y as f64)) * weight, (other.x as f64) * weight)
            };
            self.grad[id0][0] += gx;
            self.grad[id0][1] += gy;
            return;
        }
        let p0 = [self.nodes[id0].x, self.nodes[id0].y];
        let p1 = [self.nodes[id1].x, self.nodes[id1].y];
        let horizontal = item.code & 0x80 != 0;
        let along_y = if !horizontal && p0[0] != p1[0] {
            true
        } else {
            p0[1] == p1[1]
        };
        let (val, axis) = if along_y {
            if !second {
                (-(other.x as f64), 1usize)
            } else {
                (other.x as f64, 1usize)
            }
        } else if !second {
            (other.y as f64, 0usize)
        } else {
            (-(other.y as f64), 0usize)
        };
        let oth = 1 - axis;
        let d_other = p0[oth] - p1[oth];
        let e = if oth == 0 {
            item.x as f64
        } else {
            item.y as f64
        };
        let t = (e - p1[oth]) / d_other;
        let slope = (p0[axis] - p1[axis]) / d_other;
        let mut w0 = [0.0f64; 2];
        let mut w1 = [0.0f64; 2];
        w0[axis] = t;
        w0[oth] = -(slope * t);
        let one_minus = 1.0 - t;
        w1[axis] = one_minus;
        w1[oth] = -(one_minus * slope);
        self.grad[id0][0] += (w0[0] * val) * weight;
        self.grad[id0][1] += (w0[1] * val) * weight;
        self.grad[id1][0] += (w1[0] * val) * weight;
        self.grad[id1][1] += (w1[1] * val) * weight;
    }

    /// 0x4870b0: the gradient of one pixel's term. The original recomputes
    /// the pixel's colour here (0x4868f0 again); that computation depends
    /// only on the pixel's events and the contours' colours, both untouched
    /// since the energy computed them, so the colour and the enclosing
    /// region the energy kept are read back instead.
    fn pixel_gradient(&mut self, x: i32, y: i32) {
        let cell = y as usize * self.aa().meas.width + x as usize;
        let (diff, f4, base, len) = {
            let aa = self.aa();
            let px = &aa.image.pixels[cell * 4..cell * 4 + 4];
            (
                pixel_diff(aa.meas.model[cell], px),
                aa.meas.cell_f4[cell],
                aa.meas.proc_start[cell] as usize,
                aa.meas.proc_len[cell] as usize,
            )
        };
        if len == 0 {
            return;
        }
        let mut prev_ci = -1i32;
        let mut weight = 0.0f64;
        let mut first = self.aa().meas.processed[base];
        for i in 0..len {
            let cur = self.aa().meas.processed[base + i];
            if cur.ci != prev_ci {
                first = cur;
                let contour = &self.contours[cur.ci as usize];
                let mut c = [0.0f32; 4];
                for (cv, &col) in c.iter_mut().zip(contour.color.iter()) {
                    *cv = (col as f32) * INV255;
                }
                if f4 >= 0 && contour.parent >= 0 {
                    let pc = self.contours[contour.parent as usize].color;
                    for ch in 0..4 {
                        c[ch] -= (pc[ch] as f32) * INV255;
                    }
                }
                prev_ci = cur.ci;
                weight = (((diff[0] as f64 * c[0] as f64) + (diff[1] as f64 * c[1] as f64))
                    + (diff[2] as f64 * c[2] as f64)
                    + (diff[3] as f64 * c[3] as f64))
                    * 0.5;
            }
            // The loop closes: the last event of a contour's run pairs with
            // the run's first event.
            let partner = if i + 1 < len && self.aa().meas.processed[base + i + 1].ci == cur.ci {
                self.aa().meas.processed[base + i + 1]
            } else {
                first
            };
            self.event_gradient(cur, partner, weight, false);
            self.event_gradient(partner, cur, weight, true);
        }
    }

    /// 0x487fe0: the image measurement gradient into `grad`.
    pub(super) fn image_gradient(&mut self) {
        let _span = crate::profile::span("anti_aliased.image_gradient");
        let (w, h) = (self.aa().meas.width, self.aa().meas.height);
        for y in 0..h {
            for x in 0..w {
                if self.aa().meas.proc_len[y * w + x] > 0 {
                    self.pixel_gradient(x as i32, y as i32);
                }
            }
        }
    }

    /// 0x47f0c0: the direction at position `k` of contour `ci` the
    /// orientation ray leaves along (the bisector of the two edges, turned to
    /// one side, or the perpendicular when they are nearly opposite).
    fn ray_direction(&self, ci: usize, k: usize) -> [f64; 2] {
        let c = &self.contours[ci];
        let n = c.nodes.len();
        let p = &self.nodes[c.nodes[k] as usize];
        let prev = &self.nodes[c.nodes[(n + k - 1) % n] as usize];
        let next = &self.nodes[c.nodes[(k + 1) % n] as usize];
        let mut ux = prev.x - p.x;
        let mut uy = prev.y - p.y;
        let lu = ((ux * ux) + (uy * uy)).sqrt();
        if lu > 0.0 {
            let inv = 1.0 / lu;
            ux *= inv;
            uy *= inv;
        }
        let mut vx = next.x - p.x;
        let mut vy = next.y - p.y;
        let lv = ((vx * vx) + (vy * vy)).sqrt();
        if lv > 0.0 {
            let inv = 1.0 / lv;
            vx *= inv;
            vy *= inv;
        }
        let bis_x = vx + ux;
        let bis_y = vy + uy;
        let px = uy - vy;
        let py = vx - ux;
        let a = (px * px) + (py * py);
        let b = (bis_x * bis_x) + (bis_y * bis_y);
        let (mut x, mut y) = (px, py);
        if b > a {
            let cross = (vy * ux) - (uy * vx);
            if cross > 0.0 {
                x = -bis_x;
                y = -bis_y;
            } else {
                x = bis_x;
                y = bis_y;
            }
        }
        let len = ((x * x) + (y * y)).sqrt();
        if len > 0.0 {
            let inv = 1.0 / len;
            [x * inv, y * inv]
        } else {
            [x, y]
        }
    }

    /// 0x4844f0: the ray from position `k` counted against the contour's
    /// other edges; `2 - crossings` is the contour's orientation word.
    fn orientation(&mut self, ci: usize, k: usize) -> i32 {
        let dir = self.ray_direction(ci, k);
        let c = &self.contours[ci];
        let n = c.nodes.len();
        let p = self.nodes[c.nodes[k] as usize];
        self.contours[ci].dir = dir;
        let mut count = 0i32;
        let c = &self.contours[ci];
        if n - 1 > 1 {
            for i in 0..n - 2 {
                let a = self.nodes[c.nodes[(k + 1 + i) % n] as usize];
                let b = self.nodes[c.nodes[(k + 2 + i) % n] as usize];
                count += ray_crossing(&p, &dir, &a, &b);
            }
        }
        if count % 2 != 0 {
            count = 0;
        }
        2 - count
    }

    /// 0x484630: walk contour `ci` against the neighbour lists and mark the
    /// nodes of edges that cross or run inverted; returns how many nodes
    /// were newly marked.
    fn inversion_pass(&mut self, ci: usize) -> i32 {
        let _span = crate::profile::span("anti_aliased.inversion_pass");
        let parity = self.orientation(ci, 0);
        self.contours[ci].parity = parity;
        let n = self.contours[ci].nodes.len();
        for j in 0..n {
            let id = self.contours[ci].nodes[j] as usize;
            self.nodes[id].flags &= 0xf0;
        }
        let mut marked = 0i32;
        let mut running = 0i32;
        let mut since = 0i32;
        for j in 0..n {
            let id_j = self.contours[ci].nodes[j] as usize;
            let id_next = self.contours[ci].nodes[(j + 1) % n] as usize;
            let mut abs_sum = 0i32;
            let mut signed = 0i32;
            for idx in 0..self.aa().neighbours[id_j].len() {
                let pair = self.aa().neighbours[id_j][idx];
                let (ci2, k2) = self.aa().pairs[pair];
                let c2 = &self.contours[ci2];
                let n2 = c2.nodes.len();
                let a = c2.nodes[k2] as usize;
                let b = c2.nodes[(k2 + 1) % n2] as usize;
                if b == id_next && ci2 == ci {
                    let cc = self.contours[ci].nodes[(j + 2) % n] as usize;
                    let d = self.contours[ci2].nodes[(k2 + 2) % n2] as usize;
                    let r = pinch(
                        &self.nodes[id_j],
                        &self.nodes[cc],
                        &self.nodes[a],
                        &self.nodes[d],
                        &self.nodes[id_next],
                    );
                    abs_sum += r.abs();
                    signed += r;
                } else {
                    if id_j == a || id_j == b || id_next == a || id_next == b {
                        continue;
                    }
                    let r = segments_cross(
                        &self.nodes[id_j],
                        &self.nodes[id_next],
                        &self.nodes[a],
                        &self.nodes[b],
                    );
                    abs_sum += r.abs();
                    if ci == ci2 {
                        signed += r;
                    }
                }
            }
            running += signed;
            if abs_sum != 0 {
                for &id in &[id_j, id_next] {
                    let node = &mut self.nodes[id];
                    if node.flags & 2 == 0 {
                        marked += 1;
                    }
                    node.flags |= 0x16;
                    node.aux = 3;
                }
            } else if self.contours[ci].parity + running != 0 {
                let node = &mut self.nodes[id_next];
                if node.flags & 2 == 0 {
                    marked += 1;
                }
                node.flags |= 0x16;
                node.aux = 3;
                since += 1;
                if since > 20 {
                    // Wrapped like `id_next`: at the contour's last node the
                    // original reads one node id past the end of the array
                    // (the defect sweep of September 23, 2026 found it as an
                    // index panic on a dithered 1-bit logo under blended
                    // high); no reference reaches this, since it would panic.
                    let parity = self.orientation(ci, (j + 1) % n);
                    self.contours[ci].parity = parity;
                    running = 0;
                    since = i32::MIN + 1;
                }
            } else {
                self.nodes[id_next].flags |= 4;
                since = 0;
            }
        }
        marked
    }

    /// 0x47edf0: the anti-inversion counts run down by one.
    fn decay_aux(&mut self) -> i32 {
        let mut changed = 0;
        for node in self.nodes.iter_mut() {
            if node.aux > 1 {
                node.aux -= 1;
                changed += 1;
            }
        }
        changed
    }

    /// 0x4849a0: the pass over every contour.
    pub(super) fn node_set_pass(&mut self) -> i32 {
        let _span = crate::profile::span("anti_aliased.node_set_pass");
        let mut total = self.decay_aux();
        for ci in 0..self.contours.len() {
            total += self.inversion_pass(ci);
        }
        total
    }

    /// 0x485be0: the pass over the contours the measurement flagged.
    pub(super) fn changed_pass(&mut self) -> bool {
        let _span = crate::profile::span("anti_aliased.changed_pass");
        let changed: Vec<usize> = self.aa().meas.changed.iter().copied().collect();
        let mut total = 0;
        for ci in changed {
            total += self.inversion_pass(ci);
        }
        self.aa_mut().meas.changed.clear();
        total > 0
    }
}

/// 0x47e300: how the ray from `p` along `dir` meets the edge `a`-`b`: +-2
/// for a crossing by orientation, +-1 when an endpoint lies on the ray's
/// line, 0 otherwise.
fn ray_crossing(p: &super::Node, dir: &[f64; 2], a: &super::Node, b: &super::Node) -> i32 {
    let c1 = ((b.y - a.y) * dir[0]) - ((b.x - a.x) * dir[1]);
    if c1 == 0.0 {
        return 0;
    }
    let c2 = ((b.y - p.y) * dir[0]) - ((b.x - p.x) * dir[1]);
    let on_line = if 0.0001 > c2.abs() {
        true
    } else {
        let c3 = ((a.y - p.y) * dir[0]) - ((a.x - p.x) * dir[1]);
        0.0001 > c3.abs()
    };
    if on_line {
        return if c1 > 0.0 { 1 } else { -1 };
    }
    let w = ((b.x - a.x) * (p.y - a.y)) - ((b.y - a.y) * (p.x - a.x));
    let t = ((p.y - a.y) * dir[0]) - ((p.x - a.x) * dir[1]);
    if c1 > 0.0 {
        if w < 0.0 {
            return 0;
        }
        if t <= 0.0 {
            return 0;
        }
        if c1 <= t {
            return 0;
        }
        2
    } else {
        if 0.0 < w {
            return 0;
        }
        if 0.0 <= t {
            return 0;
        }
        if t <= c1 {
            return 0;
        }
        -2
    }
}

/// 0x47efe0: whether `p` lies on the segment `a`-`b` of unit direction
/// `dir` (within 1e-4 of the line and not past `b`), or coincides with an
/// end.
fn point_on_segment(p: &[f64; 2], a: &[f64; 2], b: &[f64; 2], dir: &[f64; 2]) -> bool {
    let vx = p[0] - a[0];
    let vy = p[1] - a[1];
    let cross = (vx * dir[1]) - (vy * dir[0]);
    if 0.0001 <= cross.abs() {
        return false;
    }
    let dot = (vx * dir[0]) + (vy * dir[1]);
    let lv = (vx * vx) + (vy * vy);
    let wx = p[0] - b[0];
    let wy = p[1] - b[1];
    let lw = (wx * wx) + (wy * wy);
    if !(dot < 0.0) {
        let ux = b[0] - a[0];
        let uy = b[1] - a[1];
        let len = ((ux * ux) + (uy * uy)).sqrt();
        if len >= dot {
            return true;
        }
    }
    if 1e-8 > lv {
        return true;
    }
    if 1e-8 > lw {
        return true;
    }
    false
}

/// 0x47fff0 on four points: +-1 when an endpoint touches the other
/// segment, +-2 for a proper crossing, signed by the segments' orientation,
/// 0 otherwise.
pub(super) fn cross_points(a0: &[f64; 2], a1: &[f64; 2], b0: &[f64; 2], b1: &[f64; 2]) -> i32 {
    let cross = ((a1[0] - a0[0]) * (b1[1] - b0[1])) - ((a1[1] - a0[1]) * (b1[0] - b0[0]));
    if cross == 0.0 {
        return 0;
    }
    let sign = if 0.0 > cross { -1 } else { 1 };
    let mut da = [a1[0] - a0[0], a1[1] - a0[1]];
    let la = ((da[0] * da[0]) + (da[1] * da[1])).sqrt();
    if la > 0.0 {
        let inv = 1.0 / la;
        da[0] *= inv;
        da[1] *= inv;
    }
    let mut db = [b1[0] - b0[0], b1[1] - b0[1]];
    let lb = ((db[0] * db[0]) + (db[1] * db[1])).sqrt();
    if lb > 0.0 {
        let inv = 1.0 / lb;
        db[0] *= inv;
        db[1] *= inv;
    }
    if point_on_segment(a0, b0, b1, &db)
        || point_on_segment(a1, b0, b1, &db)
        || point_on_segment(b0, a0, a1, &da)
        || point_on_segment(b1, a0, a1, &da)
    {
        return sign;
    }
    let ey = a0[1] - b0[1];
    let ex = a0[0] - b0[0];
    let t_num = ((b1[0] - b0[0]) * ey) - ((b1[1] - b0[1]) * ex);
    let u_num = ((a1[0] - a0[0]) * ey) - ((a1[1] - a0[1]) * ex);
    let (cross, t_num, u_num) = if 0.0 > cross {
        (-cross, -t_num, -u_num)
    } else {
        (cross, t_num, u_num)
    };
    if !(t_num > 0.0) {
        return 0;
    }
    if !(cross > t_num) {
        return 0;
    }
    if !(u_num > 0.0) {
        return 0;
    }
    if !(cross > u_num) {
        return 0;
    }
    sign + sign
}

/// Segments whose bounding boxes are farther apart than this in x or y give
/// `cross_points` 0: a non-zero answer needs an endpoint within 1e-4 of the
/// other segment (`point_on_segment`: under 1e-4 from its line with the
/// projection inside it, or under 1e-4 from an end) or a proper crossing,
/// and either puts the boxes within 1e-4 of each other, far inside this.
const BOX_MARGIN: f64 = 1e-3;
/// The early answer holds for coordinates that are 0 or of a magnitude in
/// this range: no product of `cross_points` comes near overflow, and two
/// different coordinates differ by far more than the square root of the
/// smallest normal double, so a segment's squared length never underflows
/// and its "unit" direction is one. NaN, infinities and anything else take
/// the full test.
const BOX_RANGE: std::ops::RangeInclusive<f64> = 1e-100..=1e6;

/// Whether 0x47fff0's crossing branch could be decided by rounding for
/// these segments (round two of the review, September 22, 2026): on nearly
/// collinear segments apart along one line, `cross`, `t_num` and `u_num` are
/// rounding noise and the full test can answer +-2 for segments that do not
/// meet, so the early 0 would differ from the original. Every difference
/// and product there is correctly rounded, so `cross` is off by at most
/// about 4 eps |A| |B| and `t_num` by 4 eps |B| |E| (E = a0 - b0); a false
/// crossing needs the ratio off by the 1e-3 box margin over a segment's
/// length, which cannot happen while |cross| exceeds 4000 eps times the
/// larger of |B|^2 (|E| + 2|A|) and |A|^2 (|E| + 2|B|). This takes 1e-12
/// (over twice that) and the 1-norm lengths (upper bounds of the lengths),
/// so it errs towards the full test.
fn rounding_decides(
    a0: &super::Node,
    a1: &super::Node,
    b0: &super::Node,
    b1: &super::Node,
) -> bool {
    let (ax, ay) = (a1.x - a0.x, a1.y - a0.y);
    let (bx, by) = (b1.x - b0.x, b1.y - b0.y);
    let cross = (ax * by) - (ay * bx);
    let (la, lb) = (ax.abs() + ay.abs(), bx.abs() + by.abs());
    let le = (a0.x - b0.x).abs() + (a0.y - b0.y).abs();
    let bound = 1e-12 * (lb * lb * (le + 2.0 * la)).max(la * la * (le + 2.0 * lb));
    !(cross.abs() > bound)
}

/// 0x47fff0 on two node segments, with an exact early answer for segments
/// that are apart (September 22, 2026): the inversion pass tests about fifty
/// neighbour edges within 5 px per edge, most of them clear of each other,
/// and the full test normalises both segments (two roots, two divisions)
/// before it gets to its usual 0.
pub(super) fn segments_cross(
    a0: &super::Node,
    a1: &super::Node,
    b0: &super::Node,
    b1: &super::Node,
) -> i32 {
    // With a NaN every comparison fails, so the boxes never read as apart.
    let apart = |a: f64, b: f64, c: f64, d: f64| {
        let (alo, ahi) = if a < b { (a, b) } else { (b, a) };
        let (blo, bhi) = if c < d { (c, d) } else { (d, c) };
        blo - ahi > BOX_MARGIN || alo - bhi > BOX_MARGIN
    };
    if apart(a0.x, a1.x, b0.x, b1.x) || apart(a0.y, a1.y, b0.y, b1.y) {
        let exact = |v: f64| v == 0.0 || BOX_RANGE.contains(&v.abs());
        if [a0.x, a0.y, a1.x, a1.y, b0.x, b0.y, b1.x, b1.y]
            .into_iter()
            .all(exact)
            && !rounding_decides(a0, a1, b0, b1)
        {
            return 0;
        }
    }
    cross_points(&[a0.x, a0.y], &[a1.x, a1.y], &[b0.x, b0.y], &[b1.x, b1.y])
}

/// 0x4811a0: whether the two passes of a contour through the pinch node
/// `o` cross, tested on the unit directions from `o`.
fn pinch(
    p0: &super::Node,
    p1: &super::Node,
    p2: &super::Node,
    p3: &super::Node,
    o: &super::Node,
) -> i32 {
    let unit = |p: &super::Node| {
        let mut x = p.x - o.x;
        let mut y = p.y - o.y;
        let len = ((y * y) + (x * x)).sqrt();
        if len > 0.0 {
            let inv = 1.0 / len;
            x *= inv;
            y *= inv;
        }
        [x, y]
    };
    cross_points(&unit(p0), &unit(p1), &unit(p2), &unit(p3))
}

#[cfg(test)]
mod slot_tests {
    use super::*;
    use crate::recovered_smoothing::{Canvas, Contour, Params, PARAM_VALUES};

    /// A pixel crossed by more loops than the record's 12 slots is taken as
    /// inconsistent, and every loop's contour goes to the changed set, not
    /// only the 12 the slots hold (round three of the Opus 5.5 review).
    #[test]
    fn a_pixel_crossed_by_more_loops_than_the_slots_flags_every_contour() {
        let loops = 14;
        let image = AaImage {
            width: 1,
            height: 1,
            pixels: vec![0; 4],
            labels: vec![0],
            region_colors: vec![[0.0; 4]; loops],
        };
        let mut contours: Vec<Contour> = (0..loops)
            .map(|_| Contour {
                parent: -1,
                ..Contour::default()
            })
            .collect();
        let params = Params::from_values(&[0.0; PARAM_VALUES]).unwrap();
        let mut s = Smoother {
            nodes: &mut [],
            contours: &mut contours,
            canvas: Canvas {
                width: 1,
                height: 1,
                border: [0; 4],
            },
            params: &params,
            phase: 0,
            near: Vec::new(),
            grad: Vec::new(),
            blind_len: 0.0,
            aa: Some(AaState::new(&image, 1, [0.0; 4], 0)),
        };
        // Each loop a small triangle inside the pixel, three events of one contour.
        let meas = &mut s.aa_mut().meas;
        for ci in 0..loops as i32 {
            for (x, y) in [(0.1f32, 0.1f32), (0.2, 0.1), (0.1, 0.2)] {
                meas.processed.push(Event {
                    ci,
                    x,
                    y,
                    ..Event::default()
                });
            }
        }
        meas.proc_start[0] = 0;
        meas.proc_len[0] = (3 * loops) as _;
        assert_eq!(s.pixel_colour(0, 0), -1.0);
        let changed: Vec<usize> = s.aa().meas.changed.iter().copied().collect();
        assert_eq!(changed, (0..loops).collect::<Vec<_>>());
        assert_eq!(s.aa().meas.proc_len[0], 0);
    }
}
