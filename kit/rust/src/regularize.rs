//! Regularities the picture asks for, put back: a run of pieces that all
//! lie along one line becomes that line, an outline, or the stretch of one
//! between the nodes it shares with a third fill, that lies on one circle
//! becomes circle arcs, and a closed outline on one ellipse becomes that
//! ellipse. The engine traces a straight anti-aliased edge as a chain of
//! slightly bent pieces that wobble a fraction of a pixel about the true
//! edge, and a circle or an ellipse as a ring of pieces that flatten and
//! bulge by as much; both read as wobble at any zoom, and no slider of the
//! original changes that. Every run is judged on the same canonical
//! geometry from each fill that shares it and junction nodes never move, so
//! shared edges stay sealed.
//!
//! Owned post-processing of the engine's output. The CLI and the desktop run
//! it after `simplify` and before `straighten` (then the desktop's rounded
//! corners), so it sees simplified pieces, fewer and longer than the
//! engine's, and draws a circle or an ellipse that simplify merged into a
//! few pieces true again. Run first instead (September 22, 2026, through
//! the desktop's chain), it lost: simplify merged its arcs again (the exact
//! circles' radius spread 0.05 -> 0.83 px), and run on both sides of
//! simplify it still lost on the circles, the ellipse and the blended logo.
use crate::geometry::{Cubic, Point};
use crate::simplify::{
    add, arriving, canonical, distance, dot, junction_keys, key, leaving, length, normalized,
    parse_all_paths, runs_of, scale, splice, sub, Edge, EdgeKey, Run,
};
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI};

mod pixels;
pub use pixels::PixelCheck;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegularizeOptions {
    /// Every point of a run may stray this far, in source pixels, from the
    /// line or circle that replaces it.
    pub band: f64,
}
impl Default for RegularizeOptions {
    fn default() -> Self {
        // The engine's tracing of a straight anti-aliased edge wobbles about
        // 0.6 px peak to peak; a run within this band of one line or circle
        // reads as that line or circle.
        Self { band: 0.8 }
    }
}
impl RegularizeOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.band.is_finite() && self.band >= 0.) {
            return Err("Regularizing needs a non-negative band in source pixels".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegularizeStats {
    /// Runs replaced by one line, and the pieces they held.
    pub lines_made: usize,
    pub pieces_into_lines: usize,
    /// Closed outlines replaced by a whole circle, stretches replaced by an
    /// arc of one, and the pieces they held.
    pub circles_made: usize,
    pub arcs_made: usize,
    pub pieces_into_arcs: usize,
    /// Closed outlines replaced by a whole ellipse, and the pieces they held.
    pub ellipses_made: usize,
    pub pieces_into_ellipses: usize,
    /// Runs whose arcs the source's pixels refused (`PixelCheck`).
    pub arcs_refused: usize,
    pub pieces_before: usize,
    pub pieces_after: usize,
}
impl RegularizeStats {
    fn absorb(&mut self, other: RegularizeStats) {
        self.lines_made += other.lines_made;
        self.pieces_into_lines += other.pieces_into_lines;
        self.circles_made += other.circles_made;
        self.arcs_made += other.arcs_made;
        self.pieces_into_arcs += other.pieces_into_arcs;
        self.ellipses_made += other.ellipses_made;
        self.pieces_into_ellipses += other.pieces_into_ellipses;
        self.arcs_refused += other.arcs_refused;
    }
}

/// Points per curved piece when a run is tested, endpoints included.
const SAMPLES_PER_CURVE: usize = 8;
/// A run is curved, however well a line fits its wobble, when its best
/// circle bulges more than this fraction of the band over the chord and
/// explains the run at least this much better than the chord does (the
/// residual ratio): the test that keeps a gentle bend from being flattened
/// while wobble, which no circle explains, still straightens.
const CURVED_SAGITTA: f64 = 0.5;
const CURVED_RESIDUAL_RATIO: f64 = 0.5;
/// A run's length-weighted mean signed deviation from its chord may be at
/// most this fraction of the band; wobble averages out, a tilt does not.
const MAX_LEAN: f64 = 0.15;
/// An arc must turn through at least this angle to be drawn as one; less
/// is a bend a fitted piece already draws well.
const MIN_SWEEP: f64 = 30. * PI / 180.;
/// Pieces meeting at a sharper turn than this are a corner, never one arc.
const MAX_KINK: f64 = 25. * PI / 180.;
/// The circle's tangent at either end of a run must agree with the run's
/// own tangent there within this angle: a rounded corner that is not an arc
/// can lie within the band of a circle whose tangents are 20 degrees off,
/// and an arc drawn from that circle kinks against its neighbours.
const MAX_TANGENT_ERROR: f64 = 8. * PI / 180.;
/// A run of half a circle or more whose radial residual has a two-lobed
/// component (an ellipse's signature) taller than this fraction of the band
/// is an ellipse the engine traced faithfully, not a wobbly circle.
const MAX_ELLIPSE: f64 = 0.25;
/// Circles smaller than this radius are the engine's business, and so are
/// ellipses whose shorter semi-axis is.
const MIN_RADIUS: f64 = 2.;

/// A fitted circle: centre and radius.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Circle {
    pub(crate) centre: Point,
    pub(crate) radius: f64,
}

/// Kåsa's algebraic least-squares circle through `points`, or `None` when
/// the points are collinear as far as the arithmetic can tell.
pub(crate) fn fit_circle(points: &[Point]) -> Option<Circle> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let mean = scale(
        points
            .iter()
            .fold(Point { x: 0., y: 0. }, |acc, p| add(acc, *p)),
        1. / n,
    );
    let (mut suu, mut suv, mut svv, mut suuu, mut svvv, mut suuv, mut suvv) =
        (0., 0., 0., 0., 0., 0., 0.);
    for p in points {
        let (u, v) = (p.x - mean.x, p.y - mean.y);
        suu += u * u;
        suv += u * v;
        svv += v * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suuv += u * u * v;
        suvv += u * v * v;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-12 {
        return None;
    }
    let r1 = (suuu + suvv) / 2.;
    let r2 = (suuv + svvv) / 2.;
    let uc = (r1 * svv - r2 * suv) / det;
    let vc = (suu * r2 - suv * r1) / det;
    let radius = (uc * uc + vc * vc + (suu + svv) / n).sqrt();
    if !radius.is_finite() {
        return None;
    }
    Some(Circle {
        centre: Point {
            x: mean.x + uc,
            y: mean.y + vc,
        },
        radius,
    })
}

/// The points of a stretch of pieces, in order, endpoints included once;
/// a line contributes its ends.
fn samples_of(edges: &[Edge]) -> Vec<Point> {
    let mut points = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        if index == 0 {
            points.push(edge.start());
        }
        if edge.line {
            points.push(edge.end());
        } else {
            for k in 1..=SAMPLES_PER_CURVE {
                points.push(edge.cubic.evaluate(k as f64 / SAMPLES_PER_CURVE as f64));
            }
        }
    }
    points
}

/// Whether a circle explains the samples as a bend: the best circle bulges
/// more than `CURVED_SAGITTA` of the band over the chord of length `len`
/// and leaves under `CURVED_RESIDUAL_RATIO` of the chord's residual.
fn bends(points: &[Point], line_residual: f64, len: f64, band: f64) -> bool {
    let Some(circle) = fit_circle(points) else {
        return false;
    };
    let sagitta = len * len / (8. * circle.radius);
    if sagitta <= band * CURVED_SAGITTA {
        return false;
    }
    let circle_residual: f64 = points
        .iter()
        .map(|p| (distance(*p, circle.centre) - circle.radius).powi(2))
        .sum();
    circle_residual <= CURVED_RESIDUAL_RATIO * CURVED_RESIDUAL_RATIO * line_residual
}

/// Whether the stretch is a bend by the test that keeps a gentle bend from
/// being drawn as one line, whatever the band says about its chord:
/// `straighten` asks it of a flat piece and its smooth neighbours before it
/// draws the piece as a line, so a bend regularize kept is not flattened
/// into a polyline one piece later.
pub(crate) fn is_bend(edges: &[Edge], band: f64) -> bool {
    let (a, b) = (edges[0].start(), edges[edges.len() - 1].end());
    let chord = sub(b, a);
    let len = length(chord);
    if len < 1e-9 {
        return false;
    }
    let dir = scale(chord, 1. / len);
    let points = samples_of(edges);
    let line_residual: f64 = points
        .iter()
        .map(|p| {
            let v = sub(*p, a);
            (v.x * dir.y - v.y * dir.x).powi(2)
        })
        .sum();
    bends(&points, line_residual, len, band)
}

/// Whether pieces `a` then `b` meet at a corner: the tangent turns more
/// than `MAX_KINK` at the node between them.
fn is_corner(a: &Edge, b: &Edge) -> bool {
    matches!((arriving(a), leaving(b)), (Some(a), Some(b)) if dot(a, b) < MAX_KINK.cos())
}

/// Whether every sample of the stretch lies within `band` of its chord and
/// between the chord's ends, and no circle explains the stretch as a bend.
fn is_straight(edges: &[Edge], band: f64) -> bool {
    let (a, b) = (edges[0].start(), edges[edges.len() - 1].end());
    let chord = sub(b, a);
    let len = length(chord);
    if len < 1e-9 {
        return false;
    }
    let dir = scale(chord, 1. / len);
    let points = samples_of(edges);
    let mut line_residual = 0.;
    // Signed deviation, each sample weighted by the length of its piece,
    // so a long piece counts for its length: a run whose samples lean to
    // one side of the chord is tilted by an end node that sits off the
    // edge, and drawing the chord would tilt the whole edge with it.
    let mut lean = 0.;
    let mut weight = 0.;
    let mut index = 0;
    for (k, edge) in edges.iter().enumerate() {
        let count = if edge.line { 1 } else { SAMPLES_PER_CURVE } + usize::from(k == 0);
        let piece_weight = distance(edge.start(), edge.end()) / count as f64;
        for p in &points[index..index + count] {
            let v = sub(*p, a);
            let along = dot(v, dir);
            let signed = v.x * dir.y - v.y * dir.x;
            let across = signed.abs();
            if across > band || along < -band || along > len + band {
                return false;
            }
            line_residual += across * across;
            lean += signed * piece_weight;
            weight += piece_weight;
        }
        index += count;
    }
    if weight > 0. && (lean / weight).abs() > band * MAX_LEAN {
        return false;
    }
    !bends(&points, line_residual, len, band)
}

/// The angles of the points about the centre, unwrapped along the run so
/// the last minus the first is the turn the run makes.
fn unwrapped_angles(points: &[Point], centre: Point) -> Vec<f64> {
    unwrapped(
        points
            .iter()
            .map(|p| (p.y - centre.y).atan2(p.x - centre.x))
            .collect(),
    )
}

/// A circle that every sample of the stretch of curved pieces lies within
/// `band` of, with the signed angle the stretch turns through around it.
/// Lines are the engine's own statement of straightness and corners its
/// statement of a kink; neither joins an arc.
fn as_arc(edges: &[Edge], band: f64) -> Option<(Circle, f64)> {
    arc_fit(edges, band, true)
}

/// `as_arc`, with the whole ring's corner test only when `corners`.
fn arc_fit(edges: &[Edge], band: f64, corners: bool) -> Option<(Circle, f64)> {
    if edges.iter().any(|e| e.line) {
        return None;
    }
    for pair in edges.windows(2) {
        let (Some(a), Some(b)) = (arriving(&pair[0]), leaving(&pair[1])) else {
            return None;
        };
        if dot(a, b) < MAX_KINK.cos() {
            return None;
        }
    }
    let points = samples_of(edges);
    let circle = fit_circle(&points)?;
    if circle.radius < MIN_RADIUS {
        return None;
    }
    if points
        .iter()
        .any(|p| (distance(*p, circle.centre) - circle.radius).abs() > band)
    {
        return None;
    }
    let angles = unwrapped_angles(&points, circle.centre);
    let sweep = angles[angles.len() - 1] - angles[0];
    // The run must go one way round: a stretch that doubles back is not
    // an arc, whatever circle its points lie on.
    let monotone = angles.windows(2).all(|w| (w[1] - w[0]) * sweep >= -1e-9);
    if !monotone || sweep.abs() > 2. * PI + 1e-6 {
        return None;
    }
    // An ellipse lies within the band of a circle too; its radial residual
    // has two lobes where a wobbly circle's has none.
    if sweep.abs() >= PI {
        let (mut c2, mut s2, mut cc, mut ss, mut cs) = (0., 0., 0., 0., 0.);
        for (p, angle) in points.iter().zip(&angles) {
            let residual = distance(*p, circle.centre) - circle.radius;
            let (cos2, sin2) = ((2. * angle).cos(), (2. * angle).sin());
            c2 += residual * cos2;
            s2 += residual * sin2;
            cc += cos2 * cos2;
            ss += sin2 * sin2;
            cs += cos2 * sin2;
        }
        let det = cc * ss - cs * cs;
        if det.abs() > 1e-9 {
            let a = (c2 * ss - s2 * cs) / det;
            let b = (cc * s2 - cs * c2) / det;
            if (a * a + b * b).sqrt() > band * MAX_ELLIPSE {
                return None;
            }
        }
    }
    // A small closed shape with corners (a rounded-square bullet, a rounded
    // triangle) lies within the band of a circle too: an 8 px square with
    // 2 px corners stays within 0.41 px of one. Its residual has three or
    // four lobes where a wobbly circle's has none (round two of the Opus
    // 5.5 review).
    if corners && sweep.abs() >= 2. * PI - 1e-6 {
        let residuals: Vec<(f64, f64)> = points
            .iter()
            .zip(&angles)
            .map(|(p, angle)| (*angle, distance(*p, circle.centre) - circle.radius))
            .collect();
        if cornered(&residuals, band) {
            return None;
        }
    }
    // And the circle must run the way the run runs at both ends.
    let tangent = |p: Point| {
        let radial = sub(p, circle.centre);
        let t = Point {
            x: -radial.y,
            y: radial.x,
        };
        normalized(if sweep >= 0. { t } else { scale(t, -1.) })
    };
    let last = edges.len() - 1;
    for (own, along) in [
        (leaving(&edges[0]), tangent(edges[0].start())),
        (arriving(&edges[last]), tangent(edges[last].end())),
    ] {
        let (Some(own), Some(along)) = (own, along) else {
            return None;
        };
        if dot(own, along) < MAX_TANGENT_ERROR.cos() {
            return None;
        }
    }
    Some((circle, sweep))
}

/// The point on `circle` at `angle`.
fn on_circle(circle: Circle, angle: f64) -> Point {
    Point {
        x: circle.centre.x + circle.radius * angle.cos(),
        y: circle.centre.y + circle.radius * angle.sin(),
    }
}

/// Pieces along `circle` from angle `from` turning `sweep`, each at most a
/// quarter turn, the first starting at `start` and the last ending at `end`
/// (the ends of the run they replace, which stay exactly where they were;
/// each is within the band of the circle).
fn arc_pieces(circle: Circle, from: f64, sweep: f64, start: Point, end: Point) -> Vec<Edge> {
    let count = ((sweep.abs() / FRAC_PI_2) - 1e-9).ceil().max(1.) as usize;
    let step = sweep / count as f64;
    let kappa = 4. / 3. * (step / 4.).tan() * circle.radius;
    let tangent = |angle: f64| Point {
        x: -angle.sin(),
        y: angle.cos(),
    };
    let mut pieces = Vec::with_capacity(count);
    for k in 0..count {
        let (a0, a1) = (from + step * k as f64, from + step * (k + 1) as f64);
        let (p0, p3) = (on_circle(circle, a0), on_circle(circle, a1));
        let p1 = add(p0, scale(tangent(a0), kappa));
        let p2 = sub(p3, scale(tangent(a1), kappa));
        pieces.push(Edge {
            cubic: Cubic {
                points: [p0, p1, p2, p3],
            },
            line: false,
            implicit: false,
        });
    }
    // Pin the ends to the run's own end nodes, carrying the neighbouring
    // handle so the tangent there keeps its direction.
    let first = &mut pieces[0];
    let d = sub(start, first.cubic.points[0]);
    first.cubic.points[0] = start;
    first.cubic.points[1] = add(first.cubic.points[1], d);
    let last = pieces.len() - 1;
    let last = &mut pieces[last];
    let d = sub(end, last.cubic.points[3]);
    last.cubic.points[3] = end;
    last.cubic.points[2] = add(last.cubic.points[2], d);
    pieces
}

/// A fitted ellipse: its centre, its semi-axes (the first along `angle`)
/// and the angle of its first axis.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ellipse {
    pub(crate) centre: Point,
    pub(crate) axes: (f64, f64),
    pub(crate) angle: f64,
}
impl Ellipse {
    /// The point at parameter `t`, the angle on the circle the ellipse is
    /// the affine image of.
    fn at(&self, t: f64) -> Point {
        let (s, c) = self.angle.sin_cos();
        let (u, v) = (self.axes.0 * t.cos(), self.axes.1 * t.sin());
        Point {
            x: self.centre.x + u * c - v * s,
            y: self.centre.y + u * s + v * c,
        }
    }
    /// The derivative of `at` by `t`.
    fn velocity(&self, t: f64) -> Point {
        let (s, c) = self.angle.sin_cos();
        let (u, v) = (-self.axes.0 * t.sin(), self.axes.1 * t.cos());
        Point {
            x: u * c - v * s,
            y: u * s + v * c,
        }
    }
    /// The parameter of the ellipse's point nearest `p` (Newton's method on
    /// the foot of the normal, from the point's own eccentric angle), and
    /// the distance to it.
    fn nearest(&self, p: Point) -> (f64, f64) {
        let (s, c) = self.angle.sin_cos();
        let d = sub(p, self.centre);
        let (u, v) = (d.x * c + d.y * s, d.y * c - d.x * s);
        let (a, b) = self.axes;
        let mut t = (v * a).atan2(u * b);
        for _ in 0..8 {
            let (st, ct) = t.sin_cos();
            let f = (a * a - b * b) * st * ct - u * a * st + v * b * ct;
            let slope = (a * a - b * b) * (ct * ct - st * st) - u * a * ct - v * b * st;
            if slope.abs() < 1e-12 {
                break;
            }
            t -= f / slope;
        }
        (t, distance(p, self.at(t)))
    }
}

type Matrix3 = [[f64; 3]; 3];

fn determinant3(m: &Matrix3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The inverse by the adjugate, or `None` when the matrix is singular as far
/// as the arithmetic can tell.
fn invert3(m: &Matrix3) -> Option<Matrix3> {
    let det = determinant3(m);
    let size = m.iter().flatten().fold(0f64, |acc, x| acc.max(x.abs()));
    if !(det.abs() > 1e-12 * size.powi(3)) {
        return None;
    }
    // Columns of the inverse are the cross products of the rows.
    let columns = [cross3(m[1], m[2]), cross3(m[2], m[0]), cross3(m[0], m[1])];
    Some(std::array::from_fn(|i| {
        std::array::from_fn(|j| columns[j][i] / det)
    }))
}

/// The real eigenvalues of a 3x3 matrix: the real roots of its
/// characteristic cubic, each polished by two Newton steps.
fn real_eigenvalues3(m: &Matrix3) -> Vec<f64> {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let minors = m[0][0] * m[1][1] - m[0][1] * m[1][0] + m[0][0] * m[2][2] - m[0][2] * m[2][0]
        + m[1][1] * m[2][2]
        - m[1][2] * m[2][1];
    let det = determinant3(m);
    // l^3 - trace l^2 + minors l - det, and with l = x + trace / 3 the
    // depressed x^3 + p x + q.
    let p = minors - trace * trace / 3.;
    let q = -2. * trace.powi(3) / 27. + trace * minors / 3. - det;
    let discriminant = (q / 2.).powi(2) + (p / 3.).powi(3);
    let roots: Vec<f64> = if p.abs() < 1e-300 {
        vec![(-q).cbrt()]
    } else if discriminant > 0. {
        let root = discriminant.sqrt();
        vec![(-q / 2. + root).cbrt() + (-q / 2. - root).cbrt()]
    } else {
        let r = 2. * (-p / 3.).sqrt();
        let phi = ((3. * q / (2. * p)) * (-3. / p).sqrt())
            .clamp(-1., 1.)
            .acos()
            / 3.;
        (0..3)
            .map(|k| r * (phi - 2. * PI * k as f64 / 3.).cos())
            .collect()
    };
    roots
        .into_iter()
        .map(|x| {
            let mut l = x + trace / 3.;
            for _ in 0..2 {
                let value = ((l - trace) * l + minors) * l - det;
                let slope = (3. * l - 2. * trace) * l + minors;
                if slope.abs() > 1e-300 {
                    l -= value / slope;
                }
            }
            l
        })
        .filter(|l| l.is_finite())
        .collect()
}

/// A unit vector `v` with `(m - lambda) v = 0`: the largest cross product of
/// two rows of `m - lambda`.
fn null_vector3(m: &Matrix3, lambda: f64) -> Option<[f64; 3]> {
    let mut rows = *m;
    for (i, row) in rows.iter_mut().enumerate() {
        row[i] -= lambda;
    }
    let v = [
        cross3(rows[0], rows[1]),
        cross3(rows[0], rows[2]),
        cross3(rows[1], rows[2]),
    ]
    .into_iter()
    .max_by(|a, b| {
        let norm = |v: &[f64; 3]| v.iter().map(|x| x * x).sum::<f64>();
        norm(a).total_cmp(&norm(b))
    })?;
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    (norm > 1e-300).then(|| v.map(|x| x / norm))
}

/// Fitzgibbon's direct least-squares ellipse through `points`, in the
/// numerically stable form of Halir and Flusser (the quadratic and linear
/// parts of the conic separated, a 3x3 eigenproblem), on the points moved
/// to their mean and scaled to unit spread; `None` when the best conic is
/// not a real ellipse.
pub(crate) fn fit_ellipse(points: &[Point]) -> Option<Ellipse> {
    if points.len() < 6 {
        return None;
    }
    let n = points.len() as f64;
    let mean = scale(
        points
            .iter()
            .fold(Point { x: 0., y: 0. }, |acc, p| add(acc, *p)),
        1. / n,
    );
    let spread = (points
        .iter()
        .map(|p| {
            let d = sub(*p, mean);
            dot(d, d)
        })
        .sum::<f64>()
        / n)
        .sqrt();
    if !(spread > 1e-9) {
        return None;
    }
    // Scatter of the quadratic terms [x^2, xy, y^2] (s1), of those against
    // the linear terms [x, y, 1] (s2) and of the linear terms (s3).
    let (mut s1, mut s2, mut s3) = ([[0.; 3]; 3], [[0.; 3]; 3], [[0.; 3]; 3]);
    for p in points {
        let (x, y) = ((p.x - mean.x) / spread, (p.y - mean.y) / spread);
        let (quadratic, linear) = ([x * x, x * y, y * y], [x, y, 1.]);
        for i in 0..3 {
            for j in 0..3 {
                s1[i][j] += quadratic[i] * quadratic[j];
                s2[i][j] += quadratic[i] * linear[j];
                s3[i][j] += linear[i] * linear[j];
            }
        }
    }
    let s3_inverse = invert3(&s3)?;
    // The linear part in terms of the quadratic one, t = -s3^-1 s2^T, and
    // the reduced scatter s1 + s2 t premultiplied by the inverse of the
    // constraint matrix of 4AC - B^2.
    let t: Matrix3 = std::array::from_fn(|i| {
        std::array::from_fn(|j| -(0..3).map(|k| s3_inverse[i][k] * s2[j][k]).sum::<f64>())
    });
    let reduced: Matrix3 = std::array::from_fn(|i| {
        std::array::from_fn(|j| s1[i][j] + (0..3).map(|k| s2[i][k] * t[k][j]).sum::<f64>())
    });
    let m = [
        reduced[2].map(|x| x / 2.),
        reduced[1].map(|x| -x),
        reduced[0].map(|x| x / 2.),
    ];
    // Of the eigenvectors, the ellipse is the one with 4AC - B^2 > 0 (the
    // smallest eigenvalue if the arithmetic offers more than one).
    let quadratic = real_eigenvalues3(&m)
        .into_iter()
        .filter_map(|lambda| Some((lambda, null_vector3(&m, lambda)?)))
        .filter(|(_, v)| 4. * v[0] * v[2] - v[1] * v[1] > 0.)
        .min_by(|a, b| a.0.total_cmp(&b.0))?
        .1;
    let linear: [f64; 3] =
        std::array::from_fn(|i| (0..3).map(|k| t[i][k] * quadratic[k]).sum::<f64>());
    let sign = if quadratic[0] + quadratic[2] < 0. {
        -1.
    } else {
        1.
    };
    let [a, b, c] = quadratic.map(|x| x * sign);
    let [d, e, f] = linear.map(|x| x * sign);
    let den = 4. * a * c - b * b;
    let (x0, y0) = ((b * e - 2. * c * d) / den, (b * d - 2. * a * e) / den);
    let at_centre = a * x0 * x0 + b * x0 * y0 + c * y0 * y0 + d * x0 + e * y0 + f;
    let angle = 0.5 * b.atan2(a - c);
    let (s, co) = angle.sin_cos();
    let along = a * co * co + b * co * s + c * s * s;
    let across = a * s * s - b * co * s + c * co * co;
    let (u2, v2) = (-at_centre / along, -at_centre / across);
    if !(u2 > 0. && v2 > 0. && u2.is_finite() && v2.is_finite()) {
        return None;
    }
    Some(Ellipse {
        centre: Point {
            x: mean.x + spread * x0,
            y: mean.y + spread * y0,
        },
        axes: (spread * u2.sqrt(), spread * v2.sqrt()),
        angle,
    })
}

/// `angles` unwrapped along the run, so the last minus the first is the
/// turn the run makes.
fn unwrapped(mut angles: Vec<f64>) -> Vec<f64> {
    for k in 1..angles.len() {
        // As many whole turns as the loops this replaced took off, in one
        // step, so an angle a far Newton step left enormous cannot loop for
        // ever; one turn, the common case, is the same subtraction.
        let step = angles[k] - angles[k - 1];
        if step > PI {
            angles[k] -= 2. * PI * ((step - PI) / (2. * PI)).ceil();
        } else if -step > PI {
            angles[k] += 2. * PI * ((-step - PI) / (2. * PI)).ceil();
        }
    }
    angles
}

/// The height of the `k`-lobed component of a radial residual, `(angle,
/// residual)` samples round a closed outline: the least-squares fit of
/// `a cos(k angle) + b sin(k angle)`.
fn lobe(residuals: &[(f64, f64)], k: f64) -> f64 {
    let (mut rc, mut rs, mut cc, mut ss, mut cs) = (0., 0., 0., 0., 0.);
    for &(angle, residual) in residuals {
        let (sin, cos) = (k * angle).sin_cos();
        rc += residual * cos;
        rs += residual * sin;
        cc += cos * cos;
        ss += sin * sin;
        cs += cos * sin;
    }
    let det = cc * ss - cs * cs;
    if det.abs() <= 1e-9 {
        return 0.;
    }
    let a = (rc * ss - rs * cs) / det;
    let b = (cc * rs - cs * rc) / det;
    (a * a + b * b).sqrt()
}

/// Whether a closed outline's residual against its circle or ellipse has a
/// three- or four-lobed component (a rounded triangle's or square's
/// corners) taller than `MAX_ELLIPSE` of the band.
fn cornered(residuals: &[(f64, f64)], band: f64) -> bool {
    [3., 4.]
        .into_iter()
        .any(|k| lobe(residuals, k) > band * MAX_ELLIPSE)
}

/// A closed outline with no line and no corner whose samples all lie within
/// `band` of one ellipse, going once round it and leaving and arriving at
/// its start node along it: that ellipse in four quarter pieces (the affine
/// image of a whole circle's), from its start node moved onto it as a
/// whole circle's is. The engine traces an ellipse as a ring of pieces that
/// flatten and bulge like a circle's; a circle's fit refuses it (the two
/// lobes of `MAX_ELLIPSE`), so without this it kept that wobble. Only whole
/// outlines: a stretch of any smooth curve lies near some ellipse, and five
/// free parameters would redraw free-form curves. A ring whose residual has
/// the lobes of corners (`cornered`) is refused.
fn whole_ellipse(run: &[Edge], band: f64) -> Option<Vec<Edge>> {
    ellipse_fit(run, band, true)
}

/// `whole_ellipse`, with the corner test only when `corners`.
fn ellipse_fit(run: &[Edge], band: f64, corners: bool) -> Option<Vec<Edge>> {
    let n = run.len();
    if run.iter().any(|e| e.line) {
        return None;
    }
    for k in 0..n {
        let (Some(a), Some(b)) = (arriving(&run[(k + n - 1) % n]), leaving(&run[k])) else {
            return None;
        };
        if dot(a, b) < MAX_KINK.cos() {
            return None;
        }
    }
    let points = samples_of(run);
    let ellipse = fit_ellipse(&points)?;
    if ellipse.axes.0.min(ellipse.axes.1) < MIN_RADIUS {
        return None;
    }
    let mut parameters = Vec::with_capacity(points.len());
    for p in &points {
        let (t, off) = ellipse.nearest(*p);
        if !(off <= band) {
            return None;
        }
        parameters.push(t);
    }
    let parameters = unwrapped(parameters);
    let sweep = parameters[parameters.len() - 1] - parameters[0];
    let monotone = parameters
        .windows(2)
        .all(|w| (w[1] - w[0]) * sweep >= -1e-9);
    if !monotone || (sweep.abs() - 2. * PI).abs() > 1e-3 {
        return None;
    }
    // A rounded square or triangle within the band of an ellipse is still a
    // shape with corners (the lobes `as_arc` tests for).
    let (s, c) = ellipse.angle.sin_cos();
    let residuals: Vec<(f64, f64)> = points
        .iter()
        .zip(&parameters)
        .map(|(p, t)| {
            let (dx, dy) = (p.x - ellipse.centre.x, p.y - ellipse.centre.y);
            let (u, v) = (dx * c + dy * s, dy * c - dx * s);
            let outside = (u / ellipse.axes.0).powi(2) + (v / ellipse.axes.1).powi(2) > 1.;
            let (_, off) = ellipse.nearest(*p);
            (*t, if outside { off } else { -off })
        })
        .collect();
    if corners && cornered(&residuals, band) {
        return None;
    }
    let from = parameters[0];
    let along = |t: f64| normalized(scale(ellipse.velocity(t), sweep.signum()));
    for (own, ellipse_way) in [
        (leaving(&run[0]), along(from)),
        (arriving(&run[n - 1]), along(from)),
    ] {
        let (Some(own), Some(ellipse_way)) = (own, ellipse_way) else {
            return None;
        };
        if dot(own, ellipse_way) < MAX_TANGENT_ERROR.cos() {
            return None;
        }
    }
    let step = FRAC_PI_2 * sweep.signum();
    let kappa = 4. / 3. * (step / 4.).tan();
    let mut pieces = Vec::with_capacity(4);
    for k in 0..4 {
        let (t0, t1) = (from + step * k as f64, from + step * (k + 1) as f64);
        let (p0, p3) = (ellipse.at(t0), ellipse.at(t1));
        pieces.push(Edge {
            cubic: Cubic {
                points: [
                    p0,
                    add(p0, scale(ellipse.velocity(t0), kappa)),
                    sub(p3, scale(ellipse.velocity(t1), kappa)),
                    p3,
                ],
            },
            line: false,
            implicit: false,
        });
    }
    // The same node closes the ring, to the bit.
    pieces[3].cubic.points[3] = pieces[0].cubic.points[0];
    Some(pieces)
}

/// Where a cyclic run is best cut so straight stretches do not straddle the
/// cut: the first node whose two pieces are not jointly straight, or the
/// start when every pair is.
fn rotation_of(edges: &[Edge], band: f64) -> usize {
    let n = edges.len();
    (0..n)
        .find(|&k| !is_straight(&[edges[(k + n - 1) % n], edges[k]], band))
        .unwrap_or(0)
}

/// One greedy pass over pieces in order, as the run's pieces with every
/// straight stretch one line and every circular stretch drawn as arcs; the
/// pass's counts, and where its first regularized stretch ends.
struct Pass {
    out: Vec<Edge>,
    stats: RegularizeStats,
    first_end: Option<usize>,
}
impl Pass {
    /// Better is more pieces regularized, then fewer stretches (one arc
    /// rather than two on one bend), then fewer pieces.
    fn score(&self) -> (usize, std::cmp::Reverse<usize>, std::cmp::Reverse<usize>) {
        let s = &self.stats;
        (
            s.pieces_into_lines + s.pieces_into_arcs,
            std::cmp::Reverse(s.lines_made + s.arcs_made),
            std::cmp::Reverse(self.out.len()),
        )
    }
}

fn greedy(edges: &[Edge], band: f64, arcs: bool) -> Pass {
    let n = edges.len();
    let mut stats = RegularizeStats::default();
    let mut first_end = None;
    // Straight stretches of two or more pieces first, greedily from the
    // start (one bent piece is the straightener's business, and a gentle
    // bend is kept); then circular stretches over what is left, the same
    // way.
    let mut out: Vec<Edge> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && is_straight(&edges[i..=j], band) {
            j += 1;
        }
        if j - i >= 2 {
            out.push(Edge::line(edges[i].start(), edges[j - 1].end(), false));
            stats.lines_made += 1;
            stats.pieces_into_lines += j - i;
            first_end.get_or_insert(j);
            i = j;
            continue;
        }
        // Not straight: the longest circular stretch from here that turns
        // enough to be an arc.
        let mut best: Option<(usize, Circle, f64)> = None;
        let mut j = i + 2;
        while arcs && j <= n {
            match as_arc(&edges[i..j], band) {
                Some((circle, sweep)) => {
                    if sweep.abs() >= MIN_SWEEP {
                        best = Some((j, circle, sweep));
                    }
                    j += 1;
                }
                None => break,
            }
        }
        if let Some((j, circle, sweep)) = best {
            let (start, end) = (edges[i].start(), edges[j - 1].end());
            let from = (start.y - circle.centre.y).atan2(start.x - circle.centre.x);
            out.extend(arc_pieces(circle, from, sweep, start, end));
            stats.arcs_made += 1;
            stats.pieces_into_arcs += j - i;
            first_end.get_or_insert(j);
            i = j;
            continue;
        }
        out.push(edges[i]);
        i += 1;
    }
    Pass {
        out,
        stats,
        first_end,
    }
}

/// The run's pieces with every straight stretch one line and every
/// circular stretch drawn as arcs; a closed outline on one circle or one
/// ellipse is that whole shape.
fn regularize_run(
    run: &[Edge],
    cyclic: bool,
    band: f64,
    allow_arcs: bool,
    stats: &mut RegularizeStats,
) -> Vec<Edge> {
    let n = run.len();
    if n == 0 {
        return Vec::new();
    }
    if cyclic && n >= 2 && allow_arcs {
        // A closed outline on one circle: the whole circle, from its own start.
        if let Some((circle, sweep)) = as_arc(run, band) {
            if (sweep.abs() - 2. * PI).abs() < 1e-3 {
                let start = run[0].start();
                let from = (start.y - circle.centre.y).atan2(start.x - circle.centre.x);
                let node = on_circle(circle, from);
                let whole = 2. * PI * sweep.signum();
                stats.circles_made += 1;
                stats.pieces_into_arcs += n;
                return arc_pieces(circle, from, whole, node, node);
            }
        }
        if let Some(pieces) = whole_ellipse(run, band) {
            stats.ellipses_made += 1;
            stats.pieces_into_ellipses += n;
            return pieces;
        }
    }
    // A ring within the band of one circle or ellipse but for its corners (a
    // small rounded square) is no circle, and no stretch of it is an arc
    // either: noded at its corners, three of a rounded square's four pieces
    // passed as a 270-degree arc (the corner test needs the whole ring) and
    // it became three quarters of a circle and one flat side (round three of
    // the Opus 5.5 review). Its straight stretches still become lines.
    let arcs = allow_arcs
        && !(cyclic
            && n >= 2
            && (arc_fit(run, band, false)
                .is_some_and(|(_, sweep)| (sweep.abs() - 2. * PI).abs() < 1e-3)
                || ellipse_fit(run, band, false).is_some()));
    if !cyclic {
        let pass = greedy(run, band, arcs);
        stats.absorb(pass.stats);
        return pass.out;
    }
    // No stretch runs over the cut of a closed outline, and its canonical
    // start (the smallest node key, the leftmost node) often lies partway
    // along a bend: cut there, a bend became two arcs on two circles, or an
    // arc and a leftover piece. So the pass is also made from the first
    // corner, and from where the first pass's first stretch ended (a node
    // where a line or an arc stopped fitting, so the stretch that ran over
    // the cut is whole from there); the best pass wins, the first on a tie.
    let rotated = |k: usize| -> Vec<Edge> { (0..n).map(|i| run[(k + i) % n]).collect() };
    let first = rotation_of(run, band);
    let pass = greedy(&rotated(first), band, arcs);
    let mut cuts = Vec::new();
    if let Some(corner) = (0..n).find(|&k| is_corner(&run[(k + n - 1) % n], &run[k])) {
        cuts.push(corner);
    }
    if let Some(end) = pass.first_end {
        cuts.push((first + end) % n);
    }
    let mut best = pass;
    let mut tried = vec![first];
    for cut in cuts {
        if tried.contains(&cut) {
            continue;
        }
        tried.push(cut);
        let pass = greedy(&rotated(cut), band, arcs);
        if pass.score() > best.score() {
            best = pass;
        }
    }
    stats.absorb(best.stats);
    best.out
}

/// The document with its straight runs made lines and its circular runs
/// made arcs; every path's data rewritten, everything else verbatim.
pub fn regularize_svg(
    svg: &str,
    options: RegularizeOptions,
) -> Result<(String, RegularizeStats), String> {
    regularize_svg_checked(svg, options, None)
}

/// `regularize_svg`, with every run that became arcs held against the
/// source's pixels when `check` gives them (`PixelCheck`).
pub fn regularize_svg_checked(
    svg: &str,
    options: RegularizeOptions,
    check: Option<PixelCheck>,
) -> Result<(String, RegularizeStats), String> {
    options.validate()?;
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let mut stats = RegularizeStats::default();
    let junctions = junction_keys(&paths);
    let is_junction = |p: Point| junctions.contains(&key(p));
    let mut cache: HashMap<Vec<EdgeKey>, Vec<Edge>> = HashMap::new();
    for path in &mut paths {
        for subpath in path.iter_mut() {
            stats.pieces_before += subpath.edges.len();
            let mut rebuilt = Vec::new();
            let runs: Vec<Run> = runs_of(subpath, &is_junction);
            for run in &runs {
                let (run_key, edges, reversed) = canonical(run);
                let done = match cache.get(&run_key) {
                    Some(done) => done.clone(),
                    None => {
                        let mut own = RegularizeStats::default();
                        let mut done =
                            regularize_run(&edges, run.cyclic, options.band, true, &mut own);
                        let curved = own.arcs_made + own.circles_made + own.ellipses_made;
                        if let (Some(check), true) = (check, curved > 0) {
                            let mut plain_stats = RegularizeStats::default();
                            let plain = regularize_run(
                                &edges,
                                run.cyclic,
                                options.band,
                                false,
                                &mut plain_stats,
                            );
                            if pixels::against(check, &done, &plain) {
                                done = plain;
                                own = plain_stats;
                                own.arcs_refused += 1;
                            }
                        }
                        stats.absorb(own);
                        cache.insert(run_key, done.clone());
                        done
                    }
                };
                if reversed {
                    rebuilt.extend(done.iter().rev().map(Edge::reversed));
                } else {
                    rebuilt.extend(done);
                }
            }
            stats.pieces_after += rebuilt.len();
            subpath.edges = rebuilt;
        }
    }
    Ok((splice(svg, &ranges, &paths), stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simplify::lerp;
    use std::f64::consts::FRAC_PI_4;

    fn wrap(paths: &[(&str, String)]) -> String {
        let mut svg = String::from(
            "<svg width=\"200pt\" height=\"200pt\" viewBox=\"0 0 200 200\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n",
        );
        for (fill, d) in paths {
            svg.push_str(&format!(
                "<g id=\"{fill}ff\">\n<path fill=\"{fill}\" opacity=\"1.00\" d=\"{d}\" />\n</g>\n"
            ));
        }
        svg.push_str("</svg>\n");
        svg
    }

    /// Path data for curved pieces through nodes at the given angles and
    /// radii about a centre, with handles tangent to the circle.
    fn arc_data(centre: Point, nodes: &[(f64, f64)]) -> String {
        let mut d = String::new();
        for (i, &(angle, radius)) in nodes.iter().enumerate() {
            let p = Point {
                x: centre.x + radius * angle.cos(),
                y: centre.y + radius * angle.sin(),
            };
            if i == 0 {
                d.push_str(&format!(" M {:.2} {:.2}", p.x, p.y));
                continue;
            }
            let (a0, r0) = nodes[i - 1];
            let q = Point {
                x: centre.x + r0 * a0.cos(),
                y: centre.y + r0 * a0.sin(),
            };
            let kappa = 4. / 3. * ((angle - a0) / 4.).tan() * (r0 + radius) / 2.;
            let h1 = (q.x - kappa * a0.sin(), q.y + kappa * a0.cos());
            let h2 = (p.x + kappa * angle.sin(), p.y - kappa * angle.cos());
            d.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                h1.0, h1.1, h2.0, h2.1, p.x, p.y
            ));
        }
        d
    }

    /// A closed ring of `n` pieces whose nodes bulge in and out by `noise`.
    fn ring(centre: Point, r: f64, n: usize, noise: f64, reverse: bool) -> String {
        let mut nodes: Vec<(f64, f64)> = (0..=n)
            .map(|k| {
                let bump = if k % 2 == 0 { noise } else { -noise };
                (2. * PI * k as f64 / n as f64, r + bump)
            })
            .collect();
        if reverse {
            nodes.reverse();
        }
        arc_data(centre, &nodes) + " Z"
    }

    #[test]
    fn a_wobbly_straight_run_becomes_one_line_and_bends_are_kept() {
        // The top edge: five pieces wobbling 0.3 px about y = 10. Then a
        // 40-degree bend of 60 px radius in two pieces (an arc), then a
        // 12-degree bend of 100 px radius in two pieces: within the band of
        // its chord, but its bulge says it is curved, so not a line, and too
        // little turn to be an arc, so left alone.
        let top = " M 10.00 10.00 C 15.00 10.30 20.00 10.30 25.00 9.70 C 30.00 9.70 35.00 10.30 40.00 10.20 C 45.00 10.20 50.00 9.80 55.00 9.90 C 60.00 9.90 65.00 10.30 70.00 10.10 C 75.00 10.10 80.00 9.80 85.00 10.00";
        let c1 = Point { x: 85., y: 70. };
        let bend = arc_data(
            c1,
            &[
                (-FRAC_PI_2, 60.),
                (-FRAC_PI_2 + 0.35, 60.),
                (-FRAC_PI_2 + 0.70, 60.),
            ],
        );
        let bend = bend.trim_start().trim_start_matches(|c: char| c != 'C');
        let end_of_bend = Point {
            x: c1.x + 60. * (-FRAC_PI_2 + 0.70).cos(),
            y: c1.y + 60. * (-FRAC_PI_2 + 0.70).sin(),
        };
        // The gentle bend continues from there, on a circle whose start
        // point is that node.
        let a0 = 0.5f64;
        let c2 = Point {
            x: end_of_bend.x - 100. * a0.cos(),
            y: end_of_bend.y - 100. * a0.sin(),
        };
        let gentle = arc_data(c2, &[(a0, 100.), (a0 + 0.105, 100.), (a0 + 0.21, 100.)]);
        let gentle = gentle.trim_start().trim_start_matches(|c: char| c != 'C');
        let d = format!("{top} {bend} {gentle} L 10.00 150.00 Z");
        let svg = wrap(&[("#000000", d)]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(stats.lines_made, 1, "{out}");
        assert_eq!(stats.pieces_into_lines, 5);
        assert!(out.contains("L 10.00 10.00 L 85.00 10.00"), "{out}");
        assert_eq!((stats.arcs_made, stats.pieces_into_arcs), (1, 2), "{out}");
        assert_eq!(stats.pieces_after, 1 + 1 + 2 + 2, "{out}");
        // Off means off.
        let (same, stats) = regularize_svg(&svg, RegularizeOptions { band: 0. }).unwrap();
        assert_eq!(stats.lines_made + stats.arcs_made + stats.circles_made, 0);
        assert!(same.contains("C 15.00 10.30"));
        assert!(regularize_svg(&svg, RegularizeOptions { band: -1. }).is_err());
    }

    #[test]
    fn a_shared_wobbly_ring_becomes_the_same_four_arc_circle_on_both_sides() {
        let centre = Point { x: 100., y: 100. };
        let outer = ring(centre, 40., 12, 0.3, false);
        let inner = ring(centre, 40., 12, 0.3, true);
        let svg = wrap(&[
            (
                "#ff0000",
                format!(" M 0.00 0.00 L 200.00 0.00 L 200.00 200.00 L 0.00 200.00 Z{inner}"),
            ),
            ("#0000ff", outer),
        ]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(stats.circles_made, 1, "{out}");
        assert_eq!(stats.pieces_into_arcs, 12);
        assert_eq!(stats.pieces_after, 4 + 4 + 4);
        let (_, paths) = parse_all_paths(&out).unwrap();
        let hole = &paths[0][1];
        let disc = &paths[1][0];
        assert_eq!((hole.edges.len(), disc.edges.len()), (4, 4));
        for edge in &disc.edges {
            for p in [edge.start(), edge.end()] {
                assert!((distance(p, centre) - 40.).abs() < 0.4, "{p:?}");
            }
        }
        // The hole is the disc reversed, node for node.
        for edge in &disc.edges {
            assert!(
                hole.edges.iter().any(|h| key(h.end()) == key(edge.start())),
                "{:?} missing from the hole",
                edge.start()
            );
        }
    }

    #[test]
    fn junction_nodes_stay_where_they_are_and_corners_never_become_arcs() {
        // Two fills share a wobbly straight edge that ends at nodes a third
        // fill also touches: the line runs exactly between those nodes. The
        // square corners of lines around them stay corners.
        let top = " M 20.00 10.00 L 80.00 10.00 L 80.00 40.00 C 70.00 39.80 60.00 40.10 50.00 40.10 C 40.00 39.70 30.00 40.30 20.00 40.00 Z".to_owned();
        let bottom = " M 20.00 40.00 C 30.00 40.30 40.00 39.70 50.00 40.10 C 60.00 40.10 70.00 39.80 80.00 40.00 L 80.00 70.00 L 20.00 70.00 Z".to_owned();
        let side =
            " M 80.00 10.00 L 120.00 10.00 L 120.00 70.00 L 80.00 70.00 L 80.00 40.00 Z".to_owned();
        let svg = wrap(&[("#ff0000", top), ("#00ff00", bottom), ("#0000ff", side)]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(stats.lines_made, 1, "{out}");
        assert_eq!(stats.arcs_made + stats.circles_made, 0, "{out}");
        assert!(out.contains("L 80.00 40.00 L 20.00 40.00"), "{out}");
        assert!(out.contains("M 20.00 40.00 L 80.00 40.00"), "{out}");
        assert_eq!(stats.pieces_after, stats.pieces_before - 2);
    }

    #[test]
    fn a_run_leaning_to_one_side_of_its_chord_is_not_drawn_as_the_chord() {
        // The top edge lies on y = 10 from x = 10 to 82 in six pieces that
        // wobble evenly about it; the corner it runs into sits 0.4 px lower,
        // at (85, 10.4). Every sample is within the band of the chord from
        // (10, 10) to that corner, but they all lean to one side of it, and
        // drawing the chord would tilt the whole edge (MAX_LEAN). The six
        // pieces on y = 10 become one line; the short piece into the corner
        // stays.
        let mut top = String::from(" M 10.00 10.00");
        for k in 0..6 {
            let x = 10. + 12. * k as f64;
            let bump = if k % 2 == 0 { 0.1 } else { -0.1 };
            top.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} 10.00",
                x + 4.,
                10. + bump,
                x + 8.,
                10. + bump,
                x + 12.
            ));
        }
        let d =
            format!("{top} C 83.00 10.10 84.00 10.25 85.00 10.40 L 85.00 60.00 L 10.00 60.00 Z");
        let svg = wrap(&[("#000000", d)]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!((stats.lines_made, stats.pieces_into_lines), (1, 6), "{out}");
        assert!(
            out.contains("L 10.00 10.00 L 82.00 10.00 C 83.00 10.10"),
            "{out}"
        );
        assert!(!out.contains("L 85.00 10.40"), "{out}");
    }

    #[test]
    fn a_rounded_corner_whose_ends_leave_off_the_circle_is_not_an_arc() {
        // A quarter turn of radius 16 in three pieces whose nodes lie on the
        // circle, but whose first and last handles leave 20 degrees off its
        // tangent: every sample stays within the band of the circle, yet an
        // arc drawn from it would kink against the edges on either side
        // (MAX_TANGENT_ERROR).
        let centre = Point { x: 50., y: 50. };
        let r = 16.;
        let step = FRAC_PI_2 / 3.;
        let kappa = 4. / 3. * (step / 4.).tan() * r;
        let turn = |v: Point, angle: f64| Point {
            x: v.x * angle.cos() - v.y * angle.sin(),
            y: v.x * angle.sin() + v.y * angle.cos(),
        };
        let off = 20f64.to_radians();
        let mut d = String::from(" M 50.00 34.00");
        for k in 0..3 {
            let (a0, a1) = (
                -FRAC_PI_2 + step * k as f64,
                -FRAC_PI_2 + step * (k + 1) as f64,
            );
            let tangent = |a: f64| Point {
                x: -a.sin(),
                y: a.cos(),
            };
            let (p0, p3) = (
                on_circle(Circle { centre, radius: r }, a0),
                on_circle(Circle { centre, radius: r }, a1),
            );
            // The outer handles turn away from the centre.
            let t0 = if k == 0 {
                turn(tangent(a0), -off)
            } else {
                tangent(a0)
            };
            let t1 = if k == 2 {
                turn(tangent(a1), off)
            } else {
                tangent(a1)
            };
            let (p1, p2) = (add(p0, scale(t0, kappa)), sub(p3, scale(t1, kappa)));
            d.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                p1.x, p1.y, p2.x, p2.y, p3.x, p3.y
            ));
        }
        d.push_str(" L 66.00 80.00 L 20.00 80.00 L 20.00 34.00 Z");
        let svg = wrap(&[("#000000", d)]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(stats.arcs_made + stats.circles_made, 0, "{out}");
        assert_eq!(stats.pieces_after, stats.pieces_before, "{out}");
    }

    #[test]
    fn an_ellipse_close_to_a_circle_does_not_become_one() {
        // An ellipse 5% taller than wide, in four quarter pieces: its points
        // stay within the band of a circle of radius 20.5, but the radial
        // residual has the two lobes of an ellipse (MAX_ELLIPSE), so the
        // outline keeps its height instead of becoming that circle.
        let (a, b) = (20., 21.);
        let k = 4. / 3. * (2f64.sqrt() - 1.);
        let d = format!(
            " M 120.00 100.00 C 120.00 {:.2} {:.2} 121.00 100.00 121.00 C {:.2} 121.00 80.00 {:.2} 80.00 100.00 C 80.00 {:.2} {:.2} 79.00 100.00 79.00 C {:.2} 79.00 120.00 {:.2} 120.00 100.00 Z",
            100. + k * b,
            100. + k * a,
            100. - k * a,
            100. + k * b,
            100. - k * b,
            100. - k * a,
            100. + k * a,
            100. - k * b,
        );
        let svg = wrap(&[("#000000", d)]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(stats.circles_made, 0, "{out}");
        let (_, paths) = parse_all_paths(&out).unwrap();
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for edge in &paths[0][0].edges {
            for step in 0..=32 {
                let y = edge.cubic.evaluate(step as f64 / 32.).y;
                (low, high) = (low.min(y), high.max(y));
            }
        }
        // A circle of radius 20.5 would span 41 px; the ellipse spans 42.
        assert!(high - low > 41.6, "{low}..{high}: {out}");
        assert_eq!(stats.ellipses_made, 1, "{out}");
    }

    #[test]
    fn a_small_rounded_square_does_not_become_a_circle() {
        // An 8 px square with corners of radius 2 about (100, 100), four
        // corner arcs joined smoothly by four straight cubics, starting at
        // the middle of a corner where the square runs the way the circle
        // does (so the tangent guard passes): every point lies within 0.43
        // px of a circle of radius 4.4, inside the band, but the residual's
        // four lobes are its corners (round two of the Opus 5.5 review).
        let arc = |cx: f64, cy: f64, from: f64, sweep: f64| {
            let k = 4. / 3. * (sweep / 4.).tan() * 2.;
            let (a0, a1) = (from, from + sweep);
            let p = |a: f64| (cx + 2. * a.cos(), cy + 2. * a.sin());
            let (p0, p3) = (p(a0), p(a1));
            format!(
                " C {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                p0.0 - k * a0.sin(),
                p0.1 + k * a0.cos(),
                p3.0 + k * a1.sin(),
                p3.1 - k * a1.cos(),
                p3.0,
                p3.1
            )
        };
        let side = |from: (f64, f64), to: (f64, f64)| {
            let at = |t: f64| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            let (a, b) = (at(1. / 3.), at(2. / 3.));
            format!(
                " C {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                a.0, a.1, b.0, b.1, to.0, to.1
            )
        };
        let quarter = FRAC_PI_2 / 2.;
        let d = format!(
            " M {:.4} {:.4}{}{}{}{}{}{}{}{}{} Z",
            102. + 2. * quarter.cos(),
            102. + 2. * quarter.sin(),
            arc(102., 102., quarter, quarter),
            side((102., 104.), (98., 104.)),
            arc(98., 102., FRAC_PI_2, FRAC_PI_2),
            side((96., 102.), (96., 98.)),
            arc(98., 98., PI, FRAC_PI_2),
            side((98., 96.), (102., 96.)),
            arc(102., 98., 1.5 * PI, FRAC_PI_2),
            side((104., 98.), (104., 102.)),
            arc(102., 102., 0., quarter),
        );
        let svg = wrap(&[("#000000", d)]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!((stats.circles_made, stats.ellipses_made), (0, 0), "{out}");
        // The fits themselves, whatever the run search offers them.
        let (_, paths) = parse_all_paths(&svg).unwrap();
        let edges = &paths[0][0].edges;
        assert!(as_arc(edges, 0.8).is_none());
        assert!(whole_ellipse(edges, 0.8).is_none());
    }

    #[test]
    fn a_rounded_square_noded_at_its_corners_gets_no_arc() {
        // The same 8 px rounded square traced as four pieces from corner
        // apex to corner apex, each leaving and arriving the way the circle
        // runs: the whole ring is refused for its corners, and three of its
        // four pieces (270 degrees, too short for the corner test) had
        // passed as one arc, leaving a lopsided blob of three quarters of a
        // circle and one flat side (round three of the Opus 5.5 review).
        let apex = |a: f64| {
            let corner = (100. + 2. * a.cos().signum(), 100. + 2. * a.sin().signum());
            (corner.0 + 2. * a.cos(), corner.1 + 2. * a.sin())
        };
        // Handles along the circle's tangent, long enough that each piece
        // reaches the side's line at its middle.
        let handle = 1.105;
        let mut d = String::new();
        for k in 0..4 {
            let (a0, a1) = (
                FRAC_PI_2 / 2. + FRAC_PI_2 * k as f64,
                FRAC_PI_2 / 2. + FRAC_PI_2 * (k + 1) as f64,
            );
            let (p0, p3) = (apex(a0), apex(a1));
            if k == 0 {
                d.push_str(&format!(" M {:.4} {:.4}", p0.0, p0.1));
            }
            d.push_str(&format!(
                " C {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                p0.0 - handle * a0.sin(),
                p0.1 + handle * a0.cos(),
                p3.0 + handle * a1.sin(),
                p3.1 - handle * a1.cos(),
                p3.0,
                p3.1
            ));
        }
        d.push_str(" Z");
        let svg = wrap(&[("#000000", d)]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(
            (stats.circles_made, stats.ellipses_made, stats.arcs_made),
            (0, 0, 0),
            "{out}"
        );
    }

    /// Path data for a rounded rectangle 80 by 50 about (100, 100) with
    /// corners of radius 16, turned by `turn`: each corner four pieces on its
    /// circle, each side three that wobble 0.15 px about it, all joined
    /// smoothly.
    fn round_rect(turn: f64) -> String {
        let (w, h, r) = (40., 25., 16.);
        let place = |p: Point| Point {
            x: 100. + p.x * turn.cos() - p.y * turn.sin(),
            y: 100. + p.x * turn.sin() + p.y * turn.cos(),
        };
        let corners = [
            (-w + r, -h + r, PI),
            (w - r, -h + r, 1.5 * PI),
            (w - r, h - r, 0.),
            (-w + r, h - r, FRAC_PI_2),
        ];
        let step = FRAC_PI_2 / 4.;
        let kappa = 4. / 3. * (step / 4.).tan() * r;
        let tangent = |a: f64| Point {
            x: -a.sin(),
            y: a.cos(),
        };
        let mut pieces: Vec<[Point; 4]> = Vec::new();
        for (k, &(x, y, from)) in corners.iter().enumerate() {
            let circle = Circle {
                centre: Point { x, y },
                radius: r,
            };
            for s in 0..4 {
                let (a0, a1) = (from + step * s as f64, from + step * (s + 1) as f64);
                let (p0, p3) = (on_circle(circle, a0), on_circle(circle, a1));
                pieces.push([
                    p0,
                    add(p0, scale(tangent(a0), kappa)),
                    sub(p3, scale(tangent(a1), kappa)),
                    p3,
                ]);
            }
            let (x, y, next) = corners[(k + 1) % 4];
            let a = pieces[pieces.len() - 1][3];
            let b = on_circle(
                Circle {
                    centre: Point { x, y },
                    radius: r,
                },
                next,
            );
            let normal = normalized(Point {
                x: a.y - b.y,
                y: b.x - a.x,
            })
            .unwrap();
            for s in 0..3 {
                let (p0, p3) = (lerp(a, b, s as f64 / 3.), lerp(a, b, (s + 1) as f64 / 3.));
                let bump = scale(normal, if s == 1 { -0.15 } else { 0.15 });
                pieces.push([
                    p0,
                    add(lerp(p0, p3, 1. / 3.), bump),
                    add(lerp(p0, p3, 2. / 3.), bump),
                    p3,
                ]);
            }
        }
        let first = place(pieces[0][0]);
        let mut d = format!(" M {:.2} {:.2}", first.x, first.y);
        for piece in &pieces {
            let [_, p1, p2, p3] = piece.map(place);
            d.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                p1.x, p1.y, p2.x, p2.y, p3.x, p3.y
            ));
        }
        d + " Z"
    }

    #[test]
    fn a_closed_outline_cut_partway_round_a_corner_keeps_that_corner_one_arc() {
        // Turned 45 degrees, the leftmost node, where the canonical walk of
        // the outline starts, is the middle of a corner. Cut there, that
        // corner came out as two 45-degree arcs fitted to two circles; the
        // walk is cut where the first pass's first arc ended instead.
        let svg = wrap(&[("#000000", round_rect(-FRAC_PI_4))]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(
            (stats.lines_made, stats.pieces_into_lines),
            (4, 12),
            "{out}"
        );
        assert_eq!((stats.arcs_made, stats.pieces_into_arcs), (4, 16), "{out}");
        assert_eq!(stats.circles_made + stats.ellipses_made, 0, "{out}");
        // Untilted, the walk starts where a side meets a corner, and the
        // outline gets the same four lines and four arcs.
        let (out, stats) = regularize_svg(
            &wrap(&[("#000000", round_rect(0.))]),
            RegularizeOptions::default(),
        )
        .unwrap();
        assert_eq!((stats.lines_made, stats.arcs_made), (4, 4), "{out}");
    }

    /// Path data for a closed ellipse about (100, 100), semi-axes 30 and 18,
    /// turned 20 degrees, in 12 pieces whose nodes sit alternately 1.2%
    /// outside and inside it (0.36 px on the long axis), each piece's
    /// handles along the ellipse's own tangent; `corner` turns the first
    /// node's outgoing handle 40 degrees.
    fn wobbly_ellipse(reverse: bool, corner: bool) -> String {
        let ellipse = Ellipse {
            centre: Point { x: 100., y: 100. },
            axes: (30., 18.),
            angle: 20f64.to_radians(),
        };
        let n = 12;
        let step = 2. * PI / n as f64;
        let kappa = 4. / 3. * (step / 4.).tan();
        let node = |k: usize| {
            let t = step * (k % n) as f64;
            let grow = if k.is_multiple_of(2) { 1.012 } else { 0.988 };
            let at = add(
                ellipse.centre,
                scale(sub(ellipse.at(t), ellipse.centre), grow),
            );
            (at, scale(ellipse.velocity(t), grow * kappa))
        };
        let mut pieces: Vec<[Point; 4]> = (0..n)
            .map(|k| {
                let ((p0, v0), (p3, v3)) = (node(k), node(k + 1));
                [p0, add(p0, v0), sub(p3, v3), p3]
            })
            .collect();
        if corner {
            let [p0, p1, ..] = pieces[0];
            let v = sub(p1, p0);
            let turn = 40f64.to_radians();
            pieces[0][1] = add(
                p0,
                Point {
                    x: v.x * turn.cos() - v.y * turn.sin(),
                    y: v.x * turn.sin() + v.y * turn.cos(),
                },
            );
        }
        if reverse {
            pieces = pieces
                .iter()
                .rev()
                .map(|&[a, b, c, d]| [d, c, b, a])
                .collect();
        }
        let mut d = format!(" M {:.2} {:.2}", pieces[0][0].x, pieces[0][0].y);
        for [_, p1, p2, p3] in &pieces {
            d.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                p1.x, p1.y, p2.x, p2.y, p3.x, p3.y
            ));
        }
        d + " Z"
    }

    #[test]
    fn a_shared_wobbly_ellipse_becomes_the_same_true_ellipse_on_both_sides() {
        let svg = wrap(&[
            (
                "#ff0000",
                format!(
                    " M 0.00 0.00 L 200.00 0.00 L 200.00 200.00 L 0.00 200.00 Z{}",
                    wobbly_ellipse(true, false)
                ),
            ),
            ("#0000ff", wobbly_ellipse(false, false)),
        ]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(stats.circles_made, 0, "{out}");
        assert_eq!((stats.ellipses_made, stats.pieces_into_ellipses), (1, 12));
        assert_eq!(stats.pieces_after, 4 + 4 + 4, "{out}");
        let (_, paths) = parse_all_paths(&out).unwrap();
        let (hole, disc) = (&paths[0][1], &paths[1][0]);
        assert_eq!((hole.edges.len(), disc.edges.len()), (4, 4));
        // The wobble is gone: every point of the new outline lies on the
        // true ellipse within 0.1 px, where the traced nodes strayed 0.36.
        let truth = Ellipse {
            centre: Point { x: 100., y: 100. },
            axes: (30., 18.),
            angle: 20f64.to_radians(),
        };
        for edge in &disc.edges {
            for step in 0..=16 {
                let p = edge.cubic.evaluate(step as f64 / 16.);
                assert!(truth.nearest(p).1 < 0.1, "{p:?} off the ellipse: {out}");
            }
        }
        // The hole is the disc reversed, node for node.
        for edge in &disc.edges {
            assert!(
                hole.edges.iter().any(|h| key(h.end()) == key(edge.start())),
                "{:?} missing from the hole",
                edge.start()
            );
        }
        // A corner in the outline says it is not an ellipse.
        let svg = wrap(&[("#0000ff", wobbly_ellipse(false, true))]);
        let (out, stats) = regularize_svg(&svg, RegularizeOptions::default()).unwrap();
        assert_eq!(stats.ellipses_made + stats.circles_made, 0, "{out}");
    }

    /// A dark shape on white, each pixel covered by 8 x 8 samples of `inside`.
    fn coverage_picture(size: usize, inside: impl Fn(f64, f64) -> bool) -> crate::raster::Raster {
        let pixels = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f64, (i / size) as f64);
                let hits = (0..64)
                    .filter(|k| {
                        inside(
                            x + (k % 8) as f64 / 8. + 1. / 16.,
                            y + (k / 8) as f64 / 8. + 1. / 16.,
                        )
                    })
                    .count();
                let v = (255. * (1. - hits as f64 / 64.)).round() as u8;
                crate::raster::Rgba([v, v, v, 255])
            })
            .collect();
        crate::raster::Raster {
            width: size,
            height: size,
            pixels,
        }
    }

    #[test]
    fn a_bowl_that_is_no_circle_keeps_its_curves_where_the_pixels_say_so() {
        // A D whose bowl is half a superellipse (exponent 2.3), within the
        // band of a circle but 0.47 px out from it at its diagonals, as a
        // letter's bowl between its stem's corners: regularize alone draws
        // the bowl as arcs, the anti-aliased pixels refuse them. The same D
        // with a true half circle traced with a wobble keeps its arcs.
        let centre = Point { x: 40., y: 40. };
        let power = 2.3;
        let superellipse =
            |a: f64| 20. / (a.cos().abs().powf(power) + a.sin().abs().powf(power)).powf(1. / power);
        let d_shape = |radius: &dyn Fn(usize, f64) -> f64| {
            let nodes: Vec<(f64, f64)> = (0..=8)
                .map(|k| {
                    let a = -FRAC_PI_2 + PI * k as f64 / 8.;
                    (a, radius(k, a))
                })
                .collect();
            wrap(&[("#000000", arc_data(centre, &nodes) + " Z")])
        };
        let bowl = d_shape(&|_, a| superellipse(a));
        let (_, plain) = regularize_svg(&bowl, RegularizeOptions::default()).unwrap();
        assert!(plain.arcs_made > 0, "{plain:?}");
        let check = |source| {
            Some(PixelCheck {
                source,
                anti_aliased: true,
            })
        };
        let picture = coverage_picture(80, |x, y| {
            x >= 40. && ((x - 40.) / 20.).powf(power) + ((y - 40.).abs() / 20.).powf(power) < 1.
        });
        let (out, checked) =
            regularize_svg_checked(&bowl, RegularizeOptions::default(), check(&picture)).unwrap();
        assert_eq!((checked.arcs_refused, checked.arcs_made), (1, 0), "{out}");
        let wobbly = d_shape(&|k, _| if k % 2 == 0 { 20.15 } else { 19.85 });
        let circle = coverage_picture(80, |x, y| x >= 40. && (x - 40.).hypot(y - 40.) < 20.);
        let (out, kept) =
            regularize_svg_checked(&wobbly, RegularizeOptions::default(), check(&circle)).unwrap();
        assert_eq!(kept.arcs_refused, 0, "{out}");
        assert!(kept.arcs_made > 0, "{out}");
    }
}
