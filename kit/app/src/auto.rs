//! Local preset estimation. This is new UI behavior, not the unrecovered native classifier.
use std::collections::BTreeMap;
use vector_rebuild::raster::Raster;
use vector_rebuild::{ImageCategory, Quality};

#[derive(Clone, Copy, Debug)]
pub struct Detection {
    pub category: ImageCategory,
    pub quality: Quality,
    pub flat_fraction: f64,
    pub palette_coverage: f64,
    pub soft_edge_fraction: f64,
    /// Share of the edge pixels that are a blend of their two opposite
    /// neighbours: 0 in aliased artwork, a third in anti-aliased artwork at
    /// any size.
    pub blend_edge_fraction: f64,
}
impl Detection {
    pub fn description(self) -> String {
        format!(
            "{} · {:?} source quality",
            category_name(self.category),
            self.quality
        )
    }
}
pub fn category_name(category: ImageCategory) -> &'static str {
    match category {
        ImageCategory::AntiAliasedArtwork => "Artwork",
        ImageCategory::AliasedArtwork => "Pixel art",
        ImageCategory::Photograph => "Photo",
    }
}

/// Bounded sampling of local variation, palette concentration and alpha coverage.
/// Source quality is a conservative resolution/artifact estimate, not a semantic judgment.
pub fn detect(raster: &Raster) -> Detection {
    let stride = (raster.pixels.len() / 200_000).max(1);
    // Flatness is measured against the pixels `reach` away, one per 256 px
    // of the longer side: a photograph scaled up k times is flat pixel to
    // pixel (the astronaut at 4x: 0.80 of its pixels within 8 levels of
    // their neighbours, against 0.44 at its own size) and was called
    // blended artwork, which traced it nine times slower (the defect sweep
    // of September 23, 2026); at 4 px it is 0.44 again. Artwork stays flat
    // at any reach but along its edges, and its palette keeps it artwork
    // (0.87 or more of its pixels in 16 colours, photographs 0.30 or less,
    // scaled or not). Pictures under 512 px are measured as before.
    let reach = (raster.width.max(raster.height) / 256).max(1);
    let mut histogram = BTreeMap::<u16, usize>::new();
    let (mut visible, mut flat, mut soft, mut partial, mut tested) = (0usize, 0, 0, 0, 0);
    let (mut edge, mut blend) = (0usize, 0);
    for index in (0..raster.pixels.len()).step_by(stride) {
        let p = raster.pixels[index].0;
        if p[3] < 16 {
            continue;
        }
        visible += 1;
        partial += usize::from(p[3] < 255);
        let key = ((p[0] as u16 >> 3) << 10) | ((p[1] as u16 >> 3) << 5) | (p[2] as u16 >> 3);
        *histogram.entry(key).or_default() += 1;
        let x = index % raster.width;
        let y = index / raster.width;
        if x < reach || y < reach || x + reach >= raster.width || y + reach >= raster.height {
            continue;
        }
        let delta = [
            index - 1,
            index + 1,
            index - raster.width,
            index + raster.width,
        ]
        .into_iter()
        .flat_map(|n| (0..4).map(move |c| p[c].abs_diff(raster.pixels[n].0[c])))
        .max()
        .unwrap_or(0);
        let far = [
            index - reach,
            index + reach,
            index - reach * raster.width,
            index + reach * raster.width,
        ]
        .into_iter()
        .flat_map(|n| (0..4).map(move |c| p[c].abs_diff(raster.pixels[n].0[c])))
        .max()
        .unwrap_or(0);
        flat += usize::from(far < 9);
        soft += usize::from((9..=48).contains(&delta));
        tested += 1;
        // An anti-aliased edge pixel lies between the colours on either side
        // of it, at least 8 levels from each; an aliased one repeats one side.
        let between = |a: usize, b: usize| {
            let (a, b) = (raster.pixels[a].0, raster.pixels[b].0);
            (0..4).all(|c| a[c].min(b[c]) <= p[c] && p[c] <= a[c].max(b[c]))
                && (0..4).any(|c| p[c].abs_diff(a[c]) >= 8)
                && (0..4).any(|c| p[c].abs_diff(b[c]) >= 8)
        };
        if delta >= 9 {
            edge += 1;
            blend += usize::from(
                between(index - 1, index + 1)
                    || between(index - raster.width, index + raster.width),
            );
        }
    }
    let mut counts: Vec<_> = histogram.values().copied().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let palette_coverage = counts.iter().take(16).sum::<usize>() as f64 / visible.max(1) as f64;
    let flat_fraction = flat as f64 / tested.max(1) as f64;
    let soft_edge_fraction = soft as f64 / tested.max(1) as f64;
    let blend_edge_fraction = blend as f64 / edge.max(1) as f64;
    // The soft-edge and palette measures are shares of the whole picture, so
    // a large picture with a short anti-aliased outline passed as aliased:
    // the two-colour S of kit/fixtures/shapes (180 px, one edge) has 0.25%
    // soft edges and 99.7% of its pixels in 16 colours, and traced with the
    // aliased preset its rim became sliver regions (S radius spread 2.08 px).
    // The blend share is one of the edges alone, measured September 22, 2026:
    // 0 on logo-without-blending and on aliased discs, 0.33 to 0.39 on the
    // three blended logos, the four shapes and anti-aliased discs from 64 to
    // 1200 px.
    let category = if flat_fraction < 0.70 && palette_coverage < 0.86 {
        ImageCategory::Photograph
    } else if soft_edge_fraction > 0.003
        || blend_edge_fraction > 0.1
        || partial as f64 / visible.max(1) as f64 > 0.001
        || palette_coverage < 0.995
    {
        ImageCategory::AntiAliasedArtwork
    } else {
        ImageCategory::AliasedArtwork
    };
    // Medium from 48 to 160 px on the shorter side, high otherwise. A
    // picture under 48 px went to low quality until round two of the Opus
    // 5.5 review, which measured three circles on a 46 x 42 px picture
    // (kit/tools/shape_set.py shape-circles-low, testing/quality-round/
    // tiny-quality.md): low drew them 0.99 px out of round with 0.86 of
    // their area, medium 0.60 and 0.96, high 0.03 and 0.99 (colour error
    // 0.45, 0.32, 0.27); a picture that small needs all its detail. The
    // outline band at half size (80 px) measured worse at high than at
    // medium (colour error 2.52 against 1.91), so medium stays there.
    let smaller = raster.width.min(raster.height);
    let quality = if (48..160).contains(&smaller) {
        Quality::Medium
    } else {
        Quality::High
    };
    Detection {
        category,
        quality,
        flat_fraction,
        palette_coverage,
        soft_edge_fraction,
        blend_edge_fraction,
    }
}
