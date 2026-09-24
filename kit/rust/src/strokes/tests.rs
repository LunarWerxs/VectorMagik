use super::*;
use crate::raster::Rgba;

/// A black stroke `width` px wide from (10, 30) to (50, 30) on white,
/// anti-aliased by 8 x 8 samples, and the engine's trace of it: a grey
/// band `traced` px wide around the same line, cut out of the white field.
fn stroke(width: f64, traced: f64, grey: u8) -> (Raster, String) {
    let pixels = (0..60 * 60)
        .map(|i| {
            let (x, y) = ((i % 60) as f64, (i / 60) as f64);
            let hits = (0..64)
                .filter(|k| {
                    let (sx, sy) = (
                        x + ((k % 8) as f64 + 0.5) / 8.,
                        y + ((k / 8) as f64 + 0.5) / 8.,
                    );
                    (10. ..=50.).contains(&sx) && (sy - 30.).abs() <= width / 2.
                })
                .count();
            let v = (255. * (1. - hits as f64 / 64.)).round() as u8;
            Rgba([v, v, v, 255])
        })
        .collect();
    let (t, b) = (30. - traced / 2., 30. + traced / 2.);
    let band =
        format!(" M 10.00 {t:.2} L 50.00 {t:.2} L 50.00 {b:.2} L 10.00 {b:.2} L 10.00 {t:.2} Z");
    let hole =
        format!(" M 10.00 {t:.2} L 10.00 {b:.2} L 50.00 {b:.2} L 50.00 {t:.2} L 10.00 {t:.2}");
    let svg = format!(
        "<svg width=\"60pt\" height=\"60pt\" viewBox=\"0 0 60 60\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 60.00 0.00 L 60.00 60.00 L 0.00 60.00 L 0.00 0.00{hole} Z\" />\n</g>\n<g id=\"#{grey:02x}{grey:02x}{grey:02x}ff\">\n<path fill=\"#{grey:02x}{grey:02x}{grey:02x}\" opacity=\"1.00\" d=\"{band}\" />\n</g>\n</svg>\n"
    );
    (
        Raster {
            width: 60,
            height: 60,
            pixels,
        },
        svg,
    )
}

#[test]
fn a_grey_widened_stroke_is_drawn_black_at_its_width() {
    // Two pixels black, traced 2.25 px of #262626 (the defect sweep's
    // numbers): drawn black, 2 px wide, the ink the same.
    let (source, svg) = stroke(2., 2.25, 0x26);
    let (out, stats) = ink_strokes(&svg, &source).unwrap();
    assert_eq!((stats.strokes, stats.inked), (1, 1), "{out}");
    let found = islands(&out).unwrap();
    let band = found.iter().find(|i| i.color != "#ffffff").unwrap();
    let v = u8::from_str_radix(&band.color[1..3], 16).unwrap();
    assert!(v <= 8, "{}", band.color);
    let (top, bottom) = (band.min.y, band.max.y);
    assert!((bottom - top - 2.).abs() < 0.15, "{top} {bottom}: {out}");
    // The field's hole moved with it: the two still tile the picture.
    let total: f64 = found.iter().map(|i| i.area()).sum();
    assert!((total - 3600.).abs() < 1e-6, "{total}");
}

#[test]
fn a_stroke_already_in_its_ink_and_a_wide_band_are_left_alone() {
    // Traced at its width in black: nothing to gain.
    let (source, svg) = stroke(2., 2., 0x00);
    let (out, stats) = ink_strokes(&svg, &source).unwrap();
    assert_eq!(stats.inked, 0);
    assert_eq!(out, svg);
    // Eight pixels wide: no stroke.
    let (source, svg) = stroke(8., 8., 0x10);
    assert_eq!(
        ink_strokes(&svg, &source).unwrap().1,
        StrokeStats::default()
    );
}
