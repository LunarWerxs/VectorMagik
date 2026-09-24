use vector_magic_rebuild::{auto::detect, load_raster};
use vector_rebuild::raster::{Raster, Rgba};
use vector_rebuild::{ImageCategory, Quality};

#[test]
fn automatic_category_matches_supplied_logos_and_photos() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut cases = Vec::new();
    for name in [
        "Logo With Blending Small",
        "Logo With Blending",
        "Logo With Transparency",
        "Logo Without Blending",
    ] {
        cases.push((
            root.join(format!(
                "kit/fixtures/samples/{}.png",
                name.to_lowercase().replace(' ', "-")
            )),
            if name.contains("Without") {
                ImageCategory::AliasedArtwork
            } else {
                ImageCategory::AntiAliasedArtwork
            },
        ));
    }
    for name in ["astronaut", "chelsea", "coffee"] {
        cases.push((
            root.join(format!("kit/fixtures/photos/{name}.png")),
            ImageCategory::Photograph,
        ));
    }
    for (path, expected) in cases {
        let result = detect(&load_raster(&path).unwrap());
        assert_eq!(result.category, expected, "{}", path.display());
        assert_eq!(result.quality, Quality::High);
        assert!([
            result.flat_fraction,
            result.palette_coverage,
            result.soft_edge_fraction,
            result.blend_edge_fraction
        ]
        .iter()
        .all(|v| v.is_finite() && (0. ..=1.).contains(v)));
    }
}

/// The exact-geometry shapes are two to four colours with anti-aliased
/// rims. The S, one edge across 180 px, was called aliased while the rule
/// only counted soft edges and palette over the whole picture, and traced
/// with the aliased preset its rim became sliver regions.
#[test]
fn anti_aliased_shapes_with_few_colours_are_blended() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/shapes");
    for name in [
        "shape-s-curve",
        "shape-circles",
        "shape-ellipse-roundrect",
        "shape-outline-band",
    ] {
        let result = detect(&load_raster(&root.join(format!("{name}.png"))).unwrap());
        assert_eq!(result.category, ImageCategory::AntiAliasedArtwork, "{name}");
        assert!(result.blend_edge_fraction > 0.3, "{name} {result:?}");
    }
    let unblended = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/samples/logo-without-blending.png");
    assert_eq!(
        detect(&load_raster(&unblended).unwrap()).blend_edge_fraction,
        0.
    );
}

/// A photograph scaled up is still a photograph: at 4x Auto called the
/// astronaut blended artwork, flat pixel to pixel, and traced it nine times
/// slower (the defect sweep of September 23, 2026). The fixtures scaled 3x
/// with Lanczos, and each scaled 2x and 4x with the app's own filter.
#[test]
fn an_enlarged_photograph_is_still_a_photograph() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/photos");
    for name in ["astronaut", "chelsea", "coffee"] {
        let x3 = detect(&load_raster(&root.join(format!("{name}-x3.png"))).unwrap());
        assert_eq!(x3.category, ImageCategory::Photograph, "{name}-x3 {x3:?}");
        let raster = load_raster(&root.join(format!("{name}.png"))).unwrap();
        for k in [2, 4] {
            let big = vector_magic_rebuild::scaled_raster(&raster, raster.width * k, raster.height * k);
            let found = detect(&big);
            assert_eq!(found.category, ImageCategory::Photograph, "{name} x{k} {found:?}");
        }
    }
}

/// A two-colour disc, its rim covered by 8x8 samples per pixel or not at all.
fn disc(size: usize, radius: f64, anti_aliased: bool) -> Raster {
    let (ink, paper) = ([20., 40., 200.], [250., 240., 230.]);
    let samples = if anti_aliased { 8 } else { 1 };
    let centre = size as f64 / 2.;
    let pixels = (0..size * size)
        .map(|i| {
            let (x, y) = ((i % size) as f64, (i / size) as f64);
            let mut inside = 0;
            for sy in 0..samples {
                for sx in 0..samples {
                    let dx = x + (sx as f64 + 0.5) / samples as f64 - centre;
                    let dy = y + (sy as f64 + 0.5) / samples as f64 - centre;
                    inside += usize::from(dx * dx + dy * dy < radius * radius);
                }
            }
            let cover = inside as f64 / (samples * samples) as f64;
            let mix = |c: usize| (paper[c] * (1. - cover) + ink[c] * cover).round() as u8;
            Rgba([mix(0), mix(1), mix(2), 255])
        })
        .collect();
    Raster {
        width: size,
        height: size,
        pixels,
    }
}

/// Whether artwork is blended does not depend on how much of the picture
/// its edges fill: the same disc is blended when anti-aliased and aliased
/// when not, from 64 px to 1200 px (sampled with a stride there).
#[test]
fn anti_aliasing_is_found_at_any_picture_size() {
    for (size, radius) in [(64, 20.), (400, 150.), (1200, 500.)] {
        let aliased = detect(&disc(size, radius, false));
        assert_eq!(aliased.category, ImageCategory::AliasedArtwork, "{size}");
        assert_eq!(aliased.blend_edge_fraction, 0., "{size}");
        let blended = detect(&disc(size, radius, true));
        assert_eq!(
            blended.category,
            ImageCategory::AntiAliasedArtwork,
            "{size} {blended:?}"
        );
    }
}

#[test]
fn automatic_quality_tracks_small_source_resolution_and_ignores_hidden_rgb() {
    // Under 48 px a picture needs all its detail: high, not low (round two
    // of the Opus 5.5 review; auto.rs has the measurements).
    for (size, quality) in [
        (32, Quality::High),
        (47, Quality::High),
        (48, Quality::Medium),
        (96, Quality::Medium),
        (159, Quality::Medium),
        (256, Quality::High),
    ] {
        let mut raster = Raster {
            width: size,
            height: size,
            pixels: (0..size * size)
                .map(|i| {
                    if i % size < size / 2 {
                        Rgba([0, 0, 0, 255])
                    } else {
                        Rgba([255, 255, 255, 255])
                    }
                })
                .collect(),
        };
        let result = detect(&raster);
        assert_eq!(result.category, ImageCategory::AliasedArtwork);
        assert_eq!(result.quality, quality);
        for (i, p) in raster.pixels.iter_mut().enumerate() {
            if i % size >= size / 2 {
                *p = Rgba([i as u8, (i * 7) as u8, (i * 11) as u8, 0]);
            }
        }
        assert_eq!(detect(&raster).category, ImageCategory::AliasedArtwork);
    }
}
