//! VectorMagik in the browser: the desktop's conversion chain compiled to
//! WebAssembly and run in a Web Worker, so a picture is traced on the
//! visitor's own computer and never uploaded (the owner, September 23, 2026:
//! "fully browser ran ... it does all of the compute on their computer").
//!
//! The chain is the command line's `--category auto --quality auto
//! --simplify auto --regularize 0.8 --straighten auto --primitives on --stack
//! on` (which is the desktop's), through the same library calls, so the
//! browser draws what the app draws. The last trace is kept, so a changed
//! setting that needs no new trace (simplify, straighten, true lines and
//! shapes) derives again in a moment.
//!
//! The interface is a handful of plain exports over the module's memory, no
//! binding generator: JavaScript copies the file and a settings string in
//! (`vm_alloc`), calls `vm_convert` or `vm_derive`, then `vm_output` for the
//! statistics (JSON), the SVG, a PDF or an EPS, and reads them back through
//! `vm_out_ptr` / `vm_out_len`. Settings are `key=value` pairs joined by `;`
//! (see `Settings::parse`).

use std::cell::RefCell;

use vector_magic_rebuild::{auto, engine, export, pdf_eps, Preparation};
use vector_rebuild::raster::Raster;
use vector_rebuild::straighten::StraightenOptions;
use vector_rebuild::{ImageCategory, Quality};

/// The largest picture the browser build opens. The desktop takes 100
/// megapixels; a browser tab's memory is the tighter bound here, and the
/// engine traces at most 16 million pixels either way.
pub const MAX_PIXELS: u64 = 50_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Simplify {
    Auto,
    Off,
    Tolerance(f64),
}

/// What a conversion runs with. Everything defaults to the desktop's
/// defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub category: Option<ImageCategory>,
    pub quality: Option<Quality>,
    pub colors: Option<usize>,
    pub background: Option<[u8; 3]>,
    pub simplify: Simplify,
    pub straighten: Option<StraightenOptions>,
    pub regularize: bool,
    pub primitives: bool,
    pub stack: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            category: None,
            quality: None,
            colors: None,
            background: None,
            simplify: Simplify::Auto,
            straighten: Some(StraightenOptions {
                auto: true,
                ..StraightenOptions::default()
            }),
            regularize: true,
            primitives: true,
            stack: true,
        }
    }
}

impl Settings {
    /// `category=auto|blended|unblended|photo; quality=auto|high|medium|low;
    /// colors=all|N; background=keep|white|black|#rrggbb;
    /// simplify=auto|off|PX; straighten=auto|off|PX; regularize=on|off;
    /// primitives=on|off; stack=on|off`, any subset, in any order.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut settings = Self::default();
        for pair in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("Setting without a value: {pair}"))?;
            let (key, value) = (key.trim(), value.trim());
            let on = |value: &str| match value {
                "on" => Ok(true),
                "off" => Ok(false),
                _ => Err(format!("{key} must be on or off")),
            };
            let pixels = |value: &str| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.)
                    .ok_or_else(|| format!("{key} must be a number of pixels"))
            };
            match key {
                "category" => {
                    settings.category = match value {
                        "auto" => None,
                        "blended" => Some(ImageCategory::AntiAliasedArtwork),
                        "unblended" => Some(ImageCategory::AliasedArtwork),
                        "photo" => Some(ImageCategory::Photograph),
                        _ => {
                            return Err("category must be auto, blended, unblended or photo".into())
                        }
                    }
                }
                "quality" => {
                    settings.quality = match value {
                        "auto" => None,
                        "high" => Some(Quality::High),
                        "medium" => Some(Quality::Medium),
                        "low" => Some(Quality::Low),
                        _ => return Err("quality must be auto, high, medium or low".into()),
                    }
                }
                "colors" => {
                    settings.colors = match value {
                        "all" => None,
                        n => Some(
                            n.parse::<usize>()
                                .ok()
                                .filter(|n| (1..=64).contains(n))
                                .ok_or("colors must be all or 1 to 64")?,
                        ),
                    }
                }
                "background" => {
                    settings.background = match value {
                        "keep" => None,
                        rgb => Some(
                            vector_magic_rebuild::parse_rgb(rgb)
                                .ok_or("background must be keep, white, black or #rrggbb")?,
                        ),
                    }
                }
                "simplify" => {
                    settings.simplify = match value {
                        "auto" => Simplify::Auto,
                        "off" => Simplify::Off,
                        px => {
                            let t = pixels(px)?;
                            if !(0.01..=3.).contains(&t) {
                                return Err("simplify must be auto, off or 0.01 to 3 px".into());
                            }
                            Simplify::Tolerance(t)
                        }
                    }
                }
                "straighten" => {
                    settings.straighten = match value {
                        "off" => None,
                        "auto" => Some(StraightenOptions {
                            auto: true,
                            ..StraightenOptions::default()
                        }),
                        px => Some(StraightenOptions {
                            flatness: pixels(px)?,
                            ..StraightenOptions::default()
                        }),
                    }
                }
                "regularize" => settings.regularize = on(value)?,
                "primitives" => settings.primitives = on(value)?,
                "stack" => settings.stack = on(value)?,
                _ => return Err(format!("Unknown setting: {key}")),
            }
        }
        Ok(settings)
    }

    /// Whether going from `self` to `next` needs a new trace (the picture
    /// the engine sees or the preset changed) rather than a new derive.
    pub fn retrace(&self, next: &Settings) -> bool {
        (self.category, self.quality, self.colors, self.background)
            != (next.category, next.quality, next.colors, next.background)
    }
}

/// The last trace and what was drawn from it.
pub struct Session {
    raw: engine::Document,
    traced: (usize, usize),
    loaded_as: Option<(usize, usize)>,
    category: ImageCategory,
    quality: Quality,
    settings: Settings,
    svg: String,
    stats: String,
}

impl Session {
    pub fn svg(&self) -> &str {
        &self.svg
    }
    pub fn stats(&self) -> &str {
        &self.stats
    }
}

/// Decode an image file's bytes (PNG, JPEG, GIF, BMP or PNM).
pub fn decode(bytes: &[u8]) -> Result<Raster, String> {
    let reader = || {
        image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let (w, h) = reader()?.into_dimensions().map_err(|_| {
        "That file is not a picture VectorMagik can open (PNG, JPEG, GIF, BMP or PNM).".to_string()
    })?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!(
            "Pictures of 1 to {} million pixels open here; the Windows app takes up to 100.",
            MAX_PIXELS / 1_000_000
        ));
    }
    let mut decoder = reader()?;
    decoder.no_limits();
    let source = decoder.decode().map_err(|e| e.to_string())?.to_rgba8();
    Ok(Raster {
        width: source.width() as usize,
        height: source.height() as usize,
        pixels: source
            .pixels()
            .map(|p| vector_rebuild::raster::Rgba(p.0))
            .collect(),
    })
}

/// Trace `bytes` with `settings` and draw the result.
pub fn convert(bytes: &[u8], settings: Settings) -> Result<Session, String> {
    let (raster, loaded_as) = vector_magic_rebuild::fit_for_engine(decode(bytes)?)?;
    trace(raster, loaded_as, settings)
}

fn trace(
    raster: Raster,
    loaded_as: Option<(usize, usize)>,
    settings: Settings,
) -> Result<Session, String> {
    let prep = Preparation {
        colors: settings.colors,
        background: settings.background,
        recolors: Vec::new(),
    };
    let prepared = if prep.is_identity() {
        raster
    } else {
        prep.apply(&raster)
    };
    let detected = auto::detect(&prepared);
    let options = engine::Options {
        category: settings.category.unwrap_or(detected.category),
        quality: settings.quality.unwrap_or(detected.quality),
        ..engine::Options::default()
    };
    let mut raw = engine::vectorize_with(&prepared, options, &[])?;
    if let Some(palette) = prep.palette_of(&prepared) {
        raw = raw.snapped(&palette, None);
    }
    let mut session = Session {
        raw,
        traced: (prepared.width, prepared.height),
        loaded_as,
        category: options.category,
        quality: options.quality,
        settings: settings.clone(),
        svg: String::new(),
        stats: String::new(),
    };
    draw(&mut session, settings)?;
    Ok(session)
}

/// Draw the kept trace again with `settings`; a change that needs a new
/// trace is refused (the caller converts again).
pub fn derive(session: &mut Session, settings: Settings) -> Result<(), String> {
    if session.settings.retrace(&settings) {
        return Err("That setting changes the trace; convert again.".into());
    }
    draw(session, settings)
}

fn draw(session: &mut Session, settings: Settings) -> Result<(), String> {
    let (tolerance, doc) = match settings.simplify {
        Simplify::Auto => {
            let (t, doc) = vector_magic_rebuild::auto_simplify_tolerance(&session.raw)?;
            (Some(t), doc)
        }
        Simplify::Tolerance(t) => (Some(t), session.raw.simplified(t)?),
        Simplify::Off => (None, session.raw.clone()),
    };
    let doc = doc.post_passes(
        settings
            .regularize
            .then_some(vector_rebuild::regularize::RegularizeOptions { band: 0.8 }),
        settings.straighten,
        settings.primitives,
        &[],
    )?;
    let drawn = if settings.stack {
        vector_rebuild::stacking::stack_svg(doc.svg())?.0
    } else {
        doc.svg().to_owned()
    };
    // Declared at the picture's own size when it was traced scaled, as the
    // command line saves it.
    let svg = match session.loaded_as {
        Some((width, height)) => {
            let (view_w, view_h) = vector_magic_rebuild::view_box_size(&drawn)
                .ok_or("The drawing has no view box to declare its size by")?;
            let (traced_w, traced_h) = session.traced;
            let declare = |view: f64, picture: usize, traced: usize| {
                ((view * picture as f64 / traced as f64).round() as u32).max(1)
            };
            vector_magic_rebuild::resize_svg(
                &drawn,
                declare(view_w, width, traced_w),
                declare(view_h, height, traced_h),
            )?
        }
        None => drawn,
    };
    let svg = export::compact_svg(&svg);
    let (source_w, source_h) = session.loaded_as.unwrap_or(session.traced);
    let fills: std::collections::BTreeSet<&str> = svg
        .split("fill=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .filter(|fill| *fill != "none")
        .collect();
    session.stats = format!(
        "{{\"category\":\"{}\",\"quality\":\"{:?}\",\"width\":{},\"height\":{},\"traced_width\":{},\"traced_height\":{},\"nodes\":{},\"segments\":{},\"colors\":{},\"simplify\":{},\"bytes\":{}}}",
        auto::category_name(session.category),
        session.quality,
        source_w,
        source_h,
        session.traced.0,
        session.traced.1,
        doc.nodes().len(),
        doc.segment_count(),
        fills.len(),
        tolerance.map_or("null".to_owned(), |t| format!("{t}")),
        svg.len(),
    );
    session.svg = svg;
    session.settings = settings;
    Ok(())
}

/// What `vm_output` can hand back.
pub fn output(session: &Session, kind: u32) -> Result<Vec<u8>, String> {
    match kind {
        0 => Ok(session.stats.as_bytes().to_vec()),
        1 => Ok(session.svg.as_bytes().to_vec()),
        2 => pdf_eps::to_pdf(&session.svg),
        3 => pdf_eps::to_eps(&session.svg),
        _ => Err("Unknown output".into()),
    }
}

// ---- The exports JavaScript calls. -------------------------------------

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn finish(result: Result<Vec<u8>, String>) -> u32 {
    let (code, bytes) = match result {
        Ok(bytes) => (0, bytes),
        Err(message) => (1, message.into_bytes()),
    };
    OUT.with(|out| *out.borrow_mut() = bytes);
    code
}

/// # Safety
/// `ptr` and `len` must describe memory this module handed out.
unsafe fn slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

#[no_mangle]
pub extern "C" fn vm_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// # Safety
/// `ptr` must come from `vm_alloc(len)` and be freed once.
#[no_mangle]
pub unsafe extern "C" fn vm_dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// Trace an image file with a settings string; 0 on success, else 1 with
/// the message in the output buffer.
///
/// # Safety
/// Both ranges must be memory from `vm_alloc` holding that many bytes.
#[no_mangle]
pub unsafe extern "C" fn vm_convert(
    image: *const u8,
    image_len: usize,
    settings: *const u8,
    settings_len: usize,
) -> u32 {
    let bytes = slice(image, image_len);
    let text = String::from_utf8_lossy(slice(settings, settings_len)).into_owned();
    finish(
        Settings::parse(&text)
            .and_then(|s| convert(bytes, s))
            .map(|session| {
                let stats = session.stats.clone().into_bytes();
                SESSION.with(|slot| *slot.borrow_mut() = Some(session));
                stats
            }),
    )
}

/// Draw the kept trace again with a settings string; 0 on success.
///
/// # Safety
/// The range must be memory from `vm_alloc` holding that many bytes.
#[no_mangle]
pub unsafe extern "C" fn vm_derive(settings: *const u8, settings_len: usize) -> u32 {
    let text = String::from_utf8_lossy(slice(settings, settings_len)).into_owned();
    finish(Settings::parse(&text).and_then(|settings| {
        SESSION.with(|slot| match slot.borrow_mut().as_mut() {
            Some(session) => derive(session, settings).map(|_| session.stats.clone().into_bytes()),
            None => Err("Convert a picture first.".into()),
        })
    }))
}

/// Put the statistics (0), the SVG (1), a PDF (2) or an EPS (3) of the last
/// drawing in the output buffer; 0 on success.
#[no_mangle]
pub extern "C" fn vm_output(kind: u32) -> u32 {
    finish(SESSION.with(|slot| match slot.borrow().as_ref() {
        Some(session) => output(session, kind),
        None => Err("Convert a picture first.".into()),
    }))
}

#[no_mangle]
pub extern "C" fn vm_out_ptr() -> *const u8 {
    OUT.with(|out| out.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn vm_out_len() -> usize {
    OUT.with(|out| out.borrow().len())
}

#[cfg(test)]
mod tests;
