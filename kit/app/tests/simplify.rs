#![cfg(feature = "desktop")]
//! Curve simplification on real engine output: fewer nodes, the same picture,
//! and no cracks between adjacent fills.
use std::path::PathBuf;
use vector_magic_rebuild::engine::{vectorize, Document, Options as EngineOptions};
use vector_magic_rebuild::{load_raster, preview};
use vector_rebuild::{ImageCategory, Quality};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Transparent pixels whose horizontal or vertical neighbours are both opaque:
/// a hairline crack between two fills. Silhouette shifts never look like this.
fn seam_pixels(image: &eframe::egui::ColorImage) -> usize {
    let [w, h] = image.size;
    let alpha = |x: usize, y: usize| image.pixels[y * w + x].a();
    let mut seams = 0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            if alpha(x, y) < 128
                && ((alpha(x - 1, y) > 200 && alpha(x + 1, y) > 200)
                    || (alpha(x, y - 1) > 200 && alpha(x, y + 1) > 200))
            {
                seams += 1;
            }
        }
    }
    seams
}

#[test]
fn simplified_artwork_keeps_the_picture_and_seals_shared_edges() {
    let source = root().join("kit/fixtures/samples/logo-with-transparency.png");
    let raster = load_raster(&source).unwrap();
    let doc = vectorize(&raster, EngineOptions::default()).unwrap();
    let simplified = doc.simplified(0.5).unwrap();
    assert!(doc.simplify_tolerance.is_none());
    assert_eq!(simplified.simplify_tolerance, Some(0.5));
    assert!(
        simplified.segment_count() < doc.segment_count(),
        "{} -> {}",
        doc.segment_count(),
        simplified.segment_count()
    );
    assert_eq!(simplified.color_count(), doc.color_count());
    assert!(simplified.svg().contains("viewBox=\"0 0 250 250\""));

    let before = preview(doc.svg()).unwrap();
    let after = preview(simplified.svg()).unwrap();
    assert_eq!(before.size, after.size);
    let mut changed = 0usize;
    for (a, b) in before.pixels.iter().zip(&after.pixels) {
        let a = a.to_array();
        let b = b.to_array();
        if (0..4).any(|c| a[c].abs_diff(b[c]) > 64) {
            changed += 1;
        }
    }
    let fraction = changed as f64 / before.pixels.len() as f64;
    assert!(fraction < 0.02, "{:.2}% of pixels changed", fraction * 100.);
    assert!(
        seam_pixels(&after) <= seam_pixels(&before),
        "simplification opened cracks between fills"
    );
    assert!(doc.nodes().len() > simplified.nodes().len());
}

#[test]
fn simplified_photo_keeps_seam_strokes_and_finishes_quickly() {
    let source = root().join("kit/fixtures/photos/chelsea.png");
    let raster = load_raster(&source).unwrap();
    let doc = vectorize(
        &raster,
        EngineOptions {
            category: ImageCategory::Photograph,
            quality: Quality::High,
            overlap_opaque_photos: true,
            optional_optimizer: false,
            advanced: None,
            owned_defaults: true,
            basic_preset: false,
        },
    )
    .unwrap();
    assert!(doc.photo_overlap);
    // "Quickly" is a RATIO, never a budget, so machine load cannot decide it:
    // the photo's simplification cost per segment against a small artwork's,
    // timed beside it, best of three rounds on both sides. Simplification
    // works run by run, so a healthy cost is linear in segments and the two
    // land close together; a cost that grows with the document (a quadratic
    // in runs or in segments) is many times the artwork's per segment on a
    // photo this size, far past the bound. Measured 2026-09-22 (release):
    // 0.67x, 7,280 photo segments against 213; a quadratic would be about 34x.
    let artwork = vectorize(
        &load_raster(&root().join("kit/fixtures/samples/logo-with-transparency.png")).unwrap(),
        EngineOptions::default(),
    )
    .unwrap();
    let seconds_per_segment = |doc: &Document| {
        let best = (0..3)
            .map(|_| {
                let started = std::time::Instant::now();
                std::hint::black_box(doc.simplified(0.5).unwrap());
                started.elapsed().as_secs_f64()
            })
            .fold(f64::INFINITY, f64::min);
        best / doc.segment_count() as f64
    };
    let ratio = seconds_per_segment(&doc) / seconds_per_segment(&artwork);
    println!(
        "simplifying {} photo segments costs {ratio:.2}x per segment what {} artwork segments do",
        doc.segment_count(),
        artwork.segment_count()
    );
    assert!(
        ratio < 8.,
        "simplifying {} photo segments costs {ratio:.1}x per segment what {} artwork segments do",
        doc.segment_count(),
        artwork.segment_count()
    );
    let simplified = doc.simplified(0.5).unwrap();
    assert!(simplified.segment_count() < doc.segment_count());
    // Only path data changes: the seam-hiding strokes and every fill survive.
    assert_eq!(
        simplified.svg().matches("stroke=\"").count(),
        doc.svg().matches("stroke=\"").count()
    );
    assert_eq!(
        simplified.svg().matches("<path ").count(),
        doc.svg().matches("<path ").count()
    );
    assert!(
        seam_pixels(&preview(simplified.svg()).unwrap())
            <= seam_pixels(&preview(doc.svg()).unwrap())
    );
}
