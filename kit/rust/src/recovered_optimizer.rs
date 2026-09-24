//! Optional fitting pass 0x49fc80 (run by 0x4a1030 when fitter+0x1c is set).
//! It assembles one Newton system over the interior controls of every curve
//! with an active derivative record: the data gradient/Hessian from the
//! records, plus a tangent-continuity penalty at each junction of two curve
//! parts, then subtracts the solution from the coordinates. The objective it
//! descends is the one evaluated by 0x49ccb0 (used by the original's own
//! finite-difference check):
//!
//! `E = sum_records sum_samples |C(t) - q|^2 + w * sum_junctions |uA - uB|^2`
//!
//! with `uA`/`uB` the unit end/start tangents meeting at the junction and
//! `w` = fitter+0x38 (constructed as 10.0 from 0x6fbef8). The gradient and
//! Hessian blocks below are exactly its derivatives, which is how the objective
//! was confirmed rather than read from names.
//!
//! Three things are deliberately not machine-code translations:
//! - The sparse solve. The original 0x49bd00 calls 0x49bb40, which loads its
//!   CSR pointers from absolute addresses 0xc/0x10/0x14 and faults (host mode
//!   `--solver-probe` records the access violation at 0x49bb90). `solve` is
//!   owned Gaussian elimination with partial pivoting on the same sparse rows.
//! - The gradient-check diagnostic 0x49f970 (finite differences, printf and
//!   CSV files). It does not touch the geometry and is not ported.
//! - Division by a zero-length tangent returns an error instead of NaN.
//!
//! The pass is off by default: fitter+0x1c is constructed as zero and no
//! preset registers it, so the shipped product never ran it.
use std::collections::BTreeMap;

use crate::fitting::{chord_length_samples, cubic_to_quadratic, quadratic_to_cubic};
use crate::geometry::{Cubic, Point};
use crate::recovered_state::{DerivativeUpdate, FinalPart};

/// Constant at 0x6fd8f8, the factor on every tangent derivative.
const TANGENT_FACTOR: f64 = -2.;
/// Default tangent-continuity weight, fitter+0x38 from 0x6fbef8.
pub const DEFAULT_TANGENT_WEIGHT: f64 = 10.;

#[derive(Clone, Copy, Debug)]
pub struct OptimizerSettings {
    /// fitter+0x38.
    pub weight: f64,
}
impl Default for OptimizerSettings {
    fn default() -> Self {
        Self {
            weight: DEFAULT_TANGENT_WEIGHT,
        }
    }
}

/// Everything 0x49fc80 reads. Records and curves are parallel per curve slot;
/// a slot without a record, or a record with `active == 0`, is inactive.
pub struct OptimizerInput<'a> {
    pub contours: &'a [Vec<usize>],
    /// Bit 0 of the first metadata word at each contour position (contour+0x20,
    /// stride 12). A set bit skips the tangent penalty at that node.
    pub corners: &'a [Vec<bool>],
    pub points: &'a [Point],
    pub parts: &'a [Vec<FinalPart>],
    pub records: &'a [Option<DerivativeUpdate>],
    pub weight: f64,
}

/// The assembled system: cumulative coordinate offsets per record (+0xa4), the
/// gradient vector, and the upper triangle of the Hessian as ordered rows.
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizerSystem {
    pub offsets: Vec<usize>,
    pub gradient: Vec<f64>,
    pub rows: Vec<BTreeMap<usize, f64>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptimizerReport {
    pub unknowns: usize,
    pub junctions: usize,
    pub objective_before: f64,
    pub objective_after: f64,
}

fn active(records: &[Option<DerivativeUpdate>], index: usize) -> Result<usize, String> {
    match records.get(index) {
        Some(Some(record)) if record.active <= 4 => Ok(record.active),
        Some(_) => Ok(0),
        None => Err("Optimizer curve index out of bounds".into()),
    }
}

/// 0x47e2c0: a curve part whose record has coordinates.
fn optimizable(part: &FinalPart, records: &[Option<DerivativeUpdate>]) -> Result<bool, String> {
    Ok(part.kind != 0 && active(records, part.index)? != 0)
}

fn curve(curves: &[Option<Cubic>], index: usize) -> Result<Cubic, String> {
    curves
        .get(index)
        .copied()
        .flatten()
        .ok_or_else(|| "Optimizer references an unwritten curve slot".into())
}

/// 0x47e1f0: control point k of a part, reversed parts reading 3-k.
fn part_point(c: Cubic, part: &FinalPart, k: usize) -> Point {
    c.points[if part.kind == 2 { 3 - k } else { k }]
}

/// Inner 4-coordinate tangent computation for `start_tangent` and `end_tangent`.
fn cubic_tangent_segment(c: Cubic, part: &FinalPart, k0: usize, k1: usize) -> Point {
    let (p0, p1) = (part_point(c, part, k0), part_point(c, part, k1));
    Point {
        x: p1.x - p0.x,
        y: p1.y - p0.y,
    }
}

/// Shared 0x47fe00/0x47ff00 head: the four-coordinate control-point difference,
/// or the curve to be read as a quadratic when the record has two coordinates.
fn tangent_head(
    curves: &[Option<Cubic>],
    records: &[Option<DerivativeUpdate>],
    part: &FinalPart,
    k0: usize,
    k1: usize,
) -> Result<(Option<Point>, Cubic), String> {
    let c = curve(curves, part.index)?;
    if active(records, part.index)? == 4 {
        return Ok((Some(cubic_tangent_segment(c, part, k0, k1)), c));
    }
    Ok((None, c))
}

/// 0x47fe00: tangent leaving the part's start.
fn start_tangent(
    curves: &[Option<Cubic>],
    records: &[Option<DerivativeUpdate>],
    part: &FinalPart,
) -> Result<Point, String> {
    let (segment, c) = tangent_head(curves, records, part, 0, 1)?;
    if let Some(segment) = segment {
        return Ok(segment);
    }
    let q = cubic_to_quadratic(c)?;
    let from = if part.kind == 2 { q[2] } else { q[0] };
    Ok(Point {
        x: q[1].x - from.x,
        y: q[1].y - from.y,
    })
}

/// 0x47ff00: tangent arriving at the part's end.
fn end_tangent(
    curves: &[Option<Cubic>],
    records: &[Option<DerivativeUpdate>],
    part: &FinalPart,
) -> Result<Point, String> {
    let (segment, c) = tangent_head(curves, records, part, 2, 3)?;
    if let Some(segment) = segment {
        return Ok(segment);
    }
    let q = cubic_to_quadratic(c)?;
    let to = if part.kind == 2 { q[0] } else { q[2] };
    Ok(Point {
        x: to.x - q[1].x,
        y: to.y - q[1].y,
    })
}

/// 0x47e250: global coordinate index of coordinate `c` of the control point on
/// the end side (`end` true) or start side of a part. Four-coordinate records
/// hold [P1.x, P1.y, P2.x, P2.y]; two-coordinate records hold the quadratic
/// middle and ignore the side.
fn coordinate_index(
    offsets: &[usize],
    records: &[Option<DerivativeUpdate>],
    part: &FinalPart,
    c: usize,
    end: bool,
) -> Result<usize, String> {
    let base = offsets[part.index] + c;
    if active(records, part.index)? != 4 {
        return Ok(base);
    }
    let forward = part.kind == 1;
    Ok(if end == forward { base + 2 } else { base })
}

/// 0x49b2b0: read coordinate `c` of a record's variables.
fn read_coordinate(curve: Cubic, active: usize, c: usize) -> Result<f64, String> {
    if active == 4 {
        let values = [
            curve.points[1].x,
            curve.points[1].y,
            curve.points[2].x,
            curve.points[2].y,
        ];
        return Ok(values[c]);
    }
    let q = cubic_to_quadratic(curve)?;
    Ok(if c == 0 { q[1].x } else { q[1].y })
}

/// 0x49b350: write coordinate `c`; two-coordinate records regenerate the cubic
/// from the modified quadratic exactly as the three-sample fit does.
fn write_coordinate(curve: &mut Cubic, active: usize, c: usize, value: f64) -> Result<(), String> {
    if active == 4 {
        match c {
            0 => curve.points[1].x = value,
            1 => curve.points[1].y = value,
            2 => curve.points[2].x = value,
            _ => curve.points[2].y = value,
        }
        return Ok(());
    }
    let mut q = cubic_to_quadratic(*curve)?;
    if c == 0 {
        q[1].x = value;
    } else {
        q[1].y = value;
    }
    *curve = quadratic_to_cubic(q)?;
    Ok(())
}

struct Junction {
    contour: usize,
    part: usize,
    next: usize,
}

/// The junction walk shared by 0x49ccb0 and 0x49fc80: every part with its
/// cyclic successor, both curves with active records, and the node between
/// them not flagged in bit 0 of its metadata word.
fn junctions(input: &OptimizerInput) -> Result<Vec<Junction>, String> {
    if input.parts.len() != input.contours.len() || input.corners.len() != input.contours.len() {
        return Err("Optimizer contour/part/corner counts disagree".into());
    }
    let mut out = Vec::new();
    for (ci, parts) in input.parts.iter().enumerate() {
        let corners = &input.corners[ci];
        if corners.len() != input.contours[ci].len() {
            return Err("Optimizer corner flags do not match the contour".into());
        }
        for (pi, part) in parts.iter().enumerate() {
            let next = (pi + 1) % parts.len();
            if !optimizable(part, input.records)? || !optimizable(&parts[next], input.records)? {
                continue;
            }
            let node = if part.kind == 2 {
                part.start_position
            } else {
                part.end_position
            };
            let corner = *corners
                .get(node)
                .ok_or("Optimizer junction position out of bounds")?;
            if corner {
                continue;
            }
            out.push(Junction {
                contour: ci,
                part: pi,
                next,
            });
        }
    }
    Ok(out)
}

fn norm(p: Point) -> f64 {
    (p.x * p.x + p.y * p.y).sqrt()
}

/// 0x49ccb0: the objective. Sample parameters come from the record's contour
/// interval and the curve's endpoints, so they do not move with the interior.
pub fn objective(input: &OptimizerInput, curves: &[Option<Cubic>]) -> Result<f64, String> {
    let mut total = 0.;
    for (index, record) in input.records.iter().enumerate() {
        let Some(record) = record else { continue };
        if record.active == 0 {
            continue;
        }
        let c = curve(curves, index)?;
        let ids = input
            .contours
            .get(record.contour)
            .ok_or("Optimizer record names a missing contour")?;
        let n = ids.len();
        let mut end = record.end;
        while end < record.start {
            end = end.checked_add(n).ok_or("Optimizer interval overflow")?;
        }
        if end <= record.start || end - record.start - 1 > n {
            return Err("Optimizer record interval is invalid".into());
        }
        let interior: Vec<_> = (record.start + 1..end)
            .map(|p| {
                input
                    .points
                    .get(ids[p % n])
                    .copied()
                    .ok_or("Optimizer node out of bounds")
            })
            .collect::<Result<_, _>>()?;
        for (t, q) in chord_length_samples(c.points[0], c.points[3], &interior)? {
            let p = c.evaluate(t);
            let (dx, dy) = (p.x - q.x, p.y - q.y);
            total += dx * dx + dy * dy;
        }
    }
    for j in junctions(input)? {
        let parts = &input.parts[j.contour];
        let a = end_tangent(curves, input.records, &parts[j.part])?;
        let b = start_tangent(curves, input.records, &parts[j.next])?;
        let (na, nb) = (norm(a), norm(b));
        let (ia, ib) = (1. / na, 1. / nb);
        let ua = Point {
            x: a.x * ia,
            y: a.y * ia,
        };
        let ub = Point {
            x: b.x * ib,
            y: b.y * ib,
        };
        let (dx, dy) = (ua.x - ub.x, ua.y - ub.y);
        total += (dx * dx + dy * dy) * input.weight;
    }
    if !total.is_finite() {
        return Err("Optimizer objective is not finite".into());
    }
    Ok(total)
}

type Matrix2 = [[f64; 2]; 2];

/// Element [r][c] is rows[r] * cols[c]; the original forms some products in
/// the other operand order, which is identical in IEEE arithmetic.
fn outer(rows: Point, cols: Point) -> Matrix2 {
    [
        [rows.x * cols.x, rows.x * cols.y],
        [rows.y * cols.x, rows.y * cols.y],
    ]
}
fn add(a: Matrix2, b: Matrix2) -> Matrix2 {
    [
        [a[0][0] + b[0][0], a[0][1] + b[0][1]],
        [a[1][0] + b[1][0], a[1][1] + b[1][1]],
    ]
}
fn sub(a: Matrix2, b: Matrix2) -> Matrix2 {
    [
        [a[0][0] - b[0][0], a[0][1] - b[0][1]],
        [a[1][0] - b[1][0], a[1][1] - b[1][1]],
    ]
}
fn scale(s: f64, a: Matrix2) -> Matrix2 {
    [[s * a[0][0], s * a[0][1]], [s * a[1][0], s * a[1][1]]]
}
fn diagonal(v: f64) -> Matrix2 {
    [[v, 0.], [0., v]]
}

/// The tangent-penalty derivatives of one junction, in the original's
/// operation order: `gradient_a = (-2/|A|)(uB - (uA.uB)uA)` and its mirror,
/// `hessian_a = (2/|A|^2)(dot I + uA uB^T + uB uA^T - 3 dot uA uA^T)`, and the
/// mixed block indexed [b-coordinate][a-coordinate].
struct JunctionTerms {
    gradient_a: [f64; 2],
    gradient_b: [f64; 2],
    hessian_a: Matrix2,
    hessian_b: Matrix2,
    mixed: Matrix2,
}

fn junction_terms(a: Point, b: Point) -> Result<JunctionTerms, String> {
    let (na, nb) = (norm(a), norm(b));
    let (ia, ib) = (1. / na, 1. / nb);
    let ua = Point {
        x: ia * a.x,
        y: ia * a.y,
    };
    let ub = Point {
        x: ib * b.x,
        y: ib * b.y,
    };
    let dot = ub.x * ua.x + ub.y * ua.y;
    let perp_a = [ub.x - dot * ua.x, ub.y - dot * ua.y];
    let ka = ia * TANGENT_FACTOR;
    let gradient_a = [ka * perp_a[0], ka * perp_a[1]];
    let perp_b = [ua.x - dot * ub.x, ua.y - dot * ub.y];
    let kb = ib * TANGENT_FACTOR;
    let gradient_b = [kb * perp_b[0], kb * perp_b[1]];
    let cross = add(outer(ua, ub), outer(ub, ua));
    let aa = outer(ua, ua);
    let triple_a = [
        [(dot * aa[0][0]) * 3., (dot * aa[0][1]) * 3.],
        [(dot * aa[1][0]) * 3., (dot * aa[1][1]) * 3.],
    ];
    let hessian_a = scale(2. / (na * na), sub(add(diagonal(dot), cross), triple_a));
    let bb = outer(ub, ub);
    let triple_b = [
        [(dot * bb[0][0]) * 3., (dot * bb[0][1]) * 3.],
        [(dot * bb[1][0]) * 3., (dot * bb[1][1]) * 3.],
    ];
    let hessian_b = scale(2. / (nb * nb), sub(add(diagonal(dot), cross), triple_b));
    let projector_b = sub(diagonal(1.), outer(ub, ub));
    let projector_a = sub(diagonal(1.), outer(ua, ua));
    // 0x49ab00 forms (argument x this): rows of projector_b against columns of projector_a.
    let mut product = [[0.; 2]; 2];
    for (j, row) in product.iter_mut().enumerate() {
        for (c, value) in row.iter_mut().enumerate() {
            *value += projector_a[0][c] * projector_b[j][0] + projector_a[1][c] * projector_b[j][1];
        }
    }
    let mixed = scale(TANGENT_FACTOR / (nb * na), product);
    let terms = JunctionTerms {
        gradient_a,
        gradient_b,
        hessian_a,
        hessian_b,
        mixed,
    };
    let finite = terms
        .gradient_a
        .iter()
        .chain(&terms.gradient_b)
        .chain(terms.hessian_a.iter().flatten())
        .chain(terms.hessian_b.iter().flatten())
        .chain(terms.mixed.iter().flatten())
        .all(|v| v.is_finite());
    if !finite {
        return Err("Optimizer junction tangent has zero length".into());
    }
    Ok(terms)
}

fn accumulate(rows: &mut [BTreeMap<usize, f64>], row: usize, column: usize, value: f64) {
    match rows[row].get_mut(&column) {
        Some(entry) => *entry += value,
        None => {
            rows[row].insert(column, value);
        }
    }
}

/// First half of 0x49fc80: offsets, gradient and the upper-triangular Hessian
/// rows, adding each junction's terms in the original write order.
pub fn assemble(
    input: &OptimizerInput,
    curves: &[Option<Cubic>],
) -> Result<OptimizerSystem, String> {
    let mut offsets = Vec::with_capacity(input.records.len());
    let mut total = 0usize;
    for record in input.records {
        offsets.push(total);
        total = total
            .checked_add(record.as_ref().map_or(0, |r| r.active))
            .ok_or("Optimizer coordinate overflow")?;
    }
    if input.records.iter().flatten().any(|r| r.active > 4) {
        return Err("Optimizer record has more than four coordinates".into());
    }
    let mut gradient = vec![0.; total];
    let mut rows = vec![BTreeMap::new(); total];
    for (record, &offset) in input.records.iter().zip(&offsets) {
        let Some(record) = record else { continue };
        for i in 0..record.active {
            gradient[offset + i] = record.gradient[i];
            for j in i..record.active {
                accumulate(&mut rows, offset + i, offset + j, record.hessian[i][j]);
            }
        }
    }
    let records = input.records;
    for j in junctions(input)? {
        let parts = &input.parts[j.contour];
        let (part_a, part_b) = (&parts[j.part], &parts[j.next]);
        let a = end_tangent(curves, records, part_a)?;
        let b = start_tangent(curves, records, part_b)?;
        let terms = junction_terms(a, b)?;
        let w = input.weight;
        let index_a = |c| coordinate_index(&offsets, records, part_a, c, true);
        let index_b = |c| coordinate_index(&offsets, records, part_b, c, false);
        for i in 0..2 {
            let ia = index_a(i)?;
            gradient[ia] -= terms.gradient_a[i] * w;
            let ib = index_b(i)?;
            gradient[ib] += terms.gradient_b[i] * w;
            for k in i..2 {
                accumulate(
                    &mut rows,
                    index_a(i)?,
                    index_a(k)?,
                    terms.hessian_a[i][k] * w,
                );
                accumulate(
                    &mut rows,
                    index_b(i)?,
                    index_b(k)?,
                    terms.hessian_b[i][k] * w,
                );
            }
            for k in 0..2 {
                let value = -(w * terms.mixed[i][k]);
                let (ib, ia) = (index_b(i)?, index_a(k)?);
                accumulate(&mut rows, ia.min(ib), ia.max(ib), value);
            }
        }
    }
    Ok(OptimizerSystem {
        offsets,
        gradient,
        rows,
    })
}

/// Owned replacement for the faulting 0x49bb40: expand the upper triangle to
/// the full symmetric matrix and eliminate with partial pivoting on sparse
/// rows. Singular or non-finite systems are reported, not guessed around.
pub fn solve(rows: &[BTreeMap<usize, f64>], b: &[f64]) -> Result<Vec<f64>, String> {
    let n = b.len();
    if rows.len() != n {
        return Err("Optimizer system dimensions disagree".into());
    }
    let mut a: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); n];
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (r, row) in rows.iter().enumerate() {
        for (&c, &v) in row {
            if c < r || c >= n {
                return Err("Optimizer system is not upper triangular".into());
            }
            *a[r].entry(c).or_insert(0.) += v;
            if c != r {
                *a[c].entry(r).or_insert(0.) += v;
            }
        }
    }
    for (r, row) in a.iter().enumerate() {
        for &c in row.keys() {
            columns[c].push(r);
        }
    }
    let mut rhs = b.to_vec();
    let mut placed = vec![usize::MAX; n];
    let mut order = Vec::with_capacity(n);
    for k in 0..n {
        let mut pivot_row = None;
        let mut best = 0.;
        for &r in &columns[k] {
            if placed[r] != usize::MAX {
                continue;
            }
            let v = a[r].get(&k).copied().unwrap_or(0.).abs();
            if v > best {
                best = v;
                pivot_row = Some(r);
            }
        }
        let Some(p) = pivot_row else {
            return Err("Optimizer system is singular".into());
        };
        if !best.is_finite() {
            return Err("Optimizer system is not finite".into());
        }
        placed[p] = k;
        order.push(p);
        let pivot_entries: Vec<(usize, f64)> = a[p].range(k..).map(|(&c, &v)| (c, v)).collect();
        let pivot = pivot_entries[0].1;
        let targets: Vec<usize> = columns[k]
            .iter()
            .copied()
            .filter(|&r| placed[r] == usize::MAX)
            .collect();
        for r in targets {
            let Some(&head) = a[r].get(&k) else { continue };
            let factor = head / pivot;
            for &(c, v) in &pivot_entries {
                if c == k {
                    continue;
                }
                match a[r].get_mut(&c) {
                    Some(entry) => *entry -= factor * v,
                    None => {
                        a[r].insert(c, -(factor * v));
                        columns[c].push(r);
                    }
                }
            }
            a[r].remove(&k);
            rhs[r] -= factor * rhs[p];
        }
    }
    let mut x = vec![0.; n];
    for k in (0..n).rev() {
        let r = order[k];
        let mut s = rhs[r];
        let mut pivot = 0.;
        for (&c, &v) in &a[r] {
            if c == k {
                pivot = v;
            } else if c > k {
                s -= v * x[c];
            }
        }
        x[k] = s / pivot;
        if !x[k].is_finite() {
            return Err("Optimizer solution is not finite".into());
        }
    }
    Ok(x)
}

/// Tail of 0x49fc80: every active coordinate becomes `value - x`, read and
/// written through the record's representation in record then coordinate order.
pub fn apply(
    records: &[Option<DerivativeUpdate>],
    offsets: &[usize],
    solution: &[f64],
    curves: &mut [Option<Cubic>],
) -> Result<(), String> {
    for (index, record) in records.iter().enumerate() {
        let Some(record) = record else { continue };
        for c in 0..record.active {
            let at = offsets[index] + c;
            let step = *solution
                .get(at)
                .ok_or("Optimizer solution is shorter than the system")?;
            let mut current = curve(curves, index)?;
            let value = read_coordinate(current, record.active, c)? - step;
            write_coordinate(&mut current, record.active, c, value)?;
            curves[index] = Some(current);
        }
    }
    Ok(())
}

/// 0x49fc80 without its diagnostic: assemble, solve, update.
pub fn optimize(
    input: &OptimizerInput,
    curves: &mut [Option<Cubic>],
) -> Result<OptimizerReport, String> {
    if !input.weight.is_finite() {
        return Err("Optimizer weight is not finite".into());
    }
    let before = objective(input, curves)?;
    let system = assemble(input, curves)?;
    let junctions = junctions(input)?.len();
    let x = solve(&system.rows, &system.gradient)?;
    apply(input.records, &system.offsets, &x, curves)?;
    Ok(OptimizerReport {
        unknowns: system.gradient.len(),
        junctions,
        objective_before: before,
        objective_after: objective(input, curves)?,
    })
}

#[cfg(test)]
mod tests;
