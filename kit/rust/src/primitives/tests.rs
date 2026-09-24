use super::*;
use crate::raster::Rgba;
use std::f64::consts::PI;

const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A colour and the points it paints.
type Painted<'a> = ([u8; 4], &'a dyn Fn(Point) -> bool);

/// A picture of `width` by `height` on white with each shape painted in its
/// colour: at pixel centres, or anti-aliased by 8 by 8 points per pixel.
fn picture(width: usize, height: usize, shapes: &[Painted], anti_aliased: bool) -> Raster {
    let n = if anti_aliased { 8 } else { 1 };
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.; 4];
            for j in 0..n {
                for i in 0..n {
                    let p = Point {
                        x: x as f64 + (i as f64 + 0.5) / n as f64,
                        y: y as f64 + (j as f64 + 0.5) / n as f64,
                    };
                    let colour = shapes
                        .iter()
                        .rev()
                        .find(|(_, inside)| inside(p))
                        .map_or(WHITE, |(c, _)| *c);
                    for k in 0..4 {
                        sum[k] += colour[k] as f64;
                    }
                }
            }
            pixels.push(Rgba(sum.map(|s| (s / (n * n) as f64).round() as u8)));
        }
    }
    Raster {
        width,
        height,
        pixels,
    }
}

fn p(x: f64, y: f64) -> Point {
    Point { x, y }
}

/// Path data for a closed polygon (`curved`: each side a cubic with its
/// handles on the side), turning the way `positive` says.
fn outline(points: &[Point], positive: bool, curved: bool) -> String {
    let mut points = points.to_vec();
    if (signed_area(&points) > 0.) != positive {
        points.reverse();
    }
    let mut d = format!(" M {:.2} {:.2}", points[0].x, points[0].y);
    for k in 1..=points.len() {
        let (a, b) = (points[k - 1], points[k % points.len()]);
        if curved {
            let (c1, c2) = (
                add(a, scale(sub(b, a), 1. / 3.)),
                add(a, scale(sub(b, a), 2. / 3.)),
            );
            d.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                c1.x, c1.y, c2.x, c2.y, b.x, b.y
            ));
        } else {
            d.push_str(&format!(" L {:.2} {:.2}", b.x, b.y));
        }
    }
    d
}

/// The engine's document for white with islands, each island's outline a
/// hole in the white.
fn document(width: usize, height: usize, islands: &[(&str, &[Point], bool)]) -> String {
    let (w, h) = (width as f64, height as f64);
    let mut white =
        format!(" M 0.00 {h:.2} L 0.00 0.00 L {w:.2} 0.00 L {w:.2} {h:.2} L 0.00 {h:.2}");
    for (_, points, curved) in islands {
        white.push_str(&outline(points, false, *curved));
    }
    let mut svg = format!(
        "<svg width=\"{width}pt\" height=\"{height}pt\" viewBox=\"0 0 {width} {height}\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\"{white} Z\" />\n</g>\n"
    );
    for (fill, points, curved) in islands {
        svg.push_str(&format!(
            "<g id=\"{fill}ff\">\n<path fill=\"{fill}\" opacity=\"1.00\" d=\"{} Z\" />\n</g>\n",
            outline(points, true, *curved)
        ));
    }
    svg.push_str("</svg>\n");
    svg
}

fn island_pieces(svg: &str, fill: &str) -> Vec<Edge> {
    let (ranges, paths) = parse_all_paths(svg).unwrap();
    let index = ranges
        .iter()
        .position(|&(start, _)| {
            svg[..start]
                .rfind("fill=\"")
                .map(|at| &svg[at + 6..at + 13])
                == Some(fill)
        })
        .unwrap();
    paths[index][0].edges.clone()
}

/// How many pixel centres the document's island and the exact shape
/// disagree on.
fn centre_mismatches(
    svg: &str,
    fill: &str,
    width: usize,
    height: usize,
    exact: &dyn Fn(Point) -> bool,
) -> usize {
    let island = polyline(&island_pieces(svg, fill), true);
    let mut wrong = 0;
    for y in 0..height {
        for x in 0..width {
            let c = p(x as f64 + 0.5, y as f64 + 0.5);
            if inside_polygon(&island, c) != exact(c) {
                wrong += 1;
            }
        }
    }
    wrong
}

const ALIASED: PrimitiveOptions = PrimitiveOptions {
    anti_aliased: false,
};
const SMOOTH: PrimitiveOptions = PrimitiveOptions { anti_aliased: true };

#[test]
fn a_small_pixel_edged_disc_traced_as_a_triangle_becomes_round() {
    // The shape set's 3.1 px disc, which the engine traced as a triangle.
    let disc = |q: Point| distance(q, p(20.5, 20.3)) < 3.1;
    let source = picture(40, 40, &[([124, 58, 237, 255], &disc)], false);
    let triangle = [p(18.9, 16.2), p(25.0, 20.3), p(17.5, 23.4)];
    let svg = document(40, 40, &[("#7c3aed", &triangle, false)]);
    let before = centre_mismatches(&svg, "#7c3aed", 40, 40, &disc);
    let (out, stats) = refit_svg(&svg, &source, ALIASED, &[]).unwrap();
    assert_eq!(stats.replaced(), 1, "{stats:?}\n{out}");
    assert!(stats.circles + stats.ellipses == 1, "{stats:?}");
    let after = centre_mismatches(&out, "#7c3aed", 40, 40, &disc);
    assert!(before > 0 && after == 0, "{before} -> {after}\n{out}");
    // The white's hole is the same outline, walked the other way.
    let hole = parse_all_paths(&out).unwrap().1[0][1].edges.clone();
    let island = island_pieces(&out, "#7c3aed");
    assert_eq!(hole.len(), island.len());
    for (a, b) in hole.iter().zip(island.iter().rev()) {
        assert_eq!(key(a.start()), key(b.end()));
    }
}

#[test]
fn a_pixel_edged_rounded_square_traced_turned_comes_back_square_on() {
    // The shape set's 12 px square with 3 px corners, which the engine
    // traced as a square turned by about 8 degrees.
    let (x0, y0, x1, y1, r) = (40., 20., 52., 32., 3.);
    let rounded = move |q: Point| {
        let (cx, cy) = (q.x.clamp(x0 + r, x1 - r), q.y.clamp(y0 + r, y1 - r));
        q.x > x0 && q.x < x1 && q.y > y0 && q.y < y1 && distance(q, p(cx, cy)) < r
    };
    let source = picture(70, 50, &[([29, 77, 216, 255], &rounded)], false);
    let turned = [
        p(40.64, 19.44),
        p(52.39, 21.41),
        p(51.36, 32.56),
        p(39.61, 30.59),
    ];
    let svg = document(70, 50, &[("#1d4dd8", &turned, false)]);
    let (out, stats) = refit_svg(&svg, &source, ALIASED, &[]).unwrap();
    assert_eq!(stats.replaced(), 1, "{stats:?}\n{out}");
    assert_eq!(
        centre_mismatches(&out, "#1d4dd8", 70, 50, &rounded),
        0,
        "{out}"
    );
    for edge in island_pieces(&out, "#1d4dd8").iter().filter(|e| e.line) {
        let d = sub(edge.end(), edge.start());
        assert!(
            d.x.abs() < 1e-9 || d.y.abs() < 1e-9,
            "a side still leans: {edge:?}"
        );
    }
}

#[test]
fn an_outline_no_shape_explains_stays_as_traced() {
    // A five-pointed star traced exactly: no circle, ellipse or rectangle
    // explains its pixels as well.
    let star: Vec<Point> = (0..10)
        .map(|k| {
            let angle = k as f64 * PI / 5. - FRAC_PI_2;
            let radius = if k % 2 == 0 { 14. } else { 6. };
            p(30. + radius * angle.cos(), 30. + radius * angle.sin())
        })
        .collect();
    let star_inside = |q: Point| inside_polygon(&star, q);
    let source = picture(60, 60, &[([200, 30, 30, 255], &star_inside)], false);
    let svg = document(60, 60, &[("#c81e1e", &star, false)]);
    let (out, stats) = refit_svg(&svg, &source, ALIASED, &[]).unwrap();
    assert_eq!((stats.judged, stats.replaced()), (1, 0), "{stats:?}");
    assert_eq!(out, svg);
}

#[test]
fn an_anti_aliased_disc_traced_with_bulges_becomes_its_circle() {
    let (centre, radius) = (p(24.3, 21.7), 6.4);
    let disc = move |q: Point| distance(q, centre) < radius;
    let source = picture(50, 44, &[([21, 128, 61, 255], &disc)], true);
    // Eight pieces through points that bulge in and out by 0.4 px.
    let bumpy: Vec<Point> = (0..8)
        .map(|k| {
            let angle = k as f64 * PI / 4.;
            let r = radius + if k % 2 == 0 { 0.4 } else { -0.4 };
            p(centre.x + r * angle.cos(), centre.y + r * angle.sin())
        })
        .collect();
    let svg = document(50, 44, &[("#15803d", &bumpy, true)]);
    let (out, stats) = refit_svg(&svg, &source, SMOOTH, &[]).unwrap();
    assert_eq!(stats.circles, 1, "{stats:?}\n{out}");
    for q in polyline(&island_pieces(&out, "#15803d"), true) {
        assert!(
            (distance(q, centre) - radius).abs() < 0.08,
            "{q:?} off the circle\n{out}"
        );
    }
}

#[test]
fn outlines_with_junctions_or_forced_nodes_are_left_alone() {
    // A forced node keeps its outline.
    let disc = |q: Point| distance(q, p(20.5, 20.3)) < 3.1;
    let source = picture(40, 40, &[([124, 58, 237, 255], &disc)], false);
    let triangle = [p(18.9, 16.2), p(25.0, 20.3), p(17.5, 23.4)];
    let svg = document(40, 40, &[("#7c3aed", &triangle, false)]);
    let (out, stats) = refit_svg(&svg, &source, ALIASED, &[p(25.0, 20.3)]).unwrap();
    assert_eq!((stats.outlines, stats.replaced()), (0, 0));
    assert_eq!(out, svg);
    // So does a node of the shape that replaced it, forced where the
    // desktop showed it (the document writes hundredths).
    let (drawn, _) = refit_svg(&svg, &source, ALIASED, &[]).unwrap();
    let shown = island_pieces(&drawn, "#7c3aed")[1].start();
    let written = p(
        (shown.x * 100.).round() / 100.,
        (shown.y * 100.).round() / 100.,
    );
    let (out, stats) = refit_svg(&svg, &source, ALIASED, &[written]).unwrap();
    assert_eq!(stats.replaced(), 0, "{stats:?}");
    assert_eq!(out, svg);
    // Two half discs of different colours: every outline has the junctions
    // where the halves meet the white.
    let top = |q: Point| distance(q, p(20., 20.)) < 8. && q.y < 20.;
    let bottom = |q: Point| distance(q, p(20., 20.)) < 8. && q.y >= 20.;
    let source = picture(
        40,
        40,
        &[([200, 0, 0, 255], &top), ([0, 0, 200, 255], &bottom)],
        false,
    );
    let half = |sign: f64| -> Vec<Point> {
        (0..=8)
            .map(|k| {
                let angle = PI * k as f64 / 8.;
                p(20. + 8. * angle.cos(), 20. + sign * 8. * angle.sin())
            })
            .collect()
    };
    let (upper, lower) = (half(-1.), half(1.));
    let mut white =
        String::from(" M 0.00 40.00 L 0.00 0.00 L 40.00 0.00 L 40.00 40.00 L 0.00 40.00");
    let mut ring: Vec<Point> = upper.clone();
    ring.extend(lower.iter().rev().skip(1).take(7));
    white.push_str(&outline(&ring, false, false));
    let svg = format!(
        "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\"{white} Z\" />\n</g>\n<g id=\"#c80000ff\">\n<path fill=\"#c80000\" opacity=\"1.00\" d=\"{} Z\" />\n</g>\n<g id=\"#0000c8ff\">\n<path fill=\"#0000c8\" opacity=\"1.00\" d=\"{} Z\" />\n</g>\n</svg>\n",
        outline(&upper, true, false),
        outline(&lower, true, false)
    );
    let (out, stats) = refit_svg(&svg, &source, ALIASED, &[]).unwrap();
    assert_eq!(stats.replaced(), 0, "{stats:?}");
    assert_eq!(out, svg);
}

#[test]
fn photographs_get_no_shapes_and_artwork_says_how_it_was_traced() {
    use crate::{basic_preset_code, ImageCategory, Quality};
    for quality in [Quality::High, Quality::Medium, Quality::Low] {
        assert_eq!(
            PrimitiveOptions::for_preset(basic_preset_code(ImageCategory::Photograph, quality)),
            None
        );
        assert_eq!(
            PrimitiveOptions::for_preset(basic_preset_code(
                ImageCategory::AntiAliasedArtwork,
                quality
            )),
            Some(SMOOTH)
        );
        assert_eq!(
            PrimitiveOptions::for_preset(basic_preset_code(ImageCategory::AliasedArtwork, quality)),
            Some(ALIASED)
        );
    }
}

#[test]
fn rounded_rectangles_close_on_their_start_and_turn_with_the_angle() {
    for radius in [0., 1.5, 4.] {
        let shape = Shape::rounded(p(10., 8.), (5., 4.), 0.3, radius);
        let pieces = shape.pieces();
        let n = pieces.len();
        for k in 0..n {
            assert_eq!(key(pieces[k].end()), key(pieces[(k + 1) % n].start()));
        }
        assert!(signed_area(&polyline(&pieces, true)) > 0.);
        for q in polyline(&pieces, true) {
            assert!(shape.signed_distance(q).abs() < 2e-3, "{radius}: {q:?}");
        }
    }
}

#[test]
fn a_pixel_edged_square_on_the_pixel_grid_stays_square() {
    // A 2 by 2 dot drawn on its pixels: at the pixel centres a circle of
    // radius 1.12 explains it as well, and used to win for its fewer
    // parameters.
    let dot = |q: Point| (20. ..22.).contains(&q.x) && (20. ..22.).contains(&q.y);
    let source = picture(40, 40, &[([21, 128, 61, 255], &dot)], false);
    let square = [p(20., 20.), p(22., 20.), p(22., 22.), p(20., 22.)];
    let svg = document(40, 40, &[("#15803d", &square, false)]);
    let (out, stats) = refit_svg(&svg, &source, ALIASED, &[]).unwrap();
    assert_eq!(stats.replaced(), 0, "{stats:?}\n{out}");
    assert_eq!(out, svg);
}
