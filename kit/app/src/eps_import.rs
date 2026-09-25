//! EPS, PostScript and Illustrator files of the PostScript era read as
//! vector artwork (the owner, September 24, 2026: "if someone throws an EPS
//! in ... do you want us to just cross-convert?").
//!
//! PostScript is a programming language, so the file is run: a small but
//! real interpreter of the Level 2 language as vector files use it (the
//! scanner; the operand, dictionary and graphics state stacks; the error
//! machinery prologs rely on; the operators of paths, colour, matrices,
//! strings, arrays, dictionaries, files and decoding filters), in pure
//! Rust. Every path the program paints becomes one `path` element of the
//! flat SVG `import.rs` describes, in painting order, in points with y
//! down, on the page the file's `%%HiResBoundingBox` (or `%%BoundingBox`)
//! gives. What it paints that is not a path (text, images, gradients) is
//! read past and named in `skipped`, and clipping is noted, not applied.
//!
//! Illustrator files of the PostScript era (up to Illustrator 8) are
//! PostScript calling Illustrator's own short operators (`m l c f k ...`),
//! defined in a procedure set the file often leaves out; such a name, when
//! undefined, is read with Illustrator's meaning as the Adobe Illustrator
//! File Format Specification gives it (recalled, not read from the
//! document), and in such a file an unknown name at the top level is
//! skipped and named rather than ending the import.
//!
//! The file is untrusted: the program runs under limits on its steps, its
//! nesting, its stacks and its memory, never touches the file system, and a
//! malformed or hostile file is refused in plain words.
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::rc::Rc;

use crate::import::Imported;
use vector_rebuild::raster::{Raster, Rgba};

mod graphics;
mod illustrator;
mod images;
mod object;
mod ops;
mod scan;
mod stream;
mod system;
#[cfg(test)]
mod tests;

use graphics::{GState, Seg, Shape};
use object::{Arr, Dict, Name, Obj, Str};
use stream::{File, Stream, Work};

/// Work a program may do: objects executed, plus one step for every 16
/// bytes a filter decodes.
const STEP_LIMIT: u64 = 50_000_000;
/// Procedures, names and files running inside one another.
const MAX_DEPTH: usize = 200;
/// The interpreter thread's stack, ample for `MAX_DEPTH`.
const STACK: usize = 64 << 20;
/// Objects on the operand stack.
const MAX_STACK: usize = 100_000;
/// Dictionaries on the dictionary stack, and saved graphics states.
const MAX_DICTS: usize = 1_000;
const MAX_GSAVE: usize = 1_000;
/// Bytes in one string, elements in one array.
const MAX_STRING: usize = 16 << 20;
const MAX_ARRAY: usize = 1 << 20;
/// Segments in one path, and in everything drawn and saved.
const MAX_PATH: usize = 1 << 20;
const MAX_ART: usize = 1 << 21;
/// Memory the program's strings, arrays and dictionaries may hold at once.
const MAX_LIVE: usize = 256 << 20;
/// The width a zero-width line (PostScript's thinnest) is drawn, in points.
const HAIRLINE: f64 = 0.25;
/// The page of a file that gives no bounding box and draws nothing.
const LETTER: [f64; 4] = [0., 0., 612., 792.];

const TOO_LONG: &str = "it runs too long (the reader stops a program after fifty million steps)";
const TOO_DEEP: &str = "its procedures call one another too deeply";
const TOO_BIG: &str = "it draws more than an import may hold";
const TOO_MUCH: &str = "it needs more memory than an import may use";

/// The PostScript errors errordict answers for.
const ERRORS: &[&str] = &[
    "configurationerror",
    "dictfull",
    "dictstackoverflow",
    "dictstackunderflow",
    "execstackoverflow",
    "interrupt",
    "invalidaccess",
    "invalidexit",
    "invalidfileaccess",
    "invalidfont",
    "invalidrestore",
    "ioerror",
    "limitcheck",
    "nocurrentpoint",
    "rangecheck",
    "stackoverflow",
    "stackunderflow",
    "syntaxerror",
    "timeout",
    "typecheck",
    "undefined",
    "undefinedfilename",
    "undefinedresource",
    "undefinedresult",
    "unmatchedmark",
    "unregistered",
    "VMerror",
];

/// The flat-path SVG of an EPS, PostScript or PostScript-era Illustrator
/// file: its first page (`pages` is 1), with what it held that is not
/// paths named in `skipped`.
pub fn to_svg(bytes: &[u8]) -> Result<Imported, String> {
    // The interpreter recurses as procedures call one another, so it runs
    // on a thread with room for its deepest nesting where there are
    // threads (a browser tab has none, and runs it where it is).
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("eps-import".into())
            .stack_size(STACK)
            .spawn_scoped(scope, || convert(bytes, STEP_LIMIT));
        match thread {
            Ok(thread) => thread
                .join()
                .unwrap_or_else(|_| Err("The PostScript reader failed on this file.".into())),
            Err(_) => convert(bytes, STEP_LIMIT),
        }
    })
}

/// The picture an EPS with no shapes holds: the largest 8-bit gray, RGB or
/// CMYK image its PostScript draws (Photoshop's EPS files), else the preview
/// saved beside the PostScript: the DOS header's TIFF, or an EPSI preview
/// (SageThumbs 2K's reader, tested on real files, gives its conventions:
/// sample 0 is white, the maximum black, rows from the bottom). `None` when
/// it holds no picture either.
pub fn picture(bytes: &[u8]) -> Result<Option<Raster>, String> {
    let drawn = std::thread::scope(|scope| {
        let job = || -> Option<Raster> {
            let program = postscript(bytes).ok()?;
            let header = Header::read(program);
            let mut machine = Machine::new(
                header.bbox.unwrap_or(LETTER),
                header.illustrator,
                STEP_LIMIT,
            );
            machine.art.keep_picture = true;
            let _ = machine.run(program);
            machine.art.picture.take().map(captured_raster)
        };
        match std::thread::Builder::new()
            .name("eps-picture".into())
            .stack_size(STACK)
            .spawn_scoped(scope, job)
        {
            Ok(thread) => thread.join().ok().flatten(),
            Err(_) => job(),
        }
    });
    if drawn.is_some() {
        return Ok(drawn);
    }
    if let Some(preview) = tiff_preview(bytes) {
        return preview.map(Some);
    }
    Ok(epsi_preview(bytes))
}

fn captured_raster(image: Captured) -> Raster {
    let row = image.width * image.comps;
    let mut pixels = Vec::with_capacity(image.width * image.height);
    for y in 0..image.height {
        let y = if image.upward {
            image.height - 1 - y
        } else {
            y
        };
        for x in 0..image.width {
            let c = &image.data[y * row + x * image.comps..][..image.comps];
            let rgb = match image.comps {
                1 => [c[0], c[0], c[0]],
                3 => [c[0], c[1], c[2]],
                _ => {
                    let k = 255 - u16::from(c[3]);
                    let v = |i: usize| ((255 - u16::from(c[i])) * k / 255) as u8;
                    [v(0), v(1), v(2)]
                }
            };
            pixels.push(Rgba([rgb[0], rgb[1], rgb[2], 255]));
        }
    }
    Raster {
        width: image.width,
        height: image.height,
        pixels,
    }
}

/// The TIFF preview a DOS EPS binary header points at, decoded.
fn tiff_preview(bytes: &[u8]) -> Option<Result<Raster, String>> {
    if !bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) || bytes.len() < 30 {
        return None;
    }
    let word = |at: usize| {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize
    };
    let (start, length) = (word(20), word(24));
    let tiff = bytes
        .get(start..start.checked_add(length)?)
        .filter(|t| !t.is_empty())?;
    Some(
        image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff)
            .map(|image| {
                let image = image.to_rgba8();
                Raster {
                    width: image.width() as usize,
                    height: image.height() as usize,
                    pixels: image.pixels().map(|p| Rgba(p.0)).collect(),
                }
            })
            .map_err(|e| format!("This EPS's preview picture could not be read: {e}")),
    )
}

/// An EPSI preview (`%%BeginPreview: width height depth lines`, hex rows in
/// comment lines), gray.
fn epsi_preview(bytes: &[u8]) -> Option<Raster> {
    let text = &bytes[..bytes.len().min(64 << 20)];
    let at = text.windows(15).position(|w| w == b"%%BeginPreview:")?;
    let rest = &text[at + 15..];
    let line_end = rest.iter().position(|&b| b == b'\n' || b == b'\r')?;
    let fields: Vec<usize> = std::str::from_utf8(&rest[..line_end])
        .ok()?
        .split_whitespace()
        .filter_map(|f| f.parse().ok())
        .collect();
    let [width, height, depth, ..] = fields[..] else {
        return None;
    };
    if width == 0 || height == 0 || !matches!(depth, 1 | 2 | 4 | 8) || width * height > 50_000_000 {
        return None;
    }
    let row_bytes = (width * depth).div_ceil(8);
    let mut packed = Vec::with_capacity(row_bytes * height);
    let mut high: Option<u8> = None;
    for line in rest[line_end..].split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.starts_with(b"%%EndPreview") || packed.len() >= row_bytes * height {
            break;
        }
        for &b in line.strip_prefix(b"%").unwrap_or(line) {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => continue,
            };
            match high.take() {
                Some(h) => packed.push(h << 4 | digit),
                None => high = Some(digit),
            }
        }
    }
    if packed.len() < row_bytes * height {
        return None;
    }
    let max = (1u16 << depth) - 1;
    let mut pixels = Vec::with_capacity(width * height);
    for row in packed[..row_bytes * height].chunks_exact(row_bytes).rev() {
        for x in 0..width {
            let bit = x * depth;
            let value = u16::from(row[bit / 8] >> (8 - depth - bit % 8)) & max;
            let grey = 255 - (value * 255 / max) as u8;
            pixels.push(Rgba([grey, grey, grey, 255]));
        }
    }
    Some(Raster {
        width,
        height,
        pixels,
    })
}

fn convert(bytes: &[u8], limit: u64) -> Result<Imported, String> {
    let program = postscript(bytes)?;
    let header = Header::read(program);
    let mut machine = Machine::new(header.bbox.unwrap_or(LETTER), header.illustrator, limit);
    let failure = machine.run(program)?;
    let art = std::mem::take(&mut machine.art);
    drop(machine);
    if let (Some(words), true) = (&failure, art.shapes.is_empty()) {
        return Err(format!("This PostScript file could not be read: {words}."));
    }
    let frame = header.bbox.or_else(|| art.extent()).unwrap_or(LETTER);
    let mut skipped = art.notes();
    if let Some(words) = failure {
        skipped.push(format!("everything after an error in the file ({words})"));
    }
    if header.pages > 1 {
        let more = header.pages - 1;
        skipped.push(format!(
            "{more} more page{} (only the first is read)",
            if more == 1 { "" } else { "s" }
        ));
    }
    Ok(Imported {
        svg: svg(&art.shapes, frame),
        pages: 1,
        skipped,
    })
}

/// The PostScript in `bytes`: the section a DOS EPS binary header points at
/// (its Windows metafile and TIFF previews are not read), or the whole
/// file, which must begin `%!`.
fn postscript(bytes: &[u8]) -> Result<&[u8], String> {
    let section = if bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) {
        let word = |at: usize| {
            bytes
                .get(at..at + 4)
                .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as usize)
        };
        let (Some(start), Some(length)) = (word(4), word(8)) else {
            return Err("The EPS file's binary header is cut short.".into());
        };
        start
            .checked_add(length)
            .and_then(|end| bytes.get(start..end))
            .ok_or("The EPS file's binary header points past the end of the file.")?
    } else {
        bytes
    };
    let start = section
        .iter()
        .position(|b| !matches!(b, 4 | b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(section.len());
    let section = &section[start..];
    let section = section.strip_prefix(b"\xef\xbb\xbf").unwrap_or(section);
    if !section.starts_with(b"%!") {
        return Err("This is not a PostScript or EPS file.".into());
    }
    Ok(section)
}

/// What the file's structuring comments say.
struct Header {
    bbox: Option<[f64; 4]>,
    pages: usize,
    illustrator: bool,
}

/// A bounding box comment as the file goes: the first with numbers, or,
/// when the header puts it off with `(atend)`, the last.
#[derive(Default)]
struct BoxComment {
    first: Option<Option<[f64; 4]>>,
    last: Option<[f64; 4]>,
}

impl BoxComment {
    fn see(&mut self, text: &[u8]) {
        let parsed = parse_box(text);
        if self.first.is_none() {
            self.first = Some(parsed);
        }
        if parsed.is_some() {
            self.last = parsed;
        }
    }

    fn result(&self) -> Option<[f64; 4]> {
        match self.first {
            Some(Some(found)) => Some(found),
            Some(None) => self.last,
            None => None,
        }
    }
}

fn parse_box(text: &[u8]) -> Option<[f64; 4]> {
    let numbers: Vec<f64> = std::str::from_utf8(text)
        .ok()?
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match numbers[..] {
        [l, b, r, t]
            if [l, b, r, t].iter().all(|v| v.is_finite())
                && r > l
                && t > b
                && r - l < 1e6
                && t - b < 1e6 =>
        {
            Some([l, b, r, t])
        }
        _ => None,
    }
}

impl Header {
    fn read(program: &[u8]) -> Self {
        let (mut hires, mut plain) = (BoxComment::default(), BoxComment::default());
        let mut pages = None;
        let mut illustrator = false;
        for line in program.split(|b| *b == b'\r' || *b == b'\n') {
            if !line.starts_with(b"%") {
                continue;
            }
            if line.starts_with(b"%AI") || line.starts_with(b"%%AI") {
                illustrator = true;
            } else if let Some(rest) = line.strip_prefix(b"%%Creator:") {
                illustrator |= rest.windows(11).any(|w| w == b"Illustrator");
            } else if let Some(rest) = line.strip_prefix(b"%%HiResBoundingBox:") {
                hires.see(rest);
            } else if let Some(rest) = line.strip_prefix(b"%%BoundingBox:") {
                plain.see(rest);
            } else if let Some(rest) = line.strip_prefix(b"%%Pages:") {
                if pages.is_none() {
                    pages = std::str::from_utf8(rest)
                        .ok()
                        .and_then(|t| t.split_whitespace().next()?.parse().ok());
                }
            }
        }
        Self {
            bbox: hires.result().or_else(|| plain.result()),
            pages: pages.unwrap_or(1),
            illustrator,
        }
    }
}

/// How a program's run ends when it does not simply finish.
enum Fault {
    /// A PostScript error, by its name, not yet handed to errordict.
    Error(&'static str),
    Stop,
    Exit,
    /// `quit`, or `showpage`: the page is done.
    Quit,
    /// A limit this reader sets on hostile or runaway files, in plain
    /// words; nothing in the program can catch it.
    Limit(&'static str),
}

type Res<T = ()> = Result<T, Fault>;

/// An image's samples as the program drew them: 8 bits a component, `comps`
/// components (gray, RGB or CMYK), rows from the top unless `upward`.
struct Captured {
    width: usize,
    height: usize,
    comps: usize,
    upward: bool,
    data: Vec<u8>,
}

/// What the program painted, and what it did that the import leaves out.
#[derive(Default)]
struct Art {
    /// The largest image drawn, kept only when `keep_picture` asked for it:
    /// the picture an EPS with no shapes offers (Photoshop's EPS files draw
    /// one image and nothing else).
    picture: Option<Captured>,
    keep_picture: bool,
    shapes: Vec<Shape>,
    segments: usize,
    text: bool,
    images: usize,
    clipped: bool,
    dashed: bool,
    patterns: bool,
    shading: bool,
    /// Names an Illustrator file used that this reader does not know.
    unknown: BTreeSet<String>,
}

impl Art {
    fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.text {
            notes.push("text".to_owned());
        }
        match self.images {
            0 => {}
            1 => notes.push("1 image".to_owned()),
            n => notes.push(format!("{n} images")),
        }
        if self.shading {
            notes.push("gradients".to_owned());
        }
        if self.patterns {
            notes.push("patterns, drawn in one colour".to_owned());
        }
        if self.dashed {
            notes.push("dashed lines, drawn solid".to_owned());
        }
        if self.clipped {
            notes.push("clipping, not applied".to_owned());
        }
        if !self.unknown.is_empty() {
            let names: Vec<&str> = self.unknown.iter().map(String::as_str).collect();
            notes.push(format!(
                "Illustrator operators it does not know: {}",
                names.join(" ")
            ));
        }
        notes
    }

    /// The bounds of everything drawn, for a file that gives no bounding
    /// box.
    fn extent(&self) -> Option<[f64; 4]> {
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for seg in self.shapes.iter().flat_map(|s| &s.path) {
            let points: &[(f64, f64)] = match seg {
                Seg::Move(p) | Seg::Line(p) => std::slice::from_ref(p),
                Seg::Curve(a, b, p) => &[*a, *b, *p],
                Seg::Close => &[],
            };
            for p in points {
                bounds = [
                    bounds[0].min(p.0),
                    bounds[1].min(p.1),
                    bounds[2].max(p.0),
                    bounds[3].max(p.1),
                ];
            }
        }
        (bounds[2] > bounds[0] && bounds[3] > bounds[1]).then_some(bounds)
    }
}

/// The interpreter.
struct Machine {
    stack: Vec<Obj>,
    dicts: Vec<Dict>,
    systemdict: Dict,
    userdict: Dict,
    errordict: Dict,
    /// `$error`.
    error_info: Dict,
    /// FontDirectory.
    fonts: Dict,
    /// Resources defined, by category.
    resources: Dict,
    internal: Dict,
    gs: GState,
    /// Saved graphics states, and the `save` each belongs to.
    gstack: Vec<(GState, Option<u32>)>,
    saves: Vec<u32>,
    next_save: u32,
    /// Path segments held by saved graphics states.
    saved_segments: usize,
    /// While a pattern's tiles are drawn over a rectangle (device space:
    /// left, bottom, right, top): only what lies wholly inside it is kept.
    within: Option<[f64; 4]>,
    /// The files being run, `currentfile` last.
    files: Vec<File>,
    /// The operands of the operators running, restored when one fails.
    snap: Vec<Obj>,
    steps: u64,
    limit: u64,
    depth: usize,
    work: Rc<Work>,
    live_base: usize,
    page: [f64; 4],
    /// The file is Illustrator's.
    illustrator: bool,
    /// An Illustrator compound path being gathered: how it is painted.
    compound: Option<(bool, bool)>,
    /// Illustrator's fill rule.
    evenodd: bool,
    packing: bool,
    seed: u32,
    art: Art,
}

impl Machine {
    fn new(page: [f64; 4], illustrator: bool, limit: u64) -> Self {
        let live_base = object::live();
        let systemdict = Dict::new(600);
        for (index, (name, _, _)) in ops::all().enumerate() {
            if !name.starts_with('.') {
                systemdict.set(name, Obj::Op(index as u16));
            }
        }
        let userdict = Dict::new(200);
        let globaldict = Dict::new(64);
        let errordict = Dict::new(40);
        for name in ERRORS {
            errordict.set(name, Obj::Op(ops::ERROR));
        }
        errordict.set("handleerror", Obj::Op(ops::HANDLE_ERROR));
        let error_info = Dict::new(16);
        for (key, value) in [
            ("newerror", Obj::Bool(false)),
            ("errorname", Obj::Null),
            ("command", Obj::Null),
            ("errorinfo", Obj::Null),
            ("ostack", Obj::Null),
            ("estack", Obj::Null),
            ("dstack", Obj::Null),
            ("recordstacks", Obj::Bool(false)),
            ("binary", Obj::Bool(false)),
        ] {
            error_info.set(key, value);
        }
        let statusdict = Dict::new(16);
        for (key, value) in [
            ("product", Obj::string(b"VectorMagik")),
            ("revision", Obj::Int(1)),
            ("jobname", Obj::string(b"")),
            ("waittimeout", Obj::Int(300)),
            ("manualfeed", Obj::Bool(false)),
            // A colour device, so a Photoshop EPS draws its composite in
            // colour (it asks `statusdict /processcolors get exec 1 gt`)
            // rather than one gray separation plate.
            (
                "processcolors",
                Obj::Array(Arr::new(vec![Obj::Int(4)], true)),
            ),
            ("setjobtimeout", Obj::Op(ops::find("setjobtimeout"))),
            ("setpapertray", Obj::Op(ops::find("setjobtimeout"))),
            ("setduplexmode", Obj::Op(ops::find("setjobtimeout"))),
        ] {
            statusdict.set(key, value);
        }
        let serverdict = Dict::new(4);
        serverdict.set("exitserver", Obj::Op(ops::find("setjobtimeout")));
        let fonts = Dict::new(64);
        let encoding = standard_encoding();
        for (key, value) in [
            ("systemdict", Obj::Dict(systemdict.clone())),
            ("userdict", Obj::Dict(userdict.clone())),
            ("globaldict", Obj::Dict(globaldict.clone())),
            ("errordict", Obj::Dict(errordict.clone())),
            ("$error", Obj::Dict(error_info.clone())),
            ("statusdict", Obj::Dict(statusdict)),
            ("serverdict", Obj::Dict(serverdict)),
            ("FontDirectory", Obj::Dict(fonts.clone())),
            ("GlobalFontDirectory", Obj::Dict(fonts.clone())),
            ("SharedFontDirectory", Obj::Dict(fonts.clone())),
            ("StandardEncoding", encoding.clone()),
            ("ISOLatin1Encoding", encoding),
            // A value rather than an operator, as Ghostscript has it:
            // Illustrator's prologs compare `systemdict /languagelevel get`.
            ("languagelevel", Obj::Int(2)),
            ("true", Obj::Bool(true)),
            ("false", Obj::Bool(false)),
            ("null", Obj::Null),
        ] {
            systemdict.set(key, value);
        }
        systemdict.make_readonly();
        Self {
            stack: Vec::new(),
            dicts: vec![systemdict.clone(), globaldict, userdict.clone()],
            systemdict,
            userdict,
            errordict,
            error_info,
            fonts,
            resources: Dict::new(16),
            internal: Dict::new(16),
            gs: GState::new(),
            gstack: Vec::new(),
            saves: Vec::new(),
            next_save: 0,
            saved_segments: 0,
            within: None,
            files: Vec::new(),
            snap: Vec::new(),
            steps: 0,
            limit,
            depth: 0,
            work: Work::new(limit.saturating_mul(16)),
            live_base,
            page,
            illustrator,
            compound: None,
            evenodd: false,
            packing: false,
            seed: 0,
            art: Art::default(),
        }
    }

    /// Runs the program: `Ok(None)` when it ends, `Ok(Some(words))` when an
    /// error nothing caught ended it, `Err` when it broke a limit.
    fn run(&mut self, program: &[u8]) -> Result<Option<String>, String> {
        let file = Stream::bytes(program.into(), &self.work);
        let result = self.run_stream(file, true);
        // A filter past its budget fails its read; the step after it would
        // have stopped the program, so the words say that.
        if self.steps.saturating_add(self.work.done() / 16) > self.limit {
            return Err(format!("This PostScript file was not read: {TOO_LONG}."));
        }
        match result {
            Ok(()) | Err(Fault::Quit) => Ok(None),
            Err(Fault::Limit(words)) => Err(format!("This PostScript file was not read: {words}.")),
            Err(Fault::Stop | Fault::Exit) => Ok(Some(self.error_words())),
            Err(Fault::Error(name)) => Ok(Some(words(name, ""))),
        }
    }

    /// The error recorded in `$error`, in plain words.
    fn error_words(&self) -> String {
        if !matches!(self.error_info.find(b"newerror"), Some(Obj::Bool(true))) {
            return "it stopped itself".to_owned();
        }
        let name = self
            .error_info
            .find(b"errorname")
            .and_then(|n| n.text())
            .unwrap_or_default();
        let name = String::from_utf8_lossy(&name);
        if name == "undefinedresource" && self.stack.len() >= 2 {
            let asked: Vec<String> = self.stack[self.stack.len() - 2..]
                .iter()
                .filter_map(|obj| obj.text())
                .map(|text| String::from_utf8_lossy(&text).into_owned())
                .collect();
            return format!(
                "it needs the resource {}, which this reader does not have",
                asked.join(" ")
            );
        }
        let command = match self.error_info.find(b"command") {
            Some(Obj::Name(text, _)) => String::from_utf8_lossy(&text).into_owned(),
            Some(Obj::Op(op)) => {
                let operator = ops::entry(op).0;
                if name == "undefined" {
                    let key = self.stack.last().and_then(|key| key.text());
                    let key = key.map(|k| format!(" {}", String::from_utf8_lossy(&k)));
                    return format!(
                        "a name it looks up with \"{operator}\"{} is not defined",
                        key.unwrap_or_default()
                    );
                }
                operator.to_owned()
            }
            _ => String::new(),
        };
        words(&name, &command)
    }

    /// Counts one step, and checks the limits a step can break.
    fn step(&mut self) -> Res {
        self.steps += 1;
        if self.steps.saturating_add(self.work.done() / 16) > self.limit {
            return Err(Fault::Limit(TOO_LONG));
        }
        if self.stack.len() > MAX_STACK {
            return Err(Fault::Limit(TOO_MUCH));
        }
        if object::live().saturating_sub(self.live_base) > MAX_LIVE {
            return Err(Fault::Limit(TOO_MUCH));
        }
        Ok(())
    }

    fn enter(&mut self) -> Res {
        if self.depth >= MAX_DEPTH {
            return Err(Fault::Limit(TOO_DEEP));
        }
        self.depth += 1;
        Ok(())
    }

    /// Executes an object met in a program: procedures and literal objects
    /// are pushed, names looked up and run.
    fn run_object(&mut self, obj: Obj) -> Res {
        self.step()?;
        match obj {
            Obj::Name(name, true) => self.run_name(&name),
            Obj::Op(op) => self.call_op(op),
            Obj::Str(text) if text.exec => self.run_text(&text),
            Obj::File(file, true) => self.run_stream(file, true),
            other => {
                self.stack.push(other);
                Ok(())
            }
        }
    }

    /// Executes an object as `exec` does: a procedure runs.
    fn exec(&mut self, obj: Obj) -> Res {
        match obj {
            Obj::Array(procedure) if procedure.exec => self.run_proc(&procedure),
            other => self.run_object(other),
        }
    }

    fn run_name(&mut self, name: &Name) -> Res {
        match self.lookup(name) {
            Some(Obj::Array(procedure)) if procedure.exec => self.run_proc(&procedure),
            Some(Obj::Op(op)) => self.call_op(op),
            Some(value) if value.executable() => {
                self.enter()?;
                let result = self.run_object(value);
                self.depth -= 1;
                result
            }
            Some(value) => {
                self.stack.push(value);
                Ok(())
            }
            None => self.undefined(name),
        }
    }

    fn run_proc(&mut self, procedure: &Arr) -> Res {
        self.enter()?;
        let mut result = Ok(());
        for index in 0..procedure.len {
            result = self.run_object(procedure.get(index));
            if result.is_err() {
                break;
            }
        }
        self.depth -= 1;
        result
    }

    /// Runs a string as a program.
    fn run_text(&mut self, text: &Str) -> Res {
        let file = Stream::bytes(text.to_vec().into(), &self.work);
        self.run_stream(file, false)
    }

    /// Runs the tokens of a file; `current` makes it `currentfile`.
    fn run_stream(&mut self, file: File, current: bool) -> Res {
        self.enter()?;
        if current {
            self.files.push(file.clone());
        }
        let result = loop {
            let token = match self.token(&file) {
                Ok(Some(token)) => token,
                Ok(None) => break Ok(()),
                Err(Fault::Error(error)) => {
                    match self.raise(error, Obj::File(file.clone(), true)) {
                        Ok(()) => continue,
                        Err(fault) => break Err(fault),
                    }
                }
                Err(fault) => break Err(fault),
            };
            if let Err(fault) = self.run_object(token) {
                break Err(fault);
            }
        };
        if current {
            self.files.pop();
        }
        self.depth -= 1;
        match result {
            Err(Fault::Exit) => self.raise("invalidexit", Obj::command("exit")),
            other => other,
        }
    }

    /// Runs an operator; when it fails, its operands go back on the stack
    /// and the error goes to errordict.
    fn call_op(&mut self, op: u16) -> Res {
        let (_, run, arity) = ops::entry(op);
        let mark = self.snap.len();
        let base = self.stack.len().saturating_sub(usize::from(arity));
        self.snap.extend_from_slice(&self.stack[base..]);
        let result = match run(self) {
            Err(Fault::Error(error)) => {
                self.stack.truncate(base);
                self.stack.extend(self.snap.drain(mark..));
                self.raise(error, Obj::Op(op))
            }
            other => other,
        };
        self.snap.truncate(mark);
        result
    }

    /// Hands an error to errordict: its standard handler records it in
    /// `$error` and stops; a program's own handler runs with the offending
    /// object on the stack.
    fn raise(&mut self, error: &'static str, command: Obj) -> Res {
        self.error_info.set("newerror", Obj::Bool(true));
        self.error_info.set("errorname", Obj::name(error));
        self.error_info.set("command", command.clone());
        match self.errordict.find(error.as_bytes()) {
            Some(Obj::Op(ops::ERROR)) | None => Err(Fault::Stop),
            Some(handler) => {
                self.stack.push(command);
                self.exec(handler)
            }
        }
    }

    /// An undefined name: Illustrator's operator of that name, or, at the
    /// top level of an Illustrator file, skipped and noted (a procedure
    /// set the file leaves out answers with an empty one); otherwise the
    /// `undefined` error.
    fn undefined(&mut self, name: &Name) -> Res {
        if let Some(result) = illustrator::operator(self, name) {
            return result;
        }
        if self.illustrator && self.depth <= 1 {
            if name.starts_with(b"Adobe_") {
                let procset = Dict::new(4);
                procset.set("initialize", Obj::nothing());
                procset.set("terminate", Obj::nothing());
                self.userdict
                    .put(Obj::Name(name.clone(), false), Obj::Dict(procset.clone()))
                    .map_err(Fault::Error)?;
                self.stack.push(Obj::Dict(procset));
            } else if self.art.unknown.len() < 24 {
                self.art
                    .unknown
                    .insert(String::from_utf8_lossy(name).into_owned());
            }
            return Ok(());
        }
        self.raise("undefined", Obj::Name(name.clone(), true))
    }

    fn lookup(&self, name: &[u8]) -> Option<Obj> {
        self.dicts.iter().rev().find_map(|dict| dict.find(name))
    }

    fn lookup_key(&self, key: &Obj) -> Option<Obj> {
        self.dicts.iter().rev().find_map(|dict| dict.get(key))
    }

    fn where_key(&self, key: &Obj) -> Option<Dict> {
        self.dicts
            .iter()
            .rev()
            .find(|dict| dict.get(key).is_some())
            .cloned()
    }

    fn current_dict(&self) -> Dict {
        self.dicts
            .last()
            .cloned()
            .unwrap_or_else(|| self.userdict.clone())
    }

    fn current_file(&self) -> File {
        match self.files.last() {
            Some(file) => file.clone(),
            None => Stream::bytes(Vec::new().into(), &self.work),
        }
    }

    fn def(&mut self, key: Obj, value: Obj) -> Res {
        self.current_dict().put(key, value).map_err(Fault::Error)
    }

    fn push(&mut self, obj: Obj) {
        self.stack.push(obj);
    }

    /// Pushes an operator's result.
    fn answer(&mut self, obj: Obj) -> Res {
        self.stack.push(obj);
        Ok(())
    }

    /// Pushes a real, refusing infinities and NaN.
    fn real(&mut self, value: f64) -> Res {
        if !value.is_finite() {
            return Err(Fault::Error("undefinedresult"));
        }
        self.answer(Obj::Real(value))
    }

    fn pop(&mut self) -> Res<Obj> {
        self.stack.pop().ok_or(Fault::Error("stackunderflow"))
    }

    fn top(&self) -> Res<&Obj> {
        self.stack.last().ok_or(Fault::Error("stackunderflow"))
    }

    fn drop_n(&mut self, count: usize) -> Res {
        if self.stack.len() < count {
            return Err(Fault::Error("stackunderflow"));
        }
        self.stack.truncate(self.stack.len() - count);
        Ok(())
    }

    fn pop_num(&mut self) -> Res<f64> {
        self.pop()?.num().ok_or(Fault::Error("typecheck"))
    }

    fn pop_int(&mut self) -> Res<i32> {
        match self.pop()? {
            Obj::Int(value) => Ok(value),
            _ => Err(Fault::Error("typecheck")),
        }
    }

    /// A non-negative count no larger than `limit`.
    fn pop_count(&mut self, limit: usize) -> Res<usize> {
        let count = self.pop_int()?;
        let count = usize::try_from(count).map_err(|_| Fault::Error("rangecheck"))?;
        if count > limit {
            return Err(Fault::Error("limitcheck"));
        }
        Ok(count)
    }

    fn pop_bool(&mut self) -> Res<bool> {
        match self.pop()? {
            Obj::Bool(value) => Ok(value),
            _ => Err(Fault::Error("typecheck")),
        }
    }

    fn pop_str(&mut self) -> Res<Str> {
        match self.pop()? {
            Obj::Str(value) => Ok(value),
            _ => Err(Fault::Error("typecheck")),
        }
    }

    fn pop_arr(&mut self) -> Res<Arr> {
        match self.pop()? {
            Obj::Array(value) => Ok(value),
            _ => Err(Fault::Error("typecheck")),
        }
    }

    fn pop_dict(&mut self) -> Res<Dict> {
        match self.pop()? {
            Obj::Dict(value) => Ok(value),
            _ => Err(Fault::Error("typecheck")),
        }
    }

    fn pop_file(&mut self) -> Res<File> {
        match self.pop()? {
            Obj::File(value, _) => Ok(value),
            _ => Err(Fault::Error("typecheck")),
        }
    }

    /// A procedure operand (an array; operators given a literal one run it
    /// all the same).
    fn pop_proc(&mut self) -> Res<Obj> {
        match self.pop()? {
            Obj::Array(procedure) => Ok(Obj::Array(procedure)),
            _ => Err(Fault::Error("typecheck")),
        }
    }

    /// The bytes of a name or string operand.
    fn pop_name(&mut self) -> Res<Vec<u8>> {
        self.pop()?.text().ok_or(Fault::Error("typecheck"))
    }

    /// Where the topmost mark is on the stack.
    fn mark_at(&self) -> Res<usize> {
        self.stack
            .iter()
            .rposition(|obj| matches!(obj, Obj::Mark))
            .ok_or(Fault::Error("unmatchedmark"))
    }

    fn new_array(&self, items: Vec<Obj>, exec: bool) -> Res<Arr> {
        if items.len() > MAX_ARRAY {
            return Err(Fault::Error("limitcheck"));
        }
        Ok(Arr::new(items, exec))
    }
}

impl Drop for Machine {
    /// Empties everything the program can reach, so the cycles it built
    /// (a dictionary holding itself) are freed with it.
    fn drop(&mut self) {
        let mut roots = std::mem::take(&mut self.stack);
        roots.extend(self.dicts.drain(..).map(Obj::Dict));
        roots.append(&mut self.snap);
        for dict in [
            &self.systemdict,
            &self.userdict,
            &self.errordict,
            &self.error_info,
            &self.fonts,
            &self.resources,
            &self.internal,
        ] {
            roots.push(Obj::Dict(dict.clone()));
        }
        roots.extend(self.gs.objects());
        for (state, _) in self.gstack.drain(..) {
            roots.extend(state.objects());
        }
        object::dismantle(roots);
    }
}

/// An error that ended the program, in plain words.
fn words(error: &str, command: &str) -> String {
    match error {
        "undefined" => format!("it uses \"{command}\", which this reader does not know"),
        "syntaxerror" => "its PostScript is malformed".to_owned(),
        "stackunderflow" => format!("\"{command}\" is missing its operands"),
        "typecheck" | "rangecheck" => format!("\"{command}\" was given the wrong kind of operand"),
        "limitcheck" | "VMerror" => TOO_MUCH.to_owned(),
        "ioerror" => "its encoded data is damaged".to_owned(),
        "undefinedfilename" | "invalidfileaccess" => {
            "it reads other files, which an import does not allow".to_owned()
        }
        "" => "it stopped with an error".to_owned(),
        other => format!("PostScript error {other} in \"{command}\""),
    }
}

/// StandardEncoding, as far as the printable ASCII range (text is not
/// drawn, so only prologs that copy or look into encodings need it).
fn standard_encoding() -> Obj {
    const PUNCTUATION: [&str; 32] = [
        "space",
        "exclam",
        "quotedbl",
        "numbersign",
        "dollar",
        "percent",
        "ampersand",
        "quoteright",
        "parenleft",
        "parenright",
        "asterisk",
        "plus",
        "comma",
        "hyphen",
        "period",
        "slash",
        "colon",
        "semicolon",
        "less",
        "equal",
        "greater",
        "question",
        "at",
        "bracketleft",
        "backslash",
        "bracketright",
        "asciicircum",
        "underscore",
        "quoteleft",
        "braceleft",
        "bar",
        "braceright",
    ];
    const DIGITS: [&str; 10] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    let mut names: Vec<Obj> = vec![Obj::name(".notdef"); 256];
    let mut punctuation = PUNCTUATION.iter();
    for code in 32u8..127 {
        let name = match code {
            b'0'..=b'9' => DIGITS[usize::from(code - b'0')].to_owned(),
            b'A'..=b'Z' | b'a'..=b'z' => char::from(code).to_string(),
            b'~' => "asciitilde".to_owned(),
            _ => punctuation.next().copied().unwrap_or(".notdef").to_owned(),
        };
        names[usize::from(code)] = Obj::name(&name);
    }
    Obj::Array(Arr::new(names, false))
}

/// A number as the SVG writes it: at most three decimals, no trailing
/// zeros.
fn num(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".to_owned(),
        _ => text.to_owned(),
    }
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The shapes as the flat SVG `import.rs` describes: the page `frame`
/// (left, bottom, right, top in points) becomes the view box, y flipped.
fn svg(shapes: &[Shape], frame: [f64; 4]) -> String {
    const JOINS: [&str; 3] = ["miter", "round", "bevel"];
    const CAPS: [&str; 3] = ["butt", "round", "square"];
    let [left, _, right, top] = frame;
    let (width, height) = (num(right - left), num(top - frame[1]));
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}pt\" height=\"{height}pt\" \
         viewBox=\"0 0 {width} {height}\">\n"
    );
    let at = |(x, y): (f64, f64)| format!("{} {}", num(x - left), num(top - y));
    for shape in shapes {
        out.push_str("<path d=\"");
        for (i, seg) in shape.path.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            let _ = match seg {
                Seg::Move(p) => write!(out, "M {}", at(*p)),
                Seg::Line(p) => write!(out, "L {}", at(*p)),
                Seg::Curve(a, b, p) => write!(out, "C {} {} {}", at(*a), at(*b), at(*p)),
                Seg::Close => write!(out, "Z"),
            };
        }
        out.push('"');
        match shape.fill {
            Some((colour, evenodd)) => {
                let _ = write!(out, " fill=\"{}\"", hex(colour));
                if evenodd {
                    out.push_str(" fill-rule=\"evenodd\"");
                }
            }
            None => out.push_str(" fill=\"none\""),
        }
        if let Some(stroke) = &shape.stroke {
            let _ = write!(
                out,
                " stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"{}\" stroke-linecap=\"{}\"",
                hex(stroke.colour),
                num(stroke.width),
                JOINS[usize::from(stroke.join.min(2))],
                CAPS[usize::from(stroke.cap.min(2))],
            );
        }
        out.push_str("/>\n");
    }
    out.push_str("</svg>\n");
    out
}
