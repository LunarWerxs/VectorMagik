//! Detail the segmentation merged away, drawn back from the pixels.
//!
//! The engine's segmentation merges a small region into its neighbour when
//! their colours are close: on the shape set's near colours, squares of 20,
//! 16 and 12 px twelve levels off a blue field vanish at every preset and
//! advanced setting measured, the original's included (the defect sweep of
//! September 23, 2026). The pixels still show them, and the traced drawing
//! is where to look: every pixel is compared with the fill of the region
//! drawn over it, and a blob of pixels that differ from that fill, of one
//! colour among themselves and enclosed by that one region, is drawn back
//! as a region of its own, with the same outline cut out of its host as a
//! hole (so the two share their edge, as every region of the engine's
//! documents does).
//!
//! What counts as lost is kept narrow: a blob at least `MIN_THICKNESS`
//! pixels thick somewhere (JPEG's colour blocks left 4 and 5 px blobs in a
//! compressed logo, which the rule would have drawn; the lost squares are
//! 12 px and more), its core (the pixels whose four neighbours are in it)
//! within `MAX_SPREAD` levels of its mean, the mean at least
//! `MIN_CONTRAST` levels off the host's fill, and no pixel of the blob
//! beside another region or the picture's border (a blob against an edge is
//! a boundary drawn in the wrong place, not a lost region; on an exact
//! palette the border is no exception, `recover_exact`). Thin strokes,
//! gradients and texture have no such core and are left alone; photographs
//! are not passed here. On anti-aliased
//! artwork the outline is the half-coverage contour of the blob's colour
//! over the host's (marching squares on the pixel centres); on pixel-edged
//! artwork, and for a blob with no partly covered pixel, it is the blob's
//! pixel edges.
use crate::geometry::{turning, Point};
use crate::raster::{corner_neighbours, hex_rgb, neighbours4, Raster};
use crate::shapes::islands;
use crate::simplify::path_ranges;

/// Levels (largest channel) a pixel must differ from its region's fill to
/// belong to a blob; the blob's rim on anti-aliased artwork is a blend, so
/// this is below the contrast asked of its core.
const MEMBER: u8 = 5;
/// Levels (largest channel) the blob's core colour must be off its host's.
const MIN_CONTRAST: u8 = 10;
/// Pixels: some pixel of the blob has every pixel within this many steps
/// (a diamond of this width) in the blob.
const MIN_THICKNESS: usize = 7;
/// Levels (largest channel) the core's pixels may stray from their mean.
const MAX_SPREAD: u8 = 6;
/// Coverage of the blob's colour at or above which a pixel is wholly the
/// blob's.
const HARD: f64 = 0.75;
/// Pixels: the outline's polygon is simplified within this distance.
const SIMPLIFY: f64 = 0.15;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoveryStats {
    /// Blobs found, and drawn back.
    pub blobs: usize,
    pub recovered: usize,
}

/// `svg` (an engine document of `source`) with the regions its
/// segmentation lost drawn back. `anti_aliased` chooses the outline.
pub fn recover_svg(
    svg: &str,
    source: &Raster,
    anti_aliased: bool,
) -> Result<(String, RecoveryStats), String> {
    recover(svg, source, anti_aliased, false)
}

/// `svg` (a document of pixel-edged `source` with an exact palette, every
/// region already in the colour of its pixels and its lines and pixel
/// shapes on their pixels) with every blob of one colour still lost drawn
/// back on its pixel edges, however small: a dotted line's dots and lone
/// pixels, which the unblended presets merge away. The thickness test is
/// for blended pixels and JPEG's blocks; here every pixel is a palette
/// colour, and a blob against the picture's border is taken too (no traced
/// boundary lies there to be misplaced). Run before the regions' colours and
/// lines were right, this drew back every 1 px line the engine had traced in
/// a blend (exact-recovery.md).
/// The blobs are painted over their region, one path per colour, with no
/// hole cut: a square per pixel with its own group, a hole to match and the
/// desktop's seam strip under that hole cost about 290 bytes a pixel. A blob
/// touching a pixel drawn in its own colour is a step that shape's outline
/// cut, not a lost feature, and is left alone. A 1 px checker the engine
/// merged into its field (drawn plain white before) comes back pixel for
/// pixel: 96 px of checker, 1 KB before, 142 KB; the rule ranks the
/// drawing's faithfulness before its size.
pub fn recover_exact(svg: &str, source: &Raster) -> Result<(String, RecoveryStats), String> {
    recover(svg, source, false, true)
}

/// Both of the above: `exact` takes every blob of one exact colour, of any
/// size.
fn recover(
    svg: &str,
    source: &Raster,
    anti_aliased: bool,
    exact: bool,
) -> Result<(String, RecoveryStats), String> {
    let (w, h) = (source.width, source.height);
    let islands = islands(svg)?;
    let mut owner = vec![u32::MAX; w * h];
    for (i, island) in islands.iter().enumerate() {
        for p in island.covered(w, h, false) {
            owner[p as usize] = i as u32;
        }
    }
    let fills: Vec<Option<[u8; 3]>> = islands.iter().map(|i| hex_rgb(&i.color)).collect();
    let fill_of = |p: usize| -> Option<[u8; 3]> {
        let o = owner[p];
        (o != u32::MAX).then(|| fills[o as usize]).flatten()
    };
    let colour = |p: usize| {
        let c = source.pixels[p].0;
        [c[0], c[1], c[2]]
    };
    let member = |p: usize| -> bool {
        source.pixels[p].0[3] == 255 && fill_of(p).is_some_and(|f| distance(colour(p), f) >= MEMBER)
    };
    let mut seen = vec![false; w * h];
    let mut stats = RecoveryStats::default();
    let mut holes: Vec<(usize, Vec<Point>)> = Vec::new();
    let mut shapes: Vec<([u8; 3], Vec<Point>)> = Vec::new();
    for start in 0..w * h {
        if seen[start] || !member(start) {
            continue;
        }
        // The blob: 4-connected members under the same region.
        let host = owner[start];
        let mut blob = vec![start];
        seen[start] = true;
        let mut enclosed = true;
        let mut next = 0;
        while next < blob.len() {
            let p = blob[next];
            next += 1;
            let (x, y) = (p % w, p / w);
            // On an exact palette the border is no boundary drawn in the
            // wrong place: every pixel is its colour, and a dot on the edge
            // is lost like any other (a dither's border dots were, September
            // 25, 2026).
            if !exact && (x == 0 || y == 0 || x + 1 == w || y + 1 == h) {
                enclosed = false;
            }
            for q in neighbours4(p, w, h) {
                if owner[q] != host {
                    enclosed = false;
                } else if !seen[q] && member(q) && (!exact || colour(q) == colour(start)) {
                    seen[q] = true;
                    blob.push(q);
                }
            }
        }
        // In exact mode a blob touching, at an edge or a corner, a pixel
        // drawn in its own colour is a step of that shape the traced outline
        // cut (a pixel-edged thin ring's or hairline's), not a lost feature:
        // drawn back square by square it cost the rings 70% more bytes.
        let step = exact
            && blob.iter().any(|&p| {
                neighbours4(p, w, h)
                    .chain(corner_neighbours(p, w, h))
                    .any(|q| fill_of(q) == Some(colour(start)))
            });
        if !enclosed || step {
            continue;
        }
        let inside = |q: usize, blob: &[usize]| blob.binary_search(&q).is_ok();
        let eroded = |set: &[usize]| -> Vec<usize> {
            set.iter()
                .copied()
                .filter(|&p| neighbours4(p, w, h).all(|q| inside(q, set)))
                .collect()
        };
        blob.sort_unstable();
        // An exact colour's blob is its core, of any size.
        let core = if exact { blob.clone() } else { eroded(&blob) };
        let mut deepest = core.clone();
        for _ in 1..MIN_THICKNESS / 2 {
            if exact {
                break;
            }
            deepest = eroded(&deepest);
        }
        if deepest.is_empty() {
            continue;
        }
        stats.blobs += 1;
        let mut sum = [0u64; 3];
        for &p in &core {
            for (s, c) in sum.iter_mut().zip(colour(p)) {
                *s += c as u64;
            }
        }
        let n = core.len() as u64;
        let mean = sum.map(|s| ((s + n / 2) / n) as u8);
        let host_fill = fills[host as usize].unwrap_or([0; 3]);
        if distance(mean, host_fill) < MIN_CONTRAST
            || core.iter().any(|&p| distance(colour(p), mean) > MAX_SPREAD)
        {
            continue;
        }
        // A blob with no partly covered pixel sits on the pixel grid even in
        // anti-aliased artwork: its pixel edges are exact, where the
        // half-coverage contour cuts each corner by half a pixel (0.33 px off
        // the near colours' squares).
        let hard = blob
            .iter()
            .all(|&p| coverage(colour(p), host_fill, mean) >= HARD);
        let outline = if anti_aliased && !hard {
            half_coverage(&blob, w, |p| {
                if inside(p, &blob) {
                    coverage(colour(p), host_fill, mean)
                } else {
                    0.
                }
            })
        } else {
            pixel_edges(&blob, w)
        };
        let Some(outline) = outline else {
            continue;
        };
        let outline = simplified(&outline, SIMPLIFY);
        if outline.len() < 3 {
            continue;
        }
        stats.recovered += 1;
        // In exact mode drawn over the host, no hole cut (see `recover_exact`).
        if !exact {
            holes.push((islands[host as usize].path, outline.clone()));
        }
        shapes.push((mean, outline));
    }
    if shapes.is_empty() {
        return Ok((svg.to_owned(), stats));
    }
    Ok((write(svg, &holes, &shapes, exact)?, stats))
}

/// The largest channel difference of two colours.
fn distance(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0)
}

/// How much of `feature` over `host` the colour `c` is: its projection on
/// the line between them, from 0 to 1.
fn coverage(c: [u8; 3], host: [u8; 3], feature: [u8; 3]) -> f64 {
    let d: Vec<f64> = (0..3).map(|i| feature[i] as f64 - host[i] as f64).collect();
    let v: f64 = (0..3).map(|i| (c[i] as f64 - host[i] as f64) * d[i]).sum();
    let n: f64 = d.iter().map(|x| x * x).sum();
    if n == 0. {
        0.
    } else {
        (v / n).clamp(0., 1.)
    }
}

/// Where the contour crosses the line between two pixel centres of the
/// grid: the first centre and whether the line runs across (to the next
/// column) or down.
type Crossing = (usize, usize, bool);

/// The blob's outline as the 0.5 contour of `alpha` sampled at pixel
/// centres over its box and a pixel round it (marching squares, linear
/// between centres), when that contour is one loop.
fn half_coverage(blob: &[usize], w: usize, alpha: impl Fn(usize) -> f64) -> Option<Vec<Point>> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for &p in blob {
        let (x, y) = (p % w, p / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    // The grid: centres from x0 - 1 to x1 + 1 (the blob never touches the
    // border, so they exist), values outside the blob's neighbourhood 0.
    let (gx0, gy0) = (x0 - 1, y0 - 1);
    let (gw, gh) = (x1 - x0 + 3, y1 - y0 + 3);
    let value: Vec<f64> = (0..gw * gh)
        .map(|i| alpha((gy0 + i / gw) * w + gx0 + i % gw))
        .collect();
    let mut loops = contour_loops(&value, gw, gh, (gx0, gy0), 0.5)?;
    if loops.len() != 1 {
        return None;
    }
    loops.pop()
}

/// Every loop of the `level` contour of `value`, a `gw` by `gh` grid of
/// samples at pixel centres whose first sits at the centre of pixel
/// `origin` (marching squares, linear between centres, a saddle cell
/// resolved by the mean of its corners). The grid's outer ring must lie
/// below `level`, so every loop closes; `None` if one does not.
pub(crate) fn contour_loops(
    value: &[f64],
    gw: usize,
    gh: usize,
    origin: (usize, usize),
    level: f64,
) -> Option<Vec<Vec<Point>>> {
    let (gx0, gy0) = origin;
    let at = |gx: usize, gy: usize| value[gy * gw + gx];
    // Each crossing of an edge between two centres is a vertex, keyed by
    // the edge; each cell joins its crossings into segments.
    let mut segments: Vec<(Crossing, Crossing)> = Vec::new();
    for cy in 0..gh - 1 {
        for cx in 0..gw - 1 {
            let corners = [
                at(cx, cy),
                at(cx + 1, cy),
                at(cx + 1, cy + 1),
                at(cx, cy + 1),
            ];
            let bits = corners
                .iter()
                .enumerate()
                .fold(0, |b, (i, v)| b | (usize::from(*v >= level) << i));
            // Edges of the cell: top (cx, cy, horizontal), right, bottom, left.
            let top = (cx, cy, true);
            let right = (cx + 1, cy, false);
            let bottom = (cx, cy + 1, true);
            let left = (cx, cy, false);
            let centre = corners.iter().sum::<f64>() / 4. >= level;
            let pairs: &[(Crossing, Crossing)] = match bits {
                0 | 15 => &[],
                1 | 14 => &[(left, top)],
                2 | 13 => &[(top, right)],
                3 | 12 => &[(left, right)],
                4 | 11 => &[(right, bottom)],
                6 | 9 => &[(top, bottom)],
                7 | 8 => &[(left, bottom)],
                5 if centre => &[(left, bottom), (top, right)],
                5 => &[(left, top), (right, bottom)],
                10 if centre => &[(left, top), (right, bottom)],
                _ => &[(left, bottom), (top, right)],
            };
            segments.extend_from_slice(pairs);
        }
    }
    // Chain the segments into loops.
    let mut links: std::collections::HashMap<Crossing, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, (a, b)) in segments.iter().enumerate() {
        links.entry(*a).or_default().push(i);
        links.entry(*b).or_default().push(i);
    }
    let mut used = vec![false; segments.len()];
    let mut loops: Vec<Vec<Crossing>> = Vec::new();
    for first in 0..segments.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let (start, mut end) = segments[first];
        let mut chain = vec![start, end];
        while end != start {
            let &next = links[&end].iter().find(|&&s| !used[s])?;
            used[next] = true;
            let (a, b) = segments[next];
            end = if a == end { b } else { a };
            chain.push(end);
        }
        chain.pop();
        loops.push(chain);
    }
    let point = |(gx, gy, horizontal): Crossing| {
        let (a, b) = if horizontal {
            (at(gx, gy), at(gx + 1, gy))
        } else {
            (at(gx, gy), at(gx, gy + 1))
        };
        let t = if (b - a).abs() < 1e-12 {
            0.5
        } else {
            (level - a) / (b - a)
        };
        let (x, y) = ((gx0 + gx) as f64 + 0.5, (gy0 + gy) as f64 + 0.5);
        if horizontal {
            Point { x: x + t, y }
        } else {
            Point { x, y: y + t }
        }
    };
    Some(
        loops
            .iter()
            .map(|chain| chain.iter().map(|&k| point(k)).collect())
            .collect(),
    )
}

/// The blob's outline along its pixel edges, corners only, when it is one
/// loop (no hole, no two parts touching at a corner).
pub(crate) fn pixel_edges(blob: &[usize], w: usize) -> Option<Vec<Point>> {
    let inside = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && blob.binary_search(&((y as usize) * w + x as usize)).is_ok()
    };
    // Directed unit edges with the blob on their right in picture
    // coordinates (y down): top edges left to right, and so on.
    let mut next: std::collections::HashMap<(i64, i64), (i64, i64)> =
        std::collections::HashMap::new();
    for &p in blob {
        let (x, y) = ((p % w) as i64, (p / w) as i64);
        let sides = [
            ((x, y - 1), (x, y), (x + 1, y)),
            ((x + 1, y), (x + 1, y), (x + 1, y + 1)),
            ((x, y + 1), (x + 1, y + 1), (x, y + 1)),
            ((x - 1, y), (x, y + 1), (x, y)),
        ];
        for ((nx, ny), a, b) in sides {
            if !inside(nx, ny) && next.insert(a, b).is_some() {
                // Two edges leave one vertex: parts touching at a corner.
                return None;
            }
        }
    }
    let start = *next.keys().min()?;
    let mut walk = vec![start];
    let mut at = next[&start];
    while at != start {
        walk.push(at);
        at = *next.get(&at)?;
        if walk.len() > next.len() {
            return None;
        }
    }
    if walk.len() != next.len() {
        return None;
    }
    let m = walk.len();
    Some(
        (0..m)
            .filter(|&i| {
                let (p, q, r) = (walk[(i + m - 1) % m], walk[i], walk[(i + 1) % m]);
                (q.0 - p.0) * (r.1 - q.1) != (q.1 - p.1) * (r.0 - q.0)
            })
            .map(|i| Point {
                x: walk[i].0 as f64,
                y: walk[i].1 as f64,
            })
            .collect(),
    )
}

/// A closed polygon with every vertex within `tolerance` of the line of its
/// kept neighbours dropped (Douglas-Peucker from its two farthest-apart
/// vertices).
pub(crate) fn simplified(polygon: &[Point], tolerance: f64) -> Vec<Point> {
    let n = polygon.len();
    if n < 4 {
        return polygon.to_vec();
    }
    let far = (1..n)
        .max_by(|&a, &b| {
            let d = |i: usize| (polygon[i].x - polygon[0].x).hypot(polygon[i].y - polygon[0].y);
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(n / 2);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    let mut stack = vec![(0, far), (far, n)];
    while let Some((a, b)) = stack.pop() {
        let (p, q) = (polygon[a], polygon[b % n]);
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let len = dx.hypot(dy).max(1e-12);
        let mut worst = (0., a);
        for (i, v) in polygon.iter().enumerate().take(b).skip(a + 1) {
            let d = ((v.x - p.x) * dy - (v.y - p.y) * dx).abs() / len;
            if d > worst.0 {
                worst = (d, i);
            }
        }
        if worst.0 > tolerance {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    polygon
        .iter()
        .zip(&keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

fn subpath(points: &[Point]) -> String {
    let mut d = format!(" M {:.2} {:.2}", points[0].x, points[0].y);
    for p in points.iter().skip(1).chain(std::iter::once(&points[0])) {
        d.push_str(&format!(" L {:.2} {:.2}", p.x, p.y));
    }
    d.push_str(" Z");
    d
}

/// The document with each hole appended to its host path and each shape
/// written after everything else, in the engine's layout (`merge`: the
/// shapes of one colour in one path).
fn write(
    svg: &str,
    holes: &[(usize, Vec<Point>)],
    shapes: &[([u8; 3], Vec<Point>)],
    merge: bool,
) -> Result<String, String> {
    let ranges = path_ranges(svg)?;
    let mut out = String::with_capacity(svg.len() + 256 * shapes.len());
    let mut last = 0;
    for (index, &(_, end)) in ranges.iter().enumerate() {
        out.push_str(&svg[last..end]);
        for (_, outline) in holes.iter().filter(|(path, _)| *path == index) {
            // Outer outlines turn positive; a hole the other way.
            let mut hole = outline.clone();
            if turning(&hole) > 0. {
                hole.reverse();
            }
            out.push_str(&subpath(&hole));
        }
        last = end;
    }
    let close = svg.rfind("</svg>").ok_or("Not an SVG document")?;
    out.push_str(&svg[last..close]);
    let newline = if svg.contains("\r\n") { "\r\n" } else { "\n" };
    if !out.ends_with('\n') {
        out.push_str(newline);
    }
    // Each shape's outline, or with `merge` each colour's outlines, in the
    // order first met.
    let mut paths: Vec<([u8; 3], String)> = Vec::new();
    for (fill, outline) in shapes {
        let mut outer = outline.clone();
        if turning(&outer) < 0. {
            outer.reverse();
        }
        match paths.iter_mut().find(|(colour, _)| merge && colour == fill) {
            Some((_, d)) => d.push_str(&subpath(&outer)),
            None => paths.push((*fill, subpath(&outer))),
        }
    }
    for (fill, d) in paths {
        let colour = format!("{:02x}{:02x}{:02x}", fill[0], fill[1], fill[2]);
        out.push_str(&format!(
            "<g id=\"#{colour}ff\">{newline}<path fill=\"#{colour}\" opacity=\"1.00\" d=\"{d}\" />{newline}</g>{newline}"
        ));
    }
    out.push_str(&svg[close..]);
    Ok(out)
}

#[cfg(test)]
mod tests;
