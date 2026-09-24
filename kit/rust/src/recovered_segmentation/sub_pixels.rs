//! The sub-pixel segmenter (0x4a9ac0 at engine+0x988 with its segmentation
//! object at engine+0x130, the derived records of 0xb8 bytes at +0x44)
//! that presets 3, 4 and 5 run after the super-pixel pass on the same label
//! image (engine+0x98) and colour table (engine+0x70):
//!
//! * 0x48b690 rebuilds the segmentation from the label image: the labels
//!   are renumbered by 0x48ac00 (colour indices kept), every region gets
//!   its pixel count and colour sums, the set of its boundary pixels and
//!   their outside neighbours (8 or 24 of them), and for its interior
//!   pixels the count, sums and sums of squares; a region colour is the
//!   per-channel median of at least three interior pixels, otherwise the
//!   mean of all its pixels; then 0x48a370 caches every boundary pixel's
//!   colour-model cost (0x48a240: the squared distance to the model colour
//!   of the shared `recovered_colour_model` over the region's neighbours,
//!   plus the self-weight penalties `self_weight_penalty_low * (1 - w)` and
//!   `self_weight_penalty_high * (self_weight_eps - w)` when positive);
//! * 0x4a8640 sweeps the regions and merges each into the neighbour with
//!   the lowest merge cost under `SubPixelSegmenter::lambda_f` (0x4a6bb0:
//!   the interior cost 0x489f40 of describing its interior pixels with the
//!   neighbour's colour, minus its own cost, plus the model cost of its
//!   boundary pixels re-evaluated as the neighbour's when the first part
//!   is not already above lambda); 0x48bc60 merges (union, folded stats,
//!   the boundary set carried over and pixels that became interior moved
//!   into the interior stats);
//! * 0x4a9ac0 runs those sweeps with growing size limits, the beach pass
//!   0x4a7f20 (which, the colour model's count field being -1 from its
//!   constructor, re-assigns every pixel to its own region and only
//!   refreshes the cached costs), the diagonal extraction 0x489a50 and a
//!   last sweep over regions of at most four pixels;
//! * 0x4a83a0 gives every region with fewer than three interior pixels the
//!   colour of the large region (at least three, the hundred largest) that
//!   explains it best when that beats its own cost less
//!   `color_cluster_margin`; 0x48b160 registers the region colours in the
//!   colour table, replaces the labels by colour indices and re-derives
//!   the super-pixel segmentation's regions from them.

use std::collections::BTreeSet;

use super::super_pixels::{colour_bytes, kept_colours, Region, SuperPixels, INV255_F32};
use super::{NEIGHBOUR_DX, NEIGHBOUR_DY};
use crate::recovered_colour_model::ColourModel;

/// The 24-neighbourhood tables 0x472cb0 sets at 0xa69078 / 0xa68f78.
pub(super) const DX24: [i32; 24] = [
    1, 0, -1, 0, -1, 1, -1, 1, -2, -2, -2, -2, -2, 2, 2, 2, 2, 2, -1, 0, 1, -1, 0, 1,
];
pub(super) const DY24: [i32; 24] = [
    0, 1, 0, -1, 1, -1, -1, 1, -2, -1, 0, 1, 2, -2, -1, 0, 1, 2, -2, -2, -2, 2, 2, 2,
];

/// 0x4a9ac0's sweep limit (engine+0x988, the constant 20).
const MAX_SWEEPS: i32 = 20;
const INFINITE: i32 = 0x7fff_ffff;

/// The key 0x4a8640 files a costed (region, neighbour) pair under in the
/// set at engine+0x9e4: `(region << 16) | neighbour` in 32 bits
/// (`shl esi,0x10` at 0x4a86db, `or esi,ebx` at 0x4a86e3). Past label
/// 65,535 the region's high bits fall off and the neighbour's high bits land
/// in the region's half, so two pairs can share a key and the second is
/// skipped as already costed; the port keeps the original's packing.
pub(super) fn visited_key(region: i32, neighbour: i32) -> u32 {
    ((region as u32) << 16) | neighbour as u32
}

/// Feature 39 of 0x4a8800 without its scan of every region per region: the
/// colour floats of the regions with more than ten interior pixels in a k-d
/// tree. The scan keeps the smallest `((d0^2 + d1^2) + d2^2) + d3^2` (each
/// `d` the single-precision difference, the squares in double) below 1e100.
/// The distances are +0 or more and a NaN never wins, so that minimum does
/// not depend on the order the regions are met in; a subtree is skipped only
/// when the square of its splitting plane's difference is at least the best
/// distance so far, which bounds every distance behind the plane (a rounded
/// difference grows with the coordinate, a square of a single is exact in
/// double, and a rounded sum of non-negative terms is at least each term).
/// A region with a non-finite colour never wins the scan and is left out; a
/// non-finite query keeps the scan's 1e100. The value is the scan's to the
/// bit, in about log(regions) steps where the scan took all of them.
pub(super) struct NearestColour {
    points: Vec<[f32; 4]>,
    /// The splitting axis of the node at each index (unused in leaves).
    axes: Vec<u8>,
}

impl NearestColour {
    /// Subtrees this small are scanned whole.
    const LEAF: usize = 8;

    pub(super) fn new(points: impl Iterator<Item = [f32; 4]>) -> Self {
        let mut points: Vec<[f32; 4]> =
            points.filter(|p| p.iter().all(|v| v.is_finite())).collect();
        let mut axes = vec![0u8; points.len()];
        Self::build(&mut points, &mut axes);
        Self { points, axes }
    }

    /// Splits at the median of the axis with the widest spread.
    fn build(points: &mut [[f32; 4]], axes: &mut [u8]) {
        if points.len() <= Self::LEAF {
            return;
        }
        let mut axis = 0;
        let mut widest = -1.0f32;
        for k in 0..4 {
            let (lo, hi) = points
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(p[k]), hi.max(p[k]))
                });
            if hi - lo > widest {
                widest = hi - lo;
                axis = k;
            }
        }
        let mid = points.len() / 2;
        points.select_nth_unstable_by(mid, |a, b| a[axis].total_cmp(&b[axis]));
        axes[mid] = axis as u8;
        let (left, right) = points.split_at_mut(mid);
        let (left_axes, right_axes) = axes.split_at_mut(mid);
        Self::build(left, left_axes);
        Self::build(&mut right[1..], &mut right_axes[1..]);
    }

    /// The scan's distance from region colour `o` to `r`.
    fn distance(o: &[f32; 4], r: &[f32; 4]) -> f64 {
        let d = [o[0] - r[0], o[1] - r[1], o[2] - r[2], o[3] - r[3]];
        ((d[0] as f64 * d[0] as f64 + d[1] as f64 * d[1] as f64) + d[2] as f64 * d[2] as f64)
            + d[3] as f64 * d[3] as f64
    }

    /// Feature 39 of the region coloured `r`.
    pub(super) fn nearest(&self, r: &[f32; 4]) -> f64 {
        let mut best = 1e100;
        if r.iter().all(|v| v.is_finite()) {
            Self::search(&self.points, &self.axes, r, &mut best);
        }
        best
    }

    fn search(points: &[[f32; 4]], axes: &[u8], r: &[f32; 4], best: &mut f64) {
        if points.len() <= Self::LEAF {
            for o in points {
                let dist = Self::distance(o, r);
                if *best > dist {
                    *best = dist;
                }
            }
            return;
        }
        let mid = points.len() / 2;
        let o = &points[mid];
        let dist = Self::distance(o, r);
        if *best > dist {
            *best = dist;
        }
        let axis = axes[mid] as usize;
        let plane = (o[axis] - r[axis]) as f64;
        let (left, right) = (&points[..mid], &points[mid + 1..]);
        let (left_axes, right_axes) = (&axes[..mid], &axes[mid + 1..]);
        let (near, near_axes, far, far_axes) = if r[axis] < o[axis] {
            (left, left_axes, right, right_axes)
        } else {
            (right, right_axes, left, left_axes)
        };
        Self::search(near, near_axes, r, best);
        if *best > plane * plane {
            Self::search(far, far_axes, r, best);
        }
    }
}

/// One derived region record (0xb8 bytes at [seg2+0x44], constructed by
/// 0x48beb0, reset by 0x48c070).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SubRegion {
    /// +4, +8: the base record's count and colour sums.
    pub count: i32,
    pub sums: [f32; 4],
    /// +0x18.
    pub colour: i32,
    /// +0x1c: the boundary pixels and their outside neighbours.
    pub set: BTreeSet<i32>,
    /// +0x30: interior pixels.
    pub interior: i32,
    /// +0x34: the sums of squares of every pixel.
    pub squares: [f32; 4],
    /// +0x44, +0x54: the interior pixels' sums and sums of squares.
    pub isums: [f32; 4],
    pub isquares: [f32; 4],
    /// +0x64.
    pub flags: u8,
    /// +0x68: the region colour.
    pub colour_floats: [f32; 4],
    /// +0x78..: the interior pixels' bytes per channel while the colour is
    /// computed.
    pub channels: [Vec<u8>; 4],
}

/// The in-bounds neighbours of (x, y) in the 8- or 24-neighbourhood, in the
/// original's table order (0x48a240 and its callers walk them this way).
fn neighbours(use24: bool, x: i32, y: i32, w: i32, h: i32) -> impl Iterator<Item = (i32, i32)> {
    let count = if use24 { 24 } else { 8 };
    (0..count).filter_map(move |k| {
        let (dx, dy) = if use24 {
            (DX24[k], DY24[k])
        } else {
            (NEIGHBOUR_DX[k], NEIGHBOUR_DY[k])
        };
        let (nx, ny) = (x + dx, y + dy);
        (nx >= 0 && nx < w && ny >= 0 && ny < h).then_some((nx, ny))
    })
}

impl Default for SubRegion {
    fn default() -> Self {
        SubRegion {
            count: 0,
            sums: [0.0; 4],
            colour: -1,
            set: BTreeSet::new(),
            interior: 0,
            squares: [0.0; 4],
            isums: [0.0; 4],
            isquares: [0.0; 4],
            flags: 0,
            colour_floats: [0.0; 4],
            channels: Default::default(),
        }
    }
}

impl SubRegion {
    /// The mean colour as `sum / count` in single precision.
    fn mean(&self) -> [f32; 4] {
        let inv = 1.0f32 / self.count as f32;
        [
            self.sums[0] * inv,
            self.sums[1] * inv,
            self.sums[2] * inv,
            self.sums[3] * inv,
        ]
    }

    /// 0x48c070: everything but the colour index.
    fn reset(&mut self) {
        self.count = 0;
        self.sums = [0.0; 4];
        self.set.clear();
        self.colour_floats = [0.0; 4];
        self.squares = [0.0; 4];
        self.interior = 0;
        self.isums = [0.0; 4];
        self.isquares = [0.0; 4];
        self.flags = 0;
        for c in self.channels.iter_mut() {
            c.clear();
        }
    }

    /// 0x48bf60: one pixel into the count, sums and sums of squares.
    fn accumulate(&mut self, f: [f32; 4]) {
        self.count += 1;
        for (s, &v) in self.sums.iter_mut().zip(f.iter()) {
            *s += v;
        }
        for (s, &v) in self.squares.iter_mut().zip(f.iter()) {
            *s += v * v;
        }
    }

    /// 0x48a7f0: one interior pixel.
    fn add_interior(&mut self, f: [f32; 4]) {
        self.interior += 1;
        for (s, &v) in self.isums.iter_mut().zip(f.iter()) {
            *s += v;
        }
        for (s, &v) in self.isquares.iter_mut().zip(f.iter()) {
            *s += v * v;
        }
    }
}

/// The sub-pixel segmentation object (engine+0x130) with the segmenter's
/// own state (engine+0x988).
#[derive(Default)]
pub(super) struct SubSeg {
    pub regions: Vec<SubRegion>,
    /// seg2+0x20.
    pub renumbered: bool,
    /// seg2+0x2c (engine+0xb8): the cached cost per pixel.
    pub cost: Vec<f32>,
    /// seg2+0x40: the 24-neighbourhood is in use.
    pub use24: bool,
    /// seg2+0x3c (engine+0xa58).
    pub cm: ColourModel,
    /// engine+0x9e4: the (region, neighbour) pairs a sweep has costed.
    pub visited: BTreeSet<u32>,
    /// The neighbour set of the region a sweep is looking at.
    pub scratch: Vec<i32>,
    /// engine+0x9a8: region sizes are interior counts.
    pub use_interior_size: bool,
    /// engine+0x9cc: the 42 features per region 0x4a8800 computes.
    pub features: Vec<[f64; 42]>,
    /// The sweep's neighbourhoods and colour-model costs (not the
    /// original's; see `sweep_term`).
    pub terms: TermCache,
}

/// A multiply-rotate hasher for the term cache's integer keys (SipHash
/// costs more than most lookups save).
#[derive(Default, Clone, Copy)]
struct KeyHasher(u64);

impl std::hash::Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }

    fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64);
    }

    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
}

type KeyBuild = std::hash::BuildHasherDefault<KeyHasher>;

/// The memo of the colour-model solves with three or more regions: the
/// pixel, the region count and the interned colours in neighbourhood order.
type SolveKey = [u32; 11];

/// Entries past which the solve memo starts over.
const SOLVE_MEMO_LIMIT: usize = 1 << 18;

/// Class terms one step may keep (classes x set places); a candidate of a
/// new class past it takes `sweep_term` for every pixel.
const STEP_TERMS_LIMIT: usize = 1 << 22;

/// Terms kept per pixel (see `Hood`).
const HOOD_TERMS: usize = 4;

/// The position `Hood` keys give a candidate that is not in the list.
const ABSENT: u64 = 15;

/// One pixel's entry in the term cache.
#[derive(Clone, Copy)]
struct Hood {
    pixel: u32,
    /// The pixel's own root first, then its neighbours' distinct roots in
    /// table order: `ids[off..off + n]` of the cache.
    off: u32,
    n: u8,
    /// The next of `keys` to replace.
    next: u8,
    /// The pixel is off the image's border (all eight neighbours inside).
    inner: bool,
    /// Unique in the sweep to the colour sequence of the list (a relabel
    /// to a region of another colour, or two entries becoming one, takes a
    /// new number).
    version: u32,
    /// Terms by (version, the sweeping region's place, the candidate's
    /// place or, when it is not in the list, its colour class); 0 is empty.
    keys: [u64; HOOD_TERMS],
    costs: [f32; HOOD_TERMS],
}

/// What `sweep` keeps between the colour-model costs it asks for (see
/// `sweep_term`): per pixel the distinct regions of its 8-neighbourhood
/// with the last terms computed from them, per label its colour, the
/// solves by pixel and colours, and the terms of the candidate being costed
/// and of the best one so far.
#[derive(Default)]
pub(super) struct TermCache {
    /// Per pixel: its entry in `hoods` this sweep (stale unless the entry
    /// names the pixel back).
    slot: Vec<u32>,
    hoods: Vec<Hood>,
    ids: Vec<i32>,
    /// The last version handed out this sweep.
    version: u32,
    /// Per label this sweep: its colour and interned class (`u32::MAX`
    /// until it is first asked for).
    colours: Vec<[f32; 4]>,
    classes: Vec<u32>,
    /// Colour bits to class, kept for the whole run so `solves` stays valid.
    intern: std::collections::HashMap<[u32; 4], u32, KeyBuild>,
    solves: std::collections::HashMap<SolveKey, f64, KeyBuild>,
    /// The rounded terms of the candidate being costed and of the best,
    /// after the first (class, count) of them, which are the class terms of
    /// the step; `costs` joins the best's for `sub_merge`.
    current: Vec<f32>,
    best: Vec<f32>,
    current_prefix: (usize, usize),
    best_prefix: (usize, usize),
    costs: Vec<f32>,
    /// The region whose set `step_hoods` and `step_keys` describe (-1:
    /// none): per place in the set the pixel's entry and the key of its
    /// term for a candidate of no place in its list, less the colour class
    /// (0 where the region's place is unknown).
    step_region: i32,
    step_hoods: Vec<u32>,
    step_keys: Vec<u64>,
    /// The first place of the step with no key.
    step_unknown: usize,
    /// Per colour class met this step: the class, a region of it, and the
    /// terms of a candidate of that class in no pixel's list, in set order,
    /// with their running sums from 0.
    step_classes: Vec<(u32, i32)>,
    step_terms: Vec<Vec<f32>>,
    step_sums: Vec<Vec<f64>>,
    /// Per entry of `hoods`: the candidate stamp of the pixels whose list
    /// may hold the candidate being costed.
    marks: Vec<u32>,
    stamp: u32,
    /// How often the versions started over.
    resets: u32,
    /// Per label: the last step (numbered per sweep) that met it as a
    /// neighbour.
    met: Vec<u32>,
    step: u32,
    /// How many pixels of the step's set the original's colour-model costs
    /// reached (see `touch`).
    reached: usize,
    /// Every term costed as the original does, for the tests to hold the
    /// cache against.
    pub reference: bool,
}

impl<'a> SuperPixels<'a> {
    /// The shared block's field at engine+0x298 the second tree reads: no
    /// preset sets it and the engine leaves it 0 (the oracle records it
    /// for every case). It is not `Shared::image_type_code`, which every
    /// preset registers as its own code at engine+0x21c (PARAMETERS.md) and
    /// which no stage of the port reads.
    pub(super) fn type_code(&self) -> i32 {
        0
    }
}

/// 0x4a9d10's erosion of one flagged region: step by step, the remaining
/// pixel whose closest unflagged 4-neighbour colour is nearest (the first in
/// scan order on a tie) takes that neighbour's label, until no remaining
/// pixel touches an unflagged region. The original rescans every remaining
/// pixel per step, k^2/2 neighbour checks for a region of k pixels (seconds
/// on a long thin line of a large picture). A pixel's distance changes only
/// when one of its 4-neighbours is relabelled, and only downward (every
/// relabel is to an unflagged label), so a heap keyed (distance, pixel) with
/// the neighbours re-keyed after each step pops the same pixel with the same
/// label every step (`erosion_heap_equals_the_scan`; distances are sums of
/// squares, non-negative and finite, so their bits order like their values).
/// `pixels` is in scan order; `closest` is `closest_unflagged_neighbour`.
pub(super) fn erode(
    labels: &mut [i32],
    width: i32,
    height: i32,
    pixels: &[i32],
    closest: impl Fn(&[i32], i32) -> Option<(f32, i32)>,
) {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashMap};
    let mut key: HashMap<i32, Option<u32>> = HashMap::with_capacity(pixels.len());
    let mut heap = BinaryHeap::new();
    for &q in pixels {
        let bits = closest(labels, q).map(|(dist, _)| dist.to_bits());
        if let Some(bits) = bits {
            heap.push(Reverse((bits, q)));
        }
        key.insert(q, bits);
    }
    while let Some(Reverse((bits, q))) = heap.pop() {
        if key.get(&q) != Some(&Some(bits)) {
            continue;
        }
        let (_, label) = closest(labels, q).expect("a keyed pixel touches an unflagged region");
        labels[q as usize] = label;
        key.remove(&q);
        let (x, y) = (q % width, q / width);
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            if nx < 0 || nx >= width || ny < 0 || ny >= height {
                continue;
            }
            let r = ny * width + nx;
            if let Some(&old) = key.get(&r) {
                let bits = closest(labels, r).map(|(dist, _)| dist.to_bits());
                if bits != old {
                    if let Some(bits) = bits {
                        heap.push(Reverse((bits, r)));
                    }
                    key.insert(r, bits);
                }
            }
        }
    }
}

/// The original's loop, kept as the reference `erode` must equal.
#[cfg(test)]
pub(super) fn erode_by_scan(
    labels: &mut [i32],
    pixels: &[i32],
    closest: impl Fn(&[i32], i32) -> Option<(f32, i32)>,
) {
    let mut remaining = pixels.to_vec();
    while !remaining.is_empty() {
        let mut best = 0.0f32;
        let mut chosen: Option<(usize, i32)> = None;
        for (k, &q) in remaining.iter().enumerate() {
            if let Some((dist, lab)) = closest(labels, q) {
                if chosen.is_none() || best > dist {
                    best = dist;
                    chosen = Some((k, lab));
                }
            }
        }
        match chosen {
            Some((k, lab)) => {
                let q = remaining.remove(k);
                labels[q as usize] = lab;
            }
            None => break,
        }
    }
}

/// One stage's state for the oracle comparison.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub struct AaSnapshot {
    pub labels: Vec<i32>,
    pub max_label: i32,
    pub label_count: i32,
    pub parents: Vec<i32>,
    /// Per derived region up to the highest label: count, interior, colour
    /// index, flags, colour floats.
    pub regions: Vec<(i32, i32, i32, u8, [f32; 4])>,
}

impl<'a> SuperPixels<'a> {
    fn sub_region_colour(&self, id: i32) -> [f32; 4] {
        let r = &self.sub.regions[id as usize];
        if r.colour < 0 {
            r.colour_floats
        } else {
            self.seg.colours[r.colour as usize]
        }
    }

    /// 0x47e160: the root label of pixel `p`, with region `b` standing in
    /// as `a` when `a` is set.
    fn label_subst(&mut self, p: usize, a: i32, b: i32) -> i32 {
        let label = self.labels.find(p);
        if a >= 0 && label == b {
            a
        } else {
            label
        }
    }

    /// 0x4a1de0 over the label image: the pixel's own region and the
    /// distinct regions of its 8 or 24 neighbours, at most nine.
    fn sub_neighbourhood(&mut self, x: i32, y: i32, a: i32, b: i32) -> usize {
        let (w, h) = (self.width as i32, self.height as i32);
        let mut ids = [0i32; 9];
        let mut n = 0usize;
        let first = if x < 0 || x >= w || y < 0 || y >= h {
            -1
        } else {
            self.label_subst((y * w + x) as usize, a, b)
        };
        ids[0] = first;
        if first >= 0 {
            n = 1;
        }
        for (nx, ny) in neighbours(self.sub.use24, x, y, w, h) {
            let id = self.label_subst((ny * w + nx) as usize, a, b);
            if id < 0 || ids[..n].contains(&id) {
                continue;
            }
            ids[n] = id;
            n += 1;
            if n >= 9 {
                break;
            }
        }
        self.sub.cm.ids = ids;
        self.sub.cm.n = n;
        n
    }

    /// 0x48a240: the colour-model cost of pixel (x, y) with region `b`
    /// evaluated as region `a` (no substitution when `a` is -1).
    fn pixel_cost(&mut self, x: i32, y: i32, a: i32, b: i32) -> f64 {
        self.sub_neighbourhood(x, y, a, b);
        let px = self.pixel(y as usize * self.width + x as usize);
        let mut colours = [[0.0f32; 4]; 9];
        for (k, colour) in colours.iter_mut().enumerate().take(self.sub.cm.n) {
            *colour = self.sub_region_colour(self.sub.cm.ids[k]);
        }
        self.model_cost(px, &colours)
    }

    /// The rest of 0x48a240 once `cm.ids` and `cm.n` hold the neighbourhood
    /// and `colours` the colour of each of its regions in that order: a
    /// function of the pixel and those colours alone (the model's other
    /// fields are scratch its evaluation writes before it reads them).
    fn model_cost(&mut self, px: [u8; 4], colours: &[[f32; 4]; 9]) -> f64 {
        let ids = self.sub.cm.ids;
        let colour_of = |id: i32| -> [f32; 4] {
            let at = ids.iter().position(|&i| i == id).expect("neighbourhood id");
            colours[at]
        };
        let (out, weight) = self.sub.cm.evaluate(px, &colour_of);
        let d = |ch: usize| px[ch] as f32 * INV255_F32 - out[ch];
        let dist = ((d(0) * d(0) + d(1) * d(1)) + d(2) * d(2)) + d(3) * d(3);
        let mut penalty = 0.0f64;
        let t1 = 1.0 - weight;
        if t1 > 0.0 {
            penalty = self.params.self_weight_penalty_low as f64 * t1;
        }
        let t2 = self.params.self_weight_eps as f64 - weight;
        if t2 > 0.0 {
            penalty += self.params.self_weight_penalty_high as f64 * t2;
        }
        dist as f64 + penalty
    }

    /// `pixel_cost(q % w, q / w, lq, i) as f32` for a pixel `q` of region
    /// `i`'s set during a sweep over the 8-neighbourhood, mostly without its
    /// nine root lookups and its colour-model solve.
    ///
    /// On a busy pixel-edged picture (a blended one-pixel checker) a region
    /// merges into a neighbour with a later label, which the same sweep then
    /// visits with the grown set, so one sweep costs set x candidates terms
    /// again and again: 35 million on a 48 px checker, where only 123,000
    /// (pixel, region colours) pairs are distinct. The reuse is exact:
    ///
    /// * the term is a function of the pixel's bytes and the colours of the
    ///   neighbourhood's regions in order (`model_cost`; the colour model's
    ///   other fields are scratch), and no region's colour changes during a
    ///   sweep (`sub_merge` folds counts, sums and sets; the colour index and
    ///   floats stay), so solves are kept under the pixel and the interned
    ///   colour bits of its regions;
    /// * the neighbourhood of 0x4a1de0 is the first-met distinct list of the
    ///   substituted roots of nine pixels, which is the substitution of the
    ///   pixel's first-met distinct roots with duplicates dropped (nine
    ///   pixels never fill more than the nine slots, so the cap never cuts).
    ///   That distinct list is kept per pixel: a union only relabels the
    ///   joining region `a` as `b` (keeping the first of the two places when
    ///   both are there), and a pixel whose list holds `a` and another root is
    ///   a boundary pixel or an outside neighbour of `a`, so in `a`'s set
    ///   (sets only ever gain those; `sub_merge` drops only pixels that turned
    ///   interior), which `sub_merge` walks to relabel them;
    /// * a pixel whose list holds one root is in `i`'s set, and every pixel
    ///   of a set has a pixel of its region in its neighbourhood (true when
    ///   the set is built, kept by unions, and no pixel ever leaves a
    ///   region), so that root is `i` and the neighbourhood is `lq` alone;
    /// * the list's entries are distinct, so substituting `lq` for `i` can
    ///   only make `i`'s place and `lq`'s place one (the first stays): the
    ///   neighbourhood's colours are the list's colours with `i`'s place
    ///   taking `lq`'s colour, less the later place when `lq` is in the list.
    ///   The term is therefore fixed by the list's colour sequence, `i`'s
    ///   place and `lq`'s place or, when it is not there, `lq`'s colour, and
    ///   is kept per pixel under those, the sequence as a version number the
    ///   relabelling renews whenever it changes a colour or the length.
    ///
    /// The caller sums the terms in the set's order as before, so every sum
    /// is the original's to the bit. The lists and versions are rebuilt each
    /// sweep; the colours are read fresh each sweep.
    fn sweep_term(&mut self, q: i32, i: i32, lq: i32) -> f32 {
        let at = self.hood(q);
        let hood = self.sub.terms.hoods[at];
        let (off, n) = (hood.off as usize, hood.n as usize);
        let mut place = Some((0u64, ABSENT));
        if n > 1 {
            let list = &self.sub.terms.ids[off..off + n];
            let own = list.iter().position(|&v| v == i);
            let other = list.iter().position(|&v| v == lq);
            place = own.map(|p| (p as u64, other.map_or(ABSENT, |p| p as u64)));
        }
        let mut key = 0u64;
        if let Some((own, other)) = place {
            let class = if other == ABSENT {
                self.term_colour(lq).1 as u64
            } else {
                0
            };
            if class < 1 << 24 {
                key = (hood.version as u64) << 33 | own << 29 | other << 25 | class << 1 | 1;
                if let Some(k) = hood.keys.iter().position(|&v| v == key) {
                    return hood.costs[k];
                }
            }
        }
        let mut ids = [0i32; 9];
        let mut m = 1usize;
        ids[0] = lq;
        if n > 1 {
            m = 0;
            for k in 0..n {
                let id = self.sub.terms.ids[off + k];
                let v = if id == i { lq } else { id };
                if !ids[..m].contains(&v) {
                    ids[m] = v;
                    m += 1;
                }
            }
        }
        let cost = self.solve_term(q, &ids, m) as f32;
        if key != 0 {
            let hood = &mut self.sub.terms.hoods[at];
            let k = hood.next as usize;
            hood.keys[k] = key;
            hood.costs[k] = cost;
            hood.next = ((k + 1) % HOOD_TERMS) as u8;
        }
        cost
    }

    /// The colour-model cost of pixel `q` over the regions `ids[..m]`,
    /// solves of three or more regions kept by pixel and colours.
    fn solve_term(&mut self, q: i32, ids: &[i32; 9], m: usize) -> f64 {
        let px = self.pixel(q as usize);
        let mut colours = [[0.0f32; 4]; 9];
        let mut key: SolveKey = [u32::MAX; 11];
        key[0] = u32::from_le_bytes(px);
        key[1] = m as u32;
        for k in 0..m {
            let (colour, class) = self.term_colour(ids[k]);
            colours[k] = colour;
            key[2 + k] = class;
        }
        if m >= 3 {
            if let Some(&cost) = self.sub.terms.solves.get(&key) {
                return cost;
            }
        }
        self.sub.cm.ids = *ids;
        self.sub.cm.n = m;
        let cost = self.model_cost(px, &colours);
        if m >= 3 {
            let solves = &mut self.sub.terms.solves;
            if solves.len() >= SOLVE_MEMO_LIMIT {
                solves.clear();
            }
            solves.insert(key, cost);
        }
        cost
    }

    /// The colour of region `id` (`sub_region_colour`) and its interned
    /// class, read once per sweep.
    fn term_colour(&mut self, id: i32) -> ([f32; 4], u32) {
        let at = id as usize;
        let class = self.sub.terms.classes[at];
        if class != u32::MAX {
            return (self.sub.terms.colours[at], class);
        }
        let colour = self.sub_region_colour(id);
        let terms = &mut self.sub.terms;
        let next = terms.intern.len() as u32;
        let class = *terms.intern.entry(colour.map(f32::to_bits)).or_insert(next);
        terms.colours[at] = colour;
        terms.classes[at] = class;
        (colour, class)
    }

    /// A version no term of this sweep is kept under yet. Past 2^31 - 1
    /// of them every kept term is dropped and the count starts over.
    fn next_version(&mut self) -> u32 {
        let terms = &mut self.sub.terms;
        if terms.version >= (1 << 31) - 1 {
            terms.version = 0;
            terms.resets += 1;
            for hood in terms.hoods.iter_mut() {
                hood.keys = [0; HOOD_TERMS];
                hood.version = 0;
            }
        }
        terms.version += 1;
        terms.version
    }

    /// The root label of pixel `p`, leaving the label image and the parents
    /// as they are (`find` compresses, and which pixels the original has
    /// compressed when is part of its state; see `touch`).
    fn root_of(&self, p: usize) -> i32 {
        let mut label = self.labels.data[p];
        while self.labels.parent[label as usize] >= 0 {
            label = self.labels.parent[label as usize];
        }
        label
    }

    /// The root lookups 0x4a1de0 makes for pixel `q` over the
    /// 8-neighbourhood (`find` on the pixel and each neighbour inside the
    /// image), for their path compression alone. Between two unions every
    /// root is fixed, and `find` only points a pixel and its label at that
    /// root, so the label image and parents after a union depend only on
    /// which pixels were looked up since the last one, not on the order or
    /// how often: the sweep looks up, once per step, every pixel the
    /// original's colour-model costs would have, and `sub_merge` those its
    /// re-evaluation after the union would have.
    fn touch(&mut self, q: i32) {
        let slot = self.sub.terms.slot[q as usize] as usize;
        let inner = match self.sub.terms.hoods.get(slot) {
            Some(hood) if hood.pixel == q as u32 => hood.inner,
            _ => false,
        };
        if inner {
            let (p, w) = (q as usize, self.width);
            for p in [
                p - w - 1,
                p - w,
                p - w + 1,
                p - 1,
                p,
                p + 1,
                p + w - 1,
                p + w,
                p + w + 1,
            ] {
                if self.labels.parent[self.labels.data[p] as usize] >= 0 {
                    self.labels.find(p);
                }
            }
            return;
        }
        let (w, h) = (self.width as i32, self.height as i32);
        self.labels.find(q as usize);
        for (nx, ny) in neighbours(false, q % w, q / w, w, h) {
            self.labels.find((ny * w + nx) as usize);
        }
    }

    /// The original's colour-model costs of this step reached the first
    /// `count` pixels of the set (see `touch`).
    fn reach(&mut self, count: usize) {
        let terms = &mut self.sub.terms;
        terms.reached = terms.reached.max(count);
    }

    /// The index of pixel `q`'s entry in the term cache, made from its own
    /// root and its 8-neighbours' distinct roots in table order if it has
    /// none this sweep.
    fn hood(&mut self, q: i32) -> usize {
        let slot = self.sub.terms.slot[q as usize] as usize;
        if let Some(hood) = self.sub.terms.hoods.get(slot) {
            if hood.pixel == q as u32 {
                return slot;
            }
        }
        let (w, h) = (self.width as i32, self.height as i32);
        let mut ids = [0i32; 9];
        ids[0] = self.root_of(q as usize);
        let mut n = 1usize;
        for (nx, ny) in neighbours(false, q % w, q / w, w, h) {
            let id = self.root_of((ny * w + nx) as usize);
            if !ids[..n].contains(&id) {
                ids[n] = id;
                n += 1;
            }
        }
        let version = self.next_version();
        let terms = &mut self.sub.terms;
        let (x, y) = (q % w, q / w);
        let hood = Hood {
            pixel: q as u32,
            off: terms.ids.len() as u32,
            n: n as u8,
            next: 0,
            inner: x > 0 && y > 0 && x < w - 1 && y < h - 1,
            version,
            keys: [0; HOOD_TERMS],
            costs: [0.0; HOOD_TERMS],
        };
        terms.ids.extend_from_slice(&ids[..n]);
        let at = terms.hoods.len();
        terms.slot[q as usize] = at as u32;
        terms.hoods.push(hood);
        at
    }

    /// Region `a` has joined `b`: every kept list of more than one root that
    /// names `a` (all of them belong to pixels of `set`, `a`'s set) names
    /// `b` there instead, once, under a new version when that changes a
    /// colour or the length.
    fn relabel_hoods(&mut self, set: &[i32], a: i32, b: i32) {
        if self.sub.terms.hoods.is_empty() {
            return;
        }
        let same = self.term_colour(a).1 == self.term_colour(b).1;
        for &q in set {
            let slot = self.sub.terms.slot[q as usize] as usize;
            let Some(&hood) = self.sub.terms.hoods.get(slot) else {
                continue;
            };
            if hood.pixel != q as u32 || hood.n < 2 {
                continue;
            }
            let (off, n) = (hood.off as usize, hood.n as usize);
            let ids = &mut self.sub.terms.ids[off..off + n];
            let Some(pa) = ids.iter().position(|&v| v == a) else {
                continue;
            };
            let renewed = match ids.iter().position(|&v| v == b) {
                Some(pb) => {
                    let (keep, drop) = (pa.min(pb), pa.max(pb));
                    ids[keep] = b;
                    ids.copy_within(drop + 1.., drop);
                    self.sub.terms.hoods[slot].n = hood.n - 1;
                    true
                }
                None => {
                    ids[pa] = b;
                    !same
                }
            };
            if renewed {
                let version = self.next_version();
                self.sub.terms.hoods[slot].version = version;
            }
        }
    }

    /// `sweep_term(q, i, lq)` for the pixel `q` at place `k` of `i`'s set,
    /// the step prepared by `step_class` (class `at`, stamp `stamp`).
    ///
    /// Where `lq` is not in the pixel's list the term depends on `lq` only
    /// through its colour (see `sweep_term`), so every candidate of one
    /// colour class shares it: it is kept per class for the step, with the
    /// running sum, filled in set order as far as any candidate of the class
    /// has gone. `lq` can be in the list only of a pixel whose list has two
    /// or more roots, and those are in `lq`'s set (see `sweep_term`), which
    /// `step_class` marked (stamp 0: `lq`'s set is the larger, and the list
    /// itself is searched): only those pixels take `sweep_term` itself.
    fn step_term(&mut self, k: usize, q: i32, i: i32, lq: i32, at: usize, stamp: u32) -> f32 {
        if self.sub.terms.step_terms[at].len() == k {
            self.push_class_term(at, k, q);
        }
        if self.near(k, lq, stamp) {
            return self.sweep_term(q, i, lq);
        }
        self.sub.terms.step_terms[at][k]
    }

    /// Whether the pixel at place `k` of the step may hold candidate `lq`
    /// in its list (or the step does not know the sweeping region's place).
    fn near(&self, k: usize, lq: i32, stamp: u32) -> bool {
        let t = &self.sub.terms;
        if t.step_keys[k] == 0 {
            return true;
        }
        let hood = t.step_hoods[k] as usize;
        if stamp != 0 {
            return t.marks[hood] == stamp;
        }
        let e = &t.hoods[hood];
        t.ids[e.off as usize..e.off as usize + e.n as usize].contains(&lq)
    }

    /// Appends the class `at` term of the pixel `q` at place `k` (the next
    /// one) and the running sum through it.
    fn push_class_term(&mut self, at: usize, k: usize, q: i32) {
        let key = self.sub.terms.step_keys[k];
        let cost = if key == 0 {
            0.0
        } else {
            self.class_term(q, self.sub.terms.step_hoods[k] as usize, key, at)
        };
        let terms = &mut self.sub.terms;
        let sum = terms.step_sums[at].last().copied().unwrap_or(0.0) + cost as f64;
        terms.step_terms[at].push(cost);
        terms.step_sums[at].push(sum);
    }

    /// The term of pixel `q` (entry `hood`, step key `key`) for a candidate
    /// of class `at` in no place of its list: the list's colours with the
    /// sweeping region's place taking the class's colour.
    fn class_term(&mut self, q: i32, hood: usize, key: u64, at: usize) -> f32 {
        let (class, region) = self.sub.terms.step_classes[at];
        let key = key | (class as u64) << 1;
        let entry = self.sub.terms.hoods[hood];
        if let Some(k) = entry.keys.iter().position(|&v| v == key) {
            return entry.costs[k];
        }
        let (off, n) = (entry.off as usize, entry.n as usize);
        let mut ids = [0i32; 9];
        ids[..n].copy_from_slice(&self.sub.terms.ids[off..off + n]);
        ids[((key >> 29) & 15) as usize] = region;
        // A region of the class may sit twice in `ids` now; both places
        // then read its colour, which is the sequence meant.
        let cost = self.solve_term(q, &ids, n) as f32;
        let entry = &mut self.sub.terms.hoods[hood];
        let k = entry.next as usize;
        entry.keys[k] = key;
        entry.costs[k] = cost;
        entry.next = ((k + 1) % HOOD_TERMS) as u8;
        cost
    }

    /// Readies `step_term` for candidate `lq` of region `i` (its set `set`):
    /// the step's entries and keys once per region, `lq`'s class and, unless
    /// `lq`'s set is the larger (stamp 0), a new stamp on the pixels of `lq`'s
    /// set. Returns the class, the stamp and the first place that may hold
    /// `lq` (the set's length if none), the class's terms and running sums
    /// filled before it; None when `lq`'s class is past what a key holds or
    /// a new class would pass `STEP_TERMS_LIMIT`.
    fn step_class(&mut self, set: &[i32], i: i32, lq: i32) -> Option<(usize, u32, usize)> {
        if self.sub.terms.step_region != i {
            self.prepare_step(set, i);
        }
        let class = self.term_colour(lq).1;
        if class >= 1 << 24 {
            return None;
        }
        let terms = &mut self.sub.terms;
        let known = terms.step_classes.iter().any(|&(c, _)| c == class);
        if !known && (terms.step_classes.len() + 1) * set.len() > STEP_TERMS_LIMIT {
            return None;
        }
        let mut stamp = 0;
        let mut first = terms.step_unknown;
        if self.sub.regions[lq as usize].set.len() <= set.len() {
            terms.marks.resize(terms.hoods.len(), 0);
            if terms.stamp == u32::MAX {
                terms.stamp = 0;
                terms.marks.fill(0);
            }
            terms.stamp += 1;
            stamp = terms.stamp;
            let mut found = false;
            for &q in self.sub.regions[lq as usize].set.iter() {
                let slot = terms.slot[q as usize] as usize;
                if terms
                    .hoods
                    .get(slot)
                    .is_some_and(|hood| hood.pixel == q as u32)
                {
                    terms.marks[slot] = stamp;
                    if !found {
                        if let Ok(k) = set.binary_search(&q) {
                            found = true;
                            first = first.min(k);
                        }
                    }
                }
            }
        }
        let at = match terms.step_classes.iter().position(|&(c, _)| c == class) {
            Some(at) => at,
            None => {
                let at = terms.step_classes.len();
                terms.step_classes.push((class, lq));
                if terms.step_terms.len() <= at {
                    terms.step_terms.push(Vec::new());
                    terms.step_sums.push(Vec::new());
                }
                terms.step_terms[at].clear();
                terms.step_sums[at].clear();
                at
            }
        };
        if stamp == 0 {
            first = (0..first).find(|&k| self.near(k, lq, 0)).unwrap_or(first);
        }
        let from = self.sub.terms.step_terms[at].len();
        for (k, &q) in set.iter().enumerate().take(first).skip(from) {
            self.push_class_term(at, k, q);
        }
        Some((at, stamp, first))
    }

    /// The step's entry and key (less the class) per place of region `i`'s
    /// set `set` (ascending), and the first place with no key.
    fn prepare_step(&mut self, set: &[i32], i: i32) {
        loop {
            let resets = self.sub.terms.resets;
            let mut hoods = std::mem::take(&mut self.sub.terms.step_hoods);
            let mut keys = std::mem::take(&mut self.sub.terms.step_keys);
            hoods.clear();
            keys.clear();
            for &q in set {
                let at = self.hood(q);
                let hood = self.sub.terms.hoods[at];
                let own = if hood.n == 1 {
                    Some(0)
                } else {
                    let (off, n) = (hood.off as usize, hood.n as usize);
                    self.sub.terms.ids[off..off + n]
                        .iter()
                        .position(|&v| v == i)
                };
                hoods.push(at as u32);
                keys.push(own.map_or(0, |own| {
                    (hood.version as u64) << 33 | (own as u64) << 29 | ABSENT << 25 | 1
                }));
            }
            let terms = &mut self.sub.terms;
            terms.step_hoods = hoods;
            terms.step_keys = keys;
            if terms.resets == resets {
                break;
            }
        }
        let terms = &mut self.sub.terms;
        let kept: usize = terms.step_terms.iter().map(Vec::capacity).sum();
        if kept > 2 * STEP_TERMS_LIMIT {
            terms.step_terms = Vec::new();
            terms.step_sums = Vec::new();
        }
        terms.step_unknown = terms
            .step_keys
            .iter()
            .position(|&key| key == 0)
            .unwrap_or(set.len());
        terms.step_region = i;
        terms.step_classes.clear();
    }

    /// A region's step starts: its set not prepared, no neighbour met.
    /// Returns the step's number.
    fn begin_step(&mut self) -> u32 {
        let terms = &mut self.sub.terms;
        terms.step_region = -1;
        terms.reached = 0;
        if terms.step == u32::MAX {
            terms.step = 0;
            terms.met.fill(0);
        }
        terms.step += 1;
        terms.step
    }

    /// A sweep starts: no neighbourhood kept, no colour read.
    fn begin_terms(&mut self) {
        let labels = (self.labels.max + 1).max(0) as usize;
        let pixels = self.width * self.height;
        let terms = &mut self.sub.terms;
        terms.hoods.clear();
        terms.ids.clear();
        terms.version = 0;
        terms.step_region = -1;
        terms.marks.clear();
        terms.stamp = 0;
        terms.classes.clear();
        terms.classes.resize(labels, u32::MAX);
        terms.met.clear();
        terms.met.resize(labels, 0);
        terms.step = 0;
        terms.colours.resize(labels, [0.0; 4]);
        if terms.slot.len() != pixels {
            terms.slot = vec![u32::MAX; pixels];
        }
    }

    /// 0x48a370: the cached cost of every region's own boundary pixels.
    fn refresh_costs(&mut self) {
        let w = self.width as i32;
        for i in 0..=self.labels.max {
            let set: Vec<i32> = self.sub.regions[i as usize].set.iter().copied().collect();
            for q in set {
                if self.labels.find(q as usize) != i {
                    continue;
                }
                let c = self.pixel_cost(q % w, q / w, -1, -1) as f32;
                self.sub.cost[q as usize] = c;
            }
        }
    }

    /// 0x489f40: the cost of describing region `i`'s interior pixels with
    /// the colour of region `j` (its own when `j` is -1): the clamped
    /// per-channel variance plus the squared colour difference times the
    /// pixel count.
    fn interior_cost(&self, i: i32, j: i32) -> f64 {
        let r = &self.sub.regions[i as usize];
        let n = r.interior;
        if n <= 0 {
            return 0.0;
        }
        let reference = self.sub_region_colour(if j >= 0 { j } else { i });
        let nf = n as f64;
        let inv = 1.0 / nf;
        let mut sum = [0.0f64; 4];
        for ch in 0..4 {
            let mean = r.isums[ch] as f64 * inv;
            let var = r.isquares[ch] as f64 - nf * (mean * mean);
            let var = if ch < 3 && !(var > 0.0) { 0.0 } else { var };
            let diff = reference[ch] as f64 - mean;
            sum[ch] = diff * diff * nf + var;
        }
        ((sum[0] + sum[1]) + sum[2]) + sum[3]
    }

    /// 0x48af40: the model cost over region `i`'s boundary set: the cached
    /// costs when `cached`, otherwise re-evaluated with `i` standing in
    /// as `j` (each term rounded to single precision, `store` writing it
    /// back to the cache).
    fn set_cost(&mut self, cached: bool, store: bool, i: i32, j: i32) -> f64 {
        let w = self.width as i32;
        let set: Vec<i32> = self.sub.regions[i as usize].set.iter().copied().collect();
        let mut sum = 0.0f64;
        if cached {
            for q in set {
                sum += self.sub.cost[q as usize] as f64;
            }
            return sum;
        }
        for q in set {
            let c = self.pixel_cost(q % w, q / w, j, i) as f32;
            if store {
                self.sub.cost[q as usize] = c;
            }
            sum += c as f64;
        }
        sum
    }

    /// 0x48b090: like `set_cost` without rounding, stopping once the sum
    /// passes `limit`.
    fn set_cost_limited(&mut self, limit: f64, i: i32, j: i32) -> f64 {
        let w = self.width as i32;
        let set: Vec<i32> = self.sub.regions[i as usize].set.iter().copied().collect();
        let mut sum = 0.0f64;
        for q in set {
            sum += self.pixel_cost(q % w, q / w, j, i);
            if sum > limit {
                return sum;
            }
        }
        sum
    }

    /// 0x48af00 (the segmentation's virtual cost): interior and boundary.
    fn region_cost(&mut self, i: i32, j: i32) -> f64 {
        let interior = self.interior_cost(i, j);
        self.set_cost(j < 0, false, i, j) + interior
    }

    /// 0x4a6bb0: the cost of region `i` joining its neighbour `lq`: its
    /// interior described by `lq`'s colour less its own cost `own`
    /// (`region_cost(i, -1)`, which no candidate of one sweep step changes,
    /// so the sweep computes it once), plus its boundary pixels `set` (the
    /// region's set in its order) re-evaluated as `lq`'s unless the first
    /// part is already above lambda.
    ///
    /// The original sums every boundary pixel for every neighbour, so a
    /// region with a long boundary and many neighbours (the lines of a
    /// grid) costs boundary x neighbours colour-model solves per sweep. The
    /// sweep only asks whether the cost is below `best`, and with
    /// non-negative penalties every term is non-negative (or NaN), so the
    /// running sum never decreases and `c1 + sum` is monotone in it: once
    /// `best > c1 + sum` fails for a partial sum it fails for the whole one
    /// (a NaN term keeps it failing), and the rest is skipped. `pixel_cost`
    /// reads nothing it wrote before (the colour model is scratch and path
    /// compression leaves every root unchanged), so skipping it changes
    /// nothing else.
    ///
    /// Each rounded term is kept in the set's order, the first
    /// `terms.current_prefix` of them as the class terms of the step and the
    /// rest in `terms.current`; a cost that beats `best` summed all of them,
    /// and they are the costs `sub_merge` stores when `lq` is chosen (see
    /// `sub_merge`). Over the 8-neighbourhood the terms come from
    /// `step_class` and `step_term`.
    fn merge_cost(&mut self, own: f64, best: f64, set: &[i32], i: i32, lq: i32) -> f64 {
        let c1 = self.interior_cost(i, lq) - own;
        if c1 > self.params.sub_lambda_f as f64 {
            return c1;
        }
        let bounded = self.params.self_weight_penalty_low >= 0.0
            && self.params.self_weight_penalty_high >= 0.0;
        let w = self.width as i32;
        let kept = !self.sub.use24 && !self.sub.terms.reference;
        let class = if kept {
            self.step_class(set, i, lq)
        } else {
            None
        };
        let mut terms = std::mem::take(&mut self.sub.terms.current);
        terms.clear();
        self.sub.terms.current_prefix = (0, 0);
        let mut sum = 0.0f64;
        let mut k = 0;
        if let Some((at, _, first)) = class {
            if first > 0 {
                // Up to `first` the terms are the class's, added from 0 in
                // the same order, so the running sum is the class's.
                sum = self.sub.terms.step_sums[at][first - 1];
                self.sub.terms.current_prefix = (at, first);
                k = first;
                if bounded && !(best > c1 + sum) {
                    // Failing here it failed at some place before, where
                    // the original stops; either cost loses to `best`.
                    let sums = &self.sub.terms.step_sums[at][..first];
                    let stop = sums.partition_point(|&s| best > c1 + s);
                    self.reach(stop + 1);
                    self.sub.terms.current = terms;
                    return c1 + sum;
                }
            }
        }
        'terms: while k < set.len() {
            let c = match class {
                Some((at, stamp, _)) => {
                    // The class's kept terms, as far as they go and no
                    // pixel may hold `lq`.
                    let kept = self.sub.terms.step_terms[at].len();
                    while k < kept && !self.near(k, lq, stamp) {
                        let c = self.sub.terms.step_terms[at][k];
                        terms.push(c);
                        sum += c as f64;
                        k += 1;
                        if bounded && !(best > c1 + sum) {
                            break 'terms;
                        }
                    }
                    if k == set.len() {
                        break;
                    }
                    self.step_term(k, set[k], i, lq, at, stamp)
                }
                None if kept => self.sweep_term(set[k], i, lq),
                None => self.pixel_cost(set[k] % w, set[k] / w, lq, i) as f32,
            };
            terms.push(c);
            sum += c as f64;
            k += 1;
            if bounded && !(best > c1 + sum) {
                break;
            }
        }
        if kept {
            self.reach(k);
        }
        self.sub.terms.current = terms;
        c1 + sum
    }

    /// 0x47e9f0: pixel `q` belongs to region `label` and so do all its
    /// neighbours inside the image.
    fn is_interior(&mut self, label: i32, q: i32) -> bool {
        let (w, h) = (self.width as i32, self.height as i32);
        let x = q % w;
        let y = q / w;
        if self.labels.find(q as usize) != label {
            return false;
        }
        for (nx, ny) in neighbours(self.sub.use24, x, y, w, h) {
            if self.labels.find((ny * w + nx) as usize) != label {
                return false;
            }
        }
        true
    }

    /// 0x48bc60: region `a` joins region `b` (the sweeping region joins
    /// the chosen neighbour). `costs` are the rounded terms `merge_cost`
    /// summed for `b` over `a`'s set, in its order.
    ///
    /// The original re-evaluates `a`'s set as `b`'s after the union
    /// (`set_cost(false, true, a, b)`, its sum unused) and stores each term.
    /// A pixel's roots after the union are its roots before it with `a`
    /// read as `b`, which is the substitution `merge_cost` made, so those are
    /// the terms `costs` holds, and they are stored instead (over the
    /// 8-neighbourhood, its root lookups kept by `touch`). The sets are
    /// joined by walking the smaller one: the joined set and the pixels found
    /// in both (visited in ascending order either way, so the interior sums
    /// fold in the same order) do not depend on which one is walked.
    fn sub_merge(&mut self, a: i32, b: i32, costs: &[f32]) {
        self.labels.union(b, a);
        // 0x488fe0: the stats fold into b.
        {
            let ra = &self.sub.regions[a as usize];
            let (count, sums, squares) = (ra.count, ra.sums, ra.squares);
            let (interior, isums, isquares) = (ra.interior, ra.isums, ra.isquares);
            let rb = &mut self.sub.regions[b as usize];
            rb.count += count;
            for (s, v) in rb.sums.iter_mut().zip(sums) {
                *s += v;
            }
            for (s, v) in rb.squares.iter_mut().zip(squares) {
                *s += v;
            }
            rb.interior += interior;
            for (s, v) in rb.isums.iter_mut().zip(isums) {
                *s += v;
            }
            for (s, v) in rb.isquares.iter_mut().zip(isquares) {
                *s += v;
            }
            let ra = &mut self.sub.regions[a as usize];
            ra.count = 0;
            ra.sums = [0.0; 4];
            ra.interior = 0;
            ra.isums = [0.0; 4];
            ra.isquares = [0.0; 4];
        }
        let set: Vec<i32> = self.sub.regions[a as usize].set.iter().copied().collect();
        debug_assert_eq!(set.len(), costs.len());
        if self.sub.use24 || self.sub.terms.reference {
            self.set_cost(false, true, a, b);
        } else {
            for (&q, &c) in set.iter().zip(costs) {
                self.sub.cost[q as usize] = c;
                self.touch(q);
            }
            self.relabel_hoods(&set, a, b);
        }
        let (ua, ub) = (a as usize, b as usize);
        if self.sub.regions[ua].set.len() > self.sub.regions[ub].set.len() {
            let taken = std::mem::take(&mut self.sub.regions[ua].set);
            let kept = std::mem::replace(&mut self.sub.regions[ub].set, taken);
            self.sub.regions[ua].set = kept;
        }
        let walked: Vec<i32> = self.sub.regions[ua].set.iter().copied().collect();
        for q in walked {
            if self.sub.regions[ub].set.insert(q) {
                continue;
            }
            if self.is_interior(b, q) {
                let f = self.pixel_floats(q as usize);
                let rb = &mut self.sub.regions[ub];
                rb.add_interior(f);
                rb.set.remove(&q);
            }
        }
        self.sub.regions[ua].set.clear();
    }

    /// 0x48ac00 on the sub-pixel segmentation: the labels compacted to
    /// 4-connected components in scan order, colour indices kept.
    fn sub_renumber(&mut self) {
        let keep = self.sub.renumbered && self.sub.regions[0].colour >= 0;
        let saved = kept_colours(keep, self.labels.max, |i| self.sub.regions[i].colour);
        let (list, count) = self.labels.renumber_components(saved.as_deref());
        if (self.sub.regions.len() as i32) < count {
            self.sub
                .regions
                .resize(count as usize, SubRegion::default());
        }
        self.sub.renumbered = true;
        if keep {
            for (i, &colour) in list.iter().enumerate() {
                self.sub.regions[i].colour = colour;
            }
        }
        for l in self.labels.data.iter_mut() {
            *l = -1 - *l;
        }
    }

    /// 0x48b690: the sub-pixel segmentation from the label image.
    pub(super) fn sub_rebuild(&mut self, use24: bool, keep: bool) {
        if !keep {
            self.sub_renumber();
        }
        self.sub.use24 = use24;
        let keep_colours = self.sub.renumbered && self.sub.regions[0].colour >= 0;
        self.labels.flatten();
        for i in 0..=self.labels.max as usize {
            let r = &mut self.sub.regions[i];
            let flags = r.flags;
            r.reset();
            if keep {
                r.flags = flags;
            }
        }
        let (w, h) = (self.width as i32, self.height as i32);
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) as usize;
                let label = self.labels.data[p] as usize;
                let mut boundary = false;
                for (nx, ny) in neighbours(use24, x, y, w, h) {
                    let q = ny * w + nx;
                    if self.labels.data[q as usize] as usize != label {
                        boundary = true;
                        self.sub.regions[label].set.insert(q);
                    }
                }
                let f = self.pixel_floats(p);
                let bytes = self.pixel(p);
                let r = &mut self.sub.regions[label];
                r.accumulate(f);
                if boundary {
                    r.set.insert(p as i32);
                } else {
                    r.add_interior(f);
                    if !keep_colours {
                        for (channel, &b) in r.channels.iter_mut().zip(bytes.iter()) {
                            channel.push(b);
                        }
                    }
                }
            }
        }
        for i in 0..=self.labels.max as usize {
            if keep_colours {
                continue;
            }
            let r = &mut self.sub.regions[i];
            if r.interior >= 3 {
                // 0x4830a0: the per-channel median of the interior pixels.
                let mut colour = [0.0f32; 4];
                for (cv, channel) in colour.iter_mut().zip(r.channels.iter()) {
                    let mut v = channel.clone();
                    let mid = v.len() / 2;
                    v.sort_unstable();
                    *cv = v[mid] as f32 * INV255_F32;
                }
                r.colour_floats = colour;
            } else {
                let scale = 1.0f32 / r.count as f32;
                for ch in 0..4 {
                    r.colour_floats[ch] = r.sums[ch] * scale;
                }
            }
            for c in r.channels.iter_mut() {
                c.clear();
            }
        }
        self.refresh_costs();
    }

    /// 0x4a8640: one sweep in which every region of at most `own_limit`
    /// pixels joins the neighbour of at most `neighbour_limit` pixels with
    /// the lowest merge cost under lambda; the number of merges.
    pub(super) fn sweep(&mut self, neighbour_limit: i32, own_limit: i32) -> i32 {
        let mut merges = 0;
        self.begin_terms();
        for i in 0..=self.labels.max {
            let r = &self.sub.regions[i as usize];
            let size = if self.sub.use_interior_size {
                r.interior
            } else {
                r.count
            };
            if self.labels.parent[i as usize] >= 0 || size > own_limit {
                continue;
            }
            let mut best = self.params.sub_lambda_f as f64;
            let mut chosen = -1;
            let mut own = None;
            let mut set = std::mem::take(&mut self.sub.scratch);
            set.clear();
            set.extend(r.set.iter().copied());
            let step = self.begin_step();
            for &q in &set {
                let lq = self.labels.find(q as usize);
                if lq == i {
                    continue;
                }
                // A neighbour met again in this step is skipped: the first
                // time it was costed (and filed) or skipped, and neither
                // the visited set nor its size has changed back since.
                if self.sub.terms.met[lq as usize] == step && !self.sub.terms.reference {
                    continue;
                }
                self.sub.terms.met[lq as usize] = step;
                let key = visited_key(i, lq);
                if self.sub.visited.contains(&key) {
                    continue;
                }
                if self.sub.regions[lq as usize].count > neighbour_limit {
                    continue;
                }
                let own = *own.get_or_insert_with(|| self.region_cost(i, -1));
                let cost = self.merge_cost(own, best, &set, i, lq);
                if best > cost {
                    best = cost;
                    chosen = lq;
                    let terms = &mut self.sub.terms;
                    std::mem::swap(&mut terms.current, &mut terms.best);
                    terms.best_prefix = terms.current_prefix;
                }
                self.sub.visited.insert(key);
            }
            let reached = self.sub.terms.reached;
            for &q in &set[..reached] {
                self.touch(q);
            }
            self.sub.scratch = set;
            if chosen >= 0 {
                let terms = &mut self.sub.terms;
                let mut costs = std::mem::take(&mut terms.costs);
                costs.clear();
                let (at, len) = terms.best_prefix;
                if len > 0 {
                    costs.extend_from_slice(&terms.step_terms[at][..len]);
                }
                costs.extend_from_slice(&terms.best);
                self.sub_merge(i, chosen, &costs);
                self.sub.terms.costs = costs;
                merges += 1;
            }
        }
        self.sub.visited.clear();
        merges
    }

    /// 0x4a7f20: regions without interior pixels that are mostly lines
    /// (at most 1.2 axis-aligned neighbours per pixel) have their pixels
    /// re-assigned to the colour model's strongest region, which the
    /// model's count field (-1) makes the pixel's own; the cached costs are
    /// refreshed on the way.
    pub(super) fn beach(&mut self) {
        let w = self.width as i32;
        for i in 0..=self.labels.max {
            if self.labels.parent[i as usize] >= 0 || self.sub.regions[i as usize].interior != 0 {
                continue;
            }
            let set: Vec<i32> = self.sub.regions[i as usize].set.iter().copied().collect();
            let mut own = Vec::new();
            for q in set {
                if self.labels.find(q as usize) == i {
                    own.push(q);
                }
            }
            let mut aligned = 0i32;
            let mut n = 0i32;
            for &q in &own {
                let x = q % w;
                let y = q / w;
                if self.label_at(x, y - 1) == i || self.label_at(x, y + 1) == i {
                    aligned += 1;
                }
                if self.label_at(x - 1, y) == i || self.label_at(x + 1, y) == i {
                    aligned += 1;
                }
                n += 1;
            }
            let ratio = aligned as f64 / n as f64;
            if ratio > 1.2 {
                continue;
            }
            loop {
                let mut changed = 0;
                let mut k = 0;
                while k < own.len() {
                    let q = own[k];
                    let old = -self.sub.cost[q as usize];
                    let c = self.pixel_cost(q % w, q / w, -1, i) as f32;
                    if self.params.beach_lambda_f > c + old {
                        // 0x4a1dc0: the argmax over a count of -1 is index 0.
                        self.labels.data[q as usize] = self.sub.cm.ids[0];
                        own.remove(k);
                        changed += 1;
                        self.sub.cost[q as usize] = c;
                    } else {
                        k += 1;
                    }
                }
                if changed == 0 {
                    break;
                }
            }
        }
    }

    /// 0x48c6f0(0, 1): the sub-pixel segmentation over the current labels.
    pub(super) fn sub_init(&mut self) {
        self.sub.cm = ColourModel::new();
        let n = (self.labels.max + 1) as usize;
        self.sub.regions = vec![SubRegion::default(); n];
        self.sub.cost = vec![0.0; self.width * self.height];
        for r in self.sub.regions.iter_mut() {
            r.colour = -1;
        }
        self.sub_rebuild(false, false);
        self.sub.renumbered = true;
    }

    /// 0x4a9ac0: the sub-pixel segmentation.
    ///
    /// Every sweep phase starts after `sub_init` or `sub_rebuild`: the
    /// sweep's cached terms (`sweep_term`, `near`, `relabel_hoods`) rely on
    /// the neighbourhood lists those build and `sub_merge` keeps. A caller
    /// that changed labels or boundary sets between sweeps without a
    /// rebuild would break their exactness silently (the adversarial review
    /// of September 23, 2026).
    pub(super) fn sub_run(&mut self) {
        self.sub.use_interior_size = false;
        self.sub.visited.clear();
        self.sub_init();
        if self.sweep(1, 1) != 0 {
            let mut i = 0;
            while i < MAX_SWEEPS {
                i += 1;
                if self.sweep(i + 1, i + 1) == 0 {
                    break;
                }
            }
        }
        self.sub_rebuild(self.sub.use_interior_size, false);
        if self.sweep(INFINITE, 1) != 0 {
            let mut i = 0;
            while i < MAX_SWEEPS {
                i += 1;
                if self.sweep(INFINITE, i + 1) == 0 {
                    break;
                }
            }
        }
        self.sub_rebuild(false, false);
        if self.sweep(INFINITE, INFINITE) != 0 {
            let mut i = 0;
            while i < MAX_SWEEPS {
                i += 1;
                if self.sweep(INFINITE, INFINITE) == 0 {
                    break;
                }
            }
        }
        if self.params.use_connected_24_iteration {
            unimplemented!("0x4a9ac0: use_connected_24_iteration is selected by no preset");
        }
        self.beach();
        self.extract_diagonals();
        self.sub_rebuild(self.sub.use_interior_size, false);
        if self.sweep(INFINITE, 4) != 0 {
            let mut i = 0;
            while i < MAX_SWEEPS {
                i += 1;
                if self.sweep(INFINITE, 4) == 0 {
                    break;
                }
            }
        }
        self.sub_rebuild(self.sub.use_interior_size, false);
    }

    /// 0x4a83a0: regions with fewer than three interior pixels adopt the
    /// colour of the large region that explains them best.
    pub(super) fn sub_recolour(&mut self) {
        let max = self.labels.max as usize;
        let mut large: Vec<(i32, i32)> = (0..=max)
            .filter(|&i| self.sub.regions[i].interior >= 3)
            .map(|i| (self.sub.regions[i].interior, i as i32))
            .collect();
        large.sort_unstable_by(|a, b| b.cmp(a));
        large.truncate(100);
        for i in 0..=max as i32 {
            if self.sub.regions[i as usize].interior >= 3 {
                continue;
            }
            let mut base = self.region_cost(i, -1) - self.params.color_cluster_margin as f64;
            let mut chosen = i;
            for &(_, j) in &large {
                let c = self.interior_cost(i, j);
                if !(base > c) {
                    continue;
                }
                let extra = self.set_cost_limited(base - c, i, j);
                let total = extra + c;
                if base > total {
                    base = total;
                    chosen = j;
                }
            }
            if chosen != i {
                let from = self.sub.regions[chosen as usize].clone();
                let r = &mut self.sub.regions[i as usize];
                r.colour_floats = from.colour_floats;
                r.colour = from.colour;
            }
        }
    }

    /// 0x48b160: the region colours into the colour table, the labels
    /// replaced by colour indices and the super-pixel segmentation's
    /// regions re-derived from them.
    pub(super) fn sub_quantise(&mut self) {
        if !(self.sub.renumbered && self.sub.regions[0].colour >= 0) {
            // 0x489de0 on the sub-pixel segmentation.
            self.seg.colour_map.clear();
            for i in 0..=self.labels.max as usize {
                let bytes = colour_bytes(self.sub_region_colour(i as i32));
                let index = self.seg.colour_index(bytes);
                self.sub.regions[i].colour = index;
            }
            self.seg.build_colour_table();
        }
        for p in 0..self.labels.data.len() {
            let label = self.labels.data[p];
            self.labels.data[p] = self.sub.regions[label as usize].colour;
        }
        let ncol = self.seg.colours.len() as i32;
        let n = if self.labels.max < ncol {
            ncol
        } else {
            self.labels.max
        };
        self.labels.set_max(n);
        if (self.seg.regions.len() as i32) <= n {
            if self.seg.renumbered && self.seg.regions[0].colour >= 0 {
                self.seg.regions.resize((n + 1) as usize, Region::default());
            } else {
                self.seg.regions = vec![Region::default(); (n + 1) as usize];
            }
        }
        self.seg.renumbered = true;
        for i in 0..ncol as usize {
            self.seg.regions[i].colour = i as i32;
        }
        self.stats();
    }

    /// 0x483150: the two edge images over the engine image (`src`, the
    /// prepared image) and `pre` (the copy 0x473b20 filters again): per
    /// pixel the summed squared colour differences to the four neighbours
    /// (the edge pixels mirrored inward), and the squared difference of the
    /// `pre` pixel to the mean of its four `pre` neighbours; single
    /// precision in the listing's order.
    pub(super) fn edge_image(&self, pre: &[u8]) -> (Vec<f32>, Vec<f32>) {
        let (w, h) = (self.width as i32, self.height as i32);
        let src = self.image;
        let at = |img: &[u8], x: i32, y: i32| -> [f32; 4] {
            let p = ((y * w + x) * 4) as usize;
            [
                img[p] as f32 * INV255_F32,
                img[p + 1] as f32 * INV255_F32,
                img[p + 2] as f32 * INV255_F32,
                img[p + 3] as f32 * INV255_F32,
            ]
        };
        let sq = |d: [f32; 4]| ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]) + d[3] * d[3];
        let mut grad = vec![0.0f32; (w * h) as usize];
        let mut lap = vec![0.0f32; (w * h) as usize];
        for y in 0..h {
            let dyp = if y < h - 1 { 1 } else { -1 };
            let dym = if y <= 0 { 1 } else { -1 };
            for x in 0..w {
                let dxp = if x < w - 1 { 1 } else { -1 };
                let dxm = if x <= 0 { 1 } else { -1 };
                let c = at(src, x, y);
                let diff = |n: [f32; 4]| [c[0] - n[0], c[1] - n[1], c[2] - n[2], c[3] - n[3]];
                let mut g = sq(diff(at(src, x + dxp, y)));
                g += sq(diff(at(src, x + dxm, y)));
                g += sq(diff(at(src, x, y + dyp)));
                g += sq(diff(at(src, x, y + dym)));
                grad[(y * w + x) as usize] = g;
                let pu = at(pre, x, y + dym);
                let pd = at(pre, x, y + dyp);
                let pl = at(pre, x + dxm, y);
                let pr = at(pre, x + dxp, y);
                let pc = at(pre, x, y);
                let mut d = [0.0f32; 4];
                for k in 0..4 {
                    let avg = (((pr[k] + pl[k]) + pd[k]) + pu[k]) * 0.25;
                    d[k] = pc[k] - avg;
                }
                lap[(y * w + x) as usize] = sq(d);
            }
        }
        (grad, lap)
    }

    /// 0x4ae670 (Magick++ `gaussianBlur(1.5, 0.75)`, ImageMagick's
    /// GaussianBlurImage 0x4bab40 over the default channels): a 5x5
    /// Gaussian kernel (`exp(-(u^2 + v^2) / (2 sigma^2))` rounded to single
    /// precision over `2 pi sigma^2`, normalised by its sum) convolved over
    /// the colour channels, the alpha channel untouched, the pixels beyond
    /// the edge replicated (`edge` 0), zero (`edge` 1) or skipped with the
    /// kernel renormalised (`edge` 2).
    pub(super) fn gaussian_blur(pixels: &[u8], w: usize, h: usize, edge: u8) -> Vec<u8> {
        let sigma = 0.75f64;
        let width = 5i32;
        let half = width / 2;
        let mut kernel = Vec::with_capacity(25);
        for i in -half..=half {
            for j in -half..=half {
                let e = (-((j * j) as f64 + (i * i) as f64) / (sigma * sigma * 2.0)).exp() as f32;
                kernel.push(e as f64 / ((sigma * sigma) * std::f64::consts::TAU));
            }
        }
        let mut normalize = 0.0f64;
        for k in &kernel {
            normalize += *k;
        }
        if normalize.abs() <= 1.0e-12 {
            normalize = 1.0;
        }
        let normalize = 1.0 / normalize;
        let kernel: Vec<f64> = kernel.iter().map(|k| normalize * k).collect();
        let mut out = pixels.to_vec();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let mut sum = [0.0f64; 3];
                let mut weight = 0.0f64;
                let mut k = 0;
                for v in -half..=half {
                    for u in -half..=half {
                        let (mut sx, mut sy) = (x + u, y + v);
                        let inside = sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32;
                        if !inside {
                            match edge {
                                0 => {
                                    sx = sx.clamp(0, w as i32 - 1);
                                    sy = sy.clamp(0, h as i32 - 1);
                                }
                                1 => {
                                    k += 1;
                                    continue;
                                }
                                _ => {
                                    k += 1;
                                    continue;
                                }
                            }
                        }
                        let p = ((sy * w as i32 + sx) * 4) as usize;
                        for ch in 0..3 {
                            sum[ch] += kernel[k] * pixels[p + ch] as f64;
                        }
                        weight += kernel[k];
                        k += 1;
                    }
                }
                if edge == 2 && weight > 0.0 {
                    for s in sum.iter_mut() {
                        *s /= weight;
                    }
                }
                let o = ((y * w as i32 + x) * 4) as usize;
                for ch in 0..3 {
                    let v = sum[ch];
                    out[o + ch] = if v < 0.0 {
                        0
                    } else if v > 255.0 {
                        255
                    } else {
                        (v + 0.5) as u8
                    };
                }
            }
        }
        out
    }

    /// 0x481f60(2): the label-boundary mask: 2 on every pixel of a 2x2
    /// block whose labels differ, then 1 on the untouched pixels of every
    /// 2x2 block that holds a 2.
    pub(super) fn boundary_mask(&self) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        let mut mask = vec![0u8; w * h];
        let labels = &self.labels.data;
        for y in 0..h.saturating_sub(1) {
            for x in 0..w.saturating_sub(1) {
                let l00 = labels[y * w + x];
                if l00 == labels[y * w + x + 1]
                    && l00 == labels[(y + 1) * w + x]
                    && l00 == labels[(y + 1) * w + x + 1]
                {
                    continue;
                }
                mask[y * w + x] = 2;
                mask[y * w + x + 1] = 2;
                mask[(y + 1) * w + x] = 2;
                mask[(y + 1) * w + x + 1] = 2;
            }
        }
        let level = 1u8;
        for y in 0..h.saturating_sub(1) {
            for x in 0..w.saturating_sub(1) {
                let at = [
                    y * w + x,
                    y * w + x + 1,
                    (y + 1) * w + x,
                    (y + 1) * w + x + 1,
                ];
                if at.iter().any(|&p| (mask[p] as i8) > level as i8) {
                    for &p in &at {
                        if mask[p] == 0 {
                            mask[p] = level;
                        }
                    }
                }
            }
        }
        mask
    }

    /// 0x4896e0: the colour of region `i` (table or record) and its
    /// luminance the two ways 0x4a8800 computes it.
    fn region_luminance(&self, i: i32, own: bool) -> f64 {
        let c = self.sub_region_colour(i);
        let (c0, c1, c2) = (c[0] as f64, c[1] as f64, c[2] as f64);
        if own {
            (c0 * 0.11 + c1 * 0.59) + c2 * 0.3
        } else {
            (c2 * 0.3 + c0 * 0.11) + c1 * 0.59
        }
    }

    /// 0x4a6df0: `m` (column-major 4x4) times `v`, single precision.
    fn mat_vec(m: &[f32; 16], v: &[f32; 4]) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for c in 0..4 {
            for r in 0..4 {
                out[r] += v[c] * m[4 * c + r];
            }
        }
        out
    }

    /// 0x4a72a0: ten power iterations for the dominant eigenvector of a
    /// 4x4 matrix, stopping early when the eigenvalue estimate moves by
    /// less than the tolerance from the value left in `estimate` (which
    /// the caller carries over from the previous region).
    fn power_iteration(m: &[f32; 16], estimate: &mut f32, tolerance: f32) -> [f32; 4] {
        let mut vec = [1.0f32; 4];
        let mut v = Self::mat_vec(m, &vec);
        for _ in 0..10 {
            let mut sum = v[0] as f64 * v[0] as f64;
            sum += v[1] as f64 * v[1] as f64;
            sum += v[2] as f64 * v[2] as f64;
            sum += v[3] as f64 * v[3] as f64;
            let norm = sum.sqrt() as f32;
            if norm > 0.0 {
                let inv = 1.0f32 / norm;
                for x in v.iter_mut() {
                    *x *= inv;
                }
            }
            vec = v;
            v = Self::mat_vec(m, &vec);
            let mut dot = v[0] as f64 * vec[0] as f64;
            dot += v[1] as f64 * vec[1] as f64;
            dot += v[2] as f64 * vec[2] as f64;
            dot += v[3] as f64 * vec[3] as f64;
            let eigenvalue = dot as f32;
            let diff = (*estimate as f64 - eigenvalue as f64).abs();
            if tolerance as f64 > diff {
                break;
            }
            *estimate = eigenvalue;
        }
        vec
    }

    /// 0x4a8800: the 42 features of every region (a 0x150-byte record at
    /// engine+0x9cc): the edge-image sums, the pixel and interior counts,
    /// the label-mask count, the neighbouring regions' size classes and
    /// luminances, the region's luminance, colour variance, interior to
    /// boundary ratio and bounding-box aspect, the per-pixel averages, the
    /// closest large region's colour distance and the PCA reconstruction
    /// error. `edge` is the pair of edge images 0x483150 leaves.
    pub(super) fn region_features(&mut self, edge: &(Vec<f32>, Vec<f32>)) -> Vec<[f64; 42]> {
        for r in self.sub.regions.iter_mut() {
            r.colour = -1;
        }
        self.sub_rebuild(false, false);
        let mask = self.boundary_mask();
        let (w, h) = (self.width, self.height);
        let n = (self.labels.max + 1) as usize;
        let mut stat = vec![[0.0f64; 42]; n];
        let mut mats = vec![[0.0f32; 16]; n];
        let mut eigen = vec![[0.0f32; 4]; n];
        for (p, &mask_p) in mask.iter().enumerate().take(w * h) {
            let idx = self.labels.data[p] as usize;
            let s = &mut stat[idx];
            s[0] += edge.0[p] as f64;
            s[1] += edge.1[p] as f64;
            s[6] += (edge.0[p] as f64).sqrt();
            s[7] += edge.1[p] as f64;
            if mask_p == 0 {
                s[10] += 1.0;
            }
            let c = self.pixel_floats(p);
            let m = &mut mats[idx];
            for col in 0..4 {
                for row in 0..4 {
                    m[4 * col + row] += c[col] * c[row];
                }
            }
        }
        let mut estimate = f32::NAN;
        for i in 0..n {
            let r = &self.sub.regions[i];
            stat[i][41] = 0.0;
            let inv = 1.0f32 / r.count as f32;
            let m = &mut mats[i];
            for x in m.iter_mut() {
                *x *= inv;
            }
            let mean = r.mean();
            for col in 0..4 {
                for row in 0..4 {
                    m[4 * col + row] -= mean[row] * mean[col];
                }
            }
            eigen[i] = Self::power_iteration(m, &mut estimate, 1.0e-6);
        }
        for p in 0..w * h {
            let idx = self.labels.data[p] as usize;
            let r = &self.sub.regions[idx];
            let px = self.pixel_floats(p);
            let mean = r.mean();
            let e = eigen[idx];
            let d = [
                px[0] - mean[0],
                px[1] - mean[1],
                px[2] - mean[2],
                px[3] - mean[3],
            ];
            let proj = ((d[0] * e[0] + d[1] * e[1]) + d[2] * e[2]) + d[3] * e[3];
            let mut err = [0.0f32; 4];
            for k in 0..4 {
                err[k] = px[k] - (e[k] * proj + mean[k]);
            }
            let sq = ((err[0] as f64 * err[0] as f64 + err[1] as f64 * err[1] as f64)
                + err[2] as f64 * err[2] as f64)
                + err[3] as f64 * err[3] as f64;
            stat[idx][41] += sq;
        }
        let large = NearestColour::new(
            self.sub.regions[..n]
                .iter()
                .filter(|o| o.interior > 10)
                .map(|o| o.colour_floats),
        );
        for (i, s) in stat.iter_mut().enumerate().take(n) {
            let r = &self.sub.regions[i];
            let count = r.count as f64;
            s[2] = s[0].sqrt();
            s[3] = s[1].sqrt();
            s[4] = (s[0] / count).sqrt();
            s[5] = (s[1] / count).sqrt();
            s[8] = count;
            s[9] = r.interior as f64;
            for k in 0..8 {
                s[25 + k] = s[k] / s[8];
            }
            // The closest region of more than ten interior pixels (0x4a8800
            // scans them all, `NearestColour` finds the same minimum).
            s[39] = large.nearest(&r.colour_floats);
            let lum = self.region_luminance(i as i32, true);
            s[21] = lum;
            let inv = 1.0f32 / r.count as f32;
            let mut var = 0.0f64;
            for k in 0..4 {
                let mean = r.sums[k] * inv;
                let d = r.squares[k] * inv - mean * mean;
                var += d as f64;
            }
            s[22] = var;
            s[23] = r.interior as f64 / (r.set.len() as u32) as f64;
            let mut neighbours = BTreeSet::new();
            let (mut max_x, mut min_x, mut max_y, mut min_y) = (0i32, w as i32, 0i32, h as i32);
            for &q in &r.set {
                neighbours.insert(self.labels.data[q as usize]);
                let x = q % w as i32;
                let y = q / w as i32;
                if max_x < x {
                    max_x = x;
                }
                if min_x > x {
                    min_x = x;
                }
                if max_y < y {
                    max_y = y;
                }
                if min_y > y {
                    min_y = y;
                }
            }
            s[24] = (max_x - min_x) as f64 / (max_y - min_y) as f64;
            s[11] = (neighbours.len() as u32) as f64 - 1.0;
            for &e in &neighbours {
                if e == i as i32 {
                    continue;
                }
                let o = &self.sub.regions[e as usize];
                if o.interior == 0 {
                    s[12] += 1.0;
                }
                if o.interior >= 5 {
                    s[16] += 1.0;
                }
                if o.interior >= 10 {
                    s[17] += 1.0;
                }
                if o.interior >= 15 {
                    s[18] += 1.0;
                }
                if o.count >= 5 {
                    s[13] += 1.0;
                }
                if o.count >= 10 {
                    s[14] += 1.0;
                }
                if o.count >= 15 {
                    s[15] += 1.0;
                }
                let other = self.region_luminance(e, false);
                if other <= lum {
                    s[20] += 1.0;
                } else {
                    s[19] += 1.0;
                }
            }
        }
        stat
    }

    /// 0x4ae090: the decision tree over a region's features that marks an
    /// anti-aliasing region.
    pub(super) fn is_anti_aliasing_region(f: &[f64; 42]) -> bool {
        if 0.00402429 <= f[23] {
            if 0.0241458 <= f[23] || 0.0473801 <= f[5] {
                return false;
            }
            return 0.0138127 <= f[22] || 0.0102382 <= f[41];
        }
        if 1.5 <= f[18] {
            if 0.5 > f[20] {
                return false;
            }
            if 0.5 <= f[19] {
                if 0.449972 <= f[41] {
                    return false;
                }
                if 0.118876 <= f[21] {
                    return !(5.5 <= f[11]);
                }
                return !(41.535 <= f[0]);
            }
            return !(0.0112894 <= f[1]);
        }
        if 1.5 > f[14] {
            return false;
        }
        if 0.0491163 <= f[5] {
            return !(0.196468 > f[22]);
        }
        !(1.5 <= f[20])
    }

    /// 0x4a9890: every region's features (over the edge images of the
    /// prepared image and its Gaussian blur), the tree's verdict into
    /// flag 1; the features stay at engine+0x9cc for the refinement.
    pub(super) fn sub_classify(&mut self) {
        let blurred = Self::gaussian_blur(self.image, self.width, self.height, 0);
        let edge = self.edge_image(&blurred);
        let features = self.region_features(&edge);
        for (i, f) in features.iter().enumerate() {
            if Self::is_anti_aliasing_region(f) {
                self.sub.regions[i].flags |= 1;
            }
        }
        self.sub.features = features;
    }

    /// 0x4ae1f0: the second decision tree, over a region's features after
    /// the refinement, that confirms an anti-aliasing region.
    pub(super) fn confirms_anti_aliasing(f: &[f64; 42], type_code: i32) -> bool {
        if 3.57554 <= f[37] {
            return 0.00258463 > f[35];
        }
        if 0.0442212 <= f[28] {
            if type_code == 0 {
                return true;
            }
            if 0.694472 <= f[34] {
                return false;
            }
            return !(3.09081 > f[0]);
        }
        if 0.00627504 <= f[39] {
            return true;
        }
        if type_code == 1 || type_code == 2 {
            return true;
        }
        !(0.934367 > f[4])
    }

    /// 0x48a540 with 0x48a440: among the four neighbours of pixel `q` that
    /// belong to an unflagged region, the one whose colour is closest to
    /// the pixel's; the squared distance and that neighbour's label.
    fn closest_unflagged_neighbour(&self, labels: &[i32], q: i32) -> Option<(f32, i32)> {
        let (w, h) = (self.width as i32, self.height as i32);
        let x = q % w;
        let y = q / w;
        let c = self.pixel_floats(q as usize);
        let mut best = 1.0e10f32;
        let mut label = -1;
        let mut found = false;
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            if nx < 0 || nx >= w || ny < 0 || ny >= h {
                continue;
            }
            let p = (ny * w + nx) as usize;
            let nlab = labels[p];
            if self.sub.regions[nlab as usize].flags & 1 != 0 {
                continue;
            }
            let n = self.pixel_floats(p);
            let d = [n[0] - c[0], n[1] - c[1], n[2] - c[2], n[3] - c[3]];
            let dist = ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]) + d[3] * d[3];
            if best > dist {
                best = dist;
                label = nlab;
                found = true;
            }
        }
        found.then_some((best, label))
    }

    /// 0x48c6f0(stage, 0): the sub-pixel segmentation re-initialised over
    /// the current labels without resetting colour indices (the records
    /// are only reallocated when the region count changed).
    fn sub_reinit(&mut self) {
        self.sub.cm = ColourModel::new();
        let n = (self.labels.max + 1) as usize;
        if self.sub.regions.len() != n {
            self.sub.regions = vec![SubRegion::default(); n];
        }
        self.sub.cost = vec![0.0; self.width * self.height];
        self.sub_rebuild(false, false);
        self.sub.renumbered = true;
    }

    /// 0x4a9d10: the refinement of the regions flag 1 marked. Each such
    /// region is eroded pixel by pixel into the neighbouring unflagged
    /// region whose adjacent pixel is closest in colour (the pixel with the
    /// smallest distance first); the model cost of the region's own and
    /// outside pixels before (from the cached costs) and after the erosion
    /// become features 33 to 38; the second tree then decides per region
    /// whether the erosion stands or its pixels return to it; finally the
    /// segmentation is rebuilt and every region's flags recomputed from
    /// both trees.
    pub(super) fn sub_refine(&mut self, type_code: i32) {
        let n = (self.labels.max + 1) as usize;
        let mut map = vec![-1i32; n];
        let mut count = 0;
        for (m, region) in map.iter_mut().zip(self.sub.regions.iter()) {
            if region.flags & 1 != 0 {
                *m = count;
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        let f = count as usize;
        let mut labels_of = vec![0i32; f];
        let mut own_cost = vec![0.0f32; f];
        let mut after_cost = vec![0.0f32; f];
        let mut outside: Vec<Vec<i32>> = vec![Vec::new(); f];
        let mut pixels: Vec<Vec<i32>> = vec![Vec::new(); f];
        self.sub_rebuild(true, true);
        for (i, &m) in map.iter().enumerate().take(n) {
            if m < 0 {
                continue;
            }
            let m = m as usize;
            labels_of[m] = i as i32;
            let interior = self.interior_cost(i as i32, -1);
            own_cost[m] = (interior + own_cost[m] as f64) as f32;
            let set: Vec<i32> = self.sub.regions[i].set.iter().copied().collect();
            for q in set {
                let lab = self.labels.data[q as usize];
                own_cost[m] += self.sub.cost[q as usize];
                if lab != i as i32 {
                    outside[m].push(q);
                }
            }
        }
        let w = self.width as i32;
        for p in 0..self.labels.data.len() {
            let m = map[self.labels.data[p] as usize];
            if m >= 0 {
                pixels[m as usize].push(p as i32);
            }
        }
        let mut labels = std::mem::take(&mut self.labels.data);
        let height = self.height as i32;
        for pixel_list in pixels.iter().take(f) {
            erode(&mut labels, w, height, pixel_list, |labels, q| {
                self.closest_unflagged_neighbour(labels, q)
            });
        }
        self.labels.data = labels;
        let snapshot = self.labels.clone();
        self.sub_renumber();
        self.sub_reinit();
        self.sub_rebuild(true, true);
        for m in 0..f {
            for &q in outside[m].iter().chain(pixels[m].iter()) {
                let c = self.pixel_cost(q % w, q / w, -1, -1);
                after_cost[m] = (c + after_cost[m] as f64) as f32;
            }
        }
        self.labels = snapshot;
        let mut restore = Vec::new();
        for m in 0..f {
            let x = own_cost[m];
            let y = after_cost[m];
            let total = (pixels[m].len() + outside[m].len()) as i32 as f32;
            let inv = 1.0f32 / total;
            let i = labels_of[m] as usize;
            let rec = &mut self.sub.features[i];
            rec[33] = x as f64;
            rec[34] = y as f64;
            rec[35] = inv as f64 * x as f64;
            rec[36] = inv as f64 * y as f64;
            let t = y as f64 - x as f64;
            rec[37] = t;
            rec[38] = t / total as f64;
            if !Self::confirms_anti_aliasing(rec, type_code) {
                restore.push(m);
            }
        }
        for m in restore {
            for &q in &pixels[m] {
                self.labels.data[q as usize] = labels_of[m];
            }
        }
        self.sub_renumber();
        self.sub_reinit();
        let zero = [0.0f64; 42];
        for i in 0..=self.labels.max as usize {
            let rec = self.sub.features.get(i).copied().unwrap_or(zero);
            let mut flags = 0u8;
            if Self::is_anti_aliasing_region(&rec) {
                flags |= 1;
            }
            if map.get(i).copied().unwrap_or(-1) >= 0
                && Self::confirms_anti_aliasing(&rec, type_code)
            {
                flags |= 2;
            }
            self.sub.regions[i].flags = flags;
        }
    }

    /// 0x4a7eb0 with 0x48c5b0: the sub-pixel state released.
    pub(super) fn sub_cleanup(&mut self) {
        self.sub.visited.clear();
        self.sub.renumbered = false;
        self.sub.regions.clear();
    }

    /// The anti-aliased path of 0x488d20 one stage at a time (the order of
    /// `--segmentation-aa-stages`): 1 the super-pixel pass, 2 the sub-pixel
    /// segmentation, 3 the recolouring, 4 the quantisation, 5 the region
    /// classification, 6 the refinement, 7 and 8 recolouring and
    /// quantisation again, 9 the cleanup with the final small-region merge.
    pub(super) fn aa_stage(&mut self, stage: u32) -> Result<(), String> {
        match stage {
            1 => self.run(false),
            2 => self.sub_run(),
            3 | 7 => self.sub_recolour(),
            4 | 8 => self.sub_quantise(),
            5 => self.sub_classify(),
            6 => {
                let code = self.type_code();
                self.sub_refine(code)
            }
            9 => {
                self.sub_cleanup();
                self.finish_small();
            }
            _ => return Err(format!("no stage {stage}")),
        }
        Ok(())
    }

    /// The state the oracle emits after a stage.
    #[cfg(test)]
    pub(super) fn aa_snapshot(&self) -> AaSnapshot {
        let max = self.labels.data.iter().copied().max().unwrap_or(-1);
        AaSnapshot {
            labels: self.labels.data.clone(),
            max_label: max,
            label_count: self.labels.count,
            parents: (0..=max)
                .map(|i| self.labels.parent.get(i as usize).copied().unwrap_or(-9))
                .collect(),
            regions: (0..=max)
                .filter_map(|i| self.sub.regions.get(i as usize))
                .map(|r| (r.count, r.interior, r.colour, r.flags, r.colour_floats))
                .collect(),
        }
    }
}
