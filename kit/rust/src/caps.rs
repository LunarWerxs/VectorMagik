//! Square ends for pixel-edged strokes.
//!
//! The engine smooths the square end of a thin pixel-edged stroke into a
//! point: both sides run to one node and meet there head-on (the defect
//! sweep of September 23, 2026: "square stroke ends tapered into blades").
//! Simplify then draws each side as one long piece from tip to tip, and
//! straightening turns each into the chord to that tip, so the band becomes
//! a long thin wedge: a 1 px line at 30 degrees kept 53% of its ink, one at
//! half a degree 16%, and a 400 px line broke in ten places.
//!
//! A node of a thin, long region (twice its area over its outline's length
//! under `MAX_WIDTH`, its outline at least `MIN_ELONGATION` times that)
//! where the outline turns back by more than `MIN_TURN` and no third region
//! meets is such a tip. Only a straight stroke is capped: exactly two tips,
//! and its whole outline within `STRAIGHT` of the line between them (or
//! `STRAIGHT_PER_WIDTH` of its width, if more). Capping every tip made pixel-edged lettering less
//! faithful (colour error 4.94 -> 5.09), and reading a square end off the
//! pixels near the tip missed the drawn lines' ends as well (caps.md). A tip
//! becomes two corners half the region's width to either side of its axis,
//! joined by a line: the region's own sides end at them and its neighbour's
//! copy of each side does too, with the cap drawn the other way round, so
//! the two stay sealed. Each corner's handle keeps its reach along the
//! axis only, so the side leaves the corner parallel to the stroke.
use crate::geometry::{dist, to_segment, Point};
use crate::shapes::islands;
use crate::simplify::{arriving, junction_keys, key, leaving, parse_all_paths, splice, Edge};
use std::collections::HashMap;

/// Pixels: a region at most this wide on average may be a stroke.
const MAX_WIDTH: f64 = 3.;
/// A stroke's outline is at least this many times its width.
const MIN_ELONGATION: f64 = 8.;
/// Degrees the outline turns at a tip, at least (180 is straight back).
const MIN_TURN: f64 = 150.;
/// Pixels: a straight stroke's outline lies this close to the line between
/// its two tips, or within three quarters of its width if that is more (a
/// 1 px line's traced sides bulge about 0.7 px, a broken ring's pieces
/// bow more: 1.5 px on a 22 px piece of a radius 40 ring).
const STRAIGHT: f64 = 1.;
const STRAIGHT_PER_WIDTH: f64 = 0.75;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapStats {
    pub caps: usize,
}

/// The two corners a tip becomes: the one on the side its outline arrives
/// from and the one on the side it leaves along, with the axis's normal to
/// tell the sides apart.
#[derive(Clone, Copy)]
struct Cap {
    tip: Point,
    /// Along the stroke, away from the tip.
    axis: Point,
    normal: Point,
    arrive_side: f64,
    arrive: Point,
    leave: Point,
}

impl Cap {
    /// The corner for a side that runs away from the tip along `away`: its
    /// tangent there, not its far end, tells the side (both sides leave the
    /// tip almost along the axis, and an axis a fraction of a degree off
    /// would put a far end 140 px away on the wrong side).
    fn corner_for(&self, away: Point) -> Point {
        let side = away.x * self.normal.x + away.y * self.normal.y;
        if (side >= 0.) == (self.arrive_side >= 0.) {
            self.arrive
        } else {
            self.leave
        }
    }

    /// The handle at a corner: the old handle's reach along the stroke's
    /// axis, none across it, so the side leaves the corner parallel to the
    /// stroke. Shifted with its end instead, a lens's handles pushed its
    /// bulge out by the corner's offset (a 2 px bar came out up to 4 px).
    fn handle(&self, corner: Point, old: Point) -> Point {
        let reach = (old.x - self.tip.x) * self.axis.x + (old.y - self.tip.y) * self.axis.y;
        Point {
            x: corner.x + self.axis.x * reach,
            y: corner.y + self.axis.y * reach,
        }
    }
}

/// `svg` with every pointed end of a thin stroke made square.
pub fn square_caps(svg: &str) -> Result<(String, CapStats), String> {
    let found = islands(svg)?;
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let junctions = junction_keys(&paths);
    let round = |p: Point| Point {
        x: (p.x * 100.).round() / 100.,
        y: (p.y * 100.).round() / 100.,
    };
    let mut caps: HashMap<[u64; 2], Cap> = HashMap::new();
    for island in &found {
        let perimeter: f64 = std::iter::once(&island.outline)
            .chain(&island.hole_outlines)
            .map(|l| {
                (0..l.len())
                    .map(|k| dist(l[k], l[(k + 1) % l.len()]))
                    .sum::<f64>()
            })
            .sum();
        if perimeter <= 0. {
            continue;
        }
        let width = 2. * island.area() / perimeter;
        if width >= MAX_WIDTH || perimeter < MIN_ELONGATION * width {
            continue;
        }
        let edges = &paths[island.path][island.outer].edges;
        let n = edges.len();
        if n < 2 {
            continue;
        }
        let mut tips: Vec<([u64; 2], Cap)> = Vec::new();
        for k in 0..n {
            let (before, after) = (&edges[(k + n - 1) % n], &edges[k]);
            let tip = after.start();
            if junctions.contains(&key(tip)) {
                continue;
            }
            let (Some(a), Some(b)) = (arriving(before), leaving(after)) else {
                continue;
            };
            if a.x * b.x + a.y * b.y > -MIN_TURN.to_radians().cos().abs() {
                continue;
            }
            // The axis runs from the tip into the stroke, between the two
            // sides' directions away from it (-a and b).
            let (sx, sy) = (b.x - a.x, b.y - a.y);
            let len = sx.hypot(sy);
            if len < 1e-9 {
                continue;
            }
            let normal = Point {
                x: -sy / len,
                y: sx / len,
            };
            // The two sides leave the tip on opposite sides of the axis.
            let (back, on) = (
                -(a.x * normal.x + a.y * normal.y),
                b.x * normal.x + b.y * normal.y,
            );
            if back * on >= 0. {
                continue;
            }
            let axis = Point {
                x: sx / len,
                y: sy / len,
            };
            let h = width / 2. * back.signum();
            let arrive = round(Point {
                x: tip.x + normal.x * h,
                y: tip.y + normal.y * h,
            });
            let leave = round(Point {
                x: tip.x - normal.x * h,
                y: tip.y - normal.y * h,
            });
            tips.push((
                key(tip),
                Cap {
                    tip,
                    axis,
                    normal,
                    arrive_side: back,
                    arrive,
                    leave,
                },
            ));
        }
        // A straight stroke: two tips, the outline along the line between.
        let [(_, first), (_, second)] = tips[..] else {
            continue;
        };
        let reach = STRAIGHT.max(STRAIGHT_PER_WIDTH * width);
        if island
            .outline
            .iter()
            .any(|&p| to_segment(p, first.tip, second.tip) > reach)
        {
            continue;
        }
        caps.extend(tips);
    }
    if caps.is_empty() {
        return Ok((svg.to_owned(), CapStats::default()));
    }
    for subpath in paths.iter_mut().flatten() {
        let closed = match (subpath.edges.first(), subpath.edges.last()) {
            (Some(first), Some(last)) => key(last.end()) == key(first.start()),
            _ => false,
        };
        let mut out: Vec<Edge> = Vec::with_capacity(subpath.edges.len() + 2);
        for edge in &subpath.edges {
            let mut e = *edge;
            let away_start = leaving(edge);
            let away_end = arriving(edge).map(|d| Point { x: -d.x, y: -d.y });
            let points = &mut e.cubic.points;
            if let (Some(cap), Some(away)) = (caps.get(&key(points[0])), away_start) {
                points[0] = cap.corner_for(away);
                points[1] = cap.handle(points[0], points[1]);
            }
            if let (Some(cap), Some(away)) = (caps.get(&key(points[3])), away_end) {
                points[3] = cap.corner_for(away);
                points[2] = cap.handle(points[3], points[2]);
            }
            if e.line {
                e = Edge::line(e.cubic.points[0], e.cubic.points[3], e.implicit);
            }
            // The cap between the corner the last edge ended at and the one
            // this one starts from.
            if let Some(last) = out.last() {
                if key(last.end()) != key(e.start()) {
                    out.push(Edge::line(last.end(), e.start(), false));
                }
            }
            out.push(e);
        }
        if let (Some(first), Some(last)) = (out.first().copied(), out.last().copied()) {
            if closed && key(last.end()) != key(first.start()) {
                out.push(Edge::line(last.end(), first.start(), false));
            }
        }
        subpath.edges = out;
    }
    Ok((splice(svg, &ranges, &paths), CapStats { caps: caps.len() }))
}

#[cfg(test)]
mod tests;
