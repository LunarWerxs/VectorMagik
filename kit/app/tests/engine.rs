//! The owned engine against the original's frozen output: 21 conversions
//! (seven images at three qualities, every basic preset) and seven more with
//! the optional pass on, byte for byte; the owner-accepted photo default
//! against its three frozen references; then the application behaviours the
//! desktop relies on. No original binary or process is involved.
use vector_magic_rebuild::engine::{
    blended_defaults, effective_advanced, parse_advanced, photo_detail_ceiling, vectorize, Options,
};
use vector_rebuild::geometry::{Cubic, Point};
use vector_rebuild::raster::{Raster, Rgba};
use vector_rebuild::{ImageCategory, Quality};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The seven images with their categories and the slugs of their frozen
/// references in kit/fixtures/reference.
fn samples() -> Vec<(std::path::PathBuf, ImageCategory, &'static str)> {
    let root = root();
    let mut cases = vec![
        (
            root.join("kit/fixtures/samples/logo-with-blending-small.png"),
            ImageCategory::AntiAliasedArtwork,
            "logo-with-blending-small",
        ),
        (
            root.join("kit/fixtures/samples/logo-with-blending.png"),
            ImageCategory::AntiAliasedArtwork,
            "logo-with-blending",
        ),
        (
            root.join("kit/fixtures/samples/logo-with-transparency.png"),
            ImageCategory::AntiAliasedArtwork,
            "logo-with-transparency",
        ),
        (
            root.join("kit/fixtures/samples/logo-without-blending.png"),
            ImageCategory::AliasedArtwork,
            "logo-without-blending",
        ),
    ];
    for name in ["astronaut", "chelsea", "coffee"] {
        cases.push((
            root.join(format!("kit/fixtures/photos/{name}.png")),
            ImageCategory::Photograph,
            name,
        ));
    }
    cases
}

fn reference(slug: &str, suffix: &str) -> String {
    let path = root().join(format!("kit/fixtures/reference/{slug}-{suffix}.svg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn engine_matches_the_original_on_seven_images_at_all_three_qualities() {
    let mut presets = std::collections::BTreeSet::new();
    for (path, category, slug) in samples() {
        let raster = vector_magic_rebuild::load_raster(&path).unwrap();
        for (quality, suffix) in [
            (Quality::High, "high"),
            (Quality::Medium, "medium"),
            (Quality::Low, "low"),
        ] {
            let document = vectorize(
                &raster,
                Options {
                    category,
                    quality,
                    overlap_opaque_photos: false,
                    optional_optimizer: false,
                    advanced: None,
                    owned_defaults: false,
                    basic_preset: false,
                },
            )
            .unwrap();
            presets.insert(document.preset);
            assert!(!document.photo_overlap && document.regions > 0 && document.curves > 0);
            assert_eq!(
                document.svg,
                reference(slug, suffix),
                "{slug} at {suffix} differs from the original"
            );
        }
    }
    assert_eq!(presets.len(), 9, "every basic preset was compared");
}

/// The improved defaults (September 22, 2026, from the measured sheets in
/// testing/quality-round): high-quality photographs trace at the
/// advanced-mode detail ceiling and blended artwork at the advanced
/// defaults, unless told otherwise; unblended artwork and the other
/// qualities keep the original's presets; the `-high-detail` references
/// freeze what was accepted.
#[test]
fn high_quality_photographs_and_blended_artwork_run_the_improved_defaults() {
    let ceiling = photo_detail_ceiling();
    assert_eq!(
        ceiling,
        parse_advanced("12,6,6", ImageCategory::Photograph).unwrap()
    );
    let blended = blended_defaults();
    assert_eq!(
        blended,
        parse_advanced("11,3,6", ImageCategory::AntiAliasedArtwork).unwrap()
    );
    for (path, category, name) in samples() {
        let expected = match category {
            ImageCategory::Photograph => Some(ceiling),
            ImageCategory::AntiAliasedArtwork => Some(blended),
            ImageCategory::AliasedArtwork => None,
        };
        let raster = vector_magic_rebuild::load_raster(&path).unwrap();
        let options = Options {
            category,
            overlap_opaque_photos: false,
            ..Default::default()
        };
        assert_eq!(effective_advanced(&options), expected, "{name}");
        for quality in [Quality::Medium, Quality::Low] {
            assert_eq!(effective_advanced(&Options { quality, ..options }), None);
        }
        assert_eq!(
            effective_advanced(&Options {
                owned_defaults: false,
                basic_preset: false,
                ..options
            }),
            None
        );
        let Some(expected) = expected else {
            continue;
        };
        let document = vectorize(&raster, options).unwrap();
        assert_eq!(document.advanced, Some(expected));
        // The reference is the file as saved, its numbers written short.
        assert_eq!(
            vector_magic_rebuild::export::compact_svg(&document.svg),
            reference(name, "high-detail"),
            "{name}: the improved default differs from the accepted one"
        );
        assert_ne!(document.svg, reference(name, "high"));
    }
    let explicit = parse_advanced("5,6,6", ImageCategory::Photograph).unwrap();
    assert_eq!(
        effective_advanced(&Options {
            category: ImageCategory::Photograph,
            advanced: Some(explicit),
            ..Default::default()
        }),
        Some(explicit)
    );
}

#[test]
fn owned_optional_pass_matches_the_original_on_seven_images() {
    let mut changed = 0;
    for (path, category, slug) in samples() {
        let raster = vector_magic_rebuild::load_raster(&path).unwrap();
        let options = Options {
            category,
            quality: Quality::High,
            overlap_opaque_photos: false,
            optional_optimizer: true,
            advanced: None,
            owned_defaults: false,
            basic_preset: false,
        };
        let optimized = vectorize(&raster, options).unwrap();
        assert!(optimized.optional_optimizer && optimized.optimizer_unknowns > 0);
        assert_eq!(
            optimized.svg,
            reference(slug, "high-optimizer"),
            "{slug}: the owned optional pass differs from the original's"
        );
        if optimized.svg != reference(slug, "high") {
            changed += 1;
        }
    }
    assert!(
        changed >= 5,
        "the optional pass moved only {changed} of seven outputs"
    );
}

#[cfg(feature = "desktop")]
fn render(svg: &str, w: u32, h: u32) -> resvg::tiny_skia::Pixmap {
    let tree = resvg::usvg::Tree::from_str(svg, &Default::default()).unwrap();
    let mut pixels = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            w as f32 / tree.size().width(),
            h as f32 / tree.size().height(),
        ),
        &mut pixels.as_mut(),
    );
    pixels
}

#[test]
#[cfg(feature = "desktop")]
fn presets_preserve_color_channels_holes_and_preview_geometry() {
    let raster = Raster {
        width: 96,
        height: 96,
        pixels: (0..96 * 96)
            .map(|i| {
                let (x, y) = (i % 96, i / 96);
                Rgba(
                    if (8..88).contains(&x)
                        && (8..88).contains(&y)
                        && !((38..58).contains(&x) && (38..58).contains(&y))
                    {
                        if x < 32 {
                            [220, 20, 40, 255]
                        } else if x < 64 {
                            [20, 180, 50, 255]
                        } else {
                            [40, 70, 220, 255]
                        }
                    } else {
                        [0, 0, 0, 0]
                    },
                )
            })
            .collect(),
    };
    for category in [
        ImageCategory::AliasedArtwork,
        ImageCategory::AntiAliasedArtwork,
        ImageCategory::Photograph,
    ] {
        for quality in [Quality::High, Quality::Medium, Quality::Low] {
            let doc = vectorize(
                &raster,
                Options {
                    category,
                    quality,
                    ..Default::default()
                },
            )
            .unwrap();
            let pixels = render(doc.svg(), 96, 96);
            assert!(
                !doc.photo_overlap,
                "transparent input must keep the engine's geometry and alpha"
            );
            assert!(
                pixels.pixel(48, 48).unwrap().alpha() < 4,
                "lost hole: {category:?} {quality:?}"
            );
            assert!(
                pixels.pixel(2, 2).unwrap().alpha() < 4,
                "lost exterior transparency"
            );
            for (x, want) in [
                (20, [220, 20, 40]),
                (48, [20, 180, 50]),
                (76, [40, 70, 220]),
            ] {
                let p = pixels.pixel(x, 20).unwrap().demultiply();
                assert!(p.alpha() > 250);
                for (a, b) in [p.red(), p.green(), p.blue()].into_iter().zip(want) {
                    assert!(
                        (a as i32 - b).abs() < 8,
                        "channel mismatch: {category:?} {quality:?}"
                    );
                }
            }
            assert!(!doc.nodes().is_empty());
            assert!(doc
                .nodes()
                .iter()
                .all(|p| p.x.is_finite() && p.y.is_finite()));
            vector_magic_rebuild::preview(doc.svg()).unwrap();
        }
    }
}

#[test]
fn incomplete_input_is_rejected_before_any_work() {
    let incomplete = Raster {
        width: 32,
        height: 32,
        pixels: vec![],
    };
    assert!(vectorize(&incomplete, Default::default())
        .unwrap_err()
        .contains("complete RGBA"));
    let tiny = Raster {
        width: 1,
        height: 1,
        pixels: vec![Rgba([0, 0, 0, 255])],
    };
    assert!(vectorize(&tiny, Default::default()).is_err());
}

#[test]
#[cfg(feature = "desktop")]
fn empty_and_minimum_size_images_remain_valid() {
    for size in [2, 32] {
        for color in [[0, 0, 0, 0], [32, 128, 200, 255]] {
            let raster = Raster {
                width: size,
                height: size,
                pixels: vec![Rgba(color); size * size],
            };
            let doc = vectorize(&raster, Default::default()).unwrap();
            let pixels = render(doc.svg(), size as u32, size as u32);
            let pixel = pixels.pixel(1, 1).unwrap().demultiply();
            assert_eq!(pixel.alpha(), color[3]);
            if color[3] != 0 {
                assert_eq!([pixel.red(), pixel.green(), pixel.blue()], color[..3]);
            }
        }
    }
}

#[test]
#[cfg(feature = "desktop")]
fn photo_export_improves_roundtrip_preserves_engine_curves_and_opaque_canvas() {
    let root = root().join("kit/fixtures/photos");
    let paths = |svg: &str| -> Vec<String> {
        svg.split(" d=\"")
            .skip(1)
            .map(|s| s.split('"').next().unwrap().to_owned())
            .collect()
    };
    for name in ["astronaut", "chelsea", "coffee"] {
        let raster = vector_magic_rebuild::load_raster(&root.join(format!("{name}.png"))).unwrap();
        let options = Options {
            category: ImageCategory::Photograph,
            ..Default::default()
        };
        let plain = vectorize(
            &raster,
            Options {
                overlap_opaque_photos: false,
                ..options
            },
        )
        .unwrap();
        let improved = vectorize(&raster, options).unwrap();
        assert!(improved.photo_overlap && !plain.photo_overlap);
        assert_eq!(
            paths(plain.svg()),
            paths(improved.svg()),
            "export changed the fitted geometry"
        );
        let raw = render(plain.svg(), raster.width as u32, raster.height as u32);
        let fixed = render(improved.svg(), raster.width as u32, raster.height as u32);
        assert!(fixed.pixels().iter().all(|p| p.alpha() == 255));
        let error = |image: &resvg::tiny_skia::Pixmap| -> f64 {
            image
                .pixels()
                .iter()
                .zip(&raster.pixels)
                .map(|(p, source)| {
                    // Pixmap channels are premultiplied; composite over white.
                    [p.red(), p.green(), p.blue()]
                        .into_iter()
                        .zip(source.0)
                        .map(|(a, b)| {
                            (a as i32 + 255 - p.alpha() as i32 - b as i32).unsigned_abs() as f64
                        })
                        .sum::<f64>()
                })
                .sum::<f64>()
                / (raster.pixels.len() * 3) as f64
        };
        assert!(
            error(&fixed) < error(&raw) * 0.85,
            "{name}: {} versus {}",
            error(&fixed),
            error(&raw)
        );
    }
}

#[test]
fn circle_is_round_between_nodes_and_stable_between_conversions() {
    let (cx, cy, radius) = (62.7, 64.2, 37.3);
    let raster = Raster {
        width: 128,
        height: 128,
        pixels: (0..128 * 128)
            .map(|i| {
                let (x, y) = (i % 128, i / 128);
                let mut covered = 0;
                for sy in 0..8 {
                    for sx in 0..8 {
                        let dx = x as f64 + (sx as f64 + 0.5) / 8. - cx;
                        let dy = y as f64 + (sy as f64 + 0.5) / 8. - cy;
                        covered += i32::from(dx * dx + dy * dy < radius * radius);
                    }
                }
                let c = (255. - covered as f64 * 255. / 64.).round() as u8;
                Rgba([c, c, c, 255])
            })
            .collect(),
    };
    let doc = vectorize(&raster, Default::default()).unwrap();
    let again = vectorize(&raster, Default::default()).unwrap();
    assert_eq!(
        doc.svg, again.svg,
        "the seeded perturbation must start from the same seed every time"
    );
    let path = doc
        .svg
        .split("<path ")
        .find(|p| p.starts_with("fill=\"#000000\""))
        .unwrap();
    let mut tokens = path
        .split(" d=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .split_whitespace();
    let mut cursor = Point { x: 0., y: 0. };
    let mut count = 0;
    let mut max_error: f64 = 0.;
    while let Some(command) = tokens.next() {
        let mut point = || Point {
            x: tokens.next().unwrap().parse().unwrap(),
            y: tokens.next().unwrap().parse().unwrap(),
        };
        match command {
            "M" => cursor = point(),
            "C" => {
                let cubic = Cubic {
                    points: [cursor, point(), point(), point()],
                };
                for i in 0..=64 {
                    let p = cubic.evaluate(i as f64 / 64.);
                    max_error = max_error.max(((p.x - cx).hypot(p.y - cy) - radius).abs());
                }
                cursor = cubic.points[3];
                count += 1;
            }
            "Z" => (),
            other => panic!("Circle unexpectedly contains {other}"),
        }
    }
    assert!(
        (4..=12).contains(&count),
        "circle has {count} cubic segments"
    );
    assert!(
        max_error < 0.5,
        "Circle bends away from its true radius by {max_error} source pixels"
    );
}

#[test]
fn anchor_counts_exclude_closed_endpoint_duplicates_in_each_subpath() {
    let raster = vector_magic_rebuild::load_raster(
        &root().join("kit/fixtures/samples/logo-with-blending-small.png"),
    )
    .unwrap();
    let mut doc = vectorize(&raster, Options::default()).unwrap();
    doc.svg = r#"<svg><path d="M 0 0 L 10 0 L 10 10 L 0 0 Z M 2 2 C 3 2 3 3 2 2 Z"/></svg>"#.into();
    assert_eq!(doc.nodes().len(), 4);
    // The engine's own form: a shape and its hole in one path, one Z at the
    // end, each loop returning to its start before the next M.
    doc.svg =
        r#"<svg><path d="M 0 0 L 10 0 L 10 10 L 0 0 M 2 2 L 3 2 L 3 3 L 2 2 Z"/></svg>"#.into();
    assert_eq!(doc.nodes().len(), 6);
}

/// A translucent region keeps its colour under the improved defaults: its
/// fill is the straight colour and its opacity the region's alpha, so it
/// renders over white and black as the source composites. The original
/// defaults keep the original's premultiplied fill, darker by the alpha;
/// otherwise the two documents are the same text.
#[test]
#[cfg(feature = "desktop")]
fn translucent_regions_keep_their_colour_under_the_improved_defaults() {
    // (255, 0, 0) at alpha 128 and (0, 96, 255) at 200 side by side, in a
    // transparent margin.
    let (width, height) = (48usize, 32usize);
    let red = [255u8, 0, 0, 128];
    let blue = [0u8, 96, 255, 200];
    let pixel = |x: usize, y: usize| {
        if !(4..width - 4).contains(&x) || !(4..height - 4).contains(&y) {
            Rgba([0, 0, 0, 0])
        } else if x < width / 2 {
            Rgba(red)
        } else {
            Rgba(blue)
        }
    };
    let raster = Raster {
        width,
        height,
        pixels: (0..width * height)
            .map(|i| pixel(i % width, i / width))
            .collect(),
    };
    for category in [
        ImageCategory::AliasedArtwork,
        ImageCategory::AntiAliasedArtwork,
    ] {
        let convert = |owned_defaults| {
            let options = Options {
                category,
                owned_defaults,
                ..Options::default()
            };
            vectorize(&raster, options).unwrap().svg
        };
        let (original, improved) = (convert(false), convert(true));
        // The original writes the premultiplied means (the blue block's are
        // (0, 75, 199) at alpha 199 or (0, 75, 200) at 200, by category),
        // the improved defaults their straight colours, at the same
        // opacities.
        for (svg, fills) in [
            (&original, ["#800000", "#004bc"]),
            (&improved, ["#ff0000", "#0060ff"]),
        ] {
            for fill in fills {
                assert!(
                    svg.contains(&format!("fill=\"{fill}")),
                    "{category:?} {fill}\n{svg}"
                );
            }
            assert!(svg.contains("opacity=\"0.50\"") && svg.contains("opacity=\"0.78\""));
        }
        if category == ImageCategory::AliasedArtwork {
            // No improved default of its own: the documents differ only in
            // each group's colour, mapped by `straight_colour`.
            let mut expected = original.clone();
            for id in original.split("<g id=\"#").skip(1) {
                let hex = &id[..8];
                let byte = |k: usize| u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).unwrap();
                let [b, g, r, a] = vector_rebuild::recovered_export::straight_colour([
                    byte(2),
                    byte(1),
                    byte(0),
                    byte(3),
                ]);
                let straight = format!("{r:02x}{g:02x}{b:02x}");
                expected = expected
                    .replace(&format!("#{hex}"), &format!("#{straight}{a:02x}"))
                    .replace(
                        &format!("fill=\"#{}\"", &hex[..6]),
                        &format!("fill=\"#{straight}\""),
                    );
            }
            assert_ne!(expected, original);
            assert_eq!(improved, expected);
        }
        // Each block's centre over white and over black, against the source
        // composited the same way.
        let composite = |c: [u8; 4], background: f64| {
            let a = f64::from(c[3]) / 255.;
            [0, 1, 2].map(|k| f64::from(c[k]) * a + background * (1. - a))
        };
        for (svg, straight) in [(&original, false), (&improved, true)] {
            let rendered = render(svg, width as u32, height as u32);
            for (x, source) in [(14u32, red), (34, blue)] {
                let p = rendered.pixel(x, 16).unwrap();
                let premultiplied = [p.red(), p.green(), p.blue()].map(f64::from);
                let alpha = f64::from(p.alpha());
                for background in [0., 255.] {
                    let expected = composite(source, background);
                    let error = (0..3)
                        .map(|k| {
                            let shown = premultiplied[k] + background * (1. - alpha / 255.);
                            (shown - expected[k]).abs()
                        })
                        .fold(0., f64::max);
                    // The premultiplied fill is scaled by the opacity a
                    // second time: dark by 44 to 64 levels over either.
                    if straight {
                        assert!(error <= 3., "{category:?} x {x} over {background}: {error}");
                    } else {
                        assert!(
                            error >= 30.,
                            "{category:?} x {x} over {background}: {error}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_pixel_edged_picture_with_few_colours_is_filled_with_exactly_them() {
    // Two colours, a block and a 1 px diagonal: the engine fills each region
    // with its pixels' mean, and a hairline merged with the white came back
    // grey (the defect sweep of September 23, 2026). Under the improved
    // defaults every fill is one of the picture's colours; the original's
    // presets keep their means.
    let (w, h) = (60, 40);
    let ink = [26, 26, 26, 255];
    let mut pixels = vec![Rgba([255, 255, 255, 255]); w * h];
    for y in 8..20 {
        for x in 8..20 {
            pixels[y * w + x] = Rgba(ink);
        }
    }
    for k in 0..30 {
        pixels[(5 + k) * w + 25 + k] = Rgba(ink);
    }
    let raster = Raster { width: w, height: h, pixels };
    let fills = |owned_defaults: bool| -> Vec<String> {
        let options = Options {
            category: ImageCategory::AliasedArtwork,
            quality: Quality::High,
            owned_defaults,
            ..Options::default()
        };
        let svg = vectorize(&raster, options).unwrap().svg;
        let mut found: Vec<String> = svg
            .split("fill=\"")
            .skip(1)
            .map(|s| s[..7].to_owned())
            .collect();
        found.sort();
        found.dedup();
        found
    };
    for fill in fills(true) {
        assert!(fill == "#1a1a1a" || fill == "#ffffff", "{fill}");
    }
    // A picture with a partly transparent pixel has no exact palette.
    let mut soft = raster.clone();
    soft.pixels[0] = Rgba([255, 255, 255, 128]);
    assert!(vectorize(
        &soft,
        Options {
            category: ImageCategory::AliasedArtwork,
            ..Options::default()
        }
    )
    .is_ok());
}

#[test]
fn the_basic_preset_can_be_asked_for_under_the_improved_defaults() {
    let options = Options {
        basic_preset: true,
        ..Options::default()
    };
    assert_eq!(effective_advanced(&options), None);
    assert_eq!(effective_advanced(&Options::default()), Some(blended_defaults()));
}

#[test]
fn pixel_art_scaled_up_by_a_whole_number_is_traced_as_its_squares() {
    // A 6 by 5 drawing scaled up 4 times: under the improved defaults the
    // outlines are the blocks' edges (lines only, corners exact) in exactly
    // the drawing's colours; the original's presets smooth it as before.
    let drawing = [
        "..XX..", ".XRRX.", "XRRRRX", ".XRRX.", "..XX..",
    ];
    let colour = |c: char| match c {
        'X' => [26, 26, 26, 255],
        'R' => [214, 40, 40, 255],
        _ => [255, 255, 255, 255],
    };
    let k = 4;
    let (w, h) = (6 * k, 5 * k);
    let pixels: Vec<Rgba> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            Rgba(colour(drawing[y / k].as_bytes()[x / k] as char))
        })
        .collect();
    let raster = Raster { width: w, height: h, pixels };
    let options = |owned_defaults: bool| Options {
        category: ImageCategory::AliasedArtwork,
        quality: Quality::High,
        owned_defaults,
        ..Options::default()
    };
    let doc = vectorize(&raster, options(true)).unwrap();
    assert!(doc.svg.contains("viewBox=\"0 0 24 20\""), "{}", doc.svg);
    assert!(!doc.svg.contains(" C "), "{}", doc.svg);
    for fill in doc.svg.split("fill=\"").skip(1).map(|s| &s[..7]) {
        assert!(["#1a1a1a", "#d62828", "#ffffff"].contains(&fill), "{fill}");
    }
    // Every node on the blocks' grid.
    for p in doc.nodes() {
        assert!(p.x % k as f64 == 0. && p.y % k as f64 == 0., "{p:?}");
    }
    assert!(vectorize(&raster, options(false)).unwrap().svg.contains(" C "));
}
