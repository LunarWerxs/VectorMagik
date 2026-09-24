use super::*;
use crate::recovered_fit::{initialize_contours, ScheduleSettings};

fn values(line: &str) -> Vec<f64> {
    line.split(',')
        .skip(1)
        .map(|v| v.parse().unwrap())
        .collect()
}
fn point(v: &[f64]) -> Point {
    Point { x: v[0], y: v[1] }
}
fn apply(record: &mut [f64], d: &DerivativeUpdate) {
    for i in 0..d.active {
        record[i] = d.gradient[i];
        for j in 0..d.active {
            record[4 + i * 4 + j] = d.hessian[i][j];
        }
    }
    record[20] = d.active as f64;
    record[22] = d.contour as f64;
    record[23] = d.start as f64;
    record[24] = d.end as f64;
}
fn compare(actual: &[f64], expected: &[f64], case: f64) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (a - b).abs() < 1e-7 * (1. + b.abs()),
            "case {case}, field {i}: {a} != {b}"
        );
    }
}
#[test]
fn preparation_matches_native_for_every_byte_state_and_empty_contours() {
    let mut count = 0;
    for line in include_str!("../../fixtures/native-preparation.csv").lines() {
        let v = values(line);
        let n = v[1] as usize;
        let lengths: Vec<_> = v[2..2 + n].iter().map(|x| *x as usize).collect();
        let mut flags: Vec<_> = (0..256)
            .map(|i| ((i + v[0] as usize) % 256) as u8)
            .collect();
        let capacity = prepare_fitting(&lengths, &mut flags).unwrap();
        assert_eq!(capacity, v[2 + n] as usize);
        assert_eq!(
            flags,
            v[3 + n..].iter().map(|x| *x as u8).collect::<Vec<_>>()
        );
        count += 1;
    }
    assert_eq!(count, 16);
    let mut flags = [4, 1];
    assert!(prepare_fitting(&[usize::MAX], &mut flags).is_err());
    assert_eq!(flags, [4, 1]);
}
#[test]
fn derivative_record_matches_96_native_calls_and_preserves_unwritten_fields() {
    let mut branches = [0; 4];
    for line in include_str!("../../fixtures/native-derivative.csv").lines() {
        let v = values(line);
        let n = v[1] as usize;
        let start = v[2] as usize;
        let end = v[3] as usize;
        let points: Vec<_> = v[4..4 + 2 * n]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| point(p))
            .collect();
        let offset = 4 + 2 * n;
        let curve = Cubic {
            points: std::array::from_fn(|i| point(&v[offset + 2 * i..])),
        };
        let middle = point(&v[offset + 8..]);
        let unwrapped = if end < start { end + n } else { end };
        let interior: Vec<_> = (start + 1..unwrapped).map(|p| points[p % n]).collect();
        let update = derivative_record(curve, middle, &interior, 0, start, end).unwrap();
        let mut actual: Vec<_> = (0..20)
            .map(|i| 1000. + i as f64)
            .chain((0..6).map(|i| 77. + i as f64))
            .collect();
        if let Some(d) = update {
            branches[match d.active {
                0 => 1,
                2 => 2,
                4 => 3,
                _ => unreachable!(),
            }] += 1;
            apply(&mut actual, &d);
        } else {
            branches[0] += 1;
        }
        compare(&actual, &v[offset + 10..], v[0]);
    }
    assert_eq!(branches.iter().sum::<usize>(), 96);
    assert!(branches.iter().all(|n| *n > 0));
}
#[test]
fn shared_contour_refinement_matches_48_original_calls_with_and_without_records() {
    let mut count = 0;
    let mut fitted = 0;
    let mut dimensions = [0; 3];
    for line in include_str!("../../fixtures/native-refinement.csv").lines() {
        let v = values(line);
        let n = v[1] as usize;
        let nc = v[2] as usize;
        let settings = ScheduleSettings {
            initial_threshold: v[3],
            final_threshold: v[4],
            ramp_fraction: v[5],
            passes: v[6] as usize,
        };
        let mut points = Vec::new();
        let mut flags = Vec::new();
        let mut indices = Vec::new();
        for p in v[7..7 + 4 * n].as_chunks::<4>().0 {
            points.push(point(p));
            flags.push(p[2] as u8);
            indices.push(p[3] as usize);
        }
        let mut offset = 7 + 4 * n;
        let mut contours = Vec::new();
        for _ in 0..nc {
            let size = v[offset] as usize;
            offset += 1;
            contours.push(
                v[offset..offset + size]
                    .iter()
                    .map(|x| *x as usize)
                    .collect(),
            );
            offset += size;
        }
        let initial =
            initialize_contours(&contours, &points, &mut flags, &mut indices, settings).unwrap();
        assert_eq!(initial.curve_count, v[offset] as usize);
        let derivatives = v[offset + 1] != 0.;
        let active = v[offset + 2] as usize;
        offset += 3;
        let result = refine_contours(
            &contours,
            &points,
            &mut flags,
            &indices,
            initial.curve_count,
            derivatives,
        )
        .unwrap();
        assert_eq!(result.active_coordinates, active);
        let node_state: Vec<_> = flags
            .iter()
            .zip(&indices)
            .flat_map(|(f, i)| [*f as f64, *i as f64])
            .collect();
        assert_eq!(node_state, &v[offset..offset + 2 * n]);
        offset += 2 * n;
        let mut curves: Vec<_> = (0..initial.curve_count * 8)
            .map(|i| 5000. + i as f64 * 0.125)
            .collect();
        let mut records: Vec<Vec<_>> = (0..initial.curve_count)
            .map(|j| {
                (0..20)
                    .map(|i| 1000. + (j * 20 + i) as f64)
                    .chain((0..6).map(|i| if i == 0 { 0. } else { 77. + i as f64 }))
                    .collect()
            })
            .collect();
        for change in result.curves {
            fitted += 1;
            for (target, p) in curves[change.index * 8..change.index * 8 + 8]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(change.fit.curve.points)
            {
                target[0] = p.x;
                target[1] = p.y;
            }
            if let Some(d) = change.derivative {
                dimensions[d.active / 2] += 1;
                apply(&mut records[change.index], &d);
            }
        }
        compare(&curves, &v[offset..offset + curves.len()], v[0]);
        offset += curves.len();
        if derivatives {
            compare(
                &records.into_iter().flatten().collect::<Vec<_>>(),
                &v[offset..],
                v[0],
            );
        } else {
            assert_eq!(offset, v.len());
        }
        count += 1;
    }
    assert_eq!(count, 48);
    assert!(fitted > 48);
    assert!(dimensions.iter().all(|n| *n > 0));
}
#[test]
fn refinement_leaves_unvisited_slots_untouched_and_rejects_bad_indices() {
    let points = [Point { x: 0., y: 0. }; 4];
    let mut flags = [4, 1, 1, 4];
    let indices = [0, 7, 7, 0];
    assert!(refine_contours(&[vec![0, 1, 2, 3]], &points, &mut flags, &indices, 2, false).is_err());
    assert_eq!(flags, [4, 1, 1, 4]);
    let result = refine_contours(&[vec![]], &points, &mut flags, &indices, 2, true).unwrap();
    assert!(result.curves.is_empty());
    assert_eq!(result.active_coordinates, 0);
}

#[test]
fn finalization_matches_96_native_cases_including_orientation_threshold_equality() {
    let mut count = 0;
    let mut kinds = [0; 3];
    for line in include_str!("../../fixtures/native-finalization.csv").lines() {
        let v = values(line);
        let n = v[1] as usize;
        let nc = v[2] as usize;
        let curves = v[3] as usize;
        let mut points = Vec::new();
        let mut flags = Vec::new();
        let mut indices = Vec::new();
        for row in v[4..4 + 4 * n].as_chunks::<4>().0 {
            points.push(point(row));
            flags.push(row[2] as u8);
            indices.push(row[3] as usize);
        }
        let mut offset = 4 + 4 * n;
        let starts: Vec<_> = v[offset..offset + 2 * curves]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| point(p))
            .collect();
        offset += 2 * curves;
        let mut contours = Vec::new();
        for _ in 0..nc {
            let size = v[offset] as usize;
            offset += 1;
            let ids: Vec<_> = v[offset..offset + size]
                .iter()
                .map(|x| *x as usize)
                .collect();
            offset += size;
            let edges: Vec<_> = v[offset..offset + size].iter().map(|x| *x as u32).collect();
            offset += size;
            contours.push((ids, edges));
        }
        for (ids, edges) in contours {
            let actual =
                finalize_contour(&ids, &edges, &points, &flags, &indices, &starts).unwrap();
            let size = v[offset] as usize;
            offset += 1;
            assert_eq!(actual.len(), size);
            for (part, row) in actual
                .into_iter()
                .zip(v[offset..offset + 7 * size].as_chunks::<7>().0)
            {
                kinds[part.kind as usize] += 1;
                assert_eq!(
                    [
                        part.index,
                        part.kind as usize,
                        part.edge as usize,
                        part.start_node,
                        part.end_node,
                        part.start_position,
                        part.end_position
                    ],
                    std::array::from_fn::<usize, 7, _>(|i| row[i] as usize),
                    "native finalization case {}",
                    v[0]
                );
                if v[0] < 4. && part.kind != 0 {
                    assert_eq!(part.kind, 1);
                }
                if (4. ..8.).contains(&v[0]) && part.kind != 0 {
                    assert_eq!(part.kind, 2);
                }
            }
            offset += 7 * size;
        }
        assert_eq!(offset, v.len());
        count += 1;
    }
    assert_eq!(count, 96);
    assert!(kinds.iter().all(|n| *n > 0));
}

#[test]
fn finalization_rejects_unbounded_runs_and_missing_curve_slots() {
    let ids = [0, 1, 2];
    let edges = [10, 20, 30];
    let points = [Point { x: 0., y: 0. }; 3];
    assert!(finalize_contour(&ids, &edges, &points, &[2; 3], &[0; 3], &points).is_err());
    assert!(finalize_contour(&ids, &edges, &points, &[4, 2, 4], &[9; 3], &points).is_err());
    assert!(finalize_contour(&[], &[], &points, &[4; 3], &[0; 3], &points).is_err());
}

#[test]
fn complete_owned_fitting_matches_48_native_stage_sequences() {
    let mut unused = 0;
    let mut kinds = [0; 3];
    for line in include_str!("../../fixtures/native-whole-fitting.csv").lines() {
        let v = values(line);
        let n = v[1] as usize;
        let nc = v[2] as usize;
        let settings = ScheduleSettings {
            initial_threshold: v[3],
            final_threshold: v[4],
            ramp_fraction: v[5],
            passes: v[6] as usize,
        };
        let mut points = Vec::new();
        let mut flags = Vec::new();
        let mut indices = Vec::new();
        for row in v[7..7 + 4 * n].as_chunks::<4>().0 {
            points.push(point(row));
            flags.push(row[2] as u8);
            indices.push(row[3] as usize);
        }
        let mut at = 7 + 4 * n;
        let mut contours = Vec::new();
        let mut edges = Vec::new();
        for _ in 0..nc {
            let count = v[at] as usize;
            at += 1;
            contours.push(v[at..at + count].iter().map(|x| *x as usize).collect());
            at += count;
            edges.push(v[at..at + count].iter().map(|x| *x as u32).collect());
            at += count;
        }
        let result = fit_contours(&contours, &edges, &points, &flags, &indices, settings).unwrap();
        assert_eq!(result.curves.len(), v[at] as usize);
        assert_eq!(v[at + 1], 0.);
        assert_eq!(v[at + 2], 0.);
        at += 3;
        for i in 0..n {
            assert_eq!(result.flags[i], v[at] as u8);
            assert_eq!(result.indices[i], v[at + 1] as usize);
            at += 2;
        }
        for (index, curve) in result.curves.iter().enumerate() {
            if let Some(curve) = curve {
                let actual: Vec<_> = curve.points.iter().flat_map(|p| [p.x, p.y]).collect();
                compare(&actual, &v[at..at + 8], v[0]);
            } else {
                unused += 1;
                for i in 0..8 {
                    assert_eq!(v[at + i], 5000. + (index * 8 + i) as f64 * 0.125);
                }
            }
            at += 8;
        }
        for parts in result.parts {
            assert_eq!(parts.len(), v[at] as usize);
            at += 1;
            for part in parts {
                kinds[part.kind as usize] += 1;
                let actual = [
                    part.index,
                    part.kind as usize,
                    part.edge as usize,
                    part.start_node,
                    part.end_node,
                    part.start_position,
                    part.end_position,
                ];
                assert_eq!(
                    actual,
                    std::array::from_fn::<usize, 7, _>(|i| v[at + i] as usize)
                );
                at += 7;
                if part.kind != 0 {
                    assert!(result.curves[part.index].is_some());
                }
            }
        }
        assert_eq!(at, v.len());
    }
    assert!(unused > 0 && kinds.iter().all(|n| *n > 0));
    let settings = ScheduleSettings {
        initial_threshold: 0.1,
        final_threshold: 1.,
        ramp_fraction: 0.5,
        passes: 3,
    };
    let empty = fit_contours(&[], &[], &[], &[], &[], settings).unwrap();
    assert!(empty.parts.is_empty() && empty.curves.is_empty());
    assert!(fit_contours(
        &[vec![0]],
        &[],
        &[Point { x: 0., y: 0. }],
        &[0],
        &[0],
        settings
    )
    .is_err());
}
