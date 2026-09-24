//! The anti-aliased smoother against the captures of `--aa-smoothing-fixtures`
//! and `--aa-smoothing-probe`: 42 blended images through the original import,
//! preprocessing, segmentation and contour construction, then the original
//! smoother whole, and stopped after every step.

use super::*;

pub(crate) struct Case {
    pub(crate) id: i64,
    pub(crate) preset: i64,
    pub(crate) image: AaImage,
    pub(crate) every: i32,
    pub(crate) units: [f64; 4],
    pub(crate) border: [i32; 4],
    pub(crate) nodes: Vec<Node>,
    pub(crate) contours: Vec<Contour>,
    pub(crate) seed: i32,
    pub(crate) params: Params,
    pub(crate) rest: Vec<f64>,
}

struct State {
    nodes: Vec<(f64, f64, u8, u8, u8)>,
    contours: Vec<(f64, f64, f64, i32, Vec<i32>)>,
    seed: i32,
}

fn take(v: &[f64], at: &mut usize) -> f64 {
    let x = v[*at];
    *at += 1;
    x
}
fn take_i(v: &[f64], at: &mut usize) -> i32 {
    take(v, at) as i32
}

pub(crate) fn parse(line: &str) -> Case {
    let mut fields = line.split(',');
    fields.next();
    let v: Vec<f64> = fields.map(|f| f.parse().unwrap()).collect();
    let mut at = 0;
    let id = take(&v, &mut at) as i64;
    let preset = take(&v, &mut at) as i64;
    let _pattern = take(&v, &mut at);
    let width = take(&v, &mut at) as i32;
    let height = take(&v, &mut at) as i32;
    let regions = take(&v, &mut at) as usize;
    let count = (width * height) as usize;
    let pixels: Vec<u8> = (0..4 * count).map(|_| take_i(&v, &mut at) as u8).collect();
    let labels: Vec<i32> = (0..count).map(|_| take_i(&v, &mut at)).collect();
    let dx: Vec<i32> = (0..8).map(|_| take_i(&v, &mut at)).collect();
    let dy: Vec<i32> = (0..8).map(|_| take_i(&v, &mut at)).collect();
    assert_eq!(dx, [1, 0, -1, 0, -1, 1, -1, 1]);
    assert_eq!(dy, [0, 1, 0, -1, 1, -1, -1, 1]);
    let every = take_i(&v, &mut at);
    let units = [
        take(&v, &mut at),
        take(&v, &mut at),
        take(&v, &mut at),
        take(&v, &mut at),
    ];
    let border = [
        take_i(&v, &mut at),
        take_i(&v, &mut at),
        take_i(&v, &mut at),
        take_i(&v, &mut at),
    ];
    let n = take(&v, &mut at) as usize;
    let nodes: Vec<Node> = (0..n)
        .map(|_| Node {
            x: take(&v, &mut at),
            y: take(&v, &mut at),
            fx: take(&v, &mut at) as f32,
            fy: take(&v, &mut at) as f32,
            state: take_i(&v, &mut at) as u8,
            flags: take_i(&v, &mut at) as u8,
            aux: take_i(&v, &mut at) as u8,
        })
        .collect();
    let mut contours = Vec::with_capacity(regions);
    let mut region_colors = Vec::new();
    for _ in 0..regions {
        let pixels = take_i(&v, &mut at);
        let region = take_i(&v, &mut at);
        let parent = take_i(&v, &mut at);
        let color = [
            take_i(&v, &mut at) as u8,
            take_i(&v, &mut at) as u8,
            take_i(&v, &mut at) as u8,
            take_i(&v, &mut at) as u8,
        ];
        let floats = [
            take(&v, &mut at) as f32,
            take(&v, &mut at) as f32,
            take(&v, &mut at) as f32,
            take(&v, &mut at) as f32,
        ];
        let k = take(&v, &mut at) as usize;
        let ids: Vec<i32> = (0..k).map(|_| take_i(&v, &mut at)).collect();
        let edges: Vec<Edge> = (0..k)
            .map(|_| Edge {
                flag: 0,
                other: take_i(&v, &mut at),
                steps: take_i(&v, &mut at),
            })
            .collect();
        if region_colors.len() <= region as usize {
            region_colors.resize(region as usize + 1, [0.0; 4]);
        }
        region_colors[region as usize] = floats;
        contours.push(Contour {
            pixels,
            area: 0.0,
            nodes: ids,
            edges,
            region,
            color,
            parent,
            dir: [0.0; 2],
            parity: 0,
        });
    }
    let seed = take_i(&v, &mut at);
    let params = Params::from_values(&v[at..at + PARAM_VALUES]).unwrap();
    at += PARAM_VALUES;
    Case {
        id,
        preset,
        image: AaImage {
            width,
            height,
            pixels,
            labels,
            region_colors,
        },
        every,
        units,
        border,
        nodes,
        contours,
        seed,
        params,
        rest: v[at..].to_vec(),
    }
}

fn state(v: &[f64], at: &mut usize) -> State {
    let n = take(v, at) as usize;
    let nodes = (0..n)
        .map(|_| {
            (
                take(v, at),
                take(v, at),
                take_i(v, at) as u8,
                take_i(v, at) as u8,
                take_i(v, at) as u8,
            )
        })
        .collect();
    let m = take(v, at) as usize;
    let contours = (0..m)
        .map(|_| {
            let area = take(v, at);
            let dx = take(v, at);
            let dy = take(v, at);
            let parity = take_i(v, at);
            let k = take(v, at) as usize;
            (
                area,
                dx,
                dy,
                parity,
                (0..k).map(|_| take_i(v, at)).collect(),
            )
        })
        .collect();
    let seed = take_i(v, at);
    State {
        nodes,
        contours,
        seed,
    }
}

fn check_state(
    case: &Case,
    nodes: &[Node],
    contours: &[Contour],
    seed: i32,
    expected: &State,
    what: &str,
) {
    assert_eq!(
        nodes.len(),
        expected.nodes.len(),
        "case {} {what}: node count",
        case.id
    );
    for (i, (node, want)) in nodes.iter().zip(&expected.nodes).enumerate() {
        assert_eq!(
            (node.state, node.flags, node.aux),
            (want.2, want.3, want.4),
            "case {} (preset {}) {what} node {i} bytes",
            case.id,
            case.preset
        );
        assert!(
            node.x == want.0 && node.y == want.1,
            "case {} (preset {}) {what} node {i}: ({:?}, {:?}) vs native ({:?}, {:?})",
            case.id,
            case.preset,
            node.x,
            node.y,
            want.0,
            want.1
        );
    }
    assert_eq!(contours.len(), expected.contours.len());
    for (i, (contour, want)) in contours.iter().zip(&expected.contours).enumerate() {
        assert!(
            contour.area == want.0,
            "case {} {what} contour {i} area {:?} vs native {:?}",
            case.id,
            contour.area,
            want.0
        );
        assert!(
            contour.dir[0] == want.1 && contour.dir[1] == want.2,
            "case {} {what} contour {i} ray {:?} vs native ({:?}, {:?})",
            case.id,
            contour.dir,
            want.1,
            want.2
        );
        assert_eq!(
            contour.parity, want.3,
            "case {} {what} contour {i} parity",
            case.id
        );
        let got: Vec<i32> = contour.edges.iter().map(|e| e.flag).collect();
        assert_eq!(
            &got, &want.4,
            "case {} {what} contour {i} entry flags",
            case.id
        );
    }
    assert_eq!(seed, expected.seed, "case {} {what}: seed", case.id);
}

fn input(case: &Case, prepare: bool) -> AaInput<'_> {
    AaInput {
        image: &case.image,
        seed: case.seed,
        every: case.every,
        units: case.units,
        prepare,
    }
}

pub(crate) fn cases(text: &str, prefix: &str) -> Vec<Case> {
    text.lines()
        .filter(|l| l.starts_with(prefix))
        .map(parse)
        .collect()
}

#[test]
fn anti_aliased_smoothing_matches_42_native_runs() {
    let mut marked = 0;
    for case in cases(
        include_str!("../../fixtures/native-aa-smoothing.csv"),
        "aasmoothing,",
    ) {
        let mut nodes = case.nodes.clone();
        let mut contours = case.contours.clone();
        let canvas = Canvas {
            width: case.image.width,
            height: case.image.height,
            border: case.border,
        };
        let mut at = 0;
        let expected = state(&case.rest, &mut at);
        assert_eq!(at, case.rest.len(), "case {}: trailing values", case.id);
        let report = smooth(
            &mut nodes,
            &mut contours,
            canvas,
            &case.params,
            Some(input(&case, true)),
        )
        .unwrap();
        check_state(
            &case,
            &nodes,
            &contours,
            report.seed,
            &expected,
            "after smoothing",
        );
        marked += report.marked;
    }
    let _ = marked;
}

#[test]
fn every_anti_aliased_step_matches_42_native_probes() {
    let mut phases = 0;
    let mut crossings = 0usize;
    for case in cases(
        include_str!("../../fixtures/native-aa-smoothing-probe.csv"),
        "aaprobe,",
    ) {
        let mut nodes = case.nodes.clone();
        let mut contours = case.contours.clone();
        let n = nodes.len();
        let canvas = Canvas {
            width: case.image.width,
            height: case.image.height,
            border: case.border,
        };
        let aa = input(&case, true);
        let mut s = Smoother {
            nodes: &mut nodes,
            contours: &mut contours,
            canvas,
            params: &case.params,
            phase: 0,
            near: Vec::new(),
            grad: vec![[0.0; 2]; n],
            blind_len: 0.0,
            aa: Some(AaState::new(aa.image, aa.every, aa.units, aa.seed)),
        };
        s.measurement_setup();
        s.find_near();
        s.build_neighbours();
        s.perturb();
        let mut at = 0;
        assert_eq!(take_i(&case.rest, &mut at), -4);
        let count = take(&case.rest, &mut at) as usize;
        assert_eq!(count, n);
        for i in 0..n {
            let k = take(&case.rest, &mut at) as usize;
            let want: Vec<(usize, usize, usize)> = (0..k)
                .map(|_| {
                    (
                        take(&case.rest, &mut at) as usize,
                        take(&case.rest, &mut at) as usize,
                        take(&case.rest, &mut at) as usize,
                    )
                })
                .collect();
            // The kd-tree's tie order is not reproduced; the inversion
            // pass only sums over the list, so the set is what matters.
            let aa = s.aa.as_ref().unwrap();
            let mut got: Vec<(usize, usize, usize)> = aa.neighbours[i]
                .iter()
                .map(|&p| (p, aa.pairs[p].0, aa.pairs[p].1))
                .collect();
            got.sort();
            let mut want = want;
            want.sort();
            assert_eq!(
                got, want,
                "case {} (preset {}): neighbours of node {i}",
                case.id, case.preset
            );
        }
        let expected = state(&case.rest, &mut at);
        let seed = s.aa.as_ref().unwrap().seed;
        check_state(&case, s.nodes, s.contours, seed, &expected, "after setup");
        assert_eq!(take_i(&case.rest, &mut at), -5);
        let energy = take(&case.rest, &mut at);
        s.phase = 0;
        let e = s.objective();
        let w = case.image.width as usize;
        let h = case.image.height as usize;
        for cell in 0..w * h {
            let crossed = take_i(&case.rest, &mut at) != 0;
            let model = [
                take(&case.rest, &mut at) as f32,
                take(&case.rest, &mut at) as f32,
                take(&case.rest, &mut at) as f32,
                take(&case.rest, &mut at) as f32,
            ];
            let aa = s.aa.as_ref().unwrap();
            assert_eq!(
                aa.meas.crossed(cell),
                crossed,
                "case {} (preset {}): pixel {} ({}, {}) crossed",
                case.id,
                case.preset,
                cell,
                cell % w,
                cell / w
            );
            assert!(
                aa.meas.model[cell] == model,
                "case {} (preset {}): pixel {} ({}, {}) model {:?} vs native {:?}",
                case.id,
                case.preset,
                cell,
                cell % w,
                cell / w,
                aa.meas.model[cell],
                model
            );
            if crossed {
                crossings += 1;
            }
        }
        assert!(
            e == energy,
            "case {} (preset {}): energy {e:?} vs native {energy:?}",
            case.id,
            case.preset
        );
        s.gradient();
        for i in 0..n {
            for c in 0..2 {
                let want = take(&case.rest, &mut at);
                let got = s.grad[i][c];
                assert!(
                    got == want,
                    "case {} (preset {}): gradient of node {i} coordinate {c}: {got:?} vs native {want:?}",
                    case.id,
                    case.preset
                );
            }
        }
        loop {
            let tag = take_i(&case.rest, &mut at);
            if tag == -3 {
                break;
            }
            let phase = tag as usize;
            let iterations = take_i(&case.rest, &mut at);
            let line_search = take_i(&case.rest, &mut at);
            let restarts = take_i(&case.rest, &mut at);
            let final_energy = take(&case.rest, &mut at);
            let expected = state(&case.rest, &mut at);
            assert_ne!(case.params.is_enabled[phase], 0);
            s.phase = phase;
            let mut cg = Cg {
                p: case.params.phases[phase],
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
            cg.run(&mut s).unwrap();
            let seed = s.aa.as_ref().unwrap().seed;
            check_state(
                &case,
                s.nodes,
                s.contours,
                seed,
                &expected,
                &format!("after phase {phase}"),
            );
            assert_eq!(
                (cg.iteration, cg.ls_iterations, cg.restarts),
                (iterations, line_search, restarts),
                "case {} (preset {}) phase {phase}: counters",
                case.id,
                case.preset
            );
            assert!(
                cg.energy == final_energy,
                "case {} phase {phase}: final energy {:?} vs native {final_energy:?}",
                case.id,
                cg.energy
            );
            phases += 1;
            if case.params.do_puncture_corners[phase] != 0 {
                assert_eq!(take_i(&case.rest, &mut at), -1);
                let expected = state(&case.rest, &mut at);
                s.puncture().unwrap();
                check_state(
                    &case,
                    s.nodes,
                    s.contours,
                    seed,
                    &expected,
                    &format!("after puncturing phase {phase}"),
                );
            }
        }
        let post = take_i(&case.rest, &mut at);
        let expected = state(&case.rest, &mut at);
        assert_eq!(at, case.rest.len(), "case {}: trailing values", case.id);
        let got = s.node_set_pass();
        let seed = s.aa.as_ref().unwrap().seed;
        check_state(&case, s.nodes, s.contours, seed, &expected, "at the end");
        assert_eq!(got, post, "case {}: closing pass count", case.id);
    }
    assert!(phases >= 100, "{phases} phases ran");
    assert!(crossings > 1000, "{crossings} crossed pixels");
}

/// Phase 0 iteration by iteration against `--aa-smoothing-iterations`: the
/// state the setup left is kept, and the phase reruns from it with
/// `cg_max_iter` = 1, 2, ... exactly as the capture did.
#[test]
fn phase_0_matches_native_iteration_by_iteration() {
    let mut runs = 0;
    for case in cases(
        include_str!("../../fixtures/native-aa-smoothing-iterations.csv"),
        "aaiter,",
    ) {
        let mut nodes = case.nodes.clone();
        let mut contours = case.contours.clone();
        let n = nodes.len();
        let canvas = Canvas {
            width: case.image.width,
            height: case.image.height,
            border: case.border,
        };
        let aa = input(&case, true);
        let mut s = Smoother {
            nodes: &mut nodes,
            contours: &mut contours,
            canvas,
            params: &case.params,
            phase: 0,
            near: Vec::new(),
            grad: vec![[0.0; 2]; n],
            blind_len: 0.0,
            aa: Some(AaState::new(aa.image, aa.every, aa.units, aa.seed)),
        };
        s.measurement_setup();
        s.find_near();
        s.build_neighbours();
        s.perturb();
        let mut at = 0;
        assert_eq!(take_i(&case.rest, &mut at), -4);
        let expected = state(&case.rest, &mut at);
        let seed = s.aa.as_ref().unwrap().seed;
        check_state(&case, s.nodes, s.contours, seed, &expected, "after setup");
        let saved_nodes = s.nodes.to_vec();
        let saved_contours = s.contours.to_vec();
        let saved_model = s.aa.as_ref().unwrap().meas.model.clone();
        loop {
            let tag = take_i(&case.rest, &mut at);
            if tag == -6 {
                break;
            }
            let it = tag;
            let iterations = take_i(&case.rest, &mut at);
            let line_search = take_i(&case.rest, &mut at);
            let restarts = take_i(&case.rest, &mut at);
            let stalled = take_i(&case.rest, &mut at);
            let final_energy = take(&case.rest, &mut at);
            let prev_best = take(&case.rest, &mut at);
            let expected = state(&case.rest, &mut at);
            let gradient: Vec<f64> = (0..2 * n).map(|_| take(&case.rest, &mut at)).collect();
            s.nodes.copy_from_slice(&saved_nodes);
            s.contours.clone_from_slice(&saved_contours);
            {
                let aa = s.aa.as_mut().unwrap();
                aa.meas.model.clone_from(&saved_model);
                aa.meas.changed.clear();
                aa.refresh_pixel_sums();
            }
            s.phase = 0;
            let mut p = case.params.phases[0];
            p.max_iter = it;
            let mut cg = Cg {
                p,
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
            cg.run(&mut s).unwrap();
            assert_eq!(
                (cg.iteration, cg.ls_iterations, cg.restarts, cg.stalled),
                (iterations, line_search, restarts, stalled),
                "case {} (preset {}) after {it} iteration(s): counters",
                case.id,
                case.preset
            );
            assert!(
                cg.energy == final_energy && cg.prev_best == prev_best,
                "case {} after {it} iteration(s): energy {:?} / best {:?} vs native {final_energy:?} / {prev_best:?}",
                case.id,
                cg.energy,
                cg.prev_best
            );
            let seed = s.aa.as_ref().unwrap().seed;
            check_state(
                &case,
                s.nodes,
                s.contours,
                seed,
                &expected,
                &format!("after {it} iteration(s)"),
            );
            s.gradient();
            for i in 0..n {
                for c in 0..2 {
                    let got = s.grad[i][c];
                    let want = gradient[2 * i + c];
                    assert!(
                        got == want,
                        "case {} (preset {}) after {it} iteration(s): gradient of node {i} coordinate {c}: {got:?} vs native {want:?}",
                        case.id,
                        case.preset
                    );
                }
            }
            runs += 1;
        }
        assert_eq!(at, case.rest.len(), "case {}: trailing values", case.id);
    }
    assert!(runs > 500, "{runs} runs");
}

/// The colour model at every pixel with two or more regions around it,
/// against `--aa-colour-model`: the region ids in the order the model
/// collects them, their weights (the two-region blend or the constrained
/// least-squares solve) and the model colour.
#[test]
fn colour_model_matches_every_native_pixel() {
    let mut solved = 0usize;
    for case in cases(
        include_str!("../../fixtures/native-aa-colour-model.csv"),
        "aacolour,",
    ) {
        let mut nodes = case.nodes.clone();
        let mut contours = case.contours.clone();
        let n = nodes.len();
        let canvas = Canvas {
            width: case.image.width,
            height: case.image.height,
            border: case.border,
        };
        let aa = input(&case, true);
        let mut s = Smoother {
            nodes: &mut nodes,
            contours: &mut contours,
            canvas,
            params: &case.params,
            phase: 0,
            near: Vec::new(),
            grad: vec![[0.0; 2]; n],
            blind_len: 0.0,
            aa: Some(AaState::new(aa.image, aa.every, aa.units, aa.seed)),
        };
        s.measurement_setup();
        let mut at = 0;
        assert_eq!(take_i(&case.rest, &mut at), -13);
        for y in 0..case.image.height {
            for x in 0..case.image.width {
                let count = s.neighbourhood(x, y);
                if count < 2 {
                    continue;
                }
                s.colour_model_evaluate(x, y);
                let cm = &s.aa().cm;
                let label = format!("case {} (preset {}) pixel ({x},{y})", case.id, case.preset);
                assert_eq!(take_i(&case.rest, &mut at), x, "{label}: x");
                assert_eq!(take_i(&case.rest, &mut at), y, "{label}: y");
                assert_eq!(
                    take(&case.rest, &mut at) as usize,
                    cm.n,
                    "{label}: region count"
                );
                for j in 0..cm.n {
                    assert_eq!(take_i(&case.rest, &mut at), cm.ids[j], "{label}: id {j}");
                }
                for j in 0..cm.n {
                    let want = take(&case.rest, &mut at);
                    assert!(
                        cm.weights[j] == want,
                        "{label}: weight {j} of {}: {:?} vs native {want:?} (mine {:?})",
                        cm.n,
                        cm.weights[j],
                        &cm.weights[..cm.n]
                    );
                }
                for j in 0..4 {
                    let want = take(&case.rest, &mut at) as f32;
                    if cm.n > 2 {
                        assert!(
                            cm.out[j] as f32 == want,
                            "{label}: model colour {j}: {:?} vs native {want:?}",
                            cm.out[j] as f32
                        );
                    }
                }
                if cm.n > 2 {
                    solved += 1;
                }
            }
        }
        assert_eq!(take_i(&case.rest, &mut at), -14, "case {}: end", case.id);
        assert_eq!(at, case.rest.len(), "case {}: trailing values", case.id);
    }
    assert!(solved > 0, "no pixel exercised the least-squares solve");
}

/// The kd-tree the node set builds, against the host's walk of the
/// library's tree (`--aa-kd-tree`): every split's dimension, value and
/// bounds, every leaf's point, in pre-order.
#[test]
fn kd_tree_matches_the_native_tree() {
    for case in cases(
        include_str!("../../fixtures/native-aa-kd-tree.csv"),
        "aatree,",
    ) {
        let mut pts = Vec::new();
        for c in &case.contours {
            for &id in &c.nodes {
                pts.push([case.nodes[id as usize].x, case.nodes[id as usize].y]);
            }
        }
        let tree = super::kd::KdTree::new(pts);
        let mine = super::kd::preorder(&tree);
        let mut at = 0;
        assert_eq!(take_i(&case.rest, &mut at), -11);
        let end = case.rest.len() - 1;
        assert_eq!(case.rest[end] as i32, -12);
        let want = &case.rest[at..end];
        for (i, (a, b)) in mine.iter().zip(want).enumerate() {
            assert!(
                a == b,
                "case {} (preset {}): kd-tree value {i}: {a:?} vs native {b:?} (mine {:?} native {:?})",
                case.id,
                case.preset,
                &mine[i.saturating_sub(8)..(i + 8).min(mine.len())],
                &want[i.saturating_sub(8)..(i + 8).min(want.len())]
            );
        }
        assert_eq!(mine.len(), want.len(), "case {}: kd-tree size", case.id);
    }
}

/// The inversion pass's early answer for segments whose boxes are apart
/// against 0x47fff0's full test, which it must equal on every input:
/// random short segments a few pixels apart, endpoints set on, just off and
/// well off the other segment around the 1e-4 touch distance and the 1e-3
/// margin, proper crossings, nearly collinear pairs apart along one line
/// with endpoints off it by rounding-scale amounts, and coordinates the
/// early answer leaves to the full test (NaN, infinities, tiny).
#[test]
fn inversion_segment_test_early_answer_matches_the_full_test() {
    let mut state = 0x853c_49e6_748f_ea9b_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let node = |x: f64, y: f64| Node {
        x,
        y,
        fx: x as f32,
        fy: y as f32,
        state: 0,
        flags: 0,
        aux: 0,
    };
    let mut cases: Vec<[[f64; 2]; 4]> = Vec::new();
    for _ in 0..200_000 {
        let (ax, ay) = (next() * 20.0, next() * 20.0);
        let (dx, dy) = ((next() - 0.5) * 4.0, (next() - 0.5) * 4.0);
        let (bx, by) = (ax + (next() - 0.5) * 6.0, ay + (next() - 0.5) * 6.0);
        let (ex, ey) = ((next() - 0.5) * 4.0, (next() - 0.5) * 4.0);
        cases.push([[ax, ay], [ax + dx, ay + dy], [bx, by], [bx + ex, by + ey]]);
    }
    for _ in 0..20_000 {
        // An endpoint of b placed along a's normal from a point of a (or
        // past its ends) at a distance around the touch and margin scales.
        let (ax, ay) = (next() * 20.0, next() * 20.0);
        let (dx, dy) = ((next() - 0.5) * 4.0, (next() - 0.5) * 4.0);
        let len = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = (-dy / len, dx / len);
        let t = next() * 1.4 - 0.2;
        let gap =
            [0.0, 5e-5, 9.9e-5, 1.01e-4, 5e-4, 9.9e-4, 1.01e-3, 2e-3][(next() * 8.0) as usize];
        let side = if next() < 0.5 { -1.0 } else { 1.0 };
        let (px, py) = (ax + t * dx + side * gap * nx, ay + t * dy + side * gap * ny);
        let (ex, ey) = ((next() - 0.5) * 4.0, (next() - 0.5) * 4.0);
        cases.push([[ax, ay], [ax + dx, ay + dy], [px, py], [px + ex, py + ey]]);
        cases.push([[ax, ay], [ax + dx, ay + dy], [px + ex, py + ey], [px, py]]);
    }
    for _ in 0..400_000 {
        // Nearly collinear and apart (round two of the review): b goes on
        // along a's line after a gap, each endpoint off the line by a
        // rounding-scale amount, where the full test's crossing branch is
        // decided by rounding rather than by the geometry.
        let base = [20.0, 2000.0, 16000.0][(next() * 3.0) as usize];
        let (ax, ay) = (next() * base, next() * base);
        let (dx, dy) = ((next() - 0.5) * 8.0, (next() - 0.5) * 8.0);
        let len = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = (-dy / len, dx / len);
        let gap = 2e-3 + next() * 0.5;
        let (s0, s1) = (1.0 + gap / len, 1.0 + gap / len + next() * 2.0);
        let off = |next: &mut dyn FnMut() -> f64| {
            let side = if next() < 0.5 { -1.0 } else { 1.0 };
            side * 10f64.powi(-11 - (next() * 8.0) as i32) * base
        };
        let (o0, o1) = (off(&mut next), off(&mut next));
        let b0 = [ax + s0 * dx + o0 * nx, ay + s0 * dy + o0 * ny];
        let b1 = [ax + s1 * dx + o1 * nx, ay + s1 * dy + o1 * ny];
        cases.push([[ax, ay], [ax + dx, ay + dy], b0, b1]);
        cases.push([[ax + dx, ay + dy], [ax, ay], b1, b0]);
    }
    for (x, y) in [
        (f64::NAN, 1.0),
        (f64::INFINITY, 1.0),
        (1e-120, 3.0),
        (-1e7, 3.0),
    ] {
        cases.push([[x, y], [2.0, 2.0], [9.0, 9.0], [9.0, 12.0]]);
        cases.push([[0.0, 0.0], [1e-170, 0.0], [0.0, 100.0], [0.0, 200.0]]);
    }
    let mut touching = 0;
    let mut crossing = 0;
    for [a0, a1, b0, b1] in &cases {
        let full = anti_aliased::cross_points(a0, a1, b0, b1);
        let early = anti_aliased::segments_cross(
            &node(a0[0], a0[1]),
            &node(a1[0], a1[1]),
            &node(b0[0], b0[1]),
            &node(b1[0], b1[1]),
        );
        assert_eq!(early, full, "{a0:?} {a1:?} {b0:?} {b1:?}");
        match full.abs() {
            1 => touching += 1,
            2 => crossing += 1,
            _ => {}
        }
    }
    assert!(
        touching > 1000 && crossing > 1000,
        "{touching} touching, {crossing} crossing"
    );
}
