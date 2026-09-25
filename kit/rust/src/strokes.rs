//! Thin anti-aliased strokes drawn in their ink, at their width.
//!
//! A stroke about a pixel wide never covers a whole pixel, so every pixel it
//! touches is a blend of its ink and the background; the engine fills the
//! region with the mean of those blends and widens it to carry the same
//! ink: a black 1 px line at 45 degrees came out #535353 and 1.44 px wide, a
//! 2 px one #262626 and 2.1 to 2.25 px wide (the defect sweep of September
//! 23, 2026; the original does the same). Recolouring alone raised the
//! error (thin rings 0.64 -> 0.73), since the width carries half the ink.
//!
//! Both are corrected together here. A region is a stroke when it is thin
//! (twice its area over its outline's length under `MAX_WIDTH`) and long
//! (its outline at least `MIN_ELONGATION` times that width), and lies on
//! one background (the pixels two away from it within `BACKGROUND_SPREAD`
//! levels of their median). Its most-inked pixels (the `INK_PERCENTILE`
//! of their share of the way from the background past the fill) give its
//! ink; when that lies at least `MIN_GAIN` of the way past the fill, the
//! fill becomes the ink and the outline moves in by what keeps the ink
//! the same (the width scaled by the fill's share of the ink's contrast).
//! Nodes where three regions meet stay, and every copy of a moved node
//! moves alike, so outlines stay sealed.
use crate::geometry::{dist, Point};
use crate::raster::{hex_rgb, neighbours4, Raster};
use crate::shapes::islands;
use crate::simplify::{arriving, junction_keys, key, leaving, parse_all_paths, splice};
use std::collections::HashMap;

/// Pixels: a region at most this wide on average may be a stroke.
const MAX_WIDTH: f64 = 3.;
/// A stroke's outline is at least this many times its width.
const MIN_ELONGATION: f64 = 8.;
/// Levels (largest channel): the background's pixels lie this close to
/// their median.
const BACKGROUND_SPREAD: u8 = 12;
/// Levels (largest channel) between the fill and the background, at least.
const MIN_CONTRAST: f64 = 24.;
/// The share of a stroke's pixels less inked than its ink.
const INK_PERCENTILE: f64 = 0.95;
/// The ink must lie this far past the fill (as a share of the fill's
/// contrast) to be worth it.
const MIN_GAIN: f64 = 0.15;
/// A node moves along its bisector at most this many times the inset.
const MAX_MITER: f64 = 3.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StrokeStats {
    /// Thin regions on one background, and those drawn in their ink.
    pub strokes: usize,
    pub inked: usize,
}

/// The direction a thin region's pixels take from `back`, as a unit vector:
/// each pixel's step weighted by its length, so the most inked count most.
/// A thin region's pixels are blends of one ink over the background, so they
/// lie on the line from it to the ink; the engine's fill is the mean its
/// colour model gave the region, and a pale ink on a dark field came out grey
/// or in a neighbour's hue (the mint and lilac small print of
/// shape-text-small-dark drawn grey and white, September 25, 2026), so the
/// ink's hue is read from the pixels. None when they give no direction.
pub(crate) fn ink_way(
    inside: &[usize],
    colour: impl Fn(usize) -> [f64; 3],
    back: [f64; 3],
) -> Option<[f64; 3]> {
    let mut sum = [0.; 3];
    for &p in inside {
        let c = colour(p);
        let step = [0, 1, 2].map(|k| c[k] - back[k]);
        let length = step.iter().map(|v| v * v).sum::<f64>().sqrt();
        for k in 0..3 {
            sum[k] += step[k] * length;
        }
    }
    let norm = sum.iter().map(|v| v * v).sum::<f64>().sqrt();
    (norm > 1e-9).then(|| sum.map(|v| v / norm))
}

/// `svg` (an engine document of `source`) with its thin strokes drawn in
/// their ink at their width.
pub fn ink_strokes(svg: &str, source: &Raster) -> Result<(String, StrokeStats), String> {
    let (w, h) = (source.width, source.height);
    let found = islands(svg)?;
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let junctions = junction_keys(&paths);
    let mut owner = vec![u32::MAX; w * h];
    for (i, island) in found.iter().enumerate() {
        for p in island.covered(w, h, false) {
            owner[p as usize] = i as u32;
        }
    }
    let colour = |p: usize| {
        let c = source.pixels[p].0;
        [c[0] as f64, c[1] as f64, c[2] as f64]
    };
    let mut stats = StrokeStats::default();
    let mut moves: HashMap<[u64; 2], Point> = HashMap::new();
    let mut recolour: HashMap<usize, [u8; 3]> = HashMap::new();
    for (i, island) in found.iter().enumerate() {
        let Some(fill) = hex_rgb(&island.color).map(|c| c.map(f64::from)) else {
            continue;
        };
        let perimeter: f64 = std::iter::once(&island.outline)
            .chain(&island.hole_outlines)
            .map(|l| {
                (0..l.len())
                    .map(|k| dist(l[k], l[(k + 1) % l.len()]))
                    .sum::<f64>()
            })
            .sum();
        let area = island.area();
        if perimeter <= 0. {
            continue;
        }
        let width = 2. * area / perimeter;
        if width >= MAX_WIDTH || perimeter < MIN_ELONGATION * width {
            continue;
        }
        let inside: Vec<usize> = island
            .covered(w, h, false)
            .into_iter()
            .map(|p| p as usize)
            .collect();
        if inside.is_empty() || inside.iter().any(|&p| source.pixels[p].0[3] != 255) {
            continue;
        }
        // The background: pixels exactly two steps from the region.
        let ring = rings(&inside, &owner, i as u32, w, h);
        if ring.len() < 8 {
            continue;
        }
        let median = |c: usize| {
            let mut v: Vec<f64> = ring.iter().map(|&p| colour(p)[c]).collect();
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let back = [median(0), median(1), median(2)];
        let spread_ok = ring
            .iter()
            .filter(|&&p| {
                (0..3).all(|c| (colour(p)[c] - back[c]).abs() <= BACKGROUND_SPREAD as f64)
            })
            .count() as f64
            >= 0.9 * ring.len() as f64;
        // Along the ink's hue as far as the fill reaches on it: the inset
        // below weighs the fill's darkness, which covers the whole width.
        let to_fill = [0, 1, 2].map(|k| fill[k] - back[k]);
        let span = ink_way(&inside, colour, back)
            .map(|way| {
                let reach: f64 = (0..3).map(|k| to_fill[k] * way[k]).sum();
                way.map(|v| v * reach)
            })
            .filter(|s| s.iter().any(|v| v.abs() > 1e-9))
            .unwrap_or(to_fill);
        let contrast = span.iter().map(|v| v.abs()).fold(0., f64::max);
        if !spread_ok || contrast < MIN_CONTRAST {
            continue;
        }
        stats.strokes += 1;
        let span2: f64 = span.iter().map(|v| v * v).sum();
        let mut shares: Vec<f64> = inside
            .iter()
            .map(|&p| {
                let c = colour(p);
                (0..3).map(|k| (c[k] - back[k]) * span[k]).sum::<f64>() / span2
            })
            .collect();
        shares.sort_by(f64::total_cmp);
        let ink_share = shares[((shares.len() - 1) as f64 * INK_PERCENTILE).round() as usize];
        if ink_share < 1. + MIN_GAIN {
            continue;
        }
        let ink = [0, 1, 2].map(|k| (back[k] + ink_share * span[k]).clamp(0., 255.).round() as u8);
        // Inward by what keeps the ink: width / share of it.
        let inset = width * (1. - 1. / ink_share) / 2.;
        for &sub in std::iter::once(&island.outer).chain(&island.holes) {
            let edges = &paths[island.path][sub].edges;
            let n = edges.len();
            for k in 0..n {
                let (before, after) = (&edges[(k + n - 1) % n], &edges[k]);
                let node = after.start();
                if junctions.contains(&key(node)) {
                    continue;
                }
                let (Some(a), Some(b)) = (arriving(before), leaving(after)) else {
                    continue;
                };
                let (tx, ty) = (a.x + b.x, a.y + b.y);
                let len = tx.hypot(ty);
                if len < 1e-9 {
                    continue;
                }
                let normal = Point {
                    x: -ty / len,
                    y: tx / len,
                };
                let probe = |s: f64| Point {
                    x: node.x + normal.x * s,
                    y: node.y + normal.y * s,
                };
                let sign = if island.contains(probe(0.25)) {
                    1.
                } else {
                    -1.
                };
                // Along the bisector by the miter distance, so each side
                // moves in by the inset (capped at a sharp corner).
                let miter = (2. / len).min(MAX_MITER);
                moves.insert(key(node), probe(sign * inset * miter));
            }
        }
        recolour.insert(island.path, ink);
        stats.inked += 1;
    }
    if stats.inked == 0 {
        return Ok((svg.to_owned(), stats));
    }
    for subpath in paths.iter_mut().flatten() {
        for edge in &mut subpath.edges {
            let line = edge.line;
            let points = &mut edge.cubic.points;
            for (end, handle) in [(0, 1), (3, 2)] {
                if let Some(&to) = moves.get(&key(points[end])) {
                    let (dx, dy) = (to.x - points[end].x, to.y - points[end].y);
                    points[end] = to;
                    points[handle] = Point {
                        x: points[handle].x + dx,
                        y: points[handle].y + dy,
                    };
                }
            }
            if line {
                *edge = crate::simplify::Edge::line(points[0], points[3], edge.implicit);
            }
        }
    }
    let moved = splice(svg, &ranges, &paths);
    Ok((refill(&moved, &recolour), stats))
}

/// The pixels exactly two steps (4-connected) from the region's pixels,
/// owned by other regions.
fn rings(inside: &[usize], owner: &[u32], own: u32, w: usize, h: usize) -> Vec<usize> {
    let mut seen: std::collections::HashSet<usize> = inside.iter().copied().collect();
    let mut first = Vec::new();
    for &p in inside {
        for q in neighbours4(p, w, h) {
            if seen.insert(q) {
                first.push(q);
            }
        }
    }
    let mut second = Vec::new();
    for &p in &first {
        for q in neighbours4(p, w, h) {
            if seen.insert(q) && owner[q] != own {
                second.push(q);
            }
        }
    }
    second
}

/// The document with the fill of each listed path (by its index among the
/// `d` attributes) replaced.
pub(crate) fn refill(svg: &str, fills: &HashMap<usize, [u8; 3]>) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    let mut index = 0;
    while let Some(at) = rest.find("<path") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find('>').map_or(rest.len(), |e| at + e);
        let element = &rest[at..end];
        let is_path = element.contains(" d=\"");
        match (is_path, fills.get(&index)) {
            (true, Some(c)) => {
                let colour = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
                let replaced = match element.find(" fill=\"#") {
                    Some(f) => format!("{} fill=\"{colour}{}", &element[..f], &element[f + 14..]),
                    None => element.to_owned(),
                };
                out.push_str(&replaced);
            }
            _ => out.push_str(element),
        }
        if is_path {
            index += 1;
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
