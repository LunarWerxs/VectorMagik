use super::*;
use crate::raster::Rgba;
use crate::shapes::islands;

const FIELD: [u8; 3] = [44, 90, 160];
const SQUARE: [u8; 3] = [44, 90, 172];

/// A 40 by 40 field with a square from `a` to `b` (in pixels, fractional
/// edges anti-aliased by exact area when `smooth`).
fn picture(a: f64, b: f64, smooth: bool) -> Raster {
    let cover = |lo: f64, hi: f64, px: f64| (hi.min(px + 1.) - lo.max(px)).clamp(0., 1.);
    let pixels = (0..40 * 40)
        .map(|i| {
            let (x, y) = ((i % 40) as f64, (i / 40) as f64);
            let mut t = cover(a, b, x) * cover(a, b, y);
            if !smooth {
                t = if t >= 0.5 { 1. } else { 0. };
            }
            let mix = |c: usize| (FIELD[c] as f64 * (1. - t) + SQUARE[c] as f64 * t).round() as u8;
            Rgba([mix(0), mix(1), mix(2), 255])
        })
        .collect();
    Raster {
        width: 40,
        height: 40,
        pixels,
    }
}

/// The field traced as one region, the square merged into it.
const MERGED: &str = "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">\r\n<g id=\"#2c5aa0ff\">\r\n<path fill=\"#2c5aa0\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 L 40.00 40.00 L 0.00 40.00 L 0.00 0.00 Z\" />\r\n</g>\r\n</svg>\r\n";

#[test]
fn a_square_merged_into_its_field_is_drawn_back_with_a_hole_to_match() {
    // On the pixel grid it comes back on its pixel edges, in anti-aliased
    // artwork too.
    let (svg, stats) = recover_svg(MERGED, &picture(12., 26., false), false).unwrap();
    assert_eq!(
        recover_svg(MERGED, &picture(12., 26., false), true)
            .unwrap()
            .0,
        svg
    );
    assert_eq!((stats.blobs, stats.recovered), (1, 1), "{svg}");
    let found = islands(&svg).unwrap();
    assert_eq!(found.len(), 2, "{svg}");
    assert_eq!(found[1].color, "#2c5aac");
    // Pixel-edged: the square's own corners, and the field has it as a hole.
    let mut corners: Vec<(i64, i64)> = found[1]
        .outline
        .iter()
        .map(|p| (p.x.round() as i64, p.y.round() as i64))
        .collect();
    corners.sort();
    corners.dedup();
    assert_eq!(
        corners,
        vec![(12, 12), (12, 26), (26, 12), (26, 26)],
        "{svg}"
    );
    assert_eq!(found[0].holes.len(), 1, "{svg}");
    assert!((found[0].area() - (1600. - 196.)).abs() < 1e-6, "{svg}");
    assert!(svg.contains("\r\n<g id=\"#2c5aacff\">\r\n"), "{svg}");
}

#[test]
fn an_anti_aliased_square_comes_back_at_its_half_coverage_edge() {
    let (svg, stats) = recover_svg(MERGED, &picture(12.3, 26.6, true), true).unwrap();
    assert_eq!(stats.recovered, 1, "{svg}");
    let found = islands(&svg).unwrap();
    let square = &found[1];
    let (min, max) = (square.min, square.max);
    for (got, want) in [(min.x, 12.3), (min.y, 12.3), (max.x, 26.6), (max.y, 26.6)] {
        assert!((got - want).abs() < 0.1, "{got} {want}: {svg}");
    }
    assert!(
        (square.area() - 14.3 * 14.3).abs() < 2.5,
        "{}: {svg}",
        square.area()
    );
}

#[test]
fn a_blob_at_an_edge_a_small_one_and_a_faint_one_stay_merged() {
    // Against the picture's border.
    let (svg, stats) = recover_svg(MERGED, &picture(0., 14., false), false).unwrap();
    assert_eq!(stats.recovered, 0, "{svg}");
    assert_eq!(svg, MERGED);
    // Two pixels square: no core.
    let (_, stats) = recover_svg(MERGED, &picture(18., 20., false), false).unwrap();
    assert_eq!(stats, RecoveryStats::default());
    // 5 px thick, as JPEG's colour blocks left in a compressed logo.
    let mut thin = picture(12., 26., false);
    for (i, p) in thin.pixels.iter_mut().enumerate() {
        if i % 40 >= 17 {
            p.0 = [FIELD[0], FIELD[1], FIELD[2], 255];
        }
    }
    let (_, stats) = recover_svg(MERGED, &thin, false).unwrap();
    assert_eq!(stats.recovered, 0);
    // Six levels off: below the contrast asked.
    let mut faint = picture(12., 26., false);
    for p in &mut faint.pixels {
        if p.0[2] == SQUARE[2] {
            p.0[2] = FIELD[2] + 6;
        }
    }
    let (_, stats) = recover_svg(MERGED, &faint, false).unwrap();
    assert_eq!(stats.recovered, 0);
}

#[test]
fn a_gradient_has_no_one_colour_core() {
    // Blue rising 1.5 levels a pixel across the middle: 36 levels in all,
    // no two neighbours more than 2 apart.
    let pixels = (0..40 * 40)
        .map(|i| {
            let x = (i % 40) as u8;
            let rise = if (8..32).contains(&x) {
                3 * (x - 8) / 2
            } else {
                0
            };
            Rgba([FIELD[0], FIELD[1], FIELD[2] + rise, 255])
        })
        .collect();
    let (_, stats) = recover_svg(
        MERGED,
        &Raster {
            width: 40,
            height: 40,
            pixels,
        },
        true,
    )
    .unwrap();
    assert_eq!(stats.recovered, 0);
}

#[test]
fn lone_pixels_and_a_dotted_line_on_an_exact_palette_come_back_as_their_squares() {
    // A lone red pixel and a dotted black line of five dots on white, all
    // of them merged into the field.
    let pixels = (0..40 * 40)
        .map(|i| match (i % 40, i / 40) {
            (10, 10) => Rgba([220, 30, 30, 255]),
            (20 | 22 | 24 | 26 | 28, 20) => Rgba([0, 0, 0, 255]),
            _ => Rgba([255, 255, 255, 255]),
        })
        .collect();
    let source = Raster {
        width: 40,
        height: 40,
        pixels,
    };
    let white = MERGED.replace("#2c5aa0", "#ffffff");
    let (svg, stats) = recover_exact(&white, &source).unwrap();
    assert_eq!(stats.recovered, 6, "{svg}");
    let found = islands(&svg).unwrap();
    assert_eq!(
        found.iter().filter(|i| i.color == "#000000").count(),
        5,
        "{svg}"
    );
    assert!(
        found
            .iter()
            .filter(|i| i.color != "#ffffff")
            .all(|i| (i.area() - 1.).abs() < 1e-9),
        "{svg}"
    );
    // The thickness test keeps them out otherwise.
    assert_eq!(recover_svg(&white, &source, false).unwrap().1.recovered, 0);
}

#[test]
fn a_step_beside_a_shape_drawn_in_its_colour_is_no_lost_feature() {
    // A black bar drawn at rows 10 to 12 and a black pixel touching its
    // corner that the outline cut off: part of the bar, left as traced.
    let pixels = (0..40 * 40)
        .map(|i| match (i % 40, i / 40) {
            (10..30, 10..13) | (30, 13) => Rgba([0, 0, 0, 255]),
            _ => Rgba([255, 255, 255, 255]),
        })
        .collect();
    let source = Raster {
        width: 40,
        height: 40,
        pixels,
    };
    let bar = "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">
<g id=\"#ffffffff\">
<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 L 40.00 40.00 L 0.00 40.00 L 0.00 0.00 M 10.00 10.00 L 10.00 13.00 L 30.00 13.00 L 30.00 10.00 L 10.00 10.00 Z\" />
</g>
<g id=\"#000000ff\">
<path fill=\"#000000\" opacity=\"1.00\" d=\" M 10.00 10.00 L 30.00 10.00 L 30.00 13.00 L 10.00 13.00 L 10.00 10.00 Z\" />
</g>
</svg>
";
    let (svg, stats) = recover_exact(bar, &source).unwrap();
    assert_eq!(stats.recovered, 0, "{svg}");
}
