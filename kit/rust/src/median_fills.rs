//! Each region's fill read back from the pixels it covers once traced.
//!
//! The engine fills a region with the colour its segmentation settled on,
//! before the smoother moved the outline; the traced region then covers
//! somewhat different pixels. A flat fill that best matches its pixels by
//! the mean absolute difference (the quality rounds' colour error) is their
//! per-channel median, so each path is filled with the median of the
//! pixels whose centres its regions cover.
use crate::raster::{hex_rgb, Raster};
use crate::shapes::islands;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MedianFillStats {
    pub recoloured: usize,
}

/// `svg` (an engine document of `source`) with every path filled with the
/// per-channel median of the opaque pixels its regions cover; a path with a
/// region thinner than `min_width` pixels (twice its area over its
/// outline's length) keeps its fill.
pub fn median_fills(
    svg: &str,
    source: &Raster,
    min_width: f64,
) -> Result<(String, MedianFillStats), String> {
    let (w, h) = (source.width, source.height);
    let found = islands(svg)?;
    // Per path, a histogram of each channel over the pixels it covers.
    let mut histograms: HashMap<usize, Box<[[u32; 256]; 3]>> = HashMap::new();
    let mut fills: HashMap<usize, [u8; 3]> = HashMap::new();
    let mut thin = std::collections::HashSet::new();
    for island in &found {
        let Some(fill) = hex_rgb(&island.color) else {
            continue;
        };
        let length: f64 = std::iter::once(&island.outline)
            .chain(&island.hole_outlines)
            .map(|l| {
                (0..l.len())
                    .map(|k| crate::geometry::dist(l[k], l[(k + 1) % l.len()]))
                    .sum::<f64>()
            })
            .sum();
        if 2. * island.area() < min_width * length {
            thin.insert(island.path);
        }
        fills.insert(island.path, fill);
        let histogram = histograms
            .entry(island.path)
            .or_insert_with(|| Box::new([[0; 256]; 3]));
        for p in island.covered(w, h, false) {
            let c = source.pixels[p as usize].0;
            if c[3] == 255 {
                for (channel, &value) in histogram.iter_mut().zip(&c[..3]) {
                    channel[value as usize] += 1;
                }
            }
        }
    }
    let mut recolour: HashMap<usize, [u8; 3]> = HashMap::new();
    for (path, histogram) in &histograms {
        let n: u32 = histogram[0].iter().sum();
        if n == 0 || thin.contains(path) {
            continue;
        }
        let median = histogram.map(|channel| {
            let mut seen = 0;
            channel
                .iter()
                .position(|&count| {
                    seen += count;
                    2 * seen >= n
                })
                .unwrap_or(0) as u8
        });
        if fills.get(path) != Some(&median) {
            recolour.insert(*path, median);
        }
    }
    let stats = MedianFillStats {
        recoloured: recolour.len(),
    };
    if recolour.is_empty() {
        return Ok((svg.to_owned(), stats));
    }
    Ok((crate::strokes::refill(svg, &recolour), stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rgba;

    #[test]
    fn a_region_takes_the_median_of_the_pixels_it_covers() {
        // A 10 by 10 square over pixels mostly (60, 120, 200), a few noisier.
        let pixels = (0..20 * 20)
            .map(|i| {
                let (x, y) = (i % 20, i / 20);
                match (x, y) {
                    (5..15, 5..15) if x == 5 => Rgba([90, 100, 250, 255]),
                    (5..15, 5..15) => Rgba([60, 120, 200, 255]),
                    _ => Rgba([255, 255, 255, 255]),
                }
            })
            .collect();
        let source = Raster {
            width: 20,
            height: 20,
            pixels,
        };
        let svg = "<svg width=\"20pt\" height=\"20pt\" viewBox=\"0 0 20 20\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 20.00 0.00 L 20.00 20.00 L 0.00 20.00 L 0.00 0.00 M 5.00 5.00 L 5.00 15.00 L 15.00 15.00 L 15.00 5.00 L 5.00 5.00 Z\" />\n</g>\n<g id=\"#4a7ad0ff\">\n<path fill=\"#4a7ad0\" opacity=\"1.00\" d=\" M 5.00 5.00 L 15.00 5.00 L 15.00 15.00 L 5.00 15.00 L 5.00 5.00 Z\" />\n</g>\n</svg>\n";
        let (out, stats) = median_fills(svg, &source, 0.).unwrap();
        assert_eq!(stats.recoloured, 1, "{out}");
        assert!(out.contains("fill=\"#3c78c8\""), "{out}");
        assert!(out.contains("fill=\"#ffffff\""), "{out}");
    }
}
