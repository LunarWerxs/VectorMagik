//! The vector export recovered from the original: hole loops 0x47d4c0, the
//! document size 0x4745d0, colour grouping 0x47c2f0, the shape walk 0x47c660
//! with piece emission 0x4741b0, and the SVG writer behind vtable 0x8dcfa0
//! (0x476170 document start, 0x475b40 visibility, 0x475de0 / 0x475a80
//! groups, 0x475f20 / 0x475ac0 paths, 0x475b50 lines, 0x475c50 cubics,
//! 0x475af0 document end). The text is byte for byte the original's,
//! including the CR LF the text-mode `_wfopen(path, L"w")` stream wrote for
//! every `endl`.
use crate::geometry::{Cubic, Point};
use crate::recovered_state::FinalPart;
use std::collections::HashMap;

/// One contour record as the export reads it (0x60 bytes at fitter+0x20).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportShape {
    /// Contour+0x14: the region colour bytes, blue, green, red, alpha.
    pub colour: [u8; 4],
    /// Contour+0x30: the enclosing contour, or -1.
    pub parent: i32,
    /// Contour+0x18 / +0x1c: the node ids around the contour.
    pub nodes: Vec<usize>,
    /// Contour+0x40 / +0x44: the fitting's line and curve records.
    pub pieces: Vec<FinalPart>,
}

/// The export settings the original keeps at export+0x0 and export+0x2c.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSettings {
    /// export+0x2c: 0 paths only, 1 paths with hole loops, 2 hole loops and
    /// one group per colour in order of first appearance.
    pub layering: i32,
    /// export+0x34: 0 strokes every path with its own fill colour at width
    /// 0.09375, anything else leaves the paths unstroked.
    pub stroking: i32,
    /// export+0x0: the resolution the node coordinates are in.
    pub dpi_base: i32,
    /// The 0x4745d0 argument; the application passes 72.
    pub dpi: i32,
    /// Owned, off in the original (and in `new`): write the straight colour
    /// of every shape the writer gives an opacity below 1.00 (alpha 11 to
    /// 250), `min(255, round(c * 255 / alpha))`, in its fill and group id.
    /// The region colours are means of premultiplied pixels (preprocessing
    /// stores `c * alpha / 255`), and the original writes those bytes with
    /// the opacity as well, so a translucent region renders darker by its
    /// alpha: (255, 0, 0) at alpha 128 becomes `#800000` at 0.50, (191, 128,
    /// 128) over white instead of (255, 128, 128). Alpha above 250 is written
    /// as 1.00 and keeps its bytes.
    pub straight_fills: bool,
}

impl ExportSettings {
    pub fn new(layering: i32, stroking: i32, dpi_base: i32) -> Self {
        Self {
            layering,
            stroking,
            dpi_base,
            dpi: 72,
            straight_fills: false,
        }
    }
}

/// `ExportSettings::straight_fills` for one region colour (blue, green, red,
/// alpha): the premultiplied channels divided by the alpha the writer prints
/// as an opacity below 1.00, rounded half up; other colours unchanged.
pub fn straight_colour(colour: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(colour[3]);
    if alpha == 0 || alpha > 0xfa {
        return colour;
    }
    let channel = |c: u8| ((u32::from(c) * 255 + alpha / 2) / alpha).min(255) as u8;
    [
        channel(colour[0]),
        channel(colour[1]),
        channel(colour[2]),
        colour[3],
    ]
}

/// What 0x4745d0 leaves at export+0x68..+0x74 and the text 0x47ca40 writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// export+0x68 / +0x6c: the largest node coordinate, rounded half up.
    pub max_x: i32,
    pub max_y: i32,
    /// export+0x70 / +0x74: the maxima scaled from `dpi_base` to `dpi`.
    pub width: i32,
    pub height: i32,
    pub svg: String,
}

/// For every shape, its closed hole loops; each entry is (child shape, piece index).
pub type HoleLoops = Vec<Vec<Vec<(usize, usize)>>>;

/// 0x47d4c0: for every shape, the closed loops its children's pieces form
/// along their shared boundary. Each entry is (child shape, piece index).
pub fn hole_loops(shapes: &[ExportShape]) -> Result<HoleLoops, String> {
    let n = shapes.len();
    let mut children = vec![Vec::new(); n];
    for (i, shape) in shapes.iter().enumerate() {
        if shape.parent != -1 {
            let parent = usize::try_from(shape.parent).ok().filter(|&p| p < n);
            children[parent.ok_or_else(|| format!("Shape {i} has an invalid parent"))?].push(i);
        }
    }
    let mut holes = vec![Vec::new(); n];
    for (i, loops) in holes.iter_mut().enumerate() {
        // Runs of consecutive pieces bordering shape i, per child, in the
        // child's own piece order; a run never wraps around the end.
        let mut runs: Vec<Vec<(usize, usize)>> = Vec::new();
        for &child in &children[i] {
            let mut run = Vec::new();
            for (j, piece) in shapes[child].pieces.iter().enumerate() {
                if piece.edge == i as u32 {
                    run.push((child, j));
                } else if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            }
            if !run.is_empty() {
                runs.push(run);
            }
        }
        let piece = |entry: (usize, usize)| -> Result<&FinalPart, String> {
            shapes[entry.0]
                .pieces
                .get(entry.1)
                .ok_or_else(|| format!("Shape {} has no piece {}", entry.0, entry.1))
        };
        // Chain runs end to start; the last matching run wins, as the
        // original's scan stores every match. The last run starting at each
        // node, looked up once, is that scan's answer without scanning every
        // run for every run (quadratic in a parent's holes: noise at 512 px,
        // the defect sweep of September 23, 2026).
        let mut last_starting: HashMap<usize, usize> = HashMap::new();
        for (l, other) in runs.iter().enumerate() {
            last_starting.insert(piece(other[0])?.start_node, l);
        }
        let mut next = vec![-1i32; runs.len()];
        for (k, run) in runs.iter().enumerate() {
            let end = piece(*run.last().unwrap())?.end_node;
            if let Some(&l) = last_starting.get(&end) {
                next[k] = l as i32;
            }
        }
        for k in 0..runs.len() {
            if next[k] < 0 {
                continue;
            }
            let mut entries = Vec::new();
            let mut current = k;
            loop {
                entries.extend_from_slice(&runs[current]);
                let following = next[current] as usize;
                next[current] = -1;
                if next[following] < 0 {
                    break;
                }
                current = following;
            }
            loops.push(entries);
        }
    }
    Ok(holes)
}

/// 0x4745d0: the largest coordinates and the scaled document size, with the
/// original's single-precision scaling.
pub fn document_size(nodes: &[Point], settings: &ExportSettings) -> (i32, i32, i32, i32) {
    let (mut max_x, mut max_y) = (0i32, 0i32);
    for node in nodes {
        if node.x > f64::from(max_x) {
            max_x = (node.x + 0.5) as i32;
        }
        if node.y > f64::from(max_y) {
            max_y = (node.y + 0.5) as i32;
        }
    }
    let scale = settings.dpi as f32;
    let inverse = 1.0f32 / settings.dpi_base as f32;
    let width = ((scale * max_x as f32) * inverse) as i32;
    let height = ((scale * max_y as f32) * inverse) as i32;
    (max_x, max_y, width, height)
}

/// 0x47c2f0: shape indices grouped by colour in order of first appearance,
/// each tagged 0 first of a group, 3 inside, 1 last, 2 alone.
pub fn colour_order(shapes: &[ExportShape]) -> Vec<(usize, i32)> {
    // Each colour's group found by a map, not a scan of the groups: a
    // photograph of noise has a colour per region (219k at 512 px, where the
    // scan was half the engine's time).
    let mut groups: Vec<(u32, Vec<usize>)> = Vec::new();
    let mut group_of: HashMap<u32, usize> = HashMap::new();
    for (i, shape) in shapes.iter().enumerate() {
        let key = u32::from_le_bytes(shape.colour);
        match group_of.get(&key) {
            Some(&g) => groups[g].1.push(i),
            None => {
                group_of.insert(key, groups.len());
                groups.push((key, vec![i]));
            }
        }
    }
    let mut order = Vec::with_capacity(shapes.len());
    for (_, members) in groups {
        let count = members.len();
        for (position, &index) in members.iter().enumerate() {
            let tag = if position == 0 {
                if count == 1 {
                    2
                } else {
                    0
                }
            } else if position == count - 1 {
                1
            } else {
                3
            };
            order.push((index, tag));
        }
    }
    order
}

/// MSVCR71's "%.2f", which the writer's `std::fixed` stream with precision 2
/// and its `sprintf("%0.2f")` both reach: the exact binary value's digits,
/// rounded half away from zero.
pub fn fixed2(x: f64) -> String {
    let mut text = String::with_capacity(12);
    push_fixed2(&mut text, x);
    text
}

/// [`fixed2`] appended to `text`. Off an exact tie, rounding the exact value
/// half away from zero is rounding it to the nearest, which Rust's `{:.2}`
/// does from the exact binary value as well; a tie (`k + 1/2` hundredths) is
/// a binary fraction only as an odd number of eighths, so exactly the values
/// whose `8 * |x|` is an odd integer, and those and the non-finite values
/// take the digit-by-digit path of `fixed2_exact`. The same text for every
/// input (`fast_fixed2_matches_the_exact_digits`), without the 64 digits per
/// coordinate the exact path formats (September 22, 2026).
pub fn push_fixed2(text: &mut String, x: f64) {
    use std::fmt::Write;
    let a = x.abs();
    let eighths = a * 8.0;
    if !a.is_finite() || (eighths.fract() == 0.0 && eighths % 2.0 == 1.0) {
        text.push_str(&fixed2_exact(x));
        return;
    }
    if x.is_sign_negative() {
        text.push('-');
    }
    write!(text, "{a:.2}").expect("writing to a String cannot fail");
}

/// "%.2f" from the exact binary value's first 64 decimals, rounded half
/// away from zero by hand.
fn fixed2_exact(x: f64) -> String {
    let exact = format!("{:.64}", x.abs());
    let (integer, fraction) = exact.split_once('.').unwrap();
    let mut digits: Vec<u8> = integer.bytes().chain(fraction.bytes().take(2)).collect();
    if fraction.as_bytes()[2] >= b'5' {
        let mut at = digits.len();
        loop {
            if at == 0 {
                digits.insert(0, b'1');
                break;
            }
            at -= 1;
            if digits[at] == b'9' {
                digits[at] = b'0';
            } else {
                digits[at] += 1;
                break;
            }
        }
    }
    let split = digits.len() - 2;
    let mut text = String::with_capacity(digits.len() + 2);
    if x.is_sign_negative() {
        text.push('-');
    }
    text.push_str(std::str::from_utf8(&digits[..split]).unwrap());
    text.push('.');
    text.push_str(std::str::from_utf8(&digits[split..]).unwrap());
    text
}

/// The SVG writer 0x8dcfa0 over an owned string.
struct Writer {
    text: String,
    stroking: i32,
}

impl Writer {
    fn endl(&mut self) {
        self.text.push_str("\r\n");
    }
    fn number(&mut self, x: f64) {
        push_fixed2(&mut self.text, x);
    }
    /// 0x476170.
    fn begin_document(&mut self, max_x: i32, max_y: i32, width: i32, height: i32) {
        self.text
            .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>");
        self.endl();
        self.text.push_str(
            "<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \
             \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">",
        );
        self.endl();
        self.text.push_str(&format!(
            "<svg width=\"{width}pt\" height=\"{height}pt\" viewBox=\"0 0 {max_x} {max_y}\" \
             version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">"
        ));
        // No line end here: every group, path and the closing tag starts
        // with its own.
    }
    /// 0x475b40: a shape shows when its alpha exceeds 10.
    fn visible(colour: [u8; 4]) -> bool {
        colour[3] > 0x0a
    }
    /// 0x475de0 with 0x475eb0: red, green, blue, alpha.
    fn open_group(&mut self, colour: [u8; 4]) {
        self.endl();
        self.text.push_str(&format!(
            "<g id=\"#{:02x}{:02x}{:02x}{:02x}\">",
            colour[2], colour[1], colour[0], colour[3]
        ));
    }
    /// 0x475a80.
    fn close_group(&mut self) {
        self.endl();
        self.text.push_str("</g>");
    }
    /// 0x475f20 with 0x476090 (fill) and 0x4760f0 (opacity).
    fn begin_path(&mut self, colour: [u8; 4]) {
        let fill = format!("{:02x}{:02x}{:02x}", colour[2], colour[1], colour[0]);
        self.endl();
        self.text.push_str("<path fill=\"#");
        self.text.push_str(&fill);
        if self.stroking == 0 {
            self.text.push_str("\" stroke=\"#");
            self.text.push_str(&fill);
            self.text.push_str("\" stroke-width=\"0.09375");
        }
        self.text.push('"');
        self.text.push_str(" opacity=\"");
        if colour[3] > 0xfa {
            self.text.push_str("1.00");
        } else {
            self.number(f64::from(colour[3]) * 0.00392156862745098);
        }
        self.text.push('"');
        self.text.push_str(" d=\"");
    }
    /// 0x475ac0.
    fn end_path(&mut self) {
        self.text.push_str(" Z\" />");
    }
    fn point(&mut self, p: Point) {
        self.number(p.x);
        self.text.push(' ');
        self.number(p.y);
    }
    /// 0x475b50: `first` 1 or 2 opens a subpath at the start point.
    fn line(&mut self, from: Point, to: Point, first: i32) {
        if first == 1 || first == 2 {
            self.text.push_str(" M ");
            self.point(from);
        }
        self.text.push_str(" L ");
        self.point(to);
    }
    /// 0x475c50.
    fn cubic(&mut self, points: [Point; 4], first: i32) {
        if first == 1 || first == 2 {
            self.text.push_str(" M ");
            self.point(points[0]);
        }
        self.text.push_str(" C ");
        self.point(points[1]);
        self.text.push(' ');
        self.point(points[2]);
        self.text.push(' ');
        self.point(points[3]);
    }
    /// 0x475af0.
    fn end_document(&mut self) {
        self.endl();
        self.text.push_str("</svg>");
        self.endl();
    }
}

/// 0x4741b0: one piece of `shape`, reversed when it is walked as a hole.
#[allow(
    clippy::too_many_arguments,
    reason = "the original's calling convention: every argument is one field the piece walk needs"
)]
fn emit_piece(
    writer: &mut Writer,
    shapes: &[ExportShape],
    nodes: &[Point],
    curves: &[Cubic],
    shape: usize,
    piece: &FinalPart,
    first: i32,
    hole: bool,
) -> Result<(), String> {
    let mut kind = piece.kind;
    if hole {
        kind = match kind {
            1 => 2,
            2 => 1,
            k => k,
        };
    }
    match kind {
        0 => {
            let ids = &shapes[shape].nodes;
            if ids.is_empty() || piece.index >= ids.len() {
                return Err(format!(
                    "Shape {shape} has no node position {}",
                    piece.index
                ));
            }
            let a = ids[piece.index];
            let b = ids[(piece.index + 1) % ids.len()];
            let (a, b) = (
                *nodes.get(a).ok_or_else(|| format!("Missing node {a}"))?,
                *nodes.get(b).ok_or_else(|| format!("Missing node {b}"))?,
            );
            if hole {
                writer.line(b, a, first);
            } else {
                writer.line(a, b, first);
            }
        }
        1 | 2 => {
            let curve = curves
                .get(piece.index)
                .ok_or_else(|| format!("Missing curve {}", piece.index))?;
            let mut points = curve.points;
            if kind == 2 {
                points.reverse();
            }
            writer.cubic(points, first);
        }
        _ => {}
    }
    Ok(())
}

/// 0x47d4c0, 0x4745d0 and 0x47ca40 in sequence: the document the original
/// writes for these fitted shapes.
pub fn export(
    shapes: &[ExportShape],
    nodes: &[Point],
    curves: &[Cubic],
    settings: &ExportSettings,
) -> Result<Document, String> {
    if settings.dpi_base == 0 {
        return Err("Export resolution base is zero".into());
    }
    let holes = hole_loops(shapes)?;
    let (max_x, max_y, width, height) = document_size(nodes, settings);
    let mut writer = Writer {
        text: String::new(),
        stroking: settings.stroking,
    };
    writer.begin_document(max_x, max_y, width, height);
    let grouped = settings.layering == 2;
    let order = if grouped {
        colour_order(shapes)
    } else {
        (0..shapes.len()).map(|i| (i, 0)).collect()
    };
    let with_holes = settings.layering == 1 || settings.layering == 2;
    for (index, tag) in order {
        let (open, close) = if grouped {
            (tag == 0 || tag == 2, tag == 1 || tag == 2)
        } else {
            (false, false)
        };
        let shape = &shapes[index];
        if !Writer::visible(shape.colour) {
            continue;
        }
        let colour = if settings.straight_fills {
            straight_colour(shape.colour)
        } else {
            shape.colour
        };
        if open {
            writer.open_group(colour);
        }
        // The original skips the shapes its bitset 0x46c330 hides
        // (export+0x3c / +0x44); nothing sets a bit, so every shape is written.
        writer.begin_path(colour);
        for (p, piece) in shape.pieces.iter().enumerate() {
            let first = i32::from(p == 0);
            emit_piece(
                &mut writer,
                shapes,
                nodes,
                curves,
                index,
                piece,
                first,
                false,
            )?;
        }
        if with_holes {
            for entries in &holes[index] {
                // The list is walked backwards, every piece reversed.
                for (n, &(child, j)) in entries.iter().enumerate().rev() {
                    let first = if n + 1 == entries.len() { 2 } else { 0 };
                    let piece = &shapes[child].pieces[j];
                    emit_piece(
                        &mut writer,
                        shapes,
                        nodes,
                        curves,
                        child,
                        piece,
                        first,
                        true,
                    )?;
                }
            }
        }
        writer.end_path();
        if close {
            writer.close_group();
        }
    }
    writer.end_document();
    Ok(Document {
        max_x,
        max_y,
        width,
        height,
        svg: writer.text,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Case {
        pub(crate) id: i64,
        pub(crate) layering: i32,
        pub(crate) stroking: i32,
        pub(crate) dpi_base: i32,
        pub(crate) shapes: Vec<ExportShape>,
        pub(crate) nodes: Vec<Point>,
        pub(crate) curves: Vec<Cubic>,
        pub(crate) size: (i32, i32, i32, i32),
        pub(crate) svg: String,
    }

    pub(crate) fn parse(line: &str) -> Case {
        let fields: Vec<&str> = line.split(',').collect();
        let id: i64 = fields[1].parse().unwrap();
        let layering: i32 = fields[6].parse().unwrap();
        let stroking: i32 = fields[7].parse().unwrap();
        // Unused curve and node slots hold the original's NaN sentinels,
        // which MSVCR71 prints as "-1.#QNAN".
        let v: Vec<f64> = fields[8..fields.len() - 1]
            .iter()
            .map(|f| {
                if f.contains('#') {
                    f64::NAN
                } else {
                    f.parse().unwrap()
                }
            })
            .collect();
        let mut at = 0;
        let mut take = || {
            let x = v[at];
            at += 1;
            x
        };
        let dpi_base = take() as i32;
        let shape_count = take() as usize;
        let mut shapes = Vec::with_capacity(shape_count);
        for _ in 0..shape_count {
            let mut colour = [0u8; 4];
            for c in colour.iter_mut() {
                *c = take() as u8;
            }
            let parent = take() as i32;
            let count = take() as usize;
            let nodes = (0..count).map(|_| take() as usize).collect();
            let pieces = (0..take() as usize)
                .map(|_| {
                    let w: Vec<f64> = (0..7).map(|_| take()).collect();
                    FinalPart {
                        index: w[0] as usize,
                        kind: w[1] as u32,
                        edge: w[2] as i32 as u32,
                        start_node: w[3] as usize,
                        end_node: w[4] as usize,
                        start_position: w[5] as usize,
                        end_position: w[6] as usize,
                    }
                })
                .collect();
            shapes.push(ExportShape {
                colour,
                parent,
                nodes,
                pieces,
            });
        }
        let nodes = (0..take() as usize)
            .map(|_| Point {
                x: take(),
                y: take(),
            })
            .collect();
        let curves = (0..take() as usize)
            .map(|_| Cubic {
                points: std::array::from_fn(|_| Point {
                    x: take(),
                    y: take(),
                }),
            })
            .collect();
        let size = (take() as i32, take() as i32, take() as i32, take() as i32);
        assert_eq!(at, v.len());
        let hex = fields[fields.len() - 1].trim();
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        Case {
            id,
            layering,
            stroking,
            dpi_base,
            shapes,
            nodes,
            curves,
            size,
            svg: String::from_utf8(bytes).unwrap(),
        }
    }

    pub(crate) fn cases() -> Vec<Case> {
        include_str!("../fixtures/native-export.csv")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(parse)
            .collect()
    }

    #[test]
    fn export_matches_native_text() {
        let cases = cases();
        assert_eq!(cases.len(), 126);
        let mut checked = 0;
        for case in &cases {
            let settings = ExportSettings::new(case.layering, case.stroking, case.dpi_base);
            let document = export(&case.shapes, &case.nodes, &case.curves, &settings)
                .unwrap_or_else(|e| panic!("case {}: {e}", case.id));
            assert_eq!(
                (
                    document.max_x,
                    document.max_y,
                    document.width,
                    document.height
                ),
                case.size,
                "case {} size",
                case.id
            );
            assert_eq!(document.svg, case.svg, "case {} text", case.id);
            checked += 1;
        }
        assert_eq!(checked, 126);
    }

    #[test]
    fn every_layering_and_stroking_combination_is_covered() {
        let cases = cases();
        for layering in 0..3 {
            for stroking in 0..2 {
                assert!(cases
                    .iter()
                    .any(|c| c.layering == layering && c.stroking == stroking));
            }
        }
        assert!(cases.iter().any(|c| c.svg.contains("<g id=")));
        assert!(cases.iter().any(|c| c.svg.contains("stroke-width")));
        assert!(cases
            .iter()
            .any(|c| c.svg.matches(" M ").count() > c.shapes.len()));
    }

    /// The owned straight fills: a translucent region's premultiplied bytes
    /// divided by its alpha, in the fill, the stroke and the group id, with
    /// the opacity and every coordinate as the original writes them; alpha
    /// above 250 (written as 1.00) and every native case are unchanged.
    #[test]
    fn straight_fills_undo_the_premultiplied_region_colour() {
        // Blue, green, red, alpha: (255, 0, 0) and (20, 40, 60) at 128.
        assert_eq!(straight_colour([0, 0, 128, 128]), [0, 0, 255, 128]);
        assert_eq!(straight_colour([30, 20, 10, 128]), [60, 40, 20, 128]);
        assert_eq!(straight_colour([7, 11, 11, 11]), [162, 255, 255, 11]);
        assert_eq!(straight_colour([200, 90, 3, 251]), [200, 90, 3, 251]);
        assert_eq!(straight_colour([9, 8, 7, 0]), [9, 8, 7, 0]);
        for alpha in 11..=250u8 {
            for c in 0..=alpha {
                let straight = straight_colour([c, 0, c, alpha]);
                let back = (u32::from(straight[0]) * u32::from(alpha) + 127) / 255;
                assert_eq!(back, u32::from(c), "alpha {alpha}, channel {c}");
            }
        }
        let shape = |colour: [u8; 4], parent: i32, nodes: Vec<usize>| ExportShape {
            colour,
            parent,
            pieces: (0..nodes.len())
                .map(|k| FinalPart {
                    index: k,
                    kind: 0,
                    edge: u32::MAX,
                    start_node: nodes[k],
                    end_node: nodes[(k + 1) % nodes.len()],
                    start_position: k,
                    end_position: (k + 1) % nodes.len(),
                })
                .collect(),
            nodes,
        };
        let nodes = [
            (0., 0.),
            (8., 0.),
            (8., 6.),
            (0., 6.),
            (2., 2.),
            (4., 2.),
            (4., 4.),
        ]
        .map(|(x, y)| Point { x, y });
        let shapes = [
            shape([0, 0, 128, 128], -1, vec![0, 1, 2, 3]),
            shape([30, 20, 10, 254], -1, vec![4, 5, 6]),
        ];
        for stroking in [0, 1] {
            let original = ExportSettings::new(2, stroking, 72);
            let straight = ExportSettings {
                straight_fills: true,
                ..original.clone()
            };
            let before = export(&shapes, &nodes, &[], &original).unwrap().svg;
            let after = export(&shapes, &nodes, &[], &straight).unwrap().svg;
            assert!(before.contains("<g id=\"#80000080\">") && before.contains("#800000\""));
            assert_eq!(
                after,
                before
                    .replace("#80000080", "#ff000080")
                    .replace("#800000\"", "#ff0000\"")
            );
            assert!(after.contains("opacity=\"0.50\"") && after.contains("#0a141efe"));
        }
        for case in &cases() {
            let settings = ExportSettings {
                straight_fills: true,
                ..ExportSettings::new(case.layering, case.stroking, case.dpi_base)
            };
            if case.shapes.iter().all(|s| s.colour[3] > 0xfa) {
                let document = export(&case.shapes, &case.nodes, &case.curves, &settings);
                assert_eq!(document.unwrap().svg, case.svg, "case {}", case.id);
            }
        }
    }

    #[test]
    fn fixed2_rounds_half_away_from_zero_on_the_exact_value() {
        assert_eq!(fixed2(0.125), "0.13");
        assert_eq!(fixed2(2.675), "2.67");
        assert_eq!(fixed2(9.995), "9.99");
        assert_eq!(fixed2(9.9950000000000001), "9.99");
        assert_eq!(fixed2(0.375), "0.38");
        assert_eq!(fixed2(99.999), "100.00");
        assert_eq!(fixed2(0.0), "0.00");
        assert_eq!(fixed2(-0.001), "-0.00");
        assert_eq!(fixed2(13.333333333333334), "13.33");
        assert_eq!(fixed2(18.666666666666668), "18.67");
    }

    /// The fast path of `push_fixed2` against the exact digits: random
    /// doubles over the coordinate range and beyond, the ties (odd eighths)
    /// and their neighbours one unit either side, hundredths and their
    /// halves, signed zeros, subnormals and huge values.
    #[test]
    fn fast_fixed2_matches_the_exact_digits() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut values = vec![0.0, -0.0, 5e-324, -5e-324, 1e-300, 1e300, -1e300, f64::MAX];
        for k in -2000i64..2000 {
            for v in [k as f64 / 8.0, k as f64 / 100.0, (2 * k + 1) as f64 / 200.0] {
                values.extend([
                    v,
                    f64::from_bits(v.to_bits() + 1),
                    f64::from_bits(v.to_bits().wrapping_sub(1)),
                ]);
            }
        }
        for _ in 0..200_000 {
            let bits = next();
            let unit = (bits >> 11) as f64 / (1u64 << 53) as f64;
            values.push((unit - 0.5) * 40_000.0);
            values.push((unit - 0.5) * 2.0);
            let raw = f64::from_bits(next());
            if raw.is_finite() {
                values.push(raw);
            }
        }
        for x in values.into_iter().filter(|x| x.is_finite()) {
            assert_eq!(fixed2(x), fixed2_exact(x), "{x:e} ({:#x})", x.to_bits());
        }
    }

    #[test]
    fn colour_order_tags_groups() {
        let shape = |colour: [u8; 4]| ExportShape {
            colour,
            parent: -1,
            nodes: Vec::new(),
            pieces: Vec::new(),
        };
        let shapes = [
            shape([1, 2, 3, 255]),
            shape([9, 9, 9, 255]),
            shape([1, 2, 3, 255]),
            shape([1, 2, 3, 255]),
        ];
        assert_eq!(colour_order(&shapes), vec![(0, 0), (2, 3), (3, 1), (1, 2)]);
    }
}
