//! Shapes of a vector document as a user sees them: one filled region (an
//! outer outline with the holes inside it), found by clicking on it, removed
//! as a whole, or rasterised back to a pixel mask so a region can be recoloured
//! in the source and traced again. Owned post-processing of the engine's
//! output, like `simplify`.
use crate::geometry::{Cubic, Point};
use crate::simplify::{parse_all_paths, write_path_data, Subpath};

/// Points per curve piece when an outline is flattened for hit tests and masks.
const FLATTEN_STEPS: usize = 12;

/// One curve piece of an outline, for drawing it at any zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub cubic: Cubic,
    pub line: bool,
}

/// One filled region: the outline of an outer contour and the holes it holds.
#[derive(Clone, Debug)]
pub struct Island {
    /// The path's fill attribute, e.g. `#6a75d7`.
    pub color: String,
    /// Index of the `<path>` in the document and of the outer contour in it.
    pub path: usize,
    pub outer: usize,
    /// Indices of the contours that are holes of this island.
    pub holes: Vec<usize>,
    pub outline: Vec<Point>,
    pub hole_outlines: Vec<Vec<Point>>,
    /// The exact pieces of the outer contour, then of each hole in `holes`
    /// order.
    pub pieces: Vec<Vec<Piece>>,
    pub min: Point,
    pub max: Point,
}
impl Island {
    /// Inside the outer outline and outside every hole.
    pub fn contains(&self, at: Point) -> bool {
        at.x >= self.min.x
            && at.x <= self.max.x
            && at.y >= self.min.y
            && at.y <= self.max.y
            && inside(&self.outline, at)
            && !self.hole_outlines.iter().any(|hole| inside(hole, at))
    }
    /// A point inside the island (inside its outline, outside its holes): a
    /// hair off the midpoint of one of the outline's edges. `None` only for
    /// an island so thin that no probe lands in it.
    pub fn probe(&self) -> Option<Point> {
        let n = self.outline.len();
        for i in 0..n {
            let (a, b) = (self.outline[i], self.outline[(i + 1) % n]);
            let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
            if len < 1e-9 {
                continue;
            }
            let mid = Point {
                x: (a.x + b.x) / 2.,
                y: (a.y + b.y) / 2.,
            };
            if let Some(found) = probe_beside(mid, a, b, len, |c| self.contains(c)) {
                return Some(found);
            }
        }
        None
    }
    /// Pixel-area of the island, from its flattened outline.
    pub fn area(&self) -> f64 {
        let outer = polygon_area(&self.outline);
        let holes: f64 = self.hole_outlines.iter().map(|h| polygon_area(h)).sum();
        (outer - holes).max(0.)
    }
    /// Which pixels of a `width` by `height` image the island covers, judged
    /// at pixel centres; `dilate` also takes every pixel next to a covered one,
    /// so the blended edge pixels between two regions go with the region.
    pub fn mask(&self, width: usize, height: usize, dilate: bool) -> Vec<bool> {
        let mut mask = vec![false; width * height];
        for index in self.covered(width, height, dilate) {
            mask[index as usize] = true;
        }
        mask
    }
    /// The pixels `mask` sets, as indices into the `width` by `height` image
    /// in row order. The work stays within the island's box (a pixel wider
    /// when dilating), so it costs the island's size rather than the
    /// image's: the desktop masks every island of a picture to drop a colour.
    pub fn covered(&self, width: usize, height: usize, dilate: bool) -> Vec<u32> {
        let mut polygons: Vec<&[Point]> = vec![&self.outline];
        polygons.extend(self.hole_outlines.iter().map(|h| h.as_slice()));
        let y0 = self.min.y.floor().max(0.) as usize;
        let y1 = (self.max.y.ceil().max(0.) as usize).min(height);
        // Each row's covered runs, between pairs of crossings of its centre line.
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        let mut crossings: Vec<f64> = Vec::new();
        for y in y0..y1 {
            let scan = y as f64 + 0.5;
            crossings.clear();
            for polygon in &polygons {
                let n = polygon.len();
                for i in 0..n {
                    let a = polygon[i];
                    let b = polygon[(i + 1) % n];
                    if (a.y > scan) != (b.y > scan) {
                        crossings.push(a.x + (scan - a.y) / (b.y - a.y) * (b.x - a.x));
                    }
                }
            }
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in crossings.as_chunks::<2>().0 {
                let from = (pair[0] - 0.5).ceil().max(0.) as usize;
                let to = ((pair[1] - 0.5).floor().max(-1.) as isize + 1).max(0) as usize;
                let to = to.min(width);
                if from < to {
                    runs.push((y, from, to));
                }
            }
        }
        if runs.is_empty() {
            return Vec::new();
        }
        // The dilation looks at the box one pixel wider, within the image.
        let (dx0, dx1) = (
            (self.min.x.floor().max(1.) as usize).saturating_sub(1),
            ((self.max.x.ceil().max(0.) as usize) + 2).min(width),
        );
        let (dy0, dy1) = (y0.saturating_sub(1), (y1 + 1).min(height));
        // One window holding the runs and the dilated box; nothing outside
        // it is allocated or scanned.
        let (mut wx0, mut wy0, mut wx1, mut wy1) = (usize::MAX, usize::MAX, 0, 0);
        for &(y, from, to) in &runs {
            (wx0, wy0, wx1, wy1) = (wx0.min(from), wy0.min(y), wx1.max(to), wy1.max(y + 1));
        }
        let dilating = dilate && dx0 < dx1 && dy0 < dy1;
        if dilating {
            (wx0, wy0, wx1, wy1) = (wx0.min(dx0), wy0.min(dy0), wx1.max(dx1), wy1.max(dy1));
        }
        let (ww, wh) = (wx1 - wx0, wy1 - wy0);
        let mut local = vec![false; ww * wh];
        for &(y, from, to) in &runs {
            let row = (y - wy0) * ww;
            local[row + from - wx0..row + to - wx0].fill(true);
        }
        if dilating {
            let source = local.clone();
            let covered = |x: i64, y: i64| {
                x >= wx0 as i64
                    && y >= wy0 as i64
                    && x < wx1 as i64
                    && y < wy1 as i64
                    && source[(y as usize - wy0) * ww + x as usize - wx0]
            };
            for y in dy0..dy1 {
                for x in dx0..dx1 {
                    let index = (y - wy0) * ww + x - wx0;
                    let (x, y) = (x as i64, y as i64);
                    if !source[index]
                        && (covered(x, y - 1)
                            || covered(x - 1, y)
                            || covered(x + 1, y)
                            || covered(x, y + 1))
                    {
                        local[index] = true;
                    }
                }
            }
        }
        let mut indices = Vec::new();
        for ly in 0..wh {
            for lx in 0..ww {
                if local[ly * ww + lx] {
                    indices.push(((wy0 + ly) * width + wx0 + lx) as u32);
                }
            }
        }
        indices
    }
}

/// The islands of a document in paint order (later paths cover earlier ones).
pub fn islands(svg: &str) -> Result<Vec<Island>, String> {
    let (ranges, paths) = parse_all_paths(svg)?;
    Ok(islands_in(svg, &ranges, &paths))
}

/// The islands of paths already parsed from `svg`, `ranges` locating each
/// path's data.
fn islands_in(svg: &str, ranges: &[(usize, usize)], paths: &[Vec<Subpath>]) -> Vec<Island> {
    ranges
        .iter()
        .zip(paths)
        .enumerate()
        .flat_map(|(path_index, (&(start, _), subpaths))| {
            islands_of(subpaths, path_index, &fill_before(svg, start))
        })
        .collect()
}

/// The topmost island under `at`, if any.
pub fn island_at(islands: &[Island], at: Point) -> Option<usize> {
    islands.iter().rposition(|island| island.contains(at))
}

/// A region to take out: identified by its colour and a point inside it, so
/// the same region is found again after the curves are simplified or the
/// image is converted again.
#[derive(Clone, Debug, PartialEq)]
pub struct Removal {
    pub color: String,
    pub at: Point,
}

/// The document without the listed regions (each outer contour with its
/// holes). A path left with no contours is dropped entirely. Regions that are
/// not found are ignored. Returns the document and how many were removed.
pub fn remove_islands(svg: &str, removals: &[Removal]) -> Result<(String, usize), String> {
    if removals.is_empty() {
        return Ok((svg.to_owned(), 0));
    }
    // One parse serves both the island search and the rewrite.
    let (ranges, paths) = parse_all_paths(svg)?;
    let all = islands_in(svg, &ranges, &paths);
    let mut doomed: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut removed = 0;
    for removal in removals {
        let found = all.iter().rposition(|island| {
            island.color.eq_ignore_ascii_case(&removal.color) && island.contains(removal.at)
        });
        if let Some(index) = found {
            let island = &all[index];
            if doomed.insert((island.path, island.outer)) {
                removed += 1;
            }
            doomed.extend(island.holes.iter().map(|h| (island.path, *h)));
        }
    }
    if doomed.is_empty() {
        return Ok((svg.to_owned(), 0));
    }
    let mut out = String::with_capacity(svg.len());
    let mut last = 0;
    for (path_index, ((start, end), path)) in ranges.iter().zip(paths.iter()).enumerate() {
        let kept: Vec<Subpath> = path
            .iter()
            .enumerate()
            .filter(|(i, _)| !doomed.contains(&(path_index, *i)))
            .map(|(_, s)| s.clone())
            .collect();
        if kept.is_empty() {
            // Drop the whole element when its extent can be found.
            if let Some((element_start, element_end)) = element_bounds(svg, *start, *end) {
                if element_start >= last {
                    out.push_str(&svg[last..element_start]);
                    last = element_end;
                    continue;
                }
            }
        }
        out.push_str(&svg[last..*start]);
        out.push_str(&write_path_data(&kept));
        last = *end;
    }
    out.push_str(&svg[last..]);
    Ok((out, removed))
}

/// The `<path ... />` element around a `d` attribute, with a following line
/// break; `None` when the markup is not the engine's simple form.
fn element_bounds(svg: &str, d_start: usize, d_end: usize) -> Option<(usize, usize)> {
    let element_start = svg[..d_start].rfind("<path")?;
    if svg[element_start..d_start].contains('>') {
        return None;
    }
    let close = d_end + svg[d_end..].find("/>")? + 2;
    if svg[d_end..close].contains('<') {
        return None;
    }
    let mut end = close;
    if svg[end..].starts_with("\r\n") {
        end += 2;
    } else if svg[end..].starts_with('\n') {
        end += 1;
    }
    Some((element_start, end))
}

/// The fill attribute of the element holding the `d` attribute at `d_start`,
/// wherever it sits in the tag.
fn fill_before(svg: &str, d_start: usize) -> String {
    let element_start = svg[..d_start].rfind("<path").unwrap_or(0);
    let element_end = svg[d_start..].find('>').map_or(svg.len(), |i| d_start + i);
    let head = &svg[element_start..element_end];
    head.find(" fill=\"")
        .map(|at| at + 1)
        .and_then(|at| {
            let value = &head[at + 6..];
            value.find('"').map(|end| value[..end].to_owned())
        })
        .unwrap_or_default()
}

fn islands_of(subpaths: &[Subpath], path: usize, color: &str) -> Vec<Island> {
    let outlines: Vec<Vec<Point>> = subpaths.iter().map(flatten).collect();
    let n = outlines.len();
    // Nesting depth of each contour: how many other contours of the same path
    // hold a point of it. Even depths are outer outlines, odd ones holes. The
    // probe is a point just inside the contour's first edge rather than a
    // vertex, so contours that touch at a vertex are not misread.
    let probes: Vec<Option<Point>> = outlines.iter().map(|o| probe_point(o)).collect();
    let depth: Vec<usize> = (0..n)
        .map(|i| {
            let Some(probe) = probes[i] else {
                return 0;
            };
            (0..n)
                .filter(|&j| j != i && outlines[j].len() >= 3 && inside(&outlines[j], probe))
                .count()
        })
        .collect();
    let mut islands = Vec::new();
    for i in 0..n {
        if outlines[i].len() < 3 || depth[i] % 2 == 1 {
            continue;
        }
        let holes: Vec<usize> = (0..n)
            .filter(|&j| {
                j != i
                    && depth[j] == depth[i] + 1
                    && outlines[j].len() >= 3
                    && probes[j].is_some_and(|probe| inside(&outlines[i], probe))
            })
            .collect();
        let (min, max) = bounds(&outlines[i]);
        let pieces_of = |s: &Subpath| -> Vec<Piece> {
            s.edges
                .iter()
                .map(|e| Piece {
                    cubic: e.cubic,
                    line: e.line,
                })
                .collect()
        };
        let mut pieces = vec![pieces_of(&subpaths[i])];
        pieces.extend(holes.iter().map(|&h| pieces_of(&subpaths[h])));
        islands.push(Island {
            color: color.to_owned(),
            path,
            outer: i,
            hole_outlines: holes.iter().map(|&h| outlines[h].clone()).collect(),
            holes,
            outline: outlines[i].clone(),
            pieces,
            min,
            max,
        });
    }
    islands
}

fn flatten(subpath: &Subpath) -> Vec<Point> {
    let mut points = Vec::new();
    for edge in &subpath.edges {
        if points.is_empty() {
            points.push(edge.start());
        }
        if edge.line {
            points.push(edge.end());
        } else {
            for step in 1..=FLATTEN_STEPS {
                points.push(edge.cubic.evaluate(step as f64 / FLATTEN_STEPS as f64));
            }
        }
    }
    if points.len() > 1 {
        let (first, last) = (points[0], points[points.len() - 1]);
        if (first.x - last.x).abs() < 1e-9 && (first.y - last.y).abs() < 1e-9 {
            points.pop();
        }
    }
    points
}

/// A point inside the polygon, a hair off the midpoint of its first edge on
/// the inner side; `None` for degenerate outlines.
fn probe_point(polygon: &[Point]) -> Option<Point> {
    if polygon.len() < 3 {
        return None;
    }
    let (a, b) = (polygon[0], polygon[1]);
    let mid = Point {
        x: (a.x + b.x) / 2.,
        y: (a.y + b.y) / 2.,
    };
    let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    if len < 1e-9 {
        return polygon.first().copied();
    }
    if let Some(found) = probe_beside(mid, a, b, len, |c| inside(polygon, c)) {
        return Some(found);
    }
    Some(mid)
}

/// The first point beside the chord a-b's midpoint that `inside` accepts:
/// three distances along the chord's normal, each tried on both sides.
fn probe_beside(
    mid: Point,
    a: Point,
    b: Point,
    len: f64,
    inside: impl Fn(Point) -> bool,
) -> Option<Point> {
    let normal = Point {
        x: -(b.y - a.y) / len,
        y: (b.x - a.x) / len,
    };
    for step in [0.05, 0.5, 2.] {
        for sign in [1., -1.] {
            let candidate = Point {
                x: mid.x + normal.x * step * sign,
                y: mid.y + normal.y * step * sign,
            };
            if inside(candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn bounds(points: &[Point]) -> (Point, Point) {
    let mut min = Point {
        x: f64::INFINITY,
        y: f64::INFINITY,
    };
    let mut max = Point {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
    };
    for p in points {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    (min, max)
}

/// Even-odd point-in-polygon.
fn inside(polygon: &[Point], at: Point) -> bool {
    let n = polygon.len();
    let mut hit = false;
    for i in 0..n {
        let a = polygon[i];
        let b = polygon[(i + 1) % n];
        if (a.y > at.y) != (b.y > at.y) {
            let x = a.x + (at.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if at.x < x {
                hit = !hit;
            }
        }
    }
    hit
}

fn polygon_area(polygon: &[Point]) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| {
            let a = polygon[i];
            let b = polygon[(i + 1) % n];
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        .abs()
        / 2.
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "<svg width=\"100pt\" height=\"100pt\" viewBox=\"0 0 100 100\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"#000000ff\">\n<path fill=\"#000000\" opacity=\"1.00\" d=\" M 0.00 0.00 L 100.00 0.00 L 100.00 100.00 L 0.00 100.00 L 0.00 0.00 M 20.00 20.00 L 20.00 80.00 L 80.00 80.00 L 80.00 20.00 L 20.00 20.00 M 40.00 40.00 L 60.00 40.00 L 60.00 60.00 L 40.00 60.00 L 40.00 40.00 Z\" />\n</g>\n<g id=\"#ff0000ff\">\n<path fill=\"#ff0000\" opacity=\"1.00\" d=\" M 20.00 20.00 L 80.00 20.00 L 80.00 80.00 L 20.00 80.00 L 20.00 20.00 M 40.00 40.00 L 40.00 60.00 L 60.00 60.00 L 60.00 40.00 L 40.00 40.00 Z\" />\n<path fill=\"#ff0000\" opacity=\"1.00\" d=\" M 5.00 5.00 C 8.00 5.00 10.00 7.00 10.00 10.00 C 10.00 13.00 8.00 15.00 5.00 15.00 C 2.00 15.00 0.00 13.00 0.00 10.00 C 0.00 7.00 2.00 5.00 5.00 5.00 Z\" />\n</g>\n</svg>\n";

    fn p(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn islands_nest_holes_and_keep_islands_inside_holes_separate() {
        let all = islands(DOC).unwrap();
        // Black: the frame (hole at 20..80) and the centre square inside that
        // hole; red: the ring (hole at 40..60) and the small blob.
        assert_eq!(all.len(), 4, "{all:?}");
        let frame = &all[0];
        assert_eq!(
            (frame.color.as_str(), frame.path, frame.outer),
            ("#000000", 0, 0)
        );
        assert_eq!(frame.holes, vec![1]);
        let centre = &all[1];
        assert_eq!((centre.path, centre.outer), (0, 2));
        assert!(centre.holes.is_empty());
        assert!(frame.contains(p(10., 50.)) && !frame.contains(p(50., 50.)));
        assert!(centre.contains(p(50., 50.)));
        assert_eq!(
            island_at(&all, p(30., 50.)),
            Some(2),
            "the red ring is on top"
        );
        assert_eq!(island_at(&all, p(50., 50.)), Some(1));
        assert_eq!(island_at(&all, p(5., 10.)), Some(3));
        assert_eq!(island_at(&all, p(200., 200.)), None);
        assert!((all[2].area() - (3600. - 400.)).abs() < 1e-6);
    }

    #[test]
    fn removing_an_island_takes_its_holes_and_drops_empty_paths() {
        let (out, removed) = remove_islands(
            DOC,
            &[
                Removal {
                    color: "#ff0000".into(),
                    at: p(30., 50.),
                },
                Removal {
                    color: "#00ff00".into(),
                    at: p(30., 50.),
                },
                Removal {
                    color: "#ff0000".into(),
                    at: p(5., 10.),
                },
            ],
        )
        .unwrap();
        assert_eq!(removed, 2, "the wrong colour is ignored");
        assert!(
            !out.contains("fill=\"#ff0000\""),
            "both red paths are gone:\n{out}"
        );
        assert!(out.contains("<g id=\"#ff0000ff\">\n</g>"));
        let left = islands(&out).unwrap();
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].holes, vec![1]);
        // Removing the frame keeps the centre square that sat in its hole.
        let (out, removed) = remove_islands(
            DOC,
            &[Removal {
                color: "#000000".into(),
                at: p(10., 10.),
            }],
        )
        .unwrap();
        assert_eq!(removed, 1);
        let left = islands(&out).unwrap();
        assert_eq!(left.len(), 3);
        assert_eq!(
            (left[0].color.as_str(), left[0].outline.len()),
            ("#000000", 4)
        );
        assert!(left[0].contains(p(50., 50.)));
        assert_eq!(remove_islands(DOC, &[]).unwrap(), (DOC.to_owned(), 0));
    }

    #[test]
    fn masks_cover_the_region_at_pixel_centres_and_dilate_by_one() {
        let all = islands(DOC).unwrap();
        let ring = &all[2];
        let mask = ring.mask(100, 100, false);
        assert!(mask[50 * 100 + 30] && !mask[50 * 100 + 50] && !mask[10 * 100 + 10]);
        assert_eq!(mask.iter().filter(|m| **m).count(), 3600 - 400);
        let wide = ring.mask(100, 100, true);
        assert!(wide[50 * 100 + 19] && wide[50 * 100 + 40] && !wide[50 * 100 + 18]);
        assert_eq!(wide.iter().filter(|m| **m).count(), 3200 + 240 + 76);
    }

    #[test]
    fn covered_pixels_are_the_pixel_centres_inside_within_the_image() {
        // Against a plain test of every pixel centre, dilated by every pixel
        // beside a covered one: in the whole image, and in one that cuts the
        // frame, the ring and the blob off at its right and bottom.
        for (width, height) in [(100, 100), (45, 12)] {
            for island in &islands(DOC).unwrap() {
                let inside: Vec<bool> = (0..width * height)
                    .map(|i| island.contains(p((i % width) as f64 + 0.5, (i / width) as f64 + 0.5)))
                    .collect();
                for dilate in [false, true] {
                    let expected: Vec<u32> = (0..width * height)
                        .filter(|&i| {
                            let (x, y) = (i % width, i / width);
                            inside[i]
                                || (dilate
                                    && ((x > 0 && inside[i - 1])
                                        || (x + 1 < width && inside[i + 1])
                                        || (y > 0 && inside[i - width])
                                        || (y + 1 < height && inside[i + width])))
                        })
                        .map(|i| i as u32)
                        .collect();
                    assert_eq!(
                        island.covered(width, height, dilate),
                        expected,
                        "{} in {width} x {height}, dilate {dilate}",
                        island.color
                    );
                }
            }
        }
    }
}
