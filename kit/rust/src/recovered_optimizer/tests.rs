use super::*;
use crate::recovered_fit::ScheduleSettings;
use crate::recovered_state::{
    fit_contours_optimized, fit_contours_optimized_guarded, fit_contours_with_records,
    FittedContours,
};

/// One native derivative record: unknown count, three derivative rows and the three part indices.
type NativeRecord = (usize, Vec<f64>, Vec<f64>, Vec<f64>, [usize; 3]);

fn values(line: &str) -> Vec<f64> {
    line.split(',')
        .skip(1)
        .map(|v| v.parse().unwrap())
        .collect()
}
fn compare(actual: &[f64], expected: &[f64], what: &str, case: f64) {
    assert_eq!(actual.len(), expected.len(), "case {case}: {what} length");
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (a - b).abs() < 1e-7 * (1. + b.abs()),
            "case {case}, {what} field {i}: {a} != {b}"
        );
    }
}
fn flat(curves: &[Option<Cubic>], records: &[Option<DerivativeUpdate>]) -> Vec<f64> {
    curves
        .iter()
        .zip(records)
        .filter(|(_, r)| r.as_ref().is_some_and(|r| r.active > 0))
        .flat_map(|(c, _)| c.unwrap().points.into_iter().flat_map(|p| [p.x, p.y]))
        .collect()
}

/// One frozen native row: the state after the four stages, the objective, the
/// captured gradient/Hessian, the supplied solution, and the updated curves.
struct Case {
    id: f64,
    settings: ScheduleSettings,
    weight: f64,
    points: Vec<Point>,
    flags: Vec<u8>,
    indices: Vec<usize>,
    contours: Vec<Vec<usize>>,
    edges: Vec<Vec<u32>>,
    corners: Vec<Vec<bool>>,
    curve_count: usize,
    total: usize,
    final_flags: Vec<(u8, usize)>,
    records: Vec<Option<NativeRecord>>,
    parts: Vec<Vec<[usize; 7]>>,
    objective_before: f64,
    gradient: Vec<f64>,
    rows: Vec<Vec<(usize, f64)>>,
    solution: Vec<f64>,
    offsets: Vec<usize>,
    updated: Vec<f64>,
    objective_after: f64,
}

fn parse(line: &str) -> Case {
    let v = values(line);
    let (n, nc) = (v[1] as usize, v[2] as usize);
    let settings = ScheduleSettings {
        initial_threshold: v[3],
        final_threshold: v[4],
        ramp_fraction: v[5],
        passes: v[6] as usize,
    };
    let weight = v[7];
    let mut at = 8;
    let (mut points, mut flags, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for row in v[at..at + 4 * n].as_chunks::<4>().0 {
        points.push(Point {
            x: row[0],
            y: row[1],
        });
        flags.push(row[2] as u8);
        indices.push(row[3] as usize);
    }
    at += 4 * n;
    let (mut contours, mut edges, mut corners) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..nc {
        let count = v[at] as usize;
        at += 1;
        contours.push(v[at..at + count].iter().map(|x| *x as usize).collect());
        at += count;
        edges.push(v[at..at + count].iter().map(|x| *x as u32).collect());
        at += count;
        corners.push(
            v[at..at + count]
                .iter()
                .map(|x| (*x as u32) & 1 == 1)
                .collect(),
        );
        at += count;
    }
    let curve_count = v[at] as usize;
    let total = v[at + 1] as usize;
    at += 2;
    let final_flags = (0..n)
        .map(|i| (v[at + 2 * i] as u8, v[at + 2 * i + 1] as usize))
        .collect();
    at += 2 * n;
    let mut records = Vec::new();
    for _ in 0..curve_count {
        let active = v[at] as usize;
        at += 1;
        if active == 0 {
            records.push(None);
            continue;
        }
        let curve = v[at..at + 8].to_vec();
        at += 8;
        let gradient = v[at..at + active].to_vec();
        at += active;
        let hessian = v[at..at + active * active].to_vec();
        at += active * active;
        let meta = [v[at] as usize, v[at + 1] as usize, v[at + 2] as usize];
        at += 3;
        records.push(Some((active, curve, gradient, hessian, meta)));
    }
    let mut parts = Vec::new();
    for _ in 0..nc {
        let count = v[at] as usize;
        at += 1;
        parts.push(
            v[at..at + 7 * count]
                .as_chunks::<7>()
                .0
                .iter()
                .map(|p| std::array::from_fn(|i| p[i] as usize))
                .collect(),
        );
        at += 7 * count;
    }
    let objective_before = v[at];
    at += 1;
    assert_eq!(v[at] as usize, total);
    at += 1;
    let gradient = v[at..at + total].to_vec();
    at += total;
    assert_eq!(v[at] as usize, total);
    at += 1;
    let mut rows = Vec::new();
    for _ in 0..total {
        let size = v[at] as usize;
        at += 1;
        rows.push(
            (0..size)
                .map(|i| (v[at + 2 * i] as usize, v[at + 2 * i + 1]))
                .collect(),
        );
        at += 2 * size;
    }
    let solution = v[at..at + total].to_vec();
    at += total;
    let offsets = v[at..at + curve_count]
        .iter()
        .map(|x| *x as usize)
        .collect();
    at += curve_count;
    let active_curves = records.iter().flatten().count();
    let updated = v[at..at + 8 * active_curves].to_vec();
    at += 8 * active_curves;
    let objective_after = v[at];
    assert_eq!(at + 1, v.len());
    Case {
        id: v[0],
        settings,
        weight,
        points,
        flags,
        indices,
        contours,
        edges,
        corners,
        curve_count,
        total,
        final_flags,
        records,
        parts,
        objective_before,
        gradient,
        rows,
        solution,
        offsets,
        updated,
        objective_after,
    }
}

/// Run the four stages with derivative records as native did, check every
/// stage output against the frozen row, then install the frozen curves so the
/// pass itself is compared without accumulated fitting rounding.
fn fit(case: &Case) -> FittedContours {
    let mut fitted = fit_contours_with_records(
        &case.contours,
        &case.edges,
        &case.points,
        &case.flags,
        &case.indices,
        case.settings,
    )
    .unwrap_or_else(|e| panic!("case {}: {e}", case.id));
    fitted.curves = stage_curves(case, &fitted);
    fitted
}

fn stage_curves(case: &Case, fitted: &FittedContours) -> Vec<Option<Cubic>> {
    assert_eq!(fitted.curves.len(), case.curve_count, "case {}", case.id);
    for (i, (flag, index)) in case.final_flags.iter().enumerate() {
        assert_eq!((fitted.flags[i], fitted.indices[i]), (*flag, *index));
    }
    let mut curves = Vec::new();
    for (slot, expected) in fitted.records.iter().zip(&case.records) {
        match (slot, expected) {
            (Some(record), Some((active, curve, gradient, hessian, meta))) => {
                assert_eq!(record.active, *active);
                assert_eq!([record.contour, record.start, record.end], *meta);
                compare(&record.gradient[..*active], gradient, "gradient", case.id);
                let block: Vec<_> = (0..*active)
                    .flat_map(|i| record.hessian[i][..*active].to_vec())
                    .collect();
                compare(&block, hessian, "hessian", case.id);
                let points: [Point; 4] = std::array::from_fn(|i| Point {
                    x: curve[2 * i],
                    y: curve[2 * i + 1],
                });
                let owned: Vec<f64> = fitted.curves[curves.len()]
                    .unwrap()
                    .points
                    .iter()
                    .flat_map(|p| [p.x, p.y])
                    .collect();
                compare(&owned, curve, "stage curve", case.id);
                curves.push(Some(Cubic { points }));
            }
            (other, None) => {
                assert!(
                    other.as_ref().is_none_or(|r| r.active == 0),
                    "case {}",
                    case.id
                );
                curves.push(None);
            }
            _ => panic!("case {}: record activity disagrees", case.id),
        }
    }
    for (parts, expected) in fitted.parts.iter().zip(&case.parts) {
        let actual: Vec<[usize; 7]> = parts
            .iter()
            .map(|p| {
                [
                    p.index,
                    p.kind as usize,
                    p.edge as usize,
                    p.start_node,
                    p.end_node,
                    p.start_position,
                    p.end_position,
                ]
            })
            .collect();
        assert_eq!(&actual, expected, "case {}", case.id);
    }
    curves
}

#[test]
fn optional_pass_matches_48_native_rows_through_assembly_and_update() {
    let mut systems = 0;
    let mut quadratic_updates = 0;
    for line in include_str!("../../fixtures/native-optimizer.csv").lines() {
        let case = parse(line);
        let fitted = fit(&case);
        let input = OptimizerInput {
            contours: &case.contours,
            corners: &case.corners,
            points: &case.points,
            parts: &fitted.parts,
            records: &fitted.records,
            weight: case.weight,
        };
        let before = objective(&input, &fitted.curves).unwrap();
        compare(&[before], &[case.objective_before], "objective", case.id);
        let system = assemble(&input, &fitted.curves).unwrap();
        assert_eq!(system.offsets, case.offsets, "case {}", case.id);
        assert_eq!(system.gradient.len(), case.total);
        compare(&system.gradient, &case.gradient, "gradient vector", case.id);
        for (r, (row, expected)) in system.rows.iter().zip(&case.rows).enumerate() {
            let keys: Vec<usize> = row.keys().copied().collect();
            let expected_keys: Vec<usize> = expected.iter().map(|e| e.0).collect();
            assert_eq!(keys, expected_keys, "case {} row {r}", case.id);
            let values: Vec<f64> = row.values().copied().collect();
            let expected_values: Vec<f64> = expected.iter().map(|e| e.1).collect();
            compare(&values, &expected_values, &format!("row {r}"), case.id);
        }
        let mut curves = fitted.curves.clone();
        apply(
            &fitted.records,
            &system.offsets,
            &case.solution,
            &mut curves,
        )
        .unwrap();
        compare(
            &flat(&curves, &fitted.records),
            &case.updated,
            "updated curves",
            case.id,
        );
        let after = objective(&input, &curves).unwrap();
        compare(
            &[after],
            &[case.objective_after],
            "objective after",
            case.id,
        );
        if case.total > 0 {
            systems += 1;
        }
        quadratic_updates += fitted
            .records
            .iter()
            .flatten()
            .filter(|r| r.active == 2)
            .count();
    }
    assert!(systems >= 30 && quadratic_updates >= 10);
}

#[test]
fn assembled_gradient_matches_finite_differences_of_the_objective() {
    // The original's own diagnostic 0x49f970 did this check; it is the proof
    // that the assembled derivatives belong to the objective 0x49ccb0.
    let mut checked = 0;
    for line in include_str!("../../fixtures/native-optimizer.csv").lines() {
        let case = parse(line);
        if case.total == 0 || case.id as usize % 4 != 1 {
            continue;
        }
        let fitted = fit(&case);
        let input = OptimizerInput {
            contours: &case.contours,
            corners: &case.corners,
            points: &case.points,
            parts: &fitted.parts,
            records: &fitted.records,
            weight: case.weight,
        };
        let system = assemble(&input, &fitted.curves).unwrap();
        let base = objective(&input, &fitted.curves).unwrap();
        for (index, record) in fitted.records.iter().enumerate() {
            let Some(record) = record else { continue };
            for c in 0..record.active {
                let h = 1e-6;
                let mut curves = fitted.curves.clone();
                let mut current = curves[index].unwrap();
                let value = read_coordinate(current, record.active, c).unwrap();
                write_coordinate(&mut current, record.active, c, value + h).unwrap();
                curves[index] = Some(current);
                let plus = objective(&input, &curves).unwrap();
                let mut current = fitted.curves[index].unwrap();
                write_coordinate(&mut current, record.active, c, value - h).unwrap();
                curves[index] = Some(current);
                let minus = objective(&input, &curves).unwrap();
                let numeric = (plus - minus) / (2. * h);
                let analytic = system.gradient[system.offsets[index] + c];
                assert!(
                    (numeric - analytic).abs() < 1e-4 * (1. + analytic.abs()),
                    "case {} coordinate {index}/{c}: {numeric} vs {analytic} (E={base})",
                    case.id
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 20);
}

#[test]
fn owned_solver_inverts_the_symmetric_system_and_reports_singularity() {
    let mut rows = vec![BTreeMap::new(); 4];
    let dense = [
        [4., 1., 0., 2.],
        [1., 3., 1., 0.],
        [0., 1., 2., 1.],
        [2., 0., 1., 5.],
    ];
    for (r, row) in dense.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if c >= r && *v != 0. {
                rows[r].insert(c, *v);
            }
        }
    }
    let b = [1., -2., 3., 0.5];
    let x = solve(&rows, &b).unwrap();
    for (r, row) in dense.iter().enumerate() {
        let product: f64 = row.iter().zip(&x).map(|(a, x)| a * x).sum();
        assert!((product - b[r]).abs() < 1e-12);
    }
    // Zero pivot on the diagonal must be handled by row exchange.
    let mut rows = vec![BTreeMap::new(); 2];
    rows[0].insert(1, 1.);
    rows[1].insert(1, 0.);
    let x = solve(&rows, &[2., 3.]).unwrap();
    assert!((x[1] - 2.).abs() < 1e-12 && (x[0] - 3.).abs() < 1e-12);
    // The upper triangle [[1,1],[.,1]] mirrors to the rank-one [[1,1],[1,1]].
    let mut singular = vec![BTreeMap::new(); 2];
    singular[0].insert(0, 1.);
    singular[0].insert(1, 1.);
    singular[1].insert(1, 1.);
    assert!(solve(&singular, &[1., 1.]).is_err());
    let mut singular = vec![BTreeMap::new(); 2];
    singular[0].insert(0, 0.);
    singular[1].insert(1, 1.);
    assert!(solve(&singular, &[1., 1.]).is_err());
    assert!(solve(&[BTreeMap::new()], &[1., 2.]).is_err());
}

#[test]
fn complete_optimizer_reduces_the_objective_on_a_native_row() {
    let mut improved = 0;
    for line in include_str!("../../fixtures/native-optimizer.csv").lines() {
        let case = parse(line);
        if case.total == 0 {
            continue;
        }
        let result = fit_contours_optimized(
            &case.contours,
            &case.edges,
            &case.corners,
            &case.points,
            &case.flags,
            &case.indices,
            case.settings,
            OptimizerSettings {
                weight: case.weight,
            },
        );
        let Ok((fitted, report)) = result else {
            continue;
        };
        assert_eq!(report.unknowns, case.total);
        assert!(fitted
            .curves
            .iter()
            .flatten()
            .all(|c| c.points.iter().all(|p| p.x.is_finite() && p.y.is_finite())));
        if report.objective_after < report.objective_before {
            improved += 1;
        }
    }
    // A Newton step on a non-convex penalty is not guaranteed to descend, but
    // the data term dominates these small systems.
    assert!(improved >= 15, "only {improved} rows improved");
}

#[test]
fn zero_length_tangents_are_rejected_instead_of_producing_nan() {
    let points = [
        Point { x: 0., y: 0. },
        Point { x: 1., y: 0. },
        Point { x: 1., y: 1. },
        Point { x: 0., y: 1. },
    ];
    let contours = vec![vec![0, 1, 2, 3]];
    let corners = vec![vec![false; 4]];
    let flat = Cubic {
        points: [points[0]; 4],
    };
    let curves = vec![Some(flat), Some(flat)];
    let record = DerivativeUpdate {
        gradient: [0.; 4],
        hessian: [[1.; 4]; 4],
        active: 4,
        contour: 0,
        start: 0,
        end: 2,
    };
    let records = vec![Some(record.clone()), Some(record)];
    let part = |index, start_position, end_position| FinalPart {
        index,
        kind: 1,
        edge: 0,
        start_node: 0,
        end_node: 0,
        start_position,
        end_position,
    };
    let parts = vec![vec![part(0, 0, 2), part(1, 2, 0)]];
    let input = OptimizerInput {
        contours: &contours,
        corners: &corners,
        points: &points,
        parts: &parts,
        records: &records,
        weight: 10.,
    };
    assert!(assemble(&input, &curves).is_err());
    assert!(objective(&input, &curves).is_err());
    let mut moved = curves.clone();
    assert!(optimize(&input, &mut moved).is_err());
}

/// The guarded pass of the improved defaults on the 48 native rows: where
/// the pass fails or its step raises the objective the curves are the plain
/// fit's, otherwise the unguarded pass's, to the bit either way; a
/// non-finite weight, which fails the unguarded pass, leaves the plain fit.
#[test]
fn guarded_optimizer_keeps_the_plain_fit_when_the_step_fails_or_rises() {
    let bits = |curves: &[Option<Cubic>]| -> Vec<Option<Vec<u64>>> {
        curves
            .iter()
            .map(|c| {
                c.map(|c| {
                    c.points
                        .iter()
                        .flat_map(|p| [p.x.to_bits(), p.y.to_bits()])
                        .collect()
                })
            })
            .collect()
    };
    let (mut rows, mut kept, mut rose, mut failed) = (0, 0, 0, 0);
    for line in include_str!("../../fixtures/native-optimizer.csv").lines() {
        let case = parse(line);
        let Ok(plain) = fit_contours_with_records(
            &case.contours,
            &case.edges,
            &case.points,
            &case.flags,
            &case.indices,
            case.settings,
        ) else {
            continue;
        };
        rows += 1;
        for weight in [case.weight, f64::NAN] {
            let settings = OptimizerSettings { weight };
            let unguarded = fit_contours_optimized(
                &case.contours,
                &case.edges,
                &case.corners,
                &case.points,
                &case.flags,
                &case.indices,
                case.settings,
                settings,
            );
            let (guarded, outcome) = fit_contours_optimized_guarded(
                &case.contours,
                &case.edges,
                &case.corners,
                &case.points,
                &case.flags,
                &case.indices,
                case.settings,
                settings,
            )
            .unwrap();
            match unguarded {
                Err(_) => {
                    assert!(!outcome.kept && outcome.report.is_none());
                    assert_eq!(bits(&guarded.curves), bits(&plain.curves));
                    failed += 1;
                }
                Ok((stepped, report)) => {
                    assert_eq!(outcome.report.as_ref(), Some(&report));
                    if report.objective_after > report.objective_before {
                        assert!(!outcome.kept);
                        assert_eq!(bits(&guarded.curves), bits(&plain.curves));
                        rose += 1;
                    } else {
                        assert!(outcome.kept);
                        assert_eq!(bits(&guarded.curves), bits(&stepped.curves));
                        kept += 1;
                    }
                }
            }
        }
    }
    assert!(
        kept >= 15 && failed >= rows,
        "{rows} rows: {kept} kept, {rose} rose, {failed} failed"
    );
}
