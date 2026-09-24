//! Shape layers: the vector art in a Photoshop document. A shape layer is a
//! vector mask (`vmsk` or `vsms`, its outline) with a fill (`SoCo`, `GdFl`
//! or `PtFl`, or `vscg` beside a stroke) and perhaps a stroke (`vstk`); a
//! fill layer without a vector mask fills the whole document. This reads
//! them for `psd::shapes`, which writes them as the app's flat paths, and
//! draws them for the compositor, whose picture would otherwise leave them
//! out.
//!
//! What flat paths cannot hold is drawn as near as they come, and named: a
//! gradient in its first stop's colour, a pattern in mid grey. A vector
//! mask's subpaths are put together by one fill rule rather than by
//! Photoshop's operations on them: even-odd when a subpath's length record
//! marks it so (its flags other than 2, as ag-psd reads them), when a
//! subpath is subtracted or excluded, or when the mask is inverted or starts
//! full (the document's rectangle is then added to the outline); nonzero
//! otherwise. An intersection is drawn as a combination and noted. Strokes
//! are centred on their paths; the compositor draws their corners round and
//! their ends as the stroke says.

use super::descriptor::{self, Descriptor, Value};
use super::{group_opacities, Layer, Reader, Rect, Window};
use crate::import::Imported;
use std::fmt::Write as _;

/// A point in document pixels, x then y.
type Point = [f64; 2];

/// How far a curve's straight pieces may stray from it, in pixels.
const TOLERANCE: f64 = 0.1;

/// How many times each pixel row is sampled: coverage is exact along a row
/// and sampled this many times down it.
const SAMPLES: usize = 5;

/// The most points one shape's outline may be flattened into, and the most
/// row steps and pixel visits drawing it may take: past either the shape is
/// refused, so a hostile outline can neither fill the memory nor keep the
/// app busy.
const POINTS: usize = 4_000_000;
const WORK: u64 = 400_000_000;

/// What a shape past those limits says.
const DETAILED: &str = "A shape in this Photoshop document is too detailed to draw";

/// What a damaged vector mask says.
const DAMAGED_MASK: &str = "A vector mask in this Photoshop document is damaged";

/// The colour patterns and unreadable colours are drawn in.
const GREY: [u8; 3] = [128; 3];

/// The blocks that make a layer vector art, as its additional layer
/// information holds them.
#[derive(Clone, Copy, Default)]
pub(super) struct Blocks<'a> {
    /// The vector mask (`vmsk` or `vsms`).
    pub mask: Option<&'a [u8]>,
    /// The fill (`SoCo`, `GdFl` or `PtFl`): a version, then a descriptor.
    pub fill: Option<&'a [u8]>,
    /// The fill as `vscg` keeps it beside a stroke: its key, then a version
    /// and a descriptor.
    pub content: Option<&'a [u8]>,
    /// The vector stroke (`vstk`).
    pub stroke: Option<&'a [u8]>,
    /// The layer effects (`lfx2` or `lmfx`).
    pub effects: Option<&'a [u8]>,
    /// Whether it is a text layer (`TySh`).
    pub text: bool,
}

/// The document shapes are drawn in: its size in pixels, and its resolution
/// in pixels an inch when resource 1005 gives one.
#[derive(Clone, Copy, Debug)]
pub(super) struct Page {
    pub width: usize,
    pub height: usize,
    pub ppi: Option<f64>,
}

/// What a shape holds that is drawn otherwise than Photoshop draws it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Approximations {
    gradients: bool,
    patterns: bool,
    colours: bool,
    intersections: bool,
    alignment: bool,
    dashes: bool,
}

/// One subpath of an outline, in document pixels.
#[derive(Clone, Debug)]
struct Subpath {
    start: Point,
    /// Cubic pieces: two control points, then the end.
    curves: Vec<[Point; 3]>,
    closed: bool,
}

/// A vector stroke.
#[derive(Clone, Debug)]
struct Stroke {
    colour: [u8; 3],
    /// Its width in document pixels.
    width: f64,
    /// Its own opacity (0 to 1), under the layer's.
    opacity: f64,
    /// SVG's names for its corners and its ends.
    join: &'static str,
    cap: &'static str,
}

/// A shape or fill layer's vector art.
#[derive(Clone, Debug)]
pub(super) struct Shape {
    /// Whether the document's rectangle is part of the filled outline: an
    /// inverted mask or one that starts full, or a fill layer without a
    /// vector mask (whose subpaths are then none).
    page: bool,
    paths: Vec<Subpath>,
    even_odd: bool,
    /// Whether it has an outline of its own: a shape layer rather than a
    /// fill layer.
    pub outlined: bool,
    /// The fill's colour, `None` when the fill is turned off.
    fill: Option<[u8; 3]>,
    stroke: Option<Stroke>,
    pub approximations: Approximations,
}

impl Shape {
    /// The vector art of a layer whose blocks are `blocks`, or `None` when it
    /// has neither a fill nor a stroke (a raster, text or adjustment layer,
    /// even one with a vector mask).
    pub(super) fn read(blocks: &Blocks, page: Page) -> Result<Option<Shape>, String> {
        let fill = match (blocks.fill, blocks.content) {
            (Some(data), _) => Some(descriptor::versioned(data)?),
            (None, Some(data)) => Some(descriptor::versioned(
                data.get(4..).ok_or(descriptor::DAMAGED)?,
            )?),
            (None, None) => None,
        };
        let stroke = blocks.stroke.map(descriptor::versioned).transpose()?;
        if fill.is_none() && stroke.is_none() {
            return Ok(None);
        }
        let mut approximations = Approximations::default();
        let (width, height) = (page.width as f64, page.height as f64);
        let outline = match blocks.mask {
            Some(data) => outline(data, width, height, &mut approximations)?,
            None => None,
        };
        let fill_enabled = stroke
            .as_ref()
            .and_then(|stroke| stroke.bool("fillEnabled"))
            .unwrap_or(true);
        let stroke = stroke
            .filter(|stroke| stroke.bool("strokeEnabled") == Some(true))
            .and_then(|stroke| read_stroke(&stroke, page, &mut approximations));
        let fill = fill
            .filter(|_| fill_enabled)
            .map(|fill| paint(&fill, &mut approximations));
        Ok(Some(match outline {
            Some(outline) => Shape {
                page: outline.page,
                outlined: !outline.paths.is_empty(),
                paths: outline.paths,
                even_odd: outline.even_odd,
                fill,
                stroke,
                approximations,
            },
            // No vector mask, or a disabled one: the fill covers everything.
            None => Shape {
                page: true,
                paths: Vec::new(),
                even_odd: false,
                outlined: false,
                fill,
                stroke: None,
                approximations,
            },
        }))
    }

    /// The stroke, when there is a path to draw it along.
    fn stroke(&self) -> Option<&Stroke> {
        self.stroke.as_ref().filter(|_| !self.paths.is_empty())
    }

    /// The fill's colour, when there is an outline to fill.
    fn fill(&self) -> Option<[u8; 3]> {
        self.fill.filter(|_| self.page || !self.paths.is_empty())
    }

    /// Writes it as flat paths at `scale` points a pixel and `opacity`: one
    /// path element, or two when the document's rectangle is filled and the
    /// subpaths alone are stroked. Returns whether it wrote anything.
    fn write(&self, page: Page, scale: f64, opacity: f64, out: &mut String) -> bool {
        let mut d = String::new();
        for path in &self.paths {
            path.data(scale, &mut d);
        }
        let mut stroke = self.stroke();
        let wrote = self.fill().is_some() || stroke.is_some();
        if let Some(rgb) = self.fill() {
            let mut filled = String::new();
            if self.page {
                rectangle(page).data(scale, &mut filled);
            }
            filled.push_str(&d);
            let _ = write!(out, "<path d=\"{filled}\" fill=\"{}\"", hex(rgb));
            if self.even_odd {
                out.push_str(" fill-rule=\"evenodd\"");
            }
            if opacity < 0.999 {
                let _ = write!(out, " fill-opacity=\"{}\"", num(opacity));
            }
            if !self.page {
                if let Some(stroke) = stroke.take() {
                    stroke.write(scale, opacity, out);
                }
            }
            out.push_str("/>\n");
        }
        if let Some(stroke) = stroke {
            let _ = write!(out, "<path d=\"{d}\" fill=\"none\"");
            stroke.write(scale, opacity, out);
            out.push_str("/>\n");
        }
        wrote
    }

    /// It drawn over the part of the document it covers, after `claim` has
    /// counted that part against the compositor's limits; `None` when it
    /// covers nothing.
    pub(super) fn draw(
        &self,
        page: Page,
        claim: &mut dyn FnMut(Rect) -> Result<(), String>,
    ) -> Result<Option<Drawn>, String> {
        let (fill, stroke) = (self.fill(), self.stroke());
        if fill.is_none() && stroke.is_none() {
            return Ok(None);
        }
        let rect = if self.page && fill.is_some() {
            Rect {
                top: 0,
                left: 0,
                bottom: page.height as i64,
                right: page.width as i64,
            }
        } else {
            // A curve lies within its control points.
            let margin = stroke.map_or(0., |stroke| stroke.width / 2.) + 1.;
            let mut bounds = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for path in &self.paths {
                for p in std::iter::once(&path.start).chain(path.curves.iter().flatten()) {
                    bounds = [
                        bounds[0].min(p[0]),
                        bounds[1].min(p[1]),
                        bounds[2].max(p[0]),
                        bounds[3].max(p[1]),
                    ];
                }
            }
            Rect {
                top: (bounds[1] - margin).floor() as i64,
                left: (bounds[0] - margin).floor() as i64,
                bottom: (bounds[3] + margin).ceil() as i64,
                right: (bounds[2] + margin).ceil() as i64,
            }
        };
        let window = Window::of(rect, page.width, page.height);
        if window.is_empty() {
            return Ok(None);
        }
        claim(Rect {
            top: window.y0 as i64,
            left: window.x0 as i64,
            bottom: window.y1 as i64,
            right: window.x1 as i64,
        })?;
        let mut work = WORK;
        let fill_coverage = match fill {
            Some(_) => {
                let mut outline = Vec::new();
                if self.page {
                    outline.push(rectangle(page).flatten(true, POINTS)?);
                }
                let mut left = POINTS;
                for path in &self.paths {
                    let points = path.flatten(true, left)?;
                    left -= points.len();
                    outline.push(points);
                }
                cover(&outline, self.even_odd, window, &mut work)?
            }
            None => Vec::new(),
        };
        let stroke_coverage = match stroke {
            Some(stroke) => stroke.cover(&self.paths, window, &mut work)?,
            None => Vec::new(),
        };
        Ok(Some(Drawn {
            window,
            fill: fill_coverage,
            stroke: stroke_coverage,
            fill_colour: fill.unwrap_or_default(),
            stroke_colour: stroke.map_or([0; 3], |stroke| stroke.colour),
            stroke_opacity: stroke.map_or(0., |stroke| stroke.opacity as f32),
        }))
    }
}

/// A shape drawn over a window of the document: how much of each pixel its
/// fill and its stroke cover (empty when it has none).
pub(super) struct Drawn {
    pub window: Window,
    fill: Vec<f32>,
    stroke: Vec<f32>,
    fill_colour: [u8; 3],
    stroke_colour: [u8; 3],
    stroke_opacity: f32,
}

impl Drawn {
    /// The colour and coverage (0 to 1) of pixel `i` of the window: the
    /// stroke over the fill.
    pub(super) fn at(&self, i: usize) -> ([u8; 3], f32) {
        let fill = self.fill.get(i).copied().unwrap_or(0.);
        let stroke = self.stroke.get(i).copied().unwrap_or(0.) * self.stroke_opacity;
        let alpha = stroke + fill * (1. - stroke);
        if alpha <= 0. {
            return ([0; 3], 0.);
        }
        let mix = |c: usize| {
            let s = f32::from(self.stroke_colour[c]) * stroke;
            let f = f32::from(self.fill_colour[c]) * fill * (1. - stroke);
            ((s + f) / alpha + 0.5) as u8
        };
        ([mix(0), mix(1), mix(2)], alpha)
    }
}

impl Stroke {
    /// Its SVG attributes at `scale` points a pixel and the layer's
    /// `opacity`.
    fn write(&self, scale: f64, opacity: f64, out: &mut String) {
        let _ = write!(
            out,
            " stroke=\"{}\" stroke-width=\"{}\"",
            hex(self.colour),
            num(self.width * scale)
        );
        if self.join != "miter" {
            let _ = write!(out, " stroke-linejoin=\"{}\"", self.join);
        }
        if self.cap != "butt" {
            let _ = write!(out, " stroke-linecap=\"{}\"", self.cap);
        }
        let alpha = opacity * self.opacity;
        if alpha < 0.999 {
            let _ = write!(out, " stroke-opacity=\"{}\"", num(alpha));
        }
    }

    /// How much of each pixel of `window` it covers along `paths`: within
    /// half its width of them (so its corners come out round), cut square at
    /// the ends of an open path unless its ends are round.
    fn cover(&self, paths: &[Subpath], window: Window, work: &mut u64) -> Result<Vec<f32>, String> {
        let half = self.width / 2.;
        let width = window.width();
        let mut coverage = vec![0f32; window.area()];
        let mut left = POINTS;
        for path in paths {
            let points = path.flatten(false, left)?;
            left -= points.len();
            // A square end reaches half the width past the path's end.
            let past = if self.cap == "square" { half } else { 0. };
            let ends = if path.closed || self.cap == "round" {
                None
            } else {
                ends(&points)
            };
            for pair in points.windows(2) {
                let [a, b] = [pair[0], pair[1]];
                let xs = range(
                    a[0].min(b[0]) - half - 1.,
                    a[0].max(b[0]) + half + 1.,
                    window.x0,
                    window.x1,
                );
                let ys = range(
                    a[1].min(b[1]) - half - 1.,
                    a[1].max(b[1]) + half + 1.,
                    window.y0,
                    window.y1,
                );
                charge(work, (xs.len() * ys.len()) as u64)?;
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let length2 = dx * dx + dy * dy;
                for y in ys {
                    for x in xs.clone() {
                        let c = [x as f64 + 0.5, y as f64 + 0.5];
                        let t = if length2 > 0. {
                            (((c[0] - a[0]) * dx + (c[1] - a[1]) * dy) / length2).clamp(0., 1.)
                        } else {
                            0.
                        };
                        let distance = (c[0] - a[0] - t * dx).hypot(c[1] - a[1] - t * dy);
                        let mut cover = (half + 0.5 - distance).clamp(0., 1.);
                        if let Some((start, first, end, last)) = ends {
                            let behind =
                                (c[0] - start[0]) * first[0] + (c[1] - start[1]) * first[1];
                            let beyond = (end[0] - c[0]) * last[0] + (end[1] - c[1]) * last[1];
                            cover *= (0.5 + past + behind).clamp(0., 1.);
                            cover *= (0.5 + past + beyond).clamp(0., 1.);
                        }
                        let slot = &mut coverage[(y - window.y0) * width + x - window.x0];
                        *slot = slot.max(cover as f32);
                    }
                }
            }
        }
        Ok(coverage)
    }
}

/// An open path's ends, through its `points`: where it starts, which way it
/// goes from there, where it ends and which way it goes there; `None` when
/// it goes nowhere.
fn ends(points: &[Point]) -> Option<(Point, Point, Point, Point)> {
    let direction = |from: Point, to: Point| {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let length = dx.hypot(dy);
        (length > 0.).then(|| [dx / length, dy / length])
    };
    let (start, end) = (*points.first()?, *points.last()?);
    let first = points.iter().find_map(|p| direction(start, *p))?;
    let last = points.iter().rev().find_map(|p| direction(*p, end))?;
    Some((start, first, end, last))
}

impl Subpath {
    /// Its path data at `scale` points a pixel, added to `d`; straight pieces
    /// are lines, and a closed subpath's last straight piece is its close.
    fn data(&self, scale: f64, d: &mut String) {
        let p = |q: Point| format!("{} {}", num(q[0] * scale), num(q[1] * scale));
        let _ = write!(d, "M{}", p(self.start));
        let mut at = self.start;
        for (i, &[c1, c2, end]) in self.curves.iter().enumerate() {
            let straight = c1 == at && c2 == end;
            if straight && !(self.closed && i + 1 == self.curves.len()) {
                let _ = write!(d, "L{}", p(end));
            } else if !straight {
                let _ = write!(d, "C{} {} {}", p(c1), p(c2), p(end));
            }
            at = end;
        }
        if self.closed {
            d.push('Z');
        }
    }

    /// Points along it within `TOLERANCE`, fewer than `most` of them;
    /// `close` adds the way back to its start.
    fn flatten(&self, close: bool, most: usize) -> Result<Vec<Point>, String> {
        if most < 2 {
            return Err(DETAILED.into());
        }
        let mut points = vec![self.start];
        let mut at = self.start;
        for &[c1, c2, end] in &self.curves {
            let steps = if c1 == at && c2 == end {
                1
            } else {
                // The distance from a cubic to its chords shrinks with the
                // square of their number (the second differences bound it).
                let bend = |a: Point, b: Point, c: Point| {
                    (a[0] - 2. * b[0] + c[0]).hypot(a[1] - 2. * b[1] + c[1])
                };
                let bend = bend(at, c1, c2).max(bend(c1, c2, end));
                (bend * 0.75 / TOLERANCE).sqrt().ceil().clamp(1., 100.) as usize
            };
            if points.len() + steps >= most {
                return Err(DETAILED.into());
            }
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let u = 1. - t;
                let (a, b, c, e) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
                points.push([
                    a * at[0] + b * c1[0] + c * c2[0] + e * end[0],
                    a * at[1] + b * c1[1] + c * c2[1] + e * end[1],
                ]);
            }
            at = end;
        }
        if close && at != self.start {
            points.push(self.start);
        }
        Ok(points)
    }
}

/// The document's rectangle as a subpath.
fn rectangle(page: Page) -> Subpath {
    let (w, h) = (page.width as f64, page.height as f64);
    let line = |from: Point, to: Point| [from, to, to];
    Subpath {
        start: [0., 0.],
        curves: vec![
            line([0., 0.], [w, 0.]),
            line([w, 0.], [w, h]),
            line([w, h], [0., h]),
            line([0., h], [0., 0.]),
        ],
        closed: true,
    }
}

/// A vector mask's outline, in document pixels.
struct Outline {
    page: bool,
    paths: Vec<Subpath>,
    even_odd: bool,
}

/// The outline of the vector mask `data` in a document `width` by `height`
/// pixels, or `None` when the mask is disabled: its version and flags (1
/// inverted, 2 not linked, 4 disabled), then 26-byte path records.
fn outline(
    data: &[u8],
    width: f64,
    height: f64,
    approximations: &mut Approximations,
) -> Result<Option<Outline>, String> {
    let mut r = Reader::new(data);
    r.skip(4)?;
    let flags = r.u32()?;
    if flags & 4 != 0 {
        return Ok(None);
    }
    /// A subpath as its records give it: its knots (the control point
    /// before, the anchor, the control point after), whether it is closed,
    /// its operation (-1 part of the one before, 0 exclude, 1 combine, 2
    /// subtract, 3 intersect) and whether it is marked even-odd.
    struct Record {
        knots: Vec<[Point; 3]>,
        closed: bool,
        operation: i16,
        even_odd: bool,
    }
    let mut subpaths: Vec<Record> = Vec::new();
    let mut starts_full = false;
    while r.rest().len() >= 26 {
        let selector = r.u16()?;
        let body = r.take(24)?;
        let word = |i: usize| [body[i], body[i + 1]];
        match selector {
            // A closed or open subpath's length record.
            0 | 3 => subpaths.push(Record {
                knots: Vec::new(),
                closed: selector == 0,
                operation: i16::from_be_bytes(word(2)),
                even_odd: u16::from_be_bytes(word(4)) != 2,
            }),
            // A knot: three points, each y then x as signed 8.24 fixed-point
            // fractions of the document's height and width.
            1 | 2 | 4 | 5 => {
                let fixed = |i: usize| {
                    let bytes = [
                        body[4 * i],
                        body[4 * i + 1],
                        body[4 * i + 2],
                        body[4 * i + 3],
                    ];
                    f64::from(i32::from_be_bytes(bytes)) / 16_777_216.
                };
                let point = |i: usize| [fixed(2 * i + 1) * width, fixed(2 * i) * height];
                let record = subpaths.last_mut().ok_or(DAMAGED_MASK)?;
                record.knots.push([point(0), point(1), point(2)]);
            }
            // The path fill rule record and the clipboard record.
            6 | 7 => {}
            // The initial fill rule record: 1 when the fill starts with all
            // pixels.
            8 => starts_full = u16::from_be_bytes(word(0)) != 0,
            _ => return Err(DAMAGED_MASK.into()),
        }
    }
    if subpaths.iter().any(|record| record.operation == 3) {
        approximations.intersections = true;
    }
    // Photoshop's mask starts empty, or full when its first subpath is
    // subtracted or it has none and the initial fill rule says so (as
    // psd-tools draws it).
    let full = match subpaths.first() {
        Some(record) => record.operation == 2,
        None => starts_full,
    };
    let inverted = flags & 1 != 0;
    let even_odd = inverted
        || full
        || subpaths
            .iter()
            .any(|record| record.even_odd || matches!(record.operation, 0 | 2));
    let paths = subpaths
        .iter()
        .filter_map(|record| subpath(&record.knots, record.closed))
        .collect();
    Ok(Some(Outline {
        page: inverted != full,
        paths,
        even_odd,
    }))
}

/// The subpath through `knots`; `None` for none, or for one open knot.
fn subpath(knots: &[[Point; 3]], closed: bool) -> Option<Subpath> {
    let (first, rest) = knots.split_first()?;
    if rest.is_empty() && !closed {
        return None;
    }
    let mut curves = Vec::with_capacity(knots.len());
    let mut before = first;
    for knot in rest {
        curves.push([before[2], knot[0], knot[1]]);
        before = knot;
    }
    if closed {
        curves.push([before[2], first[0], first[1]]);
    }
    Some(Subpath {
        start: first[1],
        curves,
        closed,
    })
}

/// The stroke a `vstk` descriptor describes, or `None` when it has no width.
fn read_stroke(d: &Descriptor, page: Page, approximations: &mut Approximations) -> Option<Stroke> {
    let (unit, value) = d.unit("strokeStyleLineWidth").unwrap_or((*b"#Pxl", 1.));
    let resolution = d
        .number("strokeStyleResolution")
        .or(page.ppi)
        .filter(|ppi| *ppi > 0. && ppi.is_finite())
        .unwrap_or(72.);
    let width = match &unit {
        b"#Pnt" => value * resolution / 72.,
        b"#Mlm" => value * resolution / 25.4,
        _ => value,
    };
    // Not a number is no width either.
    if width.partial_cmp(&0.) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    if d.enumerated("strokeStyleLineAlignment")
        .is_some_and(|alignment| alignment != "strokeStyleAlignCenter")
    {
        approximations.alignment = true;
    }
    if d.list("strokeStyleLineDashSet")
        .is_some_and(|dashes| !dashes.is_empty())
    {
        approximations.dashes = true;
    }
    Some(Stroke {
        colour: d
            .object("strokeStyleContent")
            .map_or([0; 3], |content| paint(content, approximations)),
        // No wider than the document is round.
        width: width.min(2. * (page.width + page.height) as f64),
        opacity: d
            .number("strokeStyleOpacity")
            .map_or(1., |percent| share(percent / 100.)),
        join: match d.enumerated("strokeStyleLineJoinType") {
            Some("strokeStyleRoundJoin") => "round",
            Some("strokeStyleBevelJoin") => "bevel",
            _ => "miter",
        },
        cap: match d.enumerated("strokeStyleLineCapType") {
            Some("strokeStyleRoundCap") => "round",
            Some("strokeStyleSquareCap") => "square",
            _ => "butt",
        },
    })
}

/// The one colour a fill or a stroke's content is drawn in: its colour
/// (`Clr `), a gradient's first stop (`Grad`), or mid grey for a pattern.
fn paint(content: &Descriptor, approximations: &mut Approximations) -> [u8; 3] {
    if let Some(colour) = content.object("Clr ") {
        return colour_of(colour, approximations);
    }
    if let Some(gradient) = content.object("Grad") {
        approximations.gradients = true;
        let first = gradient
            .list("Clrs")
            .into_iter()
            .flatten()
            .find_map(|stop| match stop {
                Value::Object(stop) => stop.object("Clr "),
                _ => None,
            });
        return first.map_or(GREY, |colour| colour_of(colour, approximations));
    }
    approximations.patterns = true;
    GREY
}

/// A colour descriptor as sRGB: RGB (0 to 255, or 0 to 1 in its float
/// keys), HSB, grayscale (a percentage of black ink), CMYK (percentages of
/// ink, converted naively as the composite's are) or Lab; anything else
/// (a book colour) is mid grey.
fn colour_of(c: &Descriptor, approximations: &mut Approximations) -> [u8; 3] {
    let n = |key: &str| c.number(key);
    let found = match c.class.as_str() {
        "RGBC" => match (n("Rd  "), n("Grn "), n("Bl  ")) {
            (Some(r), Some(g), Some(b)) => Some([r, g, b]),
            _ => match (n("redFloat"), n("greenFloat"), n("blueFloat")) {
                (Some(r), Some(g), Some(b)) => Some([r * 255., g * 255., b * 255.]),
                _ => None,
            },
        },
        "HSBC" => match (n("H   "), n("Strt"), n("Brgh")) {
            (Some(h), Some(s), Some(b)) => Some(hsb(h, s / 100., b / 100.)),
            _ => None,
        },
        "Grsc" => n("Gry ").map(|k| [255. * (1. - share(k / 100.)); 3]),
        "CMYC" => match (n("Cyn "), n("Mgnt"), n("Ylw "), n("Blck")) {
            (Some(c), Some(m), Some(y), Some(k)) => {
                let light = |ink: f64| 255. * (1. - share(ink / 100.)) * (1. - share(k / 100.));
                Some([light(c), light(m), light(y)])
            }
            _ => None,
        },
        "LbCl" => match (n("Lmnc"), n("A   "), n("B   ")) {
            (Some(l), Some(a), Some(b)) => {
                Some(super::lab(byte(l * 2.55), byte(a + 128.), byte(b + 128.)).map(f64::from))
            }
            _ => None,
        },
        _ => None,
    };
    match found {
        Some(rgb) => rgb.map(byte),
        None => {
            approximations.colours = true;
            GREY
        }
    }
}

/// Hue (degrees), saturation and brightness (0 to 1) as RGB from 0 to 255.
fn hsb(hue: f64, saturation: f64, brightness: f64) -> [f64; 3] {
    let h = hue.rem_euclid(360.) / 60.;
    let v = share(brightness) * 255.;
    let c = v * share(saturation);
    let x = c * (1. - (h % 2. - 1.).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// A share from 0 to 1; not a number is none.
fn share(v: f64) -> f64 {
    if v > 0. {
        v.min(1.)
    } else {
        0.
    }
}

/// A value from 0 to 255 as a byte; not a number is 0.
fn byte(v: f64) -> u8 {
    if v > 0. {
        (v.min(255.) + 0.5) as u8
    } else {
        0
    }
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// A number as short as it reads the same to a thousandth.
fn num(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" || text.is_empty() {
        "0".into()
    } else {
        text.into()
    }
}

/// The pixels from `lo` to `hi` within `from..to`; not a number is `from`.
fn range(lo: f64, hi: f64, from: usize, to: usize) -> std::ops::Range<usize> {
    let clamp = |v: f64| {
        if v > from as f64 {
            v.min(to as f64) as usize
        } else {
            from
        }
    };
    clamp(lo.floor())..clamp(hi.ceil())
}

/// Takes `n` from the work left, refusing the shape when it runs out.
fn charge(work: &mut u64, n: u64) -> Result<(), String> {
    *work = work.checked_sub(n).ok_or(DETAILED)?;
    Ok(())
}

/// One edge of a filled outline: from its top to its bottom, where it
/// crosses its top and how far it moves across a pixel down, and whether it
/// goes down (1) or up (-1).
struct Edge {
    top: f64,
    bottom: f64,
    x: f64,
    slope: f64,
    direction: i32,
}

/// How much of each pixel of `window` the closed `outlines` cover by the
/// nonzero or even-odd rule: each pixel row sampled `SAMPLES` times, every
/// sample row's spans added exactly across the row.
fn cover(
    outlines: &[Vec<Point>],
    even_odd: bool,
    window: Window,
    work: &mut u64,
) -> Result<Vec<f32>, String> {
    let (y0, y1) = (window.y0 as f64, window.y1 as f64);
    let mut edges: Vec<Edge> = outlines
        .iter()
        .flat_map(|points| points.windows(2))
        .filter_map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let (direction, top, bottom) = match a[1].partial_cmp(&b[1])? {
                std::cmp::Ordering::Less => (1, a, b),
                std::cmp::Ordering::Greater => (-1, b, a),
                std::cmp::Ordering::Equal => return None,
            };
            (bottom[1] > y0 && top[1] < y1).then(|| Edge {
                top: top[1],
                bottom: bottom[1],
                x: top[0],
                slope: (bottom[0] - top[0]) / (bottom[1] - top[1]),
                direction,
            })
        })
        .collect();
    edges.sort_by(|a, b| a.top.total_cmp(&b.top));
    let rows: f64 = edges
        .iter()
        .map(|e| (e.bottom.min(y1) - e.top.max(y0)) * SAMPLES as f64 + 1.)
        .sum();
    charge(work, rows as u64 + window.area() as u64)?;
    let width = window.width();
    let inside = |winding: i32| {
        if even_odd {
            winding % 2 != 0
        } else {
            winding != 0
        }
    };
    let mut coverage = vec![0f32; window.area()];
    // Per pixel row: the partly covered share of each pixel, and the steps
    // of the count of wholly covered sample rows.
    let (mut area, mut steps) = (vec![0f32; width + 1], vec![0f32; width + 1]);
    let (mut active, mut next, mut crossings) = (Vec::new(), 0, Vec::new());
    for (row, py) in (window.y0..window.y1).enumerate() {
        area.fill(0.);
        steps.fill(0.);
        for s in 0..SAMPLES {
            let y = py as f64 + (s as f64 + 0.5) / SAMPLES as f64;
            while next < edges.len() && edges[next].top <= y {
                active.push(next);
                next += 1;
            }
            active.retain(|&i| edges[i].bottom > y);
            crossings.clear();
            crossings.extend(active.iter().map(|&i| {
                let e = &edges[i];
                (e.x + (y - e.top) * e.slope - window.x0 as f64, e.direction)
            }));
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (mut winding, mut start) = (0, 0.);
            for &(x, direction) in &crossings {
                let was = inside(winding);
                winding += direction;
                match (was, inside(winding)) {
                    (false, true) => start = x,
                    (true, false) => span(&mut area, &mut steps, start, x),
                    _ => {}
                }
            }
        }
        let mut whole = 0.;
        for (x, value) in coverage[row * width..][..width].iter_mut().enumerate() {
            whole += steps[x];
            *value = ((whole + area[x]) / SAMPLES as f32).clamp(0., 1.);
        }
    }
    Ok(coverage)
}

/// Adds the span of one sample row from `a` to `b` (pixels from the
/// window's left edge) to a pixel row's `area` and `steps`.
fn span(area: &mut [f32], steps: &mut [f32], a: f64, b: f64) {
    let end = (area.len() - 1) as f64;
    let (a, b) = (a.clamp(0., end), b.clamp(0., end));
    if b.partial_cmp(&a) != Some(std::cmp::Ordering::Greater) {
        return;
    }
    let (ia, ib) = (a.floor() as usize, b.floor() as usize);
    if ia == ib {
        area[ia] += (b - a) as f32;
        return;
    }
    area[ia] += (ia as f64 + 1. - a) as f32;
    steps[ia + 1] += 1.;
    steps[ib] -= 1.;
    area[ib] += (b - ib as f64) as f32;
}

/// Whether a layer's effects show: the master switch on and one effect
/// enabled. Effects that cannot be read are taken to show.
fn effects_shown(data: &[u8]) -> bool {
    fn enabled(d: &Descriptor) -> bool {
        d.items.iter().any(|(key, value)| match value {
            Value::Bool(on) => *on && key == "enab",
            Value::Object(inner) => enabled(inner),
            Value::List(items) => items
                .iter()
                .any(|item| matches!(item, Value::Object(inner) if enabled(inner))),
            _ => false,
        })
    }
    // A version (0), then the descriptor's own.
    match descriptor::versioned(data.get(4..).unwrap_or_default()) {
        Ok(effects) => effects.bool("masterFXSwitch") != Some(false) && enabled(&effects),
        Err(_) => true,
    }
}

/// What a document held that its flat paths leave out, counted.
#[derive(Default)]
struct LeftOut {
    rasters: usize,
    text: bool,
    adjustments: bool,
    effects: bool,
    clipped: bool,
    masks: bool,
    blends: bool,
    approximations: Approximations,
}

impl LeftOut {
    fn notes(&self) -> Vec<String> {
        let a = &self.approximations;
        let mut notes = Vec::new();
        match self.rasters {
            0 => {}
            1 => notes.push("1 raster layer".to_owned()),
            n => notes.push(format!("{n} raster layers")),
        }
        for (left_out, note) in [
            (self.text, "text layers"),
            (self.adjustments, "adjustment layers"),
            (self.effects, "layer effects"),
            (self.clipped, "clipped layers"),
            (self.masks, "layer masks"),
            (a.gradients, "gradients drawn in one colour"),
            (a.patterns, "patterns drawn in grey"),
            (a.colours, "book colours drawn in grey"),
            (
                a.intersections,
                "shape intersections drawn as combined shapes",
            ),
            (
                a.alignment,
                "strokes inside or outside their paths drawn centred",
            ),
            (a.dashes, "dashed strokes drawn solid"),
            (self.blends, "blend modes drawn as normal"),
        ] {
            if left_out {
                notes.push(note.to_owned());
            }
        }
        notes
    }
}

/// The visible shape and fill layers of a document on `page`, bottom first,
/// as the flat paths `import::Imported` holds, at 72 points an inch (0.75
/// points a pixel without a resolution); `None` when no visible shape layer
/// draws anything. Clipped layers are left out, since flat paths cannot
/// clip.
pub(super) fn artwork(page: Page, layers: &[Layer]) -> Result<Option<Imported>, String> {
    let scale = page
        .ppi
        .filter(|ppi| *ppi > 0. && ppi.is_finite())
        .map_or(0.75, |ppi| 72. / ppi);
    let mut body = String::new();
    let mut left_out = LeftOut::default();
    let mut shapes = 0;
    for (layer, shown) in layers.iter().zip(group_opacities(layers)) {
        let Some(group) = shown else { continue };
        if layer.section != 0 {
            if matches!(layer.section, 1 | 2) && !matches!(&layer.blend, b"pass" | b"norm") {
                left_out.blends = true;
            }
            continue;
        }
        let Some(shape) = Shape::read(&layer.vector, page)? else {
            let pixels = layer
                .channels
                .iter()
                .any(|(id, data)| *id >= 0 && data.len() > 2);
            if layer.vector.text {
                left_out.text = true;
            } else if layer.adjustment {
                left_out.adjustments = true;
            } else if pixels && layer.rect.width() > 0 && layer.rect.height() > 0 {
                left_out.rasters += 1;
            }
            continue;
        };
        if layer.clipped {
            left_out.clipped = true;
            continue;
        }
        let a = shape.approximations;
        let noted = &mut left_out.approximations;
        noted.gradients |= a.gradients;
        noted.patterns |= a.patterns;
        noted.colours |= a.colours;
        noted.intersections |= a.intersections;
        noted.alignment |= a.alignment;
        noted.dashes |= a.dashes;
        left_out.effects |= layer.vector.effects.is_some_and(effects_shown);
        left_out.masks |= layer.mask.is_some_and(|mask| !mask.from_vector)
            && layer
                .channels
                .iter()
                .any(|(id, data)| *id == -2 && data.len() > 2);
        left_out.blends |= layer.blend != *b"norm";
        let opacity =
            f64::from(group) * f64::from(layer.opacity) / 255. * f64::from(layer.fill) / 255.;
        if shape.write(page, scale, opacity, &mut body) && shape.outlined {
            shapes += 1;
        }
    }
    if shapes == 0 {
        return Ok(None);
    }
    let (width, height) = (page.width as f64 * scale, page.height as f64 * scale);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}pt\" height=\"{h}pt\" \
         viewBox=\"0 0 {w} {h}\">\n",
        w = num(width),
        h = num(height)
    );
    svg.push_str(&body);
    svg.push_str("</svg>\n");
    Ok(Some(Imported {
        svg,
        pages: 1,
        skipped: left_out.notes(),
    }))
}
