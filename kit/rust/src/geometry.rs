//! Points and cubics shared by the port and the owned passes, and the ridge
//! of the original's fits. The fixed-endpoint cubic fit of 0x0049D360 (the
//! 2N-by-4 Bernstein design matrix, the ridge 1e-5 on the normal matrix and
//! the `dgesv` solve) is `recovered_fit::fit_cubic`; its 64-case comparison
//! against an independent dense solver is kept here with the fixture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cubic {
    pub points: [Point; 4],
}

impl Cubic {
    /// Bernstein evaluation in the order of 0x00479E10: the four weights as
    /// the original forms them, each control point scaled, then the sum
    /// `((P0 + P1) + P2) + P3`.
    pub fn evaluate(self, t: f64) -> Point {
        let u = 1. - t;
        let t2 = t * t;
        let w3 = t2 * t;
        let w2 = (u * t2) * 3.;
        let u2 = u * u;
        let w1 = (u2 * t) * 3.;
        let w0 = u2 * u;
        let scale = |p: Point, w: f64| Point {
            x: p.x * w,
            y: p.y * w,
        };
        let add = |a: Point, b: Point| Point {
            x: a.x + b.x,
            y: a.y + b.y,
        };
        let [p0, p1, p2, p3] = self.points;
        add(
            add(add(scale(p0, w0), scale(p1, w1)), scale(p2, w2)),
            scale(p3, w3),
        )
    }
}

/// The distance between two points.
pub fn dist(a: Point, b: Point) -> f64 {
    (b.x - a.x).hypot(b.y - a.y)
}

/// The distance from `p` to the segment from `a` to `b`.
pub fn to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-18 {
        0.
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0., 1.)
    };
    (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
}

/// The shoelace sum of a polygon (positive for the engine's outer outlines,
/// which run clockwise on screen).
pub fn turning(polygon: &[Point]) -> f64 {
    (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            a.x * b.y - b.x * a.y
        })
        .sum()
}

/// Ridge strength read from the binary double at 0x008DE7E0.
pub const LEGACY_RIDGE: f64 = 1e-5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovered_fit::fit_cubic;
    #[test]
    fn endpoints_are_exact_and_midpoint_obeys_bernstein_weights() {
        let curve = Cubic {
            points: [
                Point { x: 0., y: 0. },
                Point { x: 0., y: 4. },
                Point { x: 4., y: 4. },
                Point { x: 4., y: 0. },
            ],
        };
        assert_eq!(curve.evaluate(0.), curve.points[0]);
        assert_eq!(curve.evaluate(1.), curve.points[3]);
        assert_eq!(curve.evaluate(0.5), Point { x: 2., y: 3. });
    }
    #[test]
    fn fits_match_independent_dense_four_unknown_solver() {
        let mut cases = 0;
        for line in include_str!("../fixtures/curve-golden.csv").lines().skip(1) {
            let v: Vec<f64> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let point = |i| Point {
                x: v[i],
                y: v[i + 1],
            };
            let samples: Vec<_> = v[9..]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| (c[0], Point { x: c[1], y: c[2] }))
                .collect();
            let (curve, _) = fit_cubic(point(1), point(3), &samples).unwrap();
            for (actual, expected) in [
                curve.points[1].x,
                curve.points[1].y,
                curve.points[2].x,
                curve.points[2].y,
            ]
            .iter()
            .zip(&v[5..9])
            {
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "case {}: {actual} != {expected}",
                    v[0]
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 64);
    }
    #[test]
    fn invalid_inputs_are_rejected() {
        let p = Point { x: 0., y: 0. };
        let nan = Point { x: f64::NAN, y: 0. };
        assert!(fit_cubic(nan, p, &[(0.3, p), (0.7, p)]).is_err());
        assert!(fit_cubic(p, p, &[(0.3, p), (0.7, nan)]).is_err());
        assert!(fit_cubic(p, p, &[(0.3, p), (0.7, p), (2., p)]).is_err());
    }
}
