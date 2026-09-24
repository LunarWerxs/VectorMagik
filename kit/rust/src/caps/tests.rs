use super::*;
/// A pixel-edged stroke from (10, 30) to (90, 30) traced as a blade: both
/// sides run to one pointed node at each end, the field cut round it.
const BLADE: &str = "<svg width=\"100pt\" height=\"60pt\" viewBox=\"0 0 100 60\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 100.00 0.00 L 100.00 60.00 L 0.00 60.00 L 0.00 0.00 M 10.00 30.00 L 50.00 30.50 L 90.00 30.00 L 50.00 29.50 L 10.00 30.00 Z\" />\n</g>\n<g id=\"#000000ff\">\n<path fill=\"#000000\" opacity=\"1.00\" d=\" M 10.00 30.00 L 50.00 29.50 L 90.00 30.00 L 50.00 30.50 L 10.00 30.00 Z\" />\n</g>\n</svg>\n";

#[test]
fn a_blade_s_points_become_square_ends_on_both_sides_of_the_edge() {
    let (out, stats) = square_caps(BLADE).unwrap();
    assert_eq!(stats.caps, 2, "{out}");
    // The lens is 0.5 px wide on average: each end is two corners 0.25 px
    // either side of the axis, and neither copy keeps the point.
    let paths: Vec<&str> = out.split(" d=\"").skip(1).collect();
    for corner in ["10.00 29.75", "10.00 30.25", "90.00 29.75", "90.00 30.25"] {
        assert!(paths.iter().all(|d| d.contains(corner)), "{corner}: {out}");
    }
    assert!(
        !out.contains("10.00 30.00") && !out.contains("90.00 30.00"),
        "{out}"
    );
    // The two still tile the picture.
    let found = islands(&out).unwrap();
    let total: f64 = found.iter().map(|i| i.area()).sum();
    assert!((total - 6000.).abs() < 1e-6, "{total}: {out}");
    let stroke = found.iter().find(|i| i.color == "#000000").unwrap();
    assert!(stroke.area() > 40., "{}", stroke.area());
}

#[test]
fn a_wide_shape_s_sharp_corner_is_no_tip() {
    // The same outline scaled across: 20 px wide, no stroke.
    let wide = BLADE.replace("29.50", "20.00").replace("30.50", "40.00");
    let (out, stats) = square_caps(&wide).unwrap();
    assert_eq!(stats, CapStats::default());
    assert_eq!(out, wide);
}

#[test]
fn a_two_piece_lens_gets_a_square_end_at_both_points() {
    // A 1 px bar the engine traced as two curves bulging out between two
    // pointed nodes (a pixel-edged bar 64 px long, the defect sweep's lens).
    let lens = "<svg width=\"40pt\" height=\"100pt\" viewBox=\"0 0 40 100\">
<g id=\"#ffffffff\">
<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 40.00 0.00 L 40.00 100.00 L 0.00 100.00 L 0.00 0.00 M 20.35 19.66 C 21.35 41.20 21.27 62.79 20.65 84.34 C 19.65 62.80 19.73 41.21 20.35 19.66 Z\" />
</g>
<g id=\"#000000ff\">
<path fill=\"#000000\" opacity=\"1.00\" d=\" M 20.65 84.34 C 19.65 62.80 19.73 41.21 20.35 19.66 C 21.35 41.20 21.27 62.79 20.65 84.34 Z\" />
</g>
</svg>
";
    let (out, stats) = square_caps(lens).unwrap();
    assert_eq!(stats.caps, 2, "{out}");
    let found = islands(&out).unwrap();
    let total: f64 = found.iter().map(|i| i.area()).sum();
    assert!((total - 4000.).abs() < 1e-6, "{total}: {out}");
    let bar = found.iter().find(|i| i.color == "#000000").unwrap();
    assert!(bar.max.x - bar.min.x > 0.6, "{out}");
}

#[test]
fn a_curved_thin_stroke_keeps_its_points() {
    // A crescent 1 px thick whose middle bows 12 px off the line between
    // its tips (a letter's curved stroke): no cap.
    let crescent = BLADE.replace("29.50", "18.00").replace("30.50", "19.00");
    let (out, stats) = square_caps(&crescent).unwrap();
    assert_eq!(stats, CapStats::default());
    assert_eq!(out, crescent);
}
