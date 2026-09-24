//! Windows' Enhanced Metafile, written from the app's SVG documents
//! (September 24, 2026), as the original desktop saved it for Office and
//! other Windows programs.
//!
//! The drawing is read by `pdf_eps::page`, so what the PDF writer refuses is
//! refused here too. Each painted path is one GDI path: the polygon fill
//! mode of its fill rule (ALTERNATE for evenodd, WINDING for nonzero), a
//! solid brush of its fill and a geometric pen of its stroke (or the null
//! brush or pen), `EMR_BEGINPATH`, `EMR_MOVETOEX`, `EMR_LINETO` (or
//! `EMR_POLYLINETO16` for a run of lines), `EMR_POLYBEZIERTO16` for a run of
//! cubics (the 32-bit records when a coordinate is out of 16-bit range),
//! `EMR_CLOSEFIGURE`, `EMR_ENDPATH`, then `EMR_FILLPATH`,
//! `EMR_STROKEANDFILLPATH` or `EMR_STROKEPATH`. A brush or pen is made when
//! the colour changes and deleted once replaced, so the engine's colour
//! groups cost one brush each; the stock objects are selected back and the
//! last ones deleted before `EMR_EOF`.
//!
//! Units: the header describes a 1152 dpi reference device (16 pixels per
//! point) 508 by 381 mm, and the map mode is MM_ANISOTROPIC with equal
//! window and viewport extents, so one logical unit is 1/16 point (0.022
//! mm) and the header's bounds (device units) and frame (0.01 mm) are the
//! page's size.
//!
//! GDI paints without opacity, so a drawing with partial transparency is
//! refused, as EPS refuses it.
use crate::pdf_eps::{Page, Segment, Shape, Xy};

/// Logical units per point.
const UNITS_PER_POINT: f64 = 16.;
/// The largest coordinate written, in logical units: GDI's 2^27 limit.
const MAX_COORDINATE: f64 = (1u32 << 27) as f64;
/// The reference device: 1152 dpi, 23040 by 17280 pixels in 508 by 381 mm.
const DEVICE_PIXELS: [i32; 2] = [23040, 17280];
const DEVICE_MILLIMETRES: [i32; 2] = [508, 381];

const EMR_HEADER: u32 = 1;
const EMR_POLYBEZIERTO: u32 = 5;
const EMR_POLYLINETO: u32 = 6;
const EMR_SETWINDOWEXTEX: u32 = 9;
const EMR_SETWINDOWORGEX: u32 = 10;
const EMR_SETVIEWPORTEXTEX: u32 = 11;
const EMR_SETVIEWPORTORGEX: u32 = 12;
const EMR_EOF: u32 = 14;
const EMR_SETMAPMODE: u32 = 17;
const EMR_SETBKMODE: u32 = 18;
const EMR_SETPOLYFILLMODE: u32 = 19;
const EMR_MOVETOEX: u32 = 27;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_DELETEOBJECT: u32 = 40;
const EMR_LINETO: u32 = 54;
const EMR_SETMITERLIMIT: u32 = 58;
const EMR_BEGINPATH: u32 = 59;
const EMR_ENDPATH: u32 = 60;
const EMR_CLOSEFIGURE: u32 = 61;
const EMR_FILLPATH: u32 = 62;
const EMR_STROKEANDFILLPATH: u32 = 63;
const EMR_STROKEPATH: u32 = 64;
const EMR_POLYBEZIERTO16: u32 = 88;
const EMR_POLYLINETO16: u32 = 89;
const EMR_EXTCREATEPEN: u32 = 95;

const MM_ANISOTROPIC: u32 = 8;
const TRANSPARENT: u32 = 1;
const ALTERNATE: u32 = 1;
const WINDING: u32 = 2;
const BS_SOLID: u32 = 0;
const PS_GEOMETRIC: u32 = 0x0001_0000;
const WHITE_BRUSH: u32 = 0x8000_0000;
const NULL_BRUSH: u32 = 0x8000_0005;
const BLACK_PEN: u32 = 0x8000_0007;
const NULL_PEN: u32 = 0x8000_0008;
/// The EMF signature, " EMF".
const ENHMETA_SIGNATURE: u32 = 0x464D_4520;

/// The EMF of `svg` at its declared size.
pub fn to_emf(svg: &str) -> Result<Vec<u8>, String> {
    let page = crate::pdf_eps::page(svg)?;
    if page.translucent {
        return Err(
            "This image has partial transparency that EMF cannot preserve as vectors. \
             Save as SVG or PDF instead."
                .into(),
        );
    }
    let size = [logical(page.width)?.max(1), logical(page.height)?.max(1)];
    let mut emf = Emf::default();
    emf.record(EMR_SETMAPMODE, &[MM_ANISOTROPIC]);
    emf.record(EMR_SETWINDOWORGEX, &[0, 0]);
    emf.record(EMR_SETWINDOWEXTEX, &[size[0] as u32, size[1] as u32]);
    emf.record(EMR_SETVIEWPORTORGEX, &[0, 0]);
    emf.record(EMR_SETVIEWPORTEXTEX, &[size[0] as u32, size[1] as u32]);
    emf.record(EMR_SETBKMODE, &[TRANSPARENT]);
    // SVG's miter limit (GDI's own is 10).
    emf.record(EMR_SETMITERLIMIT, &[4]);
    for shape in &page.shapes {
        emf.shape(shape)?;
    }
    emf.record(EMR_SELECTOBJECT, &[WHITE_BRUSH]);
    emf.record(EMR_SELECTOBJECT, &[BLACK_PEN]);
    for made in [emf.brush.map(|b| b.0), emf.pen.map(|p| p.0)]
        .into_iter()
        .flatten()
    {
        emf.record(EMR_DELETEOBJECT, &[made]);
    }
    // No palette; the palette would start 16 bytes in; this record's size.
    emf.record(EMR_EOF, &[0, 16, 20]);
    emf.finish(&page, size)
}

/// A page coordinate in logical units, refused beyond GDI's range.
fn logical(value: f64) -> Result<i32, String> {
    let units = (value * UNITS_PER_POINT).round();
    if units.is_nan() || units.abs() > MAX_COORDINATE {
        return Err("The drawing reaches too far for an EMF".into());
    }
    Ok(units as i32)
}

fn point(p: Xy) -> Result<[i32; 2], String> {
    Ok([logical(p.0)?, logical(p.1)?])
}

/// A COLORREF: red, green and blue in the low three bytes.
fn colorref(c: [u8; 3]) -> u32 {
    u32::from(c[0]) | u32::from(c[1]) << 8 | u32::from(c[2]) << 16
}

/// What a pen draws: colour, width in logical units and its style bits.
type PenKey = ([u8; 3], u32, u32);

/// An EMF being written: its records after the header and their count,
/// and the brush and pen made last (object slot and what it draws) and
/// whether the null brush or pen is selected instead.
#[derive(Default)]
struct Emf {
    body: Vec<u8>,
    records: u32,
    fill_mode: u32,
    brush: Option<(u32, [u8; 3])>,
    null_brush: bool,
    pen: Option<(u32, PenKey)>,
    null_pen: bool,
}

impl Emf {
    /// A record of 32-bit words.
    fn record(&mut self, kind: u32, words: &[u32]) {
        self.body.extend_from_slice(&kind.to_le_bytes());
        self.body
            .extend_from_slice(&(8 + 4 * words.len() as u32).to_le_bytes());
        for word in words {
            self.body.extend_from_slice(&word.to_le_bytes());
        }
        self.records += 1;
    }

    /// A poly record: its bounds, count and points, 16-bit when they fit.
    fn poly(&mut self, short: u32, long: u32, points: &[[i32; 2]]) -> Result<(), String> {
        let fits = points.iter().flatten().all(|v| i16::try_from(*v).is_ok());
        let per_point = if fits { 4 } else { 8 };
        let size = u32::try_from(28 + per_point * points.len())
            .map_err(|_| "A path is too long for an EMF record")?;
        self.body
            .extend_from_slice(&(if fits { short } else { long }).to_le_bytes());
        self.body.extend_from_slice(&size.to_le_bytes());
        for v in bounds(points) {
            self.body.extend_from_slice(&v.to_le_bytes());
        }
        self.body
            .extend_from_slice(&(points.len() as u32).to_le_bytes());
        for v in points.iter().flatten() {
            if fits {
                self.body.extend_from_slice(&(*v as i16).to_le_bytes());
            } else {
                self.body.extend_from_slice(&v.to_le_bytes());
            }
        }
        self.records += 1;
        Ok(())
    }

    /// Selects a solid brush of `fill`, or the null brush.
    fn select_brush(&mut self, fill: Option<[u8; 3]>) {
        match (fill, self.brush) {
            (None, _) => {
                if !self.null_brush {
                    self.record(EMR_SELECTOBJECT, &[NULL_BRUSH]);
                    self.null_brush = true;
                }
            }
            (Some(c), Some((slot, made))) if made == c => {
                if self.null_brush {
                    self.record(EMR_SELECTOBJECT, &[slot]);
                    self.null_brush = false;
                }
            }
            (Some(c), made) => {
                let slot = made.map_or(1, |(slot, _)| 3 - slot);
                self.record(EMR_CREATEBRUSHINDIRECT, &[slot, BS_SOLID, colorref(c), 0]);
                self.record(EMR_SELECTOBJECT, &[slot]);
                if let Some((old, _)) = made {
                    self.record(EMR_DELETEOBJECT, &[old]);
                }
                self.brush = Some((slot, c));
                self.null_brush = false;
            }
        }
    }

    /// Selects a geometric pen of `key`, or the null pen.
    fn select_pen(&mut self, key: Option<PenKey>) {
        match (key, self.pen) {
            (None, _) => {
                if !self.null_pen {
                    self.record(EMR_SELECTOBJECT, &[NULL_PEN]);
                    self.null_pen = true;
                }
            }
            (Some(key), Some((slot, made))) if made == key => {
                if self.null_pen {
                    self.record(EMR_SELECTOBJECT, &[slot]);
                    self.null_pen = false;
                }
            }
            (Some(key), made) => {
                let slot = made.map_or(3, |(slot, _)| 7 - slot);
                let (colour, width, style) = key;
                // No brush bitmap: its offsets and sizes are zero; a solid
                // brush of the colour, no hatch and no dash entries.
                let pen = [
                    slot,
                    0,
                    0,
                    0,
                    0,
                    style,
                    width,
                    BS_SOLID,
                    colorref(colour),
                    0,
                    0,
                ];
                self.record(EMR_EXTCREATEPEN, &pen);
                self.record(EMR_SELECTOBJECT, &[slot]);
                if let Some((old, _)) = made {
                    self.record(EMR_DELETEOBJECT, &[old]);
                }
                self.pen = Some((slot, key));
                self.null_pen = false;
            }
        }
    }

    /// One painted path.
    fn shape(&mut self, shape: &Shape) -> Result<(), String> {
        let paint = match (shape.fill.is_some(), shape.stroke.is_some()) {
            (true, true) => EMR_STROKEANDFILLPATH,
            (true, false) => EMR_FILLPATH,
            (false, true) => EMR_STROKEPATH,
            (false, false) => return Ok(()),
        };
        let mode = if shape.evenodd { ALTERNATE } else { WINDING };
        if shape.fill.is_some() && self.fill_mode != mode {
            self.record(EMR_SETPOLYFILLMODE, &[mode]);
            self.fill_mode = mode;
        }
        self.select_brush(shape.fill);
        let width = (shape.stroke_width * UNITS_PER_POINT).round().max(1.);
        let pen = shape.stroke.map(|colour| {
            // Joins: round 0, bevel 0x1000, miter 0x2000; caps: round 0,
            // square 0x100, flat 0x200.
            let join = [0x2000, 0, 0x1000][usize::from(shape.join.min(2))];
            let cap = [0x200, 0, 0x100][usize::from(shape.cap.min(2))];
            (
                colour,
                width.min(MAX_COORDINATE) as u32,
                PS_GEOMETRIC | join | cap,
            )
        });
        self.select_pen(pen);
        self.record(EMR_BEGINPATH, &[]);
        let mut all: Vec<[i32; 2]> = Vec::new();
        let mut start = [0, 0];
        let mut closed = false;
        let mut i = 0;
        while let Some(segment) = shape.segments.get(i) {
            match *segment {
                Segment::Move(p) => {
                    start = point(p)?;
                    self.record(EMR_MOVETOEX, &start.map(|v| v as u32));
                    all.push(start);
                    closed = false;
                    i += 1;
                }
                Segment::Close => {
                    self.record(EMR_CLOSEFIGURE, &[]);
                    closed = true;
                    i += 1;
                }
                Segment::Line(_) | Segment::Cubic(..) => {
                    if closed {
                        // After a close SVG goes on from the subpath's start.
                        self.record(EMR_MOVETOEX, &start.map(|v| v as u32));
                        closed = false;
                    }
                    let cubic = matches!(segment, Segment::Cubic(..));
                    let mut points = Vec::new();
                    while let Some(next) = shape.segments.get(i) {
                        match (*next, cubic) {
                            (Segment::Line(p), false) => points.push(point(p)?),
                            (Segment::Cubic(a, b, p), true) => {
                                points.extend([point(a)?, point(b)?, point(p)?]);
                            }
                            _ => break,
                        }
                        i += 1;
                    }
                    match (cubic, points.len()) {
                        (false, 1) => self.record(EMR_LINETO, &points[0].map(|v| v as u32)),
                        (false, _) => self.poly(EMR_POLYLINETO16, EMR_POLYLINETO, &points)?,
                        (true, _) => self.poly(EMR_POLYBEZIERTO16, EMR_POLYBEZIERTO, &points)?,
                    }
                    all.extend(points);
                }
            }
        }
        self.record(EMR_ENDPATH, &[]);
        let reach = if shape.stroke.is_some() {
            (width / 2.).ceil().min(MAX_COORDINATE) as i32
        } else {
            0
        };
        let [left, top, right, bottom] = bounds(&all);
        let widened = [left - reach, top - reach, right + reach, bottom + reach];
        self.record(paint, &widened.map(|v| v as u32));
        Ok(())
    }

    /// The header before the records, the whole file as bytes.
    fn finish(self, page: &Page, size: [i32; 2]) -> Result<Vec<u8>, String> {
        let description: Vec<u8> = "VectorMagik\0drawing\0\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let padded = (description.len() + 3) & !3;
        let header_size = 108 + padded;
        let total = u32::try_from(header_size + self.body.len())
            .map_err(|_| "The drawing is too large for an EMF")?;
        // The frame in 0.01 mm, both corners inside the picture.
        let frame = |points: f64| (points / 72. * 2540.).round().max(1.) as i32 - 1;
        let mut out = Vec::with_capacity(total as usize);
        let mut put = |v: u32| out.extend_from_slice(&v.to_le_bytes());
        put(EMR_HEADER);
        put(header_size as u32);
        for v in [0, 0, size[0] - 1, size[1] - 1] {
            put(v as u32);
        }
        for v in [0, 0, frame(page.width), frame(page.height)] {
            put(v as u32);
        }
        put(ENHMETA_SIGNATURE);
        put(0x0001_0000);
        put(total);
        put(self.records + 1);
        // Five handles (slot 0 is the metafile's own) and the reserved word.
        put(5);
        put(description.len() as u32 / 2);
        put(108);
        put(0);
        for v in DEVICE_PIXELS.into_iter().chain(DEVICE_MILLIMETRES) {
            put(v as u32);
        }
        // No pixel format, no OpenGL; the reference size in micrometres.
        put(0);
        put(0);
        put(0);
        for v in DEVICE_MILLIMETRES {
            put(v as u32 * 1000);
        }
        out.extend_from_slice(&description);
        out.resize(header_size, 0);
        out.extend_from_slice(&self.body);
        Ok(out)
    }
}

/// The inclusive bounds of `points`, left, top, right and bottom.
fn bounds(points: &[[i32; 2]]) -> [i32; 4] {
    if points.is_empty() {
        return [0; 4];
    }
    points.iter().fold(
        [i32::MAX, i32::MAX, i32::MIN, i32::MIN],
        |[l, t, r, b], [x, y]| [l.min(*x), t.min(*y), r.max(*x), b.max(*y)],
    )
}

#[cfg(test)]
mod tests;
