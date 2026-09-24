use std::path::Path;
use vector_rebuild::raster::{Raster, Rgba};
pub mod auto;
#[cfg(feature = "ui")]
pub mod desktop_ui;
pub mod engine;
pub mod export;
pub mod pdf_eps;
#[cfg(feature = "desktop")]
pub mod snapshot;

/// Whether saving to `output` would replace `source`: the same path, or an
/// existing file that resolves to it. Every save that has a source asks this
/// one question (the CLI, the snapshot, the desktop's exports and preview).
pub fn same_file(source: &Path, output: &Path) -> bool {
    source == output || output.exists() && source.canonicalize().ok() == output.canonicalize().ok()
}

/// A custom die-cut sticker, `BORDER,RIM[,shadow]` widths in source pixels,
/// as both command lines take it after `--sticker`.
pub fn parse_sticker(spec: &str) -> Result<vector_rebuild::sticker::Sticker, String> {
    const USAGE: &str = "--sticker takes on, off or BORDER,RIM[,shadow]";
    let mut parts = spec.split(',');
    let mut width = || {
        parts
            .next()
            .and_then(|p| p.trim().parse::<f64>().ok())
            .filter(|w| w.is_finite() && *w >= 0.)
            .ok_or(USAGE)
    };
    let (border, edge) = (width()?, width()?);
    let shadow = match parts.next().map(str::trim) {
        None => false,
        Some("shadow") => true,
        _ => return Err(USAGE.into()),
    };
    if parts.next().is_some() {
        return Err(USAGE.into());
    }
    let sticker = vector_rebuild::sticker::Sticker {
        border,
        edge,
        shadow,
        ..Default::default()
    };
    sticker.validate()?;
    Ok(sticker)
}

/// Saves `bytes` as `output` without ever truncating it in place: they go to
/// a hidden sibling first, which is renamed over the target once complete
/// (the rename replaces an existing file on Windows too). A failed save (a
/// full disk, a dropped share) leaves the previous file whole and removes
/// the sibling.
pub(crate) fn write_replacing(output: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    replace_with(output, |file| file.write_all(bytes))
}

/// `write_replacing` with the writing done by `write`.
pub(crate) fn replace_with(
    output: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<(), String> {
    static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = output
        .file_name()
        .ok_or_else(|| format!("{} is not a file name", output.display()))?;
    let mut sibling = std::ffi::OsString::from(".");
    sibling.push(name);
    sibling.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let temporary = output.with_file_name(sibling);
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        write(&mut file)?;
        file.sync_all()
    });
    if let Err(e) = written {
        // A folder that cannot be written to fails here, and closing
        // programs would not help: its own error, as it was.
        let _ = std::fs::remove_file(&temporary);
        return Err(e.to_string());
    }
    // A virus scanner or the search indexer opening the new file for a
    // moment makes the rename fail with access denied or a sharing
    // violation (Windows errors 5 and 32); it goes through once they let
    // go. A program that keeps the target open does not let go, and a
    // folder in the way never will, so it is not waited for.
    let held = |e: &std::io::Error| {
        e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(32)
    };
    let mut wait = std::time::Duration::from_millis(25);
    let renamed = loop {
        match std::fs::rename(&temporary, output) {
            Err(e) if wait.as_millis() <= 800 && held(&e) && !output.is_dir() => {
                std::thread::sleep(wait);
                wait *= 2;
            }
            result => break result,
        }
    };
    renamed.map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        // Only a file already there can be the one held open (round three
        // of the Opus 5.5 review: every permission error said so).
        if held(&e) && output.is_file() {
            format!(
                "{} is open in another program or read-only; close it and save again ({e})",
                output.display()
            )
        } else {
            e.to_string()
        }
    })
}

/// The `viewBox` width and height of the root `<svg>` element: the source
/// size in pixels for an engine document, whose declared width and height
/// are points.
pub fn view_box_size(svg: &str) -> Option<(f64, f64)> {
    let start = svg.find("<svg")?;
    let tag = &svg[start..start + svg[start..].find('>')?];
    let at = tag.find(" viewBox=\"")? + " viewBox=\"".len();
    let value = &tag[at..at + tag[at..].find('"')?];
    let numbers: Vec<f64> = value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f64>().ok())
        .collect::<Option<_>>()?;
    match numbers[..] {
        [_, _, width, height]
            if width > 0. && height > 0. && width.is_finite() && height.is_finite() =>
        {
            Some((width, height))
        }
        _ => None,
    }
}

pub fn load_raster(path: &Path) -> Result<Raster, String> {
    load_raster_up_to(path, 16_000_000)
}

/// The largest picture the desktop opens: it scales anything bigger than the
/// engine takes down to the engine's limits itself, so a camera's 24 or 50
/// megapixels open (round two of the Opus 5.5 review: they were refused
/// before the scaling could run). 100 megapixels decode to about 400 MB (a
/// 16-bit picture to 800 MB first).
pub const DESKTOP_MAX_PIXELS: u64 = 100_000_000;

/// The largest picture the browser build opens: a tab's memory is smaller.
pub const BROWSER_MAX_PIXELS: u64 = 50_000_000;

/// `load_raster` with a pixel limit of `max_pixels`.
pub fn load_raster_up_to(path: &Path, max_pixels: u64) -> Result<Raster, String> {
    decode_up_to(
        || {
            image::ImageReader::open(path)
                .map_err(|e| e.to_string())?
                .with_guessed_format()
                .map_err(|e| e.to_string())
        },
        max_pixels,
    )
}

/// A picture from its file's `bytes` (a file dropped in a browser tab), with
/// a pixel limit of `max_pixels`.
pub fn decode_raster_up_to(bytes: &[u8], max_pixels: u64) -> Result<Raster, String> {
    decode_up_to(
        || {
            image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| e.to_string())
        },
        max_pixels,
    )
}

fn decode_up_to<R: std::io::BufRead + std::io::Seek>(
    reader: impl Fn() -> Result<image::ImageReader<R>, String>,
    max_pixels: u64,
) -> Result<Raster, String> {
    let (w, h) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > max_pixels {
        return Err(format!(
            "Image must contain 1 to {} million pixels",
            max_pixels / 1_000_000
        ));
    }
    // The pixel limit above is the one that counts: the decoder's own
    // default (512 MiB) refused a 16-bit RGBA picture over 67 megapixels
    // that the desktop's limit lets in (round three of the Opus 5.5 review).
    let mut decoder = reader()?;
    decoder.no_limits();
    let source = decoder.decode().map_err(|e| e.to_string())?.to_rgba8();
    Ok(Raster {
        width: source.width() as usize,
        height: source.height() as usize,
        pixels: source.pixels().map(|p| Rgba(p.0)).collect(),
    })
}

/// `raster` resampled to `width` by `height` with a triangle filter, on
/// premultiplied pixels so the hidden colour of transparent pixels does not
/// bleed into the edges beside them.
pub fn scaled_raster(raster: &Raster, width: usize, height: usize) -> Raster {
    let premultiplied =
        image::RgbaImage::from_fn(raster.width as u32, raster.height as u32, |x, y| {
            let [r, g, b, a] = raster.pixels[y as usize * raster.width + x as usize].0;
            let times = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
            image::Rgba([times(r), times(g), times(b), a])
        });
    let small = image::imageops::resize(
        &premultiplied,
        width as u32,
        height as u32,
        image::imageops::FilterType::Triangle,
    );
    Raster {
        width,
        height,
        pixels: small
            .pixels()
            .map(|p| {
                let [r, g, b, a] = p.0;
                let back = |c: u8| match a {
                    0 => 0,
                    a => ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
                };
                Rgba([back(r), back(g), back(b), a])
            })
            .collect(),
    }
}

/// The loaded picture at a size the engine traces: scaled down to its side
/// limit when larger, and a side of 1 px doubled, each pixel repeated (the
/// engine needs 2; the defect sweep of September 23, 2026 found 1 px wide
/// pictures refused), with the size it had when either happened, so the
/// drawing can be saved at it; refused when a side is too short even so. The
/// desktop and the CLI both load through it.
pub fn fit_for_engine(raster: Raster) -> Result<(Raster, Option<(usize, usize)>), String> {
    let original = (raster.width, raster.height);
    let (raster, widened) = if raster.width == 1 || raster.height == 1 {
        let (w, h) = (raster.width.max(2), raster.height.max(2));
        let pixels = (0..w * h)
            .map(|i| {
                let (x, y) = (
                    (i % w).min(raster.width - 1),
                    (i / w).min(raster.height - 1),
                );
                raster.pixels[y * raster.width + x]
            })
            .collect();
        (
            Raster {
                width: w,
                height: h,
                pixels,
            },
            Some(original),
        )
    } else {
        (raster, None)
    };
    let (shortest, longest) = (
        *crate::engine::SIDE_LIMITS.start(),
        *crate::engine::SIDE_LIMITS.end(),
    );
    let (width, height) = (raster.width, raster.height);
    // Both engine limits: no side over `longest` and at most 16 million
    // pixels (a 4096-px square is 16.8 million).
    let pixels = width as f64 * height as f64;
    let (by_side, by_area) = (
        longest as f64 / width.max(height) as f64,
        (16e6 / pixels).sqrt(),
    );
    let (raster, scaled_from) = if by_side < 1. || by_area < 1. {
        let factor = by_side.min(by_area);
        // Rounded down when the pixel count binds, so it cannot round over.
        let side = |px: usize| {
            let exact = px as f64 * factor;
            let px = if by_area < by_side {
                exact.floor()
            } else {
                exact.round()
            };
            (px as usize).clamp(1, longest)
        };
        (
            scaled_raster(&raster, side(width), side(height)),
            Some(original),
        )
    } else {
        (raster, None)
    };
    if raster.width < shortest || raster.height < shortest {
        return Err(format!(
            "{width} \u{00D7} {height} px is too narrow to trace: the engine needs {shortest} px \
             per side{}.",
            if scaled_from.is_some() {
                " once scaled to its limit"
            } else {
                ""
            }
        ));
    }
    Ok((raster, scaled_from.or(widened)))
}

/// The document rendered at the preview size (1600 px on its longer side,
/// at most 4x): its width, height and premultiplied RGBA bytes.
#[cfg(feature = "render")]
pub fn preview_pixels(svg: &str) -> Result<(usize, usize, Vec<u8>), String> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default())
        .map_err(|e| e.to_string())?;
    let size = tree.size();
    let factor = (1600. / size.width().max(size.height())).min(4.);
    let width = (size.width() * factor).ceil() as u32;
    let height = (size.height() * factor).ceil() as u32;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("Could not allocate preview")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(factor, factor),
        &mut pixmap.as_mut(),
    );
    Ok((width as usize, height as usize, pixmap.take()))
}

#[cfg(feature = "ui")]
pub fn preview(svg: &str) -> Result<egui::ColorImage, String> {
    let (width, height, pixels) = preview_pixels(svg)?;
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [width, height],
        &pixels,
    ))
}

/// A parsed vector document, kept between crisp renders of the same document.
#[cfg(feature = "ui")]
pub type PreviewTree = resvg::usvg::Tree;

#[cfg(feature = "ui")]
pub fn preview_tree(svg: &str) -> Result<PreviewTree, String> {
    resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).map_err(|e| e.to_string())
}

/// Render the part of the document starting at `origin` (viewBox units, which
/// are source pixels) at `scale` screen pixels per unit into a `size` pixel
/// image: crisp at any zoom. `document_width` is the viewBox width; the
/// engine declares the SVG size in points, so the parsed tree is 4/3 larger
/// than its viewBox and the transform compensates.
#[cfg(feature = "ui")]
pub fn render_region(
    tree: &PreviewTree,
    document_width: f32,
    scale: f32,
    origin: [f32; 2],
    size: [u32; 2],
) -> Result<egui::ColorImage, String> {
    if !(scale.is_finite() && scale > 0.)
        || !(document_width.is_finite() && document_width > 0.)
        || size[0] == 0
        || size[1] == 0
    {
        return Err("Region must have a positive scale, width and size".into());
    }
    let unit = tree.size().width() / document_width;
    let tree_scale = scale / unit;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(size[0], size[1]).ok_or("Could not allocate region")?;
    let transform = resvg::tiny_skia::Transform::from_scale(tree_scale, tree_scale)
        .post_translate(-origin[0] * scale, -origin[1] * scale);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [size[0] as usize, size[1] as usize],
        pixmap.data(),
    ))
}

/// Candidate tolerances tried by `auto_simplify_tolerance`, ascending. They
/// stop at half a source pixel, the engine's own precision (its drawing of
/// the exact ellipse and rounded rectangle spreads 0.24 and 0.46 px). The
/// change limit alone accepted 2 to 2.8 px on simple pictures, whose few
/// edges barely register as a share of all the pixels, and every sample's
/// fidelity turns at 0.5 to 0.7 px (kit/tools/quality_round.py, groups
/// auto-tolerance-sweep and auto-tolerance, September 22, 2026): through
/// the desktop's chain, capping at 0.5 took the colour error of the four
/// logos from 1.81, 2.20, 4.36 and 1.11 to 1.50, 1.95, 4.31 and 0.73, the
/// astronaut's from 6.38 to 6.25, and the rounded rectangle's spread from
/// 1.42 to 0.58 px with its 1.67 inflections per 100 px gone. A cap at 0.7
/// or 1 px, or a stricter change limit (0.2% or 0.1%), kept more of the
/// loss on at least one sample.
#[cfg(feature = "render")]
pub const AUTO_TOLERANCE_CANDIDATES: [f64; 6] = [0.1, 0.15, 0.2, 0.3, 0.4, 0.5];
/// Fraction of preview pixels allowed to change noticeably against the
/// engine's own rendering.
#[cfg(feature = "render")]
pub const AUTO_TOLERANCE_CHANGE_LIMIT: f64 = 0.006;

/// Fraction of pixels whose colour or alpha differs noticeably between two
/// renderings of the same size (`preview_pixels`, premultiplied RGBA).
#[cfg(feature = "render")]
pub fn changed_fraction(a: &(usize, usize, Vec<u8>), b: &(usize, usize, Vec<u8>)) -> f64 {
    if (a.0, a.1) != (b.0, b.1) || a.2.len() < 4 || a.2.len() != b.2.len() {
        return 1.;
    }
    let (pixels_a, pixels_b) = (a.2.as_chunks::<4>().0, b.2.as_chunks::<4>().0);
    let changed = pixels_a
        .iter()
        .zip(pixels_b)
        .filter(|(p, q)| (0..4).any(|c| p[c].abs_diff(q[c]) > 48))
        .count();
    changed as f64 / (a.2.len() / 4) as f64
}

/// The largest Auto tolerance a document traced with `preset` may take:
/// 0.5 px for aliased artwork, whose staircase edges simplifying is there
/// to clean up, 0.3 px for anti-aliased artwork, whose edges carry their
/// place between pixels, and 0.1 px (the smallest candidate) for
/// photographs. On three photographs (September 22, 2026,
/// testing/quality-round/simplify-photos.md) merging at 0.3 made the
/// outlines wobblier than not merging (inflections 1.45 -> 0.93 per 100 px
/// on the cat at 0.1, 1.49 -> 1.09 on the coffee, 1.09 -> 0.89 on the
/// astronaut) at the same colour error and size: simplifying first leaves
/// fewer pieces for straightening to make lines. Not simplifying at all
/// scored a little better still, but Auto picks a tolerance; the Simplify
/// toggle turns it off. Measured through the desktop's
/// chain (round two of the Opus 5.5 review, September 22, 2026,
/// testing/quality-round/straighten-r2.md): 0.3 against 0.5 took the colour
/// error of the blended logos from 1.503 and 1.950 to 1.485 and 1.929, the
/// astronaut's from 6.23 to 6.19, and the exact ellipse's spread from 0.126
/// to 0.043 px and the circles' from 0.05 to 0.025, for files 1 to 5%
/// larger; on the aliased logo it cost kinks (0.425 -> 0.515 per 100 px)
/// and 11% more bytes, where cleaning up is what the owner asks for.
#[cfg(feature = "render")]
pub fn auto_tolerance_cap(preset: usize) -> f64 {
    use vector_rebuild::{basic_preset_code, ImageCategory, Quality};
    let aliased = [Quality::High, Quality::Medium, Quality::Low]
        .into_iter()
        .any(|q| basic_preset_code(ImageCategory::AliasedArtwork, q) == preset);
    let photograph = [Quality::High, Quality::Medium, Quality::Low]
        .into_iter()
        .any(|q| basic_preset_code(ImageCategory::Photograph, q) == preset);
    if aliased {
        0.5
    } else if photograph {
        0.1
    } else {
        0.3
    }
}

/// The largest candidate tolerance up to the document's cap
/// (`auto_tolerance_cap`) whose simplified rendering stays visually the
/// engine's picture: at most `AUTO_TOLERANCE_CHANGE_LIMIT` of the preview
/// pixels change noticeably. Tried from the cap down, so the usual case,
/// the cap passing, renders one candidate instead of every smaller one
/// first (the astronaut spent 2.6 s after the engine rendering six, round
/// two). Returns the tolerance with that simplified document.
#[cfg(feature = "render")]
pub fn auto_simplify_tolerance(raw: &engine::Document) -> Result<(f64, engine::Document), String> {
    let reference = preview_pixels(raw.svg())?;
    let cap = auto_tolerance_cap(raw.preset);
    for &tolerance in AUTO_TOLERANCE_CANDIDATES
        .iter()
        .rev()
        .filter(|&&t| t <= cap)
    {
        let candidate = raw.simplified(tolerance)?;
        let image = preview_pixels(candidate.svg())?;
        if changed_fraction(&reference, &image) <= AUTO_TOLERANCE_CHANGE_LIMIT {
            return Ok((tolerance, candidate));
        }
    }
    let tolerance = AUTO_TOLERANCE_CANDIDATES[0];
    Ok((tolerance, raw.simplified(tolerance)?))
}

/// A region the user recoloured in the source so it traces as part of a
/// neighbouring shape: the pixel indices and the colour they take.
#[derive(Clone, Debug, PartialEq)]
pub struct Recolor {
    pub pixels: Vec<u32>,
    pub rgb: [u8; 3],
    /// The opacity they take too; `None` keeps each pixel's own. A dropped
    /// colour whose nearest neighbour is the transparent background takes
    /// `Some(0)`, so it goes transparent rather than painting that
    /// background's hidden colour.
    pub alpha: Option<u8>,
}

/// What the engine is given instead of the loaded image. Nothing here changes
/// the engine; it changes the pixels it sees.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Preparation {
    /// At most this many colours, found in the image itself.
    pub colors: Option<usize>,
    /// Transparency flattened onto this colour.
    pub background: Option<[u8; 3]>,
    /// Regions recoloured by merges, applied last.
    pub recolors: Vec<Recolor>,
}
impl Preparation {
    /// The palette a colour limit gives `working`, for snapping the engine's
    /// fills; `None` without a limit.
    pub fn palette_of(&self, working: &Raster) -> Option<Vec<[u8; 3]>> {
        self.colors
            .map(|count| vector_rebuild::prepare::palette(working, count))
    }
    pub fn is_identity(&self) -> bool {
        self.colors.is_none() && self.background.is_none() && self.recolors.is_empty()
    }
    pub fn apply(&self, raster: &Raster) -> Raster {
        let mut out = match self.background {
            Some(rgb) => vector_rebuild::prepare::flatten(raster, rgb),
            None => raster.clone(),
        };
        if let Some(count) = self.colors {
            out = vector_rebuild::prepare::quantize(&out, count);
        }
        for recolor in &self.recolors {
            for &index in &recolor.pixels {
                if let Some(pixel) = out.pixels.get_mut(index as usize) {
                    pixel.0[..3].copy_from_slice(&recolor.rgb);
                    if let Some(alpha) = recolor.alpha {
                        pixel.0[3] = alpha;
                    }
                }
            }
        }
        out
    }
}

/// `white`, `black` or `#rrggbb`.
pub fn parse_rgb(text: &str) -> Option<[u8; 3]> {
    match text.trim().to_ascii_lowercase().as_str() {
        "white" => Some([255, 255, 255]),
        "black" => Some([0, 0, 0]),
        hex => {
            // Exactly six hex digits: no sign, no trailing text, and only
            // ASCII, so the byte slices below fall on character boundaries.
            let hex = hex.strip_prefix('#')?;
            if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            Some([channel(0)?, channel(2)?, channel(4)?])
        }
    }
}

/// The same drawing declared at another size. Only the `width` and `height`
/// attributes of the root element change, to plain pixel numbers; the
/// `viewBox` and every coordinate stay as the engine wrote them, so the
/// picture scales exactly. The engine declares its size in points, which is
/// why a saved file looked smaller than its pixel size in some editors.
pub fn resize_svg(svg: &str, width: u32, height: u32) -> Result<String, String> {
    if width == 0 || height == 0 {
        return Err("Output size must be at least 1 \u{00D7} 1 pixel".into());
    }
    let start = svg
        .find("<svg")
        .ok_or_else(|| "Not an SVG document".to_owned())?;
    let end = start
        + svg[start..]
            .find('>')
            .ok_or_else(|| "Unterminated <svg> tag".to_owned())?;
    let tag = &svg[start..end];
    let replaced = |tag: &str, name: &str, value: u32| -> Result<String, String> {
        let needle = format!(" {name}=\"");
        let at = tag
            .find(&needle)
            .ok_or_else(|| format!("The <svg> tag has no {name}"))?
            + needle.len();
        let close = at
            + tag[at..]
                .find('"')
                .ok_or_else(|| format!("Unterminated {name}"))?;
        Ok(format!("{}{value}{}", &tag[..at], &tag[close..]))
    };
    let tag = replaced(tag, "width", width)?;
    let tag = replaced(&tag, "height", height)?;
    Ok(format!("{}{tag}{}", &svg[..start], &svg[end..]))
}

#[cfg(test)]
mod tests {
    use vector_rebuild::raster::{Raster, Rgba};

    #[test]
    fn preparation_flattens_limits_colours_and_recolours_in_that_order() {
        let raster = Raster {
            width: 4,
            height: 1,
            pixels: vec![
                Rgba([255, 0, 0, 255]),
                Rgba([250, 5, 5, 255]),
                Rgba([0, 0, 255, 255]),
                Rgba([0, 0, 255, 0]),
            ],
        };
        assert!(super::Preparation::default().is_identity());
        assert_eq!(
            super::Preparation::default().apply(&raster).pixels,
            raster.pixels
        );
        let prep = super::Preparation {
            colors: Some(8),
            background: Some([255, 255, 255]),
            recolors: vec![super::Recolor {
                pixels: vec![2],
                rgb: [1, 2, 3],
                alpha: None,
            }],
        };
        let out = prep.apply(&raster);
        assert_eq!(out.pixels[3].0, [255, 255, 255, 255], "flattened on white");
        assert_eq!(
            out.pixels[0].0,
            [255, 0, 0, 255],
            "four colours fit the limit"
        );
        assert_eq!(out.pixels[2].0, [1, 2, 3, 255], "recoloured last");
        // A recolouring with an opacity sets it too; without, each pixel keeps its own.
        let cleared = super::Preparation {
            recolors: vec![super::Recolor {
                pixels: vec![0, 3],
                rgb: [9, 9, 9],
                alpha: Some(0),
            }],
            ..super::Preparation::default()
        }
        .apply(&raster);
        assert_eq!(cleared.pixels[0].0, [9, 9, 9, 0]);
        assert_eq!(cleared.pixels[3].0, [9, 9, 9, 0]);
        assert_eq!(cleared.pixels[1].0, raster.pixels[1].0);
        let palette = prep.palette_of(&out).unwrap();
        assert_eq!(palette.len(), 4);
        assert!(super::Preparation::default().palette_of(&out).is_none());
        assert_eq!(super::parse_rgb("White"), Some([255, 255, 255]));
        assert_eq!(super::parse_rgb("#4fd1c5"), Some([0x4f, 0xd1, 0xc5]));
        assert_eq!(super::parse_rgb("#4fd1"), None);
        assert_eq!(super::parse_rgb("teal"), None);
        // Six bytes that are not six ASCII digits: refused, never sliced
        // inside a character, and no trailing text or signs accepted.
        assert_eq!(super::parse_rgb("#\u{4e2d}\u{6587}"), None);
        assert_eq!(super::parse_rgb("#ff0000zz"), None);
        assert_eq!(super::parse_rgb("#+f+f+f"), None);
        assert_eq!(super::parse_rgb("#4FD1C5"), Some([0x4f, 0xd1, 0xc5]));
    }

    #[test]
    fn sticker_specs_parse_whole_or_not_at_all() {
        let sticker = super::parse_sticker("4, 2.5,shadow").unwrap();
        assert_eq!(
            (sticker.border, sticker.edge, sticker.shadow),
            (4., 2.5, true)
        );
        assert!(!super::parse_sticker("4,2").unwrap().shadow);
        for bad in [
            "4",
            "4,x",
            "0,0",
            "4,8,blur",
            "4,8,shadow,more",
            "4,-1",
            "4,inf",
        ] {
            assert!(super::parse_sticker(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_failed_save_leaves_the_previous_file_whole() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../work/app-tests")
            .join(format!("{}-replacing", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("drawing.svg");
        std::fs::write(&output, b"previous output").unwrap();
        let listing = || {
            let mut names: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            names.sort();
            names
        };
        // The write fails halfway: the target keeps its bytes, and the
        // half-written sibling is gone.
        let error = super::replace_with(&output, |file| {
            std::io::Write::write_all(file, b"half a docu")?;
            Err(std::io::Error::other("disk full"))
        })
        .unwrap_err();
        assert!(error.contains("disk full"));
        assert_eq!(std::fs::read(&output).unwrap(), b"previous output");
        assert_eq!(listing(), ["drawing.svg"]);
        super::write_replacing(&output, b"new output").unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"new output");
        assert_eq!(listing(), ["drawing.svg"]);
        // A target that cannot be replaced (a folder) is refused, and the
        // sibling is removed.
        let folder = dir.join("folder.svg");
        std::fs::create_dir_all(folder.join("inside")).unwrap();
        assert!(super::write_replacing(&folder, b"x").is_err());
        assert!(folder.join("inside").is_dir());
        assert_eq!(listing(), ["drawing.svg", "folder.svg"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_view_box_gives_the_source_size() {
        let engine = "<?xml version=\"1.0\"?>\r\n<svg width=\"188pt\" height=\"150pt\" viewBox=\"0 0 250 200\" version=\"1.1\">";
        assert_eq!(super::view_box_size(engine), Some((250., 200.)));
        let sticker = "<svg width=\"268pt\" height=\"268pt\" viewBox=\"-9 -9 268 268\">";
        assert_eq!(super::view_box_size(sticker), Some((268., 268.)));
        assert_eq!(
            super::view_box_size("<svg viewBox=\"0,0, 10.5 3\">"),
            Some((10.5, 3.))
        );
        for bad in [
            "<svg width=\"10\" height=\"10\">",
            "<svg viewBox=\"0 0 0 10\">",
            "<svg viewBox=\"0 0 x 10\">",
            "<svg viewBox=\"0 0 10\">",
            "<html viewBox=\"0 0 1 1\">",
        ] {
            assert_eq!(super::view_box_size(bad), None, "{bad}");
        }
    }

    #[test]
    fn resize_rewrites_only_the_declared_size() {
        let svg = "<?xml version=\"1.0\"?>\n<svg width=\"250pt\" height=\"200pt\" viewBox=\"0 0 250 200\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<path d=\" M 1 1 L 2 2\" />\n</svg>\n";
        let out = super::resize_svg(svg, 1000, 800).unwrap();
        assert!(out.starts_with(
            "<?xml version=\"1.0\"?>\n<svg width=\"1000\" height=\"800\" viewBox=\"0 0 250 200\""
        ));
        assert!(out.ends_with("<path d=\" M 1 1 L 2 2\" />\n</svg>\n"));
        assert_eq!(out.len(), svg.len() - 3, "250pt/200pt became 1000/800");
        assert!(super::resize_svg(svg, 0, 800).is_err());
        assert!(super::resize_svg("<html>", 10, 10).is_err());
        assert!(super::resize_svg("<svg viewBox=\"0 0 1 1\">", 10, 10).is_err());
    }
}

#[cfg(feature = "desktop")]
pub mod preview_cli;

#[cfg(feature = "desktop")]
pub mod dragout;
