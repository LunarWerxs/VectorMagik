use super::trace;

/// A lone pixel keeps its square and a 1 px diagonal stays one area with the
/// background around it whole: smoothed like any staircase, the lone pixel
/// became a diamond (half its overlap, round depixel-r1), and bridged the
/// other way the diagonal would fall apart into its pixels.
#[test]
fn a_lone_pixel_stays_square_and_a_diagonal_stays_whole() {
    let (w, h) = (12, 12);
    let white = [255, 255, 255, 255];
    let black = [17, 17, 17, 255];
    let mut pixels = vec![white; w * h];
    pixels[2 * w + 2] = black;
    for k in 0..6 {
        pixels[(4 + k) * w + 5 + k] = black;
    }
    let (svg, areas, _) = trace(&pixels, w, h);
    // The background, the pixel and the diagonal.
    assert_eq!(areas, 3, "{svg}");
    let black_path = svg
        .lines()
        .find(|l| l.starts_with("<path fill=\"#111111\""))
        .expect("the black path");
    let d = &black_path[black_path.find(" d=\"").unwrap() + 4..];
    let d = &d[..d.find('"').unwrap()];
    let loops: Vec<Vec<(f64, f64)>> = d
        .split('M')
        .skip(1)
        .map(|sub| {
            let numbers: Vec<f64> = sub
                .split(|c: char| c == ' ' || c.is_ascii_alphabetic())
                .filter_map(|t| t.parse().ok())
                .collect();
            numbers.chunks(2).map(|p| (p[0], p[1])).collect()
        })
        .collect();
    let square = loops
        .iter()
        .find(|l| l.iter().all(|p| p.0 <= 3. && p.1 <= 3.))
        .expect("the lone pixel's outline");
    let mut corners: Vec<(f64, f64)> = square.clone();
    corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
    corners.dedup();
    assert_eq!(corners, [(2., 2.), (2., 3.), (3., 2.), (3., 3.)], "{d}");
}
