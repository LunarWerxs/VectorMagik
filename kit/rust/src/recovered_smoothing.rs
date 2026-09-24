//! Contour smoothing: the original engine's `ContourSmoother` (entry
//! 0x497c30, the object at engine+0x1888) recovered from the disassembly.
//!
//! The smoother takes the shared node set and the closed contours that
//! contour construction (`recovered_topology`) leaves and moves the node
//! positions by minimising an energy, in up to three phases with their own
//! parameters (`ContourSmoother::phase_*` in the presets):
//!
//! * the **prior** at every node of a contour (0x494730 for prior code 3,
//!   the code every preset uses; 0x493ee0 for code 0, which small regions of
//!   at most seven pixels get instead): with `a` the edge arriving at the
//!   node and `b` the edge leaving it, `na`/`nb` their pixel step counts
//!   from contour construction, code 3 is
//!   `sqrt(2 - 2 cos + 0.001) + w_len * (|b|/nb - |a|/na)^2` and code 0 is
//!   `|b/nb - a/na|^2`; nodes flagged as corners (edge entry word 0, bit 0)
//!   get no prior; the sum is scaled by `prior_strengths[phase]`;
//! * **air pressure** for regions of at most seven pixels:
//!   `prior_strength * air_pressure_weight * (signed polygon area - pixels)^2`,
//!   the area recomputed by 0x47ee30 into the contour record;
//! * the **measurement** term: for measurement type 0 the squared
//!   displacement of every node from the float position contour
//!   construction gave it; type 2 penalises only the part beyond 0.7 px;
//!   type 1 (anti-aliased presets) is the image model in `anti_aliased`.
//!
//! The minimiser is the engine's `Optimizer` (0x4ada10, at smoother+0xb8):
//! Fletcher-Reeves conjugate gradients over all node coordinates with a
//! restart every `cg_iter_for_restart` iterations or whenever the direction
//! stops descending, a stall counter against `cg_knock_out_count_down` and
//! `cg_min_iter`, and the quadratic line search 0x4ad010 (line search types 0
//! and 5, the only ones a preset selects): the energy at 0, eps and 2 eps
//! along the direction, a step to the fitted parabola's minimum bounded by
//! `cg_max_step_size` over the largest direction component, the best of the
//! four kept, eps halved each round and re-derived from the step taken. After
//! every move (0x493640) the border nodes are pinned back onto the canvas
//! edges (0x47e480) and nodes within 1.1 px of the border are kept 0.01 px
//! inside it (0x47f2d0); the gradient zeroes the pinned components.
//!
//! After a phase with `do_puncture_corners` set, 0x494a90 walks every contour
//! of at least seven nodes with a window of seven nodes, computes the five
//! turn values `2 - 2 u_i . u_{i+1}` of the six unit edges and a fixed
//! decision tree (0x4adfb0, thirteen constants over the middle turn, the
//! sums of the side turns, the smallest side turn and whether the middle
//! turn is the largest) decides whether the middle node is a corner: node
//! state 3 and bit 0 of its edge entry, which the later phases and the
//! fitter respect.
//!
//! Anti-aliased presets (`Shared::is_anti_aliased`, measurement type 1) add
//! the neighbour lists and inversion checks of the node set, the sub-pixel
//! placement from the colour model with a random perturbation, the image
//! measurement term and the restart hook; they live in `anti_aliased` and
//! run when `smooth` is given the source image.
//!
//! Floating point follows the original operation order so that the f64
//! results reproduce the x87 code, which runs at 53-bit precision.

use std::fmt;

mod anti_aliased;
mod kd;

pub use anti_aliased::AaImage;
use anti_aliased::AaState;

/// A node record as the smoother sees it (stride 0x28).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub x: f64,
    pub y: f64,
    /// The float copies at +0x10/+0x14: the position contour construction
    /// wrote, the anchor of the measurement term.
    pub fx: f32,
    pub fy: f32,
    /// +0x1c: 3 at a corner.
    pub state: u8,
    /// +0x1d: 0x80 on the canvas border; 0x10 selects the extra code-0 prior.
    pub flags: u8,
    /// +0x1e: the count weighting the extra measurement term (anti-aliased
    /// presets set it; zero otherwise).
    pub aux: u8,
}

/// One twelve-byte edge entry of a contour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edge {
    /// Word 0: bit 0 marks a corner, bit 3 a local maximum of the turn.
    pub flag: i32,
    /// Word 1: the label across the edge leaving the node.
    pub other: i32,
    /// Word 2: unit steps of the edge arriving at the node.
    pub steps: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contour {
    /// Word 0: the region's pixel count, as segmentation left it.
    pub pixels: i32,
    /// +0x8: the signed polygon area the energy keeps for small regions.
    pub area: f64,
    pub nodes: Vec<i32>,
    pub edges: Vec<Edge>,
    /// +0x10: the region id (the colour model's key); anti-aliased only.
    pub region: i32,
    /// +0x14: the region colour bytes (BGRA); anti-aliased only.
    pub color: [u8; 4],
    /// +0x30: the enclosing contour, -1 for none; anti-aliased only.
    pub parent: i32,
    /// +0x48/+0x50: the orientation ray of the last inversion pass.
    pub dir: [f64; 2],
    /// +0x58: 2 minus the ray's crossings, 0 for a consistent contour.
    pub parity: i32,
}

/// What the anti-aliased half needs beyond the records: the source image,
/// the engine generator's seed at entry, the node-set pass period
/// (smoother+0xb0) and the two diagonal unit vectors (smoother+0x620).
#[derive(Clone, Copy, Debug)]
pub struct AaInput<'a> {
    pub image: &'a AaImage,
    pub seed: i32,
    pub every: i32,
    pub units: [f64; 4],
    /// The entry's argument: run the placement and perturbation.
    pub prepare: bool,
}

/// The node set's canvas: size and how many nodes sit on the left, right,
/// top and bottom edges (numbered first, in that order).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    pub border: [i32; 4],
}

impl Canvas {
    /// The border nodes, which come first; the interior starts here.
    pub fn border_total(&self) -> usize {
        self.border.iter().map(|&b| b.max(0) as usize).sum()
    }

    /// 0x47e480's pattern over the first `n` nodes: every pinned component
    /// as (node, axis, value), axis 0 for x and 1 for y, in the order the
    /// original writes them (a later entry can overwrite an earlier one):
    /// node 0's y, the left run's x, the y of the last left node and of the
    /// first right node, the right run's x, the last right node's y, then
    /// the y of the top and the bottom runs. `state_changed` writes the
    /// values; the gradient zeroes the same components.
    fn border_pins(&self, n: usize, mut pin: impl FnMut(usize, usize, f64)) {
        let [left, right, top, bottom] = self.border;
        let w = self.width as f64;
        let h = self.height as f64;
        if n > 0 {
            pin(0, 1, 0.0);
        }
        let mut at = 0usize;
        for _ in 0..left.max(0) {
            if at < n {
                pin(at, 0, 0.0);
            }
            at += 1;
        }
        if at >= 1 && at - 1 < n {
            pin(at - 1, 1, h);
        }
        if at < n {
            pin(at, 1, 0.0);
        }
        for _ in 0..right.max(0) {
            if at < n {
                pin(at, 0, w);
            }
            at += 1;
        }
        if at >= 1 && at - 1 < n {
            pin(at - 1, 1, h);
        }
        for _ in 0..top.max(0) {
            if at < n {
                pin(at, 1, 0.0);
            }
            at += 1;
        }
        for _ in 0..bottom.max(0) {
            if at < n {
                pin(at, 1, h);
            }
            at += 1;
        }
    }
}

/// One phase's optimizer block (0xc8 bytes at smoother+0x208 + 0xc8 * phase,
/// copied to optimizer+0x8 by 0x4936c0). Field order is the block's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CgParams {
    pub iter_for_restart: i32,
    pub min_iter: i32,
    pub max_iter: i32,
    pub knock_out_count_down: i32,
    /// +0x10, unregistered, constructed 1: save the state before a line
    /// search and step from the saved state when the parabola loses.
    pub backup_state: i32,
    pub use_eps_from_step: i32,
    pub min_eps: f64,
    pub max_eps: f64,
    /// +0x28, unregistered, constructed 0.5: eps is multiplied by it after
    /// every line-search round.
    pub eps_decay: f64,
    pub step_fraction: f64,
    /// +0x38, unregistered: the finite-difference step, unused with an
    /// analytic gradient.
    pub fd_eps: f64,
    /// +0x40, unregistered, constructed 0.
    pub reserved: i32,
    pub num_blind_steps: i32,
    pub num_seeing_steps: i32,
    pub blind_blend_latest: f64,
    pub rel_tol: f64,
    pub abs_tol: f64,
    pub line_search_type: i32,
    pub quad_line_srch_rel_tol: f64,
    pub quad_line_srch_max_iter: i32,
    pub max_step_size: f64,
}

/// The smoother's registered parameters (PARAMETERS.md).
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub measurement_types: [i32; 3],
    pub prior_types: [i32; 3],
    pub prior_strengths: [f64; 3],
    pub length_penalty_weights: [f64; 3],
    pub do_puncture_corners: [i32; 3],
    pub is_enabled: [i32; 3],
    pub optimizer_type: [i32; 3],
    pub air_pressure_weight: f64,
    pub perturbation_range: f64,
    pub length_barrier_weight: f64,
    pub puncture_flatness_thresh: f64,
    pub puncture_corner_thresh: f64,
    pub anti_inv_pot_meas_scale: f64,
    pub anti_inv_pot_prior_scale: f64,
    pub phases: [CgParams; 3],
}

/// The number of values `Params::from_values` reads: the smoother's seven
/// registered triples, seven doubles and three optimizer blocks of 21.
/// Only the fixture parsers of the tests decode such rows; a conversion
/// builds its parameters in `recovered_pipeline::smoothing_params`.
#[cfg(test)]
pub const PARAM_VALUES: usize = 7 * 3 + 7 + 3 * 21;

/// What the code-3 prior and its gradient share about the edge vectors
/// a = c - p and b = n - c: their lengths, the reciprocals and the unit vectors.
struct ThreeNodeFrame {
    la: f64,
    lb: f64,
    ila: f64,
    ilb: f64,
    uax: f64,
    uay: f64,
    ubx: f64,
    uby: f64,
}

#[cfg(test)]
impl CgParams {
    /// One optimizer block from 21 values in the block's field order.
    pub fn from_values(v: &[f64]) -> Result<CgParams, SmoothingError> {
        if v.len() < 21 {
            return Err("Optimizer block needs 21 values".into());
        }
        let i = |k: usize| v[k] as i32;
        Ok(CgParams {
            iter_for_restart: i(0),
            min_iter: i(1),
            max_iter: i(2),
            knock_out_count_down: i(3),
            backup_state: i(4),
            use_eps_from_step: i(5),
            min_eps: v[6],
            max_eps: v[7],
            eps_decay: v[8],
            step_fraction: v[9],
            fd_eps: v[10],
            reserved: i(11),
            num_blind_steps: i(12),
            num_seeing_steps: i(13),
            blind_blend_latest: v[14],
            rel_tol: v[15],
            abs_tol: v[16],
            line_search_type: i(17),
            quad_line_srch_rel_tol: v[18],
            quad_line_srch_max_iter: i(19),
            max_step_size: v[20],
        })
    }
}

#[cfg(test)]
impl Params {
    /// The parameters from `PARAM_VALUES` values as the smoother object
    /// holds them: measurement types, prior types, prior strengths, length
    /// penalty weights, corner puncturing, enabled phases, optimizer types
    /// (three each), the seven doubles from +0x78 (air pressure weight,
    /// perturbation range, length barrier weight, puncture flatness and
    /// corner thresholds, the two anti-inverse-potential scales), then the
    /// three optimizer blocks.
    pub fn from_values(v: &[f64]) -> Result<Params, SmoothingError> {
        if v.len() < PARAM_VALUES {
            return Err(format!("Smoother parameters need {PARAM_VALUES} values").into());
        }
        let t3i = |k: usize| [v[k] as i32, v[k + 1] as i32, v[k + 2] as i32];
        let t3 = |k: usize| [v[k], v[k + 1], v[k + 2]];
        Ok(Params {
            measurement_types: t3i(0),
            prior_types: t3i(3),
            prior_strengths: t3(6),
            length_penalty_weights: t3(9),
            do_puncture_corners: t3i(12),
            is_enabled: t3i(15),
            optimizer_type: t3i(18),
            air_pressure_weight: v[21],
            perturbation_range: v[22],
            length_barrier_weight: v[23],
            puncture_flatness_thresh: v[24],
            puncture_corner_thresh: v[25],
            anti_inv_pot_meas_scale: v[26],
            anti_inv_pot_prior_scale: v[27],
            phases: [
                CgParams::from_values(&v[28..49])?,
                CgParams::from_values(&v[49..70])?,
                CgParams::from_values(&v[70..91])?,
            ],
        })
    }
}

/// What a run did, for tests and counters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Report {
    pub iterations: [i32; 3],
    pub line_search_iterations: [i32; 3],
    pub restarts: [i32; 3],
    pub final_energy: [f64; 3],
    pub corners: i32,
    /// The generator's seed after the run (unchanged without anti-aliasing).
    pub seed: i32,
    /// Nodes the closing node-set pass marked (anti-aliased only).
    pub marked: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SmoothingError(pub String);
impl fmt::Display for SmoothingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SmoothingError {}
impl From<String> for SmoothingError {
    fn from(s: String) -> Self {
        SmoothingError(s)
    }
}
impl From<&str> for SmoothingError {
    fn from(s: &str) -> Self {
        SmoothingError(s.to_string())
    }
}

/// 1.1f widened, the near-border band of 0x482490.
const NEAR_BORDER: f64 = 1.100000023841858;
/// The clamp inside the canvas of 0x47f2d0.
const INSET: f64 = 0.01;
/// The dead zone of measurement type 2.
const DEAD_ZONE: f64 = 0.7;
/// Below this the Fletcher-Reeves ratio is not formed (0x8dfea0).
const TINY: f64 = 1e-30;
/// Regions of at most this many pixels use the code-0 prior and air pressure.
const SMALL_REGION: i32 = 7;

/// The engine's linear congruential generator 0x468bc0 (seed at 0xa21d2c):
/// `seed = seed * 2003 mod 9973`, value `seed * 0.0001002707309736288`.
pub fn native_random(seed: &mut i32) -> f64 {
    *seed = (*seed).wrapping_mul(0x7d3) % 0x26f5;
    *seed as f64 * 0.0001002707309736288
}

struct Smoother<'a> {
    nodes: &'a mut [Node],
    contours: &'a mut [Contour],
    canvas: Canvas,
    params: &'a Params,
    phase: usize,
    /// Interior nodes within the near-border band (node set +0x3c).
    near: Vec<usize>,
    grad: Vec<[f64; 2]>,
    /// The optimizer's remembered blind step (optimizer+0x138): line search
    /// type 5 blends it from the quadratic searches' steps and keeps it
    /// across phases; the engine's zeroed allocation starts it at 0.
    blind_len: f64,
    /// The anti-aliased half's state, present for presets 3, 4 and 5.
    aa: Option<AaState<'a>>,
}

impl<'a> Smoother<'a> {
    /// The previous, current and next node of position `k` of contour `ci`
    /// and the step counts of the edges arriving and leaving the current
    /// node (smoother+0x658 / +0x65c).
    fn triple(&self, ci: usize, k: usize) -> (usize, usize, usize, i32, i32) {
        let c = &self.contours[ci];
        let n = c.nodes.len();
        let prev = c.nodes[(n + k - 1) % n] as usize;
        let cur = c.nodes[k] as usize;
        let next_k = (k + 1) % n;
        let next = c.nodes[next_k] as usize;
        (prev, cur, next, c.edges[k].steps, c.edges[next_k].steps)
    }

    /// 0x47f4f0: the node's displacement from its float position.
    fn displacement(&self, i: usize) -> (f64, f64) {
        let n = &self.nodes[i];
        (n.x - n.fx as f64, n.y - n.fy as f64)
    }

    /// 0x493ee0, prior code 0.
    fn prior0(&self, p: usize, c: usize, n: usize, na: i32, nb: i32) -> f64 {
        let (pp, cc, nn) = (&self.nodes[p], &self.nodes[c], &self.nodes[n]);
        let ax = cc.x - pp.x;
        let ay = cc.y - pp.y;
        let ina = 1.0 / na as f64;
        let vax = ina * ax;
        let vay = ina * ay;
        let bx = nn.x - cc.x;
        let by = nn.y - cc.y;
        let inb = 1.0 / nb as f64;
        let vbx = inb * bx;
        let vby = inb * by;
        let dx = vbx - vax;
        let dy = vby - vay;
        (dx * dx) + (dy * dy)
    }

    /// The two edge vectors at node `c` between `p` and `n`, their lengths,
    /// reciprocals and unit vectors, as 0x494730 and 0x494840 both set them up.
    fn prior3_setup(&self, p: usize, c: usize, n: usize) -> ThreeNodeFrame {
        let (pp, cc, nn) = (&self.nodes[p], &self.nodes[c], &self.nodes[n]);
        let ax = cc.x - pp.x;
        let ay = cc.y - pp.y;
        let bx = nn.x - cc.x;
        let by = nn.y - cc.y;
        let la2 = (ax * ax) + (ay * ay);
        let lb2 = (bx * bx) + (by * by);
        let la = la2.sqrt();
        let lb = lb2.sqrt();
        let ila = 1.0 / la;
        let uax = ax * ila;
        let uay = ay * ila;
        let ilb = 1.0 / lb;
        let ubx = bx * ilb;
        let uby = by * ilb;
        ThreeNodeFrame {
            la,
            lb,
            ila,
            ilb,
            uax,
            uay,
            ubx,
            uby,
        }
    }

    /// 0x494730, prior code 3.
    fn prior3(&self, p: usize, c: usize, n: usize, na: i32, nb: i32) -> f64 {
        let ThreeNodeFrame {
            la,
            lb,
            uax,
            uay,
            ubx,
            uby,
            ..
        } = self.prior3_setup(p, c, n);
        let dot = (ubx * uax) + (uby * uay);
        let t = (lb / nb as f64) - (la / na as f64);
        let s = ((2.0 - (dot + dot)) + 0.001).sqrt();
        let w = self.params.length_penalty_weights[self.phase];
        s + ((t * w) * t)
    }

    /// 0x494580: the code-0 prior's derivatives with respect to the previous
    /// and the next node.
    fn prior0_grad(&self, p: usize, c: usize, n: usize, na: i32, nb: i32) -> ([f64; 2], [f64; 2]) {
        let (pp, cc, nn) = (&self.nodes[p], &self.nodes[c], &self.nodes[n]);
        let bx = nn.x - cc.x;
        let by = nn.y - cc.y;
        let inb = 1.0 / nb as f64;
        let vbx = bx * inb;
        let vby = by * inb;
        let ax = cc.x - pp.x;
        let ay = cc.y - pp.y;
        let ina = 1.0 / na as f64;
        let vax = ax * ina;
        let vay = ay * ina;
        let dx = vbx - vax;
        let dy = vby - vay;
        let dx2 = dx + dx;
        let dy2 = dy + dy;
        let ga = [dx2 * ina, ina * dy2];
        let gb = [dx2 * inb, inb * dy2];
        (ga, gb)
    }

    /// 0x494840: the code-3 prior's derivatives.
    fn prior3_grad(&self, p: usize, c: usize, n: usize, na: i32, nb: i32) -> ([f64; 2], [f64; 2]) {
        let ThreeNodeFrame {
            la,
            lb,
            ila,
            ilb,
            uax,
            uay,
            ubx,
            uby,
        } = self.prior3_setup(p, c, n);
        let dot = (ubx * uax) + (uby * uay);
        let s = ((2.0 - (dot + dot)) + 0.001).sqrt();
        // d(dot)/da over |a|, and -d(dot)/db over |b|.
        let dax = (-(uax * dot) + ubx) * ila;
        let day = (-(uay * dot) + uby) * ila;
        let dbx = -((-(ubx * dot) + uax) * ilb);
        let dby = -((-(uby * dot) + uay) * ilb);
        let is = 1.0 / s;
        let mut ga = [dax * is, day * is];
        let mut gb = [dbx * is, dby * is];
        let na_f = na as f64;
        let t = (lb / nb as f64) - (la / na_f);
        let w = self.params.length_penalty_weights[self.phase];
        let cw = t * w;
        let c2 = cw + cw;
        let cax = uax * c2;
        let cay = uay * c2;
        let ina = 1.0 / na_f;
        ga[0] += cax * ina;
        ga[1] += ina * cay;
        let cbx = ubx * c2;
        let cby = uby * c2;
        let inb = 1.0 / nb as f64;
        gb[0] += inb * cbx;
        gb[1] += inb * cby;
        (ga, gb)
    }

    /// 0x47ee30: the signed polygon area, written into the contour.
    fn area(&mut self, ci: usize) -> f64 {
        let c = &self.contours[ci];
        let n = c.nodes.len();
        let mut area = 0.0f64;
        for j in 0..n {
            let q = &self.nodes[c.nodes[j] as usize];
            let p = &self.nodes[c.nodes[(j + 1) % n] as usize];
            area += (p.y * q.x) - (q.y * p.x);
        }
        area *= 0.5;
        self.contours[ci].area = area;
        area
    }

    /// 0x4941e0: the energy of the current phase.
    fn objective(&mut self) -> f64 {
        let _span = crate::profile::span("recovered_smoothing.objective");
        let p = self.phase;
        let ps = self.params.prior_strengths[p];
        let aip = self.params.anti_inv_pot_prior_scale / ps;
        let mut e = 0.0f64;
        for ci in 0..self.contours.len() {
            let n = self.contours[ci].nodes.len();
            for k in 0..n {
                let (prev, cur, next, na, nb) = self.triple(ci, k);
                if self.nodes[cur].flags & 0x10 != 0 {
                    e += self.prior0(prev, cur, next, na, nb) * aip;
                }
                if self.contours[ci].pixels <= SMALL_REGION {
                    e += self.prior0(prev, cur, next, na, nb);
                } else if self.contours[ci].edges[k].flag & 1 == 0 {
                    e += self.prior3(prev, cur, next, na, nb);
                }
            }
        }
        e *= ps;
        let air = ps * self.params.air_pressure_weight;
        for ci in 0..self.contours.len() {
            if self.contours[ci].pixels <= SMALL_REGION {
                let area = self.area(ci);
                let d = area - self.contours[ci].pixels as f64;
                e += (d * d) * air;
            }
        }
        match self.params.measurement_types[p] {
            0 => {
                for i in 0..self.nodes.len() {
                    let (dx, dy) = self.displacement(i);
                    e += (dx * dx) + (dy * dy);
                }
            }
            1 => {
                e += self.image_energy();
            }
            2 => {
                for i in 0..self.nodes.len() {
                    let (dx, dy) = self.displacement(i);
                    let r = ((dx * dx) + (dy * dy)).sqrt() - DEAD_ZONE;
                    let t = if r > 0.0 { r } else { 0.0 };
                    e += t * t;
                }
            }
            _ => {}
        }
        let aims = self.params.anti_inv_pot_meas_scale;
        for i in 0..self.nodes.len() {
            let aux = self.nodes[i].aux;
            if aux != 0 {
                let (dx, dy) = self.displacement(i);
                let sum = (dx * dx) + (dy * dy);
                e += (aux as f64 * aims) * sum;
            }
        }
        e
    }

    fn accumulate(&mut self, prev: usize, cur: usize, next: usize, ga: [f64; 2], gb: [f64; 2]) {
        let g = &mut self.grad;
        g[prev][0] += ga[0];
        g[prev][1] += ga[1];
        g[cur][0] -= ga[0];
        g[cur][1] -= ga[1];
        g[cur][0] -= gb[0];
        g[cur][1] -= gb[1];
        g[next][0] += gb[0];
        g[next][1] += gb[1];
    }

    /// 0x495b00: the analytic gradient into `grad`.
    fn gradient(&mut self) {
        let _span = crate::profile::span("recovered_smoothing.gradient");
        let p = self.phase;
        let ps = self.params.prior_strengths[p];
        let aip = self.params.anti_inv_pot_prior_scale / ps;
        for g in self.grad.iter_mut() {
            *g = [0.0, 0.0];
        }
        for ci in 0..self.contours.len() {
            let n = self.contours[ci].nodes.len();
            for k in 0..n {
                let (prev, cur, next, na, nb) = self.triple(ci, k);
                if self.nodes[cur].flags & 0x10 != 0 {
                    let (mut ga, mut gb) = self.prior0_grad(prev, cur, next, na, nb);
                    ga[0] *= aip;
                    ga[1] *= aip;
                    gb[0] *= aip;
                    gb[1] *= aip;
                    self.accumulate(prev, cur, next, ga, gb);
                }
                if self.contours[ci].pixels <= SMALL_REGION {
                    let (ga, gb) = self.prior0_grad(prev, cur, next, na, nb);
                    self.accumulate(prev, cur, next, ga, gb);
                } else if self.contours[ci].edges[k].flag & 1 == 0 {
                    let (ga, gb) = self.prior3_grad(prev, cur, next, na, nb);
                    self.accumulate(prev, cur, next, ga, gb);
                }
            }
        }
        for g in self.grad.iter_mut() {
            g[0] *= ps;
            g[1] *= ps;
        }
        let air = ps * self.params.air_pressure_weight;
        for ci in 0..self.contours.len() {
            let c = &self.contours[ci];
            if c.pixels > SMALL_REGION {
                continue;
            }
            let d = (c.area - c.pixels as f64) * air;
            let cc = d + d;
            let n = c.nodes.len();
            for k in 0..n {
                let cur = c.nodes[k] as usize;
                let nxt = c.nodes[(k + 1) % n] as usize;
                let (nx, ny) = (self.nodes[nxt].x, self.nodes[nxt].y);
                let (cx, cy) = (self.nodes[cur].x, self.nodes[cur].y);
                self.grad[cur][0] += (ny * 0.5) * cc;
                self.grad[cur][1] += (nx * -0.5) * cc;
                self.grad[nxt][0] += (cy * -0.5) * cc;
                self.grad[nxt][1] += (cx * 0.5) * cc;
            }
        }
        match self.params.measurement_types[p] {
            0 => {
                for i in 0..self.nodes.len() {
                    let (dx, dy) = self.displacement(i);
                    self.grad[i][0] += dx + dx;
                    self.grad[i][1] += dy + dy;
                }
            }
            1 => {
                self.image_gradient();
            }
            2 => {
                for i in 0..self.nodes.len() {
                    let (dx, dy) = self.displacement(i);
                    let r = ((dx * dx) + (dy * dy)).sqrt();
                    if r > DEAD_ZONE {
                        let f = 1.0 - (DEAD_ZONE / r);
                        let f = f + f;
                        self.grad[i][0] += dx * f;
                        self.grad[i][1] += dy * f;
                    }
                }
            }
            _ => {}
        }
        let aims = self.params.anti_inv_pot_meas_scale;
        for i in 0..self.nodes.len() {
            let aux = self.nodes[i].aux;
            if aux != 0 {
                let (dx, dy) = self.displacement(i);
                let f = aux as f64 * aims;
                let f = f + f;
                self.grad[i][0] += f * dx;
                self.grad[i][1] += f * dy;
            }
        }
        // The pinned components of the border nodes (0x47e480's pattern).
        let g = &mut self.grad;
        self.canvas
            .border_pins(g.len(), |i, axis, _| g[i][axis] = 0.0);
    }

    /// 0x493640: after every move, 0x47e480 pins the border nodes onto the
    /// canvas edges and 0x47f2d0 keeps the near-border nodes inside it.
    fn state_changed(&mut self) {
        let w = self.canvas.width as f64;
        let h = self.canvas.height as f64;
        let nodes = &mut self.nodes;
        self.canvas.border_pins(nodes.len(), |i, axis, value| {
            if axis == 0 {
                nodes[i].x = value;
            } else {
                nodes[i].y = value;
            }
        });
        let wi = w - INSET;
        let hi = h - INSET;
        for &i in &self.near {
            let node = &mut self.nodes[i];
            if INSET > node.x {
                node.x = INSET;
            } else if node.x > wi {
                node.x = wi;
            }
            if INSET > node.y {
                node.y = INSET;
            } else if node.y > hi {
                node.y = hi;
            }
        }
    }

    /// 0x482490: the interior nodes within the near-border band.
    fn find_near(&mut self) {
        let _span = crate::profile::span("recovered_smoothing.find_near");
        let start = self.canvas.border_total();
        let w = self.canvas.width as f64 - NEAR_BORDER;
        let h = self.canvas.height as f64 - NEAR_BORDER;
        self.near.clear();
        for i in start..self.nodes.len() {
            let (x, y) = (self.nodes[i].x, self.nodes[i].y);
            if NEAR_BORDER > x || x > w || NEAR_BORDER > y || y > h {
                self.near.push(i);
            }
        }
    }

    /// 0x494a90: mark corners after a phase.
    fn puncture(&mut self) -> Result<i32, SmoothingError> {
        let _span = crate::profile::span("recovered_smoothing.puncture");
        let mut corners = 0;
        for ci in 0..self.contours.len() {
            let n = self.contours[ci].nodes.len();
            if n < 7 {
                continue;
            }
            let ids: Vec<usize> = self.contours[ci]
                .nodes
                .iter()
                .map(|&i| i as usize)
                .collect();
            let mut window = [0usize; 7];
            for (j, w) in window.iter_mut().enumerate() {
                *w = ids[(n + j - 3) % n];
            }
            let mut units = [[0.0f64; 2]; 6];
            for j in 0..6 {
                units[j] = self.unit_edge(window[j], window[j + 1]);
            }
            let mut turns = [0.0f64; 5];
            for j in 0..5 {
                turns[j] = turn(&units[j], &units[j + 1]);
            }
            for k in 0..n {
                let (corner, local_max) = decide(&turns);
                if local_max {
                    self.contours[ci].edges[k].flag |= 8;
                }
                if corner {
                    self.nodes[window[3]].state = 3;
                    self.contours[ci].edges[k].flag |= 1;
                    corners += 1;
                }
                // Slide the window by one node.
                let newest = ids[(k + 4) % n];
                let unit = self.unit_edge(window[6], newest);
                window.rotate_left(1);
                window[6] = newest;
                units.rotate_left(1);
                units[5] = unit;
                turns.rotate_left(1);
                turns[4] = turn(&units[4], &units[5]);
            }
        }
        Ok(corners)
    }

    /// The unit vector from node `a` to node `b`, left unnormalised when the
    /// nodes coincide.
    fn unit_edge(&self, a: usize, b: usize) -> [f64; 2] {
        let ex = self.nodes[b].x - self.nodes[a].x;
        let ey = self.nodes[b].y - self.nodes[a].y;
        let len = ((0.0 + (ex * ex)) + (ey * ey)).sqrt();
        if len > 0.0 {
            let il = 1.0 / len;
            [il * ex, il * ey]
        } else {
            [ex, ey]
        }
    }
}

/// `2 - 2 u . v`, the turn between two unit edges.
fn turn(u: &[f64; 2], v: &[f64; 2]) -> f64 {
    let dot = (0.0 + (v[0] * u[0])) + (v[1] * u[1]);
    2.0 - (dot + dot)
}

/// The corner decision of 0x494a90 / 0x4adfb0 on the five turns around a
/// node: (is a corner, the middle turn is the largest).
fn decide(t: &[f64; 5]) -> (bool, bool) {
    let sum4 = (0.0 + (t[0] + t[4])) + (t[1] + t[3]);
    // The side with the smaller adjacent turn comes first.
    let (ta, tb, td) = if t[1] > t[3] {
        (t[4], t[3], t[0])
    } else {
        (t[0], t[1], t[4])
    };
    let t2 = t[2];
    let mut max = t[0];
    for &v in &t[1..] {
        if v > max {
            max = v;
        }
    }
    let local_max = t2 == max;
    let is_max = if local_max { 1.0 } else { 0.0 };
    let side = [t[0], t[1], t[3], t[4]];
    let mut min_side = side[0];
    for &v in &side[1..] {
        if min_side > v {
            min_side = v;
        }
    }
    let sum3 = ((0.0 + ta) + tb) + td;
    let corner = if 0.611194 > t2 {
        if 0.282594 > t2 {
            false
        } else if !(0.322933 > sum4) {
            false
        } else if 0.0286571 > sum4 {
            true
        } else if !(0.483023 > t2) {
            true
        } else {
            0.00209524 > sum3
        }
    } else if !(0.5 > is_max) {
        if !(0.097537 > min_side) {
            false
        } else if !(1.42496 > t2) {
            true
        } else if 0.122841 > sum3 {
            true
        } else if 0.445642 > sum4 {
            true
        } else {
            0.0046405 > td
        }
    } else {
        !(1.72002 > t2)
    };
    (corner, local_max)
}

/// The engine's `Optimizer` state (smoother+0xb8) for one phase.
struct Cg {
    p: CgParams,
    dir: Vec<f64>,
    backup: Vec<f64>,
    energy: f64,
    prev_best: f64,
    eps: f64,
    max_dir: f64,
    total: f64,
    beta: f64,
    stalled: i32,
    iteration: i32,
    restarts: i32,
    ls_iterations: i32,
    /// +0x140: where line search type 5 is in its seeing/blind cycle.
    phase_counter: i32,
}

impl Cg {
    fn state(s: &Smoother, i: usize) -> f64 {
        let node = &s.nodes[i / 2];
        if i.is_multiple_of(2) {
            node.x
        } else {
            node.y
        }
    }
    fn set_state(s: &mut Smoother, i: usize, v: f64) {
        let node = &mut s.nodes[i / 2];
        if i.is_multiple_of(2) {
            node.x = v;
        } else {
            node.y = v;
        }
    }
    fn grad(s: &Smoother, i: usize) -> f64 {
        s.grad[i / 2][i % 2]
    }

    /// 0x4ace50: move along the direction, then the state hook.
    fn shift(&mut self, s: &mut Smoother, alpha: f64) {
        for i in 0..self.dir.len() {
            let v = (alpha * self.dir[i]) + Self::state(s, i);
            Self::set_state(s, i, v);
        }
        s.state_changed();
    }

    /// 0x4ad530: direction back to steepest descent.
    fn restart(&mut self, s: &Smoother) {
        let _span = crate::profile::span("recovered_smoothing.restart");
        self.beta = 0.0;
        for i in 0..self.dir.len() {
            self.dir[i] = -Self::grad(s, i);
        }
        self.restarts += 1;
    }

    /// 0x4ad010: the quadratic line search along the direction.
    fn line_search(&mut self, s: &mut Smoother) -> f64 {
        let _span = crate::profile::span("recovered_smoothing.line_search");
        let p = self.p;
        if p.max_step_size > 0.0 && (self.max_dir * self.eps) > p.max_step_size {
            self.eps = p.max_step_size / self.max_dir;
        }
        let eps0 = self.eps;
        self.total = 0.0;
        let mut iter = 0;
        let mut e0;
        loop {
            if p.backup_state != 0 {
                for i in 0..self.backup.len() {
                    self.backup[i] = Self::state(s, i);
                }
            }
            e0 = self.energy;
            let eps = self.eps;
            self.shift(s, eps);
            let e1 = s.objective();
            self.energy = e1;
            self.shift(s, eps);
            let e2 = s.objective();
            self.energy = e2;
            let c2 = ((e2 - (e1 + e1)) + e0) / ((eps * eps) + (eps * eps));
            let extra = if c2 > 0.0 {
                let m = (e2 - e0) / (eps + eps);
                let ae = c2 * eps;
                let q = m + (ae + ae);
                q * (-1.0 / (c2 + c2))
            } else {
                0.0
            };
            let mut extra = extra;
            if p.max_step_size > 0.0 {
                if (extra * self.max_dir) > p.max_step_size {
                    extra = p.max_step_size / self.max_dir;
                }
                if -(extra * self.max_dir) > p.max_step_size {
                    extra = (-1.0 / self.max_dir) * p.max_step_size;
                }
            }
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            let e3 = if !(extra == 0.0) {
                self.shift(s, extra);
                let e = s.objective();
                self.energy = e;
                e
            } else {
                e0 + 1.0
            };
            let mut best = e0;
            let mut which = 0i32;
            if e0 > e1 {
                best = e1;
                which = 1;
            }
            if best > e2 {
                best = e2;
                which = 2;
            }
            let taken = if best > e3 {
                (eps + eps) + extra
            } else {
                let target = which as f64 * eps;
                if p.backup_state != 0 {
                    for i in 0..self.backup.len() {
                        Self::set_state(s, i, self.backup[i]);
                    }
                    self.shift(s, target);
                } else {
                    self.shift(s, -(((eps + eps) + extra) - target));
                }
                self.energy = s.objective();
                target
            };
            self.total += taken;
            self.eps *= p.eps_decay;
            if p.min_eps > self.eps {
                self.eps = p.min_eps;
            }
            if self.eps > p.max_eps {
                self.eps = p.max_eps;
            }
            iter += 1;
            if e0 == 0.0 {
                break;
            }
            let rel = (e0 - self.energy) / e0;
            if rel <= p.quad_line_srch_rel_tol {
                break;
            }
            if iter >= p.quad_line_srch_max_iter {
                break;
            }
        }
        if p.use_eps_from_step != 0 {
            let mut eps = self.total.abs() * p.step_fraction;
            if p.min_eps > eps {
                eps = p.min_eps;
            }
            if eps > p.max_eps {
                eps = p.max_eps;
            }
            self.eps = eps;
        } else {
            self.eps = eps0;
        }
        self.ls_iterations += iter;
        self.energy - e0
    }

    /// 0x4ad3e0, line search type 5: a cycle of `cg_num_seeing_steps`
    /// quadratic searches followed by `cg_num_blind_steps` blind steps of the
    /// remembered length (0.8 of it, clamped to `cg_max_step_size`), a blind
    /// step that raises the energy being undone and the cycle restarted; the
    /// remembered length blends each quadratic search's step with the last
    /// (`cg_blind_blend_latest`).
    fn line_search5(&mut self, s: &mut Smoother) -> f64 {
        let _span = crate::profile::span("recovered_smoothing.line_search5");
        let e0 = self.energy;
        let period = self.p.num_seeing_steps + self.p.num_blind_steps;
        let mut phase = self.phase_counter % period;
        self.phase_counter = phase;
        if !(phase < self.p.num_seeing_steps) {
            for i in 0..self.backup.len() {
                self.backup[i] = Self::state(s, i);
            }
            let mut step = s.blind_len;
            let max_step = self.p.max_step_size;
            if max_step > 0.0 {
                if (step * self.max_dir) > max_step {
                    step = max_step / self.max_dir;
                }
                if -(step * self.max_dir) > max_step {
                    step = -(max_step / self.max_dir);
                }
            }
            step *= 0.8;
            self.shift(s, step);
            let e = s.objective();
            self.energy = e;
            if !(e <= e0) {
                for i in 0..self.backup.len() {
                    Self::set_state(s, i, self.backup[i]);
                }
                self.energy = e0;
                self.phase_counter = 0;
                phase = 0;
            }
        }
        if phase < self.p.num_seeing_steps {
            self.line_search(s);
            let blend = self.p.blind_blend_latest;
            let kept = (1.0 - blend) * s.blind_len;
            s.blind_len = if self.iteration != 0 {
                (self.total * blend) + kept
            } else {
                self.total
            };
        }
        self.phase_counter += 1;
        self.energy - e0
    }

    /// 0x4ad770: the largest direction component, then the line search.
    fn step(&mut self, s: &mut Smoother) -> Result<(), SmoothingError> {
        if self.p.max_step_size > 0.0 {
            let mut m = self.dir[0].abs();
            for &d in &self.dir[1..] {
                let a = d.abs();
                if a > m {
                    m = a;
                }
            }
            self.max_dir = m;
        }
        match self.p.line_search_type {
            0 => {
                self.line_search(s);
                Ok(())
            }
            5 => {
                self.line_search5(s);
                Ok(())
            }
            other => Err(format!("Line search type {other} is not ported").into()),
        }
    }

    /// 0x4adf00 then 0x4ada10: a fresh conjugate-gradient run.
    fn run(&mut self, s: &mut Smoother) -> Result<(), SmoothingError> {
        let p = self.p;
        self.eps = p.max_eps;
        self.total = 0.0;
        self.iteration = 0;
        self.ls_iterations = 0;
        self.restarts = 0;
        self.stalled = 0;
        self.phase_counter = 0;
        let n = self.dir.len();
        self.energy = s.objective();
        s.gradient();
        let mut gg = 0.0f64;
        for i in 0..n {
            self.dir[i] = -Self::grad(s, i);
            gg += self.dir[i] * self.dir[i];
        }
        self.prev_best = self.energy;
        if p.max_iter <= 0 {
            self.energy = s.objective();
            return Ok(());
        }
        loop {
            if self.prev_best > self.energy {
                self.prev_best = self.energy;
            }
            self.step(s)?;
            // 0x4939a0, the after-iteration hook: with anti-aliasing the
            // node-set pass (every `every`th iteration the whole set, else
            // the contours the measurement flagged) and, when it marked
            // anything, a steepest-descent restart with the energy
            // re-evaluated (0x4ad530 with argument 0, then 0x493610).
            if s.aa.is_some() {
                let every = s.aa.as_ref().map(|a| a.every).unwrap_or(0);
                let marked = if every > 0 && self.iteration % every == every - 1 {
                    s.node_set_pass() > 0
                } else {
                    s.changed_pass()
                };
                if marked {
                    self.restart(s);
                    self.energy = s.objective();
                    self.prev_best = self.energy;
                }
            }
            let delta = self.prev_best - self.energy;
            let rel = if self.prev_best > 0.0 {
                delta / self.prev_best
            } else {
                0.0
            };
            let stall = if p.rel_tol > rel {
                true
            } else if p.abs_tol > delta {
                delta >= 0.0
            } else {
                false
            };
            if stall {
                self.stalled += 1;
                if self.stalled > p.knock_out_count_down && self.iteration > p.min_iter {
                    break;
                }
            } else {
                self.stalled = 0;
            }
            s.gradient();
            let gg_old = gg;
            let mut gg_new = 0.0f64;
            for i in 0..n {
                let g = Self::grad(s, i);
                gg_new += g * g;
            }
            gg = gg_new;
            self.beta = if TINY > gg_old.abs() {
                0.0
            } else {
                gg_new / gg_old
            };
            let mut descent = 0.0f64;
            for i in 0..n {
                let g = Self::grad(s, i);
                self.dir[i] = (self.beta * self.dir[i]) - g;
                descent -= g * self.dir[i];
            }
            if 0.0 >= descent || self.iteration % p.iter_for_restart == 0 {
                self.restart(s);
            }
            self.iteration += 1;
            if self.iteration >= p.max_iter {
                break;
            }
        }
        self.energy = s.objective();
        Ok(())
    }
}

/// Run the smoother over `nodes` and `contours` as 0x497c30 does: the node
/// positions, states, the contours' corner flags and areas are updated in
/// place. With `aa` (an anti-aliased preset) the flags, the anti-inversion
/// counts and the contours' orientation words change too, and the report
/// carries the generator's seed.
pub fn smooth(
    nodes: &mut [Node],
    contours: &mut [Contour],
    canvas: Canvas,
    params: &Params,
    aa: Option<AaInput<'_>>,
) -> Result<Report, SmoothingError> {
    if let Some(input) = &aa {
        AaState::validate(input.image, contours.len())?;
        if input.image.width != canvas.width || input.image.height != canvas.height {
            return Err("Anti-aliased image does not match the canvas".into());
        }
        if input.every <= 0 {
            return Err("The node-set pass period must be positive".into());
        }
        for (ci, c) in contours.iter().enumerate() {
            if c.region < 0 || c.region as usize >= input.image.region_colors.len() {
                return Err(format!("Contour {ci}: region id outside the colour table").into());
            }
            if c.parent >= contours.len() as i32 {
                return Err(format!("Contour {ci}: enclosing contour outside the records").into());
            }
            if c.nodes.len() < 2 {
                return Err(format!("Contour {ci}: fewer than two nodes").into());
            }
        }
    }
    if canvas.width < 2 || canvas.height < 2 {
        return Err("Canvas smaller than two pixels".into());
    }
    if canvas.border.iter().any(|&b| b < 0) {
        return Err("Negative border count".into());
    }
    if nodes.len() < 2 {
        return Err("At least 2 state variables required".into());
    }
    if nodes.len() > i32::MAX as usize / 2 {
        return Err("Too many nodes".into());
    }
    for (ci, c) in contours.iter().enumerate() {
        if c.edges.len() != c.nodes.len() {
            return Err(format!("Contour {ci}: edge entries do not match its nodes").into());
        }
        for &id in &c.nodes {
            if id < 0 || id as usize >= nodes.len() {
                return Err(format!("Contour {ci}: node id {id} outside the node set").into());
            }
        }
        for e in &c.edges {
            if e.steps <= 0 {
                return Err(format!("Contour {ci}: an edge with {} steps", e.steps).into());
            }
        }
    }
    for phase in 0..3 {
        let m = params.measurement_types[phase];
        if !(0..=2).contains(&m) {
            return Err(format!("{m} is not a valid measurement model code").into());
        }
        let p = params.prior_types[phase];
        if p != 0 && p != 1 && p != 3 {
            return Err(format!("{p} is not a valid prior code").into());
        }
        if params.is_enabled[phase] != 0 {
            if p != 3 {
                return Err(format!("Prior code {p} is not ported").into());
            }
            if m == 1 && aa.is_none() {
                return Err("Measurement type 1 (the image model) needs the source image".into());
            }
            if params.optimizer_type[phase] != 0 {
                return Err("Optimizer type 1 is not ported".into());
            }
            let cg = &params.phases[phase];
            if cg.iter_for_restart <= 0 {
                return Err("cg_iter_for_restart must be positive".into());
            }
        }
    }
    let n = nodes.len();
    let mut s = Smoother {
        nodes,
        contours,
        canvas,
        params,
        phase: 0,
        near: Vec::new(),
        grad: vec![[0.0; 2]; n],
        blind_len: 0.0,
        aa: aa.map(|input| AaState::new(input.image, input.every, input.units, input.seed)),
    };
    // 0x496740 in the original order: the image measurement when a phase
    // measures the image, the near-border list, then with anti-aliasing the
    // neighbour lists and, on request, the placement and perturbation.
    if s.aa.is_some() && params.measurement_types.contains(&1) {
        s.measurement_setup();
    }
    s.find_near();
    if s.aa.is_some() {
        s.build_neighbours();
        if aa.map(|a| a.prepare).unwrap_or(false) {
            s.perturb();
        }
    }
    let mut report = Report::default();
    for phase in 0..3 {
        if params.is_enabled[phase] == 0 {
            continue;
        }
        s.phase = phase;
        let mut cg = Cg {
            p: params.phases[phase],
            dir: vec![0.0; 2 * n],
            backup: vec![0.0; 2 * n],
            energy: 0.0,
            prev_best: 0.0,
            eps: 0.0,
            max_dir: 0.0,
            total: 0.0,
            beta: 0.0,
            stalled: 0,
            iteration: 0,
            restarts: 0,
            ls_iterations: 0,
            phase_counter: 0,
        };
        cg.run(&mut s)?;
        report.iterations[phase] = cg.iteration;
        report.line_search_iterations[phase] = cg.ls_iterations;
        report.restarts[phase] = cg.restarts;
        report.final_energy[phase] = cg.energy;
        if params.do_puncture_corners[phase] != 0 {
            report.corners += s.puncture()?;
        }
    }
    // The tail of 0x497c30: one more pass over the whole node set.
    if s.aa.is_some() {
        report.marked = s.node_set_pass();
        report.seed = s.aa.as_ref().map(|a| a.seed).unwrap_or(0);
    } else {
        report.seed = aa.map(|a| a.seed).unwrap_or(0);
    }
    Ok(report)
}

#[cfg(test)]
pub(crate) mod aa_tests;
#[cfg(test)]
mod tests;
