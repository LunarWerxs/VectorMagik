//! Pixel-edged shapes made of long straight runs, drawn on their pixels.
//!
//! The engine smooths every pixel-edged outline, and on a small sharp
//! shape the smoothing has nothing to hold on to: plus signs with 5 and 6
//! px arms came out as four-pointed stars and pinwheels, their edges 59 and
//! 31 degrees off (the defect sweep of September 23, 2026, pixel-art exp2).
//! On pixel-edged artwork with an exact palette the pixels settle such a
//! shape: a region whose own pixels (the 4-connected pixels of its colour
//! under it) have an outline with no staircase, every run shorter than
//! `MIN_RUN` (a thin stem's end) between two that are not, is that
//! polygon. It replaces the traced outline when
//! the region has no holes and one neighbour all round, whose hole is
//! redrawn with it, so the two stay sealed. A circle's or a diagonal's
//! pixel outline has runs of one and two pixels and is left to the engine.
//! The pixels may reach past the traced outline into the neighbour (a plus
//! with 5 px arms was traced as a star of half its area) but into no other
//! region: a colour whose pixels run on under a third region is not this
//! region's alone.
//!
//! A thin region whose own pixels (its colour, connected eight ways: a
//! diagonal's pixels touch at corners) are one straight chain, every centre
//! within `LINE_SPREAD` of half its width from their axis, is a line. The
//! engine traced pixel-edged lines of 1 and 2 px as lenses, blades and
//! crescents tapering to nothing (the shape set's `shape-lines` and
//! `shape-diagonals`); such a region is drawn as one band along that axis,
//! as long as its pixels reach and as wide as keeps their ink.
//!
//! Regions that touch each other as well as the region all round them
//! (lines of two colours that cross or meet) cannot each have their
//! neighbour's hole redrawn: they share edges. When the region around them
//! is painted first and their outline together is exactly one of its holes,
//! that hole goes and each is drawn on its pixels over it
//! (`refit_clusters`).
use crate::geometry::{dist, to_segment, turning, Point};
use crate::raster::{corner_neighbours, hex_rgb, neighbours4, Raster};
use crate::shapes::{islands, Island};
use crate::simplify::{parse_all_paths, splice, Edge, EdgeKey, Subpath};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Pixels: every straight run of the shape's pixel outline is this long.
const MIN_RUN: f64 = 3.;
/// The pixels of the region's colour under it cover at least this share
/// less than its traced area.
const AREA_SLACK: f64 = 0.25;
/// The pixels of the region's colour cover at most this many times its
/// traced area (a bound on the work; past it the shape is not this region).
const MAX_GROWTH: f64 = 4.;
/// Pixels: a traced outline this close to the pixel polygon is left alone.
const SETTLED: f64 = 0.2;
/// Pixels: a region is tried as a line when twice its area over its
/// outline's length (its width) is under this.
const THIN: f64 = 2.5;
/// Pixels: a line's pixel centres lie within half its width and this of
/// its axis (a pixel-edged line's centres step up to half a pixel off it).
const LINE_SPREAD: f64 = 0.5;
/// Pixels: a line is at most this wide by its ink.
const LINE_WIDTH: f64 = 2.5;
/// A line is at least this many times as long as it is wide.
const LINE_ASPECT: f64 = 4.;
/// A line has at least this many pixels.
const LINE_PIXELS: usize = 4;
/// A line's pixels cover at least this share of its traced area: the
/// engine drew a 1 px diagonal 1.27 px wide over ink 0.71 px wide.
const LINE_SHARE: f64 = 0.4;
/// A line's pixels may lie under this many pixel centres of regions of
/// other colours (where it meets another line, whose traced outline covers
/// the pixel at the joint).
const FOREIGN: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RectilinearStats {
    pub refitted: usize,
}

/// `svg` (an engine document of `source`, pixel-edged with an exact
/// palette) with its rectilinear shapes and straight lines drawn on their
/// pixels.
pub fn refit_rectilinear(svg: &str, source: &Raster) -> Result<(String, RectilinearStats), String> {
    let found = islands(svg)?;
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let pixels = Pixels::new(source, &found);
    // Every subpath by the edges it runs along, to find a shape's neighbour.
    let mut by_edges: HashMap<Vec<EdgeKey>, Vec<(usize, usize)>> = HashMap::new();
    for (pi, path) in paths.iter().enumerate() {
        for (si, sub) in path.iter().enumerate() {
            by_edges.entry(edge_set(sub)).or_default().push((pi, si));
        }
    }
    let mut done = HashSet::new();
    for (i, island) in found.iter().enumerate() {
        if !island.holes.is_empty() || pixels.fills[i].is_none() {
            continue;
        }
        // The one neighbour all round: the subpath elsewhere with exactly
        // this outline's edges, and the island it bounds.
        let own = edge_set(&paths[island.path][island.outer]);
        let Some(&(tp, ts)) = by_edges
            .get(&own)
            .and_then(|list| list.iter().find(|&&t| t != (island.path, island.outer)))
        else {
            continue;
        };
        let Some(beside) = found
            .iter()
            .position(|o| o.path == tp && (o.outer == ts || o.holes.contains(&ts)))
        else {
            continue;
        };
        let Some(polygon) = pixels.polygon(i, &[beside]) else {
            continue;
        };
        if settled(island, &polygon) {
            continue;
        }
        let outer = outward(polygon);
        let mut hole = outer.clone();
        hole.reverse();
        paths[island.path][island.outer].edges = lines(&outer);
        paths[tp][ts].edges = lines(&hole);
        done.insert(i);
    }
    let stats = RectilinearStats {
        refitted: done.len() + refit_clusters(&pixels, &mut paths, &done),
    };
    if stats.refitted == 0 {
        return Ok((svg.to_owned(), stats));
    }
    Ok((splice(svg, &ranges, &paths), stats))
}

/// Regions without holes that touch each other and one region more (lines
/// of two colours that cross or meet, the field all round them) each drawn
/// on their pixels over that region, whose hole around them goes: their
/// shared edges cannot be redrawn to match two new outlines at once, and
/// painted over the region, which is drawn first, they need none. Only
/// where the cluster's outline is exactly one hole of that region and every
/// member's pixels settle its polygon, and not when every member is on its
/// pixels already. Returns how many regions were redrawn.
fn refit_clusters(pixels: &Pixels, paths: &mut [Vec<Subpath>], done: &HashSet<usize>) -> usize {
    let found = pixels.found;
    // The islands along each edge, of their outlines or their holes.
    let mut along: HashMap<EdgeKey, Vec<usize>> = HashMap::new();
    for (i, island) in found.iter().enumerate() {
        for &sub in std::iter::once(&island.outer).chain(&island.holes) {
            for edge in &paths[island.path][sub].edges {
                let list = along.entry(edge.key().0).or_default();
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
        }
    }
    let member =
        |i: usize| !done.contains(&i) && found[i].holes.is_empty() && pixels.fills[i].is_some();
    let mut seen = HashSet::new();
    let mut dropped: Vec<(usize, usize)> = Vec::new();
    let mut refitted = 0;
    for start in 0..found.len() {
        if !member(start) || !seen.insert(start) {
            continue;
        }
        // The members connected to `start` through shared edges, and the
        // regions around them.
        let mut members = vec![start];
        let mut around = BTreeSet::new();
        let mut k = 0;
        while k < members.len() {
            let at = &found[members[k]];
            for edge in &paths[at.path][at.outer].edges {
                for &j in &along[&edge.key().0] {
                    if j == members[k] {
                        continue;
                    }
                    if !member(j) {
                        around.insert(j);
                    } else if seen.insert(j) {
                        members.push(j);
                    }
                }
            }
            k += 1;
        }
        let field = match around.iter().collect::<Vec<_>>()[..] {
            [&field] if members.len() > 1 => field,
            _ => continue,
        };
        if members.iter().any(|&m| found[m].path <= found[field].path) {
            continue;
        }
        // The cluster's outline: the members' edges no two of them share.
        let mut count: HashMap<EdgeKey, usize> = HashMap::new();
        for &m in &members {
            for edge in &paths[found[m].path][found[m].outer].edges {
                *count.entry(edge.key().0).or_default() += 1;
            }
        }
        let mut rim: Vec<EdgeKey> = count
            .into_iter()
            .filter(|&(_, n)| n == 1)
            .map(|(key, _)| key)
            .collect();
        rim.sort();
        let Some(&hole) = found[field]
            .holes
            .iter()
            .find(|&&s| edge_set(&paths[found[field].path][s]) == rim)
        else {
            continue;
        };
        // Each member's pixels may lie under the field and members of other
        // colours.
        let polygons: Option<Vec<Vec<Point>>> = members
            .iter()
            .map(|&m| {
                let open: Vec<usize> = std::iter::once(field)
                    .chain(
                        members
                            .iter()
                            .copied()
                            .filter(|&o| pixels.fills[o] != pixels.fills[m]),
                    )
                    .collect();
                pixels.polygon(m, &open)
            })
            .collect();
        let Some(polygons) = polygons else {
            continue;
        };
        if members
            .iter()
            .zip(&polygons)
            .all(|(&m, polygon)| settled(&found[m], polygon))
        {
            continue;
        }
        for (&m, polygon) in members.iter().zip(polygons) {
            paths[found[m].path][found[m].outer].edges = lines(&outward(polygon));
        }
        dropped.push((found[field].path, hole));
        refitted += members.len();
    }
    // The holes go last to first, so the indices still hold.
    dropped.sort_unstable_by(|a, b| b.cmp(a));
    for (p, s) in dropped {
        let closed = paths[p].remove(s).closed;
        // The engine closes a path's last outline only.
        if closed && s == paths[p].len() && s > 0 {
            paths[p][s - 1].closed = true;
        }
    }
    refitted
}

/// The picture's pixels and which island each pixel's centre lies in.
struct Pixels<'a> {
    source: &'a Raster,
    found: &'a [Island],
    fills: Vec<Option<[u8; 3]>>,
    owner: Vec<u32>,
}

/// How a walk over a region's own pixels goes: connected at corners too
/// (`eight`), covering at least `share` of the traced area, under at most
/// `foreign` pixel centres of regions of other colours.
struct Walk {
    eight: bool,
    share: f64,
    foreign: usize,
}

impl<'a> Pixels<'a> {
    fn new(source: &'a Raster, found: &'a [Island]) -> Self {
        let (w, h) = (source.width, source.height);
        // Which island each pixel's centre lies in (the last one painted).
        let mut owner = vec![u32::MAX; w * h];
        for (i, island) in found.iter().enumerate() {
            for p in island.covered(w, h, false) {
                owner[p as usize] = i as u32;
            }
        }
        Pixels {
            source,
            found,
            fills: found.iter().map(|o| hex_rgb(&o.color)).collect(),
            owner,
        }
    }

    fn colour(&self, p: usize) -> [u8; 3] {
        let c = self.source.pixels[p].0;
        [c[0], c[1], c[2]]
    }

    /// The pixels of island `i`'s colour whose centres it covers (`dilate`:
    /// or that touch one it covers).
    fn seeds(&self, i: usize, dilate: bool) -> Vec<usize> {
        let (w, h) = (self.source.width, self.source.height);
        self.found[i]
            .covered(w, h, dilate)
            .into_iter()
            .map(|p| p as usize)
            .filter(|&p| Some(self.colour(p)) == self.fills[i])
            .collect()
    }

    /// Island `i`'s own pixels: its colour, connected to `seeds`, under it
    /// or the islands `open`. `None` when they pass `MAX_GROWTH` times its
    /// area, cover less than `walk.share` of it, or run on under another
    /// region (under more than `walk.foreign` pixel centres of other
    /// colours, or any of its own colour).
    fn own(&self, i: usize, open: &[usize], seeds: Vec<usize>, walk: Walk) -> Option<Vec<usize>> {
        let (w, h) = (self.source.width, self.source.height);
        let island = &self.found[i];
        let fill = self.fills[i]?;
        let limit = (island.area() * MAX_GROWTH).ceil() as usize + 1;
        let mut shape: HashSet<usize> = seeds.iter().copied().collect();
        let mut stack = seeds;
        let mut strays = 0;
        while let Some(p) = stack.pop() {
            for q in neighbours4(p, w, h).chain(corner_neighbours(p, w, h).filter(|_| walk.eight)) {
                if self.colour(q) != fill || !shape.insert(q) {
                    continue;
                }
                let o = self.owner[q];
                if o != u32::MAX && o as usize != i && !open.contains(&(o as usize)) {
                    strays += 1;
                    if strays > walk.foreign || self.fills[o as usize] == Some(fill) {
                        return None;
                    }
                }
                stack.push(q);
            }
            if shape.len() > limit {
                return None;
            }
        }
        if (shape.len() as f64) < walk.share * island.area() {
            return None;
        }
        let mut pixels: Vec<usize> = shape.into_iter().collect();
        pixels.sort_unstable();
        Some(pixels)
    }

    /// The polygon island `i`'s pixels settle: a shape of straight pixel
    /// runs, or failing that, for a thin region, a straight line.
    fn polygon(&self, i: usize, open: &[usize]) -> Option<Vec<Point>> {
        let w = self.source.width;
        let island = &self.found[i];
        let own = self.seeds(i, false);
        let runs = Walk {
            eight: false,
            share: 1. - AREA_SLACK,
            foreign: 0,
        };
        let rectilinear = (!own.is_empty())
            .then(|| self.own(i, open, own.clone(), runs))
            .flatten()
            .and_then(|pixels| crate::recovery::pixel_edges(&pixels, w))
            .filter(|polygon| no_staircase(polygon));
        if rectilinear.is_some() || 2. * island.area() >= THIN * perimeter(&island.outline) {
            return rectilinear;
        }
        let own = if own.is_empty() {
            self.seeds(i, true)
        } else {
            own
        };
        let line = Walk {
            eight: true,
            share: LINE_SHARE,
            foreign: FOREIGN,
        };
        (!own.is_empty())
            .then(|| self.own(i, open, own, line))
            .flatten()
            .and_then(|pixels| line_band(&pixels, w))
    }
}

/// A subpath's edges, each once, in order.
fn edge_set(sub: &Subpath) -> Vec<EdgeKey> {
    let mut keys: Vec<EdgeKey> = sub.edges.iter().map(|e| e.key().0).collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Whether the traced outline lies on the polygon already: every point of
/// it within `SETTLED` of the polygon, and one within `SETTLED` of every
/// corner (a 2 by 2 dot's traced circle keeps within 0.2 px of its square
/// but has none of its corners).
fn settled(island: &Island, polygon: &[Point]) -> bool {
    island
        .outline
        .iter()
        .all(|&p| to_polygon(p, polygon) <= SETTLED)
        && polygon
            .iter()
            .all(|&c| island.outline.iter().any(|&p| dist(p, c) <= SETTLED))
}

/// The polygon turning as outer outlines do (positive; holes the other way).
fn outward(mut polygon: Vec<Point>) -> Vec<Point> {
    if turning(&polygon) < 0. {
        polygon.reverse();
    }
    polygon
}

/// Whether a pixel polygon has no staircase: it is a rectangle (a 2 by 2
/// dot is its pixels too), or every run shorter than `MIN_RUN` (a thin
/// stem's end) lies between two that are not. A diagonal or a curve steps
/// by one and two pixels in a row.
fn no_staircase(polygon: &[Point]) -> bool {
    let n = polygon.len();
    let run = |i: usize| dist(polygon[i % n], polygon[(i + 1) % n]);
    n == 4
        || !(0..n).any(|i| run(i) < MIN_RUN && (run(i + n - 1) < MIN_RUN || run(i + 1) < MIN_RUN))
}

/// A straight chain of pixels as one band along its axis (the principal
/// axis of their centres): as long as the pixels reach along it, as wide as
/// keeps their ink. `None` for fewer than `LINE_PIXELS` pixels, a band wider
/// than `LINE_WIDTH` or shorter than `LINE_ASPECT` widths, or a centre
/// further than half the width and `LINE_SPREAD` from the axis.
fn line_band(pixels: &[usize], w: usize) -> Option<Vec<Point>> {
    let n = pixels.len();
    if n < LINE_PIXELS {
        return None;
    }
    let centres: Vec<Point> = pixels
        .iter()
        .map(|&p| Point {
            x: (p % w) as f64 + 0.5,
            y: (p / w) as f64 + 0.5,
        })
        .collect();
    let mean = Point {
        x: centres.iter().map(|c| c.x).sum::<f64>() / n as f64,
        y: centres.iter().map(|c| c.y).sum::<f64>() / n as f64,
    };
    let (mut sxx, mut sxy, mut syy) = (0., 0., 0.);
    for c in &centres {
        let (dx, dy) = (c.x - mean.x, c.y - mean.y);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    let angle = 0.5 * (2. * sxy).atan2(sxx - syy);
    let (ux, uy) = (angle.cos(), angle.sin());
    let along = |c: &Point| (c.x - mean.x) * ux + (c.y - mean.y) * uy;
    let across = |c: &Point| ((c.y - mean.y) * ux - (c.x - mean.x) * uy).abs();
    // A pixel reaches half its extent along the axis each way of its centre.
    let reach = 0.5 * (ux.abs() + uy.abs());
    let lo = centres.iter().map(along).fold(f64::INFINITY, f64::min) - reach;
    let hi = centres.iter().map(along).fold(f64::NEG_INFINITY, f64::max) + reach;
    let width = n as f64 / (hi - lo);
    if width > LINE_WIDTH
        || hi - lo < LINE_ASPECT * width
        || centres.iter().map(across).fold(0., f64::max) > width / 2. + LINE_SPREAD
    {
        return None;
    }
    let at = |t: f64, s: f64| Point {
        x: mean.x + ux * t - uy * s,
        y: mean.y + uy * t + ux * s,
    };
    let half = width / 2.;
    Some(vec![
        at(lo, -half),
        at(hi, -half),
        at(hi, half),
        at(lo, half),
    ])
}

fn perimeter(outline: &[Point]) -> f64 {
    (0..outline.len())
        .map(|k| dist(outline[k], outline[(k + 1) % outline.len()]))
        .sum()
}

fn lines(polygon: &[Point]) -> Vec<Edge> {
    (0..polygon.len())
        .map(|i| Edge::line(polygon[i], polygon[(i + 1) % polygon.len()], false))
        .collect()
}

/// The distance from `p` to the polygon's outline.
fn to_polygon(p: Point, polygon: &[Point]) -> f64 {
    (0..polygon.len())
        .map(|i| to_segment(p, polygon[i], polygon[(i + 1) % polygon.len()]))
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests;
