#![cfg(feature = "desktop")]
//! Crisp region rendering and the automatic tolerance: geometry must agree
//! with the base preview, and Auto must pick a tolerance that keeps the
//! engine's picture.
use std::path::PathBuf;
use vector_magic_rebuild::engine::{vectorize, Options as EngineOptions};
use vector_magic_rebuild::{
    auto_simplify_tolerance, changed_fraction, load_raster, preview, preview_pixels, preview_tree,
    render_region,
    AUTO_TOLERANCE_CANDIDATES, AUTO_TOLERANCE_CHANGE_LIMIT,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn a_region_at_the_preview_scale_matches_the_preview_pixel_for_pixel() {
    let svg = std::fs::read_to_string(
        root().join("kit/fixtures/reference/logo-with-transparency-high.svg"),
    )
    .unwrap();
    let base = preview(&svg).unwrap();
    let tree = preview_tree(&svg).unwrap();
    // The document is 250 viewBox units wide but declared in points, so the
    // parsed tree is 4/3 larger. `preview` scales the tree by at most 4;
    // the same picture in screen pixels per viewBox unit is that factor
    // times the point-to-pixel ratio.
    let size = tree.size();
    let factor = (1600. / size.width().max(size.height())).min(4.);
    let unit = size.width() / 250.;
    assert!((unit - 4. / 3.).abs() < 1e-3, "points to pixels: {unit}");
    let scale = factor * unit;
    let region = render_region(
        &tree,
        250.,
        scale,
        [0., 0.],
        [base.size[0] as u32, base.size[1] as u32],
    )
    .unwrap();
    assert_eq!(region.size, base.size);
    let worst = base
        .pixels
        .iter()
        .zip(&region.pixels)
        .map(|(a, b)| {
            let a = a.to_array();
            let b = b.to_array();
            (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap()
        })
        .max()
        .unwrap();
    assert!(worst <= 2, "largest channel difference {worst}");
    // A small window at double that scale shows exactly the requested part of
    // the document, not a third-larger picture. Take a point the base preview
    // shows as blue gear body with blue all around it, and centre a zoomed
    // window on that document point: it must be blue there too.
    let blue = |p: [u8; 4]| p[2] > 150 && p[0] < 160 && p[1] < 160;
    let width = base.size[0];
    let at = |x: usize, y: usize| base.pixels[y * width + x].to_array();
    let (bx, by) = (0..base.size[1])
        .step_by(4)
        .flat_map(|y| (0..width).step_by(4).map(move |x| (x, y)))
        .find(|&(x, y)| {
            x >= 12
                && y >= 12
                && x + 12 < width
                && y + 12 < base.size[1]
                && (-12i32..=12).step_by(4).all(|dx| {
                    (-12i32..=12)
                        .step_by(4)
                        .all(|dy| blue(at((x as i32 + dx) as usize, (y as i32 + dy) as usize)))
                })
        })
        .expect("the gear sample has a solid blue area");
    let unit_x = bx as f32 / scale;
    let unit_y = by as f32 / scale;
    let zoom = scale * 2.;
    let origin = [unit_x - 200. / zoom, unit_y - 200. / zoom];
    let zoomed = render_region(&tree, 250., zoom, origin, [400, 400]).unwrap();
    let centre = zoomed.pixels[200 * 400 + 200].to_array();
    assert!(
        blue(centre),
        "{centre:?} at document ({unit_x:.1}, {unit_y:.1})"
    );
    assert!(render_region(&tree, 0., scale, [0., 0.], [10, 10]).is_err());
    assert!(render_region(&tree, 250., scale, [0., 0.], [0, 10]).is_err());
}

#[test]
fn auto_tolerance_keeps_the_engines_picture() {
    let source = root().join("kit/fixtures/samples/logo-with-transparency.png");
    let raster = load_raster(&source).unwrap();
    let raw = vectorize(&raster, EngineOptions::default()).unwrap();
    let (tolerance, document) = auto_simplify_tolerance(&raw).unwrap();
    assert!(AUTO_TOLERANCE_CANDIDATES.contains(&tolerance));
    assert_eq!(document.simplify_tolerance, Some(tolerance));
    let reference = preview_pixels(raw.svg()).unwrap();
    let image = preview_pixels(document.svg()).unwrap();
    let fraction = changed_fraction(&reference, &image);
    assert!(
        fraction <= AUTO_TOLERANCE_CHANGE_LIMIT || tolerance == AUTO_TOLERANCE_CANDIDATES[0],
        "{fraction} at {tolerance}"
    );
    assert!(document.segment_count() <= raw.segment_count());
    println!(
        "auto tolerance {tolerance} changes {:.3}% of pixels",
        fraction * 100.
    );
    // Anti-aliased artwork stops at 0.3 px (its edges carry their place
    // between pixels); the gear passes there, the largest it may take.
    assert_eq!(tolerance, 0.3);
}

#[test]
fn auto_tolerance_cap_follows_the_kind_of_picture() {
    use vector_magic_rebuild::auto_tolerance_cap;
    for preset in [0, 1, 2] {
        assert_eq!(auto_tolerance_cap(preset), 0.5, "aliased preset {preset}");
    }
    for preset in [3, 4, 5] {
        assert_eq!(auto_tolerance_cap(preset), 0.3, "anti-aliased preset {preset}");
    }
    for preset in [6, 7, 8] {
        assert_eq!(auto_tolerance_cap(preset), 0.1, "photograph preset {preset}");
    }
}
