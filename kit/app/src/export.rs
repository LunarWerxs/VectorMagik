//! File-format selection around the generated SVG. No fitting geometry changes.
//!
//! The formats and settings are the original Vector Magic desktop's own
//! (its help and dialog strings): EPS, SVG, PDF, AI, EMF and DXF; the shape
//! mode "Place shapes in cut-outs and group by color" (its default, and what
//! the engine writes: `<g id="#rrggbbaa">` colour groups of paths with hole
//! loops) or "Place shapes in cut-outs in shapes below" (the same paths
//! without the groups); "Stroke shape boundaries" or not (not by default);
//! and DXF as lines and spline curves or as lines alone, fine or coarse.
//! PNG is the drawing as pixels at its declared size.
//!
//! AI is a PDF-compatible Illustrator file: the PDF's bytes. Illustrator
//! has saved AI files as PDF with its own private data added since version
//! 9, and opens a PDF-based AI without that data as it opens any PDF, so a
//! plain PDF under the AI name is what every current Illustrator reads.
use std::path::Path;

/// The formats a document can be saved as, by output extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Svg,
    /// PDF and EPS are written from the SVG in-process (`crate::pdf_eps`).
    Pdf,
    Eps,
    /// Illustrator: the PDF (see the module comment).
    Ai,
    /// AutoCAD's drawing exchange format (`crate::dxf`).
    Dxf,
    /// Windows' Enhanced Metafile (`crate::emf`).
    Emf,
    /// The drawing as pixels; needs the `render` feature.
    Png,
}

/// Why a build without the renderer cannot save a PNG.
#[cfg(not(feature = "render"))]
const PNG_NEEDS_RENDER: &str =
    "Saving a PNG renders the drawing and needs a build with the render (or desktop) feature";

/// The format `output`'s extension asks for, case-insensitively; anything
/// but SVG, PDF, EPS, AI, DXF, EMF or PNG is refused (PNG too in a build
/// without the renderer).
pub fn output_kind(output: &Path) -> Result<OutputKind, String> {
    let extension = output
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "svg" => Ok(OutputKind::Svg),
        "pdf" => Ok(OutputKind::Pdf),
        "eps" => Ok(OutputKind::Eps),
        "ai" => Ok(OutputKind::Ai),
        "dxf" => Ok(OutputKind::Dxf),
        "emf" => Ok(OutputKind::Emf),
        #[cfg(feature = "render")]
        "png" => Ok(OutputKind::Png),
        #[cfg(not(feature = "render"))]
        "png" => Err(PNG_NEEDS_RENDER.into()),
        _ => Err("Choose an SVG, PDF, EPS, AI, DXF, EMF or PNG output filename".into()),
    }
}

/// How the curves of a DXF are written, as the original offered it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DxfMode {
    /// "Lines and spline curves" (the original's default).
    #[default]
    Splines,
    /// "Lines only, convert splines to more lines (larger file)".
    FineLines,
    /// "Lines only, convert splines to fewer lines (smaller file)".
    CoarseLines,
}

/// The original's export settings. The third shape mode, "Stack shapes on
/// top of each other", is the app's own stacked drawing and not an option
/// of the writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    /// "Place shapes in cut-outs and group by color" when true (the
    /// default), "Place shapes in cut-outs in shapes below" when false: the
    /// same paths in the same order, with or without their colour groups.
    pub group_by_color: bool,
    /// "Stroke shape boundaries": every filled path also stroked with its
    /// own fill at width 0.09375 in document units, which hides the
    /// hairline seams some renderers leave between neighbouring shapes.
    pub stroke_boundaries: bool,
    pub dxf: DxfMode,
}

impl Default for ExportOptions {
    /// The original's defaults: grouped by colour, not stroked, DXF with
    /// spline curves.
    fn default() -> Self {
        Self {
            group_by_color: true,
            stroke_boundaries: false,
            dxf: DxfMode::Splines,
        }
    }
}

/// The width the original strokes shape boundaries at, in document units
/// (`docs/EXPORT.md`, the writer).
const BOUNDARY_STROKE: &str = "0.09375";

/// The file `svg` saves as in the format `kind` under `options`: the SVG
/// itself, or another format written from it in-process.
pub fn vector_bytes(
    kind: OutputKind,
    svg: &str,
    options: &ExportOptions,
) -> Result<Vec<u8>, String> {
    let svg = apply_options(svg, options);
    match kind {
        OutputKind::Svg => Ok(svg.into_bytes()),
        OutputKind::Pdf | OutputKind::Ai => crate::pdf_eps::to_pdf(&svg),
        OutputKind::Eps => crate::pdf_eps::to_eps(&svg),
        OutputKind::Dxf => crate::dxf::to_dxf(&svg, options.dxf),
        OutputKind::Emf => crate::emf::to_emf(&svg),
        #[cfg(feature = "render")]
        OutputKind::Png => png(&svg),
        #[cfg(not(feature = "render"))]
        OutputKind::Png => Err(PNG_NEEDS_RENDER.into()),
    }
}

pub fn write_vector(
    source: &Path,
    output: &Path,
    svg: &str,
    options: &ExportOptions,
) -> Result<(), String> {
    if crate::same_file(source, output) {
        return Err("Input and output must be different files".into());
    }
    // Finish conversion before touching the selected output file, and never
    // truncate it in place.
    let bytes = vector_bytes(output_kind(output)?, svg, options)?;
    crate::write_replacing(output, &bytes)
}

/// The largest PNG written, in pixels: the most the desktop opens.
#[cfg(feature = "render")]
const MAX_PNG_PIXELS: u64 = crate::DESKTOP_MAX_PIXELS;

/// `svg` as a PNG at its declared size in CSS pixels (96 per inch, so 4/3
/// of its points), on a transparent background.
#[cfg(feature = "render")]
fn png(svg: &str) -> Result<Vec<u8>, String> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default())
        .map_err(|e| e.to_string())?;
    let size = tree.size().to_int_size();
    let (width, height) = (size.width(), size.height());
    if u64::from(width) * u64::from(height) > MAX_PNG_PIXELS {
        return Err("The drawing is too large to save as a PNG".into());
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("Could not allocate the PNG")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let mut data = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut data),
        &rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    Ok(data)
}

/// `svg` with `options` applied: without colour groups, the plain `<g>`
/// elements (those with no attribute but an `id`) are left out and their
/// paths kept in order; with boundary strokes, every filled `path` and
/// `rect` that has no stroke gets `stroke` of its own fill, width 0.09375
/// and round joins (and its `fill-opacity` as `stroke-opacity`). Under the
/// original's defaults the text is returned as it is, byte for byte. Markup
/// this cannot read is copied unchanged for the writer to judge.
pub fn apply_options(svg: &str, options: &ExportOptions) -> String {
    if options.group_by_color && !options.stroke_boundaries {
        return svg.to_owned();
    }
    let mut out = String::with_capacity(svg.len() + svg.len() / 4);
    // Per open `<g>`: whether it was left out, and the fill and whether a
    // stroke is drawn as its children inherit them.
    let mut groups: Vec<(bool, Option<&str>, bool)> = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let close = if rest.starts_with("<!--") { "-->" } else { ">" };
        let Some(end) = rest.find(close).map(|e| e + close.len()) else {
            break;
        };
        let tag = &rest[..end];
        rest = &rest[end..];
        let (closing, body) = match tag[1..].strip_prefix('/') {
            Some(body) => (true, body),
            None => (false, &tag[1..]),
        };
        let name = body
            .split(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("");
        match (closing, name) {
            (false, "g") if !tag.ends_with("/>") => {
                let attributes = tag_attributes(tag);
                let plain = attributes.iter().all(|a| a.name == "id");
                let drop = !options.group_by_color && plain;
                let parent = groups.last().map_or((None, false), |g| (g.1, g.2));
                let fill = attribute(&attributes, "fill").or(parent.0);
                let stroke =
                    attribute(&attributes, "stroke").map_or(parent.1, |s| s.trim() != "none");
                groups.push((drop, fill, stroke));
                if drop {
                    out.truncate(out.trim_end().len());
                } else {
                    out.push_str(tag);
                }
            }
            (true, "g") => {
                if groups.pop().is_some_and(|g| g.0) {
                    out.truncate(out.trim_end().len());
                } else {
                    out.push_str(tag);
                }
            }
            (false, "path" | "rect") if options.stroke_boundaries => {
                let (fill, stroke) = groups.last().map_or((None, false), |g| (g.1, g.2));
                out.push_str(&stroked(tag, fill, stroke));
            }
            _ => out.push_str(tag),
        }
    }
    out.push_str(rest);
    out
}

/// One attribute of a tag, and where its text (with the space before it)
/// lies in the tag.
struct TagAttribute<'a> {
    name: &'a str,
    value: &'a str,
    span: std::ops::Range<usize>,
}

/// The attributes of `tag` (`<name a="1" b='2' />`), as far as they read.
fn tag_attributes(tag: &str) -> Vec<TagAttribute<'_>> {
    let mut out = Vec::new();
    let name_end = tag
        .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
        .unwrap_or(tag.len());
    let mut at = name_end;
    while let Some(eq) = tag[at..].find('=').map(|e| at + e) {
        let name = tag[at..eq].trim();
        let after = eq + 1 + (tag[eq + 1..].len() - tag[eq + 1..].trim_start().len());
        let Some(quote) = tag[after..]
            .chars()
            .next()
            .filter(|q| *q == '"' || *q == '\'')
        else {
            break;
        };
        let Some(close) = tag[after + 1..].find(quote).map(|c| after + 1 + c) else {
            break;
        };
        if name.is_empty() || name.contains(|c: char| c.is_ascii_whitespace()) {
            break;
        }
        out.push(TagAttribute {
            name,
            value: &tag[after + 1..close],
            span: at..close + 1,
        });
        at = close + 1;
    }
    out
}

fn attribute<'a>(attributes: &[TagAttribute<'a>], name: &str) -> Option<&'a str> {
    attributes.iter().find(|a| a.name == name).map(|a| a.value)
}

/// A shape's tag with its boundary stroke: its own fill (or the one it
/// inherits, black by SVG's default) as its stroke, unless it has no fill,
/// a paint server for one, or a stroke already.
fn stroked(tag: &str, inherited_fill: Option<&str>, inherited_stroke: bool) -> String {
    let attributes = tag_attributes(tag);
    let own_stroke = attribute(&attributes, "stroke");
    let stroked = match own_stroke {
        Some(stroke) => stroke.trim() != "none",
        None => inherited_stroke,
    };
    let fill = attribute(&attributes, "fill")
        .or(inherited_fill)
        .unwrap_or("#000000")
        .trim();
    if stroked || fill == "none" || fill.contains("url(") {
        return tag.to_owned();
    }
    let mut added =
        format!(" stroke=\"{fill}\" stroke-width=\"{BOUNDARY_STROKE}\" stroke-linejoin=\"round\"");
    if let (Some(opacity), None) = (
        attribute(&attributes, "fill-opacity"),
        attribute(&attributes, "stroke-opacity"),
    ) {
        added.push_str(&format!(" stroke-opacity=\"{opacity}\""));
    }
    // After the fill as the original writes it, or after the element name;
    // a `stroke="none"` and the stroke settings it made moot give way.
    let replaced = ["stroke", "stroke-width", "stroke-linejoin"];
    let insert_at = attributes
        .iter()
        .find(|a| a.name == "fill")
        .map(|a| a.span.end)
        .unwrap_or_else(|| {
            tag.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
                .unwrap_or(tag.len())
        });
    let mut out = String::with_capacity(tag.len() + added.len());
    let mut at = 0;
    for a in attributes.iter().filter(|a| replaced.contains(&a.name)) {
        if a.span.start >= insert_at && at <= insert_at {
            out.push_str(&tag[at..insert_at]);
            out.push_str(&added);
            at = insert_at;
        }
        out.push_str(&tag[at..a.span.start]);
        at = a.span.end;
    }
    if at <= insert_at {
        out.push_str(&tag[at..insert_at]);
        out.push_str(&added);
        at = insert_at;
    }
    out.push_str(&tag[at..]);
    out
}

/// `svg` as it is saved under the improved defaults, drawing the same in
/// fewer bytes: every number of its path data written as short as it reads
/// the same (the engine writes two decimals everywhere, `100.00`, `72.50`,
/// `-0.00`) and no `opacity="1.00"`, the default, on every path (5% of a
/// photograph's file). Spaces and command letters stay, so every reader of
/// the engine's layout reads it.
pub fn compact_svg(svg: &str) -> String {
    let svg = svg.replace(" opacity=\"1.00\"", "");
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg.as_str();
    while let Some(at) = rest.find(" d=\"") {
        let start = at + 4;
        let Some(length) = rest[start..].find('"') else {
            break;
        };
        out.push_str(&rest[..start]);
        for (i, token) in rest[start..start + length].split(' ').enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(short(token));
        }
        rest = &rest[start + length..];
    }
    out.push_str(rest);
    out
}

/// A decimal number without its trailing zeros (and point), `-0` as `0`;
/// anything else as it is.
fn short(token: &str) -> &str {
    if !token.contains('.') || token.parse::<f64>().is_err() {
        return token;
    }
    match token.trim_end_matches('0').trim_end_matches('.') {
        "-0" | "" | "-" => "0",
        short => short,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_numbers_lose_their_trailing_zeros_and_paths_their_default_opacity() {
        let svg = "<svg width=\"40pt\" viewBox=\"0 0 40 40\"><path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 40.00 L 12.50 -0.00 C 1.25 3.10 100.00 -2.50 7.05 9.00 Z\" /><path fill=\"none\" stroke-width=\"1.00\" d=\" M 10.00 0.50 Z\" /></svg>";
        assert_eq!(
            compact_svg(svg),
            "<svg width=\"40pt\" viewBox=\"0 0 40 40\"><path fill=\"#ffffff\" d=\" M 0 40 L 12.5 0 C 1.25 3.1 100 -2.5 7.05 9 Z\" /><path fill=\"none\" stroke-width=\"1.00\" d=\" M 10 0.5 Z\" /></svg>"
        );
    }

    /// The engine's layout: colour groups on their own CR LF lines.
    const GROUPED: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n<svg width=\"20pt\" height=\"10pt\" viewBox=\"0 0 20 10\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\r\n<g id=\"#ff0000ff\">\r\n<path fill=\"#ff0000\" opacity=\"1.00\" d=\" M 0.00 0.00 L 10.00 0.00 L 10.00 10.00 Z\" />\r\n<path fill=\"#ff0000\" opacity=\"1.00\" d=\" M 12.00 0.00 L 14.00 0.00 L 14.00 2.00 Z\" />\r\n</g>\r\n<g id=\"#0000ffff\">\r\n<path fill=\"#0000ff\" opacity=\"0.50\" d=\" M 10.00 0.00 L 20.00 0.00 L 20.00 10.00 Z\" />\r\n</g>\r\n</svg>\r\n";

    #[test]
    fn the_defaults_leave_the_document_byte_for_byte() {
        assert_eq!(apply_options(GROUPED, &ExportOptions::default()), GROUPED);
        let bytes = vector_bytes(OutputKind::Svg, GROUPED, &ExportOptions::default()).unwrap();
        assert_eq!(bytes, GROUPED.as_bytes());
        assert_eq!(
            vector_bytes(OutputKind::Ai, GROUPED, &ExportOptions::default()).unwrap(),
            crate::pdf_eps::to_pdf(GROUPED).unwrap()
        );
    }

    #[test]
    fn cut_outs_without_colour_groups_keep_every_path_in_order() {
        let options = ExportOptions {
            group_by_color: false,
            ..ExportOptions::default()
        };
        let flat = apply_options(GROUPED, &options);
        assert!(!flat.contains("<g") && !flat.contains("</g>"), "{flat}");
        let paths = |svg: &str| {
            svg.match_indices("<path")
                .map(|(at, _)| &svg[at..at + svg[at..].find("/>").unwrap()])
                .collect::<Vec<_>>()
                .join("|")
        };
        assert_eq!(paths(&flat), paths(GROUPED));
        assert!(flat.contains("xmlns=\"http://www.w3.org/2000/svg\">\r\n<path fill=\"#ff0000\""));
        assert!(flat.ends_with("Z\" />\r\n</svg>\r\n"), "{flat}");
        // The same page either way.
        assert_eq!(
            crate::pdf_eps::page(&flat).unwrap(),
            crate::pdf_eps::page(GROUPED).unwrap()
        );
        // A group that carries paint or a transform stays.
        let styled = "<svg width=\"1\" height=\"1\"><g fill=\"#00ff00\"><path d=\"M 0 0 L 1 0 L 1 1 Z\" /></g></svg>";
        assert_eq!(apply_options(styled, &options), styled);
    }

    #[test]
    fn boundary_strokes_take_each_fill_and_leave_existing_strokes_alone() {
        let options = ExportOptions {
            stroke_boundaries: true,
            ..ExportOptions::default()
        };
        let stroked = apply_options(GROUPED, &options);
        assert!(stroked.contains("<g id=\"#ff0000ff\">"), "{stroked}");
        assert_eq!(
            stroked.matches("<path fill=\"#ff0000\" stroke=\"#ff0000\" stroke-width=\"0.09375\" stroke-linejoin=\"round\" opacity=\"1.00\" d=").count(),
            2,
            "{stroked}"
        );
        assert!(stroked.contains("<path fill=\"#0000ff\" stroke=\"#0000ff\" stroke-width=\"0.09375\" stroke-linejoin=\"round\" opacity=\"0.50\""));
        let page = crate::pdf_eps::page(&stroked).unwrap();
        assert!(page
            .shapes
            .iter()
            .all(|s| s.stroke == s.fill && s.join == 1));
        assert!((page.shapes[0].stroke_width - 0.09375).abs() < 1e-12);

        let mixed = "<svg width=\"4\" height=\"4\"><path d=\"M 0 0 L 1 0 L 1 1 Z\" /><path fill=\"none\" stroke=\"#000\" d=\"M 0 0 L 1 1\" /><path fill=\"#123456\" stroke=\"#abcdef\" stroke-width=\"2\" d=\"M 0 0 L 2 2\" /><path fill=\"#00ff00\" fill-opacity=\"0.4\" stroke=\"none\" stroke-width=\"3\" d=\"M 1 1 L 2 1 L 2 2 Z\" /><g stroke=\"#ff0000\"><path fill=\"#0000ff\" d=\"M 0 0 L 1 1\" /></g><rect fill=\"#010203\" x=\"1\" y=\"1\" width=\"1\" height=\"1\"/></svg>";
        let out = apply_options(mixed, &options);
        // Black by default; strokes of their own or inherited are kept.
        assert!(out.contains("<path stroke=\"#000000\" stroke-width=\"0.09375\" stroke-linejoin=\"round\" d=\"M 0 0 L 1 0 L 1 1 Z\" />"), "{out}");
        assert!(
            out.contains("<path fill=\"none\" stroke=\"#000\" d=\"M 0 0 L 1 1\" />"),
            "{out}"
        );
        assert!(
            out.contains("<path fill=\"#123456\" stroke=\"#abcdef\" stroke-width=\"2\" d="),
            "{out}"
        );
        assert!(
            out.contains("<g stroke=\"#ff0000\"><path fill=\"#0000ff\" d=\"M 0 0 L 1 1\" /></g>"),
            "{out}"
        );
        // `stroke="none"` and its moot width give way to the boundary stroke.
        assert!(out.contains("<path fill=\"#00ff00\" stroke=\"#00ff00\" stroke-width=\"0.09375\" stroke-linejoin=\"round\" stroke-opacity=\"0.4\" fill-opacity=\"0.4\" d=\"M 1 1 L 2 1 L 2 2 Z\" />"), "{out}");
        assert!(out.contains("<rect fill=\"#010203\" stroke=\"#010203\" stroke-width=\"0.09375\" stroke-linejoin=\"round\" x=\"1\""), "{out}");
        crate::pdf_eps::to_pdf(&out).unwrap();
    }

    #[test]
    fn malformed_markup_is_copied_for_the_writer_to_refuse() {
        let options = ExportOptions {
            group_by_color: false,
            stroke_boundaries: true,
            dxf: DxfMode::Splines,
        };
        for svg in [
            "<svg><g id=\"a\"><path fill=\"#fff\" d=\"M 0 0",
            "<svg><path fill=\"#fff d=\"M 0 0\" /></svg>",
            "</g></g><g",
            "<path fill='#abc' stroke=>",
            "<\u{e9}\u{e9}=\"\u{e9}\">",
            "",
        ] {
            let _ = apply_options(svg, &options);
            assert!(
                vector_bytes(OutputKind::Pdf, svg, &options).is_err(),
                "{svg}"
            );
        }
    }

    #[test]
    fn every_format_is_chosen_by_its_extension() {
        for (name, kind) in [
            ("a.ai", OutputKind::Ai),
            ("a.DXF", OutputKind::Dxf),
            ("a.emf", OutputKind::Emf),
            ("a.svg", OutputKind::Svg),
        ] {
            assert_eq!(output_kind(Path::new(name)), Ok(kind));
        }
        #[cfg(feature = "render")]
        assert_eq!(output_kind(Path::new("a.png")), Ok(OutputKind::Png));
        #[cfg(not(feature = "render"))]
        assert!(output_kind(Path::new("a.png"))
            .unwrap_err()
            .contains("render"));
        let refused = output_kind(Path::new("a.tif")).unwrap_err();
        assert!(refused.contains("DXF, EMF or PNG"), "{refused}");
    }

    #[cfg(feature = "render")]
    #[test]
    fn a_png_is_the_drawing_at_its_declared_size_on_transparency() {
        // 30 x 15 pt is 40 x 20 CSS pixels; the right half is left empty.
        let svg = "<svg width=\"30pt\" height=\"15pt\" viewBox=\"0 0 30 15\"><path fill=\"#ff0000\" d=\"M 0 0 L 15 0 L 15 15 L 0 15 Z\" /></svg>";
        let bytes = vector_bytes(OutputKind::Png, svg, &ExportOptions::default()).unwrap();
        let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (40, 20));
        assert_eq!(image.get_pixel(5, 10).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(35, 10).0[3], 0);
    }
}
