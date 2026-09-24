//! PDF files, and Illustrator files saved PDF-compatible (every AI since
//! Illustrator 9 by default is a PDF with Illustrator's own data inside),
//! read as vector artwork (September 24, 2026), in owned Rust: the file's
//! structure in `document.rs` (with `syntax.rs` and `filters.rs`), its
//! drawing here.
//!
//! One page becomes the flat-path SVG `import.rs` describes: the page's
//! CropBox (else its MediaBox, inherited down the page tree) in points,
//! turned by its `/Rotate`, y down. Every painted path is one `path`
//! element in painting order, its points carried through the current
//! transformation, a fill and stroke of one path one element with both.
//! Colours are converted to RGB (CMYK naively, as a screen shows it without
//! a profile; spot colours through their alternate space when the tint
//! transform is a simple exponential); opacity comes from `/ca` and `/CA`.
//! Form XObjects are drawn in place. What is not paths is left out and said
//! so once in `skipped`: text, images, gradients, clipping (shapes are drawn
//! whole), dashes (drawn solid), patterns (drawn in one colour).
//!
//! A file is untrusted input: every size is capped (operations, nesting,
//! decoded bytes, the SVG written), so a hostile file ends in an error or a
//! cut-short page, never a hang or an exhausted memory.
mod document;
mod filters;
mod picture;
mod syntax;

use crate::import::Imported;
use document::Document;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::rc::Rc;
use syntax::{is_delimiter, is_white, Dict, Lexer, Obj, Stream, Token};

/// Content operators run for one page, forms counted each time drawn.
const MAX_OPERATIONS: usize = 4_000_000;
/// Content bytes read for one page, forms counted each time drawn.
const MAX_CONTENT: usize = 512 << 20;
/// The largest SVG written; the rest of the page is left out.
const MAX_SVG: usize = 64 << 20;
/// Forms drawn inside forms, at most this deep.
const MAX_FORM_DEPTH: usize = 32;
/// Forms drawn for one page, so a form that draws itself twice stops long
/// before its two to the thirty-second copies.
const MAX_FORMS: usize = 100_000;
/// Graphics states saved with `q` at once.
const MAX_SAVED: usize = 1024;
/// Operands kept before one operator (the most any takes is 33).
const MAX_OPERANDS: usize = 64;
/// How deep the page tree may go, and how many pages it may hold.
const MAX_TREE_DEPTH: usize = 64;
const MAX_PAGES: usize = 100_000;
/// A zero line width is the thinnest line a device can draw: one pixel at
/// 96 per inch, as a viewer draws it at full size.
const HAIRLINE: f64 = 0.75;
/// No point is further than this from the page (in points), so no number
/// written is absurd.
const FAR: f64 = 1e7;

const IDENTITY: [f64; 6] = [1., 0., 0., 1., 0., 0.];

const TEXT: &str = "text (outline it in the program that made the file to keep it)";
const CLIPPING: &str = "clipping (shapes are drawn whole)";
const DASHES: &str = "dashes (dashed lines are drawn solid)";
const PATTERNS: &str = "patterns drawn in one colour";
const SHADINGS: &str = "gradients (shadings)";
const SPOT: &str = "spot colours this reader cannot convert, drawn in black";
const UNKNOWN_COLOUR: &str = "colours in a colour space this reader cannot convert, drawn in black";
const MASKS: &str = "transparency masks";
const BLENDING: &str = "blend modes (drawn as normal)";
const DEEP: &str = "artwork nested more than 32 levels deep";
const TOO_MUCH: &str = "the rest of the page (it holds more drawing than the app reads)";
const NO_PDF_CONTENT: &str = "the artwork itself: Illustrator saved this file without PDF \
     content; save it again with Create PDF Compatible File turned on";

/// Page `page` (from 0) of the PDF `bytes` as flat paths, with the page
/// count and what was left out.
pub fn to_svg(bytes: &[u8], page: usize) -> Result<Imported, String> {
    let doc = Document::open(bytes)?;
    let pages = pages(&doc);
    let count = pages.len();
    if count == 0 {
        return Err("This PDF has no pages.".into());
    }
    let Some(chosen) = pages.get(page) else {
        let noun = if count == 1 { "page" } else { "pages" };
        return Err(format!(
            "This PDF has {count} {noun}; there is no page {}.",
            page + 1
        ));
    };
    let (width, height, placement) = chosen.placement(&doc);
    let mut painter = Painter::new(&doc, placement);
    let content = painter.page_content(&chosen.page);
    let resources = chosen.resources.clone().unwrap_or(Obj::Null);
    painter.run(&content, &resources, 0);
    let illustrator = chosen
        .page
        .dict()
        .and_then(|d| d.get(b"PieceInfo"))
        .map(|p| doc.resolve(p))
        .is_some_and(|p| p.dict().and_then(|d| d.get(b"Illustrator")).is_some());
    if illustrator && painter.paths == 0 && painter.notes.iter().any(|n| n == TEXT) {
        painter.note(NO_PDF_CONTENT);
    }
    let mut skipped = painter.notes;
    if let Some(at) = painter.image_note {
        skipped[at] = match painter.images {
            1 => "1 image".to_owned(),
            n => format!("{n} images"),
        };
    }
    let mut svg = String::with_capacity(painter.body.len() + 128);
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}pt\" height=\"{h}pt\" \
         viewBox=\"0 0 {w} {h}\">",
        w = num(width),
        h = num(height)
    );
    svg.push_str(&painter.body);
    svg.push_str("</svg>\n");
    Ok(Imported {
        svg,
        pages: count,
        skipped,
    })
}

/// The largest picture drawn on page `page` (from 0), for a PDF whose page
/// has no shapes to keep: `None` when it draws no image, an error when its
/// image is one the app cannot read.
pub fn picture(
    bytes: &[u8],
    page: usize,
) -> Result<Option<vector_rebuild::raster::Raster>, String> {
    let doc = Document::open(bytes)?;
    let pages = pages(&doc);
    let Some(chosen) = pages.get(page) else {
        return Ok(None);
    };
    let (_, _, placement) = chosen.placement(&doc);
    let mut painter = Painter::new(&doc, placement);
    let content = painter.page_content(&chosen.page);
    let resources = chosen.resources.clone().unwrap_or(Obj::Null);
    painter.run(&content, &resources, 0);
    match painter.largest.take() {
        Some(found) => picture::decode(&doc, &found)
            .map(Some)
            .map_err(|why| format!("This PDF holds a picture, but {why}.")),
        None => Ok(None),
    }
}

/// A page and what it inherits from the page tree above it.
#[derive(Clone)]
struct Page {
    page: Obj,
    resources: Option<Obj>,
    media: Option<Obj>,
    crop: Option<Obj>,
    rotate: Option<Obj>,
}

impl Page {
    /// The page's size in points once turned, and the matrix from its own
    /// space (y up) onto that page (y down, the top left corner at 0 0).
    fn placement(&self, doc: &Document) -> (f64, f64, [f64; 6]) {
        let media = self.media.as_ref().and_then(|b| rect(doc, b));
        let crop = self.crop.as_ref().and_then(|b| rect(doc, b));
        let [x0, y0, x1, y1] = match (crop, media) {
            (Some(c), Some(m)) => {
                let cut = [
                    c[0].max(m[0]),
                    c[1].max(m[1]),
                    c[2].min(m[2]),
                    c[3].min(m[3]),
                ];
                if cut[2] - cut[0] >= 0.01 && cut[3] - cut[1] >= 0.01 {
                    cut
                } else {
                    m
                }
            }
            (Some(b), None) | (None, Some(b)) => b,
            // US Letter, what viewers assume of a page with no size.
            (None, None) => [0., 0., 612., 792.],
        };
        let rotate = self
            .rotate
            .as_ref()
            .and_then(|r| doc.resolve(r).int())
            .unwrap_or(0)
            .rem_euclid(360);
        let (w, h) = (x1 - x0, y1 - y0);
        match rotate {
            90 => (h, w, [0., 1., 1., 0., -y0, -x0]),
            180 => (w, h, [-1., 0., 0., 1., x1, -y0]),
            270 => (h, w, [0., -1., -1., 0., y1, x1]),
            _ => (w, h, [1., 0., 0., -1., -x0, y1]),
        }
    }
}

/// Every page in order, from the catalog's page tree.
fn pages(doc: &Document) -> Vec<Page> {
    let mut out = Vec::new();
    let Some(root) = doc
        .catalog()
        .and_then(|c| c.dict().and_then(|d| d.get(b"Pages")).cloned())
    else {
        return out;
    };
    let top = Page {
        page: Obj::Null,
        resources: None,
        media: None,
        crop: None,
        rotate: None,
    };
    walk(doc, &root, top, &mut HashSet::new(), &mut out, 0);
    out
}

fn walk(
    doc: &Document,
    node: &Obj,
    mut inherited: Page,
    seen: &mut HashSet<u32>,
    out: &mut Vec<Page>,
    depth: usize,
) {
    if depth > MAX_TREE_DEPTH || out.len() >= MAX_PAGES {
        return;
    }
    if let Obj::Ref(number) = node {
        if !seen.insert(*number) {
            return;
        }
    }
    let obj = doc.resolve(node);
    let Some(dict) = obj.dict() else { return };
    for (key, slot) in [
        (&b"Resources"[..], &mut inherited.resources),
        (&b"MediaBox"[..], &mut inherited.media),
        (&b"CropBox"[..], &mut inherited.crop),
        (&b"Rotate"[..], &mut inherited.rotate),
    ] {
        if let Some(value) = dict.get(key) {
            *slot = Some(value.clone());
        }
    }
    let kids = dict.get(b"Kids").map(|k| doc.resolve(k));
    match kids.as_ref().and_then(Obj::array) {
        Some(kids) if dict.name(b"Type") != Some(&b"Page"[..]) => {
            for kid in kids {
                walk(doc, kid, inherited.clone(), seen, out, depth + 1);
            }
        }
        _ if dict.name(b"Type") == Some(&b"Pages"[..]) => {}
        _ => {
            inherited.page = obj.clone();
            out.push(inherited);
        }
    }
}

/// A rectangle `[x0 y0 x1 y1]` with its corners in order, when it has size.
fn rect(doc: &Document, obj: &Obj) -> Option<[f64; 4]> {
    let obj = doc.resolve(obj);
    let items = obj.array()?;
    let mut v = [0.; 4];
    if items.len() < 4 {
        return None;
    }
    for (slot, item) in v.iter_mut().zip(items) {
        *slot = doc.resolve(item).number().filter(|n| n.is_finite())?;
    }
    let [a, b, c, d] = v;
    let r = [a.min(c), b.min(d), a.max(c), b.max(d)];
    (r[2] - r[0] >= 0.01 && r[3] - r[1] >= 0.01).then_some(r)
}

/// `m` then `n`: the matrix that applies `m` first.
fn concat(m: &[f64; 6], n: &[f64; 6]) -> [f64; 6] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn apply(m: &[f64; 6], x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// The last `N` operands as numbers, when they are.
fn operands<const N: usize>(args: &[Obj]) -> Option<[f64; N]> {
    let start = args.len().checked_sub(N)?;
    let mut out = [0.; N];
    for (slot, arg) in out.iter_mut().zip(&args[start..]) {
        *slot = arg.number().filter(|v| v.is_finite())?;
    }
    Some(out)
}

/// A number as the SVG writes it: at most three decimals (a thousandth of
/// a point), no trailing zeros.
fn num(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".to_owned(),
        _ => text.to_owned(),
    }
}

fn hex(rgb: [f64; 3]) -> String {
    let byte = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgb[0]),
        byte(rgb[1]),
        byte(rgb[2])
    )
}

/// A colour space, as far as this reader converts it to RGB.
enum Space {
    Gray,
    Rgb,
    Cmyk,
    /// CIE L*a*b*, with the a* and b* range.
    Lab([f64; 4]),
    /// Colours by number from a table in the base space.
    Indexed {
        base: Rc<Space>,
        table: Rc<[u8]>,
        high: usize,
    },
    /// Separation or DeviceN of one colorant whose tint transform is an
    /// exponential function: `low + t^exponent (high - low)` in `alternate`.
    Tint {
        alternate: Rc<Space>,
        low: Vec<f64>,
        high: Vec<f64>,
        exponent: f64,
    },
    /// A space this reader cannot convert: drawn black and noted.
    Other {
        components: usize,
        note: &'static str,
    },
    /// Patterns, drawn mid grey and noted.
    Pattern,
}

impl Space {
    fn components(&self) -> usize {
        match self {
            Space::Gray | Space::Indexed { .. } | Space::Tint { .. } | Space::Pattern => 1,
            Space::Rgb | Space::Lab(_) => 3,
            Space::Cmyk => 4,
            Space::Other { components, .. } => *components,
        }
    }

    /// The colour a space starts at when `cs` selects it.
    fn initial(&self) -> Vec<f64> {
        match self {
            Space::Cmyk => vec![0., 0., 0., 1.],
            Space::Tint { .. } => vec![1.],
            other => vec![0.; other.components()],
        }
    }

    /// Component `k` of this space from a byte of an indexed table.
    fn byte_component(&self, k: usize, byte: u8) -> f64 {
        let t = f64::from(byte) / 255.;
        match (self, k) {
            (Space::Lab(_), 0) => t * 100.,
            (Space::Lab(r), _) => r[2 * k - 2] + t * (r[2 * k - 1] - r[2 * k - 2]),
            _ => t,
        }
    }

    /// `c` in this space as RGB from 0 to 1, and what to note when the
    /// conversion is a stand-in.
    fn rgb(&self, c: &[f64]) -> ([f64; 3], Option<&'static str>) {
        let at = |i: usize| c.get(i).copied().unwrap_or(0.).clamp(0., 1.);
        match self {
            Space::Gray => ([at(0); 3], None),
            Space::Rgb => ([at(0), at(1), at(2)], None),
            Space::Cmyk => {
                let k = 1. - at(3);
                ([(1. - at(0)) * k, (1. - at(1)) * k, (1. - at(2)) * k], None)
            }
            Space::Lab(range) => {
                let l = c.first().copied().unwrap_or(0.).clamp(0., 100.);
                let a = c.get(1).copied().unwrap_or(0.).clamp(range[0], range[1]);
                let b = c.get(2).copied().unwrap_or(0.).clamp(range[2], range[3]);
                (lab_to_rgb(l, a, b), None)
            }
            Space::Indexed { base, table, high } => {
                let index = c
                    .first()
                    .copied()
                    .unwrap_or(0.)
                    .round()
                    .clamp(0., *high as f64) as usize;
                let n = base.components();
                let entry: Vec<f64> = (0..n)
                    .map(|k| base.byte_component(k, table.get(index * n + k).copied().unwrap_or(0)))
                    .collect();
                base.rgb(&entry)
            }
            Space::Tint {
                alternate,
                low,
                high,
                exponent,
            } => {
                let t = at(0).powf(*exponent);
                let out: Vec<f64> = (0..low.len().max(high.len()))
                    .map(|i| {
                        let (a, b) = (
                            low.get(i).copied().unwrap_or(0.),
                            high.get(i).copied().unwrap_or(0.),
                        );
                        a + t * (b - a)
                    })
                    .collect();
                alternate.rgb(&out)
            }
            Space::Other { note, .. } => ([0.; 3], Some(*note)),
            Space::Pattern => ([0.5; 3], Some(PATTERNS)),
        }
    }
}

/// CIE L*a*b* (D50, as PDF's Lab is used) to sRGB.
fn lab_to_rgb(l: f64, a: f64, b: f64) -> [f64; 3] {
    let fy = (l + 16.) / 116.;
    let (fx, fz) = (fy + a / 500., fy - b / 200.);
    let delta = 6. / 29.;
    let f = |t: f64| {
        if t > delta {
            t * t * t
        } else {
            3. * delta * delta * (t - 4. / 29.)
        }
    };
    let (x, y, z) = (0.9642 * f(fx), f(fy), 0.8249 * f(fz));
    let linear = [
        3.1338561 * x - 1.6168667 * y - 0.4906146 * z,
        -0.9787684 * x + 1.9161415 * y + 0.0334540 * z,
        0.0719453 * x - 0.2289914 * y + 1.4052427 * z,
    ];
    linear.map(|v| {
        let v = v.clamp(0., 1.);
        if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        }
    })
}

/// A colour space and the components last set in it.
#[derive(Clone)]
struct Colour {
    space: Rc<Space>,
    components: Vec<f64>,
}

impl Colour {
    fn new(space: Rc<Space>) -> Self {
        let components = space.initial();
        Self { space, components }
    }
}

/// The graphics state the operators change, saved by `q`, restored by `Q`.
#[derive(Clone)]
struct State {
    ctm: [f64; 6],
    fill: Colour,
    stroke: Colour,
    line_width: f64,
    cap: u8,
    join: u8,
    fill_alpha: f64,
    stroke_alpha: f64,
    /// The opacity of the transparency groups being drawn, multiplied: a
    /// group is composited as a whole at the `ca` it was drawn with, and
    /// its own content starts again at full opacity. Flat paths cannot
    /// hold a group, so each path takes the group's opacity with its own.
    group_alpha: f64,
}

/// The path being built, in page space, as SVG path data.
#[derive(Default)]
struct Path {
    d: String,
    current: Option<(f64, f64)>,
    start: Option<(f64, f64)>,
    segments: usize,
    /// A point was not finite or absurdly far out; the path is not drawn.
    broken: bool,
}

impl Path {
    fn write(&mut self, command: char, points: &[(f64, f64)]) {
        self.d.push(command);
        for (i, &(x, y)) in points.iter().enumerate() {
            if !(x.abs() < FAR && y.abs() < FAR) {
                self.broken = true;
            }
            let space = if i == 0 { "" } else { " " };
            let _ = write!(self.d, "{space}{} {}", num(x), num(y));
        }
    }

    fn move_to(&mut self, p: (f64, f64)) {
        self.write('M', &[p]);
        self.current = Some(p);
        self.start = Some(p);
    }

    fn line_to(&mut self, p: (f64, f64)) {
        if self.current.is_none() {
            return self.move_to(p);
        }
        self.write('L', &[p]);
        self.segments += 1;
        self.current = Some(p);
    }

    fn curve_to(&mut self, a: (f64, f64), b: (f64, f64), p: (f64, f64)) {
        if self.current.is_none() {
            self.move_to(a);
        }
        self.write('C', &[a, b, p]);
        self.segments += 1;
        self.current = Some(p);
    }

    fn close(&mut self) {
        if self.current.is_some() {
            self.d.push('Z');
            self.current = self.start;
        }
    }
}

/// Runs content streams and writes what they paint.
struct Painter<'d> {
    doc: &'d Document<'d>,
    state: State,
    saved: Vec<State>,
    /// `q` beyond `MAX_SAVED`, matched by the `Q`s that follow.
    unsaved: usize,
    path: Path,
    /// A `W` or `W*` waits for the next painting operator.
    clip: bool,
    body: String,
    paths: usize,
    notes: Vec<String>,
    images: usize,
    /// Where the image count goes among the notes.
    image_note: Option<usize>,
    /// The largest image drawn, the picture a page with no shapes offers.
    largest: Option<picture::Found>,
    operations: usize,
    forms_drawn: usize,
    content_read: usize,
    stopped: bool,
    /// Each form's decoded content, by object number.
    forms: HashMap<u32, Option<Rc<[u8]>>>,
    gray: Rc<Space>,
    rgb: Rc<Space>,
    cmyk: Rc<Space>,
}

impl<'d> Painter<'d> {
    fn new(doc: &'d Document<'d>, placement: [f64; 6]) -> Self {
        let gray = Rc::new(Space::Gray);
        let state = State {
            ctm: placement,
            fill: Colour::new(gray.clone()),
            stroke: Colour::new(gray.clone()),
            line_width: 1.,
            cap: 0,
            join: 0,
            fill_alpha: 1.,
            stroke_alpha: 1.,
            group_alpha: 1.,
        };
        Self {
            doc,
            state,
            saved: Vec::new(),
            unsaved: 0,
            path: Path::default(),
            clip: false,
            body: String::new(),
            paths: 0,
            notes: Vec::new(),
            images: 0,
            image_note: None,
            largest: None,
            operations: 0,
            forms_drawn: 0,
            content_read: 0,
            stopped: false,
            forms: HashMap::new(),
            gray,
            rgb: Rc::new(Space::Rgb),
            cmyk: Rc::new(Space::Cmyk),
        }
    }

    fn note(&mut self, text: &str) {
        if !self.notes.iter().any(|n| n == text) {
            self.notes.push(text.to_owned());
        }
    }

    fn unreadable(&mut self, why: &str) {
        self.note(&format!(
            "part of the page, which could not be read because {why}"
        ));
    }

    fn stop(&mut self) {
        self.stopped = true;
        self.note(TOO_MUCH);
    }

    fn image(&mut self) {
        if self.image_note.is_none() {
            self.image_note = Some(self.notes.len());
            self.notes.push(String::new());
        }
        self.images += 1;
    }

    /// The page's content streams, joined by line breaks.
    fn page_content(&mut self, page: &Obj) -> Vec<u8> {
        let Some(contents) = page.dict().and_then(|d| d.get(b"Contents")) else {
            return Vec::new();
        };
        let parts: Vec<Obj> = match self.doc.resolve(contents) {
            Obj::Array(items) => items.iter().map(|i| self.doc.resolve(i)).collect(),
            other => vec![other],
        };
        let mut out = Vec::new();
        for part in parts {
            let Obj::Stream(stream) = part else { continue };
            match self.doc.decode(&stream) {
                Ok(data) => {
                    out.extend_from_slice(&data);
                    out.push(b'\n');
                }
                Err(why) => self.unreadable(&why),
            }
        }
        out
    }

    /// Resource `name` of `category` (`/XObject`, `/ExtGState` ...),
    /// unresolved.
    fn resource(&self, resources: &Obj, category: &[u8], name: &[u8]) -> Option<Obj> {
        let resources = self.doc.resolve(resources);
        let group = self.doc.resolve(resources.dict()?.get(category)?);
        group.dict()?.get(name).cloned()
    }

    fn run(&mut self, content: &[u8], resources: &Obj, depth: usize) {
        self.content_read = self.content_read.saturating_add(content.len());
        if self.content_read > MAX_CONTENT {
            return self.stop();
        }
        let mut lexer = Lexer::new(content, 0);
        let mut args: Vec<Obj> = Vec::new();
        while !self.stopped {
            let Some(token) = lexer.next_token() else {
                break;
            };
            match token {
                Token::Word(op) => {
                    self.operations += 1;
                    if self.operations > MAX_OPERATIONS {
                        return self.stop();
                    }
                    if op == b"BI" {
                        self.inline_image(&mut lexer, resources);
                    } else {
                        self.operator(op, &args, resources, depth);
                    }
                    args.clear();
                }
                other => {
                    if let Some(obj) = lexer.object(other, false) {
                        if args.len() < MAX_OPERANDS {
                            args.push(obj);
                        }
                    }
                }
            }
        }
    }

    fn operator(&mut self, op: &[u8], args: &[Obj], resources: &Obj, depth: usize) {
        let ctm = self.state.ctm;
        let at = |x: f64, y: f64| apply(&ctm, x, y);
        match op {
            b"q" => {
                if self.saved.len() < MAX_SAVED {
                    self.saved.push(self.state.clone());
                } else {
                    self.unsaved += 1;
                }
            }
            b"Q" => {
                if self.unsaved > 0 {
                    self.unsaved -= 1;
                } else if let Some(state) = self.saved.pop() {
                    self.state = state;
                }
            }
            b"cm" => {
                if let Some(m) = operands::<6>(args) {
                    self.state.ctm = concat(&m, &ctm);
                }
            }
            b"m" => {
                if let Some([x, y]) = operands(args) {
                    self.path.move_to(at(x, y));
                }
            }
            b"l" => {
                if let Some([x, y]) = operands(args) {
                    self.path.line_to(at(x, y));
                }
            }
            b"c" => {
                if let Some([x1, y1, x2, y2, x3, y3]) = operands(args) {
                    self.path.curve_to(at(x1, y1), at(x2, y2), at(x3, y3));
                }
            }
            b"v" => {
                if let Some([x2, y2, x3, y3]) = operands(args) {
                    let first = self.path.current.unwrap_or(at(x2, y2));
                    self.path.curve_to(first, at(x2, y2), at(x3, y3));
                }
            }
            b"y" => {
                if let Some([x1, y1, x3, y3]) = operands(args) {
                    self.path.curve_to(at(x1, y1), at(x3, y3), at(x3, y3));
                }
            }
            b"h" => self.path.close(),
            b"re" => {
                if let Some([x, y, w, h]) = operands(args) {
                    self.path.move_to(at(x, y));
                    self.path.line_to(at(x + w, y));
                    self.path.line_to(at(x + w, y + h));
                    self.path.line_to(at(x, y + h));
                    self.path.close();
                }
            }
            b"f" | b"F" => self.paint(Some(false), false),
            b"f*" => self.paint(Some(true), false),
            b"S" => self.paint(None, true),
            b"B" => self.paint(Some(false), true),
            b"B*" => self.paint(Some(true), true),
            b"s" | b"b" | b"b*" => {
                self.path.close();
                let fill = (op != b"s").then_some(op == b"b*");
                self.paint(fill, true);
            }
            b"n" => self.paint(None, false),
            b"W" | b"W*" => self.clip = true,
            b"w" => {
                if let Some([w]) = operands(args) {
                    self.state.line_width = w.abs();
                }
            }
            b"J" => {
                if let Some([cap]) = operands(args) {
                    self.state.cap = cap.clamp(0., 2.) as u8;
                }
            }
            b"j" => {
                if let Some([join]) = operands(args) {
                    self.state.join = join.clamp(0., 2.) as u8;
                }
            }
            b"d" => {
                if args.first().is_some_and(dashed) {
                    self.note(DASHES);
                }
            }
            b"g" | b"G" => self.device(op == b"G", self.gray.clone(), args),
            b"rg" | b"RG" => self.device(op == b"RG", self.rgb.clone(), args),
            b"k" | b"K" => self.device(op == b"K", self.cmyk.clone(), args),
            b"cs" | b"CS" => {
                if let Some(name) = args.last() {
                    let colour = Colour::new(Rc::new(self.space(name, resources, 0)));
                    *self.colour(op == b"CS") = colour;
                }
            }
            b"sc" | b"scn" | b"SC" | b"SCN" => {
                let numbers: Vec<f64> = args.iter().filter_map(Obj::number).collect();
                let colour = self.colour(op[0] == b'S');
                let n = colour.space.components();
                if !matches!(*colour.space, Space::Pattern) && numbers.len() >= n {
                    colour.components = numbers[numbers.len() - n..].to_vec();
                }
            }
            b"gs" => {
                if let Some(name) = args.last().and_then(Obj::name) {
                    self.ext_state(name, resources);
                }
            }
            b"Do" => {
                if let Some(name) = args.last().and_then(Obj::name) {
                    self.xobject(name, resources, depth);
                }
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => self.note(TEXT),
            b"sh" => self.note(SHADINGS),
            // Everything else changes nothing drawn here: text state and
            // positioning, marked content, rendering intent, flatness,
            // the miter limit (the SVG keeps its own), compatibility
            // sections and Type 3 glyph metrics.
            _ => {}
        }
    }

    fn colour(&mut self, stroke: bool) -> &mut Colour {
        if stroke {
            &mut self.state.stroke
        } else {
            &mut self.state.fill
        }
    }

    /// `g`, `rg` and `k` (and their stroking forms): a device space and its
    /// components in one operator.
    fn device(&mut self, stroke: bool, space: Rc<Space>, args: &[Obj]) {
        let Some(start) = args.len().checked_sub(space.components()) else {
            return;
        };
        let components: Option<Vec<f64>> = args[start..].iter().map(Obj::number).collect();
        if let Some(components) = components {
            *self.colour(stroke) = Colour { space, components };
        }
    }

    /// The colour space `obj` names or spells out.
    fn space(&self, obj: &Obj, resources: &Obj, depth: usize) -> Space {
        let unknown = Space::Other {
            components: 1,
            note: UNKNOWN_COLOUR,
        };
        if depth > 8 {
            return unknown;
        }
        let obj = self.doc.resolve(obj);
        if let Some(name) = obj.name() {
            return match name {
                b"DeviceGray" | b"G" | b"CalGray" => Space::Gray,
                b"DeviceRGB" | b"RGB" | b"CalRGB" => Space::Rgb,
                b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
                b"Pattern" => Space::Pattern,
                _ => match self.resource(resources, b"ColorSpace", name) {
                    Some(named) => self.space(&named, resources, depth + 1),
                    None => unknown,
                },
            };
        }
        let Some(items) = obj.array() else {
            return unknown;
        };
        let item = |i: usize| items.get(i).map(|o| self.doc.resolve(o));
        let family = item(0);
        match family.as_ref().and_then(Obj::name).unwrap_or_default() {
            b"DeviceGray" | b"CalGray" | b"G" => Space::Gray,
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Space::Rgb,
            b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
            b"Pattern" => Space::Pattern,
            b"Lab" => {
                let range = item(1)
                    .and_then(|d| d.dict().and_then(|d| d.get(b"Range")).cloned())
                    .and_then(|r| numbers(self.doc, &r))
                    .filter(|r| r.len() == 4 && r[0] < r[1] && r[2] < r[3]);
                Space::Lab(range.map_or([-100., 100., -100., 100.], |r| [r[0], r[1], r[2], r[3]]))
            }
            b"ICCBased" => {
                let profile = item(1);
                let dict = profile.as_ref().and_then(Obj::dict);
                let n = dict
                    .and_then(|d| d.get(b"N"))
                    .and_then(|n| self.doc.resolve(n).int());
                match (n, dict.and_then(|d| d.get(b"Alternate"))) {
                    (Some(1), _) => Space::Gray,
                    (Some(3), _) => Space::Rgb,
                    (Some(4), _) => Space::Cmyk,
                    (_, Some(alternate)) => self.space(alternate, resources, depth + 1),
                    (n, None) => Space::Other {
                        components: n.and_then(|n| usize::try_from(n).ok()).unwrap_or(1),
                        note: UNKNOWN_COLOUR,
                    },
                }
            }
            b"Indexed" | b"I" => {
                let Some(base) = items.get(1) else {
                    return unknown;
                };
                let base = Rc::new(self.space(base, resources, depth + 1));
                let high = item(2).and_then(|h| h.int()).unwrap_or(0).clamp(0, 255) as usize;
                let table: Rc<[u8]> = match item(3) {
                    Some(Obj::Str(s)) => s,
                    Some(Obj::Stream(s)) => self.doc.decode(&s).unwrap_or_default().into(),
                    _ => Rc::from(Vec::new()),
                };
                Space::Indexed { base, table, high }
            }
            b"Separation" => self.tint(1, items.get(2), item(3), resources, depth),
            b"DeviceN" => {
                let inputs = item(1).and_then(|n| n.array().map(<[Obj]>::len));
                self.tint(inputs.unwrap_or(0), items.get(2), item(3), resources, depth)
            }
            _ => unknown,
        }
    }

    /// Separation and DeviceN: the alternate space through the tint
    /// transform, when there is one colorant and the transform is an
    /// exponential function (type 2); else black, noted.
    fn tint(
        &self,
        inputs: usize,
        alternate: Option<&Obj>,
        function: Option<Obj>,
        resources: &Obj,
        depth: usize,
    ) -> Space {
        let doc = self.doc;
        let exponential = function
            .as_ref()
            .and_then(Obj::dict)
            .filter(|d| d.get(b"FunctionType").map(|t| doc.resolve(t).int()) == Some(Some(2)));
        let (Some(alternate), Some(f), 1) = (alternate, exponential, inputs) else {
            return Space::Other {
                components: inputs.max(1),
                note: SPOT,
            };
        };
        let list = |key: &[u8], default: f64| {
            f.get(key)
                .and_then(|v| numbers(doc, v))
                .unwrap_or_else(|| vec![default])
        };
        Space::Tint {
            alternate: Rc::new(self.space(alternate, resources, depth + 1)),
            low: list(b"C0", 0.),
            high: list(b"C1", 1.),
            exponent: f
                .get(b"N")
                .and_then(|n| doc.resolve(n).number())
                .unwrap_or(1.),
        }
    }

    /// `gs`: line width, cap, join and opacity from a graphics state
    /// dictionary; dashes, masks and blend modes noted.
    fn ext_state(&mut self, name: &[u8], resources: &Obj) {
        let Some(raw) = self.resource(resources, b"ExtGState", name) else {
            return;
        };
        let object = self.doc.resolve(&raw);
        let Some(dict) = object.dict() else { return };
        for (key, value) in &dict.0 {
            let value = self.doc.resolve(value);
            let number = value.number().filter(|v| v.is_finite());
            match (&**key, number) {
                (b"LW", Some(w)) => self.state.line_width = w.abs(),
                (b"LC", Some(c)) => self.state.cap = c.clamp(0., 2.) as u8,
                (b"LJ", Some(j)) => self.state.join = j.clamp(0., 2.) as u8,
                (b"CA", Some(a)) => self.state.stroke_alpha = a.clamp(0., 1.),
                (b"ca", Some(a)) => self.state.fill_alpha = a.clamp(0., 1.),
                (b"D", _) => {
                    let pattern = value.array().and_then(|d| d.first()).cloned();
                    if pattern.is_some_and(|p| dashed(&self.doc.resolve(&p))) {
                        self.note(DASHES);
                    }
                }
                (b"SMask", _) if value.name() != Some(&b"None"[..]) => self.note(MASKS),
                (b"BM", _) => {
                    let mode = match &value {
                        Obj::Array(modes) => modes.first().and_then(Obj::name).map(<[u8]>::to_vec),
                        other => other.name().map(<[u8]>::to_vec),
                    };
                    if !matches!(mode.as_deref(), None | Some(b"Normal" | b"Compatible")) {
                        self.note(BLENDING);
                    }
                }
                _ => {}
            }
        }
    }

    /// `Do`: a form drawn in place, an image counted.
    fn xobject(&mut self, name: &[u8], resources: &Obj, depth: usize) {
        let Some(raw) = self.resource(resources, b"XObject", name) else {
            return;
        };
        let object = self.doc.resolve(&raw);
        let Obj::Stream(stream) = &object else { return };
        let subtype = stream.dict.get(b"Subtype").map(|s| self.doc.resolve(s));
        match subtype.as_ref().and_then(Obj::name) {
            Some(b"Image") => {
                self.image();
                let side = |key: &[u8]| {
                    stream
                        .dict
                        .get(key)
                        .map(|v| self.doc.resolve(v))
                        .and_then(|v| v.int())
                        .unwrap_or(0)
                        .max(0) as u64
                };
                let area = side(b"Width") * side(b"Height");
                if self.largest.as_ref().is_none_or(|l| area > l.area) {
                    self.largest = Some(picture::Found {
                        stream: stream.clone(),
                        area,
                    });
                }
            }
            Some(b"Form") => self.form(&raw, stream, resources, depth),
            _ => {}
        }
    }

    fn form(&mut self, raw: &Obj, stream: &Stream, resources: &Obj, depth: usize) {
        if depth >= MAX_FORM_DEPTH {
            return self.note(DEEP);
        }
        self.forms_drawn += 1;
        if self.forms_drawn > MAX_FORMS {
            return self.stop();
        }
        let key = match raw {
            Obj::Ref(number) => Some(*number),
            _ => None,
        };
        let content = match key.and_then(|k| self.forms.get(&k).cloned()) {
            Some(content) => content,
            None => {
                let decoded = match self.doc.decode(stream) {
                    Ok(data) => Some(Rc::<[u8]>::from(data)),
                    Err(why) => {
                        self.unreadable(&why);
                        None
                    }
                };
                if let Some(k) = key {
                    self.forms.insert(k, decoded.clone());
                }
                decoded
            }
        };
        let Some(content) = content else { return };
        let matrix = stream
            .dict
            .get(b"Matrix")
            .and_then(|m| numbers(self.doc, m))
            .and_then(|m| <[f64; 6]>::try_from(m).ok())
            .unwrap_or(IDENTITY);
        let own = stream.dict.get(b"Resources").map(|r| self.doc.resolve(r));
        let inner = own
            .filter(|r| r.dict().is_some())
            .unwrap_or_else(|| resources.clone());
        let state = self.state.clone();
        let (saved, unsaved) = (self.saved.len(), self.unsaved);
        let (path, clip) = (std::mem::take(&mut self.path), self.clip);
        self.state.ctm = concat(&matrix, &self.state.ctm);
        let group = stream.dict.get(b"Group").map(|g| self.doc.resolve(g));
        if group.is_some_and(|g| g.dict().is_some()) {
            self.state.group_alpha *= self.state.fill_alpha;
            (self.state.fill_alpha, self.state.stroke_alpha) = (1., 1.);
        }
        self.run(&content, &inner, depth + 1);
        self.state = state;
        self.saved.truncate(saved);
        self.unsaved = unsaved;
        self.path = path;
        self.clip = clip;
    }

    /// `BI ... ID data EI`: the image counted and its data passed over, by
    /// its size when it is unfiltered and says it, else up to the first
    /// `EI` between white space that plain operators follow.
    fn inline_image(&mut self, lexer: &mut Lexer, resources: &Obj) {
        self.image();
        let mut dict = Dict::default();
        loop {
            match lexer.next_token() {
                None => return,
                Some(Token::Word(b"ID")) => break,
                Some(Token::Name(key)) => {
                    let value = lexer.next_token().and_then(|t| lexer.object(t, false));
                    dict.0.push((key.into(), value.unwrap_or(Obj::Null)));
                }
                Some(_) => {}
            }
        }
        let data = lexer.data;
        let start = (lexer.pos + 1).min(data.len());
        let ends_word = |at: usize| {
            data.get(at)
                .is_none_or(|b| is_white(*b) || is_delimiter(*b))
        };
        let end = self
            .inline_length(&dict, resources)
            .and_then(|length| start.checked_add(length))
            .filter(|end| *end <= data.len());
        if let Some(end) = end {
            let mut after = Lexer::new(data, end);
            after.skip_space();
            if data[after.pos..].starts_with(b"EI") && ends_word(after.pos + 2) {
                lexer.pos = after.pos + 2;
                return;
            }
        }
        let plausible = |rest: &[u8]| {
            rest.iter()
                .take(32)
                .all(|b| is_white(*b) || (0x20..0x7f).contains(b))
        };
        let mut i = start;
        while i + 2 <= data.len() {
            if &data[i..i + 2] == b"EI"
                && (i == start || is_white(data[i - 1]))
                && data.get(i + 2).is_none_or(|b| is_white(*b))
                && plausible(&data[i + 2..])
            {
                lexer.pos = i + 2;
                return;
            }
            i += 1;
        }
        lexer.pos = data.len();
    }

    /// The byte length of an inline image's data, when it says it or is
    /// unfiltered.
    fn inline_length(&self, dict: &Dict, resources: &Obj) -> Option<usize> {
        let get = |short: &[u8], long: &[u8]| dict.get(short).or_else(|| dict.get(long));
        let size = |obj: Option<&Obj>| obj.and_then(Obj::int).and_then(|v| usize::try_from(v).ok());
        if let Some(length) = size(get(b"L", b"Length")) {
            return Some(length);
        }
        let filtered = match get(b"F", b"Filter") {
            None | Some(Obj::Null) => false,
            Some(Obj::Array(filters)) => !filters.is_empty(),
            Some(_) => true,
        };
        if filtered {
            return None;
        }
        let (width, height) = (size(get(b"W", b"Width"))?, size(get(b"H", b"Height"))?);
        let mask = matches!(get(b"IM", b"ImageMask"), Some(Obj::Bool(true)));
        let (bits, colours) = if mask {
            (1, 1)
        } else {
            let bits = size(get(b"BPC", b"BitsPerComponent")).unwrap_or(8);
            let colours =
                get(b"CS", b"ColorSpace").map_or(1, |cs| self.space(cs, resources, 0).components());
            (bits, colours)
        };
        let row = width.checked_mul(colours)?.checked_mul(bits)?.div_ceil(8);
        row.checked_mul(height)
    }

    /// A painting operator: the path written with the fill (`Some(evenodd)`)
    /// and stroke asked for, and cleared.
    fn paint(&mut self, fill: Option<bool>, stroke: bool) {
        let path = std::mem::take(&mut self.path);
        if std::mem::take(&mut self.clip) {
            self.note(CLIPPING);
        }
        if (fill.is_none() && !stroke) || path.segments == 0 || path.broken {
            return;
        }
        let fill_alpha = self.state.fill_alpha * self.state.group_alpha;
        let stroke_alpha = self.state.stroke_alpha * self.state.group_alpha;
        let mut element = String::from("<path");
        match fill {
            Some(evenodd) => {
                let colour = self.state.fill.clone();
                let _ = write!(element, " fill=\"{}\"", self.hex(&colour));
                if evenodd {
                    element.push_str(" fill-rule=\"evenodd\"");
                }
                if fill_alpha < 1. {
                    let _ = write!(element, " fill-opacity=\"{}\"", num(fill_alpha));
                }
            }
            None => element.push_str(" fill=\"none\""),
        }
        if stroke {
            let colour = self.state.stroke.clone();
            let ctm = self.state.ctm;
            let scale = (ctm[0].hypot(ctm[1]) + ctm[2].hypot(ctm[3])) / 2.;
            let width = self.state.line_width * scale;
            let width = if width > 0. { width } else { HAIRLINE };
            let _ = write!(
                element,
                " stroke=\"{}\" stroke-width=\"{}\"",
                self.hex(&colour),
                num(width.min(FAR))
            );
            if self.state.join != 0 {
                let join = ["miter", "round", "bevel"][usize::from(self.state.join.min(2))];
                let _ = write!(element, " stroke-linejoin=\"{join}\"");
            }
            if self.state.cap != 0 {
                let cap = ["butt", "round", "square"][usize::from(self.state.cap.min(2))];
                let _ = write!(element, " stroke-linecap=\"{cap}\"");
            }
            if stroke_alpha < 1. {
                let _ = write!(element, " stroke-opacity=\"{}\"", num(stroke_alpha));
            }
        }
        let _ = writeln!(element, " d=\"{}\"/>", path.d);
        if self.body.len() + element.len() > MAX_SVG {
            return self.stop();
        }
        self.body.push_str(&element);
        self.paths += 1;
    }

    fn hex(&mut self, colour: &Colour) -> String {
        let (rgb, note) = colour.space.rgb(&colour.components);
        if let Some(note) = note {
            self.note(note);
        }
        hex(rgb)
    }
}

/// Whether a dash array (`[3 2]`) dashes at all.
fn dashed(obj: &Obj) -> bool {
    obj.array()
        .is_some_and(|a| a.iter().any(|v| v.number().is_some_and(|v| v > 0.)))
}

/// An array of numbers, each resolved.
fn numbers(doc: &Document, obj: &Obj) -> Option<Vec<f64>> {
    doc.resolve(obj)
        .array()?
        .iter()
        .map(|v| doc.resolve(v).number().filter(|n| n.is_finite()))
        .collect()
}

#[cfg(test)]
mod tests;
