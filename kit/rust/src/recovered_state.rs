//! Preparation, refinement and derivative records recovered from original code,
//! over owned values; no native adapter or original memory layout remains.
use crate::fitting::{chord_length_samples, cubic_fit_derivatives, find_boundary_interval};
use crate::geometry::{Cubic, Point, LEGACY_RIDGE};
use crate::recovered_fit::{
    fit_contour_interval, initialize_contours, IntervalFit, ScheduleSettings,
};
use crate::recovered_optimizer::{optimize, OptimizerInput, OptimizerReport, OptimizerSettings};

#[derive(Debug)]
pub struct FittedContours {
    pub flags: Vec<u8>,
    pub indices: Vec<usize>,
    /// Native allocation includes unused terminal slots. None makes those explicit.
    pub curves: Vec<Option<Cubic>>,
    pub parts: Vec<Vec<FinalPart>>,
    /// Derivative records per curve slot (fitter+0x30), written only when the
    /// optional pass is requested; a slot never refined stays None.
    pub records: Vec<Option<DerivativeUpdate>>,
}

/// Ordinary (optional optimizer disabled) 0x4a1030 fitting flow, entirely owned.
/// Inputs are already smoothed shared contours, not raster pixels. Native progress
/// callbacks and scratch allocations have no effect on these geometric results.
pub fn fit_contours(
    contours: &[Vec<usize>],
    edges: &[Vec<u32>],
    points: &[Point],
    flags: &[u8],
    indices: &[usize],
    settings: ScheduleSettings,
) -> Result<FittedContours, String> {
    fit_stages(contours, edges, points, flags, indices, settings, false)
}

/// 0x4a1030 with fitter+0x1c set: the same four stages building derivative
/// records, then the optional pass 0x49fc80 on the finalized parts. `corners`
/// carries bit 0 of each position's first metadata word (contour+0x20).
#[allow(clippy::too_many_arguments)]
pub fn fit_contours_optimized(
    contours: &[Vec<usize>],
    edges: &[Vec<u32>],
    corners: &[Vec<bool>],
    points: &[Point],
    flags: &[u8],
    indices: &[usize],
    settings: ScheduleSettings,
    optimizer: OptimizerSettings,
) -> Result<(FittedContours, OptimizerReport), String> {
    let mut fitted = fit_contours_with_records(contours, edges, points, flags, indices, settings)?;
    let input = OptimizerInput {
        contours,
        corners,
        points,
        parts: &fitted.parts,
        records: &fitted.records,
        weight: optimizer.weight,
    };
    let report = optimize(&input, &mut fitted.curves)?;
    Ok((fitted, report))
}

/// What the guarded optional pass did.
#[derive(Clone, Debug)]
pub struct GuardedOptimizer {
    /// The pass's report when it ran to the end, None when it failed.
    pub report: Option<OptimizerReport>,
    /// The pass's step stands: it finished without raising the objective.
    pub kept: bool,
}

/// `fit_contours_optimized` that never leaves the fit worse by its own
/// measure, an owned rule of the improved defaults (not the original's): when
/// the pass fails (a singular or non-finite system, a zero-length junction
/// tangent) or its Newton step raises the objective 0x49ccb0 it descends,
/// the curves are those of the fit before the pass, and the conversion goes
/// on. A failure of the fit itself is still an error. The accepted steps are
/// the unguarded pass's to the bit.
#[allow(clippy::too_many_arguments)]
pub fn fit_contours_optimized_guarded(
    contours: &[Vec<usize>],
    edges: &[Vec<u32>],
    corners: &[Vec<bool>],
    points: &[Point],
    flags: &[u8],
    indices: &[usize],
    settings: ScheduleSettings,
    optimizer: OptimizerSettings,
) -> Result<(FittedContours, GuardedOptimizer), String> {
    let mut fitted = fit_contours_with_records(contours, edges, points, flags, indices, settings)?;
    // The pass writes the curves in place, so a failed or rejected step
    // needs the fit it started from.
    let plain = fitted.curves.clone();
    let input = OptimizerInput {
        contours,
        corners,
        points,
        parts: &fitted.parts,
        records: &fitted.records,
        weight: optimizer.weight,
    };
    let outcome = match optimize(&input, &mut fitted.curves) {
        Ok(report) => GuardedOptimizer {
            kept: !(report.objective_after > report.objective_before),
            report: Some(report),
        },
        Err(_) => GuardedOptimizer {
            report: None,
            kept: false,
        },
    };
    if !outcome.kept {
        fitted.curves = plain;
    }
    Ok((fitted, outcome))
}

/// The four stages with derivative records built (fitter+0x1c set) but the
/// optional pass not yet run; `fit_contours_optimized` continues from here.
pub fn fit_contours_with_records(
    contours: &[Vec<usize>],
    edges: &[Vec<u32>],
    points: &[Point],
    flags: &[u8],
    indices: &[usize],
    settings: ScheduleSettings,
) -> Result<FittedContours, String> {
    fit_stages(contours, edges, points, flags, indices, settings, true)
}

fn fit_stages(
    contours: &[Vec<usize>],
    edges: &[Vec<u32>],
    points: &[Point],
    flags: &[u8],
    indices: &[usize],
    settings: ScheduleSettings,
    derivatives: bool,
) -> Result<FittedContours, String> {
    if contours.len() != edges.len()
        || contours
            .iter()
            .zip(edges)
            .any(|(c, e)| c.is_empty() || c.len() != e.len())
    {
        return Err("Invalid fitting contours/edge metadata".into());
    }
    let mut flags = flags.to_vec();
    let mut indices = indices.to_vec();
    prepare_fitting(
        &contours.iter().map(Vec::len).collect::<Vec<_>>(),
        &mut flags,
    )?;
    let initial = initialize_contours(contours, points, &mut flags, &mut indices, settings)?;
    let refined = refine_contours(
        contours,
        points,
        &mut flags,
        &indices,
        initial.curve_count,
        derivatives,
    )?;
    let mut curves = vec![None; initial.curve_count];
    let mut records = vec![None; initial.curve_count];
    for change in refined.curves {
        curves[change.index] = Some(change.fit.curve);
        records[change.index] = change.derivative;
    }
    // NaN sentinels ensure finalization cannot silently use an untouched slot.
    let starts: Vec<_> = curves
        .iter()
        .map(|c| {
            c.map_or(
                Point {
                    x: f64::NAN,
                    y: f64::NAN,
                },
                |c| c.points[0],
            )
        })
        .collect();
    let parts = contours
        .iter()
        .zip(edges)
        .map(|(c, e)| finalize_contour(c, e, points, &flags, &indices, &starts))
        .collect::<Result<_, _>>()?;
    Ok(FittedContours {
        flags,
        indices,
        curves,
        parts,
        records,
    })
}

/// 0x49b560, including its five-entry jump table at 0x49b604.
/// Return the scratch interval capacity; no contours still requests one slot.
pub fn prepare_fitting(lengths: &[usize], flags: &mut [u8]) -> Result<usize, String> {
    let capacity = match lengths.iter().max() {
        Some(n) => n.checked_add(2).ok_or("Preparation capacity overflow")?,
        None => 1,
    };
    for state in flags {
        *state = match *state {
            1 | 2 | 5 => 0,
            4 => 3,
            other => other,
        };
    }
    Ok(capacity)
}

#[derive(Clone, Debug)]
pub struct DerivativeUpdate {
    pub gradient: [f64; 4],
    pub hessian: [[f64; 4]; 4],
    /// Only this many gradient entries and this square matrix block are written.
    pub active: usize,
    pub contour: usize,
    pub start: usize,
    pub end: usize,
}

/// 0x49ed80 and the quadratic derivative builder 0x49e380. None means no writes
/// at all. Low counts update metadata/active count but retain old matrix data.
/// The quadratic branch uses the middle point retained by the preceding fit,
/// replacing only its endpoints with the emitted cubic's endpoints.
pub fn derivative_record(
    curve: Cubic,
    quadratic_middle: Point,
    interior: &[Point],
    contour: usize,
    start: usize,
    end: usize,
) -> Result<Option<DerivativeUpdate>, String> {
    let samples = chord_length_samples(curve.points[0], curve.points[3], interior)?;
    if samples.is_empty() {
        return Ok(None);
    }
    let mut record = DerivativeUpdate {
        gradient: [0.; 4],
        hessian: [[0.; 4]; 4],
        active: 0,
        contour,
        start,
        end,
    };
    if samples.len() > 3 {
        let d = cubic_fit_derivatives(curve, &samples)?;
        record.gradient = d.gradient;
        record.hessian = d.damped_hessian;
        record.active = 4;
    } else if samples.len() == 3 {
        if !quadratic_middle.x.is_finite() || !quadratic_middle.y.is_finite() {
            return Err("Non-finite quadratic scratch point".into());
        }
        record.active = 2;
        record.hessian[0][0] = LEGACY_RIDGE;
        record.hessian[1][1] = LEGACY_RIDGE;
        for (t, p) in samples {
            let u = 1. - t;
            let a = 2. * u * t;
            let predicted = Point {
                x: u * u * curve.points[0].x + a * quadratic_middle.x + t * t * curve.points[3].x,
                y: u * u * curve.points[0].y + a * quadratic_middle.y + t * t * curve.points[3].y,
            };
            record.gradient[0] += 2. * a * (predicted.x - p.x);
            record.gradient[1] += 2. * a * (predicted.y - p.y);
            record.hessian[0][0] += 2. * a * a;
            record.hessian[1][1] += 2. * a * a;
        }
    }
    if record
        .gradient
        .iter()
        .chain(record.hessian.iter().flatten())
        .any(|v| !v.is_finite())
    {
        return Err("Derivative record overflow".into());
    }
    Ok(Some(record))
}

#[derive(Debug)]
pub struct RefinedCurve {
    pub index: usize,
    pub fit: IntervalFit,
    pub derivative: Option<DerivativeUpdate>,
}
#[derive(Debug)]
pub struct Refinement {
    pub curves: Vec<RefinedCurve>,
    pub active_coordinates: usize,
}

/// 0x49ef50: fit each state-one run, changing interiors to state two before
/// continuing the ordered traversal. Shared nodes must not be reset per ring.
/// Return only writes to curve slots; untouched native slots may be uninitialized.
pub fn refine_contours(
    contours: &[Vec<usize>],
    points: &[Point],
    flags: &mut [u8],
    indices: &[usize],
    curve_count: usize,
    derivatives: bool,
) -> Result<Refinement, String> {
    if flags.len() != points.len()
        || indices.len() != points.len()
        || contours.iter().flatten().any(|&id| id >= points.len())
    {
        return Err("Invalid refinement state".into());
    }
    let mut result = Refinement {
        curves: Vec::new(),
        active_coordinates: 0,
    };
    for (ci, ids) in contours.iter().enumerate() {
        let contour: Vec<_> = ids.iter().map(|&id| points[id]).collect();
        for position in 0..ids.len() {
            if flags[ids[position]] != 1 {
                continue;
            }
            let (start, end) =
                find_boundary_interval(ids, flags, position, 1, "Refinement overflow")?;
            let index = indices[ids[position]];
            if index >= curve_count {
                return Err("Refinement curve index out of bounds".into());
            }
            let fit = fit_contour_interval(&contour, start, end)?;
            let interior: Vec<_> = (start + 1..end).map(|p| contour[p % ids.len()]).collect();
            for p in start + 1..end {
                flags[ids[p % ids.len()]] = 2;
            }
            let derivative = if derivatives {
                // Only the three-sample branch uses the quadratic scratch point.
                let middle = fit.quadratic.map_or(Point { x: 0., y: 0. }, |q| q[1]);
                derivative_record(fit.curve, middle, &interior, ci, start, end)?
            } else {
                None
            };
            if let Some(d) = &derivative {
                result.active_coordinates += d.active;
            }
            result.curves.push(RefinedCurve {
                index,
                fit,
                derivative,
            });
        }
    }
    Ok(result)
}

/// The original double constant is a widened f32, not an exact f64 1e-6.
pub const ENDPOINT_MATCH_TOLERANCE: f64 = 1e-6_f32 as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FinalPart {
    /// Contour position for a line, curve index for either curve orientation.
    pub index: usize,
    /// Original tags: 0 line, 1 forward curve, 2 reversed curve.
    pub kind: u32,
    pub edge: u32,
    pub start_node: usize,
    pub end_node: usize,
    pub start_position: usize,
    pub end_position: usize,
}

/// Normal data path of 0x49b620. Rotate away from an initial state-two run,
/// emit lines between consecutive non-two nodes, and collapse two-runs into
/// oriented curve references. Edge metadata comes from the first interior node
/// for curves. Orientation uses two independent inclusive coordinate tests.
pub fn finalize_contour(
    ids: &[usize],
    edges: &[u32],
    points: &[Point],
    flags: &[u8],
    indices: &[usize],
    curve_starts: &[Point],
) -> Result<Vec<FinalPart>, String> {
    let n = ids.len();
    if n == 0
        || edges.len() != n
        || flags.len() != points.len()
        || indices.len() != points.len()
        || ids.iter().any(|&id| id >= points.len())
    {
        return Err("Invalid finalization state".into());
    }
    let start = if flags[ids[0]] != 2 {
        0
    } else {
        (1..n)
            .rev()
            .find(|&p| flags[ids[p]] != 2)
            .ok_or("No finalization boundary node")?
    };
    let limit = start.checked_add(n).ok_or("Finalization overflow")?;
    let mut p = start;
    let mut parts = Vec::new();
    while p < limit {
        let at = p % n;
        let next = (at + 1) % n;
        let id = ids[at];
        if flags[id] != 2 && flags[ids[next]] != 2 {
            parts.push(FinalPart {
                index: at,
                kind: 0,
                edge: edges[at],
                start_node: id,
                end_node: ids[next],
                start_position: at,
                end_position: next,
            });
        }
        if flags[id] == 2 {
            let prev = (at + n - 1) % n;
            let mut end = p + 1;
            while flags[ids[end % n]] == 2 {
                end += 1;
            }
            let index = indices[id];
            let curve_start = *curve_starts
                .get(index)
                .ok_or("Finalization curve index out of bounds")?;
            let from = points[ids[prev]];
            if ![curve_start.x, curve_start.y, from.x, from.y]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err("Non-finite orientation endpoint".into());
            }
            let forward = (curve_start.x - from.x).abs() <= ENDPOINT_MATCH_TOLERANCE
                && (curve_start.y - from.y).abs() <= ENDPOINT_MATCH_TOLERANCE;
            parts.push(FinalPart {
                index,
                kind: if forward { 1 } else { 2 },
                edge: edges[at],
                start_node: ids[prev],
                end_node: ids[end % n],
                start_position: prev,
                end_position: end % n,
            });
            p = end;
        } else {
            p += 1;
        }
    }
    Ok(parts)
}

#[cfg(test)]
mod tests;
