//! Hand edits of single anchor nodes: a node moved to where the user dropped
//! it. A node is found by its exact coordinates (`simplify::key`), the way
//! rounding and straightening find theirs, so every outline passing through
//! it (both sides of an edge two fills share, every boundary of a junction)
//! moves with it and the seam stays sealed. The handles on either side travel
//! with the node, as in a drawing program, so the pieces keep their shape
//! near the other end.

use crate::geometry::{Cubic, Point};
use crate::simplify::{add, key, parse_all_paths, splice, sub, Edge, Key};
use std::collections::{HashMap, HashSet};

/// One node moved from where the document has it to where it was dropped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeMove {
    pub from: Point,
    pub to: Point,
}

/// One piece of an outline, as `pieces_at` finds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodePiece {
    pub cubic: Cubic,
    pub line: bool,
}
impl NodePiece {
    /// The piece with its end at `from` moved to `to`, its handle there
    /// following (a line stays a line).
    pub fn moved(self, from: Point, to: Point) -> Self {
        let edge = Edge {
            cubic: self.cubic,
            line: self.line,
            implicit: false,
        };
        let wanted = HashMap::from([(key(from), to)]);
        let edge = moved_edge(&edge, &wanted).unwrap_or(edge);
        Self {
            cubic: edge.cubic,
            line: edge.line,
        }
    }
}

/// `p` as the document writes it (two decimals), so a moved node's key is
/// the one the next parse of the document finds.
pub fn written(p: Point) -> Point {
    let two = |v: f64| format!("{v:.2}").parse::<f64>().unwrap_or(v) + 0.;
    Point {
        x: two(p.x),
        y: two(p.y),
    }
}

/// `edge` with each end listed in `moves` moved to its target and the handle
/// at that end carried along; `None` when neither end is listed.
fn moved_edge(edge: &Edge, moves: &HashMap<Key, Point>) -> Option<Edge> {
    let [p0, p1, p2, p3] = edge.cubic.points;
    let start = moves.get(&key(p0)).copied();
    let end = moves.get(&key(p3)).copied();
    if start.is_none() && end.is_none() {
        return None;
    }
    let (q0, q3) = (start.unwrap_or(p0), end.unwrap_or(p3));
    if edge.line {
        return Some(Edge::line(q0, q3, edge.implicit));
    }
    Some(Edge {
        cubic: Cubic {
            points: [q0, add(p1, sub(q0, p0)), add(p2, sub(q3, p3)), q3],
        },
        line: false,
        implicit: edge.implicit,
    })
}

/// Move the listed nodes. A move whose node is not in the document is
/// skipped (a later trace may no longer have it), and a node moved twice
/// takes the first move listed. Returns the rewritten SVG and how many
/// nodes moved.
pub fn move_nodes(svg: &str, moves: &[NodeMove]) -> Result<(String, usize), String> {
    if moves.iter().any(|m| {
        ![m.from.x, m.from.y, m.to.x, m.to.y]
            .iter()
            .all(|v| v.is_finite())
    }) {
        return Err("A node can only move to a finite position".into());
    }
    let mut wanted: HashMap<Key, Point> = HashMap::new();
    for m in moves {
        wanted.entry(key(m.from)).or_insert_with(|| written(m.to));
    }
    if wanted.is_empty() {
        return Ok((svg.to_owned(), 0));
    }
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let mut hit: HashSet<Key> = HashSet::new();
    for subpath in paths.iter_mut().flatten() {
        for edge in &mut subpath.edges {
            if let Some(moved) = moved_edge(edge, &wanted) {
                for end in [edge.start(), edge.end()] {
                    if wanted.contains_key(&key(end)) {
                        hit.insert(key(end));
                    }
                }
                *edge = moved;
            }
        }
    }
    if hit.is_empty() {
        return Ok((svg.to_owned(), 0));
    }
    Ok((splice(svg, &ranges, &paths), hit.len()))
}

/// Every piece of the document that starts or ends at `at`, each once (the
/// two copies of an edge two fills share are one piece): what a node being
/// dragged pulls along.
pub fn pieces_at(svg: &str, at: Point) -> Result<Vec<NodePiece>, String> {
    let (_, paths) = parse_all_paths(svg)?;
    let target = key(at);
    let mut seen = HashSet::new();
    let mut pieces = Vec::new();
    for edge in paths.iter().flatten().flat_map(|s| &s.edges) {
        if key(edge.start()) != target && key(edge.end()) != target {
            continue;
        }
        if seen.insert(edge.key().0) {
            pieces.push(NodePiece {
                cubic: edge.cubic,
                line: edge.line,
            });
        }
    }
    Ok(pieces)
}

/// The outline a rounded corner's drag leaves: `pieces`, the two meeting at
/// `from` in the document before rounding, carried to `to` and rounded there
/// with `reach` as `simplify::smooth_nodes` rounds them in a document whose
/// view box is `frame` (its cap depends on it). Any other number of pieces
/// (a junction, which rounding leaves alone) comes back moved only.
pub fn rounded_at(
    pieces: &[NodePiece],
    from: Point,
    to: Point,
    reach: f64,
    frame: (f64, f64),
) -> Vec<NodePiece> {
    // Where the drop will write the node, for every branch alike.
    let to = written(to);
    let moved: Vec<NodePiece> = pieces.iter().map(|p| p.moved(from, to)).collect();
    let [a, b] = pieces else {
        return moved;
    };
    let node = key(from);
    let edge = |p: &NodePiece| Edge {
        cubic: p.cubic,
        line: p.line,
        implicit: false,
    };
    // One piece arrives at the node, the other leaves it.
    let (a, b) = (edge(a), edge(b));
    let arriving = if key(a.end()) == node {
        a
    } else {
        a.reversed()
    };
    let leaving = if key(b.start()) == node {
        b
    } else {
        b.reversed()
    };
    if key(arriving.end()) != node || key(leaving.start()) != node {
        return moved;
    }
    let wanted = HashMap::from([(node, to)]);
    let arriving = moved_edge(&arriving, &wanted).unwrap_or(arriving);
    let leaving = moved_edge(&leaving, &wanted).unwrap_or(leaving);
    let n = |p: Point| format!("{:.2} {:.2}", p.x, p.y);
    let piece = |e: &Edge| {
        let [_, p1, p2, p3] = e.cubic.points;
        if e.line {
            format!(" L {}", n(p3))
        } else {
            format!(" C {} {} {}", n(p1), n(p2), n(p3))
        }
    };
    let svg = format!(
        "<svg viewBox=\"0 0 {} {}\">\n<path fill=\"#000000\" d=\" M {}{}{} Z\" />\n</svg>",
        frame.0,
        frame.1,
        n(arriving.start()),
        piece(&arriving),
        piece(&leaving)
    );
    let rounding = crate::simplify::Rounding { at: to, reach };
    let Ok((rounded, 1)) = crate::simplify::smooth_nodes(&svg, &[rounding]) else {
        return moved;
    };
    let Ok((_, paths)) = parse_all_paths(&rounded) else {
        return moved;
    };
    // Everything but the line that closed the two pieces into an outline.
    let (open_start, open_end) = (key(written(arriving.start())), key(written(leaving.end())));
    paths
        .iter()
        .flatten()
        .flat_map(|s| &s.edges)
        .filter(|e| !(e.line && key(e.start()) == open_end && key(e.end()) == open_start))
        .map(|e| NodePiece {
            cubic: e.cubic,
            line: e.line,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = "<svg viewBox=\"0 0 20 20\">\
        <path fill=\"#ff0000\" d=\" M 0.00 0.00 L 10.00 0.00 C 12.00 3.00 12.00 7.00 10.00 10.00 L 0.00 10.00 Z\" />\
        <path fill=\"#0000ff\" d=\" M 10.00 0.00 L 20.00 0.00 L 20.00 10.00 L 10.00 10.00 C 12.00 7.00 12.00 3.00 10.00 0.00 Z\" />\
        </svg>";

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn a_shared_node_moves_in_every_outline_with_its_handles() {
        let moves = [NodeMove {
            from: point(10., 10.),
            to: point(11.254, 12.),
        }];
        let (moved, count) = move_nodes(SVG, &moves).unwrap();
        assert_eq!(count, 1);
        // Written as the document writes numbers, in both fills.
        assert_eq!(moved.matches("11.25 12.00").count(), 2, "{moved}");
        assert!(!moved.contains("10.00 10.00"), "{moved}");
        // The handle at the moved end followed it; the far one stayed.
        assert!(
            moved.contains("C 12.00 3.00 13.25 9.00 11.25 12.00"),
            "{moved}"
        );
        assert!(
            moved.contains("C 13.25 9.00 12.00 3.00 10.00 0.00"),
            "{moved}"
        );
        // Nothing else changed.
        assert_eq!(moved.matches("10.00 0.00").count(), 3, "{moved}");
    }

    #[test]
    fn a_node_that_is_not_there_changes_nothing() {
        let moves = [NodeMove {
            from: point(5., 5.),
            to: point(6., 6.),
        }];
        assert_eq!(move_nodes(SVG, &moves).unwrap(), (SVG.to_owned(), 0));
        assert!(move_nodes(
            SVG,
            &[NodeMove {
                from: point(10., 10.),
                to: point(f64::NAN, 1.),
            }]
        )
        .is_err());
    }

    #[test]
    fn the_pieces_at_a_node_count_a_shared_edge_once() {
        let pieces = pieces_at(SVG, point(10., 10.)).unwrap();
        // The shared curve once, the red bottom line, the blue bottom line.
        assert_eq!(pieces.len(), 3, "{pieces:?}");
        let curve = pieces.iter().find(|p| !p.line).unwrap();
        let dragged = curve.moved(point(10., 10.), point(11., 12.));
        assert!(dragged.cubic.points.contains(&point(11., 12.)));
        assert!(dragged.cubic.points.contains(&point(10., 0.)));
    }
    #[test]
    fn a_rounded_corner_drags_as_its_rounding() {
        let line = |a: Point, b: Point| NodePiece {
            cubic: Cubic {
                points: [a, a, b, b],
            },
            line: true,
        };
        // Given in either direction, as the document happens to store them.
        let pieces = [
            line(point(10., 0.), point(0., 0.)),
            line(point(10., 0.), point(10., 10.)),
        ];
        let outline = rounded_at(&pieces, point(10., 0.), point(12., 0.), 0.5, (20., 20.));
        let ends: Vec<Point> = outline
            .iter()
            .flat_map(|p| [p.cubic.points[0], p.cubic.points[3]])
            .collect();
        assert!(outline.len() >= 3, "{outline:?}");
        assert!(ends.contains(&point(0., 0.)) && ends.contains(&point(10., 10.)));
        // The corner at the new place is cut round, not drawn sharp.
        assert!(!ends.contains(&point(12., 0.)), "{outline:?}");
        assert!(outline.iter().any(|p| !p.line));
    }
}
