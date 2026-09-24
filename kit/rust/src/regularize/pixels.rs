//! The pixels' say over an arc (September 23, 2026). Regularize judges a run
//! by its pieces alone, and a letter bowl that is not a circle lies within
//! the band of one: at a 72 px cap it drew the bowls of a and 2 as circle
//! arcs 0.9 px into the counters, where the anti-aliased pixels settle the
//! outline to 0.1 px (the defect sweep of September 23, 2026). A run that
//! became arcs is therefore held against the source: across it the pixels
//! blend the colours on its two sides, and a pixel's blend says how much of
//! it lies on each side, which each drawing predicts from its distance to
//! the pixel's centre. When the arcs misjudge that by more than
//! `slack` per pixel of run than the same run drawn without arcs (its lines
//! still made), the run is drawn without them.
use crate::geometry::{dist, Point};
use crate::raster::Raster;
use crate::simplify::Edge;
use std::collections::HashMap;

/// The source picture a regularized run is checked against, and whether its
/// edges are anti-aliased (blends) or pixel-edged (each pixel one side).
#[derive(Clone, Copy)]
pub struct PixelCheck<'a> {
    pub source: &'a Raster,
    pub anti_aliased: bool,
}

/// Pixels between the points a drawing is sampled at.
const STEP: f64 = 0.25;
/// Pixels whose centre lies within this distance of either drawing are
/// the evidence.
const BAND: f64 = 2.;
/// The colours on each side are read this far off the run.
const SIDE: f64 = 2.;
/// Sides closer than this (largest channel, levels) settle nothing.
const MIN_SIDE_CONTRAST: f64 = 24.;
/// Misjudged pixel fraction per pixel of run the arcs may add: on
/// pixel-edged artwork every drawing misses the staircase by as much.
const SLACK_ANTI_ALIASED: f64 = 0.02;
const SLACK_PIXEL_EDGED: f64 = 0.05;

/// Whether the pixels speak against `arcs`, the run drawn with arcs, in
/// favour of `plain`, the same run drawn without them.
pub(super) fn against(check: PixelCheck, arcs: &[Edge], plain: &[Edge]) -> bool {
    let (a, b) = (polyline(arcs), polyline(plain));
    let Some((near, far)) = sides(check.source, &b) else {
        return false;
    };
    let span: Vec<f64> = (0..3).map(|c| near[c] - far[c]).collect();
    let span2: f64 = span.iter().map(|v| v * v).sum();
    let predict = |d: f64| {
        if check.anti_aliased {
            (0.5 + d).clamp(0., 1.)
        } else if d > 0. {
            1.
        } else {
            0.
        }
    };
    let (grid_a, grid_b) = (Grid::new(&a), Grid::new(&b));
    let (w, h) = (check.source.width as i64, check.source.height as i64);
    let (mut misfit_a, mut misfit_b) = (0., 0.);
    let mut pixels: Vec<(i64, i64)> = grid_a.cells().chain(grid_b.cells()).collect();
    pixels.sort_unstable();
    pixels.dedup();
    for (x, y) in pixels {
        if x < 0 || y < 0 || x >= w || y >= h {
            continue;
        }
        let centre = Point {
            x: x as f64 + 0.5,
            y: y as f64 + 0.5,
        };
        let (Some(da), Some(db)) = (grid_a.signed(&a, centre), grid_b.signed(&b, centre)) else {
            continue;
        };
        if da.abs() > BAND && db.abs() > BAND {
            continue;
        }
        let p = check.source.pixels[(y * w + x) as usize].0;
        let seen = ((0..3)
            .map(|c| (p[c] as f64 - far[c]) * span[c])
            .sum::<f64>()
            / span2)
            .clamp(0., 1.);
        misfit_a += (predict(da) - seen).abs();
        misfit_b += (predict(db) - seen).abs();
    }
    let length: f64 = b.windows(2).map(|s| dist(s[0], s[1])).sum();
    let slack = if check.anti_aliased {
        SLACK_ANTI_ALIASED
    } else {
        SLACK_PIXEL_EDGED
    };
    misfit_a > misfit_b + slack * length
}

/// The drawing sampled every `STEP` pixels or closer, ends included.
fn polyline(edges: &[Edge]) -> Vec<Point> {
    let mut out = vec![edges[0].start()];
    for edge in edges {
        let rough: f64 = edge.cubic.points.windows(2).map(|s| dist(s[0], s[1])).sum();
        let n = if edge.line {
            1
        } else {
            ((rough / STEP).ceil() as usize).max(1)
        };
        out.extend((1..=n).map(|i| edge.cubic.evaluate(i as f64 / n as f64)));
    }
    out
}

/// The mean colour `SIDE` pixels to the left of the drawing (positive
/// signed distance) and to its right, read at each segment's midpoint;
/// `None` when they are too alike to tell the sides apart.
fn sides(source: &Raster, line: &[Point]) -> Option<([f64; 3], [f64; 3])> {
    let (mut left, mut right, mut n) = ([0.; 3], [0.; 3], 0.);
    let read = |p: Point| -> Option<[f64; 3]> {
        let (x, y) = (p.x.floor() as i64, p.y.floor() as i64);
        (x >= 0 && y >= 0 && (x as usize) < source.width && (y as usize) < source.height).then(
            || {
                let c = source.pixels[y as usize * source.width + x as usize].0;
                [c[0] as f64, c[1] as f64, c[2] as f64]
            },
        )
    };
    for s in line.windows(2) {
        let len = dist(s[0], s[1]);
        if len < 1e-9 {
            continue;
        }
        let (dx, dy) = ((s[1].x - s[0].x) / len, (s[1].y - s[0].y) / len);
        let mid = Point {
            x: (s[0].x + s[1].x) / 2.,
            y: (s[0].y + s[1].y) / 2.,
        };
        // Left of the direction of travel in picture coordinates (y down),
        // the side `signed` counts positive.
        let normal = (dy, -dx);
        let at = |k: f64| Point {
            x: mid.x + normal.0 * k,
            y: mid.y + normal.1 * k,
        };
        if let (Some(l), Some(r)) = (read(at(SIDE)), read(at(-SIDE))) {
            for c in 0..3 {
                left[c] += l[c];
                right[c] += r[c];
            }
            n += 1.;
        }
    }
    if n == 0. {
        return None;
    }
    let (left, right) = (left.map(|v| v / n), right.map(|v| v / n));
    let contrast = (0..3)
        .map(|c| (left[c] - right[c]).abs())
        .fold(0., f64::max);
    (contrast >= MIN_SIDE_CONTRAST).then_some((left, right))
}

/// The segments of a polyline by the pixel cells within `BAND` of them.
struct Grid {
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl Grid {
    fn new(line: &[Point]) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, s) in line.windows(2).enumerate() {
            let (x0, x1) = (s[0].x.min(s[1].x) - BAND, s[0].x.max(s[1].x) + BAND);
            let (y0, y1) = (s[0].y.min(s[1].y) - BAND, s[0].y.max(s[1].y) + BAND);
            for y in (y0.floor() as i64)..=(y1.floor() as i64) {
                for x in (x0.floor() as i64)..=(x1.floor() as i64) {
                    cells.entry((x, y)).or_default().push(i);
                }
            }
        }
        Self { cells }
    }

    fn cells(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.cells.keys().copied()
    }

    /// The signed distance from `p` to the nearest segment listed at its
    /// cell, positive to the left of the direction of travel; `None` when no
    /// segment is listed there (it is more than `BAND` away).
    fn signed(&self, line: &[Point], p: Point) -> Option<f64> {
        let near = self.cells.get(&(p.x.floor() as i64, p.y.floor() as i64))?;
        let mut best: Option<(f64, f64)> = None;
        for &i in near {
            let (a, b) = (line[i], line[i + 1]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len2 = dx * dx + dy * dy;
            let t = if len2 < 1e-18 {
                0.
            } else {
                (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0., 1.)
            };
            let (qx, qy) = (a.x + t * dx, a.y + t * dy);
            let d = (p.x - qx).hypot(p.y - qy);
            if best.is_none_or(|(bd, _)| d < bd) {
                // Left of travel in picture coordinates: the cross product of
                // the direction with the offset is negative there (y down).
                let cross = dx * (p.y - a.y) - dy * (p.x - a.x);
                best = Some((d, if cross < 0. { d } else { -d }));
            }
        }
        best.map(|(_, s)| s)
    }
}
