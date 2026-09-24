//! Owned raster preparation ahead of the engine: flattening a transparent
//! image onto a background colour, and limiting the number of colours. Both
//! are plain pixel operations on the source; the engine then traces what it is
//! given. Neither runs unless asked for.
use crate::raster::{Raster, Rgba};
use std::collections::HashMap;

/// The image composited onto an opaque `background`, so a transparent source
/// converts as if it had been drawn on that colour. Fully opaque images come
/// back unchanged.
pub fn flatten(raster: &Raster, background: [u8; 3]) -> Raster {
    let pixels = raster
        .pixels
        .iter()
        .map(|p| {
            let [r, g, b, a] = p.0;
            if a == 255 {
                return *p;
            }
            let a = u32::from(a);
            let mix =
                |c: u8, bg: u8| ((u32::from(c) * a + u32::from(bg) * (255 - a) + 127) / 255) as u8;
            Rgba([
                mix(r, background[0]),
                mix(g, background[1]),
                mix(b, background[2]),
                255,
            ])
        })
        .collect();
    Raster {
        width: raster.width,
        height: raster.height,
        pixels,
    }
}

/// How many opaque pixels are clustered; more adds nothing visible.
const QUANTIZE_SAMPLE: usize = 65_536;
const QUANTIZE_ROUNDS: usize = 12;

/// The image reduced to at most `count` colours: every visible pixel takes the
/// nearest of `count` representative colours found by k-means over the
/// image's own pixels (deterministic seeding, so the same image always gives
/// the same palette). An image with no more than `count` colours keeps them
/// exactly. Transparency is kept as it is; fully transparent pixels
/// are not counted as a colour. Blended edge pixels snap to a palette colour,
/// so the result has hard edges: the engine's unblended preset suits it.
pub fn quantize(raster: &Raster, count: usize) -> Raster {
    let palette = palette(raster, count);
    if palette.is_empty() {
        return raster.clone();
    }
    let pixels = raster
        .pixels
        .iter()
        .map(|p| {
            if p.0[3] == 0 {
                return *p;
            }
            let rgb = nearest(&palette, [p.0[0] as f64, p.0[1] as f64, p.0[2] as f64]);
            Rgba([rgb[0], rgb[1], rgb[2], p.0[3]])
        })
        .collect();
    Raster {
        width: raster.width,
        height: raster.height,
        pixels,
    }
}

/// The representative colours `quantize` snaps to, most used first.
pub fn palette(raster: &Raster, count: usize) -> Vec<[u8; 3]> {
    let count = count.max(1);
    // Distinct colours cap the palette: asking for more than the image has
    // would only split real colours in two.
    if let Some(exact) = exact_palette(raster, count) {
        return exact;
    }
    let visible = raster.pixels.iter().filter(|p| p.0[3] > 0).count();
    let stride = (visible / QUANTIZE_SAMPLE).max(1);
    let samples: Vec<[f64; 3]> = raster
        .pixels
        .iter()
        .filter(|p| p.0[3] > 0)
        .step_by(stride)
        .map(|p| [p.0[0] as f64, p.0[1] as f64, p.0[2] as f64])
        .collect();
    // k-means++ seeding with a fixed generator, then a few Lloyd rounds.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut centres: Vec<[f64; 3]> = vec![samples[(random() % samples.len() as u64) as usize]];
    let mut nearest_d2: Vec<f64> = samples.iter().map(|s| d2(*s, centres[0])).collect();
    while centres.len() < count {
        let total: f64 = nearest_d2.iter().sum();
        if total <= 0. {
            break;
        }
        let mut pick = (random() as f64 / u64::MAX as f64) * total;
        let mut chosen = samples.len() - 1;
        for (i, d) in nearest_d2.iter().enumerate() {
            if pick <= *d {
                chosen = i;
                break;
            }
            pick -= d;
        }
        let centre = samples[chosen];
        centres.push(centre);
        for (s, d) in samples.iter().zip(nearest_d2.iter_mut()) {
            *d = d.min(d2(*s, centre));
        }
    }
    let mut members = vec![0usize; centres.len()];
    for _ in 0..QUANTIZE_ROUNDS {
        let mut sums = vec![[0f64; 3]; centres.len()];
        members.iter_mut().for_each(|m| *m = 0);
        for s in &samples {
            let i = nearest_index(&centres, *s);
            for c in 0..3 {
                sums[i][c] += s[c];
            }
            members[i] += 1;
        }
        let mut moved = 0f64;
        for (i, centre) in centres.iter_mut().enumerate() {
            if members[i] == 0 {
                continue;
            }
            let next = [
                sums[i][0] / members[i] as f64,
                sums[i][1] / members[i] as f64,
                sums[i][2] / members[i] as f64,
            ];
            moved += d2(*centre, next);
            *centre = next;
        }
        if moved < 1e-6 {
            break;
        }
    }
    let mut ordered: Vec<([u8; 3], usize)> = centres
        .iter()
        .zip(&members)
        .filter(|(_, m)| **m > 0)
        .map(|(c, m)| {
            (
                [
                    c[0].round().clamp(0., 255.) as u8,
                    c[1].round().clamp(0., 255.) as u8,
                    c[2].round().clamp(0., 255.) as u8,
                ],
                *m,
            )
        })
        .collect();
    ordered.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let mut palette: Vec<[u8; 3]> = Vec::with_capacity(ordered.len());
    for (c, _) in ordered {
        if !palette.contains(&c) {
            palette.push(c);
        }
    }
    palette
}

/// Every colour of the visible pixels, most used first (ties in the order
/// first met), or `None` as soon as there are more than `count`. Every pixel
/// is looked at, not the k-means sample: a sample of every Nth pixel missed
/// a colour a few pixels wide (a 3 by 3 dot in a 1000 by 1000 logo at 4
/// colours, stride 15, was left out at about 40% of its positions) and
/// recoloured it though the limit had room for it.
fn exact_palette(raster: &Raster, count: usize) -> Option<Vec<[u8; 3]>> {
    let mut slots: HashMap<[u8; 3], usize> = HashMap::new();
    let mut counts: Vec<([u8; 3], usize)> = Vec::new();
    // Neighbouring pixels mostly share a colour: the last slot is tried first.
    let mut last: Option<([u8; 3], usize)> = None;
    for p in raster.pixels.iter().filter(|p| p.0[3] > 0) {
        let c = [p.0[0], p.0[1], p.0[2]];
        let slot = match last {
            Some((seen, slot)) if seen == c => slot,
            _ => {
                let next = counts.len();
                let slot = *slots.entry(c).or_insert(next);
                if slot == next {
                    if next == count {
                        return None;
                    }
                    counts.push((c, 0));
                }
                slot
            }
        };
        counts[slot].1 += 1;
        last = Some((c, slot));
    }
    counts.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    Some(counts.into_iter().map(|(c, _)| c).collect())
}

fn d2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|c| (a[c] - b[c]) * (a[c] - b[c])).sum()
}
fn nearest_index(centres: &[[f64; 3]], p: [f64; 3]) -> usize {
    let mut best = 0;
    let mut best_d = f64::INFINITY;
    for (i, c) in centres.iter().enumerate() {
        let d = d2(*c, p);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}
fn nearest(palette: &[[u8; 3]], p: [f64; 3]) -> [u8; 3] {
    let mut best = palette[0];
    let mut best_d = f64::INFINITY;
    for c in palette {
        let d = d2([c[0] as f64, c[1] as f64, c[2] as f64], p);
        if d < best_d {
            best_d = d;
            best = *c;
        }
    }
    best
}

/// Whether any pixel is less than fully opaque.
pub fn has_transparency(raster: &Raster) -> bool {
    raster.pixels.iter().any(|p| p.0[3] < 255)
}

/// Every fill of an SVG document (and the engine's colour group ids) snapped
/// to the nearest colour of `palette`, or, with `within`, only the fills no
/// further than that many levels per channel from a palette colour. The
/// engine estimates each region's colour on its own, so a limited-colour
/// image comes back with a few shades of each palette colour, and a region
/// recoloured to match its neighbour can come back a level off; this makes
/// the output carry the palette exactly.
pub fn snap_fills(svg: &str, palette: &[[u8; 3]], within: Option<u8>) -> String {
    if palette.is_empty() {
        return svg.to_owned();
    }
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find("=\"#") {
        let key_start = rest[..at]
            .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .map_or(0, |i| i + 1);
        let key = &rest[key_start..at];
        let value_start = at + 3;
        let hex_len = rest[value_start..]
            .bytes()
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        out.push_str(&rest[..value_start]);
        let hex = &rest[value_start..value_start + hex_len];
        if matches!(key, "fill" | "id" | "stroke") && (hex_len == 6 || hex_len == 8) {
            let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
            let own = [channel(0), channel(2), channel(4)];
            let rgb = nearest(palette, [own[0] as f64, own[1] as f64, own[2] as f64]);
            let close = within.is_none_or(|limit| (0..3).all(|c| own[c].abs_diff(rgb[c]) <= limit));
            if close {
                out.push_str(&format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
            } else {
                out.push_str(&hex[..6]);
            }
            out.push_str(&hex[6..]);
        } else {
            out.push_str(hex);
        }
        rest = &rest[value_start + hex_len..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(width: usize, height: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Raster {
        Raster {
            width,
            height,
            pixels: (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| Rgba(f(x, y)))
                .collect(),
        }
    }

    #[test]
    fn flatten_composites_partial_alpha_and_leaves_opaque_pixels() {
        let r = raster(2, 1, |x, _| {
            if x == 0 {
                [255, 0, 0, 128]
            } else {
                [10, 20, 30, 255]
            }
        });
        assert!(has_transparency(&r));
        let out = flatten(&r, [255, 255, 255]);
        assert!(!has_transparency(&out));
        assert_eq!(out.pixels[0].0, [255, 127, 127, 255]);
        assert_eq!(out.pixels[1].0, [10, 20, 30, 255]);
        let black = flatten(&r, [0, 0, 0]);
        assert_eq!(black.pixels[0].0, [128, 0, 0, 255]);
    }

    #[test]
    fn fills_snap_to_the_palette_and_nothing_else_changes() {
        let svg = "<svg viewBox=\"0 0 4 4\"><g id=\"#3a4b0cff\"><path fill=\"#3b4a0c\" d=\" M 1 1 L 2 2\" /></g><path fill=\"#dedff4\" stroke=\"#dedff4\" d=\" M 3 3\" /><path fill=\"none\" d=\"\" /></svg>";
        let out = snap_fills(svg, &[[0x3b, 0x4b, 0x0d], [0xde, 0xe0, 0xf5]], None);
        assert_eq!(
            out,
            "<svg viewBox=\"0 0 4 4\"><g id=\"#3b4b0dff\"><path fill=\"#3b4b0d\" d=\" M 1 1 L 2 2\" /></g><path fill=\"#dee0f5\" stroke=\"#dee0f5\" d=\" M 3 3\" /><path fill=\"none\" d=\"\" /></svg>"
        );
        assert_eq!(snap_fills(svg, &[], None), svg);
        // With a limit only near fills move: the greens are within 1, the
        // light fills are 32 levels from black and stay.
        let near = snap_fills(svg, &[[0x3b, 0x4b, 0x0d], [0, 0, 0]], Some(2));
        assert!(near.contains("fill=\"#3b4b0d\"") && near.contains("fill=\"#dedff4\""));
    }

    #[test]
    fn quantize_keeps_few_colours_exactly_and_limits_many() {
        // Two flat colours: asking for four keeps both, exactly.
        let two = raster(8, 8, |x, _| {
            if x < 4 {
                [200, 30, 30, 255]
            } else {
                [30, 30, 200, 255]
            }
        });
        let out = quantize(&two, 4);
        assert_eq!(out.pixels, two.pixels);
        assert_eq!(palette(&two, 4).len(), 2);
        // A gradient with a transparent corner: two colours, alpha untouched,
        // transparent pixels ignored, and the same answer every time.
        let gradient = raster(32, 32, |x, y| {
            if x < 4 && y < 4 {
                [0, 0, 0, 0]
            } else {
                [
                    (x * 8) as u8,
                    (y * 8) as u8,
                    128,
                    if x == 5 { 100 } else { 255 },
                ]
            }
        });
        let out = quantize(&gradient, 2);
        let mut distinct: Vec<[u8; 3]> = Vec::new();
        for (p, q) in out.pixels.iter().zip(&gradient.pixels) {
            assert_eq!(p.0[3], q.0[3], "alpha is kept");
            if p.0[3] == 0 {
                assert_eq!(p.0, q.0);
                continue;
            }
            let c = [p.0[0], p.0[1], p.0[2]];
            if !distinct.contains(&c) {
                distinct.push(c);
            }
        }
        assert_eq!(distinct.len(), 2, "{distinct:?}");
        assert_eq!(out.pixels, quantize(&gradient, 2).pixels);
        // One colour: everything visible becomes one colour.
        let one = quantize(&gradient, 1);
        let visible: std::collections::HashSet<[u8; 3]> = one
            .pixels
            .iter()
            .filter(|p| p.0[3] > 0)
            .map(|p| [p.0[0], p.0[1], p.0[2]])
            .collect();
        assert_eq!(visible.len(), 1);
    }

    #[test]
    fn a_colour_the_sample_skips_is_still_in_the_exact_palette() {
        // 160,000 visible pixels: k-means samples every second one, and the
        // single red pixel at an odd position is never among them. The image
        // has four colours, so asking for four keeps all four exactly, most
        // used first.
        let image = raster(400, 400, |x, y| match (x, y) {
            (1, 0) => [255, 0, 0, 255],
            (0..200, 0..200) => [0, 0, 0, 255],
            (0..200, _) => [255, 255, 255, 255],
            _ => [0, 0, 255, 255],
        });
        assert_eq!(
            palette(&image, 4),
            vec![[0, 0, 255], [255, 255, 255], [0, 0, 0], [255, 0, 0]]
        );
        assert_eq!(quantize(&image, 4).pixels, image.pixels);
        // One colour more than allowed goes to k-means, which still gives
        // no more than the limit.
        assert!((1..=3).contains(&palette(&image, 3).len()));
    }
}
