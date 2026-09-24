//! SVG files from other programs, read into the app's flat-path SVG
//! (`import::Imported`): usvg (resvg's own parser, already in the app for
//! drawing) resolves styles, `use`, transforms and units, and this module
//! writes each visible path it finds in page space, in points, with its
//! fill and stroke. What flat paths cannot hold is left out and named:
//! gradients and patterns are drawn in one colour (the gradient's middle
//! stop), clip paths, masks and filters are not applied, embedded images
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
