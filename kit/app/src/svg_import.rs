//! SVG files from other programs, read into the app's flat-path SVG
//! (`import::Imported`): usvg (resvg's own parser, already in the app for
//! drawing) resolves styles, `use`, transforms and units, and this module
//! writes each visible path it finds in page space, in points, with its
//! fill and stroke. What flat paths cannot hold is left out and named:
//! gradients and patterns are drawn in one colour (the gradient's middle
//! stop; a rectangle filled with a pattern of solid pieces, as the app saves
//! a dithered area, is drawn as those pieces), clip paths, masks and filters
//! are not applied, embedded images
//! are left out, and text is kept only where the parser drew it as outlines
//! (the app carries no fonts, so usually it is left out).

use crate::import::Imported;
use resvg::usvg;
use std::fmt::Write as _;

/// Read the SVG `bytes` (plain or gzip-compressed SVGZ is not read: the
/// app's usvg carries no gzip).
pub fn to_svg(bytes: &[u8]) -> Result<Imported, String> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|e| format!("This SVG could not be read: {e}"))?;
    let size = tree.size();
    // CSS pixels, 96 to the inch, to points, 72 to the inch.
    let scale = 0.75;
    let (width, height) = (
        f64::from(size.width()) * scale,
        f64::from(size.height()) * scale,
    );
    let mut out = Writer {
        body: String::new(),
        skipped: Skipped::default(),
        scale,
        paths: 0,
        text_outlines: false,
    };
    out.group(tree.root(), 1.0);
    // Without fonts the parser drops text altogether, so the source says
    // whether there was any.
    if !out.text_outlines && String::from_utf8_lossy(bytes).contains("<text") {
        out.skipped.text = true;
    }
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}pt\" height=\"{h}pt\" \
         viewBox=\"0 0 {w} {h}\">",
        w = num(width),
        h = num(height)
    );
    svg.push_str(&out.body);
    svg.push_str("</svg>\n");
    if out.paths == 0 {
        return Err(if out.skipped.is_empty() {
            "This SVG has no shapes to keep.".into()
        } else {
            format!(
                "This SVG has no shapes to keep: it holds only {}.",
                out.skipped.notes().join(", ")
            )
        });
    }
    Ok(Imported {
        svg,
        pages: 1,
        skipped: out.skipped.notes(),
    })
}

/// What was left out, counted.
#[derive(Default)]
struct Skipped {
    images: usize,
    text: bool,
    gradients: bool,
    clipped: bool,
    filtered: bool,
}

impl Skipped {
    fn is_empty(&self) -> bool {
        self.images == 0 && !self.text && !self.gradients && !self.clipped && !self.filtered
    }
    fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.text {
            notes.push("text (outline it in the program that made the file to keep it)".into());
        }
        match self.images {
            0 => {}
            1 => notes.push("1 image".into()),
            n => notes.push(format!("{n} images")),
        }
        if self.gradients {
            notes.push("gradients and patterns, drawn in one colour".into());
        }
        if self.clipped {
            notes.push("clipping and masks".into());
        }
        if self.filtered {
            notes.push("filter effects".into());
        }
        notes
    }
}

struct Writer {
    body: String,
    skipped: Skipped,
    scale: f64,
    paths: usize,
    /// Whether any text came as outlines.
    text_outlines: bool,
}

impl Writer {
    /// Every path under `group`, whose opacity (with its parents') is
    /// `opacity`.
    fn group(&mut self, group: &usvg::Group, opacity: f64) {
        if group.clip_path().is_some() || group.mask().is_some() {
            self.skipped.clipped = true;
        }
        if !group.filters().is_empty() {
            self.skipped.filtered = true;
        }
        let opacity = opacity * f64::from(group.opacity().get());
        for node in group.children() {
            match node {
                usvg::Node::Group(inner) => self.group(inner, opacity),
                usvg::Node::Path(path) => self.path(path, opacity),
                usvg::Node::Image(_) => self.skipped.images += 1,
                usvg::Node::Text(text) => {
                    let outlines = text.flattened();
                    if outlines.children().is_empty() {
                        self.skipped.text = true;
                    } else {
                        self.text_outlines = true;
                        self.group(outlines, opacity);
                    }
                }
            }
        }
    }

    fn path(&mut self, path: &usvg::Path, opacity: f64) {
        if !path.is_visible() || (path.fill().is_none() && path.stroke().is_none()) {
            return;
        }
        let transform = path.abs_transform();
        let Some(data) = path.data().clone().transform(transform) else {
            return;
        };
        if let (Some(fill), None) = (path.fill(), path.stroke()) {
            if let usvg::Paint::Pattern(pattern) = fill.paint() {
                let alpha = opacity * f64::from(fill.opacity().get());
                if self.tiled(path, pattern, alpha) {
                    return;
                }
            }
        }
        let d = self.data(&data);
        if d.is_empty() {
            return;
        }
        let mut element = format!("<path d=\"{d}\"");
        match path.fill() {
            Some(fill) => {
                let rgb = self.colour(fill.paint());
                let _ = write!(element, " fill=\"{rgb}\"");
                if fill.rule() == usvg::FillRule::EvenOdd {
                    element.push_str(" fill-rule=\"evenodd\"");
                }
                let alpha = opacity * f64::from(fill.opacity().get());
                if alpha < 0.999 {
                    let _ = write!(element, " fill-opacity=\"{}\"", num(alpha));
                }
            }
            None => element.push_str(" fill=\"none\""),
        }
        if let Some(stroke) = path.stroke() {
            let rgb = self.colour(stroke.paint());
            // The stroke's width grows with the transform's mean scale.
            let (sx, sy) = transform.get_scale();
            let mean = (f64::from(sx) * f64::from(sy)).sqrt();
            let width = f64::from(stroke.width().get()) * mean * self.scale;
            let _ = write!(element, " stroke=\"{rgb}\" stroke-width=\"{}\"", num(width));
            let join = match stroke.linejoin() {
                usvg::LineJoin::Round => Some("round"),
                usvg::LineJoin::Bevel => Some("bevel"),
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => None,
            };
            if let Some(join) = join {
                let _ = write!(element, " stroke-linejoin=\"{join}\"");
            }
            let cap = match stroke.linecap() {
                usvg::LineCap::Round => Some("round"),
                usvg::LineCap::Square => Some("square"),
                usvg::LineCap::Butt => None,
            };
            if let Some(cap) = cap {
                let _ = write!(element, " stroke-linecap=\"{cap}\"");
            }
            let alpha = opacity * f64::from(stroke.opacity().get());
            if alpha < 0.999 {
                let _ = write!(element, " stroke-opacity=\"{}\"", num(alpha));
            }
        }
        element.push_str("/>\n");
        self.body.push_str(&element);
        self.paths += 1;
    }

    /// A rectangle filled with a pattern of solid, unstroked pieces, as the
    /// app saves a dithered area (`dither.rs`): the tile's pieces repeated
    /// across the rectangle, each subpath kept where it lies wholly inside
    /// it, one path per colour. False, with nothing drawn, for any other
    /// path or pattern (it is drawn in one colour, as before).
    fn tiled(&mut self, path: &usvg::Path, pattern: &usvg::Pattern, alpha: f64) -> bool {
        use usvg::tiny_skia_path::{PathBuilder, PathSegment, Transform};
        /// Pieces drawn, at most, before a pattern is drawn in one colour.
        const MAX_PIECES: usize = 1 << 21;
        let bounds = path.data().bounds();
        let (left, top, right, bottom) =
            (bounds.left(), bounds.top(), bounds.right(), bounds.bottom());
        let rectangle = path.data().segments().all(|segment| match segment {
            PathSegment::MoveTo(p) | PathSegment::LineTo(p) => {
                (p.x == left || p.x == right) && (p.y == top || p.y == bottom)
            }
            PathSegment::Close => true,
            _ => false,
        });
        if !rectangle || !pattern.transform().is_identity() {
            return false;
        }
        // The tile's subpaths with their colours.
        let mut pieces: Vec<(usvg::Color, usvg::tiny_skia_path::Path)> = Vec::new();
        for node in pattern.root().children() {
            let usvg::Node::Path(child) = node else {
                return false;
            };
            let colour = match child.fill().map(|f| f.paint()) {
                Some(usvg::Paint::Color(c)) if child.stroke().is_none() => *c,
                _ => return false,
            };
            let Some(data) = child.data().clone().transform(child.abs_transform()) else {
                return false;
            };
            let mut builder = PathBuilder::new();
            for segment in data.segments() {
                match segment {
                    PathSegment::MoveTo(p) => {
                        if let Some(done) = std::mem::take(&mut builder).finish() {
                            pieces.push((colour, done));
                        }
                        builder.move_to(p.x, p.y);
                    }
                    PathSegment::LineTo(p) => builder.line_to(p.x, p.y),
                    PathSegment::QuadTo(a, p) => builder.quad_to(a.x, a.y, p.x, p.y),
                    PathSegment::CubicTo(a, b, p) => builder.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y),
                    PathSegment::Close => builder.close(),
                }
            }
            if let Some(done) = builder.finish() {
                pieces.push((colour, done));
            }
        }
        let rect = pattern.rect();
        let (w, h) = (rect.width(), rect.height());
        let first = |lo: f32, origin: f32, step: f32| ((lo - origin) / step).floor() as i64;
        let last = |hi: f32, origin: f32, step: f32| ((hi - origin) / step).ceil() as i64;
        let (i0, i1) = (first(left, rect.x(), w), last(right, rect.x(), w));
        let (j0, j1) = (first(top, rect.y(), h), last(bottom, rect.y(), h));
        let tiles = (i1 - i0).max(0) as usize * (j1 - j0).max(0) as usize;
        if pieces.is_empty() || tiles.saturating_mul(pieces.len()) > MAX_PIECES {
            return false;
        }
        let transform = path.abs_transform();
        let slack = 1e-3 * w.max(h);
        let mut by_colour: Vec<(usvg::Color, String)> = Vec::new();
        for j in j0..j1 {
            for i in i0..i1 {
                let at =
                    Transform::from_translate(rect.x() + i as f32 * w, rect.y() + j as f32 * h);
                for (colour, piece) in &pieces {
                    let Some(moved) = piece.clone().transform(at) else {
                        continue;
                    };
                    let b = moved.bounds();
                    let inside = b.left() >= left - slack
                        && b.top() >= top - slack
                        && b.right() <= right + slack
                        && b.bottom() <= bottom + slack;
                    let Some(placed) = inside.then(|| moved.transform(transform)).flatten() else {
                        continue;
                    };
                    let d = self.data(&placed);
                    match by_colour.iter_mut().find(|(c, _)| c == colour) {
                        Some((_, all)) => all.push_str(&d),
                        None => by_colour.push((*colour, d)),
                    }
                }
            }
        }
        for (colour, d) in by_colour {
            let _ = write!(
                self.body,
                "<path d=\"{d}\" fill=\"#{:02x}{:02x}{:02x}\"",
                colour.red, colour.green, colour.blue
            );
            if alpha < 0.999 {
                let _ = write!(self.body, " fill-opacity=\"{}\"", num(alpha));
            }
            self.body.push_str("/>\n");
            self.paths += 1;
        }
        true
    }

    /// `path` as path data in points; quadratics become cubics.
    fn data(&self, path: &usvg::tiny_skia_path::Path) -> String {
        use usvg::tiny_skia_path::PathSegment;
        let s = self.scale;
        let p = |x: f32, y: f32| format!("{} {}", num(f64::from(x) * s), num(f64::from(y) * s));
        let mut d = String::new();
        let mut last = (0f32, 0f32);
        for segment in path.segments() {
            match segment {
                PathSegment::MoveTo(a) => {
                    let _ = write!(d, "M{}", p(a.x, a.y));
                    last = (a.x, a.y);
                }
                PathSegment::LineTo(a) => {
                    let _ = write!(d, "L{}", p(a.x, a.y));
                    last = (a.x, a.y);
                }
                PathSegment::QuadTo(c, a) => {
                    // The cubic through the same curve: its handles two
                    // thirds of the way to the quadratic's control point.
                    let c1 = (
                        last.0 + (c.x - last.0) * 2. / 3.,
                        last.1 + (c.y - last.1) * 2. / 3.,
                    );
                    let c2 = (a.x + (c.x - a.x) * 2. / 3., a.y + (c.y - a.y) * 2. / 3.);
                    let _ = write!(d, "C{} {} {}", p(c1.0, c1.1), p(c2.0, c2.1), p(a.x, a.y));
                    last = (a.x, a.y);
                }
                PathSegment::CubicTo(c1, c2, a) => {
                    let _ = write!(d, "C{} {} {}", p(c1.x, c1.y), p(c2.x, c2.y), p(a.x, a.y));
                    last = (a.x, a.y);
                }
                PathSegment::Close => d.push('Z'),
            }
        }
        d
    }

    /// A paint as one `#rrggbb`: a gradient's middle stop, a pattern grey.
    fn colour(&mut self, paint: &usvg::Paint) -> String {
        let color = match paint {
            usvg::Paint::Color(color) => *color,
            usvg::Paint::LinearGradient(gradient) => {
                self.skipped.gradients = true;
                middle_stop(gradient.stops())
            }
            usvg::Paint::RadialGradient(gradient) => {
                self.skipped.gradients = true;
                middle_stop(gradient.stops())
            }
            usvg::Paint::Pattern(_) => {
                self.skipped.gradients = true;
                usvg::Color::new_rgb(128, 128, 128)
            }
        };
        format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue)
    }
}

fn middle_stop(stops: &[usvg::Stop]) -> usvg::Color {
    stops
        .get(stops.len() / 2)
        .map_or(usvg::Color::black(), |stop| stop.color())
}

/// A coordinate as short as it reads the same to a thousandth.
fn num(value: f64) -> String {
    let text = format!("{:.3}", value);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" || text.is_empty() {
        "0".into()
    } else {
        text.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_comes_back_as_flat_paths_in_points_with_what_was_left_out() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <defs><linearGradient id="g"><stop offset="0" stop-color="#ff0000"/>
            <stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
            <g transform="translate(10 20)" opacity="0.5">
              <rect x="0" y="0" width="40" height="20" fill="#00ff00"/>
            </g>
            <circle cx="100" cy="50" r="10" fill="url(#g)" stroke="#000" stroke-width="4"/>
            <path d="M0 0 Q 10 10 20 0" fill="none" stroke="#123456" fill-rule="evenodd"/>
            <image href="data:image/png;base64,AAAA" width="10" height="10"/>
            <text x="5" y="5">Hi</text>
        </svg>"##;
        let imported = to_svg(svg).unwrap();
        assert!(imported
            .svg
            .starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"150pt\" height=\"75pt\" viewBox=\"0 0 150 75\">"));
        // The rectangle, moved by its group and in points, at half opacity.
        assert!(imported
            .svg
            .contains("M7.5 15L37.5 15L37.5 30L7.5 30Z\" fill=\"#00ff00\" fill-opacity=\"0.5\""));
        // The circle: its gradient drawn in one colour, its stroke 4 px = 3 pt.
        assert!(imported
            .svg
            .contains("stroke=\"#000000\" stroke-width=\"3\""));
        // The quadratic became a cubic with a stroke and no fill.
        assert!(imported.svg.contains("fill=\"none\" stroke=\"#123456\""));
        assert!(imported.skipped.iter().any(|note| note.starts_with("text")));
        assert!(imported
            .skipped
            .contains(&"gradients and patterns, drawn in one colour".to_owned()));
        // The flat SVG is what the app writes as PDF and EPS.
        assert!(crate::pdf_eps::to_pdf(&imported.svg).is_ok());
    }

    #[test]
    fn an_svg_with_nothing_to_keep_is_refused_in_plain_words() {
        let only_text = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>x</text></svg>"#;
        let error = to_svg(only_text).unwrap_err();
        assert!(error.contains("no shapes"), "{error}");
        assert!(to_svg(b"not an svg").is_err());
    }
}
