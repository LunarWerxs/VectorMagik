use super::*;
use crate::recovered_topology;

/// One captured case: the label image the original smoothed (through the
/// unmodified contour construction) and the parameters its preset gave the
/// smoother, then whatever the mode emitted.
struct Case {
    id: i64,
    preset: i64,
    width: usize,
    height: usize,
    regions: usize,
    labels: Vec<i32>,
    params: Params,
    rest: Vec<f64>,
}

struct State {
    nodes: Vec<Node>,
    /// pixels, area, entry flags
    contours: Vec<(i32, f64, Vec<i32>)>,
}

fn take(v: &[f64], at: &mut usize) -> f64 {
    let x = v[*at];
    *at += 1;
    x
}
fn take_i(v: &[f64], at: &mut usize) -> i32 {
    take(v, at) as i32
}
fn parse(line: &str) -> Case {
    let mut fields = line.split(',');
    fields.next();
    let v: Vec<f64> = fields.map(|f| f.parse().unwrap()).collect();
    let mut at = 0;
    let id = take(&v, &mut at) as i64;
    let preset = take(&v, &mut at) as i64;
    let width = take(&v, &mut at) as usize;
    let height = take(&v, &mut at) as usize;
    let regions = take(&v, &mut at) as usize;
    let labels: Vec<i32> = (0..width * height).map(|_| take_i(&v, &mut at)).collect();
    let params = Params::from_values(&v[at..at + PARAM_VALUES]).unwrap();
    at += PARAM_VALUES;
    Case {
        id,
        preset,
        width,
        height,
        regions,
        labels,
        params,
        rest: v[at..].to_vec(),
    }
}

fn state(v: &[f64], at: &mut usize) -> State {
    let n = take(v, at) as usize;
    let nodes = (0..n)
        .map(|_| Node {
            x: take(v, at),
            y: take(v, at),
            fx: take(v, at) as f32,
            fy: take(v, at) as f32,
            state: take_i(v, at) as u8,
            flags: take_i(v, at) as u8,
            aux: take_i(v, at) as u8,
        })
        .collect();
    let m = take(v, at) as usize;
    let contours = (0..m)
        .map(|_| {
            let pixels = take_i(v, at);
            let area = take(v, at);
            let k = take(v, at) as usize;
            (pixels, area, (0..k).map(|_| take_i(v, at)).collect())
        })
        .collect();
    State { nodes, contours }
}

/// The smoother's input for a case: the owned topology of the label image
/// with the region pixel counts in the contour records.
fn inputs(case: &Case) -> (Vec<Node>, Vec<Contour>, Canvas) {
    let topology =
        recovered_topology::build(&case.labels, case.width, case.height, case.regions, true)
            .unwrap();
    let nodes: Vec<Node> = topology
        .nodes
        .iter()
        .map(|n| Node {
            x: n.x,
            y: n.y,
            fx: n.x as f32,
            fy: n.y as f32,
            state: n.state,
            flags: n.flags,
            aux: 0,
        })
        .collect();
    let mut pixels = vec![0i32; case.regions];
    for &l in &case.labels {
        pixels[l as usize] += 1;
    }
    let contours: Vec<Contour> = topology
        .contours
        .iter()
        .enumerate()
        .map(|(i, c)| Contour {
            pixels: pixels[i],
            area: 0.0,
            nodes: c.nodes.clone(),
            edges: c
                .edges
                .iter()
                .map(|e| Edge {
                    flag: 0,
                    other: e.other,
                    steps: e.steps,
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    let canvas = Canvas {
        width: case.width as i32,
        height: case.height as i32,
        border: topology.border,
    };
    (nodes, contours, canvas)
}

fn smoother<'a>(
    nodes: &'a mut [Node],
    contours: &'a mut [Contour],
    canvas: Canvas,
    params: &'a Params,
) -> Smoother<'a> {
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
        aa: None,
    };
    s.find_near();
    s
}

fn check_state(case: &Case, nodes: &[Node], contours: &[Contour], expected: &State, what: &str) {
    assert_eq!(
        nodes.len(),
        expected.nodes.len(),
        "case {} {what}: node count",
        case.id
    );
    for (i, (node, want)) in nodes.iter().zip(&expected.nodes).enumerate() {
        assert_eq!(
            (node.state, node.flags, node.aux),
            (want.state, want.flags, want.aux),
            "case {} {what} node {i} bytes",
            case.id
        );
        assert_eq!(
            (node.fx, node.fy),
            (want.fx, want.fy),
            "case {} {what} node {i} float copies",
            case.id
        );
        assert!(
            node.x == want.x && node.y == want.y,
            "case {} (preset {}) {what} node {i}: ({:?}, {:?}) vs native ({:?}, {:?})",
            case.id,
            case.preset,
            node.x,
            node.y,
            want.x,
            want.y
        );
    }
    assert_eq!(contours.len(), expected.contours.len());
    for (i, (contour, (pixels, area, flags))) in contours.iter().zip(&expected.contours).enumerate()
    {
        assert_eq!(
            contour.pixels, *pixels,
            "case {} {what} contour {i} pixels",
            case.id
        );
        assert!(
            contour.area == *area,
            "case {} {what} contour {i} area {:?} vs native {:?}",
            case.id,
            contour.area,
            area
        );
        let got: Vec<i32> = contour.edges.iter().map(|e| e.flag).collect();
        assert_eq!(
            &got, flags,
            "case {} {what} contour {i} entry flags",
            case.id
        );
    }
}

#[test]
fn every_phase_matches_42_native_probes() {
    let mut punctured = 0;
    let mut phases = 0;
    for line in include_str!("../../fixtures/native-smoothing-probe.csv")
        .lines()
        .filter(|l| l.starts_with("probe,"))
    {
        let case = parse(line);
        let (mut nodes, mut contours, canvas) = inputs(&case);
        let mut at = 0;
        let energy = take(&case.rest, &mut at);
        let n = nodes.len();
        let gradient: Vec<f64> = (0..2 * n).map(|_| take(&case.rest, &mut at)).collect();
        let mut s = smoother(&mut nodes, &mut contours, canvas, &case.params);
        let e = s.objective();
        assert!(
            e == energy,
            "case {} (preset {}): energy {e:?} vs native {energy:?}",
            case.id,
            case.preset
        );
        s.gradient();
        for i in 0..n {
            for c in 0..2 {
                let got = s.grad[i][c];
                let want = gradient[2 * i + c];
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
            if tag == -2 {
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
            check_state(
                &case,
                s.nodes,
                s.contours,
                &expected,
                &format!("after phase {phase}"),
            );
            phases += 1;
            if case.params.do_puncture_corners[phase] != 0 {
                assert_eq!(take_i(&case.rest, &mut at), -1);
                let expected = state(&case.rest, &mut at);
                punctured += s.puncture().unwrap();
                check_state(
                    &case,
                    s.nodes,
                    s.contours,
                    &expected,
                    &format!("after puncturing phase {phase}"),
                );
            }
        }
        let expected = state(&case.rest, &mut at);
        assert_eq!(at, case.rest.len(), "case {}: trailing values", case.id);
        check_state(&case, &nodes, &contours, &expected, "at the end");
    }
    assert!(phases > 60, "{phases} phases ran");
    assert!(punctured > 20, "{punctured} corners punctured");
}

#[test]
fn smoothing_matches_42_native_runs() {
    let mut corners = 0;
    let mut iterations = 0;
    let mut small = 0;
    for line in include_str!("../../fixtures/native-smoothing.csv")
        .lines()
        .filter(|l| l.starts_with("smoothing,"))
    {
        let case = parse(line);
        let (mut nodes, mut contours, canvas) = inputs(&case);
        let mut at = 0;
        let expected = state(&case.rest, &mut at);
        assert_eq!(at, case.rest.len(), "case {}: trailing values", case.id);
        small += contours.iter().filter(|c| c.pixels <= SMALL_REGION).count();
        let report = smooth(&mut nodes, &mut contours, canvas, &case.params, None).unwrap();
        check_state(&case, &nodes, &contours, &expected, "after smoothing");
        corners += report.corners;
        iterations += report.iterations.iter().sum::<i32>();
    }
    assert!(corners > 20, "{corners} corners punctured");
    assert!(
        iterations > 1000,
        "{iterations} conjugate-gradient iterations"
    );
    assert!(small > 5, "{small} small regions");
}

#[test]
fn native_random_follows_the_engine_generator() {
    let mut seed = 1;
    let first = native_random(&mut seed);
    assert_eq!(seed, 2003);
    assert!((first - 2003.0 * 0.0001002707309736288).abs() < 1e-18);
    let mut s = 9972;
    native_random(&mut s);
    assert_eq!(s, (9972i64 * 2003 % 9973) as i32);
}

#[test]
fn bad_inputs_are_refused() {
    let case = parse(
        include_str!("../../fixtures/native-smoothing.csv")
            .lines()
            .find(|l| l.starts_with("smoothing,"))
            .unwrap(),
    );
    let (mut nodes, mut contours, canvas) = inputs(&case);
    let tiny = AaImage {
        width: 2,
        height: 2,
        pixels: vec![0; 16],
        labels: vec![0; 4],
        region_colors: vec![[0.0; 4]],
    };
    let input = AaInput {
        image: &tiny,
        seed: 1,
        every: 10,
        units: [0.0; 4],
        prepare: true,
    };
    assert!(smooth(&mut nodes, &mut contours, canvas, &case.params, Some(input)).is_err());
    let mut one = nodes[..1].to_vec();
    assert!(smooth(&mut one, &mut [], canvas, &case.params, None).is_err());
    let mut broken = contours.clone();
    broken[0].edges.pop();
    assert!(smooth(&mut nodes, &mut broken, canvas, &case.params, None).is_err());
    let mut wild = contours.clone();
    wild[0].nodes[0] = nodes.len() as i32;
    assert!(smooth(&mut nodes, &mut wild, canvas, &case.params, None).is_err());
    let mut params = case.params.clone();
    params.prior_types[0] = 2;
    assert!(smooth(&mut nodes, &mut contours, canvas, &params, None).is_err());
    let mut params = case.params.clone();
    params.measurement_types[0] = 1;
    assert!(smooth(&mut nodes, &mut contours, canvas, &params, None).is_err());
    let mut params = case.params.clone();
    params.phases[0].line_search_type = 4;
    assert!(smooth(&mut nodes, &mut contours, canvas, &params, None).is_err());
}

#[test]
fn corner_decision_tree_edges() {
    // A sharp middle turn that is the local maximum on a flat run.
    assert_eq!(decide(&[0.0, 0.0, 1.5, 0.0, 0.0]), (true, true));
    // The same turn on a busy run: the side turns disqualify it.
    assert_eq!(decide(&[0.2, 0.2, 1.5, 0.2, 0.2]), (false, true));
    // Not the local maximum: only a turn of 1.72002 or more counts.
    assert_eq!(decide(&[0.1, 1.9, 1.5, 0.0, 0.0]), (false, false));
    assert_eq!(decide(&[0.1, 1.9, 1.73, 0.0, 0.0]), (true, false));
    // A modest turn on an otherwise straight run.
    assert_eq!(decide(&[0.0, 0.0, 0.4, 0.0, 0.0]), (true, true));
    assert_eq!(decide(&[0.0, 0.0, 0.2, 0.0, 0.0]), (false, true));
}
