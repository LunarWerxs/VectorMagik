//! The graphics side of the interpreter: the graphics state, paths built in
//! device space (points, y up: the default user space), painting into the
//! shapes the import returns, colour spaces reduced to sRGB (CMYK by
//! PostScript's own naive formula), and matrices. Images and text are in
//! `images.rs`, Illustrator's own operators in `illustrator.rs`.
use std::rc::Rc;

use super::object::{Arr, Dict, Obj};
use super::ops::{fail, int, Entry};
use super::{Fault, Machine, Res, HAIRLINE, MAX_ART, MAX_GSAVE, MAX_PATH, TOO_BIG};

/// A point in device space.
pub(super) type Pt = (f64, f64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Seg {
    Move(Pt),
    Line(Pt),
    Curve(Pt, Pt, Pt),
    Close,
}

/// One painted path: what `svg` writes as a `path` element.
pub(super) struct Shape {
    pub(super) path: Vec<Seg>,
    /// The fill colour, and whether the even-odd rule fills it.
    pub(super) fill: Option<([u8; 3], bool)>,
    pub(super) stroke: Option<Stroke>,
}

pub(super) struct Stroke {
    pub(super) colour: [u8; 3],
    /// In points.
    pub(super) width: f64,
    pub(super) join: u8,
    pub(super) cap: u8,
}

/// A colour space, as far as reducing its colours to sRGB needs.
#[derive(Clone)]
pub(super) enum Space {
    Gray,
    Rgb,
    Cmyk,
    /// The base space, the highest index and the lookup string or procedure.
    Indexed(Box<Space>, usize, Obj),
    /// Separation and DeviceN: the number of components, the alternate
    /// space and the tint transform.
    Tint(usize, Box<Space>, Obj),
    /// Patterns, drawn in one colour; an uncoloured pattern's colour comes
    /// from its underlying space.
    Pattern(Option<Box<Space>>),
}

impl Space {
    pub(super) fn count(&self) -> usize {
        match self {
            Space::Gray | Space::Indexed(..) | Space::Pattern(_) => 1,
            Space::Rgb => 3,
            Space::Cmyk => 4,
            Space::Tint(n, ..) => *n,
        }
    }

    fn initial(&self) -> Vec<f64> {
        match self {
            Space::Gray | Space::Rgb => vec![0.; self.count()],
            Space::Cmyk => vec![0., 0., 0., 1.],
            Space::Indexed(..) => vec![0.],
            Space::Tint(n, ..) => vec![1.; *n],
            Space::Pattern(_) => Vec::new(),
        }
    }
}

/// The graphics state `gsave` keeps.
#[derive(Clone)]
pub(super) struct GState {
    pub(super) ctm: [f64; 6],
    pub(super) path: Rc<Vec<Seg>>,
    /// The start of the current subpath and the current point.
    start: Option<Pt>,
    pub(super) point: Option<Pt>,
    pub(super) rgb: [f64; 3],
    pub(super) space: Space,
    space_obj: Obj,
    comps: Vec<f64>,
    patterned: bool,
    width: f64,
    pub(super) cap: u8,
    pub(super) join: u8,
    miter: f64,
    dash: (Obj, f64),
    flat: f64,
    pub(super) font: Obj,
    null_device: bool,
    /// The path is `strokepath`'s outline of a line this wide (points):
    /// filled, it is written as that stroke.
    outline: Option<f64>,
    /// Illustrator keeps a fill and a stroke colour apart.
    pub(super) fill_rgb: [f64; 3],
    pub(super) stroke_rgb: [f64; 3],
}

impl GState {
    pub(super) fn new() -> Self {
        Self {
            ctm: IDENTITY,
            path: Rc::new(Vec::new()),
            start: None,
            point: None,
            rgb: [0.; 3],
            space: Space::Gray,
            space_obj: Obj::name("DeviceGray"),
            comps: vec![0.],
            patterned: false,
            width: 1.,
            cap: 0,
            join: 0,
            miter: 10.,
            dash: (Obj::Array(Arr::new(Vec::new(), false)), 0.),
            flat: 1.,
            font: Obj::Null,
            null_device: false,
            outline: None,
            fill_rgb: [0.; 3],
            stroke_rgb: [0.; 3],
        }
    }

    /// The objects the state holds, for taking the interpreter apart.
    pub(super) fn objects(&self) -> Vec<Obj> {
        let mut out = vec![
            self.space_obj.clone(),
            self.dash.0.clone(),
            self.font.clone(),
        ];
        match &self.space {
            Space::Indexed(_, _, lookup) => out.push(lookup.clone()),
            Space::Tint(_, _, tint) => out.push(tint.clone()),
            _ => {}
        }
        out
    }
}

pub(super) const IDENTITY: [f64; 6] = [1., 0., 0., 1., 0., 0.];

/// The matrix that applies `a` and then `b`.
pub(super) fn multiply(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn invert(m: [f64; 6]) -> Option<[f64; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0. || !det.is_finite() {
        return None;
    }
    Some([
        m[3] / det,
        -m[1] / det,
        -m[2] / det,
        m[0] / det,
        (m[2] * m[5] - m[3] * m[4]) / det,
        (m[1] * m[4] - m[0] * m[5]) / det,
    ])
}

fn apply(m: [f64; 6], (x, y): Pt) -> Pt {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

pub(super) fn apply_delta(m: [f64; 6], (x, y): Pt) -> Pt {
    (m[0] * x + m[2] * y, m[1] * x + m[3] * y)
}

pub(super) fn rgb8(rgb: [f64; 3]) -> [u8; 3] {
    rgb.map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
}

pub(super) fn cmyk_to_rgb(c: f64, m: f64, y: f64, k: f64) -> [f64; 3] {
    [
        1. - (c + k).min(1.),
        1. - (m + k).min(1.),
        1. - (y + k).min(1.),
    ]
}

fn hsb_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let h = h.clamp(0., 1.) * 6.;
    let sector = h.floor();
    let f = h - sector;
    let (p, q, t) = (v * (1. - s), v * (1. - s * f), v * (1. - s * (1. - f)));
    match sector as i32 {
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        5 => [v, p, q],
        _ => [v, t, p],
    }
}

fn rgb_to_hsb([r, g, b]: [f64; 3]) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let s = if max > 0. { delta / max } else { 0. };
    let h = if delta == 0. {
        0.
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        (b - r) / delta + 2.
    } else {
        (r - g) / delta + 4.
    };
    [h / 6., s, max]
}

/// The end points of a path: the start of its last subpath and its
/// current point.
fn ends(path: &[Seg]) -> (Option<Pt>, Option<Pt>) {
    let (mut start, mut point) = (None, None);
    for seg in path {
        match *seg {
            Seg::Move(p) => {
                start = Some(p);
                point = Some(p);
            }
            Seg::Line(p) | Seg::Curve(_, _, p) => point = Some(p),
            Seg::Close => point = start,
        }
    }
    (start, point)
}

impl Machine {
    fn device(&self, point: Pt) -> Pt {
        apply(self.gs.ctm, point)
    }

    /// The current point in user space.
    fn user_point(&self) -> Res<Pt> {
        let point = self.gs.point.ok_or(Fault::Error("nocurrentpoint"))?;
        let inverse = invert(self.gs.ctm).ok_or(Fault::Error("undefinedresult"))?;
        Ok(apply(inverse, point))
    }

    /// `x y` from the stack, in user space.
    fn pop_xy(&mut self) -> Res<Pt> {
        let y = self.pop_num()?;
        let x = self.pop_num()?;
        Ok((x, y))
    }

    pub(super) fn pop_point(&mut self) -> Res<Pt> {
        let point = self.pop_xy()?;
        Ok(self.device(point))
    }

    /// `dx dy` from the stack, added to the current point.
    fn pop_relative(&mut self) -> Res<Pt> {
        let delta = apply_delta(self.gs.ctm, self.pop_xy()?);
        let (x, y) = self.gs.point.ok_or(Fault::Error("nocurrentpoint"))?;
        Ok((x + delta.0, y + delta.1))
    }

    fn append(&mut self, seg: Seg) -> Res {
        let finite = match seg {
            Seg::Move(p) | Seg::Line(p) => p.0.is_finite() && p.1.is_finite(),
            Seg::Curve(a, b, p) => [a, b, p].iter().all(|p| p.0.is_finite() && p.1.is_finite()),
            Seg::Close => true,
        };
        if !finite {
            return fail("undefinedresult");
        }
        let path = Rc::make_mut(&mut self.gs.path);
        if path.len() >= MAX_PATH {
            return fail("limitcheck");
        }
        path.push(seg);
        Ok(())
    }

    pub(super) fn new_path(&mut self) {
        match Rc::get_mut(&mut self.gs.path) {
            Some(path) => path.clear(),
            None => self.gs.path = Rc::new(Vec::new()),
        }
        self.gs.start = None;
        self.gs.point = None;
        self.gs.outline = None;
    }

    pub(super) fn move_to(&mut self, p: Pt) -> Res {
        if let Some(Seg::Move(_)) = self.gs.path.last() {
            Rc::make_mut(&mut self.gs.path).pop();
        }
        self.append(Seg::Move(p))?;
        self.gs.start = Some(p);
        self.gs.point = Some(p);
        Ok(())
    }

    /// After a closepath, a line or curve begins a new subpath at the
    /// start of the closed one.
    fn reopen(&mut self) -> Res {
        if let (Some(Seg::Close), Some(start)) = (self.gs.path.last(), self.gs.start) {
            self.append(Seg::Move(start))?;
        }
        Ok(())
    }

    pub(super) fn line_to(&mut self, p: Pt) -> Res {
        if self.gs.point.is_none() {
            return fail("nocurrentpoint");
        }
        self.reopen()?;
        self.append(Seg::Line(p))?;
        self.gs.point = Some(p);
        Ok(())
    }

    pub(super) fn curve_to(&mut self, a: Pt, b: Pt, p: Pt) -> Res {
        if self.gs.point.is_none() {
            return fail("nocurrentpoint");
        }
        self.reopen()?;
        self.append(Seg::Curve(a, b, p))?;
        self.gs.point = Some(p);
        Ok(())
    }

    pub(super) fn close_path(&mut self) -> Res {
        if self.gs.point.is_some() && !matches!(self.gs.path.last(), Some(Seg::Close) | None) {
            self.append(Seg::Close)?;
            self.gs.point = self.gs.start;
        }
        Ok(())
    }

    /// A circular arc in user space from angle `from` to `to` (degrees),
    /// as cubic curves of at most 90 degrees each, joined to the current
    /// point by a line when there is one.
    fn arc_path(&mut self, centre: Pt, radius: f64, from: f64, to: f64, clockwise: bool) -> Res {
        let mut sweep = to - from;
        if clockwise && sweep > 0. {
            sweep = -(from - to).rem_euclid(360.);
        } else if !clockwise && sweep < 0. {
            sweep = (to - from).rem_euclid(360.);
        }
        let sweep = sweep.clamp(-3600., 3600.);
        let on = |angle: f64| {
            let (sin, cos) = angle.to_radians().sin_cos();
            (centre.0 + radius * cos, centre.1 + radius * sin)
        };
        let start = self.device(on(from));
        match self.gs.point {
            Some(point) if point == start => {}
            Some(_) => self.line_to(start)?,
            None => self.move_to(start)?,
        }
        let pieces = (sweep.abs() / 90.).ceil() as usize;
        if pieces == 0 {
            return Ok(());
        }
        let step = sweep / pieces as f64;
        let k = 4. / 3. * (step.to_radians() / 4.).tan();
        for piece in 0..pieces {
            let (s0, c0) = (from + step * piece as f64).to_radians().sin_cos();
            let (s1, c1) = (from + step * (piece + 1) as f64).to_radians().sin_cos();
            let a = (
                centre.0 + radius * (c0 - k * s0),
                centre.1 + radius * (s0 + k * c0),
            );
            let b = (
                centre.0 + radius * (c1 + k * s1),
                centre.1 + radius * (s1 - k * c1),
            );
            let end = (centre.0 + radius * c1, centre.1 + radius * s1);
            self.curve_to(self.device(a), self.device(b), self.device(end))?;
        }
        Ok(())
    }

    /// `arct`'s arc: tangent to the line from the current point to `p1`
    /// and to the line from `p1` to `p2`; answers the two tangent points.
    fn tangent_arc(&mut self, p1: Pt, p2: Pt, radius: f64) -> Res<(Pt, Pt)> {
        let p0 = self.user_point()?;
        let v1 = (p0.0 - p1.0, p0.1 - p1.1);
        let v2 = (p2.0 - p1.0, p2.1 - p1.1);
        let (l1, l2) = (v1.0.hypot(v1.1), v2.0.hypot(v2.1));
        let cross = v1.0 * v2.1 - v1.1 * v2.0;
        if l1 == 0. || l2 == 0. || radius == 0. || cross.abs() <= 1e-12 * l1 * l2 {
            self.line_to(self.device(p1))?;
            return Ok((p1, p1));
        }
        let u1 = (v1.0 / l1, v1.1 / l1);
        let u2 = (v2.0 / l2, v2.1 / l2);
        let half = (u1.0 * u2.0 + u1.1 * u2.1).clamp(-1., 1.).acos() / 2.;
        let reach = radius.abs() / half.tan();
        let t1 = (p1.0 + u1.0 * reach, p1.1 + u1.1 * reach);
        let t2 = (p1.0 + u2.0 * reach, p1.1 + u2.1 * reach);
        let bisector = (u1.0 + u2.0, u1.1 + u2.1);
        let length = bisector.0.hypot(bisector.1);
        let out = radius.abs() / half.sin() / length;
        let centre = (p1.0 + bisector.0 * out, p1.1 + bisector.1 * out);
        let from = (t1.1 - centre.1).atan2(t1.0 - centre.0).to_degrees();
        let to = (t2.1 - centre.1).atan2(t2.0 - centre.0).to_degrees();
        let turn = (p1.0 - p0.0) * (p2.1 - p1.1) - (p1.1 - p0.1) * (p2.0 - p1.0);
        self.arc_path(centre, radius.abs(), from, to, turn < 0.)?;
        Ok((t1, t2))
    }

    /// The line width in points: the user-space width scaled by the CTM's
    /// mean scale (the square root of its determinant), a zero width
    /// (PostScript's thinnest line) drawn a quarter point wide.
    pub(super) fn stroke_width(&self, ctm: [f64; 6]) -> f64 {
        let width = self.gs.width * (ctm[0] * ctm[3] - ctm[1] * ctm[2]).abs().sqrt();
        if width > 0. {
            width
        } else {
            HAIRLINE
        }
    }

    pub(super) fn take_path(&mut self) -> Vec<Seg> {
        let path = std::mem::take(&mut self.gs.path);
        let path = Rc::try_unwrap(path).unwrap_or_else(|shared| (*shared).clone());
        self.new_path();
        path
    }

    fn fill_path(&mut self, evenodd: bool) -> Res {
        let outline = self.gs.outline;
        let colour = rgb8(self.gs.rgb);
        let path = self.take_path();
        let shape = match outline {
            Some(width) => Shape {
                path,
                fill: None,
                stroke: Some(Stroke {
                    colour,
                    width,
                    join: self.gs.join,
                    cap: self.gs.cap,
                }),
            },
            None => Shape {
                path,
                fill: Some((colour, evenodd)),
                stroke: None,
            },
        };
        self.emit(shape)
    }

    fn stroke_path(&mut self, ctm: [f64; 6]) -> Res {
        if matches!(&self.gs.dash.0, Obj::Array(a) if a.len > 0) {
            self.art.dashed = true;
        }
        let stroke = Stroke {
            colour: rgb8(self.gs.rgb),
            width: self.stroke_width(ctm),
            join: self.gs.join,
            cap: self.gs.cap,
        };
        let path = self.take_path();
        self.emit(Shape {
            path,
            fill: None,
            stroke: Some(stroke),
        })
    }

    /// Adds a painted shape to the artwork; a stroke of the path just
    /// filled (`gsave fill grestore stroke`) joins that fill's element.
    pub(super) fn emit(&mut self, shape: Shape) -> Res {
        let draws = shape
            .path
            .iter()
            .any(|seg| matches!(seg, Seg::Line(_) | Seg::Curve(..)));
        if self.gs.null_device || !draws {
            return Ok(());
        }
        if self.gs.patterned {
            self.art.patterns = true;
        }
        if shape.fill.is_none() {
            if let Some(last) = self.art.shapes.last_mut() {
                if last.stroke.is_none() && last.fill.is_some() && last.path == shape.path {
                    last.stroke = shape.stroke;
                    return Ok(());
                }
            }
        }
        self.art.segments += shape.path.len();
        if self.art.segments > MAX_ART {
            return Err(Fault::Limit(TOO_BIG));
        }
        self.art.shapes.push(shape);
        Ok(())
    }

    pub(super) fn push_state(&mut self, save: Option<u32>) -> Res {
        if self.gstack.len() >= MAX_GSAVE {
            return fail("limitcheck");
        }
        self.saved_segments += self.gs.path.len();
        if self.saved_segments > MAX_ART {
            return Err(Fault::Limit(TOO_BIG));
        }
        self.gstack.push((self.gs.clone(), save));
        Ok(())
    }

    pub(super) fn pop_state(&mut self) -> Option<(GState, Option<u32>)> {
        let entry = self.gstack.pop()?;
        self.saved_segments = self.saved_segments.saturating_sub(entry.0.path.len());
        Some(entry)
    }

    fn set_space(&mut self, space: Space, name: Obj) {
        self.gs.space = space;
        self.gs.space_obj = name;
        self.gs.patterned = false;
    }

    /// Sets the current colour to `comps` in the current colour space.
    fn set_colour(&mut self, comps: Vec<f64>) -> Res {
        let space = self.gs.space.clone();
        self.gs.rgb = self.rgb_of(&space, &comps)?;
        self.gs.comps = comps;
        Ok(())
    }

    pub(super) fn pop_numbers(&mut self, count: usize) -> Res<Vec<f64>> {
        if self.stack.len() < count {
            return fail("stackunderflow");
        }
        let items = self.stack.split_off(self.stack.len() - count);
        items
            .iter()
            .map(|item| item.num().ok_or(Fault::Error("typecheck")))
            .collect()
    }

    fn rgb_of(&mut self, space: &Space, comps: &[f64]) -> Res<[f64; 3]> {
        let c = |i: usize| comps.get(i).copied().unwrap_or(0.).clamp(0., 1.);
        Ok(match space {
            Space::Gray => [c(0); 3],
            Space::Rgb => [c(0), c(1), c(2)],
            Space::Cmyk => cmyk_to_rgb(c(0), c(1), c(2), c(3)),
            Space::Indexed(base, high, lookup) => {
                let index = comps.first().copied().unwrap_or(0.).round();
                let index = index.clamp(0., *high as f64) as usize;
                let n = base.count();
                let values = match lookup {
                    Obj::Str(table) => (0..n)
                        .map(|i| {
                            let at = index * n + i;
                            if at < table.len {
                                f64::from(table.get(at)) / 255.
                            } else {
                                0.
                            }
                        })
                        .collect(),
                    procedure => {
                        self.push(int(index));
                        self.exec(procedure.clone())?;
                        self.pop_numbers(n)?
                    }
                };
                self.rgb_of(base, &values)?
            }
            Space::Tint(n, alternate, transform) => {
                for i in 0..*n {
                    self.push(Obj::Real(comps.get(i).copied().unwrap_or(0.)));
                }
                self.exec(transform.clone())?;
                let values = self.pop_numbers(alternate.count())?;
                self.rgb_of(alternate, &values)?
            }
            Space::Pattern(_) => [0.5; 3],
        })
    }

    /// A colour space from its name or array; the base of an indexed,
    /// separation or pattern space must be a device (or CIE) space.
    fn parse_space(&mut self, obj: &Obj, nested: bool) -> Res<Space> {
        let (family, parts) = match obj {
            Obj::Name(name, _) => (name.to_vec(), None),
            Obj::Array(a) if a.len > 0 => {
                (a.get(0).text().ok_or(Fault::Error("typecheck"))?, Some(a))
            }
            _ => return fail("typecheck"),
        };
        let part = |i: usize| parts.filter(|a| i < a.len).map(|a| a.get(i));
        Ok(match family.as_slice() {
            b"DeviceGray" | b"CIEBasedA" | b"CalGray" | b"G" => Space::Gray,
            b"DeviceRGB" | b"CIEBasedABC" | b"CIEBasedDEF" | b"CalRGB" | b"Lab" | b"RGB" => {
                Space::Rgb
            }
            b"DeviceCMYK" | b"CIEBasedDEFG" | b"CMYK" => Space::Cmyk,
            b"ICCBased" => {
                let count = match part(1) {
                    Some(Obj::Dict(d)) => d.find(b"N").and_then(|n| n.num()).unwrap_or(3.),
                    _ => 3.,
                };
                match count as i32 {
                    1 => Space::Gray,
                    4 => Space::Cmyk,
                    _ => Space::Rgb,
                }
            }
            b"Indexed" | b"I" if !nested => {
                let base = self.parse_space(&part(1).ok_or(Fault::Error("rangecheck"))?, true)?;
                let high = part(2)
                    .and_then(|h| h.num())
                    .ok_or(Fault::Error("rangecheck"))?;
                let lookup = part(3).ok_or(Fault::Error("rangecheck"))?;
                Space::Indexed(Box::new(base), high.clamp(0., 4095.) as usize, lookup)
            }
            b"Separation" | b"DeviceN" if !nested => {
                let count = match (family.as_slice(), part(1)) {
                    (b"DeviceN", Some(Obj::Array(names))) => names.len.clamp(1, 32),
                    (b"DeviceN", _) => return fail("rangecheck"),
                    _ => 1,
                };
                let alternate =
                    self.parse_space(&part(2).ok_or(Fault::Error("rangecheck"))?, true)?;
                let transform = part(3).ok_or(Fault::Error("rangecheck"))?;
                Space::Tint(count, Box::new(alternate), transform)
            }
            b"Pattern" if !nested => match part(1) {
                Some(base) => Space::Pattern(Some(Box::new(self.parse_space(&base, true)?))),
                None => Space::Pattern(None),
            },
            _ => return fail("undefined"),
        })
    }

    /// A pattern as the current colour: an uncoloured pattern takes its
    /// colour from the components below it, a coloured one is drawn grey.
    fn set_pattern(&mut self, pattern: &Dict, base: Option<&Space>) -> Res {
        let uncoloured = matches!(pattern.find(b"PaintType"), Some(Obj::Int(2)));
        self.gs.rgb = match (uncoloured, base) {
            (true, Some(base)) => {
                let comps = self.pop_numbers(base.count())?;
                self.rgb_of(base, &comps)?
            }
            _ => [0.5; 3],
        };
        self.gs.patterned = true;
        Ok(())
    }
}

/// The six numbers of a matrix array.
pub(super) fn matrix_of(obj: &Obj) -> Option<[f64; 6]> {
    match obj {
        Obj::Array(a) if a.len == 6 => a.numbers().and_then(|v| v.try_into().ok()),
        _ => None,
    }
}

impl Machine {
    pub(super) fn pop_matrix(&mut self) -> Res<(Arr, [f64; 6])> {
        let array = self.pop_arr()?;
        let matrix = matrix_of(&Obj::Array(array.clone())).ok_or(Fault::Error("rangecheck"))?;
        Ok((array, matrix))
    }

    /// Whether a matrix array is on top of the stack (the matrix forms of
    /// translate, scale, rotate and the transforms).
    fn matrix_on_top(&self) -> bool {
        matches!(self.stack.last(), Some(Obj::Array(a)) if a.len == 6)
    }

    fn answer_matrix(&mut self, array: Arr, matrix: [f64; 6]) -> Res {
        if array.len != 6 {
            return fail("rangecheck");
        }
        for (i, v) in matrix.iter().enumerate() {
            array.set(i, Obj::Real(*v));
        }
        self.answer(Obj::Array(array))
    }

    pub(super) fn answer_point(&mut self, (x, y): Pt) -> Res {
        if !x.is_finite() || !y.is_finite() {
            return fail("undefinedresult");
        }
        self.push(Obj::Real(x));
        self.answer(Obj::Real(y))
    }

    /// `translate`, `scale` and `rotate`: `m` applied to the CTM, or, with
    /// a matrix operand, written into it.
    fn transform_by(&mut self, m: [f64; 6], into: Option<Arr>) -> Res {
        match into {
            Some(array) => self.answer_matrix(array, m),
            None => {
                self.gs.ctm = multiply(m, self.gs.ctm);
                Ok(())
            }
        }
    }

    fn pop_optional_matrix(&mut self) -> Res<Option<Arr>> {
        Ok(if self.matrix_on_top() {
            Some(self.pop_arr()?)
        } else {
            None
        })
    }

    /// `transform` and its kin: a point (or distance, with `delta`) through
    /// the CTM or a given matrix, or back (`inverse`).
    fn map_point(&mut self, delta: bool, inverse: bool) -> Res {
        let matrix = match self.pop_optional_matrix()? {
            Some(array) => matrix_of(&Obj::Array(array)).ok_or(Fault::Error("typecheck"))?,
            None => self.gs.ctm,
        };
        let matrix = if inverse {
            invert(matrix).ok_or(Fault::Error("undefinedresult"))?
        } else {
            matrix
        };
        let point = self.pop_xy()?;
        let mapped = if delta {
            apply_delta(matrix, point)
        } else {
            apply(matrix, point)
        };
        self.answer_point(mapped)
    }

    /// Rectangles from `x y w h` or an array of them.
    fn pop_rects(&mut self) -> Res<Vec<[f64; 4]>> {
        if let Some(Obj::Array(_)) = self.stack.last() {
            let values = self.pop_arr()?.numbers().ok_or(Fault::Error("typecheck"))?;
            if !values.len().is_multiple_of(4) {
                return fail("rangecheck");
            }
            return Ok(values.chunks(4).map(|r| [r[0], r[1], r[2], r[3]]).collect());
        }
        let h = self.pop_num()?;
        let w = self.pop_num()?;
        let (x, y) = self.pop_xy()?;
        Ok(vec![[x, y, w, h]])
    }

    fn rect_path(&self, rects: &[[f64; 4]]) -> Vec<Seg> {
        let mut path = Vec::with_capacity(rects.len() * 5);
        for [x, y, w, h] in rects {
            path.push(Seg::Move(self.device((*x, *y))));
            path.push(Seg::Line(self.device((x + w, *y))));
            path.push(Seg::Line(self.device((x + w, y + h))));
            path.push(Seg::Line(self.device((*x, y + h))));
            path.push(Seg::Close);
        }
        path
    }

    fn stroke_rects(&mut self) -> Res {
        let extra = if self.matrix_on_top() && self.stack.len() >= 2 {
            let (_, matrix) = self.pop_matrix()?;
            Some(matrix)
        } else {
            None
        };
        let rects = self.pop_rects()?;
        let path = self.rect_path(&rects);
        let ctm = extra.map_or(self.gs.ctm, |m| multiply(m, self.gs.ctm));
        let saved = std::mem::replace(&mut self.gs.path, Rc::new(path));
        let (start, point, outline) = (self.gs.start, self.gs.point, self.gs.outline.take());
        let result = self.stroke_path(ctm);
        self.gs.path = saved;
        (self.gs.start, self.gs.point, self.gs.outline) = (start, point, outline);
        result
    }
}

/// The path with its curves as lines no further than about `flat` from
/// them; `None` when that would pass the size a path may have.
fn flatten(path: &[Seg], flat: f64) -> Option<Vec<Seg>> {
    let mut out = Vec::with_capacity(path.len());
    let (mut start, mut current) = ((0., 0.), (0., 0.));
    for seg in path {
        if out.len() > MAX_PATH {
            return None;
        }
        match *seg {
            Seg::Move(p) => {
                start = p;
                current = p;
                out.push(*seg);
            }
            Seg::Line(p) => {
                current = p;
                out.push(*seg);
            }
            Seg::Curve(a, b, p) => {
                let bend = |u: Pt, v: Pt, w: Pt| (u.0 - 2. * v.0 + w.0).hypot(u.1 - 2. * v.1 + w.1);
                let depth = bend(current, a, b).max(bend(a, b, p));
                let pieces = (0.75 * depth / flat.max(0.01))
                    .sqrt()
                    .ceil()
                    .clamp(1., 100.) as usize;
                for piece in 1..=pieces {
                    let t = piece as f64 / pieces as f64;
                    let s = 1. - t;
                    let (w0, w1, w2, w3) = (s * s * s, 3. * s * s * t, 3. * s * t * t, t * t * t);
                    out.push(Seg::Line((
                        w0 * current.0 + w1 * a.0 + w2 * b.0 + w3 * p.0,
                        w0 * current.1 + w1 * a.1 + w2 * b.1 + w3 * p.1,
                    )));
                }
                current = p;
            }
            Seg::Close => {
                current = start;
                out.push(Seg::Close);
            }
        }
    }
    Some(out)
}

fn reverse(path: &[Seg]) -> Vec<Seg> {
    let mut out = Vec::with_capacity(path.len());
    let mut i = 0;
    while i < path.len() {
        let Seg::Move(first) = path[i] else {
            i += 1;
            continue;
        };
        let mut j = i + 1;
        while j < path.len() && !matches!(path[j], Seg::Move(_)) {
            j += 1;
        }
        let body = &path[i + 1..j];
        let closed = matches!(body.last(), Some(Seg::Close));
        let segs: Vec<Seg> = body.iter().copied().filter(|s| *s != Seg::Close).collect();
        let mut starts = Vec::with_capacity(segs.len());
        let mut current = first;
        for seg in &segs {
            starts.push(current);
            if let Seg::Line(p) | Seg::Curve(_, _, p) = seg {
                current = *p;
            }
        }
        out.push(Seg::Move(current));
        for (seg, from) in segs.iter().zip(starts).rev() {
            out.push(match *seg {
                Seg::Curve(a, b, _) => Seg::Curve(b, a, from),
                _ => Seg::Line(from),
            });
        }
        if closed {
            out.push(Seg::Close);
        }
        i = j;
    }
    out
}

pub(super) fn pop_drop(m: &mut Machine, count: usize) -> Res {
    m.drop_n(count)
}

fn set_space_op(m: &mut Machine) -> Res {
    let obj = m.pop()?;
    let space = m.parse_space(&obj, false)?;
    let initial = space.initial();
    let pattern = matches!(space, Space::Pattern(_));
    m.set_space(space, obj);
    if pattern {
        m.gs.rgb = [0.5; 3];
        m.gs.patterned = true;
        Ok(())
    } else {
        m.set_colour(initial)
    }
}

fn set_colour_op(m: &mut Machine) -> Res {
    if let Space::Pattern(base) = m.gs.space.clone() {
        let pattern = m.pop_dict()?;
        return m.set_pattern(&pattern, base.as_deref());
    }
    let count = m.gs.space.count();
    let comps = m.pop_numbers(count)?;
    m.set_colour(comps)
}

fn set_pattern_op(m: &mut Machine) -> Res {
    let pattern = m.pop_dict()?;
    let uncoloured = matches!(pattern.find(b"PaintType"), Some(Obj::Int(2)));
    let base = match &m.gs.space {
        Space::Pattern(base) => base.as_deref().cloned(),
        other if uncoloured => Some(other.clone()),
        _ => None,
    };
    m.gs.space = Space::Pattern(base.clone().map(Box::new));
    m.gs.space_obj = Obj::name("Pattern");
    m.set_pattern(&pattern, base.as_ref())
}

fn current_colour_space(m: &mut Machine) -> Res {
    let space = match &m.gs.space_obj {
        array @ Obj::Array(_) => array.clone(),
        name => Obj::Array(Arr::new(vec![name.clone()], false)),
    };
    m.answer(space)
}

fn current_cmyk(m: &mut Machine) -> Res {
    let values = match m.gs.space {
        Space::Cmyk => m.gs.comps.clone(),
        _ => {
            let [r, g, b] = m.gs.rgb;
            vec![1. - r, 1. - g, 1. - b, 0.]
        }
    };
    for v in values {
        m.push(Obj::Real(v));
    }
    Ok(())
}

fn path_bbox(m: &mut Machine) -> Res {
    let current = m.gs.point.ok_or(Fault::Error("nocurrentpoint"))?;
    let mut points = vec![current];
    for seg in m.gs.path.iter() {
        match *seg {
            Seg::Move(p) | Seg::Line(p) => points.push(p),
            Seg::Curve(a, b, p) => points.extend([a, b, p]),
            Seg::Close => {}
        }
    }
    let inverse = invert(m.gs.ctm).ok_or(Fault::Error("undefinedresult"))?;
    let bounds = |pts: &[Pt]| {
        pts.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)],
        )
    };
    let [x0, y0, x1, y1] = bounds(&points);
    let corners: Vec<Pt> = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        .iter()
        .map(|p| apply(inverse, *p))
        .collect();
    for v in bounds(&corners) {
        m.push(Obj::Real(v));
    }
    Ok(())
}

fn path_forall(m: &mut Machine) -> Res {
    let close = m.pop()?;
    let curve = m.pop()?;
    let line = m.pop()?;
    let start = m.pop()?;
    let inverse = invert(m.gs.ctm).ok_or(Fault::Error("undefinedresult"))?;
    let path = m.gs.path.clone();
    for seg in path.iter() {
        let (points, procedure) = match *seg {
            Seg::Move(p) => (vec![p], &start),
            Seg::Line(p) => (vec![p], &line),
            Seg::Curve(a, b, p) => (vec![a, b, p], &curve),
            Seg::Close => (Vec::new(), &close),
        };
        for p in points {
            let (x, y) = apply(inverse, p);
            m.push(Obj::Real(x));
            m.push(Obj::Real(y));
        }
        m.step()?;
        match m.exec(procedure.clone()) {
            Ok(()) => {}
            Err(Fault::Exit) => break,
            Err(fault) => return Err(fault),
        }
    }
    Ok(())
}

fn replace_path(m: &mut Machine, path: Vec<Seg>) -> Res {
    if path.len() > MAX_PATH {
        return fail("limitcheck");
    }
    let (start, point) = ends(&path);
    m.gs.path = Rc::new(path);
    m.gs.start = start;
    m.gs.point = point;
    Ok(())
}

fn clip_path(m: &mut Machine) -> Res {
    let [l, b, r, t] = m.page;
    m.new_path();
    let path = vec![
        Seg::Move((l, b)),
        Seg::Line((r, b)),
        Seg::Line((r, t)),
        Seg::Line((l, t)),
        Seg::Close,
    ];
    replace_path(m, path)
}

fn exec_form(m: &mut Machine) -> Res {
    let form = m.pop_dict()?;
    let paint = form.find(b"PaintProc").ok_or(Fault::Error("undefined"))?;
    let matrix = form
        .find(b"Matrix")
        .and_then(|m| matrix_of(&m))
        .unwrap_or(IDENTITY);
    m.push_state(None)?;
    m.gs.ctm = multiply(matrix, m.gs.ctm);
    m.new_path();
    m.push(Obj::Dict(form));
    let result = m.exec(paint);
    if let Some((state, _)) = m.pop_state() {
        m.gs = state;
    }
    result
}

pub(super) fn grestore(m: &mut Machine) -> Res {
    match m.gstack.last() {
        Some((state, Some(_))) => m.gs = state.clone(),
        Some((_, None)) => {
            if let Some((state, _)) = m.pop_state() {
                m.gs = state;
            }
        }
        None => {}
    }
    Ok(())
}

fn grestore_all(m: &mut Machine) -> Res {
    loop {
        match m.gstack.last() {
            Some((state, Some(_))) => {
                m.gs = state.clone();
                return Ok(());
            }
            Some((_, None)) => {
                if let Some((state, _)) = m.pop_state() {
                    m.gs = state;
                }
            }
            None => return Ok(()),
        }
    }
}

fn set_dash(m: &mut Machine) -> Res {
    let offset = m.pop_num()?;
    let array = m.pop_arr()?;
    let values = array.numbers().ok_or(Fault::Error("typecheck"))?;
    if values.iter().any(|v| *v < 0.) {
        return fail("rangecheck");
    }
    let array = if values.iter().all(|v| *v == 0.) {
        Arr::new(Vec::new(), false)
    } else {
        array
    };
    m.gs.dash = (Obj::Array(array), offset);
    Ok(())
}

fn arc_op(m: &mut Machine, clockwise: bool) -> Res {
    let to = m.pop_num()?;
    let from = m.pop_num()?;
    let radius = m.pop_num()?;
    let centre = m.pop_xy()?;
    m.arc_path(centre, radius, from, to, clockwise)
}

fn arc_to(m: &mut Machine, answer: bool) -> Res {
    let radius = m.pop_num()?;
    let p2 = m.pop_xy()?;
    let p1 = m.pop_xy()?;
    let (t1, t2) = m.tangent_arc(p1, p2, radius)?;
    if answer {
        for v in [t1.0, t1.1, t2.0, t2.1] {
            m.push(Obj::Real(v));
        }
    }
    Ok(())
}

fn set_cap_or_join(m: &mut Machine, join: bool) -> Res {
    let value = m.pop_int()?;
    if !(0..=2).contains(&value) {
        return fail("rangecheck");
    }
    if join {
        m.gs.join = value as u8;
    } else {
        m.gs.cap = value as u8;
    }
    Ok(())
}

fn set_device_colour(m: &mut Machine, space: Space, name: &str, comps: usize) -> Res {
    let values = m.pop_numbers(comps)?;
    m.set_space(space, Obj::name(name));
    m.set_colour(values)
}

/// The graphics operators: their names, what they do, and how many
/// operands an error puts back.
pub(super) static OPS: &[Entry] = &[
    // The graphics state.
    ("gsave", |m| m.push_state(None), 0),
    ("grestore", grestore, 0),
    ("grestoreall", grestore_all, 0),
    (
        "initgraphics",
        |m| {
            let font = std::mem::replace(&mut m.gs.font, Obj::Null);
            m.gs = GState::new();
            m.gs.font = font;
            Ok(())
        },
        0,
    ),
    (
        "setlinewidth",
        |m| {
            m.gs.width = m.pop_num()?.abs();
            Ok(())
        },
        1,
    ),
    ("currentlinewidth", |m| m.answer(Obj::Real(m.gs.width)), 0),
    ("setlinecap", |m| set_cap_or_join(m, false), 1),
    (
        "currentlinecap",
        |m| m.answer(Obj::Int(i32::from(m.gs.cap))),
        0,
    ),
    ("setlinejoin", |m| set_cap_or_join(m, true), 1),
    (
        "currentlinejoin",
        |m| m.answer(Obj::Int(i32::from(m.gs.join))),
        0,
    ),
    (
        "setmiterlimit",
        |m| {
            let limit = m.pop_num()?;
            if limit < 1. {
                return fail("rangecheck");
            }
            m.gs.miter = limit;
            Ok(())
        },
        1,
    ),
    ("currentmiterlimit", |m| m.answer(Obj::Real(m.gs.miter)), 0),
    ("setdash", set_dash, 2),
    (
        "currentdash",
        |m| {
            let (array, offset) = m.gs.dash.clone();
            m.push(array);
            m.answer(Obj::Real(offset))
        },
        0,
    ),
    (
        "setflat",
        |m| {
            m.gs.flat = m.pop_num()?.clamp(0.2, 100.);
            Ok(())
        },
        1,
    ),
    ("currentflat", |m| m.answer(Obj::Real(m.gs.flat)), 0),
    ("setstrokeadjust", |m| pop_drop(m, 1), 1),
    ("currentstrokeadjust", |m| m.answer(Obj::Bool(false)), 0),
    (
        "nulldevice",
        |m| {
            m.gs.null_device = true;
            Ok(())
        },
        0,
    ),
    // Colour.
    (
        "setgray",
        |m| set_device_colour(m, Space::Gray, "DeviceGray", 1),
        1,
    ),
    (
        "setrgbcolor",
        |m| set_device_colour(m, Space::Rgb, "DeviceRGB", 3),
        3,
    ),
    (
        "setcmykcolor",
        |m| set_device_colour(m, Space::Cmyk, "DeviceCMYK", 4),
        4,
    ),
    (
        "sethsbcolor",
        |m| {
            let hsb = m.pop_numbers(3)?;
            m.set_space(Space::Rgb, Obj::name("DeviceRGB"));
            m.set_colour(hsb_to_rgb(hsb[0], hsb[1], hsb[2]).to_vec())
        },
        3,
    ),
    (
        "currentgray",
        |m| {
            let gray = match m.gs.space {
                Space::Gray => m.gs.comps.first().copied().unwrap_or(0.),
                _ => 0.3 * m.gs.rgb[0] + 0.59 * m.gs.rgb[1] + 0.11 * m.gs.rgb[2],
            };
            m.answer(Obj::Real(gray))
        },
        0,
    ),
    (
        "currentrgbcolor",
        |m| {
            for v in m.gs.rgb {
                m.push(Obj::Real(v));
            }
            Ok(())
        },
        0,
    ),
    (
        "currenthsbcolor",
        |m| {
            for v in rgb_to_hsb(m.gs.rgb) {
                m.push(Obj::Real(v));
            }
            Ok(())
        },
        0,
    ),
    ("currentcmykcolor", current_cmyk, 0),
    ("setcolorspace", set_space_op, 1),
    ("currentcolorspace", current_colour_space, 0),
    ("setcolor", set_colour_op, 5),
    (
        "currentcolor",
        |m| {
            for v in m.gs.comps.clone() {
                m.push(Obj::Real(v));
            }
            Ok(())
        },
        0,
    ),
    ("setpattern", set_pattern_op, 5),
    (
        "makepattern",
        |m| {
            m.pop()?;
            let pattern = m.pop_dict()?;
            let copy = Dict::new(pattern.size() + 1);
            for (key, value) in pattern.entries() {
                copy.put(key, value).map_err(Fault::Error)?;
            }
            copy.set("Implementation", Obj::Null);
            m.answer(Obj::Dict(copy))
        },
        2,
    ),
    // Matrices.
    ("matrix", |m| m.answer(Obj::numbers(&IDENTITY)), 0),
    (
        "identmatrix",
        |m| {
            let array = m.pop_arr()?;
            m.answer_matrix(array, IDENTITY)
        },
        1,
    ),
    (
        "initmatrix",
        |m| {
            m.gs.ctm = IDENTITY;
            Ok(())
        },
        0,
    ),
    (
        "currentmatrix",
        |m| {
            let array = m.pop_arr()?;
            m.answer_matrix(array, m.gs.ctm)
        },
        1,
    ),
    (
        "defaultmatrix",
        |m| {
            let array = m.pop_arr()?;
            m.answer_matrix(array, IDENTITY)
        },
        1,
    ),
    (
        "setmatrix",
        |m| {
            m.gs.ctm = m.pop_matrix()?.1;
            Ok(())
        },
        1,
    ),
    (
        "translate",
        |m| {
            let into = m.pop_optional_matrix()?;
            let (x, y) = m.pop_xy()?;
            m.transform_by([1., 0., 0., 1., x, y], into)
        },
        3,
    ),
    (
        "scale",
        |m| {
            let into = m.pop_optional_matrix()?;
            let (x, y) = m.pop_xy()?;
            m.transform_by([x, 0., 0., y, 0., 0.], into)
        },
        3,
    ),
    (
        "rotate",
        |m| {
            let into = m.pop_optional_matrix()?;
            let (sin, cos) = m.pop_num()?.to_radians().sin_cos();
            m.transform_by([cos, sin, -sin, cos, 0., 0.], into)
        },
        2,
    ),
    (
        "concat",
        |m| {
            let (_, matrix) = m.pop_matrix()?;
            m.gs.ctm = multiply(matrix, m.gs.ctm);
            Ok(())
        },
        1,
    ),
    (
        "concatmatrix",
        |m| {
            let target = m.pop_arr()?;
            let (_, b) = m.pop_matrix()?;
            let (_, a) = m.pop_matrix()?;
            m.answer_matrix(target, multiply(a, b))
        },
        3,
    ),
    (
        "invertmatrix",
        |m| {
            let target = m.pop_arr()?;
            let (_, matrix) = m.pop_matrix()?;
            let inverse = invert(matrix).ok_or(Fault::Error("undefinedresult"))?;
            m.answer_matrix(target, inverse)
        },
        2,
    ),
    ("transform", |m| m.map_point(false, false), 3),
    ("itransform", |m| m.map_point(false, true), 3),
    ("dtransform", |m| m.map_point(true, false), 3),
    ("idtransform", |m| m.map_point(true, true), 3),
    // Paths.
    (
        "newpath",
        |m| {
            m.new_path();
            Ok(())
        },
        0,
    ),
    (
        "moveto",
        |m| {
            let p = m.pop_point()?;
            m.move_to(p)
        },
        2,
    ),
    (
        "rmoveto",
        |m| {
            let p = m.pop_relative()?;
            m.move_to(p)
        },
        2,
    ),
    (
        "lineto",
        |m| {
            let p = m.pop_point()?;
            m.line_to(p)
        },
        2,
    ),
    (
        "rlineto",
        |m| {
            let p = m.pop_relative()?;
            m.line_to(p)
        },
        2,
    ),
    (
        "curveto",
        |m| {
            let p = m.pop_point()?;
            let b = m.pop_point()?;
            let a = m.pop_point()?;
            m.curve_to(a, b, p)
        },
        6,
    ),
    (
        "rcurveto",
        |m| {
            let p = m.pop_relative()?;
            let b = m.pop_relative()?;
            let a = m.pop_relative()?;
            m.curve_to(a, b, p)
        },
        6,
    ),
    ("closepath", |m| m.close_path(), 0),
    ("arc", |m| arc_op(m, false), 5),
    ("arcn", |m| arc_op(m, true), 5),
    ("arct", |m| arc_to(m, false), 5),
    ("arcto", |m| arc_to(m, true), 5),
    (
        "currentpoint",
        |m| {
            let point = m.user_point()?;
            m.answer_point(point)
        },
        0,
    ),
    ("pathbbox", path_bbox, 0),
    (
        "flattenpath",
        |m| {
            let path = flatten(&m.gs.path, m.gs.flat).ok_or(Fault::Error("limitcheck"))?;
            replace_path(m, path)
        },
        0,
    ),
    (
        "reversepath",
        |m| {
            let path = reverse(&m.gs.path);
            replace_path(m, path)
        },
        0,
    ),
    ("pathforall", path_forall, 4),
    ("clippath", clip_path, 0),
    ("initclip", |_| Ok(()), 0),
    (
        "clip",
        |m| {
            m.art.clipped = true;
            Ok(())
        },
        0,
    ),
    (
        "eoclip",
        |m| {
            m.art.clipped = true;
            Ok(())
        },
        0,
    ),
    (
        "rectclip",
        |m| {
            m.pop_rects()?;
            m.art.clipped = true;
            m.new_path();
            Ok(())
        },
        4,
    ),
    (
        "strokepath",
        |m| {
            m.gs.outline = Some(m.stroke_width(m.gs.ctm));
            Ok(())
        },
        0,
    ),
    ("setbbox", |m| pop_drop(m, 4), 4),
    ("ucache", |_| Ok(()), 0),
    // Painting.
    ("fill", |m| m.fill_path(false), 0),
    ("eofill", |m| m.fill_path(true), 0),
    ("stroke", |m| m.stroke_path(m.gs.ctm), 0),
    (
        "rectfill",
        |m| {
            let rects = m.pop_rects()?;
            let path = m.rect_path(&rects);
            let colour = rgb8(m.gs.rgb);
            m.emit(Shape {
                path,
                fill: Some((colour, false)),
                stroke: None,
            })
        },
        4,
    ),
    ("rectstroke", |m| m.stroke_rects(), 5),
    (
        "erasepage",
        |m| {
            m.art.shapes.clear();
            m.art.segments = 0;
            Ok(())
        },
        0,
    ),
    ("showpage", |_| Err(Fault::Quit), 0),
    ("copypage", |_| Ok(()), 0),
    (
        "shfill",
        |m| {
            m.pop()?;
            m.art.shading = true;
            Ok(())
        },
        1,
    ),
    ("execform", exec_form, 1),
];
