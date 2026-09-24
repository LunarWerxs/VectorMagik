use super::*;

/// One frozen native row: the label image, the extra-array flag and every
/// node and contour record 0x499eb0 left, with the untouched fields.
struct Case {
    id: usize,
    width: usize,
    height: usize,
    regions: usize,
    thin: bool,
    labels: Vec<i32>,
    nodes: Vec<(Node, f32, f32, i32, u8)>,
    contours: Vec<(Contour, Vec<i32>, Vec<i32>, i32)>,
    border: [i32; 4],
    size: (i32, i32),
}

fn parse(line: &str) -> Case {
    let v: Vec<f64> = line
        .split(',')
        .skip(1)
        .map(|t| t.parse().unwrap())
        .collect();
    let mut at = 0;
    let mut next = || {
        at += 1;
        v[at - 1]
    };
    let (id, width, height, regions, flag) = (
        next() as usize,
        next() as usize,
        next() as usize,
        next() as usize,
        next() as i32,
    );
    let labels: Vec<i32> = (0..width * height).map(|_| next() as i32).collect();
    let count = next() as usize;
    let nodes = (0..count)
        .map(|_| {
            let (x, y, fx, fy, curve) =
                (next(), next(), next() as f32, next() as f32, next() as i32);
            let (state, flags, b1e) = (next() as u8, next() as u8, next() as u8);
            (Node { x, y, state, flags }, fx, fy, curve, b1e)
        })
        .collect();
    let count = next() as usize;
    let contours = (0..count)
        .map(|_| {
            let head: Vec<i32> = (0..6).map(|_| next() as i32).collect();
            let n = next() as usize;
            let ids: Vec<i32> = (0..n).map(|_| next() as i32).collect();
            let metadata_count = next() as usize;
            assert_eq!(metadata_count, n);
            let mut edges = Vec::new();
            for _ in 0..n {
                edges.push(Edge {
                    other: next() as i32,
                    steps: next() as i32,
                });
            }
            let m = next() as usize;
            let neighbours: Vec<i32> = (0..m).map(|_| next() as i32).collect();
            let enclosing = next() as i32;
            let tail: Vec<i32> = (0..11).map(|_| next() as i32).collect();
            (
                Contour {
                    nodes: ids,
                    edges,
                    neighbours,
                    enclosing,
                },
                head,
                tail,
                enclosing,
            )
        })
        .collect();
    let border = [next() as i32, next() as i32, next() as i32, next() as i32];
    let size = (next() as i32, next() as i32);
    assert_eq!(at, v.len(), "case {id}: unread values");
    Case {
        id,
        width,
        height,
        regions,
        thin: flag == 0,
        labels,
        nodes,
        contours,
        border,
        size,
    }
}

#[test]
fn tables_match_the_constructed_builder() {
    let line = include_str!("../../fixtures/native-topology.csv")
        .lines()
        .find(|l| l.starts_with("tables,"))
        .unwrap();
    let v: Vec<i64> = line
        .split(',')
        .skip(1)
        .map(|t| t.parse().unwrap())
        .collect();
    let words: Vec<i32> = v[..24].iter().map(|&x| x as i32).collect();
    assert_eq!(&words[0..4], &CORNER_DX);
    assert_eq!(&words[4..8], &CORNER_DY);
    assert_eq!(&words[8..12], &PIXEL_DX);
    assert_eq!(&words[12..16], &PIXEL_DY);
    assert_eq!(&words[16..20], &STEP_DX);
    assert_eq!(&words[20..24], &STEP_DY);
    assert_eq!(v[24] as usize, PATTERNS.len());
    for (i, (length, mask, keep)) in PATTERNS.iter().enumerate() {
        assert_eq!(v[25 + 3 * i] as i32, *length);
        assert_eq!(v[26 + 3 * i] as u32, *mask);
        assert_eq!(v[27 + 3 * i] as u32, *keep);
    }
}

#[test]
fn topology_matches_48_native_label_images() {
    let mut thinned = 0;
    let mut junctions = 0;
    let mut enclosed = 0;
    for line in include_str!("../../fixtures/native-topology.csv")
        .lines()
        .filter(|l| l.starts_with("topology,"))
    {
        let case = parse(line);
        let topology = build(
            &case.labels,
            case.width,
            case.height,
            case.regions,
            case.thin,
        )
        .unwrap();
        assert_eq!(
            topology.nodes.len(),
            case.nodes.len(),
            "case {}: node count",
            case.id
        );
        for (i, (node, (expected, fx, fy, curve, b1e))) in
            topology.nodes.iter().zip(&case.nodes).enumerate()
        {
            assert_eq!(node, expected, "case {} node {i}", case.id);
            // The single-precision copies, the curve index the constructor
            // sets to -1 and the spare byte are what the original leaves.
            assert_eq!((node.x as f32, node.y as f32), (*fx, *fy));
            assert_eq!((*curve, *b1e), (-1, 0));
            if node.state == 3 {
                junctions += 1;
            }
        }
        assert_eq!(topology.contours.len(), case.contours.len());
        for (i, (contour, (expected, head, tail, enclosing))) in
            topology.contours.iter().zip(&case.contours).enumerate()
        {
            assert_eq!(
                contour.nodes, expected.nodes,
                "case {} contour {i} nodes",
                case.id
            );
            assert_eq!(
                contour.edges, expected.edges,
                "case {} contour {i} edges",
                case.id
            );
            assert_eq!(
                contour.neighbours, expected.neighbours,
                "case {} contour {i} neighbours",
                case.id
            );
            assert_eq!(
                contour.enclosing, *enclosing,
                "case {} contour {i}",
                case.id
            );
            assert!(head.iter().all(|&w| w == 0) && tail.iter().all(|&w| w == 0));
            if contour.enclosing >= 0 {
                enclosed += 1;
            }
            if contour.edges.iter().any(|e| e.steps > 1) {
                thinned += 1;
            }
        }
        assert_eq!(topology.border, case.border, "case {}", case.id);
        assert_eq!(case.size, (case.width as i32, case.height as i32));
    }
    assert!(thinned > 100, "{thinned} contours lost nodes to thinning");
    assert!(junctions > 500, "{junctions} corner nodes");
    assert!(enclosed > 10, "{enclosed} contours name an enclosing one");
}

#[test]
fn bad_label_images_are_refused() {
    assert!(build(&[0, 0, 0], 2, 2, 1, true).is_err());
    assert!(build(&[0, 1, 0, 0], 2, 2, 1, true).is_err());
    assert!(build(&[0, -1, 0, 0], 2, 2, 1, true).is_err());
    let one = build(&[0; 4], 2, 2, 1, true).unwrap();
    assert_eq!(one.nodes.len(), 4, "only the canvas corners survive");
    assert_eq!(one.contours[0].nodes, vec![0, 2, 3, 1]);
    assert_eq!(one.border, [2, 2, 0, 0]);
}
