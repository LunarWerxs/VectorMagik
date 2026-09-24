//! Owned interval fitting and merge/shift scheduling recovered from Vector Magic.
//! Native addresses and executed comparisons: docs/RUST_PIPELINE.md.
//! No original machine code, native pointer layouts or allocator are used here.
use crate::fitting::{
    chord_length_samples, find_boundary_interval, fit_low_count, quadratic_to_cubic,
};
use crate::geometry::{Cubic, Point, LEGACY_RIDGE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitDegree {
    Seed,
    Line,
    Quadratic,
    Cubic,
}

#[derive(Clone, Copy, Debug)]
pub struct IntervalFit {
    pub curve: Cubic,
    /// Original quadratic scratch value, needed by the derivative-record caller.
    pub quadratic: Option<[Point; 3]>,
    /// Native dispatch error; the low-count branch reports only its first
    /// sample's LINE residual, rather than the emitted cubic's total residual.
    pub squared_error: f64,
    pub degree: FitDegree,
}

/// Validates that both endpoints of a fit are finite.
fn check_endpoints(start: Point, end: Point, name: &str) -> Result<(), String> {
    if ![start, end]
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite())
    {
        return Err(format!("Non-finite {} endpoint", name));
    }
    Ok(())
}

/// Shared sample guard of 0x49dcf0 and 0x49d360: a finite parameter inside
/// [0, 1] with a finite point.
fn check_sample(t: f64, p: Point, name: &str) -> Result<(), String> {
    if !t.is_finite() || !(0. ..=1.).contains(&t) || !p.x.is_finite() || !p.y.is_finite() {
        return Err(format!("Invalid {} sample", name));
    }
    Ok(())
}

/// 0x49dcf0: one quadratic handle, fixed endpoints. The original builds a
/// 2n-by-2 Bernstein design matrix, forms the normal matrix and right-hand
/// side with the reference `dgemm` (each entry a sum over the samples in
/// order, the ridge 1e-5 added to the diagonal after the sum), solves with
/// `dgesv` and sums the squared residuals of its own evaluation order.
pub fn fit_quadratic(
    start: Point,
    end: Point,
    samples: &[(f64, Point)],
) -> Result<([Point; 3], f64), String> {
    check_endpoints(start, end, "quadratic")?;
    let (mut cc, mut bx, mut by) = (0., 0., 0.);
    for &(t, p) in samples {
        check_sample(t, p, "quadratic")?;
        let u = 1. - t;
        let u2 = u * u;
        let ut = u * t;
        let c = ut + ut;
        let t2 = t * t;
        let rx = p.x - (u2 * start.x + t2 * end.x);
        let ry = p.y - (u2 * start.y + t2 * end.y);
        cc += c * c;
        bx += c * rx;
        by += c * ry;
    }
    // The 2-by-2 system decouples; dgesv keeps the pivot in place and divides.
    let mut matrix = [cc + LEGACY_RIDGE, 0., 0., cc + LEGACY_RIDGE];
    let mut rhs = [bx, by];
    if crate::recovered_lapack::dgesv(2, &mut matrix, &mut rhs) != 0 {
        return Err("Singular quadratic system".into());
    }
    let middle = Point {
        x: rhs[0],
        y: rhs[1],
    };
    let mut error = 0.;
    for &(t, p) in samples {
        let u = 1. - t;
        let tu = t * u;
        let c = tu + tu;
        let x = ((c * middle.x) + ((u * start.x) * u)) + ((t * t) * end.x);
        let y = ((c * middle.y) + ((u * start.y) * u)) + ((t * t) * end.y);
        let dx = p.x - x;
        let dy = p.y - y;
        error += (0. + dx * dx) + dy * dy;
    }
    if !middle.x.is_finite() || !middle.y.is_finite() || !error.is_finite() {
        return Err("Quadratic fit overflow".into());
    }
    Ok(([start, middle, end], error))
}

/// 0x49d360: the fixed-endpoint cubic fit. The 2n-by-4 design matrix holds
/// the two inner Bernstein weights per coordinate row; `dgemm` forms the
/// normal matrix (sums over the samples in order, the ridge 1e-5 added after
/// each sum) and the right-hand side, `dgesv` solves the 4-by-4 system, and
/// the squared error sums the residuals of 0x479e10's evaluation.
pub fn fit_cubic(
    start: Point,
    end: Point,
    samples: &[(f64, Point)],
) -> Result<(Cubic, f64), String> {
    check_endpoints(start, end, "cubic")?;
    let (mut aa, mut ab, mut bb) = (0., 0., 0.);
    let (mut ax, mut ay, mut bx, mut by) = (0., 0., 0., 0.);
    for &(u, p) in samples {
        check_sample(u, p, "cubic")?;
        let om_u = 1. - u;
        let om_u2 = om_u * om_u;
        let u2 = u * u;
        let b0 = om_u2 * om_u;
        let b1 = (om_u2 * u) * 3.;
        let b2 = (u2 * om_u) * 3.;
        let b3 = u2 * u;
        let rx = p.x - (b0 * start.x + b3 * end.x);
        let ry = p.y - (b0 * start.y + b3 * end.y);
        aa += b1 * b1;
        ab += b1 * b2;
        bb += b2 * b2;
        ax += b1 * rx;
        ay += b1 * ry;
        bx += b2 * rx;
        by += b2 * ry;
    }
    // Column-major 4-by-4: unknowns (P1.x, P1.y, P2.x, P2.y).
    let mut matrix = [0.; 16];
    matrix[0] = aa + LEGACY_RIDGE;
    matrix[5] = aa + LEGACY_RIDGE;
    matrix[10] = bb + LEGACY_RIDGE;
    matrix[15] = bb + LEGACY_RIDGE;
    matrix[2] = ab;
    matrix[8] = ab;
    matrix[7] = ab;
    matrix[13] = ab;
    let mut rhs = [ax, ay, bx, by];
    if crate::recovered_lapack::dgesv(4, &mut matrix, &mut rhs) != 0 {
        return Err("Singular cubic system".into());
    }
    let curve = Cubic {
        points: [
            start,
            Point {
                x: rhs[0],
                y: rhs[1],
            },
            Point {
                x: rhs[2],
                y: rhs[3],
            },
            end,
        ],
    };
    let mut error = 0.;
    for &(u, p) in samples {
        let q = curve.evaluate(u);
        let dx = p.x - q.x;
        let dy = p.y - q.y;
        error += (0. + dx * dx) + dy * dy;
    }
    if !curve
        .points
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite())
        || !error.is_finite()
    {
        return Err("Cubic fit overflow".into());
    }
    Ok((curve, error))
}

/// Numerical dispatch from 0x49e960 including chord parameterization from
/// 0x49c7b0. The caller provides the four initial seed points; the zero-interior
/// branch preserves them. This is important for one-step intervals.
pub fn fit_interval(seed: Cubic, interior: &[Point]) -> Result<IntervalFit, String> {
    if !seed
        .points
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite())
    {
        return Err("Non-finite interval seed".into());
    }
    let samples = chord_length_samples(seed.points[0], seed.points[3], interior)?;
    let (curve, squared_error, degree, quadratic) = match samples.len() {
        0 => (seed, 0., FitDegree::Seed, None),
        1 | 2 => {
            let (curve, error) = fit_low_count(seed.points[0], seed.points[3], &samples)?;
            (curve, error, FitDegree::Line, None)
        }
        3 => {
            let (quadratic, error) = fit_quadratic(seed.points[0], seed.points[3], &samples)?;
            (
                quadratic_to_cubic(quadratic)?,
                error,
                FitDegree::Quadratic,
                Some(quadratic),
            )
        }
        _ => {
            let (curve, error) = fit_cubic(seed.points[0], seed.points[3], &samples)?;
            (curve, error, FitDegree::Cubic, None)
        }
    };
    if !squared_error.is_finite() {
        return Err("Interval error overflow".into());
    }
    Ok(IntervalFit {
        curve,
        quadratic,
        squared_error,
        degree,
    })
}

/// Seed and collect a cyclic contour interval as 0x49e960 does. End positions
/// may be unwrapped. Equal start/end means zero length, not a full circuit.
pub fn fit_contour_interval(
    contour: &[Point],
    start: usize,
    end: usize,
) -> Result<IntervalFit, String> {
    if contour.is_empty() {
        return Err("Empty contour".into());
    }
    let n = contour.len();
    let end = if end < start {
        end.checked_add(
            (start - end)
                .div_ceil(n)
                .checked_mul(n)
                .ok_or("Interval overflow")?,
        )
        .ok_or("Interval overflow")?
    } else {
        end
    };
    if end - start > n {
        return Err("Interval exceeds one contour circuit".into());
    }
    let point = |i: usize| contour[i % n];
    let seed = Cubic {
        points: [
            point(start),
            point(start.checked_add(1).ok_or("Interval overflow")?),
            point(end.checked_add(n - 1).ok_or("Interval overflow")?),
            point(end),
        ],
    };
    let interior: Vec<_> = (start.saturating_add(1)..end).map(point).collect();
    fit_interval(seed, &interior)
}

#[derive(Clone, Copy, Debug)]
pub struct ScheduleSettings {
    pub initial_threshold: f64,
    pub final_threshold: f64,
    pub ramp_fraction: f64,
    pub passes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FitInterval {
    pub start: usize,
    pub length: usize,
}
#[derive(Debug)]
pub struct Schedule {
    /// Relative to the supplied start. Excludes the original terminal record.
    pub intervals: Vec<FitInterval>,
    pub thresholds: Vec<f64>,
    pub merges: usize,
    pub backward_shifts: usize,
    pub forward_shifts: usize,
    pub threshold_after_passes: f64,
}

/// An ordered write to a contour node. Keeping writes ordered also preserves
/// behavior when several contour positions refer to the same shared node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeMark {
    pub position: usize,
    pub state: u8,
    /// None preserves the node's previous index, as the native byte-only write.
    pub curve_index: Option<usize>,
}

#[derive(Debug)]
pub struct ScheduledMarks {
    pub writes: Vec<NodeMark>,
    pub next_curve_index: usize,
}

/// Node-state portion of 0x4a0dc0 after 0x49f8b0 returns. Interval starts get
/// state 5, interiors get state 1 and the current curve index, then both outer
/// endpoints get state 4. The terminal record consumes an index too.
/// Numeric states are retained without inventing unconfirmed semantic names.
pub fn mark_schedule(
    intervals: &[FitInterval],
    node_count: usize,
    start: usize,
    end: usize,
    first_curve_index: usize,
) -> Result<ScheduledMarks, String> {
    if node_count == 0 || end <= start || end - start > node_count {
        return Err("Invalid marking interval".into());
    }
    let mut covered = 0;
    for interval in intervals {
        if interval.start != covered || interval.length == 0 {
            return Err("Non-contiguous marking schedule".into());
        }
        covered = covered
            .checked_add(interval.length)
            .ok_or("Marking overflow")?;
    }
    if covered != end - start {
        return Err("Marking schedule does not cover the interval".into());
    }
    let next_curve_index = first_curve_index
        .checked_add(intervals.len())
        .and_then(|v| v.checked_add(1))
        .ok_or("Curve index overflow")?;
    let mut writes = Vec::new();
    for (i, interval) in intervals.iter().enumerate() {
        let begin = start + interval.start;
        writes.push(NodeMark {
            position: begin % node_count,
            state: 5,
            curve_index: None,
        });
        for position in begin + 1..begin + interval.length {
            writes.push(NodeMark {
                position: position % node_count,
                state: 1,
                curve_index: Some(first_curve_index + i),
            });
        }
    }
    writes.push(NodeMark {
        position: end % node_count,
        state: 5,
        curve_index: None,
    });
    for position in [start, end] {
        writes.push(NodeMark {
            position: position % node_count,
            state: 4,
            curve_index: None,
        });
    }
    Ok(ScheduledMarks {
        writes,
        next_curve_index,
    })
}

#[derive(Clone)]
struct Entry {
    range: FitInterval,
    error: Option<f64>,
    next: Option<usize>,
}

#[derive(Debug)]
pub struct InitialFit {
    pub curve_count: usize,
    pub interval_calls: usize,
    /// Last scheduled (contour, start, unwrapped end), for native state checks.
    pub last_interval: Option<(usize, usize, usize)>,
}

/// 0x4a0f00: walk contours in their original order, processing state-zero nodes.
/// Node identity and mutable flags are shared across contours. Original array
/// allocation and progress callbacks are host concerns, not fitting decisions.
pub fn initialize_contours(
    contours: &[Vec<usize>],
    points: &[Point],
    flags: &mut [u8],
    curve_indices: &mut [usize],
    settings: ScheduleSettings,
) -> Result<InitialFit, String> {
    if flags.len() != points.len()
        || curve_indices.len() != points.len()
        || contours.iter().flatten().any(|&id| id >= points.len())
        || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        return Err("Invalid initial fitting state".into());
    }
    let mut result = InitialFit {
        curve_count: 0,
        interval_calls: 0,
        last_interval: None,
    };
    for (ci, ids) in contours.iter().enumerate() {
        let contour: Vec<_> = ids.iter().map(|&id| points[id]).collect();
        for position in 0..ids.len() {
            if flags[ids[position]] != 0 {
                continue;
            }
            let (start, end) =
                find_boundary_interval(ids, flags, position, 0, "Initial fit overflow")?;
            let schedule = schedule_contour(&contour, start, end, settings)?;
            let marks = mark_schedule(
                &schedule.intervals,
                ids.len(),
                start,
                end,
                result.curve_count,
            )?;
            for mark in marks.writes {
                let id = ids[mark.position];
                flags[id] = mark.state;
                if let Some(index) = mark.curve_index {
                    curve_indices[id] = index;
                }
            }
            result.curve_count = marks.next_curve_index;
            result.interval_calls += 1;
            result.last_interval = Some((ci, start, end));
        }
    }
    Ok(result)
}

/// 0x49a560 / 0x49f8b0 / 0x49f310: sequential adjacent merges followed by
/// one-node boundary shifts. This is not recursive maximum-error splitting.
/// The terminal record and post-update traversal order are preserved.
pub fn schedule_contour(
    contour: &[Point],
    start: usize,
    end: usize,
    settings: ScheduleSettings,
) -> Result<Schedule, String> {
    let n = contour.len();
    if n == 0 || end <= start || end - start > n || n > 1_000_000 {
        return Err("Invalid scheduling interval".into());
    }
    if contour.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err("Non-finite scheduling point".into());
    }
    let s = settings;
    if ![s.initial_threshold, s.final_threshold, s.ramp_fraction]
        .iter()
        .all(|x| x.is_finite() && *x > 0.)
        || s.passes > 1000
    {
        return Err("Invalid schedule settings".into());
    }
    let span = end - start;
    let mut entries: Vec<_> = (0..=span)
        .map(|i| Entry {
            range: FitInterval {
                start: i,
                length: 1,
            },
            error: None,
            next: if i < span { Some(i + 1) } else { None },
        })
        .collect();
    let error = |range: FitInterval| -> Result<f64, String> {
        let begin = start.checked_add(range.start).ok_or("Interval overflow")?;
        Ok(fit_contour_interval(
            contour,
            begin,
            begin.checked_add(range.length).ok_or("Interval overflow")?,
        )?
        .squared_error)
    };
    let mut out = Schedule {
        intervals: Vec::new(),
        thresholds: Vec::new(),
        merges: 0,
        backward_shifts: 0,
        forward_shifts: 0,
        threshold_after_passes: s.initial_threshold,
    };
    let mut threshold = s.initial_threshold;
    let factor = if s.passes == 0 {
        1.
    } else {
        ((s.final_threshold / s.initial_threshold).ln() / (s.passes as f64 * s.ramp_fraction)).exp()
    };
    if !factor.is_finite() {
        return Err("Threshold overflow".into());
    }
    for pass in 0..s.passes {
        out.thresholds.push(threshold);
        let mut i = 0;
        while let Some(j) = entries[i].next {
            if entries[j].next.is_none() {
                break;
            }
            let (left, right) = (entries[i].range, entries[j].range);
            let le = match entries[i].error {
                Some(v) => v,
                None => error(left)?,
            };
            let re = match entries[j].error {
                Some(v) => v,
                None => error(right)?,
            };
            entries[i].error = Some(le);
            entries[j].error = Some(re);
            let merged = FitInterval {
                start: left.start,
                length: left.length + right.length,
            };
            // An interval's error depends on its range alone, so every new
            // range keeps the error just computed for it instead of fitting
            // it again when the walk reaches it.
            let me = error(merged)?;
            if me - (le + re) < threshold {
                entries[i].range = merged;
                entries[i].next = entries[j].next;
                entries[i].error = Some(me);
                out.merges += 1;
            } else {
                // 0x49f0d0 changes sign according to interval growth/shrinkage.
                // The two calls' difference is the total shifted error delta.
                let left_back = FitInterval {
                    length: left.length - 1,
                    ..left
                };
                let right_back = FitInterval {
                    start: right.start - 1,
                    length: right.length + 1,
                };
                let rbe = error(right_back)?;
                let lbe = error(left_back)?;
                if (rbe - re) - (le - lbe) < 0. {
                    if left_back.length == 0 {
                        return Err("Native backward-shift invariant violated".into());
                    }
                    entries[i].range = left_back;
                    entries[j].range = right_back;
                    entries[i].error = Some(lbe);
                    entries[j].error = Some(rbe);
                    out.backward_shifts += 1;
                } else {
                    let left_forward = FitInterval {
                        length: left.length + 1,
                        ..left
                    };
                    let right_forward = FitInterval {
                        start: right.start + 1,
                        length: right.length - 1,
                    };
                    let lfe = error(left_forward)?;
                    let rfe = error(right_forward)?;
                    if (lfe - le) - (re - rfe) < 0. {
                        if right_forward.length == 0 {
                            return Err("Native forward-shift invariant violated".into());
                        }
                        entries[i].range = left_forward;
                        entries[j].range = right_forward;
                        entries[i].error = Some(lfe);
                        entries[j].error = Some(rfe);
                        out.forward_shifts += 1;
                    }
                }
            }
            // Native advances to the *updated* successor, even after merging.
            i = entries[i].next.ok_or("Missing terminal interval")?;
        }
        threshold = if (pass as f64) < s.passes as f64 * s.ramp_fraction {
            threshold * factor
        } else {
            s.final_threshold
        };
    }
    let mut i = 0;
    while entries[i].next.is_some() {
        out.intervals.push(entries[i].range);
        i = entries[i].next.unwrap();
    }
    out.threshold_after_passes = threshold;
    Ok(out)
}

#[cfg(test)]
mod tests;
