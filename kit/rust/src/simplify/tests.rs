//! The tests of `simplify`.

use super::*;

fn document(paths: &[(&str, String)]) -> String {
    let mut svg = String::from(
        "<svg width=\"1000pt\" height=\"1000pt\" viewBox=\"0 0 1000 1000\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n",
    );
    for (fill, d) in paths {
        svg.push_str(&format!(
            "<path fill=\"{fill}\" opacity=\"1.00\" d=\"{d}\" />\n"
        ));
    }
    svg.push_str("</svg>\n");
    svg
}

fn arc_point(cx: f64, cy: f64, r: f64, angle: f64) -> Point {
    Point {
        x: cx + r * angle.cos(),
        y: cy + r * angle.sin(),
    }
}

/// A circle drawn as `pieces` exact circular arcs.
fn circle_path(cx: f64, cy: f64, r: f64, pieces: usize) -> String {
    let step = std::f64::consts::TAU / pieces as f64;
    let k = 4. / 3. * (step / 4.).tan();
    let start = arc_point(cx, cy, r, 0.);
    let mut d = format!(" M {:.2} {:.2}", start.x, start.y);
    for i in 0..pieces {
        let a0 = i as f64 * step;
        let a1 = a0 + step;
        let p0 = arc_point(cx, cy, r, a0);
        let p3 = if i + 1 == pieces {
            start
        } else {
            arc_point(cx, cy, r, a1)
        };
        let p1 = Point {
            x: p0.x - r * k * a0.sin(),
            y: p0.y + r * k * a0.cos(),
        };
        let p2 = Point {
            x: p3.x + r * k * a1.sin(),
            y: p3.y - r * k * a1.cos(),
        };
        d.push_str(&format!(
            " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
            p1.x, p1.y, p2.x, p2.y, p3.x, p3.y
        ));
    }
    d.push_str(" Z");
    d
}

fn path_data(svg: &str) -> Vec<String> {
    svg.split(" d=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap().to_owned())
        .collect()
}

fn segments(d: &str) -> usize {
    d.bytes().filter(|b| matches!(b, b'L' | b'C')).count()
}

#[test]
fn a_circle_of_many_arcs_becomes_a_few_within_tolerance() {
    let svg = document(&[("#000000", circle_path(100., 100., 80., 36))]);
    let (out, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 0.5 }).unwrap();
    assert_eq!(stats.segments_before, 36);
    assert!(
        (3..=6).contains(&stats.segments_after),
        "{}",
        stats.segments_after
    );
    assert_eq!(stats.runs, 1);
    assert_eq!(stats.junctions, 0);
    let subpaths = parse_path_data(&path_data(&out)[0]).unwrap();
    assert!(subpaths[0].closed);
    for edge in &subpaths[0].edges {
        for k in 0..=50 {
            let p = edge.cubic.evaluate(k as f64 / 50.);
            let radial = (distance(p, Point { x: 100., y: 100. }) - 80.).abs();
            assert!(radial <= 0.5 + 0.02, "radial error {radial}");
        }
    }
}

#[test]
fn the_largest_tolerance_keeps_a_big_circle_on_its_circle() {
    // At the Simplify slider's far end a ring of radius 264 px went to two
    // cubics of 150 and 172 degrees, 4.6 px off round (the owner's GitHub
    // mark, September 23, 2026): no merge may carry a circle's pieces more
    // than ON_CIRCLE off it, so the ring stays in arcs of a third of it or
    // less (one cubic of 120 degrees is 0.4 px off round at this radius).
    let (c, r) = (400., 264.);
    let svg = document(&[("#000000", circle_path(c, c, r, 16))]);
    let (out, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 3. }).unwrap();
    assert!(
        (3..16).contains(&stats.segments_after),
        "{}",
        stats.segments_after
    );
    let subpaths = parse_path_data(&path_data(&out)[0]).unwrap();
    for edge in &subpaths[0].edges {
        for k in 0..=50 {
            let p = edge.cubic.evaluate(k as f64 / 50.);
            let radial = (distance(p, Point { x: c, y: c }) - r).abs();
            assert!(radial <= ON_CIRCLE + 0.02, "radial error {radial}");
        }
    }
}

#[test]
fn a_shared_boundary_is_simplified_identically_on_both_sides() {
    // Two fills meeting along a gently waving line drawn as 24 pieces.
    let mut boundary = Vec::new();
    for i in 0..=24 {
        let t = i as f64 / 24.;
        boundary.push(Point {
            x: 100. + 6. * (t * std::f64::consts::TAU).sin(),
            y: 20. + 160. * t,
        });
    }
    let fmt = |p: Point| format!("{:.2} {:.2}", p.x, p.y);
    let mut left = format!(" M 20.00 20.00 L {}", fmt(boundary[0]));
    for p in &boundary[1..] {
        left.push_str(&format!(" L {}", fmt(*p)));
    }
    left.push_str(" L 20.00 180.00 L 20.00 20.00 Z");
    let mut right = format!(" M 180.00 180.00 L {}", fmt(boundary[24]));
    for p in boundary[..24].iter().rev() {
        right.push_str(&format!(" L {}", fmt(*p)));
    }
    right.push_str(" L 180.00 20.00 L 180.00 180.00 Z");
    let svg = document(&[("#ff0000", left), ("#0000ff", right)]);
    let (out, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 0.4 }).unwrap();
    assert_eq!(stats.paths, 2);
    assert_eq!(stats.junctions, 2, "the boundary ends are junctions");
    assert!(stats.shared_runs >= 1);
    assert!(stats.segments_after < stats.segments_before);
    let paths = path_data(&out);
    let a = parse_path_data(&paths[0]).unwrap();
    let b = parse_path_data(&paths[1]).unwrap();
    let keys = |subpath: &Subpath| -> HashSet<EdgeKey> {
        subpath.edges.iter().map(|e| e.key().0).collect()
    };
    let shared: Vec<EdgeKey> = keys(&a[0]).intersection(&keys(&b[0])).copied().collect();
    // Every edge along the boundary appears in both fills; the corners of
    // each rectangle are its own.
    assert!(!shared.is_empty() && shared.len() < 24, "{}", shared.len());
    assert_eq!(a[0].edges.len() - shared.len(), 3);
    assert_eq!(b[0].edges.len() - shared.len(), 3);
    // The junction nodes at both ends of the boundary survive.
    let nodes = |subpath: &Subpath| -> HashSet<Key> {
        subpath.edges.iter().map(|e| key(e.start())).collect()
    };
    for end in [boundary[0], boundary[24]] {
        assert!(nodes(&a[0]).contains(&key(end)) && nodes(&b[0]).contains(&key(end)));
    }
}

#[test]
fn sharp_corners_and_unsupported_commands_are_left_alone() {
    let square = " M 10.00 10.00 L 90.00 10.00 L 90.00 90.00 L 10.00 90.00 L 10.00 10.00 Z";
    let svg = document(&[("#000000", square.to_owned())]);
    let (out, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 0.5 }).unwrap();
    assert_eq!((stats.segments_before, stats.segments_after), (4, 4));
    assert_eq!(segments(&path_data(&out)[0]), 4);
    let arc = document(&[("#000000", " M 0.00 0.00 A 5 5 0 0 1 10 10 Z".to_owned())]);
    assert!(simplify_svg(&arc, SimplifyOptions::default()).is_err());
    assert!(simplify_svg(&svg, SimplifyOptions { tolerance: 0. }).is_err());
}

#[test]
fn a_tiny_tolerance_changes_nothing_and_the_rest_of_the_document_is_copied() {
    let svg = document(&[
        ("#00ff00", circle_path(60., 60., 40., 12)),
        (
            "#000000",
            " M 0.00 0.00 L 200.00 0.00 L 200.00 200.00 L 0.00 200.00 L 0.00 0.00 Z".to_owned(),
        ),
    ]);
    let (out, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 1e-9 }).unwrap();
    assert_eq!(stats.segments_before, stats.segments_after);
    assert!(out.contains("fill=\"#00ff00\"") && out.contains("viewBox=\"0 0 1000 1000\""));
    let before: HashSet<Key> = parse_path_data(&path_data(&svg)[0]).unwrap()[0]
        .edges
        .iter()
        .map(|e| key(e.start()))
        .collect();
    let after: HashSet<Key> = parse_path_data(&path_data(&out)[0]).unwrap()[0]
        .edges
        .iter()
        .map(|e| key(e.start()))
        .collect();
    assert_eq!(before, after);
}

/// Every sampled point of an edge, for checks on where a curve goes.
fn samples(edge: &Edge) -> Vec<Point> {
    (0..=24)
        .map(|i| edge.cubic.evaluate(i as f64 / 24.))
        .collect()
}

#[test]
fn rounding_a_corner_cuts_the_tip_and_bridges_it_with_a_tangent_arc() {
    // A pointed bump on a rectangle; the tip at (120, 60) is a sharp corner.
    let shape = " M 20.00 100.00 L 100.00 100.00 L 120.00 60.00 L 140.00 100.00 L 180.00 100.00 L 180.00 180.00 L 20.00 180.00 L 20.00 100.00 Z";
    let svg = document(&[("#000000", shape.to_owned())]);
    let tip = Point { x: 120., y: 60. };
    let (out, count) = smooth_nodes(&svg, &[Rounding { at: tip, reach: 1. }]).unwrap();
    assert_eq!(count, 1);
    let subpath = &parse_path_data(&path_data(&out)[0]).unwrap()[0];
    assert_eq!(
        subpath.edges.len(),
        6,
        "the two whole arms became one arc: {out}"
    );
    let arc = subpath
        .edges
        .iter()
        .find(|e| key(e.start()) == key(Point { x: 100., y: 100. }))
        .unwrap();
    assert!(!arc.line);
    assert_eq!(key(arc.end()), key(Point { x: 140., y: 100. }));
    // Tangent to both arms at the cut points, and short of the old tip.
    let up = normalized(Point { x: 20., y: -40. }).unwrap();
    let down = normalized(Point { x: 20., y: 40. }).unwrap();
    assert!(dot(leaving(arc).unwrap(), up) > 0.999);
    assert!(dot(arriving(arc).unwrap(), down) > 0.999);
    let top = samples(arc)
        .iter()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min);
    assert!(top > 61. && top < 95., "the arc peaks at y = {top:.1}");
    assert!(!subpath.edges.iter().any(|e| key(e.start()) == key(tip)));
    // Nodes absent from the document are ignored.
    let (_, count) = smooth_nodes(
        &out,
        &[Rounding {
            at: Point { x: 1., y: 1. },
            reach: 1.,
        }],
    )
    .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn a_short_reach_rounds_only_near_the_node_and_keeps_the_rest_exactly() {
    let shape = " M 20.00 100.00 L 100.00 100.00 L 120.00 60.00 L 140.00 100.00 L 180.00 100.00 L 180.00 180.00 L 20.00 180.00 L 20.00 100.00 Z";
    let svg = document(&[("#000000", shape.to_owned())]);
    let tip = Point { x: 120., y: 60. };
    let (out, count) = smooth_nodes(
        &svg,
        &[Rounding {
            at: tip,
            reach: 0.25,
        }],
    )
    .unwrap();
    assert_eq!(count, 1);
    let subpath = &parse_path_data(&path_data(&out)[0]).unwrap()[0];
    assert_eq!(
        subpath.edges.len(),
        8,
        "the tip node went, a cut point on each side came: {out}"
    );
    // The far three quarters of each arm stay straight lines ending at the
    // exact cut points; the arc between them never reaches the old tip.
    let far_in = subpath
        .edges
        .iter()
        .find(|e| key(e.start()) == key(Point { x: 100., y: 100. }))
        .unwrap();
    assert!(far_in.line);
    assert_eq!(key(far_in.end()), key(Point { x: 115., y: 70. }));
    let far_out = subpath
        .edges
        .iter()
        .find(|e| key(e.end()) == key(Point { x: 140., y: 100. }))
        .unwrap();
    assert!(far_out.line);
    assert_eq!(key(far_out.start()), key(Point { x: 125., y: 70. }));
    let arc = subpath
        .edges
        .iter()
        .find(|e| key(e.start()) == key(Point { x: 115., y: 70. }))
        .unwrap();
    assert!(!arc.line);
    assert_eq!(key(arc.end()), key(Point { x: 125., y: 70. }));
    assert!(dot(leaving(arc).unwrap(), arriving(far_in).unwrap()) > 0.999);
    assert!(dot(arriving(arc).unwrap(), leaving(far_out).unwrap()) > 0.999);
    let top = samples(arc)
        .iter()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min);
    assert!(top > 60.5 && top < 70., "{top:.2}");
    assert!(smooth_nodes(&svg, &[Rounding { at: tip, reach: 0. }]).is_err());
    assert!(smooth_nodes(
        &svg,
        &[Rounding {
            at: tip,
            reach: 1.5
        }]
    )
    .is_err());
}

#[test]
fn each_node_keeps_its_own_reach_on_a_shared_piece() {
    // The bump's tip (120, 60) and its right foot (140, 100) share the
    // piece between them; each is rounded with its own reach.
    let shape = " M 20.00 100.00 L 100.00 100.00 L 120.00 60.00 L 140.00 100.00 L 180.00 100.00 L 180.00 180.00 L 20.00 180.00 L 20.00 100.00 Z";
    let svg = document(&[("#000000", shape.to_owned())]);
    let tip = Point { x: 120., y: 60. };
    let foot = Point { x: 140., y: 100. };
    // Tight at both ends: the tip's arms are equal (44.7), so its cuts sit
    // a quarter along each; the foot's shorter arm is 40, so both its cuts
    // are 10 long. The shared piece keeps its exact middle between them.
    let (out, count) = smooth_nodes(
        &svg,
        &[
            Rounding {
                at: tip,
                reach: 0.25,
            },
            Rounding {
                at: foot,
                reach: 0.25,
            },
        ],
    )
    .unwrap();
    assert_eq!(count, 2);
    let subpath = &parse_path_data(&path_data(&out)[0]).unwrap()[0];
    assert_eq!(
        subpath.edges.len(),
        9,
        "two nodes gone, four cut points in: {out}"
    );
    let middle = subpath
        .edges
        .iter()
        .find(|e| key(e.start()) == key(Point { x: 125., y: 70. }))
        .unwrap();
    assert!(middle.line);
    let along = normalized(sub(foot, tip)).unwrap();
    let foot_cut = sub(foot, scale(along, 10.));
    assert!(
        distance(middle.end(), foot_cut) < 0.01,
        "{:?}",
        middle.end()
    );
    let far_out = subpath
        .edges
        .iter()
        .find(|e| key(e.end()) == key(Point { x: 180., y: 100. }))
        .unwrap();
    assert!(far_out.line);
    assert!(distance(far_out.start(), Point { x: 150., y: 100. }) < 0.01);
    assert!(!subpath.edges.iter().any(|e| key(e.start()) == key(foot)));
    // A wide tip next to a tight foot would cover the shared piece and
    // more; the cuts share it in proportion and the two arcs meet at that
    // point with the piece's own direction.
    let (out, count) = smooth_nodes(
        &svg,
        &[
            Rounding { at: tip, reach: 1. },
            Rounding {
                at: foot,
                reach: 0.25,
            },
        ],
    )
    .unwrap();
    assert_eq!(count, 2);
    let subpath = &parse_path_data(&path_data(&out)[0]).unwrap()[0];
    let shared = distance(tip, foot);
    let t = 1. / (1. + 10. / shared);
    let meet = lerp(tip, foot, t);
    let first = subpath
        .edges
        .iter()
        .find(|e| distance(e.end(), meet) < 0.01)
        .expect("an arc ends at the shared cut");
    let second = subpath
        .edges
        .iter()
        .find(|e| distance(e.start(), meet) < 0.01)
        .expect("an arc starts at the shared cut");
    assert!(!first.line && !second.line);
    assert!(dot(arriving(first).unwrap(), leaving(second).unwrap()) > 0.999);
    assert_eq!(key(first.start()), key(Point { x: 100., y: 100. }));
    // Reach is validated per node.
    assert!(smooth_nodes(
        &svg,
        &[
            Rounding { at: tip, reach: 1. },
            Rounding {
                at: foot,
                reach: 0.
            },
        ],
    )
    .is_err());
}

#[test]
fn a_corner_between_long_edges_rounds_no_more_than_the_cap() {
    // A 200 by 200 square in a 200 wide picture: a quarter of a side would
    // be 50; the cap (0.15 * 200 * 0.25) holds the cut at 7.5 each way.
    let svg = "<svg width=\"200pt\" height=\"200pt\" viewBox=\"0 0 200 200\" xmlns=\"http://www.w3.org/2000/svg\">\n<path fill=\"#000000\" d=\" M 0.00 0.00 L 200.00 0.00 L 200.00 200.00 L 0.00 200.00 L 0.00 0.00 Z\" />\n</svg>\n";
    assert_eq!(viewbox_size(svg), Some((200., 200.)));
    let corner = Point { x: 200., y: 0. };
    let (out, count) = smooth_nodes(
        svg,
        &[Rounding {
            at: corner,
            reach: 0.25,
        }],
    )
    .unwrap();
    assert_eq!(count, 1);
    let subpath = &parse_path_data(&path_data(&out)[0]).unwrap()[0];
    let arc = subpath.edges.iter().find(|e| !e.line).unwrap();
    assert!(distance(arc.start(), Point { x: 192.5, y: 0. }) < 0.01);
    assert!(distance(arc.end(), Point { x: 200., y: 7.5 }) < 0.01);
    // Without a viewBox the quarter rule stands.
    let bare = svg.replace(" viewBox=\"0 0 200 200\"", "");
    assert_eq!(viewbox_size(&bare), None);
    let (out, _) = smooth_nodes(
        &bare,
        &[Rounding {
            at: corner,
            reach: 0.25,
        }],
    )
    .unwrap();
    let subpath = &parse_path_data(&path_data(&out)[0]).unwrap()[0];
    let arc = subpath.edges.iter().find(|e| !e.line).unwrap();
    assert!(distance(arc.start(), Point { x: 150., y: 0. }) < 0.01);
}

#[test]
fn rounding_keeps_shared_edges_shared_and_skips_junctions() {
    // Two fills meeting along a boundary with a kink at (110, 100).
    let left = " M 20.00 20.00 L 100.00 20.00 L 110.00 100.00 L 100.00 180.00 L 20.00 180.00 L 20.00 20.00 Z";
    let right = " M 180.00 180.00 L 100.00 180.00 L 110.00 100.00 L 100.00 20.00 L 180.00 20.00 L 180.00 180.00 Z";
    let svg = document(&[("#ff0000", left.to_owned()), ("#0000ff", right.to_owned())]);
    let kink = Point { x: 110., y: 100. };
    let corner = Point { x: 100., y: 20. };
    let (out, count) = smooth_nodes(
        &svg,
        &[
            Rounding {
                at: kink,
                reach: 0.5,
            },
            Rounding {
                at: corner,
                reach: 1.,
            },
        ],
    )
    .unwrap();
    assert_eq!(count, 1, "the junction at (100, 20) is skipped");
    let paths = path_data(&out);
    let a = parse_path_data(&paths[0]).unwrap();
    let b = parse_path_data(&paths[1]).unwrap();
    let keys = |subpath: &Subpath| -> HashSet<EdgeKey> {
        subpath.edges.iter().map(|e| e.key().0).collect()
    };
    let shared: HashSet<EdgeKey> = keys(&a[0]).intersection(&keys(&b[0])).copied().collect();
    assert_eq!(
        shared.len(),
        3,
        "the two cut halves and the arc are identical on both sides"
    );
    assert!(!a[0].edges.iter().any(|e| key(e.start()) == key(kink)));
    assert!(!b[0].edges.iter().any(|e| key(e.start()) == key(kink)));
    assert!(a[0].edges.iter().any(|e| !e.line) && b[0].edges.iter().any(|e| !e.line));
}

#[test]
fn two_nodes_on_one_smooth_bend_merge_into_one_segment() {
    // A quarter circle split unevenly into two arcs, joined to straight sides.
    let r = 60.;
    let (cx, cy) = (100., 100.);
    let a0 = 0.;
    let a1 = 0.2;
    let a2 = std::f64::consts::FRAC_PI_2;
    let mut d = String::from(" M 40.00 40.00 L 100.00 40.00");
    let p = |a: f64| arc_point(cx, cy, r, a - std::f64::consts::FRAC_PI_2);
    let ctrl = |from: f64, to: f64| {
        let k = 4. / 3. * ((to - from) / 4.).tan();
        let s = p(from);
        let e = p(to);
        let sa = from - std::f64::consts::FRAC_PI_2;
        let ea = to - std::f64::consts::FRAC_PI_2;
        (
            Point {
                x: s.x - r * k * sa.sin(),
                y: s.y + r * k * sa.cos(),
            },
            Point {
                x: e.x + r * k * ea.sin(),
                y: e.y - r * k * ea.cos(),
            },
        )
    };
    for (from, to) in [(a0, a1), (a1, a2)] {
        let (c1, c2) = ctrl(from, to);
        let e = p(to);
        d.push_str(&format!(
            " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
            c1.x, c1.y, c2.x, c2.y, e.x, e.y
        ));
    }
    d.push_str(" L 160.00 160.00 L 40.00 160.00 L 40.00 40.00 Z");
    let svg = document(&[("#000000", d)]);
    let (_, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 0.3 }).unwrap();
    assert_eq!(stats.segments_before, 6);
    assert_eq!(
        stats.segments_after, 5,
        "the two arcs become one, the corners stay"
    );
}

/// Every piece of every path as its orientation-independent key and the way
/// this path walks it, sorted per path so a rotated start compares equal.
fn walked(svg: &str) -> Vec<Vec<(EdgeKey, bool)>> {
    path_data(svg)
        .iter()
        .map(|d| {
            let mut keys: Vec<(EdgeKey, bool)> = parse_path_data(d)
                .unwrap()
                .iter()
                .flat_map(|s| s.edges.iter().map(Edge::key))
                .collect();
            keys.sort();
            keys
        })
        .collect()
}

/// No piece of the document runs from a node to itself.
fn no_piece_returns_to_its_start(svg: &str) -> bool {
    path_data(svg).iter().all(|d| {
        parse_path_data(d)
            .unwrap()
            .iter()
            .flat_map(|s| s.edges.iter())
            .all(|e| key(e.start()) != key(e.end()))
    })
}

#[test]
fn a_single_shared_piece_between_junctions_keeps_each_fills_direction() {
    // One curve piece shared by two fills, between nodes a third fill also
    // touches: a run that reads the same both ways. Each fill must get it
    // back in its own direction.
    let top = " M 20.00 10.00 L 80.00 10.00 L 80.00 40.00 C 60.00 45.00 40.00 35.00 20.00 40.00 Z";
    let bottom =
        " M 20.00 40.00 C 40.00 35.00 60.00 45.00 80.00 40.00 L 80.00 70.00 L 20.00 70.00 Z";
    let side = " M 80.00 10.00 L 120.00 10.00 L 120.00 70.00 L 80.00 70.00 L 80.00 40.00 Z";
    let svg = document(&[
        ("#ff0000", top.to_owned()),
        ("#00ff00", bottom.to_owned()),
        ("#0000ff", side.to_owned()),
    ]);
    let (out, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 0.5 }).unwrap();
    assert_eq!(stats.segments_before, stats.segments_after, "{out}");
    assert_eq!(walked(&out), walked(&svg), "{out}");
    assert!(no_piece_returns_to_its_start(&out), "{out}");
    let curve = |d: &str| -> EdgeKey {
        parse_path_data(d).unwrap()[0]
            .edges
            .iter()
            .find(|e| !e.line)
            .unwrap()
            .key()
            .0
    };
    let paths = path_data(&out);
    assert_eq!(curve(&paths[0]), curve(&paths[1]));
}

#[test]
fn a_shared_two_piece_ring_keeps_each_fills_winding() {
    // A disc drawn as two half-circle pieces and the hole it fills: a ring of
    // two pieces reads the same both ways from any start, so without the
    // tie-break the second fill would take the first one's winding.
    let hole = " M 0.00 0.00 L 100.00 0.00 L 100.00 100.00 L 0.00 100.00 Z M 30.00 50.00 C 30.00 72.00 70.00 72.00 70.00 50.00 C 70.00 28.00 30.00 28.00 30.00 50.00 Z";
    let disc = " M 30.00 50.00 C 30.00 28.00 70.00 28.00 70.00 50.00 C 70.00 72.00 30.00 72.00 30.00 50.00 Z";
    let svg = document(&[("#ff0000", hole.to_owned()), ("#0000ff", disc.to_owned())]);
    let (out, stats) = simplify_svg(&svg, SimplifyOptions { tolerance: 0.5 }).unwrap();
    assert_eq!(stats.segments_before, stats.segments_after, "{out}");
    assert_eq!(walked(&out), walked(&svg), "{out}");
    assert!(no_piece_returns_to_its_start(&out), "{out}");
    // Both fills hold the same two pieces, each walked the other way round.
    let (a, b) = (&walked(&out)[0], &walked(&out)[1]);
    for (key, reversed) in b {
        assert!(a.contains(&(*key, !reversed)), "{key:?}");
    }
}

#[test]
fn no_merge_cuts_a_corner_of_the_picture() {
    // A region in the picture's corner meets the rest along one shared
    // curve; its outline runs 14 px along each side of the frame to the
    // corner, between the two junctions the curve ends at. At 5 px the two
    // frame pieces fitted one cubic that cut the corner and bulged out of
    // the picture; the corner is kept like a junction, and the far side's
    // frame pieces, which do merge, stay on the frame.
    let svg = "<svg width=\"100pt\" height=\"100pt\" viewBox=\"0 0 100 100\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n\
<path fill=\"#ff0000\" opacity=\"1.00\" d=\" M 86.00 100.00 L 100.00 100.00 L 100.00 86.00 C 94.00 88.00 88.00 94.00 86.00 100.00 Z\" />\n\
<path fill=\"#0000ff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 100.00 0.00 L 100.00 40.00 L 100.00 86.00 C 94.00 88.00 88.00 94.00 86.00 100.00 L 0.00 100.00 Z\" />\n\
</svg>\n";
    let (out, _) = simplify_svg(svg, SimplifyOptions { tolerance: 5. }).unwrap();
    let data = path_data(&out);
    assert!(data[0].contains("L 100.00 100.00 L 100.00 86.00"), "{out}");
    assert!(!data[1].contains("L 100.00 40.00"), "{out}");
    for d in &data {
        for value in d.split_whitespace().filter_map(|v| v.parse::<f64>().ok()) {
            assert!(
                (0. ..=100.).contains(&value),
                "{value} outside the picture: {out}"
            );
        }
    }
}

/// A staircase of 8 px square steps as the engine traces 8x pixel art,
/// each step a straight cubic: one cubic through two steps stays within
/// half a pixel of them but cuts the corner between (0.5 px off each on the
/// defect sweep's sprite32-x8; this staircase is its own, September 23,
/// 2026). No merge crosses a corner turning more than 45 degrees.
#[test]
fn no_merge_cuts_a_square_corner_inside_the_picture() {
    let d = " M 24.00 103.99 C 26.67 104.00 29.33 104.00 31.99 104.01 C 32.00 106.67 32.00 109.33 32.01 111.99 C 34.67 112.00 37.33 112.00 39.99 112.01 C 40.00 114.67 40.00 117.33 40.01 119.99 L 40.01 140.00 L 24.00 140.00 Z";
    let svg = document(&[("#000000", d.to_owned())]);
    let (out, _) = simplify_svg(&svg, SimplifyOptions { tolerance: 0.5 }).unwrap();
    for corner in ["31.99 104.01", "32.01 111.99", "39.99 112.01"] {
        assert!(path_data(&out)[0].contains(corner), "{corner}: {out}");
    }
}

/// A straight piece running into a tight arc: the one cubic through both
/// often swings out the other way first. Every such fit is offered at a
/// tolerance it meets, and only the turn-back rule keeps the two pieces.
#[test]
fn no_merge_adds_a_turn_back_the_pieces_did_not_have() {
    let mut refused = 0;
    for straight in [2., 4., 8., 16.] {
        for radius in [0.5, 1., 2., 4.] {
            for degrees in [30., 60., 90., 120.] {
                let sweep = f64::to_radians(degrees);
                let corner = Point { x: straight, y: 0. };
                let end_angle = sweep - std::f64::consts::FRAC_PI_2;
                let end = arc_point(straight, radius, radius, end_angle);
                let k = 4. / 3. * (sweep / 4.).tan() * radius;
                let line = Edge::line(Point { x: 0., y: 0. }, corner, false);
                let arc = Edge {
                    cubic: Cubic {
                        points: [
                            corner,
                            Point {
                                x: straight + k,
                                y: 0.,
                            },
                            Point {
                                x: end.x + k * end_angle.sin(),
                                y: end.y - k * end_angle.cos(),
                            },
                            end,
                        ],
                    },
                    line: false,
                    implicit: false,
                };
                let (a, b) = (Piece::new(line), Piece::new(arc));
                let mut samples = a.samples.clone();
                samples.extend_from_slice(&b.samples[1..]);
                let (fit, error) = fit_samples(&samples, a.start_direction(), b.end_direction())
                    .expect("a straight piece and an arc fit");
                if !adds_turn_back(&line.cubic, &arc.cubic, &fit) {
                    continue;
                }
                refused += 1;
                let kept = simplify_run(&[line, arc], false, error + 1e-9);
                assert_eq!(
                    kept.len(),
                    2,
                    "{straight} px into a {degrees} degree arc of radius {radius} merged at {error}"
                );
            }
        }
    }
    assert!(
        refused > 0,
        "no fit turned back, so the test proves nothing"
    );
}
