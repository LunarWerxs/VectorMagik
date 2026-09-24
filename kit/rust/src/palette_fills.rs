//! Fills of pixel-edged artwork with an exact palette, read from the pixels
//! under each region.
//!
//! The engine fills each region with the mean of its pixels, and the palette
//! snap (`prepare::snap_fills`) takes the palette colour nearest that mean.
//! A region traced as a band about a pixel wide (a 1 px diagonal, whose
//! pixels touch at corners only) covers about as many of its neighbour's
//! pixels as of its own, so its mean lies halfway to the neighbour and the
//! nearest palette colour can be a third one: the black, green and blue 1 px
//! diagonals of the shape set (`shape-diagonals`) came out purple, the
//! palette colour nearest their blends with white. Every pixel of such a
//! picture is a palette colour, so a region's colour is read from the pixels
//! whose centres it covers: the most common colour that none of the regions
//! beside it has (a region in its neighbour's colour draws nothing there),
//! when at least `MIN_SHARE` of them have it and no fewer than have the
//! nearest colour; otherwise the nearest colour stays. A large region holding
//! a merged detail of another colour keeps its own, the detail's share being
//! small.
use crate::raster::{hex_rgb, Raster};
use crate::shapes::islands;
use crate::simplify::{parse_all_paths, EdgeKey};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A region takes the colour of at least this share of the pixels it covers.
const MIN_SHARE: f64 = 1. / 3.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaletteFillStats {
    pub recoloured: usize,
}

/// `svg` (an engine document of `source`, pixel-edged with an exact
/// palette, its fills already snapped to the nearest palette colour) with
/// each region's fill read from the pixels under it.
pub fn fills_from_pixels(svg: &str, source: &Raster) -> Result<(String, PaletteFillStats), String> {
    let (w, h) = (source.width, source.height);
    let found = islands(svg)?;
    let (_, paths) = parse_all_paths(svg)?;
    let mut fill: HashMap<usize, [u8; 3]> = HashMap::new();
    let mut counts: HashMap<usize, BTreeMap<[u8; 3], usize>> = HashMap::new();
    for island in &found {
        let Some(colour) = hex_rgb(&island.color) else {
            continue;
        };
        fill.insert(island.path, colour);
        let tally = counts.entry(island.path).or_default();
        for p in island.covered(w, h, false) {
            let c = source.pixels[p as usize].0;
            if c[3] == 255 {
                *tally.entry([c[0], c[1], c[2]]).or_default() += 1;
            }
        }
    }
    // The paths along every edge, to find each region's neighbours.
    let mut along: HashMap<EdgeKey, Vec<usize>> = HashMap::new();
    for (pi, path) in paths.iter().enumerate() {
        for edge in path.iter().flat_map(|sub| &sub.edges) {
            let list = along.entry(edge.key().0).or_default();
            if list.last() != Some(&pi) {
                list.push(pi);
            }
        }
    }
    let mut stats = PaletteFillStats::default();
    let mut recolour: HashMap<usize, [u8; 3]> = HashMap::new();
    for (&pi, tally) in &counts {
        let own = fill[&pi];
        let total: usize = tally.values().sum();
        if total == 0 {
            continue;
        }
        let beside: HashSet<[u8; 3]> = paths[pi]
            .iter()
            .flat_map(|sub| &sub.edges)
            .flat_map(|edge| &along[&edge.key().0])
            .filter(|&&other| other != pi)
            .filter_map(|other| fill.get(other).copied())
            .collect();
        // The most common colour no neighbour has, the darker first on a tie
        // (any fixed order would do).
        let Some((&colour, &count)) = tally
            .iter()
            .filter(|(c, _)| !beside.contains(*c))
            .max_by_key(|&(c, n)| (*n, std::cmp::Reverse(*c)))
        else {
            continue;
        };
        let nearest = tally.get(&own).copied().unwrap_or(0);
        if colour != own && count as f64 >= MIN_SHARE * total as f64 && count >= nearest {
            recolour.insert(pi, colour);
        }
    }
    stats.recoloured = recolour.len();
    if recolour.is_empty() {
        return Ok((svg.to_owned(), stats));
    }
    Ok((crate::strokes::refill(svg, &recolour), stats))
}

#[cfg(test)]
mod tests;
