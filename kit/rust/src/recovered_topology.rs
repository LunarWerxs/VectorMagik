//! Contour construction, ported from the original engine's builder at
//! engine+0x17e8 (entry 0x0049EEB0 = `0x499eb0`). From a label image (one
//! region id per pixel, as segmentation leaves it) it makes the shared node
//! set and one closed contour per region:
//!
//! 1. `0x498a50` marks every vertex of the pixel grid that touches a label
//!    change and numbers the nodes: the left and right columns of vertices
//!    first, then the top and bottom rows, then the interior in scan order.
//! 2. Every node gets a record with its position; the four canvas corners
//!    are corners (state 3).
//! 3. For each region, `0x4999d0` walks its boundary from the first node
//!    corner of its first pixel, keeping the region on its right hand,
//!    collecting the node ids and the label across every edge, and the
//!    sorted distinct labels across all edges (its neighbours, -1 for the
//!    outside removed).
//! 4. Every node where fewer than two of the six pairs of the four pixels
//!    around it agree is a corner (a junction of three regions, or a
//!    boundary meeting the canvas edge).
//! 5. `0x4993e0` thins the nodes: border nodes that are not corners go,
//!    and (unless the builder's extra array is non-empty) interior nodes in
//!    the middle of straight runs and regular stair-steps go, found by the
//!    constructor's pattern table over windows of up to 31 unit steps. The
//!    survivors are renumbered and every contour rewritten with, per kept
//!    node, the label across the edge leaving it and the number of unit steps
//!    of the edge arriving at it.
//! 6. A contour's `enclosing` is the first neighbour that does not list it
//!    back, or -1.
//!
//! The record fields the port owns are exactly those the original writes
//! here; the rest of the 0x60 contour record and the 0x28 node record are
//! left for later stages (the metadata word the smoother uses as a corner
//! flag is uninitialised at this point in the original).

/// The builder's direction tables, as its constructor leaves them (read
/// from the constructed engine by the host's `--topology-fixtures`).
/// Corners of a pixel in the order the tracer's start is looked for.
const CORNER_DX: [i32; 4] = [0, 1, 1, 0];
const CORNER_DY: [i32; 4] = [0, 0, 1, 1];
/// The four pixels around a vertex: up-left, down-left, down-right, up-right.
const PIXEL_DX: [i32; 4] = [-1, -1, 0, 0];
const PIXEL_DY: [i32; 4] = [-1, 0, 0, -1];
/// The four unit steps: left, down, right, up. Step `k` runs between the
/// pixels `PIXEL[k]` and `PIXEL[k + 1]`, with the former on the right hand.
const STEP_DX: [i32; 4] = [-1, 0, 1, 0];
const STEP_DY: [i32; 4] = [0, 1, 0, -1];
/// Stair-step patterns `(length, mask, keep)`: a run of `length` unit steps
/// whose "moved in x" or "moved in y" bits equal `mask` keeps only the nodes
/// whose bit is set in `keep`, counted back from the newest.
const PATTERNS: [(i32, u32, u32); 6] = [
    (5, 0, 33),
    (9, 170, 585),
    (9, 146, 585),
    (13, 4369, 8481),
    (10, 132, 1057),
    (10, 330, 1057),
];
/// Contours shorter than this are not thinned.
const THIN_MINIMUM: usize = 20;
/// The longest run the thinning looks at.
const WINDOW: i32 = 31;

const REMOVED: u8 = 0x40;
const SEEN: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub x: f64,
    pub y: f64,
    /// 3 at a corner, 0 otherwise (record byte +0x1c).
    pub state: u8,
    /// Record byte +0x1d: 0x80 on the canvas border (and, after thinning,
    /// on nodes the run walk passed).
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    /// The label across the edge leaving this node (-1 outside the canvas).
    pub other: i32,
    /// Unit steps of the edge arriving at this node.
    pub steps: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contour {
    pub nodes: Vec<i32>,
    pub edges: Vec<Edge>,
    /// Sorted distinct labels across the contour's edges, without -1.
    pub neighbours: Vec<i32>,
    /// The first neighbour that does not list this contour back, or -1.
    pub enclosing: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Topology {
    pub nodes: Vec<Node>,
    pub contours: Vec<Contour>,
    /// Nodes left on the left, right, top and bottom canvas edges.
    pub border: [i32; 4],
}

/// A row-major grid: owned for the vertex numbering and the tracer's marks,
/// borrowed for the caller's label image, which the builder only reads.
struct Grid<C = Vec<i32>> {
    width: usize,
    height: usize,
    cells: C,
}
impl Grid {
    fn new(width: usize, height: usize, fill: i32) -> Self {
        Self {
            width,
            height,
            cells: vec![fill; width * height],
        }
    }
    fn set(&mut self, x: i32, y: i32, value: i32) {
        self.cells[y as usize * self.width + x as usize] = value;
    }
}
impl<C: std::ops::Deref<Target = [i32]>> Grid<C> {
    fn at(&self, x: i32, y: i32) -> i32 {
        self.cells[y as usize * self.width + x as usize]
    }
    /// The value, or -1 outside.
    fn label(&self, x: i32, y: i32) -> i32 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            -1
        } else {
            self.at(x, y)
        }
    }
}

/// The topology of `labels` (`width` by `height` region ids in `0..regions`,
/// every region four-connected). `thin` is false when the builder's extra
/// array is non-empty, which skips the straight-run thinning.
pub fn build(
    labels: &[i32],
    width: usize,
    height: usize,
    regions: usize,
    thin: bool,
) -> Result<Topology, String> {
    if width == 0 || height == 0 || labels.len() != width * height {
        return Err("Label image size does not match its data".into());
    }
    if labels.iter().any(|&l| l < 0 || l as usize >= regions) {
        return Err("Label outside the region count".into());
    }
    let (w, h) = (width as i32, height as i32);
    let image = Grid {
        width,
        height,
        cells: labels,
    };
    // 0x498a50: vertices touching a label change, then the numbering.
    let mut grid = Grid::new(width + 1, height + 1, 0);
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            if image.at(x, y) != image.at(x, y + 1) {
                grid.set(x, y + 1, grid.at(x, y + 1) + 1);
                grid.set(x + 1, y + 1, grid.at(x + 1, y + 1) + 1);
            }
            if image.at(x, y) != image.at(x + 1, y) {
                grid.set(x + 1, y, grid.at(x + 1, y) + 1);
                grid.set(x + 1, y + 1, grid.at(x + 1, y + 1) + 1);
            }
        }
    }
    // The vertex the loops above never reach from its own pixel.
    if w >= 2
        && h >= 2
        && (image.at(w - 1, h - 1) != image.at(w - 2, h - 1)
            || image.at(w - 1, h - 1) != image.at(w - 1, h - 2))
    {
        grid.set(w - 1, h - 1, grid.at(w - 1, h - 1) + 1);
    }
    let mut next = 0;
    for y in 0..=h {
        grid.set(0, y, next);
        next += 1;
    }
    for y in 0..=h {
        grid.set(w, y, next);
        next += 1;
    }
    for x in 1..w {
        grid.set(x, 0, next);
        next += 1;
    }
    for x in 1..w {
        grid.set(x, h, next);
        next += 1;
    }
    for y in 1..h {
        for x in 1..w {
            if grid.at(x, y) > 0 {
                grid.set(x, y, next);
                next += 1;
            } else {
                grid.set(x, y, -1);
            }
        }
    }
    // The node records, in numbering order (0x480230).
    let mut nodes = vec![
        Node {
            x: 0.,
            y: 0.,
            state: 0,
            flags: 0
        };
        next as usize
    ];
    for y in 0..=h {
        for x in 0..=w {
            let index = grid.at(x, y);
            if index >= 0 {
                let corner = (x == 0 || x == w) && (y == 0 || y == h);
                nodes[index as usize] = Node {
                    x: x as f64,
                    y: y as f64,
                    state: if corner { 3 } else { 0 },
                    flags: 0,
                };
            }
        }
    }
    let mut border = [h + 1, h + 1, w - 1, w - 1];
    // Trace every region from its first node corner in scan order (0x4999d0).
    let mut visited = Grid::new(width + 1, height + 1, -1);
    let mut contours = vec![Contour::default(); regions];
    for y in 0..h {
        for x in 0..w {
            let label = image.at(x, y);
            if !contours[label as usize].nodes.is_empty() {
                continue;
            }
            for m in 0..4 {
                let (cx, cy) = (x + CORNER_DX[m], y + CORNER_DY[m]);
                if grid.at(cx, cy) >= 0 {
                    contours[label as usize] = trace(&image, &grid, &mut visited, label, cx, cy)?;
                    break;
                }
            }
        }
    }
    // Junctions: fewer than two of the six pairs around the vertex agree.
    for y in 0..=h {
        for x in 0..=w {
            let index = grid.at(x, y);
            if index < 0 {
                continue;
            }
            let l: [i32; 4] =
                std::array::from_fn(|m| image.label(x + PIXEL_DX[m], y + PIXEL_DY[m]));
            let agree = [(1, 2), (2, 3), (1, 3), (0, 2), (0, 1), (0, 3)]
                .iter()
                .filter(|(a, b)| l[*a] == l[*b])
                .count();
            if agree < 2 {
                nodes[index as usize].state = 3;
            }
        }
    }
    thin_and_compact(&mut nodes, &mut contours, &mut border, thin);
    // 0x49a213: the enclosing contour. Every neighbour list is sorted and
    // distinct (`trace`), so a binary search answers the linear scan's
    // question.
    for i in 0..contours.len() {
        let mut enclosing = -1;
        for &other in &contours[i].neighbours {
            let lists_back = contours[other as usize]
                .neighbours
                .binary_search(&(i as i32))
                .is_ok();
            if !lists_back {
                enclosing = other;
                break;
            }
        }
        contours[i].enclosing = enclosing;
    }
    Ok(Topology {
        nodes,
        contours,
        border,
    })
}

/// 0x4999d0: the boundary of `label` from the node at (`x`, `y`).
fn trace(
    image: &Grid<&[i32]>,
    grid: &Grid,
    visited: &mut Grid,
    label: i32,
    mut x: i32,
    mut y: i32,
) -> Result<Contour, String> {
    let (w, h) = (image.width as i32, image.height as i32);
    let mut ids = Vec::new();
    let mut others = Vec::new();
    let (mut prev_x, mut prev_y) = (-1, -1);
    // The candidate slots persist between steps as the original's stack
    // arrays do: slot 0 starts at the start vertex.
    let mut candidate = [(x, y, 0usize, 0i32); 4];
    let mut stored_other = [0i32; 4];
    let mut steps = 0usize;
    loop {
        ids.push(grid.at(x, y));
        steps += 1;
        if steps > 4 * (image.width + 1) * (image.height + 1) {
            return Err(format!("Contour of region {label} does not close"));
        }
        let mut n = 0usize;
        for k in 0..4 {
            let (nx, ny) = (x + STEP_DX[k], y + STEP_DY[k]);
            let (bx, by) = (x + PIXEL_DX[k], y + PIXEL_DY[k]);
            let k2 = (k + 1) % 4;
            let (cx, cy) = (x + PIXEL_DX[k2], y + PIXEL_DY[k2]);
            if nx < 0 || nx > w || ny < 0 || ny > h {
                continue;
            }
            if grid.at(nx, ny) < 0 {
                continue;
            }
            let (lb, lc) = (image.label(bx, by), image.label(cx, cy));
            if lb == lc || lb != label {
                continue;
            }
            stored_other[n] = lc;
            if visited.at(nx, ny) == label {
                continue;
            }
            candidate[n] = (nx, ny, k, lc);
            n += 1;
        }
        let other;
        if n > 1 && prev_x != -1 && prev_y != -1 {
            // A vertex with two ways on: keep hugging the pixel the last
            // step ran along.
            let hugging = (0..n).find(|&i| {
                let k = candidate[i].2;
                x + PIXEL_DX[k] == prev_x && y + PIXEL_DY[k] == prev_y
            });
            match hugging {
                Some(i) => {
                    other = candidate[i].3;
                    x = candidate[i].0;
                    y = candidate[i].1;
                }
                None => return Err(format!("Contour of region {label} cannot continue")),
            }
        } else {
            if n <= 1 {
                visited.set(x, y, label);
            }
            let k = candidate[0].2;
            prev_x = x + PIXEL_DX[k];
            prev_y = y + PIXEL_DY[k];
            other = if n == 0 {
                stored_other[0]
            } else {
                candidate[0].3
            };
            x = candidate[0].0;
            y = candidate[0].1;
        }
        others.push(other);
        if visited.at(x, y) == label {
            break;
        }
    }
    let mut neighbours = others.clone();
    neighbours.sort_unstable();
    neighbours.dedup();
    neighbours.retain(|&l| l != -1);
    Ok(Contour {
        edges: others
            .iter()
            .map(|&o| Edge { other: o, steps: 1 })
            .collect(),
        nodes: ids,
        neighbours,
        enclosing: -1,
    })
}

/// 0x4993e0: mark removable nodes, thin straight runs, renumber.
fn thin_and_compact(
    nodes: &mut Vec<Node>,
    contours: &mut [Contour],
    border: &mut [i32; 4],
    thin: bool,
) {
    let mut removed = [0i32; 4];
    let mut index = 0usize;
    for run in 0..4 {
        let skip = usize::from(run < 2);
        index += skip;
        let count = border[run] as usize;
        for _ in skip..count.saturating_sub(skip) {
            if nodes[index].state != 3 {
                nodes[index].flags |= REMOVED;
                removed[run] += 1;
            }
            index += 1;
        }
        index += skip;
    }
    // 2 (w + h) nodes were numbered along the canvas edge.
    let on_border = (2 * (border[0] + border[2])) as usize;
    for node in nodes.iter_mut().take(on_border) {
        node.flags |= SEEN;
    }
    if thin {
        for contour in contours.iter_mut() {
            thin_runs(nodes, &contour.nodes);
        }
    }
    for run in 0..4 {
        border[run] -= removed[run];
    }
    let mut remap = vec![-1i32; nodes.len()];
    let mut kept = Vec::with_capacity(nodes.len());
    for (i, node) in nodes.iter().enumerate() {
        if node.flags & REMOVED == 0 {
            remap[i] = kept.len() as i32;
            kept.push(*node);
        }
    }
    *nodes = kept;
    for contour in contours.iter_mut() {
        let count = contour.nodes.len();
        let mut ids = Vec::new();
        let mut edges = Vec::new();
        let mut last = 0;
        for c in 0..count {
            let mapped = remap[contour.nodes[c] as usize];
            if mapped < 0 {
                continue;
            }
            ids.push(mapped);
            edges.push(Edge {
                other: contour.edges[c].other,
                steps: (c - last) as i32,
            });
            last = c;
        }
        if let Some(first) = edges.first_mut() {
            first.steps = (count - last) as i32;
        }
        contour.nodes = ids;
        contour.edges = edges;
    }
}

/// The straight-run and stair-step walk of 0x4993e0 over one contour of at
/// least `THIN_MINIMUM` nodes: `n + WINDOW` consecutive node pairs, unit
/// steps rounded from the positions, runs broken by a change of direction
/// or by a corner, a border node, a removed node or a node already walked.
fn thin_runs(nodes: &mut [Node], ids: &[i32]) {
    let n = ids.len();
    if n < THIN_MINIMUM {
        return;
    }
    let (mut run, mut last_dx, mut last_dy, mut xbits, mut ybits) = (0i32, 0i32, 0i32, 0u32, 0u32);
    for j in 1..(n as i32 + WINDOW) {
        let prev = ids[((j - 1) % n as i32) as usize] as usize;
        let cur = ids[(j % n as i32) as usize] as usize;
        if nodes[prev].flags & REMOVED != 0
            || nodes[cur].flags & SEEN != 0
            || nodes[prev].state == 3
            || nodes[cur].state == 3
        {
            run = 0;
            last_dx = 0;
            last_dy = 0;
            xbits = 0;
            ybits = 0;
            continue;
        }
        if j >= WINDOW {
            nodes[prev].flags |= SEEN;
        }
        let dx = (nodes[prev].x - nodes[cur].x + 0.5).floor() as i32;
        let dy = (nodes[prev].y - nodes[cur].y + 0.5).floor() as i32;
        let broken = (last_dx != 0 && dx != 0 && dx != last_dx)
            || (last_dy != 0 && dy != 0 && dy != last_dy);
        if broken {
            run = 0;
            xbits = 0;
            ybits = 0;
        }
        if dx != 0 {
            last_dx = dx;
        }
        if dy != 0 {
            last_dy = dy;
        }
        xbits = (xbits << 1) | u32::from(dx != 0);
        ybits = (ybits << 1) | u32::from(dy != 0);
        run += 1;
        if run > WINDOW {
            run = WINDOW;
        } else if run < 5 {
            continue;
        }
        // 0x498930: the first pattern no longer than the run that either
        // bit string matches.
        let matched = PATTERNS.iter().find(|(length, mask, _)| {
            let window = (1u32 << *length) - 1;
            *length <= run && (xbits & window == *mask || ybits & window == *mask)
        });
        let Some(&(length, _, keep)) = matched else {
            continue;
        };
        let mut bits = keep;
        let mut at = j + n as i32;
        for _ in 0..=length {
            if bits & 1 == 0 {
                let id = ids[(at % n as i32) as usize] as usize;
                nodes[id].flags |= REMOVED | SEEN;
            }
            bits >>= 1;
            at -= 1;
        }
        run = 0;
        last_dx = 0;
        last_dy = 0;
        xbits = 0;
        ybits = 0;
    }
}

#[cfg(test)]
mod tests;
