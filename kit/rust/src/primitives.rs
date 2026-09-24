//! Closed outlines drawn as the shape the pixels show: a circle, an ellipse,
//! a rectangle or a rounded rectangle, fitted to the source picture along
//! the outline and kept only when it explains those pixels at least as well
//! as the traced outline does.
//!
//! The engine traces a small shape from its pixel steps and places its
//! corners where the steps say: on a pixel-edged picture a 12 px rounded
//! square came out as a square turned 8 degrees (each corner slid one step
//! round the rounded corner, the same way at every corner), a 6 px disc as
//! a triangle and a 9 px one as a pentagon, and on an anti-aliased picture
//! small rounded squares came out with bulging sides
//! (kit/fixtures/shapes/shape-small-details*.png, September 23, 2026).
//! `regularize` judges an outline by the outline alone, which is exactly
//! what is wrong there, so this pass asks the pixels instead.
//!
//! For every outline that closes on itself with no junction and no node on
//! the picture's frame, the pixels within `BAND` of it that are a mix of the
//! two fills either side of it (and nearer this outline than any other) say
//! how much of each pixel the inside fill covers. Each kind of shape is
//! fitted to where that coverage crosses one half, and scored by the summed
//! difference between the coverage it predicts and the coverage seen, the
//! same sum the traced outline gets: at the pixel's centre on pixel-edged
//! artwork, over `SUBSAMPLES` squared points of the pixel on anti-aliased
//! artwork. A shape is a candidate only when it is at least as faithful as
//! the traced outline and stays within `MAX_MOVE` of it everywhere; of the
//! candidates the one with the least error plus `PARAMETER_COST` per free
//! parameter wins, so a circle beats an ellipse that explains the pixels
//! only as well. Both fills sharing the outline get the same shape, and a
//! node the desktop forces is never moved.
//!
//! Owned post-processing, run last in the chain (after straighten, whose
//! bow tolerance would flatten a small rounded corner's arcs into chamfers).
use crate::geometry::{Cubic, Point};
use crate::raster::Raster;
use crate::regularize::{fit_circle, fit_ellipse};
use crate::simplify::{
    add, canonical, distance, dot, junction_keys, key, length, parse_all_paths, runs_of, scale,
    splice, sub, Edge, EdgeKey, Key,
};
use std::collections::{HashMap, HashSet};
use std::f64::consts::FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimitiveOptions {
    /// The picture was traced as anti-aliased artwork: an edge pixel carries
    /// the share of it the shape covers, judged over its area; otherwise a
    /// pixel is inside or outside by its centre.
    pub anti_aliased: bool,
}
impl PrimitiveOptions {
    /// The options for a document traced with the basic preset `preset`
    /// (`crate::basic_preset_code`), or `None` for photographs, whose
    /// thousands of small regions are texture, not drawn shapes.
    pub fn for_preset(preset: usize) -> Option<Self> {
        use crate::{basic_preset_code, ImageCategory, Quality};
        let kind = |category| {
            [Quality::High, Quality::Medium, Quality::Low]
                .into_iter()
                .any(|q| basic_preset_code(category, q) == preset)
        };
        if kind(ImageCategory::Photograph) {
            return None;
        }
        Some(Self {
            anti_aliased: kind(ImageCategory::AntiAliasedArtwork),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrimitiveStats {
    /// Closed outlines with no junction and no node on the frame.
    pub outlines: usize,
    /// Of those, the ones the pixels could judge (two fills with enough
    /// contrast, enough of the outline's edge seen).
    pub judged: usize,
    pub circles: usize,
    pub ellipses: usize,
    pub rectangles: usize,
    pub rounded_rectangles: usize,
}
impl PrimitiveStats {
    pub fn replaced(&self) -> usize {
        self.circles + self.ellipses + self.rectangles + self.rounded_rectangles
    }
}

/// Pixels whose centre lies within this many source pixels of the traced
/// outline are its evidence: the engine's outline strays from the edge by
/// about a pixel at worst (the turned square's corners).
const BAND: f64 = 2.5;
/// No point of a replacement may lie further than this from the traced
/// outline, nor any point of the traced outline further from it.
const MAX_MOVE: f64 = 1.5;
/// A traced outline within this of the best shape everywhere already is
/// that shape (`regularize` draws a wobbly circle true), and stays: redrawn
/// from the pixels it moved by a hundredth or two of a pixel, and on the
/// anti-aliased exact circles the pixels' own read of coverage runs 0.007
/// low (resvg), so the redrawn circles came out 0.02 px small. On
/// pixel-edged artwork, where the error counts misjudged pixel centres and
/// no colour is read between two fills, a shape that misjudges fewer
/// centres still wins (the regularized exact ellipse misjudged 7, its fit
/// 1).
const MIN_MOVE: f64 = 0.1;
/// The score a free parameter costs, in pixels of coverage error: a shape
/// with one more parameter must explain half a pixel more to win.
const PARAMETER_COST: f64 = 0.5;
/// The two fills must differ by at least this much (premultiplied RGBA
/// distance, 0..510) for a pixel's mix of them to say anything.
const MIN_CONTRAST: f64 = 32.;
/// A pixel further than this share of the two fills' distance (and at least
/// `MIN_RESIDUAL` levels) from every mix of them shows a third colour.
const THIRD_COLOUR: f64 = 0.3;
const MIN_RESIDUAL: f64 = 16.;
/// The pixels at least this far from the outline on either side show that
/// side's colour; at least `MIN_DEEP` of them, or the fill is taken.
const DEEP: f64 = 1.;
const MIN_DEEP: usize = 3;
/// A shape may leave this many more pixels misjudged per pixel of outline
/// than the traced outline does: a renderer's hard edges miss the exact
/// shape by a pixel here and there (resvg's crisp edges: 11 pixel centres
/// round a 70 px rounded rectangle), which the traced outline follows as
/// wobble and a true shape does not.
const SLACK_PER_PX: f64 = 0.04;
/// A forced node this close to a node of an outline's replacement is that
/// node as the document wrote it (to hundredths of a pixel).
const FORCED_REACH: f64 = 0.01;
/// Fewer edge crossings than this is too little to fit a shape to.
const MIN_EDGE_POINTS: usize = 8;
/// Points per side of a pixel when anti-aliased coverage is judged.
const SUBSAMPLES: usize = 8;
/// A pixel whose centre lies this far from a shape's outline is wholly on
/// one side of it (half a pixel's diagonal).
const WHOLE_PIXEL: f64 = 0.7072;
/// The smallest half-width a fitted shape may have.
const MIN_HALF: f64 = 0.75;
/// Points per curved piece when an outline is flattened.
const FLATTEN_STEPS: usize = 16;
/// Outlines with more pixels of evidence than this are left alone: they are
/// big enough for the engine and `regularize` to draw well, and the fits
/// cost time in proportion.
const MAX_EVIDENCE: usize = 20_000;
/// The grid the outlines are indexed in, in source pixels.
const GRID_CELL: f64 = 4.;
/// Levenberg-Marquardt iterations per fit.
const ITERATIONS: usize = 25;
/// A kind whose algebraic starting fit leaves the edge points further than
/// this many times its `MAX_EDGE_RMS_*` from it is not tried further.
const START_RMS_FACTOR: f64 = 4.;
/// At most this many edge points take part in the first fit (evenly
/// strided), which only has to land near the answer.
const MAX_FIT_POINTS: usize = 250;
/// A shape whose first fit leaves the edge points further than this from
/// it, root mean square, is not the outline's shape: on a pixel-edged
/// picture the crossings lie up to half a pixel either side of the true
/// edge (0.29 px root mean square when spread evenly), on an anti-aliased
/// one far closer. Refused before the costlier fit to the pixels.
const MAX_EDGE_RMS_ALIASED: f64 = 0.45;
const MAX_EDGE_RMS_SMOOTH: f64 = 0.3;
/// The second fit, to the pixels themselves, on pixel-edged artwork: every
/// pixel centre must lie this far on its own side of the outline, and the
/// edge crossings keep the outline in the middle of the room the pixels
/// leave, weighted by `EDGE_WEIGHT`.
const MARGIN: f64 = 0.05;
const EDGE_WEIGHT: f64 = 0.3;
/// The pixels the second fit reads: those whose centre lies within this
/// many pixels of the first fit's outline.
const PIXEL_REACH: f64 = 2.;
/// A quarter circle's cubic handle, as a fraction of the radius.
const KAPPA: f64 = 0.552_284_749_830_793_6;

type Colour = [f64; 4];
const TRANSPARENT: Colour = [0.; 4];

/// The kinds of shape tried, from the fewest free parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Circle,
    AxisEllipse,
    Ellipse,
    AxisRectangle,
    AxisRoundedRectangle,
    Rectangle,
    RoundedRectangle,
}
impl Kind {
    fn parameters(self) -> usize {
        match self {
            Kind::Circle => 3,
            Kind::AxisEllipse | Kind::AxisRectangle => 4,
            Kind::Ellipse | Kind::AxisRoundedRectangle | Kind::Rectangle => 5,
            Kind::RoundedRectangle => 6,
        }
    }
    fn shape(self, t: &[f64]) -> Shape {
        let centre = Point { x: t[0], y: t[1] };
        match self {
            Kind::Circle => Shape::Oval {
                centre,
                axes: (t[2].abs(), t[2].abs()),
                angle: 0.,
            },
            Kind::AxisEllipse => Shape::Oval {
                centre,
                axes: (t[2].abs(), t[3].abs()),
                angle: 0.,
            },
            Kind::Ellipse => Shape::Oval {
                centre,
                axes: (t[2].abs(), t[3].abs()),
                angle: t[4],
            },
            Kind::AxisRectangle => Shape::rounded(centre, (t[2], t[3]), 0., 0.),
            Kind::AxisRoundedRectangle => Shape::rounded(centre, (t[2], t[3]), 0., t[4]),
            Kind::Rectangle => Shape::rounded(centre, (t[2], t[3]), t[4], 0.),
            Kind::RoundedRectangle => Shape::rounded(centre, (t[2], t[3]), t[4], t[5]),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    /// A circle when the axes are equal; the first axis along `angle`.
    Oval {
        centre: Point,
        axes: (f64, f64),
        angle: f64,
    },
    /// A rectangle with half-sides `half` (the first along `angle`) and
    /// corners rounded by `radius`, at most the shorter half-side.
    Rounded {
        centre: Point,
        half: (f64, f64),
        angle: f64,
        radius: f64,
    },
}
impl Shape {
    fn rounded(centre: Point, half: (f64, f64), angle: f64, radius: f64) -> Self {
        let half = (half.0.abs(), half.1.abs());
        Shape::Rounded {
            centre,
            half,
            angle,
            radius: radius.abs().min(half.0.min(half.1)),
        }
    }
    fn valid(&self) -> bool {
        let (a, b, centre) = match *self {
            Shape::Oval { centre, axes, .. } => (axes.0, axes.1, centre),
            Shape::Rounded { centre, half, .. } => (half.0, half.1, centre),
        };
        a >= MIN_HALF && b >= MIN_HALF && centre.x.is_finite() && centre.y.is_finite()
    }
    /// Distance from the outline, negative inside.
    fn signed_distance(&self, p: Point) -> f64 {
        match *self {
            Shape::Oval {
                centre,
                axes,
                angle,
            } => {
                if axes.0 == axes.1 {
                    return distance(p, centre) - axes.0;
                }
                // Sampson's first-order distance, the implicit function over
                // its gradient: exact to first order at the outline, which is
                // all the fits and the coverage need, and free of the eight
                // Newton steps of the true foot point (which made the ellipse
                // fits most of the pass's time).
                let (s, c) = angle.sin_cos();
                let d = sub(p, centre);
                let (u, v) = (d.x * c + d.y * s, d.y * c - d.x * s);
                let (a2, b2) = (axes.0 * axes.0, axes.1 * axes.1);
                let f = u * u / a2 + v * v / b2 - 1.;
                let gradient = 2. * ((u / a2).powi(2) + (v / b2).powi(2)).sqrt();
                if gradient > 1e-12 {
                    f / gradient
                } else {
                    -axes.0.min(axes.1)
                }
            }
            Shape::Rounded {
                centre,
                half,
                angle,
                radius,
            } => {
                let (s, c) = angle.sin_cos();
                let d = sub(p, centre);
                let (u, v) = ((d.x * c + d.y * s).abs(), (d.y * c - d.x * s).abs());
                let (qx, qy) = (u - (half.0 - radius), v - (half.1 - radius));
                let outside = qx.max(0.).hypot(qy.max(0.));
                outside + qx.max(qy).min(0.) - radius
            }
        }
    }
    /// The outline as pieces, turning the way the angle grows; the last
    /// piece ends on the first one's start to the bit.
    fn pieces(&self) -> Vec<Edge> {
        match *self {
            Shape::Oval {
                centre,
                axes,
                angle,
            } => {
                let (s, c) = angle.sin_cos();
                let u = Point {
                    x: axes.0 * c,
                    y: axes.0 * s,
                };
                let v = Point {
                    x: -axes.1 * s,
                    y: axes.1 * c,
                };
                let at = |t: f64| add(centre, add(scale(u, t.cos()), scale(v, t.sin())));
                let along = |t: f64| add(scale(u, -t.sin()), scale(v, t.cos()));
                let nodes: Vec<Point> = (0..4).map(|k| at(k as f64 * FRAC_PI_2)).collect();
                (0..4)
                    .map(|k| {
                        let (t0, t1) = (k as f64 * FRAC_PI_2, (k + 1) as f64 * FRAC_PI_2);
                        let (p0, p3) = (nodes[k], nodes[(k + 1) % 4]);
                        curve(
                            p0,
                            add(p0, scale(along(t0), KAPPA)),
                            sub(p3, scale(along(t1), KAPPA)),
                            p3,
                        )
                    })
                    .collect()
            }
            Shape::Rounded {
                centre,
                half,
                angle,
                radius,
            } => {
                let (s, c) = angle.sin_cos();
                let (eu, ev) = (Point { x: c, y: s }, Point { x: -s, y: c });
                let world = |u: f64, v: f64| add(centre, add(scale(eu, u), scale(ev, v)));
                let (a, b, r) = (half.0, half.1, radius);
                if r < 1e-3 {
                    let corners = [world(a, -b), world(a, b), world(-a, b), world(-a, -b)];
                    return (0..4)
                        .map(|k| Edge::line(corners[k], corners[(k + 1) % 4], false))
                        .collect();
                }
                // Each side's two ends, walked with the angle: the side at
                // +u, then +v, -u, -v; each corner's arc joins one side's end
                // to the next side's start.
                let (ia, ib) = (a - r, b - r);
                let sides = [
                    (world(a, -ib), world(a, ib)),
                    (world(ia, b), world(-ia, b)),
                    (world(-a, ib), world(-a, -ib)),
                    (world(-ia, -b), world(ia, -b)),
                ];
                // The outward direction of each side, which is the tangent
                // direction the angle turns towards at the next corner.
                let tangents = [ev, scale(eu, -1.), scale(ev, -1.), eu];
                let mut pieces = Vec::with_capacity(8);
                for k in 0..4 {
                    let (from, to) = sides[k];
                    if distance(from, to) > 1e-9 {
                        pieces.push(Edge::line(from, to, false));
                    }
                    let next = sides[(k + 1) % 4].0;
                    let handle = KAPPA * r;
                    pieces.push(curve(
                        to,
                        add(to, scale(tangents[k], handle)),
                        sub(next, scale(tangents[(k + 1) % 4], handle)),
                        next,
                    ));
                }
                // Where a side vanished its two ends are equal but computed
                // apart; join every piece to the next one's start exactly.
                let n = pieces.len();
                for k in 0..n {
                    let start = pieces[(k + 1) % n].cubic.points[0];
                    pieces[k].cubic.points[3] = start;
                    if pieces[k].line {
                        pieces[k] = Edge::line(pieces[k].start(), start, false);
                    }
                }
                pieces
            }
        }
    }
}

fn curve(p0: Point, p1: Point, p2: Point, p3: Point) -> Edge {
    Edge {
        cubic: Cubic {
            points: [p0, p1, p2, p3],
        },
        line: false,
        implicit: false,
    }
}

/// The points along a run, its start once; a closed run's last point (its
/// start again) is dropped.
fn polyline(edges: &[Edge], closed: bool) -> Vec<Point> {
    let mut points = Vec::new();
    for edge in edges {
        if points.is_empty() {
            points.push(edge.start());
        }
        if edge.line {
            points.push(edge.end());
        } else {
            for step in 1..=FLATTEN_STEPS {
                points.push(edge.cubic.evaluate(step as f64 / FLATTEN_STEPS as f64));
            }
        }
    }
    if closed && points.len() > 1 && distance(points[0], points[points.len() - 1]) < 1e-9 {
        points.pop();
    }
    points
}

/// Twice the signed area of a closed polygon.
fn signed_area(polygon: &[Point]) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum()
}

/// Four straight edges, each upright or level, between whole-pixel points.
fn grid_rectangle(edges: &[Edge]) -> bool {
    let whole = |v: f64| (v - v.round()).abs() < 1e-9;
    edges.len() == 4
        && edges.iter().all(|e| {
            let (a, b) = (e.start(), e.end());
            e.line && whole(a.x) && whole(a.y) && (a.x == b.x || a.y == b.y)
        })
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    let t = if len2 > 0. {
        (dot(sub(p, a), ab) / len2).clamp(0., 1.)
    } else {
        0.
    };
    distance(p, add(a, scale(ab, t)))
}

/// Even-odd point in polygon.
fn inside_polygon(polygon: &[Point], p: Point) -> bool {
    let n = polygon.len();
    let mut hit = false;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            hit = !hit;
        }
    }
    hit
}

/// The nonzero winding number of a path's outlines about `p`.
fn winding(polygons: &[Vec<Point>], p: Point) -> i32 {
    let mut w = 0;
    for polygon in polygons {
        let n = polygon.len();
        for i in 0..n {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            let cross = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
            if a.y <= p.y {
                if b.y > p.y && cross > 0. {
                    w += 1;
                }
            } else if b.y <= p.y && cross < 0. {
                w -= 1;
            }
        }
    }
    w
}

/// A point just inside a closed polygon, beside the middle of its first
/// edge that has a length.
fn inner_probe(polygon: &[Point]) -> Option<Point> {
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        let len = distance(a, b);
        if len < 1e-6 {
            continue;
        }
        let mid = scale(add(a, b), 0.5);
        let normal = Point {
            x: -(b.y - a.y) / len,
            y: (b.x - a.x) / len,
        };
        for step in [0.05, 0.2, 0.5] {
            for sign in [1., -1.] {
                let probe = add(mid, scale(normal, step * sign));
                if inside_polygon(polygon, probe) {
                    return Some(probe);
                }
            }
        }
        return None;
    }
    None
}

/// Every run's segments in a uniform grid, for the nearest outline to a
/// pixel.
struct Grid {
    cell: f64,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
    segments: Vec<(Point, Point, usize)>,
}
impl Grid {
    fn new(width: f64, height: f64) -> Self {
        let columns = (width / GRID_CELL).ceil().max(1.) as usize;
        let rows = (height / GRID_CELL).ceil().max(1.) as usize;
        Self {
            cell: GRID_CELL,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
            segments: Vec::new(),
        }
    }
    fn span(&self, lo: f64, hi: f64, count: usize) -> std::ops::RangeInclusive<usize> {
        let clamp = |v: f64| ((v / self.cell).floor().max(0.) as usize).min(count - 1);
        clamp(lo)..=clamp(hi)
    }
    fn insert(&mut self, a: Point, b: Point, owner: usize) {
        let index = self.segments.len();
        self.segments.push((a, b, owner));
        for row in self.span(a.y.min(b.y), a.y.max(b.y), self.rows) {
            for column in self.span(a.x.min(b.x), a.x.max(b.x), self.columns) {
                self.cells[row * self.columns + column].push(index);
            }
        }
    }
    /// The distance from `p` to the nearest segment of `owner` and to the
    /// nearest of any other run, each looked for within `reach`.
    fn nearest(&self, p: Point, reach: f64, owner: usize) -> (f64, f64) {
        let (mut own, mut other) = (f64::INFINITY, f64::INFINITY);
        for row in self.span(p.y - reach, p.y + reach, self.rows) {
            for column in self.span(p.x - reach, p.x + reach, self.columns) {
                for &index in &self.cells[row * self.columns + column] {
                    let (a, b, who) = self.segments[index];
                    let d = segment_distance(p, a, b);
                    if who == owner {
                        own = own.min(d);
                    } else {
                        other = other.min(d);
                    }
                }
            }
        }
        (own, other)
    }
}

/// The pixels that judge one outline: each with the share of it the inside
/// fill covers, and the distance of its centre from the traced outline.
struct Evidence {
    x0: usize,
    y0: usize,
    width: usize,
    height: usize,
    /// Coverage per pixel of the box, NaN where the pixel is no evidence.
    alpha: Vec<f64>,
    pixels: Vec<(usize, usize, f64, f64)>,
}

fn premultiplied(pixel: [u8; 4]) -> Colour {
    let a = pixel[3] as f64;
    [
        pixel[0] as f64 * a / 255.,
        pixel[1] as f64 * a / 255.,
        pixel[2] as f64 * a / 255.,
        a,
    ]
}

fn gather(
    source: &Raster,
    grid: &Grid,
    outline: &[Point],
    owner: usize,
    inside_fill: Colour,
    outside_fill: Colour,
) -> Option<Evidence> {
    let (mut min, mut max) = (outline[0], outline[0]);
    for p in outline {
        min = Point {
            x: min.x.min(p.x),
            y: min.y.min(p.y),
        };
        max = Point {
            x: max.x.max(p.x),
            y: max.y.max(p.y),
        };
    }
    let x0 = (min.x - BAND).floor().max(0.) as usize;
    let y0 = (min.y - BAND).floor().max(0.) as usize;
    let x1 = ((max.x + BAND).ceil().max(0.) as usize).min(source.width);
    let y1 = ((max.y + BAND).ceil().max(0.) as usize).min(source.height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (width, height) = (x1 - x0, y1 - y0);
    // The pixels near this outline and nearer it than any other, each with
    // its colour, its distance from the outline and the side it lies on.
    let mut near: Vec<(usize, usize, Colour, f64, bool)> = Vec::new();
    for y in y0..y1 {
        let crossings = crossings_at(outline, y as f64 + 0.5);
        for x in x0..x1 {
            let centre = Point {
                x: x as f64 + 0.5,
                y: y as f64 + 0.5,
            };
            let (own, other) = grid.nearest(centre, BAND, owner);
            if !(own <= BAND) || other < own {
                continue;
            }
            let colour = premultiplied(source.pixels[y * source.width + x].0);
            let within = crossings.partition_point(|&c| c < centre.x) % 2 == 1;
            near.push((x, y, colour, own, within));
            if near.len() > MAX_EVIDENCE {
                return None;
            }
        }
    }
    // The two colours as the pixels show them away from the edge: the
    // engine's fills are region averages, which a shape's own anti-aliased
    // rim pulls towards its neighbour's colour, and a coverage read against
    // them comes out short at the rim.
    let side_colour = |inner: bool, fill: Colour| -> Colour {
        let mut channels: [Vec<f64>; 4] = Default::default();
        for &(_, _, colour, own, within) in &near {
            if within == inner && own >= DEEP {
                for k in 0..4 {
                    channels[k].push(colour[k]);
                }
            }
        }
        if channels[0].len() < MIN_DEEP {
            return fill;
        }
        channels.map(|mut values| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        })
    };
    let (inside, outside) = (
        side_colour(true, inside_fill),
        side_colour(false, outside_fill),
    );
    let axis: Colour = std::array::from_fn(|k| inside[k] - outside[k]);
    let axis2: f64 = axis.iter().map(|v| v * v).sum();
    if axis2.sqrt() < MIN_CONTRAST {
        return None;
    }
    let tolerance = (THIRD_COLOUR * axis2.sqrt()).max(MIN_RESIDUAL);
    let mut alpha = vec![f64::NAN; width * height];
    let mut pixels = Vec::new();
    for (x, y, colour, own, _) in near {
        let d: Colour = std::array::from_fn(|k| colour[k] - outside[k]);
        let t = d.iter().zip(&axis).map(|(a, b)| a * b).sum::<f64>() / axis2;
        let residual = (0..4)
            .map(|k| (d[k] - t * axis[k]).powi(2))
            .sum::<f64>()
            .sqrt();
        if residual > tolerance {
            continue;
        }
        let a = t.clamp(0., 1.);
        alpha[(y - y0) * width + (x - x0)] = a;
        pixels.push((x, y, a, own));
    }
    Some(Evidence {
        x0,
        y0,
        width,
        height,
        alpha,
        pixels,
    })
}

/// Where a closed polygon crosses the horizontal line at `y`, sorted.
fn crossings_at(polygon: &[Point], y: f64) -> Vec<f64> {
    let m = polygon.len();
    let mut crossings = Vec::new();
    for i in 0..m {
        let (a, b) = (polygon[i], polygon[(i + 1) % m]);
        if (a.y > y) != (b.y > y) {
            crossings.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
        }
    }
    crossings.sort_by(f64::total_cmp);
    crossings
}

/// Where the seen coverage crosses one half between two neighbouring
/// pixels, by linear interpolation of the two: on a pixel-edged picture the
/// middle of the pixel side between an inside and an outside pixel.
fn edge_points(evidence: &Evidence) -> Vec<Point> {
    let mut points = Vec::new();
    let (w, h) = (evidence.width, evidence.height);
    for ly in 0..h {
        for lx in 0..w {
            let a = evidence.alpha[ly * w + lx];
            if a.is_nan() {
                continue;
            }
            let centre = Point {
                x: (evidence.x0 + lx) as f64 + 0.5,
                y: (evidence.y0 + ly) as f64 + 0.5,
            };
            for (dx, dy) in [(1, 0), (0, 1)] {
                let (nx, ny) = (lx + dx, ly + dy);
                if nx >= w || ny >= h {
                    continue;
                }
                let b = evidence.alpha[ny * w + nx];
                if b.is_nan() || (a - 0.5) * (b - 0.5) >= 0. {
                    continue;
                }
                let t = (a - 0.5) / (a - b);
                points.push(Point {
                    x: centre.x + t * dx as f64,
                    y: centre.y + t * dy as f64,
                });
            }
        }
    }
    points
}

/// The summed difference between the coverage `inside` gives each evidence
/// pixel and the coverage seen. `distance_of` says how far a pixel's centre
/// lies from the outline judged, so a pixel wholly on one side is decided
/// by its centre alone.
fn coverage_error(
    evidence: &Evidence,
    anti_aliased: bool,
    inside: &dyn Fn(Point) -> bool,
    distance_of: &dyn Fn(Point, f64) -> f64,
) -> f64 {
    let n = if anti_aliased { SUBSAMPLES } else { 1 };
    let mut total = 0.;
    for &(x, y, alpha, own) in &evidence.pixels {
        let centre = Point {
            x: x as f64 + 0.5,
            y: y as f64 + 0.5,
        };
        let covered = if n == 1 || distance_of(centre, own) >= WHOLE_PIXEL {
            if inside(centre) {
                1.
            } else {
                0.
            }
        } else {
            let mut hits = 0;
            for j in 0..n {
                for i in 0..n {
                    let p = Point {
                        x: x as f64 + (i as f64 + 0.5) / n as f64,
                        y: y as f64 + (j as f64 + 0.5) / n as f64,
                    };
                    if inside(p) {
                        hits += 1;
                    }
                }
            }
            hits as f64 / (n * n) as f64
        };
        total += (covered - alpha).abs();
    }
    total
}

/// The crossings of a closed polygon with each horizontal line through the
/// sample rows of the evidence box, sorted, for a fast inside test.
struct Scan {
    y0: usize,
    n: usize,
    rows: Vec<Vec<f64>>,
}
impl Scan {
    fn new(polygon: &[Point], evidence: &Evidence, n: usize) -> Self {
        let count = evidence.height * n;
        let rows = (0..count)
            .map(|row| crossings_at(polygon, evidence.y0 as f64 + (row as f64 + 0.5) / n as f64))
            .collect();
        Self {
            y0: evidence.y0,
            n,
            rows,
        }
    }
    fn inside(&self, p: Point) -> bool {
        let row = ((p.y - self.y0 as f64) * self.n as f64).floor();
        if row < 0. || row as usize >= self.rows.len() {
            return false;
        }
        let crossings = &self.rows[row as usize];
        crossings.partition_point(|&x| x < p.x) % 2 == 1
    }
}

/// Solves the small dense system `a x = b` by elimination with partial
/// pivoting.
fn solve(mut a: Vec<f64>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let m = b.len();
    for col in 0..m {
        let pivot =
            (col..m).max_by(|&i, &j| a[i * m + col].abs().total_cmp(&a[j * m + col].abs()))?;
        if a[pivot * m + col].abs() < 1e-14 {
            return None;
        }
        if pivot != col {
            for k in 0..m {
                a.swap(pivot * m + k, col * m + k);
            }
            b.swap(pivot, col);
        }
        for row in col + 1..m {
            let f = a[row * m + col] / a[col * m + col];
            for k in col..m {
                a[row * m + k] -= f * a[col * m + k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.; m];
    for row in (0..m).rev() {
        let s: f64 = (row + 1..m).map(|k| a[row * m + k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row * m + row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Levenberg-Marquardt on `residuals` of the parameters, from `start`.
fn refine(start: &[f64], residuals: &dyn Fn(&[f64]) -> Vec<f64>) -> Vec<f64> {
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let m = start.len();
    let mut theta = start.to_vec();
    let mut r = residuals(&theta);
    let n = r.len();
    let mut current = cost(&r);
    let mut damping = 1e-3;
    for _ in 0..ITERATIONS {
        if current == 0. {
            break;
        }
        let mut jacobian = vec![0.; n * m];
        for j in 0..m {
            let h = 1e-6 * (1. + theta[j].abs());
            let (mut plus, mut minus) = (theta.clone(), theta.clone());
            plus[j] += h;
            minus[j] -= h;
            let (rp, rm) = (residuals(&plus), residuals(&minus));
            for i in 0..n {
                jacobian[i * m + j] = (rp[i] - rm[i]) / (2. * h);
            }
        }
        let mut jtj = vec![0.; m * m];
        let mut jtr = vec![0.; m];
        for i in 0..n {
            let row = &jacobian[i * m..(i + 1) * m];
            for a in 0..m {
                jtr[a] += row[a] * r[i];
                for b in 0..m {
                    jtj[a * m + b] += row[a] * row[b];
                }
            }
        }
        let mut improved = false;
        while damping < 1e8 {
            let mut system = jtj.clone();
            for j in 0..m {
                system[j * m + j] += damping * jtj[j * m + j].max(1e-9);
            }
            let Some(step) = solve(system, jtr.iter().map(|v| -v).collect()) else {
                damping *= 10.;
                continue;
            };
            let trial: Vec<f64> = theta.iter().zip(&step).map(|(a, b)| a + b).collect();
            let rt = residuals(&trial);
            let c = cost(&rt);
            if c < current {
                let gain = current - c;
                theta = trial;
                r = rt;
                current = c;
                damping = (damping * 0.3).max(1e-9);
                improved = gain > 1e-12 * (1. + current);
                break;
            }
            damping *= 10.;
        }
        if !improved {
            break;
        }
    }
    theta
}

/// The smallest-area rectangle holding `points`, from the directions of
/// their convex hull's sides: its centre, half-sides and angle.
fn min_area_rectangle(points: &[Point]) -> Option<(Point, (f64, f64), f64)> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    sorted.dedup_by(|a, b| distance(*a, *b) < 1e-12);
    if sorted.len() < 3 {
        return None;
    }
    let cross =
        |o: Point, a: Point, b: Point| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut hull: Vec<Point> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let walk: Box<dyn Iterator<Item = &Point>> = if pass == 0 {
            Box::new(sorted.iter())
        } else {
            Box::new(sorted.iter().rev())
        };
        for &p in walk {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    let mut best: Option<(f64, Point, (f64, f64), f64)> = None;
    for i in 0..hull.len() {
        let side = sub(hull[(i + 1) % hull.len()], hull[i]);
        if length(side) < 1e-9 {
            continue;
        }
        let angle = side.y.atan2(side.x);
        let (s, c) = angle.sin_cos();
        let (mut lo, mut hi) = (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        for p in &hull {
            let (u, v) = (p.x * c + p.y * s, p.y * c - p.x * s);
            lo = (lo.0.min(u), lo.1.min(v));
            hi = (hi.0.max(u), hi.1.max(v));
        }
        let area = (hi.0 - lo.0) * (hi.1 - lo.1);
        if best.as_ref().is_none_or(|b| area < b.0) {
            let (mu, mv) = ((lo.0 + hi.0) / 2., (lo.1 + hi.1) / 2.);
            let centre = Point {
                x: mu * c - mv * s,
                y: mu * s + mv * c,
            };
            best = Some((
                area,
                centre,
                ((hi.0 - lo.0) / 2., (hi.1 - lo.1) / 2.),
                angle,
            ));
        }
    }
    best.map(|(_, centre, half, angle)| (centre, half, angle))
}

/// The largest distance from a point of either closed polyline to the other.
fn hausdorff(a: &[Point], b: &[Point]) -> f64 {
    let directed = |from: &[Point], to: &[Point]| {
        let m = to.len();
        from.iter()
            .map(|p| {
                (0..m)
                    .map(|i| segment_distance(*p, to[i], to[(i + 1) % m]))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0., f64::max)
    };
    directed(a, b).max(directed(b, a))
}

/// The residuals of the second fit, of the shape `kind` makes of its
/// parameters against the pixels near the first fit's outline (`first`):
/// on anti-aliased artwork the coverage a pixel would get, as a ramp one
/// pixel wide across the outline, less the coverage seen; on pixel-edged
/// artwork how far each pixel centre lies on the wrong side (or within
/// `MARGIN` of the outline), with the edge crossings' distances at
/// `EDGE_WEIGHT` to keep the outline centred.
fn pixel_residuals<'a>(
    kind: Kind,
    first: &[f64],
    evidence: &'a Evidence,
    points: &'a [Point],
    anti_aliased: bool,
) -> impl Fn(&[f64]) -> Vec<f64> + 'a {
    let near = kind.shape(first);
    let pixels: Vec<(Point, f64)> = evidence
        .pixels
        .iter()
        .map(|&(x, y, alpha, _)| {
            (
                Point {
                    x: x as f64 + 0.5,
                    y: y as f64 + 0.5,
                },
                alpha,
            )
        })
        .filter(|(centre, _)| near.signed_distance(*centre).abs() <= PIXEL_REACH)
        .collect();
    move |t: &[f64]| {
        let shape = kind.shape(t);
        let mut residuals: Vec<f64> = pixels
            .iter()
            .map(|&(centre, alpha)| {
                let d = shape.signed_distance(centre);
                if anti_aliased {
                    (0.5 - d).clamp(0., 1.) - alpha
                } else if alpha >= 0.5 {
                    (d + MARGIN).max(0.)
                } else {
                    (MARGIN - d).max(0.)
                }
            })
            .collect();
        if !anti_aliased {
            residuals.extend(
                points
                    .iter()
                    .map(|p| EDGE_WEIGHT * shape.signed_distance(*p)),
            );
        }
        residuals
    }
}

/// The best shape for one outline, as pieces turning the same way as
/// `outline`, or `None` when the traced outline stays.
fn best_shape(
    evidence: &Evidence,
    outline: &[Point],
    anti_aliased: bool,
) -> Option<(Kind, Vec<Edge>)> {
    let points = edge_points(evidence);
    if points.len() < MIN_EDGE_POINTS {
        return None;
    }
    let n = if anti_aliased { SUBSAMPLES } else { 1 };
    let scan = Scan::new(outline, evidence, n);
    let traced = coverage_error(evidence, anti_aliased, &|p| scan.inside(p), &|_, own| own);
    let perimeter: f64 = (0..outline.len())
        .map(|i| distance(outline[i], outline[(i + 1) % outline.len()]))
        .sum();
    let slack = SLACK_PER_PX * perimeter;
    let mut starts: Vec<(Kind, Vec<f64>)> = Vec::new();
    if let Some(circle) = fit_circle(&points) {
        starts.push((
            Kind::Circle,
            vec![circle.centre.x, circle.centre.y, circle.radius],
        ));
    }
    let (mut lo, mut hi) = (points[0], points[0]);
    for p in &points {
        lo = Point {
            x: lo.x.min(p.x),
            y: lo.y.min(p.y),
        };
        hi = Point {
            x: hi.x.max(p.x),
            y: hi.y.max(p.y),
        };
    }
    let (cx, cy) = ((lo.x + hi.x) / 2., (lo.y + hi.y) / 2.);
    let (a, b) = ((hi.x - lo.x) / 2., (hi.y - lo.y) / 2.);
    starts.push((Kind::AxisEllipse, vec![cx, cy, a, b]));
    if let Some(e) = fit_ellipse(&points) {
        starts.push((
            Kind::Ellipse,
            vec![e.centre.x, e.centre.y, e.axes.0, e.axes.1, e.angle],
        ));
    }
    // A rounded rectangle's corner radius from the traced outline's area,
    // 4ab - (4 - pi) r^2 for half-sides a and b.
    let area = signed_area(outline).abs() / 2.;
    let corner = |a: f64, b: f64| {
        ((4. * a * b - area).max(0.) / (4. - std::f64::consts::PI))
            .sqrt()
            .min(a.min(b))
    };
    starts.push((Kind::AxisRectangle, vec![cx, cy, a, b]));
    starts.push((Kind::AxisRoundedRectangle, vec![cx, cy, a, b, corner(a, b)]));
    if let Some((centre, half, angle)) = min_area_rectangle(&points) {
        starts.push((
            Kind::Rectangle,
            vec![centre.x, centre.y, half.0, half.1, angle],
        ));
        starts.push((
            Kind::RoundedRectangle,
            vec![
                centre.x,
                centre.y,
                half.0,
                half.1,
                angle,
                corner(half.0, half.1),
            ],
        ));
    }
    let stride = points.len().div_ceil(MAX_FIT_POINTS);
    let fit_points: Vec<Point> = points.iter().step_by(stride).copied().collect();
    let max_rms = if anti_aliased {
        MAX_EDGE_RMS_SMOOTH
    } else {
        MAX_EDGE_RMS_ALIASED
    };
    let mut candidates: Vec<(f64, f64, Kind, Shape)> = Vec::new();
    for (kind, start) in starts {
        let distances = |t: &[f64]| -> Vec<f64> {
            let shape = kind.shape(t);
            fit_points
                .iter()
                .map(|p| shape.signed_distance(*p))
                .collect()
        };
        let initial = distances(&start);
        let initial_rms =
            (initial.iter().map(|d| d * d).sum::<f64>() / initial.len() as f64).sqrt();
        if !(initial_rms <= START_RMS_FACTOR * max_rms) {
            continue;
        }
        let first = refine(&start, &distances);
        let spread = distances(&first);
        let rms = (spread.iter().map(|d| d * d).sum::<f64>() / spread.len() as f64).sqrt();
        if !(rms <= max_rms) || !kind.shape(&first).valid() {
            continue;
        }
        let shape = kind.shape(&refine(
            &first,
            &pixel_residuals(kind, &first, evidence, &fit_points, anti_aliased),
        ));
        if !shape.valid() {
            continue;
        }
        let error = coverage_error(
            evidence,
            anti_aliased,
            &|p| shape.signed_distance(p) < 0.,
            &|p, _| shape.signed_distance(p).abs(),
        );
        if error <= traced + slack {
            candidates.push((
                error + PARAMETER_COST * kind.parameters() as f64,
                error,
                kind,
                shape,
            ));
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let turning = signed_area(outline).signum();
    for (_, error, kind, shape) in candidates {
        let mut pieces = shape.pieces();
        let drawn = polyline(&pieces, true);
        if turning != 0. && signed_area(&drawn).signum() != turning {
            pieces = pieces.iter().rev().map(Edge::reversed).collect();
        }
        let moved = hausdorff(&drawn, outline);
        if moved < MIN_MOVE && (anti_aliased || error > traced - 0.5) {
            return None;
        }
        if moved <= MAX_MOVE {
            return Some((kind, pieces));
        }
    }
    None
}

/// The fill of the path element holding the `d` attribute at `d_start`,
/// premultiplied by its opacity; `None` when it is not `#rrggbb`.
fn path_colour(svg: &str, d_start: usize) -> Option<Colour> {
    let element_start = svg[..d_start].rfind("<path")?;
    let head = &svg[element_start..d_start];
    let attribute = |name: &str| -> Option<&str> {
        let at = head.find(&format!(" {name}=\""))? + name.len() + 3;
        let value = &head[at..];
        Some(&value[..value.find('"')?])
    };
    let fill = attribute("fill")?;
    if fill.len() != 7 || !fill.starts_with('#') {
        return None;
    }
    let channel = |k: usize| u8::from_str_radix(&fill[1 + 2 * k..3 + 2 * k], 16).ok();
    let opacity = attribute("opacity").map_or(Some(1.), |v| v.parse::<f64>().ok())?;
    let a = 255. * opacity.clamp(0., 1.);
    Some([
        channel(0)? as f64 * a / 255.,
        channel(1)? as f64 * a / 255.,
        channel(2)? as f64 * a / 255.,
        a,
    ])
}

fn viewbox_size(svg: &str) -> Option<(f64, f64)> {
    let at = svg.find("viewBox=\"")? + 9;
    let rest = &svg[at..];
    let numbers: Vec<f64> = rest[..rest.find('"')?]
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    match numbers[..] {
        [0., 0., w, h] => Some((w, h)),
        _ => None,
    }
}

/// One run of boundary, once however many fills share it.
struct RunInfo {
    edges: Vec<Edge>,
    cyclic: bool,
    /// The paths walking it.
    paths: Vec<usize>,
}

/// The document with every closed outline the pixels show to be a circle,
/// an ellipse, a rectangle or a rounded rectangle drawn as that shape;
/// every path's data rewritten, everything else verbatim. `source` is the
/// picture the engine traced; a document whose view box is not its size is
/// returned unchanged. Nodes in `forced` keep their outlines.
pub fn refit_svg(
    svg: &str,
    source: &Raster,
    options: PrimitiveOptions,
    forced: &[Point],
) -> Result<(String, PrimitiveStats), String> {
    let mut stats = PrimitiveStats::default();
    let (width, height) = (source.width as f64, source.height as f64);
    if viewbox_size(svg) != Some((width, height))
        || source.pixels.len() != source.width * source.height
    {
        return Ok((svg.to_owned(), stats));
    }
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let colours: Vec<Option<Colour>> = ranges
        .iter()
        .map(|&(start, _)| path_colour(svg, start))
        .collect();
    let junctions = junction_keys(&paths);
    let is_junction = |p: Point| junctions.contains(&key(p));
    let forced_keys: HashSet<Key> = forced.iter().map(|p| key(*p)).collect();
    let mut ids: HashMap<Vec<EdgeKey>, usize> = HashMap::new();
    let mut runs: Vec<RunInfo> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        for subpath in path {
            for run in runs_of(subpath, &is_junction) {
                let (run_key, edges, _) = canonical(&run);
                let id = *ids.entry(run_key).or_insert_with(|| {
                    runs.push(RunInfo {
                        edges,
                        cyclic: run.cyclic,
                        paths: Vec::new(),
                    });
                    runs.len() - 1
                });
                runs[id].paths.push(index);
            }
        }
    }
    let outlines: Vec<Vec<Point>> = runs.iter().map(|r| polyline(&r.edges, r.cyclic)).collect();
    let mut grid = Grid::new(width, height);
    for (id, points) in outlines.iter().enumerate() {
        let m = points.len();
        let segments = if runs[id].cyclic {
            m
        } else {
            m.saturating_sub(1)
        };
        for i in 0..segments {
            grid.insert(points[i], points[(i + 1) % m], id);
        }
    }
    let path_polygons: Vec<Vec<Vec<Point>>> = paths
        .iter()
        .map(|path| path.iter().map(|s| polyline(&s.edges, true)).collect())
        .collect();
    let on_frame =
        |p: &Point| p.x <= 1e-6 || p.y <= 1e-6 || p.x >= width - 1e-6 || p.y >= height - 1e-6;
    let mut replacements: HashMap<usize, Vec<Edge>> = HashMap::new();
    for (id, run) in runs.iter().enumerate() {
        let outline = &outlines[id];
        if !run.cyclic || outline.len() < 3 || outline.iter().any(on_frame) {
            continue;
        }
        if run
            .edges
            .iter()
            .any(|e| forced_keys.contains(&key(e.start())))
        {
            continue;
        }
        // On pixel-edged artwork an upright rectangle on the pixel grid is
        // its pixels: judged at the pixel centres a circle through them
        // explains them as well with fewer parameters, and a 2 by 2 dot
        // came out round.
        if !options.anti_aliased && grid_rectangle(&run.edges) {
            continue;
        }
        stats.outlines += 1;
        let (mut inside, mut outside) = (None, None);
        let Some(probe) = inner_probe(outline) else {
            continue;
        };
        let mut consistent = run.paths.len() <= 2;
        for &path in &run.paths {
            let Some(colour) = colours[path] else {
                consistent = false;
                break;
            };
            let side = if winding(&path_polygons[path], probe) != 0 {
                &mut inside
            } else {
                &mut outside
            };
            if side.is_some() {
                consistent = false;
            }
            *side = Some(colour);
        }
        if !consistent {
            continue;
        }
        let Some(evidence) = gather(
            source,
            &grid,
            outline,
            id,
            inside.unwrap_or(TRANSPARENT),
            outside.unwrap_or(TRANSPARENT),
        ) else {
            continue;
        };
        stats.judged += 1;
        let Some((kind, pieces)) = best_shape(&evidence, outline, options.anti_aliased) else {
            continue;
        };
        // A node of the shape the desktop showed, forced from there: the
        // outline keeps its traced form, where the forcing applies.
        if pieces.iter().any(|e| {
            forced
                .iter()
                .any(|f| distance(*f, e.start()) <= FORCED_REACH)
        }) {
            continue;
        }
        match kind {
            Kind::Circle => stats.circles += 1,
            Kind::AxisEllipse | Kind::Ellipse => stats.ellipses += 1,
            Kind::AxisRectangle | Kind::Rectangle => stats.rectangles += 1,
            Kind::AxisRoundedRectangle | Kind::RoundedRectangle => stats.rounded_rectangles += 1,
        }
        replacements.insert(id, pieces);
    }
    if replacements.is_empty() {
        return Ok((svg.to_owned(), stats));
    }
    for path in &mut paths {
        for subpath in path.iter_mut() {
            let found = runs_of(subpath, &is_junction);
            let [run] = &found[..] else {
                continue;
            };
            if !run.cyclic {
                continue;
            }
            let (run_key, _, reversed) = canonical(run);
            if let Some(done) = ids.get(&run_key).and_then(|id| replacements.get(id)) {
                subpath.edges = if reversed {
                    done.iter().rev().map(Edge::reversed).collect()
                } else {
                    done.clone()
                };
            }
        }
    }
    Ok((splice(svg, &ranges, &paths), stats))
}

#[cfg(test)]
mod tests;
