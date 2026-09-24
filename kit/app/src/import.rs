//! Files from other programs (the owner, September 24, 2026: "allow
//! Photoshop imports, like PSD ... let people throw in things like other
//! vectors ... instead of saying 'This is unsupported,' it says ... do you
//! want us to convert this to a raster, then trace it, or do you want us to
//! just cross-convert?").
//!
//! A picture (PNG, JPEG, GIF, BMP, PNM and Photoshop's PSD and PSB) is traced
//! as always. A vector file (SVG, PDF, Illustrator's AI, EPS) is read into
//! an SVG of flat filled and stroked paths, the same small language the
//! app's own documents speak (`pdf_eps.rs` reads it), so it can be saved
//! straight away in every format the app writes (a cross-conversion), or
//! drawn as pixels and traced like any picture.

/// What a file holds, told from its first bytes (and its name only for SVG,
/// which is text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// Something `image` decodes (or refuses with its own message).
    Picture,
    /// Photoshop's document or large document format (`psd.rs`).
    Photoshop,
    Svg,
    /// PDF, and Illustrator files saved PDF-compatible (every AI since
    /// Illustrator 9 by default) (`pdf_import.rs`).
    Pdf,
    /// PostScript: EPS, PS, and Illustrator files of the PostScript era
    /// (`eps_import.rs`).
    PostScript,
}

impl InputKind {
    /// Whether the file is vector artwork rather than a picture.
    pub fn is_vector(self) -> bool {
        matches!(
            self,
            InputKind::Svg | InputKind::Pdf | InputKind::PostScript
        )
    }
    /// The format's name, as the app says it.
    pub fn label(self) -> &'static str {
        match self {
            InputKind::Picture => "picture",
            InputKind::Photoshop => "Photoshop document",
            InputKind::Svg => "SVG",
            InputKind::Pdf => "PDF",
            InputKind::PostScript => "EPS",
        }
    }
}

/// The kind of file `bytes` is; `name` (its file name) only decides between
/// an SVG and a picture when the text does not.
pub fn input_kind(bytes: &[u8], name: &str) -> InputKind {
    if bytes.starts_with(b"8BPS") {
        return InputKind::Photoshop;
    }
    // A DOS EPS binary header, or PostScript's own comment.
    if bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) || bytes.starts_with(b"%!") {
        return InputKind::PostScript;
    }
    // PDF allows junk before its header, within the first kilobyte.
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(5).any(|w| w == b"%PDF-") {
        return InputKind::Pdf;
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let svg_name = name.to_ascii_lowercase().ends_with(".svg");
    if (text.starts_with('<') && text.contains("<svg")) || (svg_name && text.starts_with('<')) {
        return InputKind::Svg;
    }
    InputKind::Picture
}

/// Vector artwork read from another program's file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Imported {
    /// The artwork as flat paths: `<svg width="Wpt" height="Hpt" viewBox="0 0
    /// W H">` (W and H in points) holding `path` elements in page space, y
    /// down, each with `fill` (`#rrggbb` or `none`), `fill-rule`,
    /// `fill-opacity`, `stroke`, `stroke-width`, `stroke-linejoin`,
    /// `stroke-linecap` and `stroke-opacity` as they apply, no transforms and
    /// no groups: what `pdf_eps.rs` reads.
    pub svg: String,
    /// How many pages the file has; `svg` is the page asked for.
    pub pages: usize,
    /// What the file held that is not paths and was left out, once each in
    /// plain words ("text", "3 images", "gradients drawn in one colour").
    pub skipped: Vec<String>,
}

/// The vector artwork in `bytes`, a file of `kind` (`input_kind`), as flat
/// paths.
pub fn read_vector(kind: InputKind, bytes: &[u8]) -> Result<Imported, String> {
    match kind {
        #[cfg(feature = "render")]
        InputKind::Svg => crate::svg_import::to_svg(bytes),
        #[cfg(not(feature = "render"))]
        InputKind::Svg => Err("This build of VectorMagik reads no SVG files.".into()),
        InputKind::Pdf => crate::pdf_import::to_svg(bytes, 0),
        InputKind::PostScript => crate::eps_import::to_svg(bytes),
        InputKind::Picture | InputKind::Photoshop => {
            Err("This is a picture, not vector artwork.".into())
        }
    }
}

/// The shape layers of a Photoshop document as vector artwork, to trace or
/// to convert as they are (`psd.rs`): `None` when it has none, and then it
/// is only a picture.
pub fn photoshop_shapes(bytes: &[u8]) -> Result<Option<Imported>, String> {
    crate::psd::shapes(bytes)
}

/// Whether `imported` holds any shapes at all.
pub fn has_shapes(imported: &Imported) -> bool {
    imported.svg.contains("<path")
}

/// The picture a vector file with no shapes holds instead (a scanned logo
/// saved as a PDF): `None` when it holds none, an error when it holds one
/// the app cannot read.
pub fn picture_in(
    kind: InputKind,
    bytes: &[u8],
) -> Result<Option<vector_rebuild::raster::Raster>, String> {
    match kind {
        InputKind::Pdf => crate::pdf_import::picture(bytes, 0),
        InputKind::PostScript => crate::eps_import::picture(bytes),
        _ => Ok(None),
    }
}

/// `imported` drawn as a picture whose longer side is `longest` pixels, on
/// transparency: what a vector file is traced from.
#[cfg(feature = "render")]
pub fn rasterize(
    imported: &Imported,
    longest: u32,
) -> Result<vector_rebuild::raster::Raster, String> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(&imported.svg, &usvg::Options::default())
        .map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = longest as f32 / size.width().max(size.height()).max(1e-3);
    let width = ((size.width() * scale).round() as u32).max(1);
    let height = ((size.height() * scale).round() as u32).max(1);
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or("The picture to trace is too large to draw.")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let pixels = pixmap
        .pixels()
        .iter()
        .map(|p| {
            let c = p.demultiply();
            vector_rebuild::raster::Rgba([c.red(), c.green(), c.blue(), c.alpha()])
        })
        .collect();
    Ok(vector_rebuild::raster::Raster {
        width: width as usize,
        height: height as usize,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_told_apart_by_their_first_bytes() {
        assert_eq!(input_kind(b"8BPS\0\x01", "a.psd"), InputKind::Photoshop);
        assert_eq!(input_kind(b"%PDF-1.7\n", "a.ai"), InputKind::Pdf);
        assert_eq!(input_kind(b"\n\n%PDF-1.4", "a.pdf"), InputKind::Pdf);
        assert_eq!(
            input_kind(b"%!PS-Adobe-3.0 EPSF-3.0", "a.eps"),
            InputKind::PostScript
        );
        assert_eq!(
            input_kind(&[0xC5, 0xD0, 0xD3, 0xC6, 0, 0], "a.eps"),
            InputKind::PostScript
        );
        assert_eq!(
            input_kind(b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<svg>", "x"),
            InputKind::Svg
        );
        assert_eq!(input_kind(b"<svg xmlns=\"\"/>", "x"), InputKind::Svg);
        assert_eq!(
            input_kind(b"\x89PNG\r\n\x1a\n", "a.png"),
            InputKind::Picture
        );
        assert!(InputKind::Pdf.is_vector() && !InputKind::Photoshop.is_vector());
    }

    #[cfg(feature = "render")]
    #[test]
    fn a_vector_file_is_drawn_as_a_picture_to_trace() {
        let imported = crate::svg_import::to_svg(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        let raster = rasterize(&imported, 400).unwrap();
        assert_eq!((raster.width, raster.height), (400, 200));
        assert_eq!(raster.pixels[10 * 400 + 10].0, [255, 0, 0, 255]);
        assert_eq!(raster.pixels[10 * 400 + 300].0[3], 0);
    }
}
