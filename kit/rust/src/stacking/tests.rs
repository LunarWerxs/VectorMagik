use super::*;
/// Path data for a closed polygon from its corners, a line per side back to
/// the first.
fn polygon(points: &[(f64, f64)]) -> String {
    let mut d = format!(" M {:.2} {:.2}", points[0].0, points[0].1);
    for (x, y) in points.iter().skip(1).chain(std::iter::once(&points[0])) {
        d.push_str(&format!(" L {x:.2} {y:.2}"));
    }
    d
}

fn document(paths: &[(&str, &str, String)]) -> String {
    let mut svg = String::from(
        "<svg width=\"30pt\" height=\"20pt\" viewBox=\"0 0 30 20\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n",
    );
    for (fill, opacity, d) in paths {
        svg.push_str(&format!(
            "<g id=\"{fill}ff\">\n<path fill=\"{fill}\" opacity=\"{opacity}\" d=\"{d} Z\" />\n</g>\n"
        ));
    }
    svg.push_str("</svg>\n");
    svg
}

/// Background (0..30 x 0..20) with two squares side by side, sharing the
/// edge x = 15: the background has them as holes, walked the other way.
fn two_squares(second_opacity: &str) -> String {
    let a = [(5., 5.), (15., 5.), (15., 15.), (5., 15.)];
    let b = [(15., 5.), (25., 5.), (25., 15.), (15., 15.)];
    let reversed = |q: &[(f64, f64)]| q.iter().rev().copied().collect::<Vec<_>>();
    let frame = polygon(&[(0., 20.), (0., 0.), (30., 0.), (30., 20.)]);
    // The two holes are one outline round both squares.
    let hole = polygon(&reversed(&[
        (5., 5.),
        (15., 5.),
        (25., 5.),
        (25., 15.),
        (15., 15.),
        (5., 15.),
    ]));
    document(&[
        ("#ffffff", "1.00", format!("{frame}{hole}")),
        ("#ff0000", "1.00", polygon(&a)),
        ("#0000ff", second_opacity, polygon(&b)),
    ])
}

fn subpaths(svg: &str) -> Vec<Vec<Subpath>> {
    parse_all_paths(svg).unwrap().1
}

/// The d attribute and, for a strip, the stroke colour of every path.
fn elements(svg: &str) -> Vec<(Option<String>, String)> {
    let (ranges, _) = parse_all_paths(svg).unwrap();
    ranges
        .iter()
        .map(|&(start, end)| {
            let head = &svg[svg[..start].rfind("<path").unwrap()..start];
            let stroke = head
                .find(" stroke=\"")
                .map(|at| head[at + 9..at + 16].to_owned());
            (stroke, svg[start..end].to_owned())
        })
        .collect()
}

#[test]
fn a_strip_of_the_earlier_colour_runs_under_each_edge_a_later_region_shares() {
    let svg = two_squares("1.00");
    let (out, stats) = stack_svg(&svg).unwrap();
    assert_eq!(
        stats,
        StackStats {
            regions: 3,
            stripped: 2
        }
    );
    let (before, after) = (elements(&svg), elements(&out));
    // White, its strip, red, its strip, blue: the regions' own data as it
    // was, each strip in its region's colour.
    assert_eq!(after.len(), 5, "{out}");
    assert_eq!(after[0], before[0]);
    assert_eq!(after[2], before[1]);
    assert_eq!(after[4], before[2]);
    assert_eq!(after[1].0.as_deref(), Some("#ffffff"));
    assert_eq!(after[3].0.as_deref(), Some("#ff0000"));
    // Under the white: the whole hole round both squares, closed; under the
    // red: the one edge the blue square shares.
    let strips = subpaths(&out);
    assert_eq!(
        (
            strips[1].len(),
            strips[1][0].edges.len(),
            strips[1][0].closed
        ),
        (1, 6, true)
    );
    assert_eq!(
        (
            strips[3].len(),
            strips[3][0].edges.len(),
            strips[3][0].closed
        ),
        (1, 1, false)
    );
    let shared = strips[3][0].edges[0];
    assert_eq!((shared.start().x, shared.end().x), (15., 15.));
    // No coordinate moved: every piece written is one of the document's.
    let known: std::collections::HashSet<EdgeKey> = subpaths(&svg)
        .iter()
        .flatten()
        .flat_map(|s| s.edges.iter().map(|e| e.key().0))
        .collect();
    for edge in strips.iter().flatten().flat_map(|s| s.edges.iter()) {
        assert!(known.contains(&edge.key().0), "{edge:?}");
    }
}

#[test]
fn a_translucent_neighbour_gets_no_strip_under_it() {
    let (out, stats) = stack_svg(&two_squares("0.50")).unwrap();
    // Only the white's run round the red square, one open run of its three
    // outer sides; nothing under the translucent blue.
    assert_eq!(stats.stripped, 1, "{out}");
    let strips = subpaths(&out);
    assert_eq!(strips.len(), 4);
    assert_eq!(
        (
            strips[1].len(),
            strips[1][0].edges.len(),
            strips[1][0].closed
        ),
        (1, 3, false)
    );
    for edge in &strips[1][0].edges {
        assert!(edge.start().x <= 15. && edge.end().x <= 15., "{out}");
    }
}

#[test]
fn stroked_documents_and_lone_regions_are_left_as_they_are() {
    let stroked = two_squares("1.00").replace(
        "<path fill=\"#ff0000\"",
        "<path fill=\"#ff0000\" stroke=\"#ff0000\" stroke-width=\"0.5\"",
    );
    assert_eq!(stack_svg(&stroked).unwrap().0, stroked);
    let lone = document(&[(
        "#123456",
        "1.00",
        polygon(&[(0., 20.), (0., 0.), (30., 0.), (30., 20.)]),
    )]);
    let (out, stats) = stack_svg(&lone).unwrap();
    assert_eq!((out, stats.stripped), (lone.clone(), 0));
}
