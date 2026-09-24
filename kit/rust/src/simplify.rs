//! Owned post-fit curve simplification. Neighbouring curve pieces of a vector
//! document are merged wherever one cubic stays within a tolerance of the
//! original fit, so a circle traced as many short arcs becomes a few, and two
//! nodes sitting next to each other on one smooth bend become one.
//!
//! Boundaries shared by two fills are recognised by their identical reversed
//! coordinates, simplified once and written back to both sides, so adjacent
//! regions keep meeting exactly and no seams open. Nodes where three or more
//! boundaries meet are never removed.
//!
//! This is new behaviour layered on top of the recovered pipeline's output. It
//! is not recovered original code and it only runs when asked for; the
//! faithful comparisons never pass through it.
use crate::geometry::{Cubic, Point};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimplifyOptions {
    /// Largest allowed distance, in source pixels, between the simplified
    /// curve and the curve it replaces.
    pub tolerance: f64,
    /// Also smooth the kinks the merges keep (`smooth_kinks`): for a
    /// tolerance chosen by hand, never for Auto's, which the quality rounds
    /// of September 24, 2026 hold to the faithful drawing.
    pub smooth_kinks: bool,
}
impl Default for SimplifyOptions {
    fn default() -> Self {
        Self {
            tolerance: 0.5,
            smooth_kinks: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimplifyStats {
    pub paths: usize,
    pub subpaths: usize,
    pub segments_before: usize,
    pub segments_after: usize,
    pub runs: usize,
    pub shared_runs: usize,
    pub junctions: usize,
}

/// Points sampled per original segment when measuring a replacement.
pub(crate) const SAMPLES_PER_SEGMENT: usize = 12;
/// A closed outline keeps at least this many pieces.
const MIN_CLOSED_SEGMENTS: usize = 3;
const FIT_ITERATIONS: usize = 6;
/// Rounds of the handle-length search after the least-squares fit.
const HANDLE_SEARCH_ROUNDS: usize = 12;
/// No merge may bend back against the way the curve was turning by this many
/// degrees or more where the pieces it replaces did not: the inflection of
/// kit/tools/outline_metrics.py (INFLECTION_DEGREES). Without the rule the
/// merges added inflections on photographs and rounded shapes (September 22,
/// 2026, testing/quality-round/merge-rule.md).
const TURN_BACK_DEGREES: f64 = 2.;
/// Tangent samples per cubic when following its turning (outline_metrics.py's
/// STEPS).
const TURN_STEPS: usize = 16;

/// A node's identity: its coordinates' bits. `simplify`, `regularize` and
/// `straighten` all match nodes by this one key.
pub(crate) type Key = [u64; 2];

pub(crate) fn key(p: Point) -> Key {
    // Adding zero folds -0.0 into 0.0 so both copies of a shared edge agree.
    [(p.x + 0.).to_bits(), (p.y + 0.).to_bits()]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct EdgeKey {
    ends: [Key; 2],
    controls: [Key; 2],
    line: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Edge {
    pub(crate) cubic: Cubic,
    pub(crate) line: bool,
    /// A closing line the source left to `Z`; written back the same way when
    /// it survives untouched at the end of its outline.
    pub(crate) implicit: bool,
}
impl Edge {
    pub(crate) fn line(from: Point, to: Point, implicit: bool) -> Self {
        Self {
            cubic: Cubic {
                points: [from, lerp(from, to, 1. / 3.), lerp(from, to, 2. / 3.), to],
            },
            line: true,
            implicit,
        }
    }
    pub(crate) fn start(&self) -> Point {
        self.cubic.points[0]
    }
    pub(crate) fn end(&self) -> Point {
        self.cubic.points[3]
    }
    /// Orientation-independent identity and whether this occurrence runs
    /// against the canonical direction.
    pub(crate) fn key(&self) -> (EdgeKey, bool) {
        let [p0, p1, p2, p3] = self.cubic.points;
        let (a, b) = (key(p0), key(p3));
        let controls = |first: Point, second: Point| {
            if self.line {
                [[0; 2]; 2]
            } else {
                [key(first), key(second)]
            }
        };
        if a <= b {
            (
                EdgeKey {
                    ends: [a, b],
                    controls: controls(p1, p2),
                    line: self.line,
                },
                false,
            )
        } else {
            (
                EdgeKey {
                    ends: [b, a],
                    controls: controls(p2, p1),
                    line: self.line,
                },
                true,
            )
        }
    }
    pub(crate) fn reversed(&self) -> Self {
        let [a, b, c, d] = self.cubic.points;
        Self {
            cubic: Cubic {
                points: [d, c, b, a],
            },
            line: self.line,
            implicit: self.implicit,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Subpath {
    pub(crate) edges: Vec<Edge>,
    /// Ended with `Z`. The engine writes several outlines per path and closes
    /// only the last one with `Z`, so this is a serialisation detail.
    pub(crate) closed: bool,
}
impl Subpath {
    /// The outline returns to its start, so its first node is nothing special
    /// and merges may wrap around it. A fill treats such an outline as closed
    /// whether or not `Z` was written.
    pub(crate) fn cyclic(&self) -> bool {
        match (self.edges.first(), self.edges.last()) {
            (Some(first), Some(last)) => key(first.start()) == key(last.end()),
            _ => false,
        }
    }
}

pub(crate) fn parse_path_data(d: &str) -> Result<Vec<Subpath>, String> {
    let tokens: Vec<&str> = d.split_whitespace().collect();
    let number = |token: Option<&&str>| -> Result<f64, String> {
        let token = token.ok_or("Path data ends inside a command")?;
        let value: f64 = token
            .parse()
            .map_err(|_| format!("Bad path coordinate {token:?}"))?;
        if !value.is_finite() {
            return Err(format!("Non-finite path coordinate {token:?}"));
        }
        Ok(value)
    };
    let mut subpaths = Vec::new();
    let mut current: Option<(Point, Point, Vec<Edge>)> = None;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "M" => {
                if let Some((_, _, edges)) = current.take() {
                    if !edges.is_empty() {
                        subpaths.push(Subpath {
                            edges,
                            closed: false,
                        });
                    }
                }
                let p = Point {
                    x: number(tokens.get(i + 1))?,
                    y: number(tokens.get(i + 2))?,
                };
                current = Some((p, p, Vec::new()));
                i += 3;
            }
            "L" => {
                let (_, cursor, edges) = current
                    .as_mut()
                    .ok_or("Path data draws before its first M")?;
                let p = Point {
                    x: number(tokens.get(i + 1))?,
                    y: number(tokens.get(i + 2))?,
                };
                edges.push(Edge::line(*cursor, p, false));
                *cursor = p;
                i += 3;
            }
            "C" => {
                let (_, cursor, edges) = current
                    .as_mut()
                    .ok_or("Path data draws before its first M")?;
                let mut values = [0.; 6];
                for (k, value) in values.iter_mut().enumerate() {
                    *value = number(tokens.get(i + 1 + k))?;
                }
                let points = [
                    *cursor,
                    Point {
                        x: values[0],
                        y: values[1],
                    },
                    Point {
                        x: values[2],
                        y: values[3],
                    },
                    Point {
                        x: values[4],
                        y: values[5],
                    },
                ];
                edges.push(Edge {
                    cubic: Cubic { points },
                    line: false,
                    implicit: false,
                });
                *cursor = points[3];
                i += 7;
            }
            "Z" | "z" => {
                let (start, cursor, mut edges) = current
                    .take()
                    .ok_or("Path data closes before its first M")?;
                if key(cursor) != key(start) {
                    edges.push(Edge::line(cursor, start, true));
                }
                if !edges.is_empty() {
                    subpaths.push(Subpath {
                        edges,
                        closed: true,
                    });
                }
                i += 1;
            }
            other => return Err(format!("Unsupported path command {other:?}")),
        }
    }
    if let Some((_, _, edges)) = current {
        if !edges.is_empty() {
            subpaths.push(Subpath {
                edges,
                closed: false,
            });
        }
    }
    Ok(subpaths)
}

pub(crate) fn write_path_data(subpaths: &[Subpath]) -> String {
    let mut d = String::new();
    for subpath in subpaths {
        let Some(first) = subpath.edges.first() else {
            continue;
        };
        let start = first.start();
        d.push_str(&format!(" M {:.2} {:.2}", start.x, start.y));
        let count = subpath.edges.len();
        for (index, edge) in subpath.edges.iter().enumerate() {
            let closing = subpath.closed && index + 1 == count;
            if closing && edge.implicit && edge.line && key(edge.end()) == key(start) {
                continue;
            }
            let [_, p1, p2, p3] = edge.cubic.points;
            if edge.line {
                d.push_str(&format!(" L {:.2} {:.2}", p3.x, p3.y));
            } else {
                d.push_str(&format!(
                    " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                    p1.x, p1.y, p2.x, p2.y, p3.x, p3.y
                ));
            }
        }
        if subpath.closed {
            d.push_str(" Z");
        }
    }
    d
}

// Small vector helpers; `Point` deliberately carries no operators.
pub(crate) fn add(a: Point, b: Point) -> Point {
    Point {
        x: a.x + b.x,
        y: a.y + b.y,
    }
}
pub(crate) fn sub(a: Point, b: Point) -> Point {
    Point {
        x: a.x - b.x,
        y: a.y - b.y,
    }
}
pub(crate) fn scale(a: Point, s: f64) -> Point {
    Point {
        x: a.x * s,
        y: a.y * s,
    }
}
pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a.x * b.x + a.y * b.y
}
pub(crate) fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}
pub(crate) fn distance(a: Point, b: Point) -> f64 {
    length(sub(a, b))
}
pub(crate) fn lerp(a: Point, b: Point, t: f64) -> Point {
    add(a, scale(sub(b, a), t))
}
pub(crate) fn normalized(a: Point) -> Option<Point> {
    let len = length(a);
    (len > 1e-12).then(|| scale(a, 1. / len))
}

/// Direction of travel leaving a piece's start: towards the first control
/// point that is not on the start; `None` for a piece that never leaves it.
pub(crate) fn leaving(edge: &Edge) -> Option<Point> {
    let [p0, p1, p2, p3] = edge.cubic.points;
    [p1, p2, p3]
        .into_iter()
        .find_map(|p| normalized(sub(p, p0)))
}

/// Direction from a piece's end back into it: towards the last control
/// point that is not on the end; `None` for a piece that never leaves it.
pub(crate) fn backwards(edge: &Edge) -> Option<Point> {
    let [p0, p1, p2, p3] = edge.cubic.points;
    [p2, p1, p0]
        .into_iter()
        .find_map(|p| normalized(sub(p, p3)))
}

/// Direction of travel arriving at a piece's end: `backwards` negated, the
/// same value as normalising p3 - p (only a zero's sign can differ).
pub(crate) fn arriving(edge: &Edge) -> Option<Point> {
    backwards(edge).map(|d| scale(d, -1.))
}

#[derive(Clone, Debug)]
struct Piece {
    edge: Edge,
    /// Points of the original curve this piece replaces, endpoints included.
    samples: Vec<Point>,
}
impl Piece {
    fn new(edge: Edge) -> Self {
        let samples = (0..=SAMPLES_PER_SEGMENT)
            .map(|k| edge.cubic.evaluate(k as f64 / SAMPLES_PER_SEGMENT as f64))
            .collect();
        Self { edge, samples }
    }
    fn start_direction(&self) -> Point {
        leaving(&self.edge).unwrap_or(Point { x: 1., y: 0. })
    }
    /// Points backwards into the curve from its end.
    fn end_direction(&self) -> Point {
        backwards(&self.edge).unwrap_or(Point { x: -1., y: 0. })
    }
}

fn derivative(cubic: &Cubic, t: f64) -> Point {
    let [p0, p1, p2, p3] = cubic.points;
    let u = 1. - t;
    let a = scale(sub(p1, p0), 3. * u * u);
    let b = scale(sub(p2, p1), 6. * u * t);
    let c = scale(sub(p3, p2), 3. * t * t);
    add(add(a, b), c)
}
fn second_derivative(cubic: &Cubic, t: f64) -> Point {
    let [p0, p1, p2, p3] = cubic.points;
    let a = add(sub(p2, scale(p1, 2.)), p0);
    let b = add(sub(p3, scale(p2, 2.)), p1);
    scale(add(scale(a, 1. - t), scale(b, t)), 6.)
}

/// Move each interior parameter one Newton step toward its sample's closest
/// point, then return the largest sample distance. Endpoints stay pinned. The
/// result is an honest upper bound of the true geometric error.
fn measure(samples: &[Point], u: &mut [f64], cubic: &Cubic, newton_steps: usize) -> f64 {
    let n = samples.len();
    for _ in 0..newton_steps {
        for i in 1..n - 1 {
            let p = samples[i];
            let q = cubic.evaluate(u[i]);
            let q1 = derivative(cubic, u[i]);
            let q2 = second_derivative(cubic, u[i]);
            let offset = sub(q, p);
            let numerator = dot(offset, q1);
            let denominator = dot(q1, q1) + dot(offset, q2);
            if denominator.abs() > 1e-12 {
                let next = u[i] - numerator / denominator;
                if next.is_finite() {
                    u[i] = next.clamp(0., 1.);
                }
            }
        }
    }
    samples
        .iter()
        .zip(u.iter())
        .map(|(&p, &t)| distance(p, cubic.evaluate(t)))
        .fold(0., f64::max)
}

/// The largest distance from `samples` to `cubic`, measured as `fit_samples`
/// measures its trials, so the two can be compared.
pub(crate) fn largest_distance(samples: &[Point], cubic: &Cubic) -> Option<f64> {
    let mut u = chord_parameters(samples)?;
    Some(measure(samples, &mut u, cubic, 4))
}

fn chord_parameters(samples: &[Point]) -> Option<Vec<f64>> {
    let n = samples.len();
    let mut u = vec![0.; n];
    for i in 1..n {
        u[i] = u[i - 1] + distance(samples[i - 1], samples[i]);
    }
    let total = u[n - 1];
    if !(total > 0.) {
        return None;
    }
    for value in &mut u {
        *value /= total;
    }
    Some(u)
}

/// Least-squares cubic through the first and last sample with the given end
/// tangent directions (Schneider's method), refined by Newton
/// reparameterisation and then by a short search over the two handle lengths,
/// which finds the short handle least squares misses when its chord-length
/// parameters couple the two coordinates. Returns the best cubic and its
/// largest sample distance.
pub(crate) fn fit_samples(samples: &[Point], t1: Point, t2: Point) -> Option<(Cubic, f64)> {
    let n = samples.len();
    if n < 3 {
        return None;
    }
    let mut u = chord_parameters(samples)?;
    let first = samples[0];
    let last = samples[n - 1];
    let chord = distance(first, last);
    let build = |alpha1: f64, alpha2: f64| Cubic {
        points: [
            first,
            add(first, scale(t1, alpha1)),
            add(last, scale(t2, alpha2)),
            last,
        ],
    };
    let mut best: Option<(Cubic, f64, f64, f64)> = None;
    for _ in 0..FIT_ITERATIONS {
        let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0., 0., 0., 0., 0.);
        for (i, &p) in samples.iter().enumerate() {
            let t = u[i];
            let s = 1. - t;
            let b0 = s * s * s;
            let b1 = 3. * s * s * t;
            let b2 = 3. * s * t * t;
            let b3 = t * t * t;
            let a1 = scale(t1, b1);
            let a2 = scale(t2, b2);
            c00 += dot(a1, a1);
            c01 += dot(a1, a2);
            c11 += dot(a2, a2);
            let base = add(scale(first, b0 + b1), scale(last, b2 + b3));
            let residual = sub(p, base);
            x0 += dot(a1, residual);
            x1 += dot(a2, residual);
        }
        let det = c00 * c11 - c01 * c01;
        let (mut alpha1, mut alpha2) = if det.abs() > 1e-12 {
            ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det)
        } else {
            (0., 0.)
        };
        // A handle the solve wants collapsed or flipped is pinned short and the
        // other handle re-solved alone; only a degenerate system falls back to
        // the equal-thirds guess. Resetting both, as Schneider's original does,
        // bulges the curve near an almost-straight join.
        let epsilon = 1e-3 * chord;
        let usable1 = alpha1.is_finite() && alpha1 > epsilon;
        let usable2 = alpha2.is_finite() && alpha2 > epsilon;
        match (usable1, usable2) {
            (true, true) => {}
            (false, true) if c11.abs() > 1e-12 => {
                alpha1 = epsilon;
                alpha2 = ((x1 - c01 * alpha1) / c11).max(epsilon);
            }
            (true, false) if c00.abs() > 1e-12 => {
                alpha2 = epsilon;
                alpha1 = ((x0 - c01 * alpha2) / c00).max(epsilon);
            }
            _ => {
                alpha1 = chord / 3.;
                alpha2 = chord / 3.;
            }
        }
        let cubic = build(alpha1, alpha2);
        let error = measure(samples, &mut u, &cubic, 1);
        if !error.is_finite() {
            break;
        }
        if best.as_ref().is_none_or(|(_, e, _, _)| error < *e) {
            best = Some((cubic, error, alpha1, alpha2));
        }
        if error < 1e-9 {
            break;
        }
    }
    let (mut cubic, mut error, mut alpha1, mut alpha2) = best?;
    // Handle-length search: shorten or lengthen either handle while the true
    // error keeps dropping. Each trial starts from chord parameters so a bad
    // earlier parameterisation cannot hide a better curve.
    let mut rounds = 0;
    while error > 1e-9 && rounds < HANDLE_SEARCH_ROUNDS {
        rounds += 1;
        let mut improved = false;
        for (f1, f2) in [
            (0.5, 1.),
            (1., 0.5),
            (0.25, 1.),
            (1., 0.25),
            (1.5, 1.),
            (1., 1.5),
            (0.5, 0.5),
        ] {
            let (a1, a2) = (alpha1 * f1, alpha2 * f2);
            if a1 < 1e-3 * chord || a2 < 1e-3 * chord {
                continue;
            }
            let trial = build(a1, a2);
            let mut params = chord_parameters(samples)?;
            let trial_error = measure(samples, &mut params, &trial, 4);
            if trial_error.is_finite() && trial_error < error - 1e-9 {
                cubic = trial;
                error = trial_error;
                alpha1 = a1;
                alpha2 = a2;
                improved = true;
                break;
            }
        }
        if !improved {
            break;
        }
    }
    Some((cubic, error))
}

/// Tangent directions at TURN_STEPS + 1 even parameters. A handle lying on
/// its node leaves the end tangent to the next control point.
fn tangents(cubic: &Cubic) -> [Point; TURN_STEPS + 1] {
    let [p0, p1, p2, p3] = cubic.points;
    let mut directions = [Point { x: 0., y: 0. }; TURN_STEPS + 1];
    for (k, direction) in directions.iter_mut().enumerate() {
        *direction = derivative(cubic, k as f64 / TURN_STEPS as f64);
    }
    if length(directions[0]) < 1e-12 {
        directions[0] = sub(p2, p0);
    }
    if length(directions[TURN_STEPS]) < 1e-12 {
        directions[TURN_STEPS] = sub(p3, p1);
    }
    directions
}

/// Signed turn from direction a to direction b, in degrees.
fn signed_turn(a: Point, b: Point) -> f64 {
    (a.x * b.y - a.y * b.x).atan2(dot(a, b)).to_degrees()
}

/// Reversals of a tangent's direction of turning by TURN_BACK_DEGREES or
/// more, with hysteresis, so rounding noise on a straight piece and a lobe
/// smaller than that do not count (outline_metrics.py's ZigZag).
#[derive(Default)]
struct TurnBacks {
    phi: f64,
    high: f64,
    low: f64,
    extreme: f64,
    direction: i8,
    count: usize,
}
impl TurnBacks {
    fn feed(&mut self, delta: f64) {
        self.phi += delta;
        let phi = self.phi;
        match self.direction {
            0 => {
                self.high = self.high.max(phi);
                self.low = self.low.min(phi);
                if self.high - self.low >= TURN_BACK_DEGREES {
                    self.direction = if phi >= self.high { 1 } else { -1 };
                    self.extreme = phi;
                }
            }
            1 if phi > self.extreme => self.extreme = phi,
            1 if self.extreme - phi >= TURN_BACK_DEGREES => {
                self.count += 1;
                self.direction = -1;
                self.extreme = phi;
            }
            -1 if phi < self.extreme => self.extreme = phi,
            -1 if phi - self.extreme >= TURN_BACK_DEGREES => {
                self.count += 1;
                self.direction = 1;
                self.extreme = phi;
            }
            _ => {}
        }
    }
    fn follow(&mut self, directions: &[Point]) {
        for pair in directions.windows(2) {
            self.feed(signed_turn(pair[0], pair[1]));
        }
    }
}

/// Whether `merged` turns back more often than `a`, the turn at their node
/// and `b` did, walked in that order.
fn adds_turn_back(a: &Cubic, b: &Cubic, merged: &Cubic) -> bool {
    let (along_a, along_b) = (tangents(a), tangents(b));
    let mut pieces = TurnBacks::default();
    pieces.follow(&along_a);
    pieces.feed(signed_turn(along_a[TURN_STEPS], along_b[0]));
    pieces.follow(&along_b);
    let mut replacement = TurnBacks::default();
    replacement.follow(&tangents(merged));
    replacement.count > pieces.count
}

/// Pieces meeting at a turn of this many degrees or more are a corner: one
/// cubic through both cuts it however close it stays (on 8x pixel art the
/// Auto tolerance of 0.5 px took 0.5 px off every square corner the engine
/// had drawn exactly; the defect sweep of September 23, 2026).
const CORNER_TURN: f64 = 45.;

/// How far a merge may carry pieces that together follow one circle off
/// that circle: the desktop's regularize tolerance, so a run it would draw
/// as true arcs stays a run it can see. Without it the Simplify slider's far
/// end (3 px) merged the GitHub mark's ring into two cubics of 150 and 172
/// degrees, 4.6 px off round, which no later pass could draw round again
/// (the owner, September 23, 2026: "it was a circle before").
const ON_CIRCLE: f64 = 0.8;
/// Pieces follow a circle only when they sweep a quarter of it or more
/// around a radius of at least `ON_CIRCLE_MIN_RADIUS` (regularize's floor).
/// Measured at 3 px over the samples the rule changes (work/octocat/exp.sh):
/// a 30 degree sweep held letter curves that only look circular over a
/// short stretch (total +0.0351 against +0.0244 at 90), and a radius floor
/// of 10 or 20 px lost the small letters' gain and was less faithful.
const ON_CIRCLE_MIN_SWEEP: f64 = 90.;
const ON_CIRCLE_MIN_RADIUS: f64 = 2.;
/// Points of the merged cubic held against the circle.
const ON_CIRCLE_STEPS: usize = 24;
/// A merge that stays this close to its pieces is not checked: it cannot
/// carry them visibly off a circle, and the Auto tolerances (0.1 px on
/// photographs) keep every result they gave before the rule (the quality
/// round of September 23, 2026: one line in 56,110 pieces on astronaut-x3
/// otherwise).
const ON_CIRCLE_SLACK: f64 = ON_CIRCLE / 4.;

/// Whether `samples` lie within `ON_CIRCLE` of one circle over a real sweep
/// of it while `merged`, drawn in their place, leaves that band.
fn leaves_its_circle(samples: &[Point], merged: &Cubic) -> bool {
    let Some(circle) = crate::regularize::fit_circle(samples) else {
        return false;
    };
    if circle.radius < ON_CIRCLE_MIN_RADIUS {
        return false;
    }
    let off = |p: Point| (length(sub(p, circle.centre)) - circle.radius).abs();
    if samples.iter().any(|&p| off(p) > ON_CIRCLE) {
        return false;
    }
    let sweep: f64 = samples
        .windows(2)
        .map(|w| signed_turn(sub(w[0], circle.centre), sub(w[1], circle.centre)))
        .sum();
    if sweep.abs() < ON_CIRCLE_MIN_SWEEP {
        return false;
    }
    (0..=ON_CIRCLE_STEPS)
        .any(|k| off(merged.evaluate(k as f64 / ON_CIRCLE_STEPS as f64)) > ON_CIRCLE)
}

/// The cubic through both pieces and its largest distance from them; a
/// cubic that adds a turn-back, that would join two pieces across a corner,
/// or that takes pieces following one circle off it, costs infinity,
/// however close it stays.
fn try_merge(a: &Piece, b: &Piece) -> (f64, Cubic) {
    if let (Some(t), Some(u)) = (arriving(&a.edge), leaving(&b.edge)) {
        if dot(t, u) <= CORNER_TURN.to_radians().cos() {
            return (f64::INFINITY, a.edge.cubic);
        }
    }
    let mut samples = a.samples.clone();
    samples.extend_from_slice(&b.samples[1..]);
    match fit_samples(&samples, a.start_direction(), b.end_direction()) {
        Some((cubic, _)) if adds_turn_back(&a.edge.cubic, &b.edge.cubic, &cubic) => {
            (f64::INFINITY, cubic)
        }
        Some((cubic, error)) if error > ON_CIRCLE_SLACK && leaves_its_circle(&samples, &cubic) => {
            (f64::INFINITY, cubic)
        }
        Some((cubic, error)) => (error, cubic),
        None => (f64::INFINITY, a.edge.cubic),
    }
}

/// Greedy merging within one run: always take the cheapest acceptable pair
/// next, and never merge across the run's ends.
fn simplify_run(run: &[Edge], cyclic: bool, options: SimplifyOptions) -> Vec<Edge> {
    let tolerance = options.tolerance;
    let mut pieces: Vec<Piece> = run.iter().map(|e| Piece::new(*e)).collect();
    let mut candidates: Vec<Option<(f64, Cubic)>> = vec![None; pieces.len()];
    let minimum = if cyclic { MIN_CLOSED_SEGMENTS } else { 1 };
    loop {
        let n = pieces.len();
        if n <= minimum {
            break;
        }
        let pairs = if cyclic { n } else { n - 1 };
        let mut best: Option<(usize, f64)> = None;
        for i in 0..pairs {
            let j = (i + 1) % n;
            if candidates[i].is_none() {
                candidates[i] = Some(try_merge(&pieces[i], &pieces[j]));
            }
            let error = candidates[i].as_ref().map_or(f64::INFINITY, |c| c.0);
            if error <= tolerance && best.is_none_or(|(_, b)| error < b) {
                best = Some((i, error));
            }
        }
        let Some((i, _)) = best else {
            break;
        };
        let j = (i + 1) % n;
        let (_, cubic) = candidates[i].take().expect("candidate was just computed");
        let mut samples = pieces[i].samples.clone();
        samples.extend_from_slice(&pieces[j].samples[1..]);
        pieces[i] = Piece {
            edge: Edge {
                cubic,
                line: false,
                implicit: false,
            },
            samples,
        };
        pieces.remove(j);
        candidates.remove(j);
        let n = pieces.len();
        let merged = if j > i { i } else { i - 1 };
        candidates[merged] = None;
        candidates[(merged + n - 1) % n] = None;
    }
    if options.smooth_kinks {
        smooth_kinks(&mut pieces, cyclic, tolerance);
    }
    pieces.into_iter().map(|p| p.edge).collect()
}

/// A turn smaller than this at a node is already smooth.
const KINK_MIN_TURN: f64 = 0.5;

/// Whether `new` turns back more often than `old` did.
fn turns_back_more(old: &Cubic, new: &Cubic) -> bool {
    let count = |cubic: &Cubic| {
        let mut turns = TurnBacks::default();
        turns.follow(&tangents(cubic));
        turns.count
    };
    count(new) > count(old)
}

/// A turn below this at a node is a kink that `smooth_kinks` may smooth; a
/// sharper one is a corner the drawing keeps. Measured on September 24, 2026
/// at the Simplify slider's far end (3 px) against smoothing everything the
/// merge rule calls no corner (`CORNER_TURN`, 45) and against the best of five
/// shared tangents instead of the halfway one: 20 with the halfway tangent
/// moved the drawing least (geometry +0.0086, pixels +0.0105 against the
/// merges alone) for nearly the largest gain (total -0.1042 against -0.1125),
/// and a turn of 20 to 45 degrees is a letter's serif or a leaf's tip more
/// often than a tracing error (testing/quality-round/kink-v20-3px.md).
const KINK_TURN: f64 = 20.;

/// Smooth the kinks the merges kept: a node where two pieces meet at a turn
/// below `KINK_TURN` has both pieces fitted again to the curve they replace,
/// leaving the node along one shared tangent, the halfway one (a straight
/// piece keeps its direction and the curve beside it turns to meet it). The node is smoothed when both stay within
/// `tolerance` and neither adds a turn-back. Merging can only remove a node,
/// so a node it had to keep kept its kink however far the slider went (the
/// owner's GitHub mark, September 23, 2026: a 17 degree kink on the cat's
/// head beside its ear, at every tolerance up to 3 px). Only for a tolerance
/// chosen by hand (`SimplifyOptions::smooth_kinks`): at Auto's it is smoother
/// by the rule's own weights but a little less faithful, which the rule
/// refuses (testing/quality-round/kink-v20.md).
fn smooth_kinks(pieces: &mut [Piece], cyclic: bool, tolerance: f64) {
    let n = pieces.len();
    let nodes = if cyclic { n } else { n.saturating_sub(1) };
    for i in 0..nodes {
        let j = (i + 1) % n;
        if i == j || (pieces[i].edge.line && pieces[j].edge.line) {
            continue;
        }
        let (a, b) = (&pieces[i], &pieces[j]);
        let (Some(t), Some(u)) = (arriving(&a.edge), leaving(&b.edge)) else {
            continue;
        };
        if !(KINK_MIN_TURN..KINK_TURN).contains(&signed_turn(t, u).abs()) {
            continue;
        }
        let refit = |piece: &Piece, start: Point, end: Point| -> Option<(Edge, f64)> {
            if piece.edge.line {
                return Some((piece.edge, 0.));
            }
            let (cubic, error) = fit_samples(&piece.samples, start, end)?;
            (!turns_back_more(&piece.edge.cubic, &cubic)).then_some((
                Edge {
                    cubic,
                    line: false,
                    implicit: false,
                },
                error,
            ))
        };
        let pair = |shared: Point| -> Option<(Edge, Edge, f64)> {
            let (first, e1) = refit(a, a.start_direction(), scale(shared, -1.))?;
            let (second, e2) = refit(b, shared, b.end_direction())?;
            Some((first, second, e1.max(e2)))
        };
        let best = if a.edge.line {
            pair(t)
        } else if b.edge.line {
            pair(u)
        } else {
            normalized(add(t, u)).and_then(pair)
        };
        if let Some((first, second, error)) = best {
            if error <= tolerance {
                pieces[i].edge = first;
                pieces[j].edge = second;
            }
        }
    }
}

/// One stretch of boundary between two junctions, or a whole junction-free
/// outline, in the orientation this subpath walks it.
pub(crate) struct Run {
    pub(crate) edges: Vec<Edge>,
    pub(crate) cyclic: bool,
}

pub(crate) fn runs_of(subpath: &Subpath, is_junction: &dyn Fn(Point) -> bool) -> Vec<Run> {
    let n = subpath.edges.len();
    let junctions: Vec<usize> = (0..n)
        .filter(|&i| is_junction(subpath.edges[i].start()))
        .collect();
    if !subpath.cyclic() {
        let mut bounds = vec![0];
        bounds.extend(junctions.iter().copied().filter(|&i| i > 0));
        bounds.push(n);
        bounds.dedup();
        return bounds
            .windows(2)
            .map(|w| Run {
                edges: subpath.edges[w[0]..w[1]].to_vec(),
                cyclic: false,
            })
            .collect();
    }
    if junctions.is_empty() {
        return vec![Run {
            edges: subpath.edges.clone(),
            cyclic: true,
        }];
    }
    let mut runs = Vec::new();
    for (k, &start) in junctions.iter().enumerate() {
        let end = junctions[(k + 1) % junctions.len()];
        let mut edges = Vec::new();
        let mut i = start;
        loop {
            edges.push(subpath.edges[i]);
            i = (i + 1) % n;
            if i == end {
                break;
            }
        }
        runs.push(Run {
            edges,
            cyclic: false,
        });
    }
    runs
}

/// The run's identity independent of direction and, for outlines, of where
/// the walk started; plus the edges in that canonical order and whether this
/// occurrence is the reverse of it.
pub(crate) fn canonical(run: &Run) -> (Vec<EdgeKey>, Vec<Edge>, bool) {
    let keys: Vec<EdgeKey> = run.edges.iter().map(|e| e.key().0).collect();
    let n = keys.len();
    let forward_from =
        |start: usize| -> Vec<EdgeKey> { (0..n).map(|k| keys[(start + k) % n]).collect() };
    let backward_from =
        |start: usize| -> Vec<EdgeKey> { (0..n).map(|k| keys[(start + n - k) % n]).collect() };
    // A run whose key sequence reads the same both ways (one piece, or a
    // palindrome) has no direction of its own: the first piece's own
    // orientation against its key decides, so the two fills sharing it
    // agree (before this tie-break, September 22, 2026, a cached run could
    // come back reversed for the other fill and collapse into a piece from
    // a node to itself: 1,173 such pieces on the astronaut at 0.5 px).
    let (start, reversed) = if run.cyclic {
        let min = (0..n).min_by_key(|&i| keys[i]).unwrap_or(0);
        let (forward, backward) = (forward_from(min), backward_from(min));
        let reversed = if forward == backward {
            run.edges[min].key().1
        } else {
            backward < forward
        };
        (min, reversed)
    } else {
        let backward: Vec<EdgeKey> = keys.iter().rev().copied().collect();
        if backward == keys {
            let reversed = run.edges[0].key().1;
            (if reversed { n - 1 } else { 0 }, reversed)
        } else if backward < keys {
            (n - 1, true)
        } else {
            (0, false)
        }
    };
    let edges: Vec<Edge> = if reversed {
        (0..n)
            .map(|k| run.edges[(start + n - k) % n].reversed())
            .collect()
    } else {
        (0..n).map(|k| run.edges[(start + k) % n]).collect()
    };
    let canonical_keys = if reversed {
        backward_from(start)
    } else {
        forward_from(start)
    };
    (canonical_keys, edges, reversed)
}

/// Parse one subpath list per byte range, in order.
pub(crate) fn parse_paths(
    svg: &str,
    ranges: &[(usize, usize)],
) -> Result<Vec<Vec<Subpath>>, String> {
    ranges
        .iter()
        .map(|&(start, end)| parse_path_data(&svg[start..end]))
        .collect::<Result<_, _>>()
}

/// The byte ranges of every path element and the subpaths parsed from each.
pub(crate) type ParsedPaths = (Vec<(usize, usize)>, Vec<Vec<Subpath>>);

pub(crate) fn parse_all_paths(svg: &str) -> Result<ParsedPaths, String> {
    let ranges = path_ranges(svg)?;
    let paths = parse_paths(svg, &ranges)?;
    Ok((ranges, paths))
}

/// Simplify every `d` attribute of an SVG produced by the pipeline. Anything
/// outside the path data, including fills, strokes and the document root, is
/// copied through unchanged.
pub fn simplify_svg(
    svg: &str,
    options: SimplifyOptions,
) -> Result<(String, SimplifyStats), String> {
    if !options.tolerance.is_finite() || options.tolerance <= 0. {
        return Err("Simplification tolerance must be a positive distance".into());
    }
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let mut stats = SimplifyStats {
        paths: paths.len(),
        ..Default::default()
    };
    let mut occurrences: HashMap<EdgeKey, usize> = HashMap::new();
    for subpath in paths.iter().flatten() {
        stats.subpaths += 1;
        stats.segments_before += subpath.edges.len();
        for edge in &subpath.edges {
            *occurrences.entry(edge.key().0).or_default() += 1;
        }
    }
    let mut junctions = junction_keys(&paths);
    junctions.extend(border_turns(svg, &paths));
    stats.junctions = junctions.len();
    let is_junction = |p: Point| junctions.contains(&key(p));

    let mut cache: HashMap<Vec<EdgeKey>, Vec<Edge>> = HashMap::new();
    for path in &mut paths {
        for subpath in path.iter_mut() {
            let mut rebuilt = Vec::new();
            for run in runs_of(subpath, &is_junction) {
                let (run_key, edges, reversed) = canonical(&run);
                let shared = edges
                    .iter()
                    .any(|e| occurrences.get(&e.key().0).copied().unwrap_or(0) > 1);
                let simplified = match cache.get(&run_key) {
                    Some(done) => done.clone(),
                    None => {
                        stats.runs += 1;
                        if shared {
                            stats.shared_runs += 1;
                        }
                        let done = simplify_run(&edges, run.cyclic, options);
                        cache.insert(run_key, done.clone());
                        done
                    }
                };
                if reversed {
                    rebuilt.extend(simplified.iter().rev().map(Edge::reversed));
                } else {
                    rebuilt.extend(simplified);
                }
            }
            stats.segments_after += rebuilt.len();
            subpath.edges = rebuilt;
        }
    }

    Ok((splice(svg, &ranges, &paths), stats))
}

/// Byte ranges of every `d="..."` value in the document.
pub(crate) fn path_ranges(svg: &str) -> Result<Vec<(usize, usize)>, String> {
    let mut ranges = Vec::new();
    let mut search = 0;
    while let Some(found) = svg[search..].find(" d=\"") {
        let start = search + found + 4;
        let end = start + svg[start..].find('"').ok_or("Unterminated path data")?;
        ranges.push((start, end));
        search = end + 1;
    }
    Ok(ranges)
}

/// The document with every path's data rewritten; everything else verbatim.
pub(crate) fn splice(svg: &str, ranges: &[(usize, usize)], paths: &[Vec<Subpath>]) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut last = 0;
    for ((start, end), path) in ranges.iter().zip(paths) {
        out.push_str(&svg[last..*start]);
        out.push_str(&write_path_data(path));
        last = *end;
    }
    out.push_str(&svg[last..]);
    out
}

/// Nodes on the picture's border where an outline turns off the side it runs
/// along (a corner of the picture, or where a region's edge leaves the
/// frame): kept like junctions, so no merge cuts or overshoots the frame.
/// Without them, at the desktop's Auto 2.8 px, the border pieces around the
/// picture's corner of a shape touching it became one cubic that left the
/// picture by 8 px and uncovered the corner (shape-outline-band, September
/// 22, 2026). The viewBox is the engine's, from the origin.
fn border_turns(svg: &str, paths: &[Vec<Subpath>]) -> HashSet<Key> {
    let Some((width, height)) = viewbox_size(svg) else {
        return HashSet::new();
    };
    // The sides of the frame a point lies on: left, top, right, bottom.
    let sides = |p: Point| [p.x == 0., p.y == 0., p.x == width, p.y == height];
    let mut along: HashMap<Key, Vec<Option<usize>>> = HashMap::new();
    for edge in paths.iter().flatten().flat_map(|s| s.edges.iter()) {
        let points = edge.cubic.points;
        // The side the whole piece runs along, if it runs along one.
        let side = (0..4).find(|&s| points.iter().all(|p| sides(*p)[s]));
        for end in [edge.start(), edge.end()] {
            if sides(end).contains(&true) {
                along.entry(key(end)).or_default().push(side);
            }
        }
    }
    along
        .into_iter()
        .filter(|(_, sides)| sides.iter().any(|s| s.is_none() || *s != sides[0]))
        .map(|(k, _)| k)
        .collect()
}

/// Nodes where three or more distinct edges meet, plus the ends of any open
/// outline: never merged and never rounded.
pub(crate) fn junction_keys(paths: &[Vec<Subpath>]) -> HashSet<Key> {
    let mut incident: HashMap<Key, HashSet<EdgeKey>> = HashMap::new();
    let mut forced: HashSet<Key> = HashSet::new();
    for subpath in paths.iter().flatten() {
        for edge in &subpath.edges {
            let (edge_key, _) = edge.key();
            incident
                .entry(key(edge.start()))
                .or_default()
                .insert(edge_key);
            incident
                .entry(key(edge.end()))
                .or_default()
                .insert(edge_key);
        }
        if !subpath.cyclic() {
            if let (Some(first), Some(last)) = (subpath.edges.first(), subpath.edges.last()) {
                forced.insert(key(first.start()));
                forced.insert(key(last.end()));
            }
        }
    }
    incident
        .iter()
        .filter(|(_, edges)| edges.len() >= 3)
        .map(|(k, _)| *k)
        .chain(forced)
        .collect()
}

/// Exact de Casteljau split of a piece at parameter `t`.
fn split_edge(edge: &Edge, t: f64) -> (Edge, Edge) {
    let [p0, p1, p2, p3] = edge.cubic.points;
    let p01 = lerp(p0, p1, t);
    let p12 = lerp(p1, p2, t);
    let p23 = lerp(p2, p3, t);
    let p012 = lerp(p01, p12, t);
    let p123 = lerp(p12, p23, t);
    let mid = lerp(p012, p123, t);
    let make = |points: [Point; 4]| Edge {
        cubic: Cubic { points },
        line: edge.line,
        implicit: false,
    };
    (make([p0, p01, p012, mid]), make([mid, p123, p23, p3]))
}

/// One node to round, and how far the rounding reaches: the fraction in
/// (0, 1] of the shorter of the two pieces meeting there that is cut away on
/// each side and replaced by the rounded corner, but never more than that
/// fraction of `ROUNDING_CAP` times the picture's shorter side, so a corner
/// between two long straight edges still rounds like a corner, not a lens.
/// Both sides are cut by the same length, so the corner comes out as an even
/// arc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rounding {
    pub at: Point,
    pub reach: f64,
}

/// The largest cut, as a fraction of the picture's shorter side, that a
/// rounding with reach 1 may make on each side of a corner.
pub const ROUNDING_CAP: f64 = 0.15;

/// The `viewBox` width and height of the document's root element.
pub fn viewbox_size(svg: &str) -> Option<(f64, f64)> {
    let at = svg.find("viewBox=\"")? + 9;
    let end = at + svg[at..].find('"')?;
    let values: Vec<f64> = svg[at..end]
        .split([' ', ','])
        .filter(|v| !v.is_empty())
        .filter_map(|v| v.parse().ok())
        .collect();
    match values.as_slice() {
        [_, _, w, h] if *w > 0. && *h > 0. => Some((*w, *h)),
        _ => None,
    }
}

/// A piece with its ends cut back for the fillets at its nodes: the part kept
/// exactly (none when the cuts meet), and where each cut lies with the piece's
/// forward direction there. All in the piece's canonical orientation.
struct Trim {
    middle: Option<Edge>,
    start: (Point, Point),
    end: (Point, Point),
}

fn trim(edge: &Edge, from_start: f64, from_end: f64) -> Trim {
    let dir = |t: f64| direction_at(edge, t);
    if from_start + from_end >= 1. - 1e-9 {
        let t = if from_start <= 0. {
            0.
        } else if from_end <= 0. {
            1.
        } else {
            from_start / (from_start + from_end)
        };
        let point = if t <= 0. {
            edge.start()
        } else if t >= 1. {
            edge.end()
        } else {
            split_edge(edge, t).1.start()
        };
        let at = (point, dir(t));
        return Trim {
            middle: None,
            start: at,
            end: at,
        };
    }
    let (start_point, rest) = if from_start > 0. {
        let (_, rest) = split_edge(edge, from_start);
        (rest.start(), rest)
    } else {
        (edge.start(), *edge)
    };
    let (end_point, middle) = if from_end > 0. {
        let local = 1. - from_end / (1. - from_start);
        let (middle, tail) = split_edge(&rest, local);
        (tail.start(), middle)
    } else {
        (rest.end(), rest)
    };
    Trim {
        middle: Some(middle),
        start: (start_point, dir(from_start)),
        end: (end_point, dir(1. - from_end)),
    }
}

/// A piece's length along its control polygon: exact for lines, a close
/// upper bound for the engine's short curves, and enough to size a cut.
fn extent(edge: &Edge) -> f64 {
    let [p0, p1, p2, p3] = edge.cubic.points;
    if edge.line {
        distance(p0, p3)
    } else {
        distance(p0, p1) + distance(p1, p2) + distance(p2, p3)
    }
}

/// The unit direction of travel at parameter `t`; the chord when the curve
/// is degenerate there.
fn direction_at(edge: &Edge, t: f64) -> Point {
    normalized(derivative(&edge.cubic, t))
        .or_else(|| normalized(sub(edge.end(), edge.start())))
        .unwrap_or(Point { x: 1., y: 0. })
}

/// The rounded corner between two cut ends: one cubic from `a` to `b` leaving
/// `a` along `ta` and arriving at `b` along `tb`, with handle lengths that
/// make it a close approximation of a circular arc through those tangents,
/// so a square corner becomes a quarter circle and a sharp tip a round cap.
fn fillet(a: Point, ta: Point, b: Point, tb: Point) -> Option<Edge> {
    let chord = distance(a, b);
    if !(chord > 1e-9) {
        return None;
    }
    let turn = dot(ta, tb).clamp(-1., 1.).acos();
    let handle = if turn < 1e-6 {
        chord / 3.
    } else {
        let radius = chord / (2. * (turn / 2.).sin());
        4. / 3. * (turn / 4.).tan() * radius
    };
    Some(Edge {
        cubic: Cubic {
            points: [a, add(a, scale(ta, handle)), sub(b, scale(tb, handle)), b],
        },
        line: false,
        implicit: false,
    })
}

struct Corner {
    prev: EdgeKey,
    next: EdgeKey,
    arc: Edge,
}

/// Round the listed corners. Each corner node is cut away: the two pieces
/// meeting there are shortened by the same length (the node's reach times
/// the shorter piece), and the gap is bridged by one arc-like cubic tangent
/// to both cut ends, the way a rounded-corner tool works. The corner node
/// itself disappears and the two cut points become nodes. A piece cut from
/// both ends by two rounded nodes keeps its middle, or, when the cuts cover
/// it, hands the meeting point to both fillets. Pieces shared by two fills
/// are cut once and get the same fillet on both sides. Nodes absent from the
/// document and junctions are ignored. Returns the rewritten SVG and how many
/// corners were rounded.
pub fn smooth_nodes(svg: &str, nodes: &[Rounding]) -> Result<(String, usize), String> {
    if nodes
        .iter()
        .any(|n| !(n.reach.is_finite() && n.reach > 0. && n.reach <= 1.))
    {
        return Err("Rounding reach must be between 0 and 1".into());
    }
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let wanted: HashMap<Key, f64> = nodes.iter().map(|n| (key(n.at), n.reach)).collect();
    if wanted.is_empty() {
        return Ok((svg.to_owned(), 0));
    }
    let junctions = junction_keys(&paths);
    let cap = viewbox_size(svg).map(|(w, h)| w.min(h) * ROUNDING_CAP);

    // Pass 1: how far each piece is cut from its canonical start and end, and
    // which two pieces meet at each rounded corner.
    let mut cuts: HashMap<EdgeKey, (f64, f64)> = HashMap::new();
    let mut corners: HashMap<Key, (Edge, Edge)> = HashMap::new();
    for subpath in paths.iter().flatten() {
        let n = subpath.edges.len();
        if n < 2 {
            continue;
        }
        let cyclic = subpath.cyclic();
        for i in 0..n {
            if i == 0 && !cyclic {
                continue;
            }
            let k = key(subpath.edges[i].start());
            let Some(&reach) = wanted.get(&k) else {
                continue;
            };
            if junctions.contains(&k) {
                continue;
            }
            let prev = subpath.edges[(i + n - 1) % n];
            let next = subpath.edges[i];
            let (len_prev, len_next) = (extent(&prev), extent(&next));
            let mut cut_length = reach * len_prev.min(len_next);
            if let Some(cap) = cap {
                cut_length = cut_length.min(reach * cap);
            }
            let mut cut = |edge: &Edge, length: f64, at_traversal_end: bool| {
                let fraction = if length > 1e-9 {
                    (cut_length / length).min(1.)
                } else {
                    1.
                };
                let (edge_key, reversed) = edge.key();
                let entry = cuts.entry(edge_key).or_insert((0., 0.));
                if at_traversal_end != reversed {
                    entry.1 = entry.1.max(fraction);
                } else {
                    entry.0 = entry.0.max(fraction);
                }
            };
            cut(&prev, len_prev, true);
            cut(&next, len_next, false);
            corners.entry(k).or_insert((prev, next));
        }
    }
    if corners.is_empty() {
        return Ok((svg.to_owned(), 0));
    }
    for (from_start, from_end) in cuts.values_mut() {
        let total = *from_start + *from_end;
        if total > 1. {
            *from_start /= total;
            *from_end /= total;
        }
    }

    // Pass 2: cut every touched piece once, in its canonical orientation.
    let mut trims: HashMap<EdgeKey, Trim> = HashMap::new();
    for subpath in paths.iter().flatten() {
        for edge in &subpath.edges {
            let (edge_key, reversed) = edge.key();
            if trims.contains_key(&edge_key) {
                continue;
            }
            let Some(&(from_start, from_end)) = cuts.get(&edge_key) else {
                continue;
            };
            let canonical = if reversed { edge.reversed() } else { *edge };
            trims.insert(edge_key, trim(&canonical, from_start, from_end));
        }
    }

    // Pass 3: one arc per corner between the cut ends of its two pieces.
    let mut arcs: HashMap<Key, Corner> = HashMap::new();
    for (k, (prev, next)) in &corners {
        let cut_end = |edge: &Edge| -> (Point, Point) {
            let (edge_key, reversed) = edge.key();
            let t = &trims[&edge_key];
            if reversed {
                (t.start.0, scale(t.start.1, -1.))
            } else {
                t.end
            }
        };
        let cut_start = |edge: &Edge| -> (Point, Point) {
            let (edge_key, reversed) = edge.key();
            let t = &trims[&edge_key];
            if reversed {
                (t.end.0, scale(t.end.1, -1.))
            } else {
                t.start
            }
        };
        let (a, ta) = cut_end(prev);
        let (b, tb) = cut_start(next);
        if let Some(arc) = fillet(a, ta, b, tb) {
            arcs.insert(
                *k,
                Corner {
                    prev: prev.key().0,
                    next: next.key().0,
                    arc,
                },
            );
        }
    }

    // Pass 4: rebuild every outline from the kept middles and the arcs, in
    // whichever direction the outline runs.
    for subpath in paths.iter_mut().flatten() {
        let n = subpath.edges.len();
        let cyclic = subpath.cyclic();
        let mut rebuilt = Vec::with_capacity(n + arcs.len());
        for i in 0..n {
            let edge = subpath.edges[i];
            let (edge_key, reversed) = edge.key();
            match trims.get(&edge_key) {
                Some(t) => {
                    if let Some(middle) = t.middle {
                        rebuilt.push(if reversed { middle.reversed() } else { middle });
                    }
                }
                None => rebuilt.push(edge),
            }
            if i + 1 < n || cyclic {
                let next = subpath.edges[(i + 1) % n];
                if let Some(corner) = arcs.get(&key(edge.end())) {
                    let (nk, _) = next.key();
                    if corner.prev == edge_key && corner.next == nk {
                        rebuilt.push(corner.arc);
                    } else if corner.prev == nk && corner.next == edge_key {
                        rebuilt.push(corner.arc.reversed());
                    }
                }
            }
        }
        subpath.edges = rebuilt;
    }
    Ok((splice(svg, &ranges, &paths), arcs.len()))
}

#[cfg(test)]
mod tests;
