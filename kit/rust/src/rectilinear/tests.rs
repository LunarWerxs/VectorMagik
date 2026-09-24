use super::*;
use crate::raster::Rgba;

/// A white picture with the pixels `ink` says in `colour`.
fn picture(w: usize, h: usize, colour: [u8; 3], ink: impl Fn(usize, usize) -> bool) -> Raster {
    let pixels = (0..w * h)
        .map(|i| {
            if ink(i % w, i / w) {
                Rgba([colour[0], colour[1], colour[2], 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        })
        .collect();
    Raster {
        width: w,
        height: h,
        pixels,
    }
}

/// The field with `d` cut out of it as a hole and drawn in `fill`.
fn document(d: &str, fill: &str) -> String {
    documents(&[(d, fill)])
}

/// The field with each `(d, fill)` cut out of it as a hole and drawn in its
/// fill.
fn documents(regions: &[(&str, &str)]) -> String {
    let mut holes = String::new();
    let mut groups = String::new();
    for (d, fill) in regions {
        let mut hole: Vec<&str> = d.trim().trim_end_matches(" Z").split(" L ").collect();
        let start = hole[0].trim_start_matches("M ");
        // The outline returns to its start: that last node is the first.
        if hole.last() == Some(&start) {
            hole.pop();
        }
        holes.push_str(&format!(" M {start}"));
        for p in hole[1..].iter().rev() {
            holes.push_str(&format!(" L {p}"));
        }
        holes.push_str(&format!(" L {start}"));
        groups.push_str(&format!(
            "<g id=\"{fill}ff\">\n<path fill=\"{fill}\" opacity=\"1.00\" d=\"{d}\" />\n</g>\n"
        ));
    }
    format!(
        "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 L 40.00 40.00 L 0.00 40.00 L 0.00 0.00{holes} Z\" />\n</g>\n{groups}</svg>\n"
    )
}

#[test]
fn a_plus_traced_as_a_star_is_drawn_on_its_pixels() {
    // A plus of 5 px arms, 15 px across, centred on (20, 20); the engine's
    // star: its eight outer corners pulled in.
    let plus = picture(40, 40, [0x2c, 0x5a, 0xa0], |x, y| {
        ((17..22).contains(&x) && (13..28).contains(&y))
            || ((13..28).contains(&y) && (17..22).contains(&x))
            || ((13..28).contains(&x) && (17..22).contains(&y))
    });
    let star = " M 17.50 13.00 L 21.50 13.00 L 22.00 17.00 L 27.00 17.50 L 27.00 21.50 L 22.00 22.00 L 21.50 28.00 L 17.50 28.00 L 17.00 22.00 L 13.00 21.50 L 13.00 17.50 L 17.00 17.00 L 17.50 13.00 Z";
    let svg = document(star, "#2c5aa0");
    let (out, stats) = refit_rectilinear(&svg, &plus).unwrap();
    assert_eq!(stats.refitted, 1, "{out}");
    let found = islands(&out).unwrap();
    let shape = found.iter().find(|i| i.color == "#2c5aa0").unwrap();
    // 15 x 5 twice, less the 5 x 5 they share.
    assert!(
        (shape.area() - 125.).abs() < 1e-9,
        "{}: {out}",
        shape.area()
    );
    let total: f64 = found.iter().map(|i| i.area()).sum();
    assert!((total - 1600.).abs() < 1e-9, "{total}: {out}");
    let outlines: Vec<&str> = out.split(" d=\"").skip(1).collect();
    for corner in ["17.00 13.00", "22.00 13.00", "28.00 17.00", "13.00 22.00"] {
        assert!(
            outlines.iter().all(|d| d.contains(corner)),
            "{corner}: {out}"
        );
    }
}

#[test]
fn a_pixel_disc_is_left_to_the_engine() {
    // A disc of radius 6: its pixel outline steps by one and two pixels.
    let disc = picture(40, 40, [0x2c, 0x5a, 0xa0], |x, y| {
        (x as f64 + 0.5 - 20.).hypot(y as f64 + 0.5 - 20.) < 6.
    });
    let round = " M 14.00 20.00 L 20.00 14.00 L 26.00 20.00 L 20.00 26.00 L 14.00 20.00 Z";
    let svg = document(round, "#2c5aa0");
    let (out, stats) = refit_rectilinear(&svg, &disc).unwrap();
    assert_eq!(stats.refitted, 0);
    assert_eq!(out, svg);
}

#[test]
fn a_thin_stem_s_end_is_no_staircase_and_a_diagonal_is() {
    let at = |p: &[(f64, f64)]| p.iter().map(|&(x, y)| Point { x, y }).collect::<Vec<_>>();
    // A T with a 1 px bar and stem: its 1 px ends each sit between long runs.
    let t = at(&[
        (10., 10.),
        (17., 10.),
        (17., 11.),
        (14., 11.),
        (14., 18.),
        (13., 18.),
        (13., 11.),
        (10., 11.),
    ]);
    assert!(no_staircase(&t));
    // Three steps of a 1 px diagonal: short runs in a row.
    let stairs = at(&[
        (0., 0.),
        (1., 0.),
        (1., 1.),
        (2., 1.),
        (2., 2.),
        (3., 2.),
        (3., 3.),
        (0., 3.),
    ]);
    assert!(!no_staircase(&stairs));
}

#[test]
fn a_plus_pinched_to_a_star_of_half_its_area_is_drawn_on_its_pixels() {
    // The plus of 5 px arms again, traced as the engine traced the shape
    // set's: a thin star whose arms reach its tips, 51 of its 125 px.
    let plus = picture(40, 40, [0x7c, 0x3a, 0xed], |x, y| {
        ((17..22).contains(&x) && (13..28).contains(&y))
            || ((13..28).contains(&x) && (17..22).contains(&y))
    });
    let star = " M 19.50 13.00 L 21.00 18.00 L 28.00 19.50 L 21.00 21.00 L 19.50 28.00 L 18.00 21.00 L 13.00 19.50 L 18.00 18.00 L 19.50 13.00 Z";
    let svg = document(star, "#7c3aed");
    let (out, stats) = refit_rectilinear(&svg, &plus).unwrap();
    assert_eq!(stats.refitted, 1, "{out}");
    let found = islands(&out).unwrap();
    let shape = found.iter().find(|i| i.color == "#7c3aed").unwrap();
    assert!(
        (shape.area() - 125.).abs() < 1e-9,
        "{}: {out}",
        shape.area()
    );
}

#[test]
fn a_colour_that_runs_on_under_another_region_is_left_alone() {
    // The same star, and a bar of its colour joined to the plus's right arm
    // that the engine traced as a region of its own.
    let plus = picture(40, 40, [0x7c, 0x3a, 0xed], |x, y| {
        ((17..22).contains(&x) && (13..28).contains(&y))
            || ((13..28).contains(&x) && (17..22).contains(&y))
            || ((28..36).contains(&x) && (18..21).contains(&y))
    });
    let star = " M 19.50 13.00 L 21.00 18.00 L 28.00 19.50 L 21.00 21.00 L 19.50 28.00 L 18.00 21.00 L 13.00 19.50 L 18.00 18.00 L 19.50 13.00 Z";
    let bar = " M 29.00 18.00 L 36.00 18.00 L 36.00 21.00 L 29.00 21.00 L 29.00 18.00 Z";
    let svg = documents(&[(star, "#7c3aed"), (bar, "#7c3aed")]);
    let (out, stats) = refit_rectilinear(&svg, &plus).unwrap();
    assert_eq!(stats.refitted, 0, "{out}");
    assert_eq!(out, svg);
}

#[test]
fn a_one_pixel_diagonal_traced_as_a_crescent_is_drawn_as_a_band() {
    // 25 pixels of a 1 px diagonal from (5, 30) up to (29, 6), traced as the
    // engine traced the shape set's: one curved side, one straight.
    let line = picture(40, 40, [0x1d, 0x4e, 0xd8], |x, y| {
        (5..30).contains(&x) && y == 35 - x
    });
    let crescent = " M 5.12 31.29 C 13.00 22.00 21.00 14.00 29.26 6.03 L 5.12 31.29 Z";
    // `document` reverses lines only: the field's hole written out.
    let hole = " M 5.12 31.29 L 29.26 6.03 C 21.00 14.00 13.00 22.00 5.12 31.29";
    let svg = format!(
        "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 L 40.00 40.00 L 0.00 40.00 L 0.00 0.00{hole} Z\" />\n</g>\n<g id=\"#1d4ed8ff\">\n<path fill=\"#1d4ed8\" opacity=\"1.00\" d=\"{crescent}\" />\n</g>\n</svg>\n"
    );
    let (out, stats) = refit_rectilinear(&svg, &line).unwrap();
    assert_eq!(stats.refitted, 1, "{out}");
    let found = islands(&out).unwrap();
    let band = found.iter().find(|i| i.color == "#1d4ed8").unwrap();
    // Its ink, and from the first pixel's corner to the last's.
    assert!((band.area() - 25.).abs() < 0.05, "{}: {out}", band.area());
    assert!(
        (band.min.x - 4.75).abs() < 0.01 && (band.max.x - 30.25).abs() < 0.01,
        "{out}"
    );
    let total: f64 = found.iter().map(|i| i.area()).sum();
    assert!((total - 1600.).abs() < 0.05, "{total}: {out}");
}

#[test]
fn a_thin_arc_is_no_line() {
    // A quarter circle of radius 12, 1 px wide, traced along its pixels.
    let arc = picture(40, 40, [0x1d, 0x4e, 0xd8], |x, y| {
        let r = (x as f64 + 0.5 - 8.).hypot(y as f64 + 0.5 - 8.);
        x >= 8 && y >= 8 && (r - 12.).abs() < 0.5
    });
    let point = |r: f64, degrees: f64| {
        let a = degrees.to_radians();
        format!("{:.2} {:.2}", 8. + r * a.cos(), 8. + r * a.sin())
    };
    let mut d = format!(" M {}", point(12.4, 0.));
    for k in 1..=9 {
        d.push_str(&format!(" L {}", point(12.4, 10. * k as f64)));
    }
    for k in (0..=9).rev() {
        d.push_str(&format!(" L {}", point(11.6, 10. * k as f64)));
    }
    d.push_str(&format!(" L {} Z", point(12.4, 0.)));
    let svg = document(&d, "#1d4ed8");
    let (out, stats) = refit_rectilinear(&svg, &arc).unwrap();
    assert_eq!(stats.refitted, 0, "{out}");
    assert_eq!(out, svg);
}

#[test]
fn lines_of_two_colours_that_meet_are_drawn_over_the_field() {
    // A black 1 px line along row 10 meeting a red one down column 20, traced
    // as slivers that share an edge, the field's one hole around both.
    let pixels = (0..40 * 40)
        .map(|i| match (i % 40, i / 40) {
            (20, 10..25) => Rgba([0xdc, 0x26, 0x26, 255]),
            (5..20, 10) => Rgba([0, 0, 0, 255]),
            _ => Rgba([255, 255, 255, 255]),
        })
        .collect();
    let source = Raster {
        width: 40,
        height: 40,
        pixels,
    };
    let black =
        " M 5.00 10.50 L 12.00 10.35 L 20.00 10.00 L 20.00 11.00 L 12.00 10.65 L 5.00 10.50 Z";
    let red = " M 20.00 10.00 L 21.00 10.00 L 21.00 24.00 L 20.50 25.60 L 20.00 24.00 L 20.00 11.00 L 20.00 10.00 Z";
    let hole = " M 5.00 10.50 L 12.00 10.65 L 20.00 11.00 L 20.00 24.00 L 20.50 25.60 L 21.00 24.00 L 21.00 10.00 L 20.00 10.00 L 12.00 10.35 L 5.00 10.50";
    let svg = format!(
        "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 L 40.00 40.00 L 0.00 40.00 L 0.00 0.00{hole} Z\" />\n</g>\n<g id=\"#000000ff\">\n<path fill=\"#000000\" opacity=\"1.00\" d=\"{black}\" />\n</g>\n<g id=\"#dc2626ff\">\n<path fill=\"#dc2626\" opacity=\"1.00\" d=\"{red}\" />\n</g>\n</svg>\n"
    );
    let (out, stats) = refit_rectilinear(&svg, &source).unwrap();
    assert_eq!(stats.refitted, 2, "{out}");
    let found = islands(&out).unwrap();
    let area = |colour: &str| found.iter().find(|i| i.color == colour).unwrap().area();
    // The field whole, the lines on their pixels over it.
    assert!((area("#ffffff") - 1600.).abs() < 1e-9, "{out}");
    assert!((area("#000000") - 15.).abs() < 1e-9, "{out}");
    assert!((area("#dc2626") - 15.).abs() < 1e-9, "{out}");
    assert!(found.iter().all(|i| i.holes.is_empty()), "{out}");
}

#[test]
fn a_two_by_two_dot_traced_as_a_circle_is_drawn_as_its_square() {
    let dot = picture(40, 40, [0x15, 0x80, 0x3d], |x, y| {
        (20..22).contains(&x) && (20..22).contains(&y)
    });
    // The engine's round dot over the four pixels: within 0.2 px of the
    // square all round, but without its corners.
    let round = " M 19.90 21.00 L 20.18 20.18 L 21.00 19.90 L 21.82 20.18 L 22.10 21.00 L 21.82 21.82 L 21.00 22.10 L 20.18 21.82 L 19.90 21.00 Z";
    let svg = document(round, "#15803d");
    let (out, stats) = refit_rectilinear(&svg, &dot).unwrap();
    assert_eq!(stats.refitted, 1, "{out}");
    let found = islands(&out).unwrap();
    let square = found.iter().find(|i| i.color == "#15803d").unwrap();
    assert!(
        (square.area() - 4.).abs() < 1e-9,
        "{}: {out}",
        square.area()
    );
}
