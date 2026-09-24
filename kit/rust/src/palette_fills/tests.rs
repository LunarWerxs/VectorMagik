use super::*;
use crate::raster::Rgba;

/// A white picture with the pixels `ink` says black.
fn picture(w: usize, h: usize, ink: impl Fn(usize, usize) -> bool) -> Raster {
    let pixels = (0..w * h)
        .map(|i| {
            if ink(i % w, i / w) {
                Rgba([0, 0, 0, 255])
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

/// A white field with each `(d, fill)` cut out of it as a hole and drawn
/// in its fill.
fn document(w: usize, h: usize, regions: &[(&str, &str)]) -> String {
    let mut field = format!(" M 0.00 0.00 L {w}.00 0.00 L {w}.00 {h}.00 L 0.00 {h}.00 L 0.00 0.00");
    let mut groups = String::new();
    for (d, fill) in regions {
        let hole: Vec<&str> = d.trim().trim_end_matches(" Z").split(" L ").collect();
        let start = hole[0].trim_start_matches("M ");
        field.push_str(&format!(" M {start}"));
        for p in hole[1..].iter().rev() {
            field.push_str(&format!(" L {p}"));
        }
        field.push_str(&format!(" L {start}"));
        groups.push_str(&format!(
            "<g id=\"{fill}ff\">\n<path fill=\"{fill}\" opacity=\"1.00\" d=\"{d}\" />\n</g>\n"
        ));
    }
    format!(
        "<svg width=\"{w}pt\" height=\"{h}pt\" viewBox=\"0 0 {w} {h}\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\"{field} Z\" />\n</g>\n{groups}</svg>\n"
    )
}

#[test]
fn one_pixel_diagonals_snapped_to_a_third_colour_or_to_their_field_come_back_black() {
    // Two 1 px black diagonals on white. The first traced as the engine
    // does, a band 1.27 px wide a third of a pixel off its pixels, and
    // snapped to purple, the palette colour nearest its blend with white;
    // the second on its pixels and snapped to white.
    let source = picture(60, 40, |x, y| {
        (5..25).contains(&y) && (x == y || x == y + 30)
    });
    let purple = " M 5.16 4.67 L 25.42 24.80 L 24.54 25.72 L 4.26 5.57 Z";
    let white = " M 35.35 4.65 L 55.35 24.65 L 54.65 25.35 L 34.65 5.35 Z";
    let svg = document(60, 40, &[(purple, "#7c3aed"), (white, "#ffffff")]);
    let (out, stats) = fills_from_pixels(&svg, &source).unwrap();
    assert_eq!(stats.recoloured, 2, "{out}");
    assert_eq!(out.matches("fill=\"#000000\"").count(), 2, "{out}");
    // The field keeps its colour.
    assert!(
        out.contains("<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00"),
        "{out}"
    );
}

#[test]
fn a_field_split_in_two_keeps_its_colour_over_a_merged_line() {
    // The engine split a white field in two and merged a 1 px diagonal into
    // them: a twentieth of each half is black, and both halves stay white.
    let source = picture(40, 40, |x, y| x == y);
    let left = " M 0.00 0.00 L 20.00 0.00 L 20.00 40.00 L 0.00 40.00 L 0.00 0.00 Z";
    let right = " M 20.00 0.00 L 40.00 0.00 L 40.00 40.00 L 20.00 40.00 L 20.00 0.00 Z";
    let svg = format!(
        "<svg width=\"40pt\" height=\"40pt\" viewBox=\"0 0 40 40\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\"{left}\" />\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\"{right}\" />\n</g>\n</svg>\n"
    );
    let (out, stats) = fills_from_pixels(&svg, &source).unwrap();
    assert_eq!(stats.recoloured, 0);
    assert_eq!(out, svg);
}
