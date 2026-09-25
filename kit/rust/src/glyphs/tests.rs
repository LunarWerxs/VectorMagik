use super::*;
use crate::raster::Rgba;
use crate::shapes::islands;

const INK: [u8; 3] = [20, 20, 20];

/// A 40 by 40 white picture with ink over the rectangles `boxes` (x0, y0,
/// x1, y1), anti-aliased by exact area.
fn picture(boxes: &[(f64, f64, f64, f64)], ink: [u8; 3]) -> Raster {
    let cover = |lo: f64, hi: f64, px: f64| (hi.min(px + 1.) - lo.max(px)).clamp(0., 1.);
    let pixels = (0..40 * 40)
        .map(|i| {
            let (x, y) = ((i % 40) as f64, (i / 40) as f64);
            let t: f64 = boxes
                .iter()
                .map(|&(x0, y0, x1, y1)| cover(x0, x1, x) * cover(y0, y1, y))
                .sum::<f64>()
                .min(1.);
            let mix = |c: usize| (255. * (1. - t) + ink[c] as f64 * t).round() as u8;
            Rgba([mix(0), mix(1), mix(2), 255])
        })
        .collect();
    Raster {
        width: 40,
        height: 40,
        pixels,
    }
}

/// A white field with one grey region traced over `(x0, y0)`-`(x1, y1)`.
fn traced(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    format!(
        "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">\r\n\
         <g id=\"#ffffffff\">\r\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 \
         L 40.00 40.00 L 0.00 40.00 L 0.00 0.00 M {x0:.2} {y0:.2} L {x0:.2} {y1:.2} L {x1:.2} {y1:.2} \
         L {x1:.2} {y0:.2} L {x0:.2} {y0:.2} Z\" />\r\n</g>\r\n\
         <g id=\"#555555ff\">\r\n<path fill=\"#555555\" opacity=\"1.00\" d=\" M {x0:.2} {y0:.2} \
         L {x1:.2} {y0:.2} L {x1:.2} {y1:.2} L {x0:.2} {y1:.2} L {x0:.2} {y0:.2} Z\" />\r\n</g>\r\n</svg>\r\n"
    )
}

/// The islands of `svg` in an ink darker than the traced grey #555555 (a
/// stroke with no pixel wholly covered takes the darkest pixel's colour).
fn inked(svg: &str) -> Vec<Island> {
    islands(svg)
        .unwrap()
        .into_iter()
        .filter(|i| hex_rgb(&i.color).is_some_and(|c| c.iter().all(|&v| v < 0x55)))
        .collect()
}

#[test]
fn an_i_whose_dot_the_trace_merged_comes_back_as_stem_and_dot() {
    // A 1.3 px stem with its dot 3 px above it, as small print draws an i,
    // traced as one region.
    let source = picture(&[(18.2, 20.3, 19.5, 30.), (18.2, 16., 19.5, 17.3)], INK);
    let svg = traced(18.2, 15.5, 19.5, 30.);
    let (redrawn, stats) = redraw_glyphs(&svg, &source).unwrap();
    assert_eq!((stats.candidates, stats.redrawn), (1, 1), "{redrawn}");
    let letters = inked(&redrawn);
    assert_eq!(letters.len(), 2, "{redrawn}");
    // The dot above the gap and the stem below it, each about where its
    // pixels are.
    let dot = letters.iter().find(|i| i.max.y < 19.).expect("the dot");
    let stem = letters.iter().find(|i| i.min.y > 19.).expect("the stem");
    let near = |a: f64, b: f64| (a - b).abs() < 0.35;
    assert!(
        near(dot.min.y, 16.) && near(dot.max.y, 17.3),
        "{:?} {:?}",
        dot.min,
        dot.max
    );
    assert!(
        near(stem.min.y, 20.3) && near(stem.max.y, 30.),
        "{:?} {:?}",
        stem.min,
        stem.max
    );
    assert!(near(stem.min.x, 18.2) && near(stem.max.x, 19.5));
    // The field has both as holes, sharing their outlines, and the old
    // grey region is gone.
    let all = islands(&redrawn).unwrap();
    let field = all.iter().find(|i| i.color == "#ffffff").unwrap();
    assert_eq!(field.holes.len(), 2, "{redrawn}");
    assert!(!redrawn.contains("fill=\"#555555\""), "{redrawn}");
}

#[test]
fn a_letter_traced_in_the_wrong_hue_takes_its_pixels_ink() {
    // A blue stem 2 px wide that the trace filled grey, as the engine's
    // colour model groups a pale or thin ink with its neighbours'.
    let blue = [30, 60, 200];
    let source = picture(&[(18., 18., 20., 30.)], blue);
    let svg = traced(18., 18., 20., 30.);
    let (redrawn, stats) = redraw_glyphs(&svg, &source).unwrap();
    assert_eq!(stats.redrawn, 1, "{redrawn}");
    let letter = islands(&redrawn)
        .unwrap()
        .into_iter()
        .find(|i| i.color != "#ffffff")
        .expect("the stem");
    let c = hex_rgb(&letter.color).unwrap();
    assert!(
        (0..3).all(|k| (i32::from(c[k]) - i32::from(blue[k])).abs() <= 6),
        "{c:?}"
    );
}

#[test]
fn a_thick_shape_and_a_faint_one_are_left_to_the_engine() {
    // Wider than lettering this small, and a stroke as thin but longer.
    let square = picture(&[(10., 10., 30., 30.)], INK);
    let svg = traced(10., 10., 30., 30.);
    assert_eq!(
        redraw_glyphs(&svg, &square).unwrap(),
        (svg, GlyphStats::default())
    );
    let long = picture(&[(18.2, 5., 19.5, 35.)], INK);
    let svg = traced(18.2, 5., 19.5, 35.);
    assert_eq!(
        redraw_glyphs(&svg, &long).unwrap(),
        (svg, GlyphStats::default())
    );
    // Too faint against its background to be ink.
    let faint = picture(&[(18.2, 16., 19.5, 30.)], [235, 235, 235]);
    let svg = traced(18.2, 15.5, 19.5, 30.);
    assert_eq!(redraw_glyphs(&svg, &faint).unwrap().1.redrawn, 0);
}

/// How much of pixel (`px`, `py`) lies inside `inside`, by a 400 by 400
/// grid of samples (to about 1e-5).
fn area(px: usize, py: usize, inside: impl Fn(f64, f64) -> bool) -> f64 {
    let n = 400;
    let mut hits = 0;
    for sy in 0..n {
        for sx in 0..n {
            let x = px as f64 + (sx as f64 + 0.5) / n as f64;
            let y = py as f64 + (sy as f64 + 0.5) / n as f64;
            hits += usize::from(inside(x, y));
        }
    }
    hits as f64 / (n * n) as f64
}

/// A straight edge placed from its box-filtered coverage lands on the line
/// at every column's centre; a band thinner than a pixel within a few
/// thousandths on average (a hundredth or three where it crosses from one
/// pixel into the next partway along the column), where marching squares'
/// linear crossing is off by a few hundredths, varying with the edge's phase
/// against the grid (the wobble).
#[test]
fn a_straight_edge_and_a_thin_band_are_placed_exactly_from_their_coverage() {
    let (w, h) = (14, 12);
    let edge = |x: f64| 0.3 * x + 3.37;
    let solid: Vec<f64> = (0..w * h)
        .map(|i| area(i % w, i / w, |x, y| y > edge(x)))
        .collect();
    let band: Vec<f64> = (0..w * h)
        .map(|i| area(i % w, i / w, |x, y| y > edge(x) && y < edge(x) + 0.7))
        .collect();
    let mut linear_worst: f64 = 0.;
    for (value, is_band) in [(&solid, false), (&band, true)] {
        let (mut declined, mut placed, mut off) = (0, 0, 0.);
        for column in 1..w - 1 {
            let line: Vec<f64> = (0..h).map(|r| value[r * w + column]).collect();
            let i = (0..h - 1)
                .find(|&r| line[r] < 0.3 && line[r + 1] >= line[r] + 0.1)
                .unwrap();
            let truth = edge(column as f64 + 0.5);
            let t = (0.5 - line[i]) / (line[i + 1] - line[i]);
            linear_worst = linear_worst.max((i as f64 + 0.5 + t - truth).abs());
            let Some(at) = edge_along(&line, i) else {
                // Only a band that lies within one pixel of the column.
                assert!(is_band && line.iter().filter(|&&c| c > CLEAR).count() == 1);
                declined += 1;
                continue;
            };
            let limit = if is_band { 0.04 } else { 2e-3 };
            assert!(
                (at - truth).abs() < limit,
                "column {column}: {at} for {truth}"
            );
            placed += 1;
            off += (at - truth).abs();
        }
        assert!(declined * 3 <= w, "{declined} columns declined");
        assert!(off / f64::from(placed) < 0.01, "{off} over {placed}");
    }
    assert!(linear_worst > 0.02, "{linear_worst}");
    // A steep edge along a column is not placed from that column.
    let steep: Vec<f64> = (0..h)
        .map(|r| area(3, r, |x, y| x < 3.2 + 0.05 * y))
        .collect();
    assert_eq!(edge_along(&steep, 4), None);
}

#[test]
fn a_rippled_loop_is_drawn_without_turning_back_and_close_to_it() {
    // A circle of radius 3 traced as 40 points, each off it by 0.04 px in
    // turn out and in, as marching squares' crossings wobble.
    let loop_: Vec<Point> = (0..40)
        .map(|k| {
            let a = k as f64 / 40. * std::f64::consts::TAU;
            let r = 3. + if k % 2 == 0 { 0.04 } else { -0.04 };
            Point {
                x: 10. + r * a.cos(),
                y: 10. + r * a.sin(),
            }
        })
        .collect();
    let edges = spline(&loop_, RIPPLE);
    // The tangent along the drawing, sampled: it never turns back.
    let mut directions = Vec::new();
    for e in &edges {
        let [p0, p1, p2, p3] = e.cubic.points;
        for s in 0..8 {
            let t = s as f64 / 8.;
            let u = 1. - t;
            directions.push(Point {
                x: 3.
                    * (u * u * (p1.x - p0.x) + 2. * u * t * (p2.x - p1.x) + t * t * (p3.x - p2.x)),
                y: 3.
                    * (u * u * (p1.y - p0.y) + 2. * u * t * (p2.y - p1.y) + t * t * (p3.y - p2.y)),
            });
        }
    }
    let n = directions.len();
    for k in 0..n {
        let turn = angle(directions[k], directions[(k + 1) % n]);
        assert!(turn > -1e-9, "turns back {turn} at {k} of {n}");
    }
    // And it stays within the ripple's tolerance of the circle, and of
    // every point, with room for the spline's cut across each point.
    for e in &edges {
        for s in 0..=8 {
            let p = e.cubic.evaluate(s as f64 / 8.);
            let off = ((p.x - 10.).hypot(p.y - 10.) - 3.).abs();
            assert!(off < RIPPLE, "{off} off the circle at {p:?}");
        }
    }
    // The ripple is gone, not drawn: fewer pieces than points.
    assert!(edges.len() < loop_.len(), "{} pieces", edges.len());
}

/// A line of small print whose letters the trace merged into one thin region
/// too long to redraw as a letter (the engine's "Capt" on the pale band) is
/// redrawn whole from its coverage, in the run's ink: before, the merged
/// letters kept the trace's grey.
#[test]
fn a_run_whose_letters_the_trace_merged_is_redrawn_whole() {
    let strokes = [8., 12., 17., 22., 27.].map(|x| (x, 10., x + 1.3, 20.));
    let source = picture(&strokes, INK);
    // The first stroke traced alone; the other four as one comb, joined at
    // its foot, 16.3 px long.
    let a = [(8., 10.), (9.3, 10.), (9.3, 20.), (8., 20.)];
    let mut comb: Vec<(f64, f64)> = Vec::new();
    for x in [12., 17., 22., 27.] {
        if !comb.is_empty() {
            comb.push((x, 19.));
        }
        comb.extend([(x, 10.), (x + 1.3, 10.), (x + 1.3, 19.)]);
    }
    comb.truncate(comb.len() - 1);
    comb.extend([(28.3, 20.), (12., 20.)]);
    let loop_of = |points: &[(f64, f64)]| {
        let mut d = String::new();
        for (k, (x, y)) in points.iter().chain(points.first()).enumerate() {
            d.push_str(&format!(
                " {} {x:.2} {y:.2}",
                if k == 0 { "M" } else { "L" }
            ));
        }
        d
    };
    let reversed = |points: &[(f64, f64)]| {
        let mut r = points.to_vec();
        r.reverse();
        r
    };
    let svg = format!(
        "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">\r\n\
         <g id=\"#ffffffff\">\r\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 \
         L 40.00 40.00 L 0.00 40.00 L 0.00 0.00{}{} Z\" />\r\n</g>\r\n\
         <g id=\"#555555ff\">\r\n<path fill=\"#555555\" opacity=\"1.00\" d=\"{} Z\" />\r\n\
         <path fill=\"#555555\" opacity=\"1.00\" d=\"{} Z\" />\r\n</g>\r\n</svg>\r\n",
        loop_of(&reversed(&a)),
        loop_of(&reversed(&comb)),
        loop_of(&a),
        loop_of(&comb),
    );
    let (redrawn, stats) = redraw_glyphs(&svg, &source).unwrap();
    assert_eq!(stats.runs, 1, "{redrawn}");
    assert_eq!(inked(&redrawn).len(), 5, "{redrawn}");
}
