//! The super-pixel segmenter (0x4ab730 at engine+0x934, its segmentation
//! object [engine+0xa44] and its label image [Segmenter+0x108]) recovered
//! from the disassembly: the paths 0x488d20 takes for the presets whose
//! shared block has `is_anti_aliased` = 0.
//!
//! * 0x4aab30 seeds the label image with the identity, copies
//!   `lambda_initial` into the running lambda and derives the per-iteration
//!   rise `(lambda_final / lambda_initial) ^ (1 / (max_iterations *
//!   rise_fraction))`.
//! * 0x4aaac0 labels every 16x16 block through a quadtree (0x481c60): a
//!   block whose total variance (0x481130, over the four channels as
//!   `byte / 255` doubles) is at most `lambda_pre` is one super-pixel,
//!   otherwise its four quadrants are tried in turn down to single pixels.
//! * 0x48ae30 renumbers the labels (0x48ac00: the union-find is flattened,
//!   every 4-connected component of one label gets the next number in scan
//!   order through the flood fill 0x484100, colour indices survive the
//!   renumbering) and recomputes every region's pixel count and single
//!   precision colour sums.
//! * 0x4ab3c0 walks every pixel and its right and lower neighbours (mode 0;
//!   mode 1 takes all eight neighbours, mode 2 merges any region under
//!   `min_num_pixels`); 0x4aafd0 merges the two regions when the merge cost
//!   0x4aac30 (`|Sp|^2 / np + |Sq|^2 / nq - |Sp + Sq|^2 / (np + nq)`) is
//!   below the running lambda, and otherwise moves the boundary pixel that
//!   sits closer to the other region's mean colour.
//! * 0x4ab730 runs `max_iterations` such passes, raising lambda by the rise
//!   after each and capping it at `lambda_final`; presets 6 to 9 then
//!   register every region's rounded mean colour in the colour table
//!   (0x489de0 through 0x484c90, distinct colours in order of first use,
//!   0x482e90 building the float table) before 0x4ab710 merges the regions
//!   under `min_num_pixels` once more and 0x488c70 fills the contour
//!   records the contour builder reads.

use std::collections::BTreeMap;
use std::collections::VecDeque;

use super::{NEIGHBOUR_DX, NEIGHBOUR_DY};

/// The `Segmenter` parameters of a preset (engine+0x930..0x954 with the
/// shared block's anti-aliasing flag).
#[derive(Clone, Debug, PartialEq)]
pub struct SegmenterParams {
    pub max_iterations: i32,
    pub lambda_pre: f32,
    pub lambda_initial: f32,
    pub lambda_final: f32,
    pub min_num_pixels: i32,
    pub num_iter_btwn_renum: i32,
    pub rise_fraction: f32,
    pub size_penalty_weight: f32,
    pub size_penalty_power: f32,
    pub use_over_seg_diag_extr: bool,
    pub is_anti_aliased: bool,
    /// `SubPixelSegmenter::beach_lambda_f` (engine+0x98c).
    pub beach_lambda_f: f32,
    /// `SubPixelSegmenter::color_cluster_margin` (engine+0x990).
    pub color_cluster_margin: f32,
    /// `SubPixelSegmenter::use_connected_24_iteration` (engine+0x994).
    pub use_connected_24_iteration: bool,
    /// `SubPixelSegmenter::lambda_f` (engine+0x9a4).
    pub sub_lambda_f: f32,
    /// `SubPixelSegmentation::self_weight_eps`, `_penalty_low`,
    /// `_penalty_high` (engine+0x160, 0x164, 0x168).
    pub self_weight_eps: f32,
    pub self_weight_penalty_low: f32,
    pub self_weight_penalty_high: f32,
}

impl SegmenterParams {
    /// The parameters a preset's parameter map registers.
    pub fn from_parameters(p: &crate::Parameters) -> Result<Self, String> {
        let get = |key: &str| -> Result<f64, String> {
            p.get(key)
                .and_then(|v| v.first().copied())
                .ok_or_else(|| format!("missing parameter {key}"))
        };
        Ok(Self {
            max_iterations: get("Segmenter::max_iterations")? as i32,
            lambda_pre: get("Segmenter::lambda_pre")? as f32,
            lambda_initial: get("Segmenter::lambda_initial")? as f32,
            lambda_final: get("Segmenter::lambda_final")? as f32,
            min_num_pixels: get("Segmenter::min_num_pixels")? as i32,
            num_iter_btwn_renum: get("Segmenter::num_iter_btwn_renum")? as i32,
            rise_fraction: get("Segmenter::rise_fraction")? as f32,
            size_penalty_weight: get("Segmenter::size_penalty_weight")? as f32,
            size_penalty_power: get("Segmenter::size_penalty_power")? as f32,
            use_over_seg_diag_extr: get("Segmenter::use_over_seg_diag_extr")? != 0.0,
            is_anti_aliased: get("Shared::is_anti_aliased")? != 0.0,
            beach_lambda_f: get("SubPixelSegmenter::beach_lambda_f")? as f32,
            color_cluster_margin: get("SubPixelSegmenter::color_cluster_margin")? as f32,
            use_connected_24_iteration: get("SubPixelSegmenter::use_connected_24_iteration")?
                != 0.0,
            sub_lambda_f: get("SubPixelSegmenter::lambda_f")? as f32,
            self_weight_eps: get("SubPixelSegmentation::self_weight_eps")? as f32,
            self_weight_penalty_low: get("SubPixelSegmentation::self_weight_penalty_low")? as f32,
            self_weight_penalty_high: get("SubPixelSegmentation::self_weight_penalty_high")? as f32,
        })
    }
}

/// One region record (0x1c bytes at [seg+0x24]): pixel count, the single
/// precision sums of `byte / 255` per channel and the colour index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub count: i32,
    pub sums: [f32; 4],
    pub colour: i32,
}

impl Default for Region {
    fn default() -> Self {
        Region {
            count: 0,
            sums: [0.0; 4],
            colour: -1,
        }
    }
}

/// One contour record as 0x488c70 leaves it: pixel count (+0), colour index
/// (+0x10) and colour bytes (+0x14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub pixels: i32,
    pub colour: i32,
    pub bytes: [u8; 4],
}

/// The result of 0x488d20: the label image, its union-find state, the colour
/// table and the contour records.
#[derive(Clone, Debug, PartialEq)]
pub struct Segmentation {
    pub width: usize,
    pub height: usize,
    pub labels: Vec<i32>,
    pub label_count: i32,
    pub max_label: i32,
    pub parents: Vec<i32>,
    pub colours: Vec<[f32; 4]>,
    pub records: Vec<Record>,
}

pub(super) const INV255_F32: f32 = 1.0 / 255.0;
const INV255: f64 = 0.00392156862745098;

/// The label image with its union-find (the object at [Segmenter+0x108]).
#[derive(Clone)]
pub(super) struct Labels {
    pub(super) data: Vec<i32>,
    pub(super) width: usize,
    /// +0xc: live labels.
    pub(super) count: i32,
    /// +0x10 / +0x14: the parent table.
    pub(super) parent: Vec<i32>,
    /// +0x18: a union happened since the last flatten.
    pub(super) dirty: bool,
    /// +0x1c: the highest label.
    pub(super) max: i32,
}

/// The colour index of every live region when `keep` says the renumbering
/// must carry them over (0x48ac00's first block), else nothing.
pub(super) fn kept_colours(
    keep: bool,
    max: i32,
    colour: impl Fn(usize) -> i32,
) -> Option<Vec<i32>> {
    keep.then(|| (0..=max as usize).map(colour).collect())
}

impl Labels {
    /// 0x48ac00's walk: flatten, then give every 4-connected component a
    /// fresh negative label in scan order, collecting `saved[old label]` for
    /// each when `saved` is given. Returns what was collected and the count.
    pub(super) fn renumber_components(&mut self, saved: Option<&[i32]>) -> (Vec<i32>, i32) {
        self.flatten();
        let mut list = Vec::new();
        let mut count = 0;
        let mut next = -1;
        for p in 0..self.data.len() {
            let label = self.data[p];
            if label >= 0 {
                if let Some(saved) = saved {
                    list.push(saved[label as usize]);
                }
                self.flood(label, next, p);
                count += 1;
                next -= 1;
            }
        }
        self.set_max(count - 1);
        (list, count)
    }

    /// 0x47e8e0: find the root of `node` in `parent`, compressing path.
    fn find_root(parent: &mut [i32], node: usize) -> i32 {
        let mut root = parent[node];
        while parent[root as usize] >= 0 {
            root = parent[root as usize];
        }
        parent[node] = root;
        root
    }

    /// 0x47e8e0: the root label of pixel `p`, compressing the path.
    pub(super) fn find(&mut self, p: usize) -> i32 {
        let label = self.data[p];
        if self.parent[label as usize] < 0 {
            return label;
        }
        let root = Self::find_root(&mut self.parent, label as usize);
        self.data[p] = root;
        root
    }

    /// 0x47e1b0: `b` joins `a`.
    pub(super) fn union(&mut self, a: i32, b: i32) {
        self.dirty = true;
        self.parent[b as usize] = a;
        self.count -= 1;
    }

    /// 0x47fcb0: every pixel gets its root and the parents reset.
    pub(super) fn flatten(&mut self) {
        if !self.dirty {
            return;
        }
        for p in 0..self.data.len() {
            let label = self.data[p];
            if self.parent[label as usize] >= 0 {
                let root = Self::find_root(&mut self.parent, label as usize);
                self.data[p] = root;
            }
        }
        self.dirty = false;
        self.parent.fill(-1);
    }

    /// 0x481bb0: the highest label is `max`, the live count `max + 1`.
    pub(super) fn set_max(&mut self, max: i32) {
        self.max = max;
        self.count = max + 1;
        if self.parent.len() as i32 <= max {
            self.parent = vec![-1; (max + 1) as usize];
            self.dirty = false;
        }
    }

    /// 0x484100: the 4-connected component of `old` around `p` becomes
    /// `new`.
    pub(super) fn flood(&mut self, old: i32, new: i32, p: usize) {
        assert!(self.data[p] == old && old != new);
        let w = self.width;
        let mut queue = std::collections::VecDeque::from([p]);
        while let Some(start) = queue.pop_front() {
            if self.data[start] != old {
                continue;
            }
            let row = start - start % w;
            let check_neighbour = |s: usize, data: &mut [i32], q: &mut VecDeque<usize>| {
                data[s] = new;
                if s >= w && data[s - w] == old {
                    q.push_back(s - w);
                }
                if s + w < data.len() && data[s + w] == old {
                    q.push_back(s + w);
                }
            };
            let mut s = start;
            loop {
                check_neighbour(s, &mut self.data, &mut queue);
                if s == row || self.data[s - 1] != old {
                    break;
                }
                s -= 1;
            }
            let mut s = start + 1;
            while s < row + w && self.data[s] == old {
                check_neighbour(s, &mut self.data, &mut queue);
                s += 1;
            }
        }
    }
}

/// The segmentation object [engine+0xa44]: the regions, the renumbering
/// state and the colour table (engine+0x70).
#[derive(Default)]
pub(super) struct Seg {
    pub(super) regions: Vec<Region>,
    /// +0x20: 0x48ac00 ran before.
    pub(super) renumbered: bool,
    /// The colour table's map (packed BGRA bytes to index) and float table.
    pub(super) colour_map: BTreeMap<u32, i32>,
    pub(super) colours: Vec<[f32; 4]>,
}

impl Seg {
    /// 0x489450: the mean colour as `sum / count` in single precision.
    fn mean(&self, i: usize) -> [f32; 4] {
        let r = &self.regions[i];
        let scale = 1.0f32 / r.count as f32;
        [
            r.sums[0] * scale,
            r.sums[1] * scale,
            r.sums[2] * scale,
            r.sums[3] * scale,
        ]
    }

    /// 0x489730: the colour floats of region `i`, from the table when it
    /// has an index.
    fn colour_floats(&self, i: usize) -> [f32; 4] {
        let c = self.regions[i].colour;
        if c < 0 {
            self.mean(i)
        } else {
            self.colours[c as usize]
        }
    }

    /// 0x484c90: the index of a colour, a new one when it is unseen.
    pub(super) fn colour_index(&mut self, bytes: [u8; 4]) -> i32 {
        let key = u32::from_le_bytes(bytes);
        match self.colour_map.get(&key) {
            Some(&index) if index != -1 => index,
            _ => {
                let index = self.colour_map.len() as i32;
                self.colour_map.insert(key, index);
                index
            }
        }
    }

    /// 0x482e90: the float table from the map.
    pub(super) fn build_colour_table(&mut self) {
        self.colours = vec![[0.0; 4]; self.colour_map.len()];
        for (&key, &index) in &self.colour_map {
            let b = key.to_le_bytes();
            self.colours[index as usize] = [
                b[0] as f32 * INV255_F32,
                b[1] as f32 * INV255_F32,
                b[2] as f32 * INV255_F32,
                b[3] as f32 * INV255_F32,
            ];
        }
    }
}

/// 0x469480 / 0x489de0: colour floats to bytes, `trunc(f * 255 + 0.5)`.
pub(super) fn colour_bytes(floats: [f32; 4]) -> [u8; 4] {
    let mut out = [0u8; 4];
    for (o, f) in out.iter_mut().zip(floats) {
        *o = ((f as f64) * 255.0 + 0.5) as i32 as u8;
    }
    out
}

/// The super-pixel segmenter (engine+0x934) with its image, label image and
/// segmentation object.
pub(super) struct SuperPixels<'a> {
    pub(super) image: &'a [u8],
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) params: SegmenterParams,
    /// +0x10, zeroed while the over-segmentation pass runs.
    pub(super) min_num_pixels: i32,
    /// +0x40: the running lambda.
    pub(super) lambda: f32,
    /// +0x50: the per-iteration rise.
    pub(super) rise: f32,
    pub(super) labels: Labels,
    pub(super) seg: Seg,
    /// The sub-pixel segmenter (engine+0x988) and its segmentation object.
    pub(super) sub: super::sub_pixels::SubSeg,
}

/// The 2-neighbourhood 0x4ab3c0 walks in modes 0 and 2: right and down.
const TWO_DX: [usize; 2] = [1, 0];
const TWO_DY: [usize; 2] = [0, 1];

impl<'a> SuperPixels<'a> {
    pub(super) fn pixel(&self, p: usize) -> [u8; 4] {
        let b = &self.image[p * 4..p * 4 + 4];
        [b[0], b[1], b[2], b[3]]
    }

    pub(super) fn pixel_floats(&self, p: usize) -> [f32; 4] {
        let b = self.pixel(p);
        [
            b[0] as f32 * INV255_F32,
            b[1] as f32 * INV255_F32,
            b[2] as f32 * INV255_F32,
            b[3] as f32 * INV255_F32,
        ]
    }

    /// 0x4aab30: identity labels, the running lambda and the rise.
    pub(super) fn setup(&mut self) {
        for (p, l) in self.labels.data.iter_mut().enumerate() {
            *l = p as i32;
        }
        self.labels.dirty = false;
        self.labels.parent.fill(-1);
        self.seg.renumbered = false;
        self.seg.regions.clear();
        self.lambda = self.params.lambda_initial;
        let ratio = self.params.lambda_final as f64 / self.params.lambda_initial as f64;
        let steps = self.params.max_iterations as f64 * self.params.rise_fraction as f64;
        self.rise = (ratio.ln() / steps).exp() as f32;
    }

    /// 0x47fd20: every pixel of the block gets the next label.
    fn label_block(&mut self, x: i32, y: i32, size: i32, counter: &mut i32) {
        for yy in y..y + size {
            for xx in x..x + size {
                if xx >= 0 && (xx as usize) < self.width && yy >= 0 && (yy as usize) < self.height {
                    self.labels.data[yy as usize * self.width + xx as usize] = *counter;
                }
            }
        }
        *counter += 1;
    }

    /// 0x481130: the total variance of a block from its channel sums, sums
    /// of squares and pixel count.
    fn variance(sums: &[f64; 4], squares: &[f64; 4], n: i32) -> f64 {
        if n == 0 {
            return 0.0;
        }
        let total_sq = ((squares[0] + squares[1]) + squares[2]) + squares[3];
        let sumsq =
            ((sums[0] * sums[0] + sums[1] * sums[1]) + sums[2] * sums[2]) + sums[3] * sums[3];
        (total_sq - sumsq / n as f64) / n as f64
    }

    /// 0x481c60: the quadtree over one block; returns the channel sums, the
    /// sums of squares, the pixel count and the variance, labelling the
    /// block (when `top`) or its accepted quadrants.
    fn block(
        &mut self,
        x: i32,
        y: i32,
        size: i32,
        top: bool,
        counter: &mut i32,
    ) -> ([f64; 4], [f64; 4], i32, f64) {
        if x < 0 || x as usize >= self.width || y < 0 || y as usize >= self.height {
            return ([0.0; 4], [0.0; 4], 0, 0.0);
        }
        if size == 1 {
            let b = self.pixel(y as usize * self.width + x as usize);
            let mut sums = [0.0; 4];
            let mut squares = [0.0; 4];
            for ch in 0..4 {
                sums[ch] = b[ch] as f64 * INV255;
                squares[ch] = sums[ch] * sums[ch];
            }
            return (sums, squares, 1, 0.0);
        }
        let half = size / 2;
        let quadrants = [(x, y), (x + half, y), (x, y + half), (x + half, y + half)];
        let mut sums = [0.0; 4];
        let mut squares = [0.0; 4];
        let mut n = 0;
        let mut measures = [0.0; 4];
        for (k, &(qx, qy)) in quadrants.iter().enumerate() {
            let (s, q, c, m) = self.block(qx, qy, half, false, counter);
            for ch in 0..4 {
                sums[ch] += s[ch];
                squares[ch] += q[ch];
            }
            n += c;
            measures[k] = m;
        }
        let measure = Self::variance(&sums, &squares, n);
        let lambda_pre = self.params.lambda_pre as f64;
        if lambda_pre < measure {
            for (k, &(qx, qy)) in quadrants.iter().enumerate() {
                if !(lambda_pre < measures[k]) {
                    self.label_block(qx, qy, half, counter);
                }
            }
        } else if top {
            self.label_block(x, y, size, counter);
        }
        (sums, squares, n, measure)
    }

    /// 0x4aaac0: the initial super-pixels over every 16x16 block.
    fn init(&mut self) {
        let mut counter = (self.width * self.height) as i32;
        for y in (0..self.height as i32).step_by(16) {
            for x in (0..self.width as i32).step_by(16) {
                self.block(x, y, 16, true, &mut counter);
            }
        }
    }

    /// 0x48ac00: compact the labels to the 4-connected components in scan
    /// order, keeping colour indices.
    fn renumber(&mut self) {
        let keep = self.seg.renumbered && self.seg.regions[0].colour >= 0;
        let saved = kept_colours(keep, self.labels.max, |i| self.seg.regions[i].colour);
        let (list, count) = self.labels.renumber_components(saved.as_deref());
        if (self.seg.regions.len() as i32) < count {
            if keep {
                self.seg.regions.resize(count as usize, Region::default());
            } else {
                self.seg.regions = vec![Region::default(); count as usize];
            }
        }
        self.seg.renumbered = true;
        if keep {
            for (i, &colour) in list.iter().enumerate() {
                self.seg.regions[i].colour = colour;
            }
        }
        for l in self.labels.data.iter_mut() {
            *l = -1 - *l;
        }
    }

    /// 0x48ae30: renumber, then recompute every region's count and sums.
    pub(super) fn stats(&mut self) {
        self.renumber();
        for i in 0..=self.labels.max as usize {
            self.seg.regions[i].count = 0;
            self.seg.regions[i].sums = [0.0; 4];
        }
        for p in 0..self.labels.data.len() {
            let root = self.labels.find(p) as usize;
            let f = self.pixel_floats(p);
            let r = &mut self.seg.regions[root];
            r.count += 1;
            for (s, &v) in r.sums.iter_mut().zip(f.iter()) {
                *s += v;
            }
        }
    }

    /// 0x4aac30: the cost of merging two regions.
    fn cost(&self, lp: i32, lq: i32) -> f64 {
        let rp = self.seg.regions[lp as usize];
        let rq = self.seg.regions[lq as usize];
        let sq = |s: [f32; 4]| ((s[0] * s[0] + s[1] * s[1]) + s[2] * s[2]) + s[3] * s[3];
        let sqp = sq(rp.sums);
        let sqq = sq(rq.sums);
        let mut c = [0.0f32; 4];
        for (cv, (&q, &p)) in c.iter_mut().zip(rq.sums.iter().zip(rp.sums.iter())) {
            *cv = q + p;
        }
        let sqc = sq(c);
        let np = rp.count as f32;
        let nq = rq.count as f32;
        let ns = (rp.count + rq.count) as f32;
        let cost = (sqp / np + sqq / nq) - sqc / ns;
        if self.params.size_penalty_weight > 0.0 {
            let power = self.params.size_penalty_power as f64;
            let penalty = (1.0 / (rq.count as f64).powf(power)
                + 1.0 / (rp.count as f64).powf(power))
                - 1.0 / ((rp.count + rq.count) as f64).powf(power);
            cost as f64 - self.params.size_penalty_weight as f64 * penalty
        } else {
            cost as f64
        }
    }

    /// 0x4aab90: region `lq` joins `lp`.
    fn merge(&mut self, lp: i32, lq: i32) {
        self.labels.union(lp, lq);
        let rq = self.seg.regions[lq as usize];
        let rp = &mut self.seg.regions[lp as usize];
        rp.count += rq.count;
        for ch in 0..4 {
            rp.sums[ch] += rq.sums[ch];
        }
        let rq = &mut self.seg.regions[lq as usize];
        rq.count = 0;
        rq.sums = [0.0; 4];
    }

    /// 0x4aaea0: pixel `p` leaves region `from` for region `to`.
    fn move_pixel(&mut self, p: usize, from: i32, to: i32) {
        self.labels.data[p] = to;
        let f = self.pixel_floats(p);
        let rt = &mut self.seg.regions[to as usize];
        rt.count += 1;
        for (s, &v) in rt.sums.iter_mut().zip(f.iter()) {
            *s += v;
        }
        let rf = &mut self.seg.regions[from as usize];
        rf.count -= 1;
        for (s, &v) in rf.sums.iter_mut().zip(f.iter()) {
            *s -= v;
        }
        if rf.count == 0 {
            self.labels.count -= 1;
        }
    }

    /// The region's mean colour on the 0..255 scale, and the single
    /// precision distance of a pixel to it (0x4aafd0's boundary test).
    fn mean255(&self, l: i32) -> [f32; 4] {
        let r = self.seg.regions[l as usize];
        let scale = 255.0f32 / r.count as f32;
        [
            r.sums[0] * scale,
            r.sums[1] * scale,
            r.sums[2] * scale,
            r.sums[3] * scale,
        ]
    }

    fn distance(&self, p: usize, mean: [f32; 4]) -> f32 {
        let b = self.pixel(p);
        let d = |ch: usize| b[ch] as f32 - mean[ch];
        ((d(0) * d(0) + d(1) * d(1)) + d(2) * d(2)) + d(3) * d(3)
    }

    fn merge_if_small(&mut self, lp: i32, lq: i32) -> bool {
        let np = self.seg.regions[lp as usize].count;
        let nq = self.seg.regions[lq as usize].count;
        if np < self.min_num_pixels || nq < self.min_num_pixels {
            self.merge(lp, lq);
            true
        } else {
            false
        }
    }

    /// 0x4aafd0: merge the regions of neighbours `p` and `q` when the cost
    /// is below lambda (or, at the final lambda, when one is under
    /// `min_num_pixels`), otherwise move whichever pixel sits closer to the
    /// other region's mean. Returns 0 when nothing changed.
    fn try_merge(&mut self, p: usize, q: usize, lp: i32, lq: i32) -> i32 {
        let cost = self.cost(lp, lq);
        if (self.lambda as f64) > cost {
            self.merge(lp, lq);
            return 1;
        }
        if self.lambda == self.params.lambda_final && self.merge_if_small(lp, lq) {
            return 1;
        }
        let mean_q = self.mean255(lq);
        let dpq = self.distance(p, mean_q);
        let mean_p = self.mean255(lp);
        let dpp = self.distance(p, mean_p);
        if dpp > dpq {
            self.move_pixel(p, lp, lq);
            return 2;
        }
        let dqp = self.distance(q, mean_p);
        let dqq = self.distance(q, mean_q);
        if dqq > dqp {
            self.move_pixel(q, lq, lp);
            return 3;
        }
        0
    }

    /// 0x4ab3c0: one pass over every pixel and its neighbours.
    fn iterate(&mut self, mode: i32) {
        let (w, h) = (self.width, self.height);
        for y in 0..h {
            for x in 0..w {
                let p = y * w + x;
                let mut lp = self.labels.find(p);
                match mode {
                    0 => {
                        for k in 0..2 {
                            if x + TWO_DX[k] < w && y + TWO_DY[k] < h {
                                let q = p + TWO_DX[k] + TWO_DY[k] * w;
                                let lq = self.labels.find(q);
                                if lp != lq && self.try_merge(p, q, lp, lq) != 0 {
                                    lp = self.labels.find(p);
                                }
                            }
                        }
                    }
                    1 => {
                        for k in 0..8 {
                            let nx = x as i32 + NEIGHBOUR_DX[k];
                            let ny = y as i32 + NEIGHBOUR_DY[k];
                            if nx < w as i32 && ny < h as i32 && nx >= 0 && ny >= 0 {
                                let q = ny as usize * w + nx as usize;
                                let lq = self.labels.find(q);
                                if lp != lq && self.try_merge(p, q, lp, lq) != 0 {
                                    lp = self.labels.find(p);
                                }
                            }
                        }
                    }
                    _ => {
                        for k in 0..2 {
                            if x + TWO_DX[k] < w && y + TWO_DY[k] < h {
                                let q = p + TWO_DX[k] + TWO_DY[k] * w;
                                let lq = self.labels.find(q);
                                if lp != lq && self.merge_if_small(lp, lq) {
                                    lp = self.labels.find(p);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 0x4ab730: the whole super-pixel segmentation; `over_segment` is the
    /// diagonal-extraction flag of presets 0 to 2.
    pub(super) fn run(&mut self, over_segment: bool) {
        self.init();
        self.stats();
        let saved_min = self.min_num_pixels;
        if over_segment {
            self.min_num_pixels = 0;
        }
        for i in 0..self.params.max_iterations {
            self.iterate(0);
            if (i + 1) % self.params.num_iter_btwn_renum == 0 {
                self.stats();
            }
            self.lambda *= self.rise;
            if self.lambda > self.params.lambda_final {
                self.lambda = self.params.lambda_final;
            }
        }
        if over_segment {
            self.iterate(1);
            self.extract_diagonals();
            self.min_num_pixels = saved_min;
        }
        self.stats();
        if self.min_num_pixels > 0 {
            self.iterate(2);
            self.stats();
        }
        // 0x48abe0: the region array is trimmed to the labels in use.
        let n = (self.labels.max + 1) as usize;
        self.seg.regions.resize(n, Region::default());
    }

    /// 0x47e920: the root label at (x, y), -1 outside the image.
    pub(super) fn label_at(&mut self, x: i32, y: i32) -> i32 {
        if x < 0 || x as usize >= self.width || y < 0 || y as usize >= self.height {
            return -1;
        }
        self.labels.find(y as usize * self.width + x as usize)
    }

    /// 0x47eab0: (x, y) shares its label with (x, y + dy) and (x + dx, y)
    /// but not with (x + dx, y + dy): a corner pointing that way.
    fn corner(&mut self, x: i32, y: i32, dx: i32, dy: i32) -> bool {
        let a = self.label_at(x, y);
        let b = self.label_at(x, y + dy);
        let c = self.label_at(x + dx, y);
        let d = self.label_at(x + dx, y + dy);
        a == b && a == c && a != d
    }

    /// 0x489790: the squared colour distance of two pixels on the 0..1
    /// scale, summed in double precision.
    fn distance2(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> f64 {
        let p1 = self.pixel(y1 as usize * self.width + x1 as usize);
        let p2 = self.pixel(y2 as usize * self.width + x2 as usize);
        let d = |ch: usize| ((p1[ch] as f32 - p2[ch] as f32) * INV255_F32) as f64;
        ((d(0) * d(0) + d(1) * d(1)) + d(2) * d(2)) + d(3) * d(3)
    }

    /// 0x489870: the squared second difference of three pixels along a
    /// line, single precision, zero when an end lies outside the image.
    fn second_difference(&self, x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32) -> f32 {
        let inside = |x: i32, y: i32| {
            x >= 0 && (x as usize) < self.width && y >= 0 && (y as usize) < self.height
        };
        if !inside(x0, y0) || !inside(x2, y2) {
            return 0.0;
        }
        let p0 = self.pixel(y0 as usize * self.width + x0 as usize);
        let p1 = self.pixel(y1 as usize * self.width + x1 as usize);
        let p2 = self.pixel(y2 as usize * self.width + x2 as usize);
        let r = |ch: usize| ((p0[ch] as f32 - p1[ch] as f32 * 2.0) + p2[ch] as f32) * INV255_F32;
        ((r(0) * r(0) + r(1) * r(1)) + r(2) * r(2)) + r(3) * r(3)
    }

    /// 0x489a50: where a 2x2 block joins two regions only diagonally, one of
    /// the two other pixels is given the diagonal's label so the diagonal
    /// becomes 4-connected: the pixel that is not itself a corner, or the
    /// one closer in colour to the diagonal. When both diagonals match, the
    /// smoother direction (smaller second differences around the block)
    /// is the one extracted.
    pub(super) fn extract_diagonals(&mut self) {
        let (w, h) = (self.width as i32, self.height as i32);
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let a = self.label_at(x, y);
                let b = self.label_at(x + 1, y);
                let c = self.label_at(x, y + 1);
                let d = self.label_at(x + 1, y + 1);
                if a == c || a == b || d == c || d == b {
                    continue;
                }
                let ad = a == d;
                let cb = c == b;
                if !ad && !cb {
                    continue;
                }
                let along_a = if ad && cb {
                    let mut s1 = 0.0f32;
                    let mut s2 = 0.0f32;
                    for yy in [y + 1, y + 2] {
                        for xx in [x - 1, x] {
                            s1 = (s1 as f64
                                + self.second_difference(xx, yy - 2, xx + 1, yy - 1, xx + 2, yy)
                                    as f64) as f32;
                            s2 = (s2 as f64
                                + self.second_difference(xx, yy, xx + 1, yy - 1, xx + 2, yy - 2)
                                    as f64) as f32;
                        }
                    }
                    if s1 > s2 {
                        false
                    } else if s2 > s1 {
                        true
                    } else {
                        continue;
                    }
                } else {
                    ad
                };
                // The diagonal's label, the two candidate pixels, the two
                // diagonal pixels and the corner directions.
                let (label, p1, p2, q1, q2, dir1, dir2) = if along_a {
                    (
                        a,
                        (x + 1, y),
                        (x, y + 1),
                        (x + 1, y + 1),
                        (x, y),
                        (1, -1),
                        (-1, 1),
                    )
                } else {
                    (
                        c,
                        (x, y),
                        (x + 1, y + 1),
                        (x + 1, y),
                        (x, y + 1),
                        (-1, -1),
                        (1, 1),
                    )
                };
                let r1 = self.corner(p1.0, p1.1, dir1.0, dir1.1);
                let r2 = self.corner(p2.0, p2.1, dir2.0, dir2.1);
                let target = if r1 {
                    if r2 {
                        continue;
                    }
                    p2
                } else if r2 {
                    p1
                } else {
                    let v1 = self.distance2(q1.0, q1.1, p1.0, p1.1) as f32;
                    let v2 = self.distance2(q2.0, q2.1, p1.0, p1.1);
                    let sa = (v2 + v1 as f64) as f32;
                    let v3 = self.distance2(q1.0, q1.1, p2.0, p2.1) as f32;
                    let v4 = self.distance2(q2.0, q2.1, p2.0, p2.1);
                    let sb = v4 + v3 as f64;
                    if sb > sa as f64 {
                        p1
                    } else {
                        p2
                    }
                };
                self.labels.data[target.1 as usize * self.width + target.0 as usize] = label;
            }
        }
    }

    /// 0x4ab820 (presets 0 to 2): every region's mean colour is truncated
    /// to bytes, the distinct byte colours become the colour table, and
    /// the label image is replaced by the colour index of each pixel's
    /// region. The original also has a region under ten pixels adopt the
    /// colour of the large region (at least ten pixels, the hundred largest)
    /// minimising `count * |mean difference|^2` when that falls below its
    /// starting best score, the constant 10 the constructor 0x4aaa20 stores
    /// at engine+0x958, negated. A score is a sum of squares times a count,
    /// +0 or more (or NaN), so the adoption is never taken and the port
    /// leaves the search out.
    fn quantise_colours(&mut self) {
        let max = self.labels.max as usize;
        let bytes: Vec<[u8; 4]> = (0..=max)
            .map(|i| {
                let m = self.mean255(i as i32);
                [
                    m[0] as i32 as u8,
                    m[1] as i32 as u8,
                    m[2] as i32 as u8,
                    m[3] as i32 as u8,
                ]
            })
            .collect();
        self.seg.colour_map.clear();
        for (i, &b) in bytes.iter().enumerate().take(max + 1) {
            let index = self.seg.colour_index(b);
            self.seg.regions[i].colour = index;
        }
        self.seg.build_colour_table();
        for p in 0..self.labels.data.len() {
            let root = self.labels.find(p) as usize;
            self.labels.data[p] = self.seg.regions[root].colour;
        }
        // 0x48ab80: the labels are now the colour indices.
        let n = self.seg.colours.len() as i32;
        self.labels.set_max(n - 1);
        if (self.seg.regions.len() as i32) < n {
            if self.seg.renumbered && self.seg.regions[0].colour >= 0 {
                self.seg.regions.resize(n as usize, Region::default());
            } else {
                self.seg.regions = vec![Region::default(); n as usize];
            }
        }
        self.seg.renumbered = true;
        for i in 0..n as usize {
            self.seg.regions[i].colour = i as i32;
        }
    }

    /// 0x4ab710: the final merge of regions under `min_num_pixels`.
    pub(super) fn finish_small(&mut self) {
        if self.min_num_pixels > 0 {
            self.iterate(2);
            self.stats();
        }
    }

    /// 0x489de0: every region's rounded mean colour joins the colour table.
    pub(super) fn register_colours(&mut self) {
        assert!(
            !(self.seg.renumbered && self.seg.regions[0].colour >= 0),
            "0x489de0: colours already registered"
        );
        self.seg.colour_map.clear();
        for i in 0..=self.labels.max as usize {
            let bytes = colour_bytes(self.seg.colour_floats(i));
            let index = self.seg.colour_index(bytes);
            self.seg.regions[i].colour = index;
        }
        self.seg.build_colour_table();
    }

    /// 0x488c70: the contour records.
    pub(super) fn records(&self) -> Vec<Record> {
        (0..=self.labels.max as usize)
            .map(|i| Record {
                pixels: self.seg.regions[i].count,
                colour: self.seg.regions[i].colour,
                bytes: colour_bytes(self.seg.colour_floats(i)),
            })
            .collect()
    }
}

impl<'a> SuperPixels<'a> {
    /// The segmenter over an engine image, set up by 0x4aab30, for the
    /// stage-by-stage tests.
    #[cfg(test)]
    pub(super) fn for_test(
        image: &'a [u8],
        width: usize,
        height: usize,
        params: &SegmenterParams,
    ) -> SuperPixels<'a> {
        let mut sp = SuperPixels::new(image, width, height, params);
        sp.setup();
        sp
    }

    fn new(
        image: &'a [u8],
        width: usize,
        height: usize,
        params: &SegmenterParams,
    ) -> SuperPixels<'a> {
        let n = width * height;
        SuperPixels {
            image,
            width,
            height,
            params: params.clone(),
            min_num_pixels: params.min_num_pixels,
            lambda: 0.0,
            rise: 0.0,
            labels: Labels {
                data: vec![0; n],
                width,
                count: n as i32,
                parent: vec![-1; n],
                dirty: false,
                max: n as i32 - 1,
            },
            seg: Seg::default(),
            sub: super::sub_pixels::SubSeg::default(),
        }
    }
}

/// 0x488d20(0, 1, 0): `image` is the engine's source image (BGRA, the
/// output of [`super::preprocess`]). Both sides are at least 2 pixels, the
/// bound `recovered_pipeline::vectorize` sets: the sub-pixel edge image
/// reads the next column and row, or the previous ones at the far edge, so
/// a 1-pixel side would read pixel -1.
pub fn segment(
    image: &[u8],
    width: usize,
    height: usize,
    params: &SegmenterParams,
) -> Result<Segmentation, String> {
    if width < 2 || height < 2 {
        return Err("segment: the image needs at least 2 pixels a side".to_owned());
    }
    if image.len() != width * height * 4 {
        return Err("segment: image size mismatch".to_owned());
    }
    if params.num_iter_btwn_renum <= 0 {
        return Err("segment: num_iter_btwn_renum must be positive".to_owned());
    }
    let mut sp = SuperPixels::new(image, width, height, params);
    {
        let _span = crate::profile::span("segment.setup");
        sp.setup();
    }
    if params.is_anti_aliased {
        const STAGES: [&str; 9] = [
            "segment.aa1_run",
            "segment.aa2_sub_run",
            "segment.aa3_recolour",
            "segment.aa4_quantise",
            "segment.aa5_classify",
            "segment.aa6_refine",
            "segment.aa7_recolour",
            "segment.aa8_quantise",
            "segment.aa9_cleanup",
        ];
        for (stage, name) in (1..=9).zip(STAGES) {
            let _span = crate::profile::span(name);
            sp.aa_stage(stage)?;
        }
    } else if params.use_over_seg_diag_extr {
        let _span = crate::profile::span("segment.over_segmented");
        sp.run(true);
        sp.quantise_colours();
        sp.stats();
        sp.finish_small();
    } else {
        let _span = crate::profile::span("segment.plain");
        sp.run(false);
        sp.register_colours();
        sp.finish_small();
    }
    let records = sp.records();
    Ok(Segmentation {
        width,
        height,
        labels: sp.labels.data,
        label_count: sp.labels.count,
        max_label: sp.labels.max,
        parents: sp.labels.parent[..=sp.labels.max as usize].to_vec(),
        colours: sp.seg.colours,
        records,
    })
}
