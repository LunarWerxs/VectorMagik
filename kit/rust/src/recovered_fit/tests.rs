//! The tests of `recovered_fit`.

use super::*;
fn fixture(line: &str) -> Vec<f64> {
    line.split(',')
        .skip(1)
        .map(|s| s.parse().unwrap())
        .collect()
}
fn points(values: &[f64]) -> Vec<Point> {
    values
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| Point { x: v[0], y: v[1] })
        .collect()
}
#[test]
fn complete_interval_dispatch_matches_96_actual_native_cases() {
    let mut counts = [0; 4];
    for line in include_str!("../../fixtures/native-intervals.csv")
        .lines()
        .filter(|s| s.starts_with("interval,"))
    {
        let v = fixture(line);
        let n = v[1] as usize;
        let p = points(&v[4..4 + 2 * n]);
        let fit = fit_contour_interval(&p, v[2] as usize, v[3] as usize).unwrap();
        counts[fit.degree as usize] += 1;
        let actual: Vec<_> = fit
            .curve
            .points
            .into_iter()
            .flat_map(|p| [p.x, p.y])
            .chain([fit.squared_error])
            .collect();
        for (a, b) in actual.iter().zip(&v[4 + 2 * n..]) {
            assert!(a.to_bits() == b.to_bits(), "case {}: {a:e} != {b:e}", v[0]);
        }
    }
    assert_eq!(counts.iter().sum::<usize>(), 96);
    assert!(
        counts.into_iter().all(|n| n > 0),
        "every native dispatch branch must execute"
    );
}
#[test]
fn merge_and_shift_scheduler_matches_96_actual_native_cases() {
    let mut cases = 0;
    let mut merges = 0;
    let mut back = 0;
    let mut forward = 0;
    for line in include_str!("../../fixtures/native-intervals.csv")
        .lines()
        .filter(|s| s.starts_with("schedule,"))
    {
        let v = fixture(line);
        let n = v[1] as usize;
        let p = points(&v[8..8 + 2 * n]);
        let settings = ScheduleSettings {
            initial_threshold: v[4],
            final_threshold: v[5],
            ramp_fraction: v[6],
            passes: v[7] as usize,
        };
        let result = schedule_contour(&p, v[2] as usize, v[3] as usize, settings).unwrap();
        let ranges: Vec<_> = v[9 + 2 * n..v.len() - 1]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|r| FitInterval {
                start: r[0] as usize,
                length: r[1] as usize,
            })
            .collect();
        assert_eq!(ranges.len(), v[8 + 2 * n] as usize);
        assert_eq!(result.intervals, ranges, "native schedule case {}", v[0]);
        let expected_threshold = v[v.len() - 1];
        assert!(
            (result.threshold_after_passes - expected_threshold).abs()
                < 1e-12 * (1. + expected_threshold.abs())
        );
        assert_eq!(
            result.intervals.iter().map(|r| r.length).sum::<usize>(),
            (v[3] - v[2]) as usize
        );
        merges += result.merges;
        back += result.backward_shifts;
        forward += result.forward_shifts;
        cases += 1;
    }
    assert_eq!(cases, 96);
    assert!(
        merges > 0 && back > 0 && forward > 0,
        "insufficient decisions: {merges}/{back}/{forward}"
    );
}
#[test]
fn initial_shared_contour_traversal_matches_48_actual_native_calls() {
    let mut cases = 0;
    let mut calls = 0;
    for line in include_str!("../../fixtures/native-initial.csv").lines() {
        let v = fixture(line);
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
        for r in v[7..7 + 4 * n].as_chunks::<4>().0 {
            points.push(Point { x: r[0], y: r[1] });
            flags.push(r[2] as u8);
            indices.push(r[3] as usize);
        }
        let mut offset = 7 + 4 * n;
        let mut contours = Vec::new();
        for _ in 0..nc {
            let count = v[offset] as usize;
            offset += 1;
            contours.push(
                v[offset..offset + count]
                    .iter()
                    .map(|x| *x as usize)
                    .collect(),
            );
            offset += count;
        }
        let result =
            initialize_contours(&contours, &points, &mut flags, &mut indices, settings).unwrap();
        assert_eq!(
            result.curve_count, v[offset] as usize,
            "native initial case {}",
            v[0]
        );
        assert_eq!(result.curve_count, v[offset + 1] as usize);
        let actual: Vec<_> = flags.into_iter().zip(indices).collect();
        let expected: Vec<_> = v[offset + 2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|r| (r[0] as u8, r[1] as usize))
            .collect();
        assert_eq!(actual, expected, "native initial case {}", v[0]);
        cases += 1;
        calls += result.interval_calls;
    }
    assert_eq!(cases, 48);
    assert!(calls >= 48);
}
#[test]
fn initial_traversal_preserves_empty_and_already_marked_nodes() {
    let settings = ScheduleSettings {
        initial_threshold: 0.01,
        final_threshold: 1.,
        ramp_fraction: 0.5,
        passes: 4,
    };
    let points = [Point { x: 1., y: 2. }; 3];
    let mut flags = [3, 4, 5];
    let mut indices = [22, 33, 44];
    let result = initialize_contours(
        &[vec![], vec![0, 1, 2]],
        &points,
        &mut flags,
        &mut indices,
        settings,
    )
    .unwrap();
    assert_eq!(result.curve_count, 0);
    assert_eq!(result.interval_calls, 0);
    assert_eq!(result.last_interval, None);
    assert_eq!(flags, [3, 4, 5]);
    assert_eq!(indices, [22, 33, 44]);
    assert!(initialize_contours(&[vec![3]], &points, &mut flags, &mut indices, settings).is_err());
    assert!(initialize_contours(&[], &points, &mut flags[..2], &mut indices, settings).is_err());
}
#[test]
fn scheduled_node_states_and_indices_match_96_native_calls() {
    let mut cases = 0;
    for line in include_str!("../../fixtures/native-intervals.csv")
        .lines()
        .filter(|s| s.starts_with("marks,"))
    {
        let v = fixture(line);
        let n = v[1] as usize;
        let p = points(&v[8..8 + 2 * n]);
        let start = v[2] as usize;
        let end = v[3] as usize;
        let schedule = schedule_contour(
            &p,
            start,
            end,
            ScheduleSettings {
                initial_threshold: v[4],
                final_threshold: v[5],
                ramp_fraction: v[6],
                passes: v[7] as usize,
            },
        )
        .unwrap();
        let base = 8 + 2 * n;
        let marks = mark_schedule(&schedule.intervals, n, start, end, v[base] as usize).unwrap();
        assert_eq!(marks.next_curve_index, v[base + 1] as usize);
        let mut states: Vec<_> = (0..n)
            .map(|i| {
                let id = if v[0] as usize % 2 == 1 { n - 1 - i } else { i };
                (6 + (id % 4) as u8, 1000 + id)
            })
            .collect();
        for mark in marks.writes {
            let state = &mut states[mark.position];
            state.0 = mark.state;
            if let Some(index) = mark.curve_index {
                state.1 = index;
            }
        }
        let expected: Vec<_> = v[base + 2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|r| (r[0] as u8, r[1] as usize))
            .collect();
        assert_eq!(states, expected, "native node-marking case {}", v[0]);
        cases += 1;
    }
    assert_eq!(cases, 96);
}
#[test]
fn marking_preserves_write_order_terminal_index_and_invalid_schedule_errors() {
    let ranges = [
        FitInterval {
            start: 0,
            length: 2,
        },
        FitInterval {
            start: 2,
            length: 2,
        },
    ];
    let result = mark_schedule(&ranges, 4, 1, 5, 7).unwrap();
    assert_eq!(result.next_curve_index, 10);
    assert_eq!(
        result.writes.iter().map(|m| m.position).collect::<Vec<_>>(),
        vec![1, 2, 3, 0, 1, 1, 1]
    );
    assert_eq!(
        result.writes.iter().map(|m| m.state).collect::<Vec<_>>(),
        vec![5, 1, 5, 1, 5, 4, 4]
    );
    // Two contour positions share one physical node. Ordered writes must
    // retain the index from an earlier interior write at the final endpoint.
    let ids = [0, 0, 1, 2];
    let mut nodes = [(9, 99); 3];
    for mark in result.writes {
        let node = &mut nodes[ids[mark.position]];
        node.0 = mark.state;
        if let Some(index) = mark.curve_index {
            node.1 = index;
        }
    }
    assert_eq!(nodes, [(4, 8), (1, 7), (5, 99)]);
    assert!(mark_schedule(&ranges, 4, 0, 3, 7).is_err());
    assert!(mark_schedule(
        &[FitInterval {
            start: 1,
            length: 4
        }],
        4,
        0,
        4,
        0
    )
    .is_err());
    assert!(mark_schedule(&ranges, 4, 0, 4, usize::MAX).is_err());
}
#[test]
fn threshold_equality_does_not_merge_and_pass_order_is_preserved() {
    let p = [
        Point { x: 0., y: 0. },
        Point { x: 1., y: 1. },
        Point { x: 2., y: 0. },
    ];
    let e = fit_contour_interval(&p, 0, 2).unwrap().squared_error;
    let settings = ScheduleSettings {
        initial_threshold: e,
        final_threshold: e,
        ramp_fraction: 0.5,
        passes: 1,
    };
    let exact = schedule_contour(&p, 0, 2, settings).unwrap();
    assert_eq!(exact.intervals.len(), 2);
    let above = schedule_contour(
        &p,
        0,
        2,
        ScheduleSettings {
            initial_threshold: e + 1e-6,
            ..settings
        },
    )
    .unwrap();
    assert_eq!(above.intervals.len(), 1);
    let flat = vec![Point { x: 0., y: 0. }; 9];
    let one = schedule_contour(
        &flat,
        0,
        8,
        ScheduleSettings {
            initial_threshold: 1.,
            final_threshold: 4.,
            passes: 1,
            ..settings
        },
    )
    .unwrap();
    assert_eq!(
        one.intervals.iter().map(|x| x.length).collect::<Vec<_>>(),
        vec![2, 2, 2, 2]
    );
}
#[test]
fn quadratic_ridge_and_degenerate_interval_contracts_are_explicit() {
    let p = Point { x: 11., y: 23. };
    let (q, error) = fit_quadratic(p, p, &[(0., p), (1., p)]).unwrap();
    assert_eq!(q[1], Point { x: 0., y: 0. });
    assert_eq!(error, 0.);
    assert!(fit_quadratic(p, p, &[(f64::NAN, p)]).is_err());
    assert!(fit_contour_interval(&[], 0, 1).is_err());
    assert!(fit_contour_interval(&[p], 0, 2).is_err());
    let fit = fit_contour_interval(&[p; 5], 0, 4).unwrap();
    assert_eq!(fit.squared_error, 0.);
}
