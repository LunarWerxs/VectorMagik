//! Hand edits of single anchor nodes: a node moved to where the user dropped
//! it, and a node deleted. A node is found by its exact coordinates
//! (`simplify::key`), the way rounding and straightening find theirs, so
//! every outline passing through it (both sides of an edge two fills share,
//! every boundary of a junction) changes with it and the seam stays sealed.
//! The handles on either side travel with a moved node, as in a drawing
//! program, so the pieces keep their shape near the other end.

use crate::geometry::{Cubic, Point};
use crate::simplify::{
    add, backwards, fit_samples, key, largest_distance, leaving, normalized, parse_all_paths,
    splice, sub, Edge, Key, Subpath, SAMPLES_PER_SEGMENT,
};
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

/// One node deleted by hand. The two pieces meeting there become one piece
/// from the node before to the node after: with `keep_shape`, one cubic
/// fitted to the curve the two drew, leaving and arriving along their outer
/// tangents (Simplify's merge, `simplify::fit_samples`), so the outline stays
/// where one cubic can follow it; without, the cubic that keeps the two
/// pieces' outer handles, as if the node had never been there (two lines
/// become one line).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeDeletion {
    pub at: Point,
    pub keep_shape: bool,
}

/// Why a node cannot be deleted.
pub const ABSENT: &str = "This node is not in the drawing.";
const JUNCTION: &str = "Three or more boundaries meet at this node, so it stays.";
const OPEN_END: &str = "The end of an open outline stays.";
const TOO_FEW: &str = "An outline keeps at least three nodes.";
/// A closed outline keeps this many nodes at the least.
const MIN_OUTLINE_NODES: usize = 3;

/// Why the node at `at` cannot be deleted, or `None` when it can: it must be
/// in the drawing, join exactly two pieces (a junction of three or more
/// fills joins more), not end an open outline, and leave every closed
/// outline through it at least three nodes.
pub fn deletion_refusal(svg: &str, at: Point) -> Result<Option<&'static str>, String> {
    let (_, paths) = parse_all_paths(svg)?;
    Ok(refusal(&paths, key(at)))
}

fn refusal(paths: &[Vec<Subpath>], node: Key) -> Option<&'static str> {
    let mut incident = HashSet::new();
    let mut too_few = false;
    let mut open_end = false;
    for subpath in paths.iter().flatten() {
        let mut touches = false;
        for edge in &subpath.edges {
            if key(edge.start()) == node || key(edge.end()) == node {
                incident.insert(edge.key().0);
                touches = true;
            }
        }
        if !touches {
            continue;
        }
        if subpath.cyclic() {
            too_few |= subpath.edges.len() <= MIN_OUTLINE_NODES;
        } else if let (Some(first), Some(last)) = (subpath.edges.first(), subpath.edges.last()) {
            open_end |= key(first.start()) == node || key(last.end()) == node;
        }
    }
    if incident.is_empty() {
        Some(ABSENT)
    } else if incident.len() > 2 {
        Some(JUNCTION)
    } else if open_end {
        Some(OPEN_END)
    } else if too_few {
        Some(TOO_FEW)
    } else {
        None
    }
}

/// Where `node` sits in `subpath`: the index of the piece leaving it (the
/// piece arriving is the one before). An open outline's first node has no
/// piece arriving.
fn leaving_index(subpath: &Subpath, node: Key) -> Option<usize> {
    let cyclic = subpath.cyclic();
    (0..subpath.edges.len())
        .filter(|&i| cyclic || i > 0)
        .find(|&i| key(subpath.edges[i].start()) == node)
}

/// The piece without the node between `arriving` and `leaving`, keeping
/// their outer handles.
fn joined(arriving: &Edge, leaving: &Edge) -> Edge {
    if arriving.line && leaving.line {
        return Edge::line(arriving.start(), leaving.end(), false);
    }
    let [p0, p1, _, _] = arriving.cubic.points;
    let [_, _, p2, p3] = leaving.cubic.points;
    Edge {
        cubic: Cubic {
            points: [p0, p1, p2, p3],
        },
        line: false,
        implicit: false,
    }
}

/// One cubic through the curve `arriving` and `leaving` drew, along their
/// outer tangents; two lines running on in one direction stay a line. The
/// plain join has the same tangents, so it is a candidate too: the fit's
/// handle search is local, and whichever stays closer to the curve wins.
fn refitted(arriving: &Edge, leaving_edge: &Edge) -> Edge {
    let (a, b) = (arriving.start(), leaving_edge.end());
    if arriving.line && leaving_edge.line {
        let (u, v) = (sub(arriving.end(), a), sub(b, leaving_edge.start()));
        let cross = u.x * v.y - u.y * v.x;
        let lengths = (u.x.hypot(u.y) * v.x.hypot(v.y)).max(1e-300);
        if (cross / lengths).abs() < 1e-9 && u.x * v.x + u.y * v.y > 0. {
            return Edge::line(a, b, false);
        }
    }
    let sample = |edge: &Edge| {
        (0..=SAMPLES_PER_SEGMENT)
            .map(|k| edge.cubic.evaluate(k as f64 / SAMPLES_PER_SEGMENT as f64))
            .collect::<Vec<Point>>()
    };
    let mut samples = sample(arriving);
    samples.extend_from_slice(&sample(leaving_edge)[1..]);
    let chord = normalized(sub(b, a));
    let start = leaving(arriving).or(chord);
    let end = backwards(leaving_edge).or(chord.map(|c| Point { x: -c.x, y: -c.y }));
    let plain = joined(arriving, leaving_edge);
    let Some((cubic, error)) = start
        .zip(end)
        .and_then(|(start, end)| fit_samples(&samples, start, end))
    else {
        return plain;
    };
    match largest_distance(&samples, &plain.cubic) {
        Some(plain_error) if plain_error <= error => plain,
        _ => Edge {
            cubic,
            line: false,
            implicit: false,
        },
    }
}

/// Delete the listed nodes, one after another. A node that is not in the
/// document, or that `deletion_refusal` refuses there, is skipped (a later
/// trace may no longer have it, or have it as a junction), and a node listed
/// twice takes its first listing. The new piece is worked out once, from the
/// first outline through the node, and every outline through it gets that
/// piece or its reverse, so both fills of a shared edge stay sealed. Returns
/// the rewritten SVG and how many nodes went.
pub fn delete_nodes(svg: &str, deletions: &[NodeDeletion]) -> Result<(String, usize), String> {
    if deletions
        .iter()
        .any(|d| !(d.at.x.is_finite() && d.at.y.is_finite()))
    {
        return Err("A node to delete must have a finite position".into());
    }
    if deletions.is_empty() {
        return Ok((svg.to_owned(), 0));
    }
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let mut listed = HashSet::new();
    let mut deleted = 0;
    for deletion in deletions {
        let node = key(deletion.at);
        if !listed.insert(node) || refusal(&paths, node).is_some() {
            continue;
        }
        let Some((arriving, leaving_edge)) = paths.iter().flatten().find_map(|s| {
            let n = s.edges.len();
            leaving_index(s, node).map(|i| (s.edges[(i + n - 1) % n], s.edges[i]))
        }) else {
            continue;
        };
        let piece = if deletion.keep_shape {
            refitted(&arriving, &leaving_edge)
        } else {
            joined(&arriving, &leaving_edge)
        };
        let pair = (arriving.key().0, leaving_edge.key().0);
        for subpath in paths.iter_mut().flatten() {
            let Some(i) = leaving_index(subpath, node) else {
                continue;
            };
            let n = subpath.edges.len();
            let before = (i + n - 1) % n;
            let here = (subpath.edges[before].key().0, subpath.edges[i].key().0);
            let replacement = if here == pair {
                piece
            } else if here == (pair.1, pair.0) {
                piece.reversed()
            } else {
                continue;
            };
            if i == 0 {
                // The outline's first node: it now starts where its last
                // piece did.
                subpath.edges[0] = replacement;
                subpath.edges.pop();
            } else {
                subpath.edges[before] = replacement;
                subpath.edges.remove(i);
            }
        }
        deleted += 1;
    }
    if deleted == 0 {
        return Ok((svg.to_owned(), 0));
    }
    Ok((splice(svg, &ranges, &paths), deleted))
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

    /// Two fills sharing an S-shaped boundary of two pieces, whose middle
    /// node joins only those two.
    const SHARED_S: &str = "<svg viewBox=\"0 0 20 20\">\
        <path fill=\"#ff0000\" d=\" M 0.00 0.00 L 10.00 0.00 C 12.00 3.00 12.00 7.00 10.00 10.00 C 8.00 13.00 8.00 17.00 10.00 20.00 L 0.00 20.00 Z\" />\
        <path fill=\"#0000ff\" d=\" M 10.00 0.00 L 20.00 0.00 L 20.00 20.00 L 10.00 20.00 C 8.00 17.00 8.00 13.00 10.00 10.00 C 12.00 7.00 12.00 3.00 10.00 0.00 Z\" />\
        </svg>";

    fn cubics(svg: &str) -> Vec<Vec<Edge>> {
        let (_, paths) = parse_all_paths(svg).unwrap();
        paths
            .iter()
            .map(|p| {
                p.iter()
                    .flat_map(|s| s.edges.clone())
                    .filter(|e| !e.line)
                    .collect()
            })
            .collect()
    }

    /// The largest distance from the S's two pieces to `edge`, sampled.
    fn off_the_s(edge: &Edge) -> f64 {
        let (_, paths) = parse_all_paths(SHARED_S).unwrap();
        let curve: Vec<Point> = paths[0]
            .iter()
            .flat_map(|s| s.edges.iter().filter(|e| !e.line))
            .flat_map(|e| (0..=40).map(move |k| e.cubic.evaluate(k as f64 / 40.)))
            .collect();
        let drawn: Vec<Point> = (0..=400)
            .map(|k| edge.cubic.evaluate(k as f64 / 400.))
            .collect();
        curve
            .iter()
            .map(|p| {
                drawn
                    .iter()
                    .map(|q| (p.x - q.x).hypot(p.y - q.y))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0., f64::max)
    }

    #[test]
    fn a_deleted_node_keeps_the_outer_handles_in_both_fills() {
        let at = point(10., 10.);
        assert_eq!(deletion_refusal(SHARED_S, at).unwrap(), None);
        let deletion = NodeDeletion {
            at,
            keep_shape: false,
        };
        let (svg, count) = delete_nodes(SHARED_S, &[deletion]).unwrap();
        assert_eq!(count, 1);
        assert!(!svg.contains("10.00 10.00"), "{svg}");
        // One piece with the two outer handles, each fill walking it its way.
        assert!(svg.contains("C 12.00 3.00 8.00 17.00 10.00 20.00"), "{svg}");
        assert!(svg.contains("C 8.00 17.00 12.00 3.00 10.00 0.00"), "{svg}");
        // Listed twice, the node goes once; listed again, it is gone.
        let (again, count) = delete_nodes(SHARED_S, &[deletion, deletion]).unwrap();
        assert_eq!((again.as_str(), count), (svg.as_str(), 1));
        assert_eq!(delete_nodes(&svg, &[deletion]).unwrap(), (svg.clone(), 0));
    }

    #[test]
    fn a_node_deleted_keeping_the_shape_stays_on_the_curve_and_sealed() {
        let at = point(10., 10.);
        let plain = delete_nodes(
            SHARED_S,
            &[NodeDeletion {
                at,
                keep_shape: false,
            }],
        )
        .unwrap()
        .0;
        let (svg, count) = delete_nodes(
            SHARED_S,
            &[NodeDeletion {
                at,
                keep_shape: true,
            }],
        )
        .unwrap();
        assert_eq!(count, 1);
        assert!(!svg.contains("10.00 10.00"), "{svg}");
        let fills = cubics(&svg);
        let (red, blue) = (&fills[0], &fills[1]);
        assert_eq!((red.len(), blue.len()), (1, 1), "{svg}");
        // Both fills carry the same cubic, written the same, one reversed.
        let n = |p: Point| format!("{:.2} {:.2}", p.x, p.y);
        let [a0, a1, a2, a3] = red[0].cubic.points;
        let [b0, b1, b2, b3] = blue[0].cubic.points;
        assert_eq!(
            [n(a0), n(a1), n(a2), n(a3)],
            [n(b3), n(b2), n(b1), n(b0)],
            "{svg}"
        );
        // It follows the S far closer than the plain join does.
        let kept = off_the_s(&red[0]);
        let joined = off_the_s(&cubics(&plain)[0][0]);
        assert!(kept < 0.5 && kept < joined / 2., "{kept} against {joined}");
    }

    #[test]
    fn keeping_the_shape_is_never_further_off_than_the_plain_join() {
        // A corner between a line and a curve, walked both ways.
        let line = Edge::line(point(0., 0.), point(10., 0.), false);
        let curve = Edge {
            cubic: Cubic {
                points: [
                    point(10., 0.),
                    point(13., 3.),
                    point(13., 7.),
                    point(10., 10.),
                ],
            },
            line: false,
            implicit: false,
        };
        for (a, b) in [(line, curve), (curve.reversed(), line.reversed())] {
            let samples: Vec<Point> = [a, b]
                .iter()
                .enumerate()
                .flat_map(|(i, e)| {
                    (usize::from(i > 0)..=SAMPLES_PER_SEGMENT)
                        .map(move |k| e.cubic.evaluate(k as f64 / SAMPLES_PER_SEGMENT as f64))
                })
                .collect();
            let kept = largest_distance(&samples, &refitted(&a, &b).cubic).unwrap();
            let plain = largest_distance(&samples, &joined(&a, &b).cubic).unwrap();
            assert!(kept <= plain, "{kept} against {plain}");
        }
    }

    #[test]
    fn an_outlines_first_node_goes_and_two_lines_become_one() {
        // The red square's first node: its closing line and first line join.
        let (svg, count) = delete_nodes(
            SVG,
            &[NodeDeletion {
                at: point(0., 0.),
                keep_shape: false,
            }],
        )
        .unwrap();
        assert_eq!(count, 1);
        assert!(
            svg.contains(
                "d=\" M 0.00 10.00 L 10.00 0.00 C 12.00 3.00 12.00 7.00 10.00 10.00 L 0.00 10.00 Z\""
            ),
            "{svg}"
        );
        // Two lines running on in one direction stay one line when refitted.
        let straight = "<svg viewBox=\"0 0 20 20\">\
            <path fill=\"#000000\" d=\" M 0.00 0.00 L 5.00 0.00 L 10.00 0.00 L 10.00 10.00 L 0.00 10.00 Z\" />\
            </svg>";
        let (svg, _) = delete_nodes(
            straight,
            &[NodeDeletion {
                at: point(5., 0.),
                keep_shape: true,
            }],
        )
        .unwrap();
        assert!(
            svg.contains("d=\" M 0.00 0.00 L 10.00 0.00 L 10.00 10.00 L 0.00 10.00 Z\""),
            "{svg}"
        );
    }

    #[test]
    fn junctions_open_ends_and_the_last_three_nodes_stay() {
        // Where the two squares and their shared curve meet, three pieces.
        let junction = point(10., 10.);
        assert_eq!(deletion_refusal(SVG, junction).unwrap(), Some(JUNCTION));
        assert_eq!(
            delete_nodes(
                SVG,
                &[NodeDeletion {
                    at: junction,
                    keep_shape: true,
                }],
            )
            .unwrap(),
            (SVG.to_owned(), 0)
        );
        let triangle = "<svg viewBox=\"0 0 20 20\">\
            <path fill=\"#000000\" d=\" M 0.00 0.00 L 10.00 0.00 L 10.00 10.00 Z\" />\
            <path fill=\"#ffffff\" d=\" M 12.00 0.00 L 15.00 0.00 L 18.00 3.00\" />\
            </svg>";
        assert_eq!(
            deletion_refusal(triangle, point(10., 0.)).unwrap(),
            Some(TOO_FEW)
        );
        assert_eq!(
            deletion_refusal(triangle, point(12., 0.)).unwrap(),
            Some(OPEN_END)
        );
        // An open outline's middle node is an ordinary one.
        assert_eq!(deletion_refusal(triangle, point(15., 0.)).unwrap(), None);
        assert_eq!(
            deletion_refusal(triangle, point(3., 3.)).unwrap(),
            Some(ABSENT)
        );
        assert!(delete_nodes(
            SVG,
            &[NodeDeletion {
                at: point(f64::NAN, 0.),
                keep_shape: false,
            }]
        )
        .is_err());
    }
}
