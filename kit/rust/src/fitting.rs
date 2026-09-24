//! Small fitting primitives recovered from explicit instruction spans.
//! See docs/FITTING.md for addresses, caller contracts, and unresolved behavior.
//! Finite-value/range checks and safe slice interfaces are new Rust API choices.
use crate::geometry::{Cubic, Point, LEGACY_RIDGE};

/// Outputs of 0x0049D8A0, ordered [P1.x, P1.y, P2.x, P2.y].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicFitDerivatives {
    /// Gradient of the unregularized sum of squared sample residuals.
    pub gradient: [f64; 4],
    /// Row-major Hessian of that sum, plus LEGACY_RIDGE on the diagonal.
    /// The original symmetric matrix is column-major; the entries coincide.
    pub damped_hessian: [[f64; 4]; 4],
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// Reconstruct 0x0049D8A0: g = 2 Aᵀ(C(t)-sample), H = 2 AᵀA + 1e-5 I.
/// Endpoints and supplied parameters are fixed. No fit is solved or curve mutated.
/// Unlike the direct fit of 0x0049D360 (`recovered_fit::fit_cubic`), whose
/// solve carries the ridge, the ridge does NOT contribute to the gradient.
/// Empty samples yield zero gradient and the diagonal damping matrix.
pub fn cubic_fit_derivatives(
    curve: Cubic,
    samples: &[(f64, Point)],
) -> Result<CubicFitDerivatives, String> {
    if !curve.points.into_iter().all(finite) {
        return Err("Non-finite control point".into());
    }
    let mut result = CubicFitDerivatives {
        gradient: [0.; 4],
        damped_hessian: [[0.; 4]; 4],
    };
    for i in 0..4 {
        result.damped_hessian[i][i] = LEGACY_RIDGE;
    }
    for &(t, point) in samples {
        if !t.is_finite() || !(0. ..=1.).contains(&t) || !finite(point) {
            return Err("Invalid sample".into());
        }
        let u = 1. - t;
        let a = 3. * u * u * t;
        let b = 3. * u * t * t;
        let predicted = curve.evaluate(t);
        for (coordinate, residual) in [predicted.x - point.x, predicted.y - point.y]
            .into_iter()
            .enumerate()
        {
            result.gradient[coordinate] += 2. * a * residual;
            result.gradient[coordinate + 2] += 2. * b * residual;
            result.damped_hessian[coordinate][coordinate] += 2. * a * a;
            result.damped_hessian[coordinate][coordinate + 2] += 2. * a * b;
            result.damped_hessian[coordinate + 2][coordinate] += 2. * a * b;
            result.damped_hessian[coordinate + 2][coordinate + 2] += 2. * b * b;
        }
    }
    if !result
        .gradient
        .iter()
        .chain(result.damped_hessian.iter().flatten())
        .all(|x| x.is_finite())
    {
        return Err("Derivative overflow".into());
    }
    Ok(result)
}

/// Numerical portion of sample collector 0x0049C7B0.
/// Return only the supplied interior points, parameterized by cumulative chord
/// length from start, including the last chord to end in the denominator.
/// Repeated points are retained. A zero total length uses denominator 1.
/// Contour indexing and the original optional node-flag mutation are excluded.
pub fn chord_length_samples(
    start: Point,
    end: Point,
    interior: &[Point],
) -> Result<Vec<(f64, Point)>, String> {
    if !finite(start) || !finite(end) || !interior.iter().copied().all(finite) {
        return Err("Non-finite point".into());
    }
    if interior.is_empty() {
        return Ok(Vec::new());
    }
    let distance = |a: Point, b: Point| {
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        (dx * dx + dy * dy).sqrt()
    };
    let mut total = 0.;
    let mut previous = start;
    let mut samples = Vec::with_capacity(interior.len());
    for &point in interior {
        total += distance(previous, point);
        samples.push((total, point));
        previous = point;
    }
    total += distance(previous, end);
    if !total.is_finite() {
        return Err("Chord length overflow".into());
    }
    let inverse_length = 1. / if total == 0. { 1. } else { total };
    for (t, _) in &mut samples {
        *t *= inverse_length;
        if !t.is_finite() {
            return Err("Parameter overflow".into());
        }
    }
    Ok(samples)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkDirection {
    Backward,
    Forward,
}

/// 0x0049A970: convert three quadratic Bernstein points to ascending powers.
fn quadratic_to_power(q: [Point; 3]) -> Result<[Point; 3], String> {
    if !q.iter().copied().all(finite) {
        return Err("Non-finite point".into());
    }
    let out = [
        q[0],
        Point {
            x: 2. * (q[1].x - q[0].x),
            y: 2. * (q[1].y - q[0].y),
        },
        Point {
            x: q[0].x - 2. * q[1].x + q[2].x,
            y: q[0].y - 2. * q[1].y + q[2].y,
        },
    ];
    if !out.iter().copied().all(finite) {
        return Err("Conversion overflow".into());
    }
    Ok(out)
}

/// 0x0049AEE0: inverse binomial/forward-difference recurrence, degree three.
/// Input coefficients multiply 1, t, t^2, t^3, respectively.
fn power_to_cubic(a: [Point; 4]) -> Result<Cubic, String> {
    if !a.iter().copied().all(finite) {
        return Err("Non-finite coefficient".into());
    }
    // 0x49aee0 multiplies by the reciprocal of C(3, i) and subtracts each
    // earlier term, the signed binomial formed as an integer.
    let mut points = a;
    let mut outer = 1i32;
    for i in 0..4usize {
        if i > 0 {
            outer = (4 - i as i32) * outer / i as i32;
        }
        let inverse = 1. / outer as f64;
        points[i].x = a[i].x * inverse;
        points[i].y = a[i].y * inverse;
        let mut inner = 1i32;
        for j in 0..i {
            if j > 0 {
                inner = ((i as i32 - j as i32 + 1) * inner) / j as i32;
            }
            let factor = (if (i + j) % 2 == 0 { 1 } else { -1 }) as f64 * inner as f64;
            points[i].x -= factor * points[j].x;
            points[i].y -= factor * points[j].y;
        }
    }
    if !points.iter().copied().all(finite) {
        return Err("Conversion overflow".into());
    }
    Ok(Cubic { points })
}

#[inline(always)]
fn next_binomial(coeff: i32, n: i32, k: i32) -> i32 {
    coeff * (n - k + 1) / k
}

/// Advances the outer C(n, i) coefficient of 0x0047F650 and 0x0047F790; the
/// initial i == 0 keeps the starting 1.
#[inline(always)]
fn advance_outer_binomial(coeff: i32, n: i32, i: usize) -> i32 {
    if i > 0 {
        next_binomial(coeff, n, i as i32)
    } else {
        coeff
    }
}

/// Advances the inner C(i, j) coefficient of 0x0047F650 and 0x0047F790 and
/// returns it signed by (-1)^(i+j); the initial j == 0 keeps the starting 1.
#[inline(always)]
fn signed_inner_binomial(inner: &mut i32, i: i32, j: i32) -> f64 {
    if j > 0 {
        *inner = next_binomial(*inner, i, j);
    }
    (if (i + j) % 2 == 0 { *inner } else { -*inner }) as f64
}

/// 0x0047F650: cubic Bernstein points to ascending powers by the binomial
/// forward-difference sum, `A_i = C(3,i) * sum_j (-1)^(i+j) C(i,j) P_j`,
/// accumulated in ascending j with each signed binomial formed as an integer.
fn cubic_to_power(c: Cubic) -> Result<[Point; 4], String> {
    if !c.points.into_iter().all(finite) {
        return Err("Non-finite control point".into());
    }
    let mut out = [Point { x: 0., y: 0. }; 4];
    let mut outer = 1i32;
    for (i, o) in out.iter_mut().enumerate() {
        outer = advance_outer_binomial(outer, 3, i);
        let (mut sx, mut sy) = (0., 0.);
        let mut inner = 1i32;
        for j in 0..=i {
            let signed = signed_inner_binomial(&mut inner, i as i32, j as i32);
            sx += signed * c.points[j].x;
            sy += signed * c.points[j].y;
        }
        *o = Point {
            x: sx * outer as f64,
            y: sy * outer as f64,
        };
    }
    if !out.iter().copied().all(finite) {
        return Err("Conversion overflow".into());
    }
    Ok(out)
}

/// 0x0047F790: the degree-two inverse recurrence, `Q_i = A_i * (1/C(2,i)) -
/// sum_{j<i} (-1)^(i+j) C(i,j) Q_j`, subtracting each earlier term in order.
fn power_to_quadratic(a: [Point; 3]) -> Result<[Point; 3], String> {
    if !a.iter().copied().all(finite) {
        return Err("Non-finite coefficient".into());
    }
    let mut q = [Point { x: 0., y: 0. }; 3];
    let mut outer = 1i32;
    for i in 0..3usize {
        outer = advance_outer_binomial(outer, 2, i);
        let inverse = 1. / outer as f64;
        q[i] = Point {
            x: a[i].x * inverse,
            y: a[i].y * inverse,
        };
        let mut inner = 1i32;
        for j in 0..i {
            let signed = signed_inner_binomial(&mut inner, i as i32, j as i32);
            q[i].x -= signed * q[j].x;
            q[i].y -= signed * q[j].y;
        }
    }
    if !q.iter().copied().all(finite) {
        return Err("Conversion overflow".into());
    }
    Ok(q)
}

/// 0x0049B2B0/0x0047FE00 read a two-coordinate record's curve as a quadratic:
/// 0x0047F650, then 0x0047F900 keeps the first three power coefficients (the
/// cubic term is dropped, exact for a degree-elevated quadratic), then 0x0047F790.
pub fn cubic_to_quadratic(c: Cubic) -> Result<[Point; 3], String> {
    let a = cubic_to_power(c)?;
    power_to_quadratic([a[0], a[1], a[2]])
}

/// Three-interior-sample branch of 0x0049E960 uses 0x0049A970, then embeds
/// the three coefficients at 0x0049A2F0 into a zeroed degree-three polynomial,
/// and calls 0x0049AEE0. This is exact quadratic degree elevation in real math.
pub fn quadratic_to_cubic(q: [Point; 3]) -> Result<Cubic, String> {
    let a = quadratic_to_power(q)?;
    power_to_cubic([a[0], a[1], a[2], Point { x: 0., y: 0. }])
}

/// One/two-interior-sample branch at 0x0049EC4F..0x0049ED39.
/// Handles use line interpolation at .25/.75, but the returned squared error
/// evaluates the LINE at the FIRST sample only. The second sample is ignored.
/// This deliberately does not substitute thirds or evaluate the output cubic.
/// Finite/range checks and Result are new safe API choices.
pub fn fit_low_count(
    start: Point,
    end: Point,
    samples: &[(f64, Point)],
) -> Result<(Cubic, f64), String> {
    if !finite(start)
        || !finite(end)
        || !(1..=2).contains(&samples.len())
        || samples
            .iter()
            .any(|&(t, p)| !t.is_finite() || !(0. ..=1.).contains(&t) || !finite(p))
    {
        return Err("Expected one or two finite samples".into());
    }
    // 0x0049AAB0 evaluates (1-t)*start+t*end, including the error query.
    let line = |t: f64| Point {
        x: (1. - t) * start.x + t * end.x,
        y: (1. - t) * start.y + t * end.y,
    };
    let curve = Cubic {
        points: [start, line(0.25), line(0.75), end],
    };
    let prediction = line(samples[0].0);
    let dx = prediction.x - samples[0].1.x;
    let dy = prediction.y - samples[0].1.y;
    let error = dx * dx + dy * dy;
    if !error.is_finite() || !curve.points.iter().copied().all(finite) {
        return Err("Fit overflow".into());
    }
    Ok((curve, error))
}

/// 0x0049A7A0 for the +/-1 directions used by the fitting callers.
/// Step BEFORE inspecting the flag; stop at a different flag or the original
/// node ID, even if that ID occurs at another position in the contour.
/// Returns a contour position, not a node ID. Flags are indexed by node ID.
fn find_boundary_stop(
    node_ids: &[usize],
    flags: &[u8],
    start: usize,
    direction: WalkDirection,
    skip_flag: u8,
) -> Result<usize, String> {
    if start >= node_ids.len() || node_ids.iter().any(|&id| id >= flags.len()) {
        return Err("Invalid contour or node index".into());
    }
    let original_id = node_ids[start];
    let mut position = start;
    loop {
        position = match direction {
            WalkDirection::Forward => {
                if position + 1 == node_ids.len() {
                    0
                } else {
                    position + 1
                }
            }
            WalkDirection::Backward => {
                if position == 0 {
                    node_ids.len() - 1
                } else {
                    position - 1
                }
            }
        };
        let id = node_ids[position];
        if flags[id] != skip_flag || id == original_id {
            return Ok(position);
        }
    }
}

/// Finds the backward and forward boundary positions around `position`, unwrapping
/// cyclic wraparound by adding `node_ids.len()` if `end <= start`.
pub fn find_boundary_interval(
    node_ids: &[usize],
    flags: &[u8],
    position: usize,
    skip_flag: u8,
    overflow_msg: &'static str,
) -> Result<(usize, usize), String> {
    let start = find_boundary_stop(
        node_ids,
        flags,
        position,
        WalkDirection::Backward,
        skip_flag,
    )?;
    let mut end = find_boundary_stop(node_ids, flags, position, WalkDirection::Forward, skip_flag)?;
    if end <= start {
        end = end.checked_add(node_ids.len()).ok_or(overflow_msg)?;
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }
    fn curve() -> Cubic {
        Cubic {
            points: [point(0., 0.), point(2., -4.), point(6., 8.), point(0., 0.)],
        }
    }
    fn near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn low_count_handles_are_quarters_but_error_is_first_line_sample_only() {
        let start = point(0., 0.);
        let end = point(16., 8.);
        let first = (0.25, point(4., 5.));
        let (curve, error) = fit_low_count(start, end, &[first]).unwrap();
        assert_eq!(curve.points, [start, point(4., 2.), point(12., 6.), end]);
        assert_eq!(error, 9.);
        assert_eq!(
            fit_low_count(start, end, &[first, (0.9, point(-500., 700.))]).unwrap(),
            (curve, error)
        );
        // The emitted cubic has a different parameterization from the line.
        let q = curve.evaluate(first.0);
        assert_ne!((q.x - first.1.x).powi(2) + (q.y - first.1.y).powi(2), error);
        assert!(fit_low_count(start, end, &[]).is_err());
        assert!(fit_low_count(start, end, &[first; 3]).is_err());
        assert!(fit_low_count(start, end, &[(f64::NAN, start)]).is_err());
    }

    #[test]
    fn quadratic_elevation_matches_independent_de_casteljau_and_end_derivatives() {
        for seed in 0..64 {
            let q: [Point; 3] = std::array::from_fn(|i| {
                point(
                    ((seed * 31 + i * 17) % 101) as f64 - 50.,
                    ((seed * 43 + i * 13) % 97) as f64 - 40.,
                )
            });
            let c = quadratic_to_cubic(q).unwrap();
            let lerp = |a: Point, b: Point, t: f64| {
                point((1. - t) * a.x + t * b.x, (1. - t) * a.y + t * b.y)
            };
            for step in 0..=40 {
                let t = step as f64 / 40.;
                let expected = lerp(lerp(q[0], q[1], t), lerp(q[1], q[2], t), t);
                let actual = c.evaluate(t);
                near(actual.x, expected.x, 2e-13);
                near(actual.y, expected.y, 2e-13);
            }
            near(
                3. * (c.points[1].x - c.points[0].x),
                2. * (q[1].x - q[0].x),
                1e-13,
            );
            near(
                3. * (c.points[3].y - c.points[2].y),
                2. * (q[2].y - q[1].y),
                1e-13,
            );
        }
    }

    #[test]
    fn cubic_power_conversion_matches_horner_including_nonzero_cubic_term() {
        let a = [
            point(7., -3.),
            point(-12., 8.),
            point(18., -21.),
            point(-5., 10.),
        ];
        let c = power_to_cubic(a).unwrap();
        for step in 0..=100 {
            let t = step as f64 / 100.;
            let q = c.evaluate(t);
            near(
                q.x,
                ((a[3].x * t + a[2].x) * t + a[1].x) * t + a[0].x,
                1e-14,
            );
            near(
                q.y,
                ((a[3].y * t + a[2].y) * t + a[1].y) * t + a[0].y,
                1e-14,
            );
        }
        assert!(power_to_cubic([point(f64::MAX, 0.); 4]).is_err());
        assert!(quadratic_to_cubic([point(f64::NAN, 0.); 3]).is_err());
    }

    #[test]
    fn derivative_sign_factor_order_and_damping_match_hand_calculation() {
        // t=1/2: A's two nonzero entries per coordinate are 3/8.
        // C=(3,1.5), residual=(2,-1.5). The factor 2 is binary constant 0x6FBEE8.
        let d = cubic_fit_derivatives(curve(), &[(0.5, point(1., 3.))]).unwrap();
        assert_eq!(d.gradient, [1.5, -1.125, 1.5, -1.125]);
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i % 2 == j % 2 { 0.28125 } else { 0. }
                    + if i == j { LEGACY_RIDGE } else { 0. };
                assert_eq!(d.damped_hessian[i][j], expected);
            }
        }
        // A ridge objective would have nonzero gradient here. This routine doesn't.
        let exact = [(0.2, curve().evaluate(0.2)), (0.8, curve().evaluate(0.8))];
        assert_eq!(
            cubic_fit_derivatives(curve(), &exact).unwrap().gradient,
            [0.; 4]
        );
    }

    #[test]
    fn derivatives_match_independent_dense_reference() {
        let mut cases = 0;
        for line in include_str!("../fixtures/fitting-derivatives-golden.csv")
            .lines()
            .skip(1)
        {
            let v: Vec<f64> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let curve = Cubic {
                points: std::array::from_fn(|i| point(v[1 + 2 * i], v[2 + 2 * i])),
            };
            let samples: Vec<_> = v[29..]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|p| (p[0], point(p[1], p[2])))
                .collect();
            let d = cubic_fit_derivatives(curve, &samples).unwrap();
            for (a, e) in d
                .gradient
                .iter()
                .chain(d.damped_hessian.iter().flatten())
                .zip(&v[9..29])
            {
                near(*a, *e, 2e-10);
            }
            cases += 1;
        }
        assert_eq!(cases, 48);
    }

    #[test]
    fn gradient_and_hessian_agree_with_finite_differences_of_data_error() {
        let c = curve();
        let samples = [
            (0.13, point(1., -2.)),
            (0.42, point(5., 4.)),
            (0.9, point(-2., 1.)),
        ];
        let d = cubic_fit_derivatives(c, &samples).unwrap();
        let perturb = |mut c: Cubic, i: usize, delta: f64| {
            let p = &mut c.points[1 + i / 2];
            if i.is_multiple_of(2) {
                p.x += delta;
            } else {
                p.y += delta;
            }
            c
        };
        // De Casteljau is independent of the production Bernstein evaluation.
        let error = |c: Cubic| -> f64 {
            samples
                .iter()
                .map(|&(t, q)| {
                    let mut p = c.points;
                    for n in (1..4).rev() {
                        for i in 0..n {
                            p[i] = point(
                                (1. - t) * p[i].x + t * p[i + 1].x,
                                (1. - t) * p[i].y + t * p[i + 1].y,
                            );
                        }
                    }
                    (p[0].x - q.x).powi(2) + (p[0].y - q.y).powi(2)
                })
                .sum()
        };
        let h = 0.001;
        for i in 0..4 {
            near(
                (error(perturb(c, i, h)) - error(perturb(c, i, -h))) / (2. * h),
                d.gradient[i],
                1e-9,
            );
            for j in 0..4 {
                let numerical = (error(perturb(perturb(c, i, h), j, h))
                    - error(perturb(perturb(c, i, h), j, -h))
                    - error(perturb(perturb(c, i, -h), j, h))
                    + error(perturb(perturb(c, i, -h), j, -h)))
                    / (4. * h * h);
                near(
                    numerical + if i == j { LEGACY_RIDGE } else { 0. },
                    d.damped_hessian[i][j],
                    5e-8,
                );
            }
        }
    }

    #[test]
    fn empty_endpoint_and_repeated_samples_have_distinct_effects() {
        let empty = cubic_fit_derivatives(curve(), &[]).unwrap();
        let ends = cubic_fit_derivatives(curve(), &[(0., point(100., 50.)), (1., point(-3., 20.))])
            .unwrap();
        assert_eq!(empty, ends);
        let one = cubic_fit_derivatives(curve(), &[(0.5, point(1., 3.))]).unwrap();
        let two = cubic_fit_derivatives(curve(), &[(0.5, point(1., 3.)); 2]).unwrap();
        for i in 0..4 {
            assert_eq!(two.gradient[i], 2. * one.gradient[i]);
            near(
                two.damped_hessian[i][i],
                2. * one.damped_hessian[i][i] - LEGACY_RIDGE,
                1e-15,
            );
        }
    }

    #[test]
    fn chord_parameters_use_lengths_and_include_both_endpoint_chords() {
        let interior = [point(3., 0.), point(3., 4.)];
        let samples = chord_length_samples(point(0., 0.), point(6., 4.), &interior).unwrap();
        near(samples[0].0, 0.3, 1e-15);
        near(samples[1].0, 0.7, 1e-15);
        assert_eq!(samples.iter().map(|s| s.1).collect::<Vec<_>>(), interior);
        let reversed =
            chord_length_samples(point(6., 4.), point(0., 0.), &[interior[1], interior[0]])
                .unwrap();
        for i in 0..2 {
            near(reversed[i].0, 1. - samples[1 - i].0, 1e-15);
        }
    }

    #[test]
    fn chord_parameters_retain_duplicates_and_collapse_to_zero() {
        let p = point(7., -2.);
        let q = point(10., 2.);
        assert_eq!(
            chord_length_samples(p, p, &[p, p, p]).unwrap(),
            vec![(0., p); 3]
        );
        assert_eq!(
            chord_length_samples(p, q, &[p, p, q]).unwrap(),
            vec![(0., p), (0., p), (1., q)]
        );
        assert!(chord_length_samples(p, q, &[]).unwrap().is_empty());
    }

    #[test]
    fn boundary_walk_steps_first_wraps_and_stops_at_repeated_node_identity() {
        use WalkDirection::*;
        let ids = [2, 0, 3, 1];
        let flags = [1, 1, 4, 1];
        assert_eq!(find_boundary_stop(&ids, &flags, 0, Forward, 1).unwrap(), 0);
        assert_eq!(find_boundary_stop(&ids, &flags, 3, Forward, 1).unwrap(), 0);
        assert_eq!(find_boundary_stop(&ids, &flags, 1, Backward, 1).unwrap(), 0);
        // Repeated ID stops before returning to the original position.
        assert_eq!(
            find_boundary_stop(&[0, 1, 0, 2], &[1, 1, 1], 0, Forward, 1).unwrap(),
            2
        );
        assert_eq!(
            find_boundary_stop(&[0, 1, 0, 2], &[1, 1, 1], 0, Backward, 1).unwrap(),
            2
        );
        assert_eq!(find_boundary_stop(&[0], &[1], 0, Backward, 1).unwrap(), 0);
    }

    #[test]
    fn boundary_walk_matches_exhaustive_small_ring_reference() {
        use WalkDirection::*;
        for n in 1..=6 {
            let ids: Vec<_> = (0..n).collect();
            for mask in 0..(1usize << n) {
                let flags: Vec<_> = (0..n).map(|i| ((mask >> i) & 1) as u8).collect();
                for start in 0..n {
                    for direction in [Backward, Forward] {
                        for skip in [0, 1] {
                            let expected = (1..=n)
                                .map(|step| match direction {
                                    Forward => (start + step) % n,
                                    Backward => (start + n - step) % n,
                                })
                                .find(|&p| flags[p] != skip || p == start)
                                .unwrap();
                            assert_eq!(
                                find_boundary_stop(&ids, &flags, start, direction, skip).unwrap(),
                                expected
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn safe_api_rejects_invalid_indices_nonfinite_data_and_overflow() {
        let p = point(0., 0.);
        for t in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(cubic_fit_derivatives(curve(), &[(t, p)]).is_err());
        }
        let mut c = curve();
        c.points[3].x = f64::INFINITY;
        assert!(cubic_fit_derivatives(c, &[]).is_err());
        assert!(cubic_fit_derivatives(curve(), &[(0.5, point(f64::NAN, 0.))]).is_err());
        let large = Cubic {
            points: [point(f64::MAX, 0.); 4],
        };
        assert!(cubic_fit_derivatives(large, &[(0.5, point(-f64::MAX, 0.))]).is_err());
        assert!(chord_length_samples(p, p, &[point(f64::MAX, 0.)]).is_err());
        assert!(chord_length_samples(p, point(f64::NAN, 0.), &[]).is_err());
        assert!(find_boundary_stop(&[], &[], 0, WalkDirection::Forward, 0).is_err());
        assert!(find_boundary_stop(&[1], &[0], 0, WalkDirection::Forward, 0).is_err());
        assert!(find_boundary_stop(&[0], &[0], 1, WalkDirection::Backward, 0).is_err());
    }
}
