//! Dithered areas saved as pattern fills (September 24, 2026; the owner, on
//! a pattern fill for dithered pixel art: "I'm down with that"). A picture
//! with an exact palette comes back pixel for pixel
//! (`vector_rebuild::recovery::recover_exact`), so a dithered area is one
//! square per pixel: a 96 px checker saved as 4,608 squares, 207 KB of SVG.
//! Where one shape's squares repeat a tile of at most `MAX_PERIOD` cells each
//! way across a rectangle, the saved SVG, PDF and EPS (and the AI, the PDF's
//! bytes) draw that rectangle once, filled with the tile as a pattern: the
//! same pixels in a few hundred bytes. The DXF draws the tile as a block
//! placed as a rectangular array, and the EMF, which has no pattern that
//! scales, writes the area's squares as one polygon record (September 25,
//! 2026; `dxf.rs`, `emf.rs`). The document in the app keeps its squares, so
//! every edit and the preview see what they always did; `svg_import` draws
//! a pattern of this kind back as its squares.
use crate::pdf_eps::Segment;
use std::fmt::Write as _;

/// Cells a tile may span each way: ordered dithers repeat every 2, 4 or 8.
const MAX_PERIOD: usize = 8;
/// The fewest squares a pattern replaces: a pattern's definition costs about
/// as much as a dozen squares.
const MIN_SQUARES: usize = 16;
/// Tiles a run spans each way, at least.
const MIN_TILES: usize = 2;
/// The largest grid (cells in the bounding box of one size of square) that
/// is searched: 63 periods cost a pass over the grid each per run.
const MAX_GRID: usize = 1 << 20;
/// Runs found per shape and square size, at most.
const MAX_RUNS: usize = 256;

/// A square one subpath outlines: its corner with the smaller coordinates
/// and its side, in user units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Square {
    pub x: f64,
    pub y: f64,
    pub side: f64,
}

/// A rectangle whose squares of one size repeat one tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The rectangle in user units; its corner is the tile's.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The squares' side.
    pub side: f64,
    /// The tile's size in cells.
    pub period: (usize, usize),
    /// The tile's inked cells, counted from its corner.
    pub inked: Vec<(usize, usize)>,
}

impl Run {
    /// The tile's width and height in user units.
    pub fn tile(&self) -> (f64, f64) {
        (
            self.period.0 as f64 * self.side,
            self.period.1 as f64 * self.side,
        )
    }
}

/// The subpaths of `segments`, each from its `Move`.
pub fn subpaths(segments: &[Segment]) -> Vec<&[Segment]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, segment) in segments.iter().enumerate() {
        if matches!(segment, Segment::Move(_)) && i > start {
            out.push(&segments[start..i]);
            start = i;
        }
    }
    if start < segments.len() {
        out.push(&segments[start..]);
    }
    out
}

/// The square `subpath` outlines, when it is one: its four corners visited
/// once each along the square's sides, in either turn, closed or not.
pub fn square(subpath: &[Segment]) -> Option<Square> {
    let mut points = Vec::with_capacity(5);
    let mut closed = false;
    for (i, segment) in subpath.iter().enumerate() {
        match *segment {
            Segment::Move(p) if i == 0 => points.push(p),
            Segment::Line(p) if !closed => points.push(p),
            Segment::Close if !closed => closed = true,
            _ => return None,
        }
    }
    if points.len() == 5 && points[4] == points[0] {
        points.pop();
    }
    if points.len() != 4 {
        return None;
    }
    let (x0, x1) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.0), hi.max(p.0))
        });
    let (y0, y1) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    let side = x1 - x0;
    // The points are finite (the path reader refuses others), so no NaN.
    if side <= 0. || (y1 - y0 - side).abs() > 1e-9 * side.max(1.) {
        return None;
    }
    let corner = |p: (f64, f64)| (p.0 == x0 || p.0 == x1) && (p.1 == y0 || p.1 == y1);
    for k in 0..4 {
        let (a, b) = (points[k], points[(k + 1) % 4]);
        if !corner(a) || a == b || (a.0 != b.0 && a.1 != b.1) {
            return None;
        }
    }
    let (a, c) = (points[0], points[2]);
    (a.0 != c.0 && a.1 != c.1).then_some(Square { x: x0, y: y0, side })
}

/// The runs among `squares` (one shape's), and for each square whether a
/// run draws it.
pub fn runs(squares: &[Square]) -> (Vec<Run>, Vec<bool>) {
    let mut covered = vec![false; squares.len()];
    let mut out = Vec::new();
    if squares.len() < MIN_SQUARES {
        return (out, covered);
    }
    let mut sides: Vec<f64> = squares.iter().map(|s| s.side).collect();
    sides.sort_by(f64::total_cmp);
    sides.dedup();
    for side in sides {
        let cell = |v: f64| {
            let c = v / side;
            ((c - c.round()).abs() < 1e-6).then_some(c.round() as i64)
        };
        let cells: Vec<(i64, i64, usize)> = squares
            .iter()
            .enumerate()
            .filter(|(_, s)| s.side == side)
            .filter_map(|(i, s)| Some((cell(s.x)?, cell(s.y)?, i)))
            .collect();
        if cells.len() < MIN_SQUARES {
            continue;
        }
        let (cx0, cx1) = cells.iter().fold((i64::MAX, i64::MIN), |(lo, hi), c| {
            (lo.min(c.0), hi.max(c.0))
        });
        let (cy0, cy1) = cells.iter().fold((i64::MAX, i64::MIN), |(lo, hi), c| {
            (lo.min(c.1), hi.max(c.1))
        });
        let (w, h) = ((cx1 - cx0 + 1) as usize, (cy1 - cy0 + 1) as usize);
        if w.saturating_mul(h) > MAX_GRID {
            continue;
        }
        // The square on each cell; a second on the same cell is drawn as it is.
        let mut owner = vec![usize::MAX; w * h];
        for &(cx, cy, i) in &cells {
            let k = (cy - cy0) as usize * w + (cx - cx0) as usize;
            if owner[k] == usize::MAX {
                owner[k] = i;
            }
        }
        let mut grid = Grid::new(w, h, &owner);
        for (rect, period) in grid.runs() {
            let [x0, y0, x1, y1] = rect;
            for y in y0..y1 {
                for x in x0..x1 {
                    if let Some(&i) = owner.get(y * w + x).filter(|i| **i != usize::MAX) {
                        covered[i] = true;
                    }
                }
            }
            let inked = (0..period.1)
                .flat_map(|j| (0..period.0).map(move |i| (i, j)))
                .filter(|&(i, j)| grid.ink(x0 + i, y0 + j))
                .collect();
            out.push(Run {
                x: (cx0 + x0 as i64) as f64 * side,
                y: (cy0 + y0 as i64) as f64 * side,
                width: (x1 - x0) as f64 * side,
                height: (y1 - y0) as f64 * side,
                side,
                period,
                inked,
            });
        }
    }
    (out, covered)
}

/// `segments` without the squares that runs draw, and the runs; `None` when
/// no run is found.
pub fn split(segments: &[Segment]) -> Option<(Vec<Segment>, Vec<Run>)> {
    let parts = subpaths(segments);
    let squares: Vec<(usize, Square)> = parts
        .iter()
        .enumerate()
        .filter_map(|(i, part)| Some((i, square(part)?)))
        .collect();
    if squares.len() < MIN_SQUARES {
        return None;
    }
    let (found, covered) = runs(&squares.iter().map(|s| s.1).collect::<Vec<_>>());
    if found.is_empty() {
        return None;
    }
    let mut dropped = vec![false; parts.len()];
    for (k, (i, _)) in squares.iter().enumerate() {
        dropped[*i] = covered[k];
    }
    let rest = parts
        .iter()
        .zip(&dropped)
        .filter(|(_, d)| !**d)
        .flat_map(|(part, _)| part.iter().copied())
        .collect();
    Some((rest, found))
}

/// The squares of one size on their grid, and the rectangles already given
/// to runs.
struct Grid {
    w: usize,
    h: usize,
    ink: Vec<bool>,
    /// Inked cells above and left of each grid corner, `(w + 1) * (h + 1)`.
    sums: Vec<u32>,
    taken: Vec<bool>,
}

impl Grid {
    fn new(w: usize, h: usize, owner: &[usize]) -> Self {
        let ink: Vec<bool> = owner.iter().map(|&i| i != usize::MAX).collect();
        let mut sums = vec![0u32; (w + 1) * (h + 1)];
        for y in 0..h {
            for x in 0..w {
                sums[(y + 1) * (w + 1) + x + 1] = u32::from(ink[y * w + x])
                    + sums[y * (w + 1) + x + 1]
                    + sums[(y + 1) * (w + 1) + x]
                    - sums[y * (w + 1) + x];
            }
        }
        Self {
            w,
            h,
            ink,
            sums,
            taken: vec![false; w * h],
        }
    }

    /// Whether cell `(x, y)` holds a square; outside the grid none does.
    fn ink(&self, x: usize, y: usize) -> bool {
        x < self.w && y < self.h && self.ink[y * self.w + x]
    }

    /// Inked cells in `[x0, x1) x [y0, y1)`, clipped to the grid.
    fn count(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> u32 {
        let (x1, y1) = (x1.min(self.w), y1.min(self.h));
        if x0 >= x1 || y0 >= y1 {
            return 0;
        }
        let s = |x: usize, y: usize| self.sums[y * (self.w + 1) + x];
        s(x1, y1) + s(x0, y0) - s(x0, y1) - s(x1, y0)
    }

    /// Runs, most squares first: each round takes, over every tile size, the
    /// largest rectangle that repeats its tile and is not yet taken.
    fn runs(&mut self) -> Vec<([usize; 4], (usize, usize))> {
        let mut periods: Vec<(usize, usize)> = (1..=MAX_PERIOD)
            .flat_map(|py| (1..=MAX_PERIOD).map(move |px| (px, py)))
            .filter(|&p| p != (1, 1))
            .collect();
        periods.sort_by_key(|&(px, py)| (px * py, px));
        let mut out = Vec::new();
        for _ in 0..MAX_RUNS {
            let mut best: Option<(u32, [usize; 4], (usize, usize))> = None;
            for &(px, py) in &periods {
                if px * MIN_TILES > self.w || py * MIN_TILES > self.h {
                    continue;
                }
                let Some(rect) = self.largest(px, py) else {
                    continue;
                };
                let rect = self.grown(rect, (px, py));
                let [x0, y0, x1, y1] = rect;
                let inked = self.count(x0, y0, x1, y1);
                // A tile with every cell inked is a solid block, which the
                // engine draws as one shape: not a dither.
                let area = ((x1 - x0) * (y1 - y0)) as u32;
                if inked < area && best.is_none_or(|b| inked > b.0) {
                    best = Some((inked, rect, (px, py)));
                }
            }
            let Some((inked, rect, period)) = best else {
                break;
            };
            if (inked as usize) < MIN_SQUARES || !self.repeats(rect, period) {
                break;
            }
            let [x0, y0, x1, y1] = rect;
            for y in y0..y1 {
                for x in x0..x1 {
                    self.taken[y * self.w + x] = true;
                }
            }
            out.push((rect, period));
        }
        out
    }

    /// The largest rectangle, at least `MIN_TILES` tiles each way, over
    /// which every cell matches the cells a period to its right and below
    /// and every tile-sized window holds a square: such a rectangle repeats
    /// the tile at its corner. The largest rectangle under a histogram, row
    /// by row.
    fn largest(&self, px: usize, py: usize) -> Option<[usize; 4]> {
        let (w, h) = (self.w, self.h);
        let mut heights = vec![0usize; w];
        let mut best: Option<(usize, [usize; 4])> = None;
        let mut stack: Vec<usize> = Vec::with_capacity(w);
        for y in 0..h {
            for (x, height) in heights.iter_mut().enumerate() {
                let here = self.ink(x, y);
                let fits = !self.taken[y * w + x]
                    && here == self.ink(x + px, y)
                    && here == self.ink(x, y + py)
                    && self.count(x, y, x + px, y + py) > 0;
                *height = if fits { *height + 1 } else { 0 };
            }
            stack.clear();
            for x in 0..=w {
                let bar = if x < w { heights[x] } else { 0 };
                while let Some(&top) = stack.last() {
                    if heights[top] < bar {
                        break;
                    }
                    stack.pop();
                    let height = heights[top];
                    let left = stack.last().map_or(0, |&l| l + 1);
                    let width = x - left;
                    if width >= px * MIN_TILES && height >= py * MIN_TILES {
                        let area = width * height;
                        if best.is_none_or(|b| area > b.0) {
                            best = Some((area, [left, y + 1 - height, x, y + 1]));
                        }
                    }
                }
                if x < w {
                    stack.push(x);
                }
            }
        }
        best.map(|b| b.1)
    }

    /// `rect` grown a column or a row at a time on any side while the new
    /// cells match the cells a period inside and are not taken: the test in
    /// `largest` looks a period ahead, so it stops a period short of where
    /// the dither ends.
    fn grown(&self, rect: [usize; 4], (px, py): (usize, usize)) -> [usize; 4] {
        let [mut x0, mut y0, mut x1, mut y1] = rect;
        let free = |x: usize, y: usize| !self.taken[y * self.w + x];
        loop {
            let mut moved = false;
            if x1 < self.w
                && (y0..y1).all(|y| free(x1, y) && self.ink(x1, y) == self.ink(x1 - px, y))
            {
                x1 += 1;
                moved = true;
            }
            if y1 < self.h
                && (x0..x1).all(|x| free(x, y1) && self.ink(x, y1) == self.ink(x, y1 - py))
            {
                y1 += 1;
                moved = true;
            }
            if x0 > 0
                && (y0..y1)
                    .all(|y| free(x0 - 1, y) && self.ink(x0 - 1, y) == self.ink(x0 - 1 + px, y))
            {
                x0 -= 1;
                moved = true;
            }
            if y0 > 0
                && (x0..x1)
                    .all(|x| free(x, y0 - 1) && self.ink(x, y0 - 1) == self.ink(x, y0 - 1 + py))
            {
                y0 -= 1;
                moved = true;
            }
            if !moved {
                return [x0, y0, x1, y1];
            }
        }
    }

    /// Whether every cell of `rect` is the tile's cell at its corner.
    fn repeats(&self, rect: [usize; 4], (px, py): (usize, usize)) -> bool {
        let [x0, y0, x1, y1] = rect;
        (y0..y1).all(|y| {
            (x0..x1).all(|x| self.ink(x, y) == self.ink(x0 + (x - x0) % px, y0 + (y - y0) % py))
        })
    }
}

/// `svg` (a document the app wrote) with each shape's runs drawn as pattern
/// fills. A `path` is looked at when it is filled with one colour, not
/// stroked and not transformed, and every subpath of its data starts with an
/// absolute `M`; everything else is copied as it is, and a document without
/// runs comes back unchanged.
pub fn patterned_svg(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut defs = String::new();
    let mut rest = svg;
    while let Some(at) = rest.find("<path") {
        let after = &rest[at..];
        let Some(end) = after.find("/>") else {
            break;
        };
        let element = &after[..end + 2];
        out.push_str(&rest[..at]);
        let whole = !element[..end].contains('>')
            && element[5..].starts_with(|c: char| c.is_ascii_whitespace());
        match whole.then(|| patterned_path(element, &mut defs)).flatten() {
            Some(text) => out.push_str(&text),
            None => out.push_str(element),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    if defs.is_empty() {
        return out;
    }
    let Some(close) = out
        .find("<svg")
        .and_then(|at| out[at..].find('>').map(|end| at + end + 1))
    else {
        return svg.to_owned();
    };
    out.insert_str(close, &format!("\n<defs>\n{defs}</defs>"));
    out
}

/// `element` (one self-closing `path`) with its runs taken out and drawn
/// after it as rectangles filled with patterns, whose definitions go to
/// `defs`; `None` when it has none.
fn patterned_path(element: &str, defs: &mut String) -> Option<String> {
    let fill = attribute(element, "fill")?;
    let d = attribute(element, "d")?;
    if !fill.starts_with('#')
        || attribute(element, "stroke").is_some_and(|s| s.trim() != "none")
        || attribute(element, "transform").is_some()
        || d.contains('m')
    {
        return None;
    }
    // The data's subpaths as written: the text before the first `M`, then
    // each `M` to the next.
    let starts: Vec<usize> = d.match_indices('M').map(|(i, _)| i).collect();
    let first = *starts.first()?;
    let chunks: Vec<&str> = starts
        .iter()
        .enumerate()
        .map(|(k, &i)| &d[i..starts.get(k + 1).copied().unwrap_or(d.len())])
        .collect();
    let squares: Vec<(usize, Square)> = chunks
        .iter()
        .enumerate()
        .filter_map(|(k, chunk)| {
            let segments = crate::pdf_eps::path_segments(chunk).ok()?;
            Some((k, square(&segments)?))
        })
        .collect();
    let (found, covered) = runs(&squares.iter().map(|s| s.1).collect::<Vec<_>>());
    if found.is_empty() {
        return None;
    }
    let mut dropped = vec![false; chunks.len()];
    for (k, (i, _)) in squares.iter().enumerate() {
        dropped[*i] = covered[k];
    }
    let kept: String = chunks
        .iter()
        .zip(&dropped)
        .filter(|(_, d)| !**d)
        .map(|(chunk, _)| *chunk)
        .collect();
    let mut out = String::new();
    if !kept.trim().is_empty() {
        out.push_str(&with_attribute(
            element,
            "d",
            &format!("{}{kept}", &d[..first]),
        ));
    }
    let bare = without_attribute(element, "id");
    for run in &found {
        let id = format!("vm-dither-{}", defs.matches("<pattern ").count());
        let (tile_w, tile_h) = run.tile();
        let s = num(run.side);
        let mut cells = String::new();
        for &(i, j) in &run.inked {
            let _ = write!(
                cells,
                "M{} {}h{s}v{s}h-{s}z",
                num(i as f64 * run.side),
                num(j as f64 * run.side)
            );
        }
        let _ = writeln!(
            defs,
            "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" \
             height=\"{}\"><path fill=\"{fill}\" d=\"{cells}\"/></pattern>",
            num(run.x),
            num(run.y),
            num(tile_w),
            num(tile_h)
        );
        let rect = format!(
            " M {} {} H {} V {} H {} Z",
            num(run.x),
            num(run.y),
            num(run.x + run.width),
            num(run.y + run.height),
            num(run.x)
        );
        let painted = with_attribute(&bare, "fill", &format!("url(#{id})"));
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&with_attribute(&painted, "d", &rect));
    }
    Some(out)
}

/// A number as SVG writes it: the shortest text that reads back the same.
fn num(value: f64) -> String {
    if value == 0. {
        "0".to_owned()
    } else {
        format!("{value}")
    }
}

/// The span of attribute `name`'s value in `element`.
fn value_span(element: &str, name: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(at) = element[from..].find(name).map(|i| i + from) {
        let before = element[..at].chars().next_back();
        let after = element[at + name.len()..].trim_start();
        if before.is_some_and(|c| c.is_ascii_whitespace()) && after.starts_with('=') {
            let value = after[1..].trim_start();
            let quote = value.chars().next().filter(|q| *q == '"' || *q == '\'')?;
            let start = element.len() - value.len() + 1;
            let end = start + element[start..].find(quote)?;
            return Some((start, end));
        }
        from = at + name.len();
    }
    None
}

fn attribute<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    value_span(element, name).map(|(start, end)| &element[start..end])
}

fn with_attribute(element: &str, name: &str, value: &str) -> String {
    match value_span(element, name) {
        Some((start, end)) => format!("{}{value}{}", &element[..start], &element[end..]),
        None => element.to_owned(),
    }
}

fn without_attribute(element: &str, name: &str) -> String {
    match value_span(element, name) {
        Some((start, end)) => {
            let name_at = element[..start].rfind(name).unwrap_or(start);
            let cut = element[..name_at].trim_end().len();
            format!("{}{}", &element[..cut], &element[end + 1..])
        }
        None => element.to_owned(),
    }
}

#[cfg(test)]
mod tests;
