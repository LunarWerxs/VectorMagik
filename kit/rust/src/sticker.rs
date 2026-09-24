//! A die-cut sticker around the traced shapes: a border hugging the outer
//! edge of everything in the document, a wider rim outside it (the white
//! edge of a sticker) and an optional hard shadow so a white rim still shows
//! on a white page. Owned post-processing of the engine's output, like
//! `simplify` and `shapes`: the original paths are kept byte for byte and
//! painted last. Underneath them the same paths are painted again, filled and
//! stroked in one colour with a round-joined stroke of twice the wanted
//! width; widening every shape and letting the copies overlap widens their
//! union, so the outer outline of the whole picture appears without any
//! polygon arithmetic, and holes larger than the rim keep a rim of their
//! own. The canvas grows by the sticker's reach so nothing is clipped.
use crate::geometry::Point;
use crate::shapes::{island_at, Island, Removal};
use crate::simplify::path_ranges;

/// Widest border or rim accepted, in source pixels.
pub const MAX_WIDTH: f64 = 512.;
/// Opacity of the shadow layer.
const SHADOW_OPACITY: &str = "0.35";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sticker {
    /// Width of the border hugging the shapes, in source pixels; 0 for none.
    pub border: f64,
    pub border_rgb: [u8; 3],
    /// Width of the rim outside the border, in source pixels; 0 for none.
    pub edge: f64,
    pub edge_rgb: [u8; 3],
    /// A hard shadow displaced down and right by `shadow_offset`.
    pub shadow: bool,
}
impl Default for Sticker {
    fn default() -> Self {
        Self::for_size(250, 250)
    }
}
impl Sticker {
    /// A black border and a white rim sized for a `width` by `height` image:
    /// the border about 1.2% of the longer side, the rim twice that.
    pub fn for_size(width: usize, height: usize) -> Self {
        let border = (width.max(height) as f64 * 0.012).round().clamp(2., 24.);
        Self {
            border,
            border_rgb: [0, 0, 0],
            edge: border * 2.,
            edge_rgb: [255, 255, 255],
            shadow: false,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        for width in [self.border, self.edge] {
            if !width.is_finite() || !(0. ..=MAX_WIDTH).contains(&width) {
                return Err(format!("Sticker widths must be 0 to {MAX_WIDTH} pixels"));
            }
        }
        if self.border + self.edge <= 0. {
            return Err("A sticker needs a border or a rim".into());
        }
        Ok(())
    }
    /// How far the shadow is displaced, in source pixels: 40% of the
    /// outline's width, at least one pixel; 0 without a shadow.
    pub fn shadow_offset(&self) -> f64 {
        if self.shadow {
            ((self.border + self.edge) * 0.4).round().max(1.)
        } else {
            0.
        }
    }
    /// How far anything the sticker paints reaches outside a shape.
    pub fn reach(&self) -> f64 {
        self.border + self.edge + self.shadow_offset()
    }
    /// The canvas grows by this on every side, in whole source pixels.
    pub fn margin(&self) -> f64 {
        self.reach().ceil()
    }
}

/// The document with the sticker painted under its shapes and the canvas
/// widened by the sticker's margin on every side. Every coordinate stays as
/// written; only the root element's `viewBox`, `width` and `height` change.
pub fn apply(svg: &str, sticker: &Sticker) -> Result<String, String> {
    sticker.validate()?;
    let start = svg.find("<svg").ok_or("Not an SVG document")?;
    let end = start
        + svg[start..]
            .find('>')
            .ok_or_else(|| "Unterminated <svg> tag".to_owned())?;
    let tag = &svg[start..end];
    let view: Vec<f64> = attribute(tag, "viewBox")
        .ok_or("The <svg> tag has no viewBox")?
        .1
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<f64>()
                .map_err(|_| format!("Bad viewBox number {t:?}"))
        })
        .collect::<Result<_, _>>()?;
    if view.len() != 4 || view.iter().any(|v| !v.is_finite()) || view[2] <= 0. || view[3] <= 0. {
        return Err("The viewBox must hold four numbers with a positive size".into());
    }
    let margin = sticker.margin();
    let mut root = replace_attribute(
        tag,
        "viewBox",
        &format!(
            "{} {} {} {}",
            number(view[0] - margin),
            number(view[1] - margin),
            number(view[2] + 2. * margin),
            number(view[3] + 2. * margin)
        ),
    )?;
    // The declared size keeps its unit and its ratio to the viewBox.
    for (name, extent) in [("width", view[2]), ("height", view[3])] {
        if let Some((_, value)) = attribute(&root, name) {
            let digits = value
                .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
                .unwrap_or(value.len());
            let declared: f64 = value[..digits]
                .parse()
                .map_err(|_| format!("Bad {name} {value:?}"))?;
            let unit = value[digits..].to_owned();
            let scaled = declared / extent * (extent + 2. * margin);
            root = replace_attribute(&root, name, &format!("{}{unit}", number(scaled)))?;
        }
    }
    let ranges = path_ranges(svg)?;
    let mut layers = String::new();
    let mut layer = |id: &str, rgb: [u8; 3], width: f64, extra: &str| {
        layers.push_str(&format!(
            "<g id=\"sticker-{id}\" fill=\"{color}\" stroke=\"{color}\" stroke-width=\"{width:.2}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{extra}>\n",
            color = hex(rgb),
        ));
        for &(from, to) in &ranges {
            layers.push_str("<path d=\"");
            layers.push_str(&svg[from..to]);
            layers.push_str("\" />\n");
        }
        layers.push_str("</g>\n");
    };
    let outline = sticker.border + sticker.edge;
    if sticker.shadow {
        let offset = number(sticker.shadow_offset());
        layer(
            "shadow",
            [0, 0, 0],
            2. * outline,
            &format!(" opacity=\"{SHADOW_OPACITY}\" transform=\"translate({offset} {offset})\""),
        );
    }
    if sticker.edge > 0. {
        layer("edge", sticker.edge_rgb, 2. * outline, "");
    }
    if sticker.border > 0. {
        layer("border", sticker.border_rgb, 2. * sticker.border, "");
    }
    let rest = &svg[end + 1..];
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
        .unwrap_or(rest);
    let mut out = String::with_capacity(svg.len() + layers.len() + root.len() + 2);
    out.push_str(&svg[..start]);
    out.push_str(&root);
    out.push_str(">\n");
    out.push_str(&layers);
    out.push_str(rest);
    Ok(out)
}

/// The shapes that make up an opaque source's background, as removals: every
/// shape of the colour that covers most of the canvas border, where it
/// touches the border. `opaque(x, y)` says whether the source pixel there is
/// opaque; a border that is mostly transparent has no background to cut, and
/// so has a border no single colour dominates.
pub fn background_removals(
    islands: &[Island],
    width: usize,
    height: usize,
    opaque: impl Fn(usize, usize) -> bool,
) -> Vec<Removal> {
    if width == 0 || height == 0 || islands.is_empty() {
        return Vec::new();
    }
    let step = (2 * (width + height) / 400).max(1);
    let mut samples: Vec<(usize, usize)> = Vec::new();
    for x in (0..width).step_by(step) {
        samples.push((x, 0));
        samples.push((x, height - 1));
    }
    for y in (0..height).step_by(step) {
        samples.push((0, y));
        samples.push((width - 1, y));
    }
    // The corners are on both a row and a column: one vote each.
    samples.sort_unstable();
    samples.dedup();
    let mut covered = 0;
    let mut counts: Vec<(String, usize)> = Vec::new();
    for &(x, y) in &samples {
        if !opaque(x, y) {
            continue;
        }
        covered += 1;
        let at = Point {
            x: x as f64 + 0.5,
            y: y as f64 + 0.5,
        };
        if let Some(index) = island_at(islands, at) {
            let color = islands[index].color.to_ascii_lowercase();
            match counts.iter_mut().find(|(c, _)| *c == color) {
                Some(entry) => entry.1 += 1,
                None => counts.push((color, 1)),
            }
        }
    }
    if covered * 2 < samples.len() {
        return Vec::new();
    }
    let Some((color, count)) = counts.into_iter().max_by_key(|(_, n)| *n) else {
        return Vec::new();
    };
    if count * 3 < covered {
        return Vec::new();
    }
    let touches = |island: &Island| {
        island.min.x <= 1.
            || island.min.y <= 1.
            || island.max.x >= width as f64 - 1.
            || island.max.y >= height as f64 - 1.
    };
    islands
        .iter()
        .filter(|island| island.color.eq_ignore_ascii_case(&color) && touches(island))
        .filter_map(|island| {
            island.probe().map(|at| Removal {
                color: island.color.clone(),
                at,
            })
        })
        .collect()
}

/// The value of ` name="..."` in a tag with the value's byte range.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<((usize, usize), &'a str)> {
    let needle = format!(" {name}=\"");
    let at = tag.find(&needle)? + needle.len();
    let close = at + tag[at..].find('"')?;
    Some(((at, close), &tag[at..close]))
}

fn replace_attribute(tag: &str, name: &str, value: &str) -> Result<String, String> {
    let ((at, close), _) =
        attribute(tag, name).ok_or_else(|| format!("The <svg> tag has no {name}"))?;
    Ok(format!("{}{value}{}", &tag[..at], &tag[close..]))
}

/// A number with at most two decimals and no trailing zeros.
fn number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::{islands, remove_islands};

    const DOC: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<svg width=\"100pt\" height=\"80pt\" viewBox=\"0 0 100 80\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"#ffffffff\">\n<path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 0.00 L 100.00 0.00 L 100.00 80.00 L 0.00 80.00 L 0.00 0.00 M 20.00 20.00 L 20.00 60.00 L 80.00 60.00 L 80.00 20.00 L 20.00 20.00 Z\" />\n</g>\n<g id=\"#ff0000ff\">\n<path fill=\"#ff0000\" opacity=\"1.00\" d=\" M 20.00 20.00 L 80.00 20.00 L 80.00 60.00 L 20.00 60.00 L 20.00 20.00 Z\" />\n</g>\n</svg>\n";

    #[test]
    fn sticker_layers_go_under_the_shapes_and_the_canvas_grows() {
        let sticker = Sticker {
            border: 3.,
            border_rgb: [0, 0, 0],
            edge: 5.,
            edge_rgb: [255, 255, 255],
            shadow: true,
        };
        assert_eq!(sticker.shadow_offset(), 3., "40% of 8, rounded");
        assert_eq!(sticker.margin(), 11.);
        let out = apply(DOC, &sticker).unwrap();
        assert!(out.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<svg width=\"122pt\" height=\"102pt\" viewBox=\"-11 -11 122 102\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"sticker-shadow\" fill=\"#000000\" stroke=\"#000000\" stroke-width=\"16.00\" stroke-linejoin=\"round\" stroke-linecap=\"round\" opacity=\"0.35\" transform=\"translate(3 3)\">\n<path d=\" M 0.00 0.00"), "{out}");
        let shadow = out.find("sticker-shadow").unwrap();
        let edge = out.find("sticker-edge").unwrap();
        let border = out.find("sticker-border").unwrap();
        let shapes = out.find("<g id=\"#ffffffff\">").unwrap();
        assert!(shadow < edge && edge < border && border < shapes);
        assert!(out.contains(
            "<g id=\"sticker-edge\" fill=\"#ffffff\" stroke=\"#ffffff\" stroke-width=\"16.00\""
        ));
        assert!(out.contains(
            "<g id=\"sticker-border\" fill=\"#000000\" stroke=\"#000000\" stroke-width=\"6.00\""
        ));
        assert_eq!(out.matches("<path d=\"").count(), 6, "two paths per layer");
        assert!(
            out.ends_with(&DOC[DOC.find("<g id=\"#ffffffff\">").unwrap()..]),
            "the shapes are untouched"
        );
        // Rim and shadow off: one layer, and the margin is the border alone.
        let plain = Sticker {
            edge: 0.,
            shadow: false,
            ..sticker
        };
        assert_eq!(plain.margin(), 3.);
        let out = apply(DOC, &plain).unwrap();
        assert!(out.contains("viewBox=\"-3 -3 106 86\""));
        assert_eq!(out.matches("<g id=\"sticker-").count(), 1);
        assert!(apply(
            DOC,
            &Sticker {
                border: 0.,
                edge: 0.,
                ..plain
            }
        )
        .is_err());
        assert!(apply("<html>", &plain).is_err());
        assert!(apply("<svg width=\"1\" height=\"1\">", &plain).is_err());
        // A document already declared in pixels keeps its ratio.
        let doubled = DOC.replacen(
            "width=\"100pt\" height=\"80pt\"",
            "width=\"200\" height=\"160\"",
            1,
        );
        let out = apply(&doubled, &plain).unwrap();
        assert!(
            out.contains("<svg width=\"212\" height=\"172\" viewBox=\"-3 -3 106 86\""),
            "{out}"
        );
    }

    #[test]
    fn the_background_is_the_border_colour_and_its_removal_leaves_the_object() {
        let all = islands(DOC).unwrap();
        let removals = background_removals(&all, 100, 80, |_, _| true);
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].color, "#ffffff");
        let (out, removed) = remove_islands(DOC, &removals).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(islands(&out).unwrap().len(), 1, "the red square stays");
        // A transparent border has no background to cut.
        assert!(background_removals(&all, 100, 80, |_, _| false).is_empty());
        assert!(background_removals(&all, 100, 80, |x, y| x == 0 && y < 10).is_empty());
        assert!(background_removals(&[], 100, 80, |_, _| true).is_empty());
    }

    #[test]
    fn defaults_follow_the_image_size() {
        let small = Sticker::for_size(250, 250);
        assert_eq!((small.border, small.edge), (3., 6.));
        let large = Sticker::for_size(4000, 3000);
        assert_eq!((large.border, large.edge), (24., 48.));
        let tiny = Sticker::for_size(16, 16);
        assert_eq!((tiny.border, tiny.edge), (2., 4.));
        assert!(!small.shadow && small.shadow_offset() == 0.);
        assert_eq!(number(-0.001), "0");
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(250.), "250");
    }
}
