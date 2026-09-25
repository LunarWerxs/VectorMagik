//! Pixel-edged artwork of exact colours, traced from its pixels.
//!
//! A picture drawn without anti-aliasing (a screenshot's text, an aliased
//! logo, an icon) is exact colours on the pixel grid. The engine smooths
//! its outlines like any picture, and at a 10 px cap that breaks letters:
//! on the shape set's pixel-edged text the & lost its loop, the 8 its
//! counters, "tt" came out as "(t" (the smallest overlap with the true
//! letters 0.57, colour error 4.49; September 25, 2026). The pixels settle
//! such a picture: each area of one colour is where it is, to the pixel.
//!
//! This traces them directly. Areas are 4-connected on a grid of three
//! cells per pixel, where a diagonal contact of one colour across two
//! pixels of another (a checkerboard corner) bridges the scarcer of the two
//! colours with the two cells beside the corner, so a 1 px diagonal stroke
//! stays whole and its background parts. The areas' outlines are walked
//! along the cell edges and cut into chains where three areas meet (the
//! picture's frame counting as one); each chain is smoothed once and used,
//! reversed, by the area on its other side too, so neighbours share every
//! edge as the engine's own documents do. A chain on the frame stays on it.
//! Smoothing a chain: a pixel corner with a side or a cap on both hands is
//! kept (`square_corners`) and slid to where the straight runs beside it
//! meet, every other step is replaced by the middles of its pixel edges,
//! and the junctions slide to where the sides leaving them meet
//! (`junction_moves`); then the polygon with the fewest segments through
//! the middles, each within `RIPPLE` of its fitted line (`polygon`); each
//! vertex moved to where the lines fitted either side of it cross, two
//! short bends that turn a corner together merged into it; then a quadratic
//! B-spline on it, a line at each end and at corners, all round a closed
//! chain without corners.
//! Written in the engine's layout: one path per colour (each area's outer
//! outline, positive shoelace area, then its holes), in order of first
//! appearance. Fully transparent areas are left out.
use std::collections::HashMap;

/// Grid cells per pixel.
const CELLS: i64 = 3;
/// Pixels: how far a middle may lie from the line of its polygon segment,
/// and a vertex move, the rasterizer's half a pixel (0.4 and 0.6 px both
/// scored worse on the pixel-edged shapes, September 25, 2026).
const RIPPLE: f64 = 0.5;
/// Degrees: the smoothed outline turning this much or more at a point is a
/// corner.
const CORNER_DEGREES: f64 = 55.;
/// Pixels: a run beside a pixel corner this long or longer is a side, not
/// a step of a staircase.
const RUN: f64 = 2.;
/// Pixels: a bend at most this far from the next, and degrees: the two
/// turning this much together, make a chamfer, merged into one corner.
const CHAMFER: f64 = 1.;
const CHAMFER_DEGREES: f64 = 70.;
/// Pixels: how far a junction or a kept corner may slide off its pixel
/// corner to where the sides fitted beside it meet (a 1 px line's cap
/// corner needs half a pixel, the junction at its end about as much).
const SLIDE: f64 = 0.75;
/// The pull of a junction to its pixel corner against the fitted sides: small,
/// so a junction with one side projects onto it and one between parallel
/// sides lands halfway.
const PULL: f64 = 1e-3;

type Vertex = (i64, i64);
type Point = (f64, f64);

/// A drawn piece: a line or a quadratic (start, control, end).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Piece {
    Line(Point, Point),
    Quad(Point, Point, Point),
}

impl Piece {
    fn reversed(self) -> Self {
        match self {
            Piece::Line(a, b) => Piece::Line(b, a),
            Piece::Quad(a, c, b) => Piece::Quad(b, c, a),
        }
    }
}

/// A chain between junctions (or a closed one), in its canonical direction:
/// on the frame, its pixel edges; else its steps, fitted and drawn once its
/// junctions have moved.
enum Chain {
    Framed(Vec<Point>),
    Smooth(Steps),
}

/// A chain's staircase read: the middles of its pixel edges and its kept
/// corners (`keep`, its two ends among them), whether a closed chain starts
/// on a kept corner, whether its ends are a junction (`pinned`), and
/// whether it is a loop without one (`closed`).
struct Steps {
    pts: Vec<Point>,
    keep: Vec<bool>,
    starts_on_corner: bool,
    pinned: bool,
    closed: bool,
}

/// A chain's steps fitted: the polygon's vertices (`q`), which are kept
/// corners, the line fitted to each segment between two of them,
/// whether a closed chain starts on a kept corner, and whether it is a
/// closed chain without one (drawn all round, with no start).
struct Fitted {
    q: Vec<Point>,
    kept: Vec<bool>,
    lines: Vec<Option<(Point, Point)>>,
    starts_on_corner: bool,
    cyclic: bool,
}

/// The engine-layout SVG of `pixels` (`width` by `height`, RGBA), and the
/// number of areas drawn and of pieces written.
pub fn trace(pixels: &[[u8; 4]], width: usize, height: usize) -> (String, usize, usize) {
    let colour = |i: usize| {
        let p = pixels[i];
        if p[3] == 0 {
            [0; 4]
        } else {
            p
        }
    };
    // Colour ids and their areas in pixels.
    let mut ids: HashMap<[u8; 4], usize> = HashMap::new();
    let mut count: Vec<usize> = Vec::new();
    let pixel_id: Vec<usize> = (0..width * height)
        .map(|i| {
            let next = ids.len();
            let id = *ids.entry(colour(i)).or_insert(next);
            if id == count.len() {
                count.push(0);
            }
            count[id] += 1;
            id
        })
        .collect();
    let mut palette = vec![[0u8; 4]; ids.len()];
    for (c, &id) in &ids {
        palette[id] = *c;
    }
    // The grid, with the diagonal bridges.
    let (gw, gh) = (width * CELLS as usize, height * CELLS as usize);
    let mut grid = vec![0usize; gw * gh];
    for y in 0..gh {
        for x in 0..gw {
            grid[y * gw + x] = pixel_id[(y / CELLS as usize) * width + x / CELLS as usize];
        }
    }
    for y in 0..height.saturating_sub(1) {
        for x in 0..width.saturating_sub(1) {
            let [a, b, c, d] = [
                pixel_id[y * width + x],
                pixel_id[y * width + x + 1],
                pixel_id[(y + 1) * width + x],
                pixel_id[(y + 1) * width + x + 1],
            ];
            if a == d && b == c && a != b {
                let (cx, cy) = ((x + 1) * CELLS as usize, (y + 1) * CELLS as usize);
                if count[a] <= count[b] {
                    // Top-left to bottom-right across the corner.
                    grid[(cy - 1) * gw + cx] = a;
                    grid[cy * gw + cx - 1] = a;
                } else {
                    grid[(cy - 1) * gw + cx - 1] = b;
                    grid[cy * gw + cx] = b;
                }
            }
        }
    }
    // 4-connected areas of the grid, numbered in scan order.
    let mut label = vec![usize::MAX; gw * gh];
    let mut areas: Vec<[u8; 4]> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..gw * gh {
        if label[start] != usize::MAX {
            continue;
        }
        let own = grid[start];
        let id = areas.len();
        areas.push(palette[own]);
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % gw, i / gw);
            let mut visit = |j: usize| {
                if label[j] == usize::MAX && grid[j] == own {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < gw {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - gw);
            }
            if y + 1 < gh {
                visit(i + gw);
            }
        }
    }
    let at = |x: i64, y: i64| -> Option<usize> {
        (x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh)
            .then(|| label[y as usize * gw + x as usize])
    };
    // A junction: three or more of the four cells round a vertex differ (the
    // outside of the frame counting as one).
    let junction = |v: Vertex| {
        let mut cells: Vec<Option<usize>> = vec![
            at(v.0 - 1, v.1 - 1),
            at(v.0, v.1 - 1),
            at(v.0 - 1, v.1),
            at(v.0, v.1),
        ];
        cells.sort_unstable();
        cells.dedup();
        cells.len() >= 3
    };
    let on_frame = |v: Vertex| v.0 == 0 || v.1 == 0 || v.0 == gw as i64 || v.1 == gh as i64;
    // Every unit edge between two areas (or an area and the frame), directed
    // with its area on the walk's positive side, as `pixel_art` walks them.
    let mut edges: Vec<Vec<(Vertex, Vertex)>> = vec![Vec::new(); areas.len()];
    for y in 0..gh as i64 {
        for x in 0..gw as i64 {
            let Some(own) = at(x, y) else {
                continue;
            };
            let sides = [
                ((x, y - 1), (x, y), (x + 1, y)),
                ((x + 1, y), (x + 1, y), (x + 1, y + 1)),
                ((x, y + 1), (x + 1, y + 1), (x, y + 1)),
                ((x - 1, y), (x, y + 1), (x, y)),
            ];
            for ((nx, ny), a, b) in sides {
                if at(nx, ny) != Some(own) {
                    edges[own].push((a, b));
                }
            }
        }
    }
    // Every area's loops, the outer outline first (the largest positive
    // area), then its holes.
    let shoelace = |l: &Vec<Vertex>| -> i64 {
        (0..l.len())
            .map(|i| {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum()
    };
    let mut outlines: Vec<(usize, Vec<Vec<Vertex>>)> = Vec::new();
    for (id, list) in edges.iter().enumerate() {
        if areas[id][3] == 0 || list.is_empty() {
            continue;
        }
        let mut loops = walk_loops(list);
        if loops.is_empty() {
            continue;
        }
        loops.sort_by_key(|l| std::cmp::Reverse(shoelace(l)));
        outlines.push((id, loops));
    }
    // Every chain fitted once, in its canonical direction; the junctions
    // moved to where the sides fitted beside them meet; then each chain
    // drawn once between them.
    let mut fits: HashMap<Vec<Vertex>, Chain> = HashMap::new();
    for l in outlines.iter().flat_map(|(_, loops)| loops) {
        for chain in loop_chains(l, &junction) {
            fits.entry(canonical(&chain).0).or_insert_with_key(|key| {
                let points: Vec<Point> = key
                    .iter()
                    .map(|v| (v.0 as f64 / CELLS as f64, v.1 as f64 / CELLS as f64))
                    .collect();
                if key.iter().all(|&v| on_frame(v)) {
                    Chain::Framed(points)
                } else {
                    Chain::Smooth(steps(&points, junction(key[0])))
                }
            });
        }
    }
    let moves = junction_moves(&fits, gw as i64, gh as i64);
    let mut chains: HashMap<Vec<Vertex>, Vec<Piece>> = HashMap::new();
    let mut groups: Vec<([u8; 4], Vec<String>)> = Vec::new();
    let (mut drawn_areas, mut pieces_written) = (0, 0);
    for (id, loops) in &outlines {
        let fill = areas[*id];
        let mut d = String::new();
        for l in loops {
            let mut drawn_loop = Vec::new();
            for chain in loop_chains(l, &junction) {
                let (key, forward) = canonical(&chain);
                let pieces = chains.entry(key).or_insert_with_key(|key| {
                    // A closed chain has no junction to move to.
                    let last = key.len() - 1;
                    let ends = [moves.get(&key[0]).copied(), moves.get(&key[last]).copied()];
                    match &fits[key] {
                        Chain::Framed(points) => framed(points, ends),
                        Chain::Smooth(s) => drawn(&fitted(s, ends)),
                    }
                });
                if forward {
                    drawn_loop.extend(pieces.iter().copied());
                } else {
                    drawn_loop.extend(pieces.iter().rev().map(|p| p.reversed()));
                }
            }
            pieces_written += drawn_loop.len();
            write_loop(&mut d, &drawn_loop);
        }
        d.push_str(" Z");
        drawn_areas += 1;
        match groups.iter_mut().find(|(c, _)| *c == fill) {
            Some((_, paths)) => paths.push(d),
            None => groups.push((fill, vec![d])),
        }
    }
    // One path per colour, as the engine writes its documents (a path per
    // area wrote the dotted lines' 56 dots as 56 paths).
    let groups: Vec<([u8; 4], Vec<String>)> = groups
        .into_iter()
        .map(|(fill, ds)| {
            let path = |d: &str| {
                format!(
                    "<path fill=\"#{:02x}{:02x}{:02x}\" opacity=\"{}\" d=\"{d}\" />",
                    fill[0],
                    fill[1],
                    fill[2],
                    if fill[3] > 0xfa {
                        "1.00".to_owned()
                    } else {
                        format!("{:.2}", fill[3] as f64 / 255.)
                    }
                )
            };
            (fill, vec![path(&ds.concat())])
        })
        .collect();
    let mut svg = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\r\n");
    svg.push_str(&format!(
        "<svg width=\"{width}pt\" height=\"{height}pt\" viewBox=\"0 0 {width} {height}\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">"
    ));
    for (c, paths) in &groups {
        svg.push_str(&format!(
            "\r\n<g id=\"#{:02x}{:02x}{:02x}{:02x}\">",
            c[0], c[1], c[2], c[3]
        ));
        for p in paths {
            svg.push_str("\r\n");
            svg.push_str(p);
        }
        svg.push_str("\r\n</g>");
    }
    svg.push_str("\r\n</svg>\r\n");
    (svg, drawn_areas, pieces_written)
}

/// An area's loops from its directed unit edges: at a vertex where it
/// touches itself across a corner, the walk turns right first, then goes
/// straight, then left, keeping each loop tight.
fn walk_loops(list: &[(Vertex, Vertex)]) -> Vec<Vec<Vertex>> {
    let mut leaving: HashMap<Vertex, Vec<usize>> = HashMap::new();
    for (k, &(a, _)) in list.iter().enumerate() {
        leaving.entry(a).or_default().push(k);
    }
    let mut used = vec![false; list.len()];
    let mut loops = Vec::new();
    for first in 0..list.len() {
        if used[first] {
            continue;
        }
        let mut walk = vec![list[first].0];
        let mut k = first;
        loop {
            used[k] = true;
            let (a, b) = list[k];
            if b == list[first].0 {
                break;
            }
            walk.push(b);
            let dir = (b.0 - a.0, b.1 - a.1);
            let pick = leaving[&b]
                .iter()
                .copied()
                .filter(|&c| !used[c])
                .min_by_key(|&c| {
                    let (_, e) = list[c];
                    let d = (e.0 - b.0, e.1 - b.1);
                    let cross = dir.0 * d.1 - dir.1 * d.0;
                    let along = dir.0 * d.0 + dir.1 * d.1;
                    match (cross.signum(), along.signum()) {
                        (1, _) => 0,
                        (0, 1) => 1,
                        _ => 2,
                    }
                });
            let Some(next) = pick else {
                break;
            };
            k = next;
        }
        if walk.len() >= 4 {
            loops.push(walk);
        }
    }
    loops
}

/// A loop cut into chains at its junctions, each from one junction to the
/// next; a loop with none is one closed chain from its least vertex, so the
/// areas on both sides of it cut it alike.
fn loop_chains(l: &[Vertex], junction: &impl Fn(Vertex) -> bool) -> Vec<Vec<Vertex>> {
    let n = l.len();
    let cuts: Vec<usize> = (0..n).filter(|&i| junction(l[i])).collect();
    if cuts.is_empty() {
        let start = (0..n).min_by_key(|&i| l[i]).unwrap_or(0);
        return vec![(0..=n).map(|k| l[(start + k) % n]).collect()];
    }
    (0..cuts.len())
        .map(|j| {
            let (c, d) = (cuts[j], cuts[(j + 1) % cuts.len()]);
            let span = match (d + n - c) % n {
                0 => n,
                s => s,
            };
            (0..=span).map(|k| l[(c + k) % n]).collect()
        })
        .collect()
}

/// A chain's key, the direction it is fitted and drawn in: itself or its
/// reverse, whichever sorts first (a closed chain: the one whose second
/// vertex is the lesser); and whether that is itself.
fn canonical(chain: &[Vertex]) -> (Vec<Vertex>, bool) {
    let reversed: Vec<Vertex> = chain.iter().rev().copied().collect();
    if chain <= reversed.as_slice() {
        (chain.to_vec(), true)
    } else {
        (reversed, false)
    }
}

/// A chain on the frame as lines through its turns, its first and last
/// points replaced by `ends` (its junctions moved along the frame).
fn framed(points: &[Point], ends: [Option<Point>; 2]) -> Vec<Piece> {
    let last = points.len() - 1;
    let mut turns = vec![ends[0].unwrap_or(points[0])];
    for i in 1..last {
        if turn(points[i - 1], points[i], points[i + 1]).abs() > 1e-9 {
            turns.push(points[i]);
        }
    }
    turns.push(ends[1].unwrap_or(points[last]));
    turns.windows(2).map(|w| Piece::Line(w[0], w[1])).collect()
}

/// Where each junction moves: the point nearest, in least squares, to the
/// lines the chains that meet there leave it along (`end_line`), pulled by
/// `PULL` to its pixel corner. A
/// junction on the frame moves only along it, a frame corner not at all, and
/// a move longer than `SLIDE` is not made. Without this, a thin line's side
/// ran as one chord between the pixel corners at its ends, and a 1 px line
/// crossing another was drawn half a pixel thick (September 25, 2026).
fn junction_moves(fits: &HashMap<Vec<Vertex>, Chain>, gw: i64, gh: i64) -> HashMap<Vertex, Point> {
    // Per junction, the normal equations' sums: nn^T (xx, xy, yy) and
    // n n.c (x, y) over the lines' unit normals n and points c. The keys
    // sorted, so the sums add in one order.
    let mut sums: HashMap<Vertex, [f64; 5]> = HashMap::new();
    let mut keys: Vec<&Vec<Vertex>> = fits.keys().collect();
    keys.sort_unstable();
    for key in keys {
        let Chain::Smooth(s) = &fits[key] else {
            continue;
        };
        if !s.pinned {
            continue;
        }
        for (v, last) in [(key[0], false), (key[key.len() - 1], true)] {
            let Some((c, d)) = end_line(s, last) else {
                continue;
            };
            let n = (-d.1, d.0);
            let nc = n.0 * c.0 + n.1 * c.1;
            let s = sums.entry(v).or_insert([0.; 5]);
            s[0] += n.0 * n.0;
            s[1] += n.0 * n.1;
            s[2] += n.1 * n.1;
            s[3] += n.0 * nc;
            s[4] += n.1 * nc;
        }
    }
    let mut moves = HashMap::new();
    for (v, [xx, xy, yy, bx, by]) in sums {
        let corner = (v.0 as f64 / CELLS as f64, v.1 as f64 / CELLS as f64);
        let (a, b, c) = (xx + PULL, xy, yy + PULL);
        let (ex, ey) = (bx + PULL * corner.0, by + PULL * corner.1);
        let p = match (v.0 == 0 || v.0 == gw, v.1 == 0 || v.1 == gh) {
            (true, true) => continue,
            (true, false) => (corner.0, (ey - b * corner.0) / c),
            (false, true) => ((ex - b * corner.1) / a, corner.1),
            (false, false) => {
                let det = a * c - b * b;
                ((ex * c - b * ey) / det, (a * ey - b * ex) / det)
            }
        };
        if (p.0 - corner.0).hypot(p.1 - corner.1) <= SLIDE {
            moves.insert(v, p);
        }
    }
    moves
}

fn turn(a: Point, b: Point, c: Point) -> f64 {
    let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
    (u.0 * v.1 - u.1 * v.0).atan2(u.0 * v.0 + u.1 * v.1)
}

fn unit(d: Point) -> Point {
    let l = d.0.hypot(d.1).max(1e-12);
    (d.0 / l, d.1 / l)
}

fn mid(a: Point, b: Point) -> Point {
    ((a.0 + b.0) / 2., (a.1 + b.1) / 2.)
}

/// Which pixel corners of `core` (a closed chain's first repeated at its
/// end) are corners of the drawing: both runs beside one at least `RUN` px
/// long or a cap, turning the same way at both its ends. A staircase has a
/// single step beside every corner, turning back the way it came (a digital
/// line or curve steps one pixel at a time in one of its two directions);
/// a square's corners, a dash, a lone pixel, a stem's end or a plus's arm
/// have none, and smoothing them made a lone pixel a diamond (half its
/// overlap) and rounded the pluses (September 25, 2026).
fn square_corners(core: &[Point], closed: bool) -> Vec<bool> {
    let n = core.len();
    let m = if closed { n - 1 } else { n };
    let step = |i: usize, d: isize| -> Option<usize> {
        let j = i as isize + d;
        if closed {
            Some(j.rem_euclid(m as isize) as usize)
        } else {
            (0..m as isize).contains(&j).then_some(j as usize)
        }
    };
    let sense: Vec<i32> = (0..m)
        .map(|i| match (step(i, -1), step(i, 1)) {
            (Some(a), Some(c)) => {
                let t = turn(core[a], core[i], core[c]);
                i32::from(t > 1e-9) - i32::from(t < -1e-9)
            }
            _ => 0,
        })
        .collect();
    // The run from `i` to the next corner: long, and a side (long or a cap).
    let long = |i: usize| {
        step(i, 1)
            .is_some_and(|j| (core[j].0 - core[i].0).hypot(core[j].1 - core[i].1) >= RUN - 1e-9)
    };
    let side =
        |i: usize| long(i) || step(i, 1).is_some_and(|j| sense[i] != 0 && sense[i] == sense[j]);
    // A lone rectangle (a pixel, a dash) keeps all four; else one run beside
    // the corner is long (a 1 px diagonal's end, two caps, is a staircase's).
    let rectangle = closed && m == 4;
    let mut sharp: Vec<bool> = (0..m)
        .map(|i| {
            let Some(p) = step(i, -1) else {
                return false;
            };
            sense[i] != 0 && (rectangle || (long(p) && side(i)) || (side(p) && long(i)))
        })
        .collect();
    if closed {
        sharp.push(sharp[0]);
    }
    sharp
}

/// A chain's staircase read: its kept corners and the middles of its pixel
/// edges. A `pinned` chain runs from a junction to a junction, the same one
/// when it is a loop's only junction: such a loop is closed for its corners
/// but starts where it is pinned.
fn steps(chain: &[Point], pinned: bool) -> Steps {
    // The staircase's corners.
    let mut core = vec![chain[0]];
    for i in 1..chain.len() - 1 {
        if turn(chain[i - 1], chain[i], chain[i + 1]).abs() > 1e-9 {
            core.push(chain[i]);
        }
    }
    core.push(chain[chain.len() - 1]);
    let closed = core[0] == core[core.len() - 1] && core.len() > 2;
    let mut fixed = square_corners(&core, closed);
    let mut starts_on_corner = false;
    if closed && !pinned {
        let n = core.len() - 1;
        match (0..n).find(|&i| fixed[i]) {
            // A closed chain with a kept corner starts there.
            Some(s) => {
                core = (0..=n).map(|k| core[(s + k) % n]).collect();
                fixed = (0..=n).map(|k| fixed[(s + k) % n]).collect();
                starts_on_corner = true;
            }
            // Else at the middle of an edge: a start pinned on a staircase
            // corner biases the outline by up to half a pixel.
            None => {
                let m = mid(core[0], core[1]);
                let mut rotated = vec![m];
                rotated.extend_from_slice(&core[1..]);
                rotated.push(m);
                core = rotated;
                fixed.push(false);
            }
        }
    }
    fixed[0] = true;
    let last = core.len() - 1;
    fixed[last] = true;
    // The middles of the steps, the kept corners among them.
    let mut pts: Vec<Point> = Vec::new();
    let mut keep: Vec<bool> = Vec::new();
    let push = |p: Point, k: bool, pts: &mut Vec<Point>, keep: &mut Vec<bool>| {
        if pts.last() != Some(&p) {
            pts.push(p);
            keep.push(k);
        }
    };
    for i in 0..core.len() {
        if fixed[i] {
            push(core[i], true, &mut pts, &mut keep);
        }
        if i + 1 < core.len() {
            // The middle of every whole pixel edge along the step, so a long
            // run weighs as much as its pixels; a bridge's third-of-a-pixel
            // edges only join (their middles would pull a thin stroke's
            // sides into its necks).
            let (a, b) = (core[i], core[i + 1]);
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let units = (length + 1e-9).floor() as usize;
            if units == 0 && length > 0.5 {
                // A pixel edge a bridge took a third of: a 1 px diagonal's
                // sides are only these.
                push(mid(a, b), false, &mut pts, &mut keep);
            }
            // Centred on the run: a run a bridge shortened at its ends has
            // its middles where the pixels' are.
            let lead = (length - units as f64) / 2.;
            for u in 0..units {
                let f = (lead + u as f64 + 0.5) / length;
                push(
                    (a.0 + f * (b.0 - a.0), a.1 + f * (b.1 - a.1)),
                    false,
                    &mut pts,
                    &mut keep,
                );
            }
        }
    }
    // Each kept corner slid to where the straight runs beside it meet (on a
    // hand with fewer than three middles, the edge itself), before the
    // ripple: pinned on its pixel corner, a 1 px line's cap corner held the
    // middle of the last run of the side before it, and the side bent into
    // the cap.
    let n = pts.len();
    let cyclic = starts_on_corner.then_some(n - 1);
    let slid: Vec<(usize, Point)> = (0..n)
        .filter(|&k| keep[k] && ((k > 0 && k + 1 < n) || (k == 0 && cyclic.is_some())))
        .filter_map(|k| {
            let hand = |forward: bool| {
                run_line(&pts, &keep, k, forward, cyclic, true).or_else(|| {
                    let j = beside(k, forward, n, cyclic)?;
                    Some((pts[k], unit((pts[j].0 - pts[k].0, pts[j].1 - pts[k].1))))
                })
            };
            let p = crossing(hand(false)?, hand(true)?)?;
            ((p.0 - pts[k].0).hypot(p.1 - pts[k].1) <= SLIDE).then_some((k, p))
        })
        .collect();
    for (k, p) in slid {
        pts[k] = p;
        if k == 0 {
            pts[n - 1] = p;
        }
    }
    Steps {
        pts,
        keep,
        starts_on_corner,
        pinned,
        closed: closed && !pinned,
    }
}

/// The index beside `k` of `n` points, forward or back; a closed chain of
/// `cyclic` distinct points (its first repeated at its end) wraps round.
fn beside(k: usize, forward: bool, n: usize, cyclic: Option<usize>) -> Option<usize> {
    match (cyclic, forward) {
        (Some(m), true) => Some((k + 1) % m),
        (Some(m), false) => Some((k + m - 1) % m),
        (None, true) => (k + 1 < n).then_some(k + 1),
        (None, false) => k.checked_sub(1),
    }
}

/// The line a chain leaves its point `from` along, forward or back: fitted
/// to the middles met up to the first kept point, as far as every one stays
/// within `RIPPLE` of it; None with fewer than three.
fn run_line(
    pts: &[Point],
    keep: &[bool],
    from: usize,
    forward: bool,
    cyclic: Option<usize>,
    corner: bool,
) -> Option<(Point, Point)> {
    // The run also ends where the outline turns a corner, the step off a
    // kept `corner` counting (a cap's middle lies on the far side's line),
    // the step off a junction, whose pixel corner lies off every side, not.
    let corner_angle = CORNER_DEGREES.to_radians();
    let mut walk: Vec<Point> = if corner { vec![pts[from]] } else { Vec::new() };
    let mut set = Vec::new();
    let mut line = None;
    let mut k = from;
    loop {
        k = match beside(k, forward, pts.len(), cyclic) {
            Some(j) if j != from && !keep[j] => j,
            _ => break,
        };
        if let [.., a, b] = walk[..] {
            if turn(a, b, pts[k]).abs() >= corner_angle {
                break;
            }
        }
        walk.push(pts[k]);
        set.push(pts[k]);
        if set.len() < 3 {
            continue;
        }
        let Some((c, d)) = fitted_line(&set) else {
            break;
        };
        if set
            .iter()
            .any(|p| ((p.0 - c.0) * d.1 - (p.1 - c.1) * d.0).abs() > RIPPLE)
        {
            break;
        }
        line = Some((c, d));
    }
    line
}

/// The line a pinned chain leaves its junction along (its last end's when
/// `last`), `run_line` from it. The segment the ripple leaves beside a
/// junction pinned on its pixel corner can be a step or two, whose line
/// points anywhere.
fn end_line(s: &Steps, last: bool) -> Option<(Point, Point)> {
    let from = if last { s.pts.len() - 1 } else { 0 };
    run_line(&s.pts, &s.keep, from, !last, None, false)
}

/// A chain's steps fitted with its ends replaced by `ends` (its junctions,
/// moved) and fixed (the module's doc), up to the lines fitted between the
/// points left.
fn fitted(s: &Steps, ends: [Option<Point>; 2]) -> Fitted {
    let (mut pts, keep) = (s.pts.clone(), &s.keep);
    let last = pts.len() - 1;
    if let Some(p) = ends[0] {
        pts[0] = p;
    }
    if let Some(p) = ends[1] {
        pts[last] = p;
    }
    let cyclic = s.closed && !s.starts_on_corner;
    let v = polygon(&pts, keep);
    let q: Vec<Point> = v.iter().map(|&i| pts[i]).collect();
    let mut kept: Vec<bool> = v.iter().map(|&i| keep[i]).collect();
    if cyclic {
        // The start of a closed chain without corners is no corner.
        kept[0] = false;
        let last = kept.len() - 1;
        kept[last] = false;
    }
    // The line fitted to each segment's middles: a straight edge's mean runs
    // between the polygon's vertices (potrace places its vertices the same
    // way). A kept corner is left out of the fits when its side has points
    // of its own: drawn to the pixel corners at its ends, a 1 px line's side
    // ran half a pixel off the line at each end, the two sides opposite
    // ways, and the line tapered.
    let lines = (0..v.len() - 1)
        .map(|k| {
            let mut set: Vec<Point> = pts[v[k] + 1..v[k + 1]].to_vec();
            set.extend([k, k + 1].iter().filter(|&&e| !kept[e]).map(|&e| q[e]));
            if set.len() < 2 {
                set.extend([q[k], q[k + 1]]);
            }
            fitted_line(&set)
        })
        .collect();
    Fitted {
        q,
        kept,
        lines,
        starts_on_corner: s.starts_on_corner,
        cyclic,
    }
}

/// Pixels: a segment of the polygon passes this many middles at most (the
/// fit is checked point by point, so its cost grows with the square).
const MAX_SEGMENT: usize = 256;

/// The polygon through the middles with the fewest segments, each passing
/// every middle between its ends within `RIPPLE` of the line fitted to them,
/// and of those the one fitting them best; it breaks at every kept point.
/// Its indices into `pts`. The ripple's thinning had tested each chord
/// between two extremes, which a straight staircase's middles cross from
/// both sides by up to its whole tolerance, so a straight side kept a zigzag
/// and was drawn as a wave (9 to 15 inflections per 100 px on the
/// pixel-edged shapes, September 25, 2026).
fn polygon(pts: &[Point], keep: &[bool]) -> Vec<usize> {
    let n = pts.len();
    // Prefix sums of x, y, xx, xy, yy, for each range's fitted line and its
    // squared error in constant time.
    let mut sums = vec![[0f64; 5]; n + 1];
    for (i, p) in pts.iter().enumerate() {
        let s = sums[i];
        sums[i + 1] = [
            s[0] + p.0,
            s[1] + p.1,
            s[2] + p.0 * p.0,
            s[3] + p.0 * p.1,
            s[4] + p.1 * p.1,
        ];
    }
    // The line through pts[i..=j] (centroid, direction) and its squared error.
    let line = |i: usize, j: usize| -> ((Point, Point), f64) {
        let m = (j - i + 1) as f64;
        let s: Vec<f64> = (0..5).map(|k| sums[j + 1][k] - sums[i][k]).collect();
        let c = (s[0] / m, s[1] / m);
        let (xx, xy, yy) = (
            s[2] / m - c.0 * c.0,
            s[3] / m - c.0 * c.1,
            s[4] / m - c.1 * c.1,
        );
        let angle = 0.5 * (2. * xy).atan2(xx - yy);
        let spread = ((xx - yy) * (xx - yy) / 4. + xy * xy).sqrt();
        (
            (c, (angle.cos(), angle.sin())),
            (((xx + yy) / 2. - spread) * m).max(0.),
        )
    };
    let fits = |i: usize, j: usize| {
        let ((c, d), _) = line(i, j);
        pts[i..=j]
            .iter()
            .all(|p| ((p.0 - c.0) * d.1 - (p.1 - c.1) * d.0).abs() <= RIPPLE)
    };
    let mut out = vec![0];
    let mut a = 0;
    while a + 1 < n {
        let b = (a + 1..n).find(|&i| keep[i]).unwrap_or(n - 1);
        // The furthest end each start reaches, never less than the last
        // start's (a range inside a fitting one fits).
        let mut reach = vec![0; b - a];
        let mut j = a + 1;
        for i in a..b {
            j = j.max(i + 1);
            while j < b && j + 1 - i < MAX_SEGMENT && fits(i, j + 1) {
                j += 1;
            }
            reach[i - a] = j;
        }
        // Fewest segments, then least squared error, from a to each point.
        let mut best: Vec<(usize, f64, usize)> = vec![(usize::MAX, 0., a); b - a + 1];
        best[0] = (0, 0., a);
        for i in a..b {
            let (count, error, _) = best[i - a];
            if count == usize::MAX {
                continue;
            }
            for j in i + 1..=reach[i - a] {
                let e = error + line(i, j).1;
                let slot = &mut best[j - a];
                if count + 1 < slot.0 || (count + 1 == slot.0 && e < slot.1) {
                    *slot = (count + 1, e, i);
                }
            }
        }
        let mut path = Vec::new();
        let mut k = b;
        while k != a {
            path.push(k);
            k = best[k - a].2;
        }
        out.extend(path.into_iter().rev());
        a = b;
    }
    out
}

/// A fitted chain drawn: each vertex but a kept corner (slid already) moved
/// to where the lines fitted to its two segments cross, when that lies
/// within `RIPPLE` of it, the chamfers merged, then the spline.
fn drawn(fit: &Fitted) -> Vec<Piece> {
    let (lines, qk) = (&fit.lines, &fit.kept);
    let mut q = fit.q.clone();
    let last = q.len() - 1;
    let corner_angle = CORNER_DEGREES.to_radians();
    if fit.cyclic && last >= 3 {
        // All round: every point moved between its two lines, then the
        // spline from each edge's middle to the next's, so the outline has
        // no start to bend at.
        let m = last;
        let v: Vec<Point> = (0..m)
            .map(|k| match (lines[(k + m - 1) % m], lines[k]) {
                (Some(a), Some(b)) if !qk[k] => crossing(a, b)
                    .filter(|p| (p.0 - q[k].0).hypot(p.1 - q[k].1) <= RIPPLE)
                    .unwrap_or(q[k]),
                _ => q[k],
            })
            .collect();
        let mut out = Vec::new();
        for k in 0..m {
            let (a, c, b) = (v[(k + m - 1) % m], v[k], v[(k + 1) % m]);
            let (start, end) = (mid(a, c), mid(c, b));
            if qk[k] || turn(a, c, b).abs() >= corner_angle {
                out.push(Piece::Line(start, c));
                out.push(Piece::Line(c, end));
            } else {
                out.push(Piece::Quad(start, c, end));
            }
        }
        return out;
    }
    let moved: Vec<Point> = (0..q.len())
        .map(|k| {
            let ends = k == 0 || k == last;
            if ends && !(fit.starts_on_corner && q.len() > 3) {
                return q[k];
            }
            let (before, after) = if ends {
                (lines[q.len() - 2], lines[0])
            } else {
                (lines[k - 1], lines[k])
            };
            if qk[k] {
                // Slid already (`steps`).
                return q[k];
            }
            match (before, after) {
                (Some(a), Some(b)) => crossing(a, b)
                    .filter(|p| (p.0 - q[k].0).hypot(p.1 - q[k].1) <= RIPPLE)
                    .unwrap_or(q[k]),
                _ => q[k],
            }
        })
        .collect();
    q = moved;
    // A chamfer: two short bends the same way that together turn a corner
    // (a square corner's staircase gives two 45-degree bends half a pixel
    // apart) become one corner where the sides on either side cross.
    let mut corner = qk.clone();
    let mut k = 1;
    while k + 2 < q.len() {
        let (t1, t2) = (
            turn(q[k - 1], q[k], q[k + 1]),
            turn(q[k], q[k + 1], q[k + 2]),
        );
        let short = (q[k + 1].0 - q[k].0).hypot(q[k + 1].1 - q[k].1) <= CHAMFER;
        let free = !corner[k] && !corner[k + 1];
        if free && short && t1 * t2 > 0. && (t1 + t2).abs() >= CHAMFER_DEGREES.to_radians() {
            let (a, b) = (q[k - 1], q[k + 2]);
            let join = crossing(
                (a, unit((q[k].0 - a.0, q[k].1 - a.1))),
                (b, unit((q[k + 1].0 - b.0, q[k + 1].1 - b.1))),
            )
            .filter(|p| {
                (p.0 - q[k].0).hypot(p.1 - q[k].1) <= CHAMFER
                    && (p.0 - q[k + 1].0).hypot(p.1 - q[k + 1].1) <= CHAMFER
            });
            if let Some(p) = join {
                q[k] = p;
                corner[k] = true;
                q.remove(k + 1);
                corner.remove(k + 1);
                continue;
            }
        }
        k += 1;
    }
    let qk = corner;
    if q.len() == 2 {
        return vec![Piece::Line(q[0], q[1])];
    }
    // The quadratic B-spline on it: from each edge's middle to the next's
    // with the point between as control; lines at the ends and at corners.
    let mut out = Vec::new();
    let mut at = q[0];
    for k in 1..q.len() - 1 {
        let (a, c, b) = (q[k - 1], q[k], q[k + 1]);
        let mut start = mid(a, c);
        let mut end = mid(c, b);
        if k == 1 {
            start = a;
        }
        if k == q.len() - 2 {
            end = b;
        }
        if at != start {
            out.push(Piece::Line(at, start));
        }
        if qk[k] || turn(a, c, b).abs() >= corner_angle {
            out.push(Piece::Line(start, c));
            out.push(Piece::Line(c, end));
        } else {
            out.push(Piece::Quad(start, c, end));
        }
        at = end;
    }
    if at != q[q.len() - 1] {
        out.push(Piece::Line(at, q[q.len() - 1]));
    }
    // Lines that run straight on are one line.
    let mut joined: Vec<Piece> = Vec::with_capacity(out.len());
    for p in out {
        if let (Some(Piece::Line(a, b)), Piece::Line(_, c)) = (joined.last().copied(), p) {
            if turn(a, b, c).abs() < 1e-9 {
                *joined.last_mut().unwrap() = Piece::Line(a, c);
                continue;
            }
        }
        joined.push(p);
    }
    joined
}

/// The least-squares line through `points` (their centroid and principal
/// direction); None when they are one point.
fn fitted_line(points: &[Point]) -> Option<(Point, Point)> {
    let n = points.len() as f64;
    let c = (
        points.iter().map(|p| p.0).sum::<f64>() / n,
        points.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let (mut xx, mut xy, mut yy) = (0., 0., 0.);
    for p in points {
        let (dx, dy) = (p.0 - c.0, p.1 - c.1);
        xx += dx * dx;
        xy += dx * dy;
        yy += dy * dy;
    }
    if xx + yy <= 1e-12 {
        return None;
    }
    let angle = 0.5 * (2. * xy).atan2(xx - yy);
    Some((c, (angle.cos(), angle.sin())))
}

/// Where two lines (a point and a direction each) cross; None when they run
/// within a degree of parallel.
fn crossing(a: (Point, Point), b: (Point, Point)) -> Option<Point> {
    let ((p, u), (q, v)) = (a, b);
    let det = u.0 * v.1 - u.1 * v.0;
    if det.abs() < 1f64.to_radians().sin() {
        return None;
    }
    let t = ((q.0 - p.0) * v.1 - (q.1 - p.1) * v.0) / det;
    Some((p.0 + t * u.0, p.1 + t * u.1))
}

/// A loop's pieces into path data: its start, then each piece, a quadratic
/// as its cubic.
fn write_loop(d: &mut String, pieces: &[Piece]) {
    let Some(first) = pieces.first() else {
        return;
    };
    let start = match *first {
        Piece::Line(a, _) | Piece::Quad(a, _, _) => a,
    };
    // A number to two decimals, without the zeros a whole or half pixel
    // leaves (the engine's documents write them so).
    let n = |v: f64| {
        let s = format!("{v:.2}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" {
            "0".to_owned()
        } else {
            s.to_owned()
        }
    };
    d.push_str(&format!(" M {} {}", n(start.0), n(start.1)));
    for piece in pieces {
        match *piece {
            Piece::Line(_, b) => d.push_str(&format!(" L {} {}", n(b.0), n(b.1))),
            Piece::Quad(a, c, b) => {
                let c1 = (a.0 + 2. / 3. * (c.0 - a.0), a.1 + 2. / 3. * (c.1 - a.1));
                let c2 = (b.0 + 2. / 3. * (c.0 - b.0), b.1 + 2. / 3. * (c.1 - b.1));
                d.push_str(&format!(
                    " C {} {} {} {} {} {}",
                    n(c1.0),
                    n(c1.1),
                    n(c2.0),
                    n(c2.1),
                    n(b.0),
                    n(b.1)
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;
