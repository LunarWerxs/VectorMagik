//! DXF, AutoCAD's drawing exchange format, written from the app's SVG
//! documents (September 24, 2026) in the original desktop's three ways:
//! "Lines and spline curves" (its default), "Lines only, convert splines to
//! more lines (larger file)" and "Lines only, convert splines to fewer lines
//! (smaller file)".
//!
//! The drawing is read by `pdf_eps::page`, so what the PDF writer refuses is
//! refused here too. Coordinates are the page's points with y up (the SVG's
//! y flipped about the page height); DXF has no unit for points, so the
//! header says unitless (`$INSUNITS` 0 where the version has it) and the
//! extents are the page. Every colour is one layer named `COLOR_RRGGBB`
//! with the nearest AutoCAD colour index, and every entity carries that
//! index (group 62) and, in the spline mode's R2000 file, its exact colour
//! (group 420; R12 has no such code, and Illustrator's AutoCAD import
//! refuses an R12 file that carries one). Every subpath is one
//! outline, closed when the SVG closes it or fills it (a fill closes every
//! subpath), open otherwise. Opacity has no place in these entities and is
//! not written.
//!
//! The versions. The line modes write R12 (AC1009): each outline a
//! `POLYLINE` with its `VERTEX`es and `SEQEND`, the oldest and most widely
//! read form, which needs no handles or object tree. `SPLINE` does not exist
//! before R13, so the spline mode writes R2000 (AC1015), the oldest version
//! the common readers take splines from, with what a strict reader requires
//! of it: a header with `$ACADVER` and `$HANDSEED`, an empty `CLASSES`
//! section, the nine symbol tables with their standard entries (linetypes
//! ByBlock, ByLayer and Continuous, layer 0, text and dimension style
//! Standard, application ACAD, the model and paper space block records),
//! every layer's lineweight (default) and plot style (group 390, the
//! `Normal` placeholder of the `ACAD_PLOTSTYLENAME` dictionary: without it
//! Illustrator's AutoCAD import refuses the file, measured September 24,
//! 2026), the two space blocks, a handle and an owner on every object, and
//! the `OBJECTS` section's root dictionary with its `ACAD_GROUP` and
//! `ACAD_PLOTSTYLENAME` dictionaries.
//! There each run of cubic pieces is one `SPLINE` (degree 3, the Bezier
//! pieces' own control points under a knot vector with multiplicity 3 at
//! interior joins and 4 at the ends) and each run of straight pieces an
//! `LWPOLYLINE`, end to end along the outline; an outline of straight
//! pieces alone is one closed or open `LWPOLYLINE`.
//!
//! Dithered areas (September 25, 2026). Where a filled path's squares repeat
//! a tile (`dither.rs`, as the SVG, PDF and EPS find them), the tile is a
//! block of its inked squares and the area AutoCAD's rectangular array of
//! it: one `INSERT` with column and row counts ("MINSERT"). A rectangle ends
//! where the repetition stops, not on a tile's edge, so the columns and rows
//! it ends in are arrays of the part of the tile they hold. Exploded, the
//! arrays are the same closed square outlines the path holds, in both
//! versions (R12 has the counts too). A DXF has no fills, so a hatch, whose
//! pattern is lines clipped to a boundary, would draw the squares' edges
//! only as loose lines, and those on the boundary itself as the reader
//! decides; the arrays keep every square an outline.
use crate::dither::Run;
use crate::export::DxfMode;
use crate::pdf_eps::{Page, Segment, Xy};
use std::collections::HashMap;
use std::fmt::Write as _;

/// The flattening tolerances of the line modes, in the DXF's own units,
/// points: "more lines" keeps every chord within 0.05 pt (0.018 mm) of its
/// curve, "fewer lines" within 0.5 pt (0.18 mm).
const FINE_TOLERANCE: f64 = 0.05;
const COARSE_TOLERANCE: f64 = 0.5;
/// The most chords one cubic is cut into: at 0.05 pt it takes a curve about
/// a kilometre across to need more, so only a hostile file reaches it.
const MAX_STEPS: f64 = 4096.;
/// The most vertices a line-mode file holds (about 600 MB of DXF).
const MAX_VERTICES: usize = 10_000_000;

/// The DXF of `svg`, its curves written as `mode` says.
pub fn to_dxf(svg: &str, mode: DxfMode) -> Result<Vec<u8>, String> {
    let page = crate::pdf_eps::page(svg)?;
    let drawing = Drawing::of(&page);
    let text = match mode {
        DxfMode::Splines => r2000(&page, &drawing),
        DxfMode::FineLines => r12(&page, &drawing, FINE_TOLERANCE)?,
        DxfMode::CoarseLines => r12(&page, &drawing, COARSE_TOLERANCE)?,
    };
    Ok(text.into_bytes())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Piece {
    Line(Xy),
    Cubic(Xy, Xy, Xy),
}

impl Piece {
    fn end(self) -> Xy {
        match self {
            Piece::Line(p) | Piece::Cubic(_, _, p) => p,
        }
    }
}

/// A subpath in DXF coordinates: where it starts and its pieces.
struct Outline {
    /// Its colour's index in the layer list.
    layer: usize,
    start: Xy,
    pieces: Vec<Piece>,
    closed: bool,
}

/// What a file draws: a layer per colour, the outlines and arrays in paint
/// order, and the blocks the arrays place.
struct Drawing {
    layers: Vec<Layer>,
    items: Vec<Item>,
    blocks: Vec<Block>,
}

enum Item {
    Outline(Outline),
    Array(Array),
}

/// Part of a dithered area's tile: the squares of its inked cells as closed
/// outlines, its lower-left corner at the block's origin.
struct Block {
    name: String,
    outlines: Vec<Outline>,
}

/// A block placed as AutoCAD's rectangular array: `columns` by `rows`
/// copies `step` apart, the lower-left one at `at`.
struct Array {
    layer: usize,
    block: usize,
    at: Xy,
    columns: usize,
    rows: usize,
    step: Xy,
}

impl Drawing {
    /// The page's colours in order of first use and its outlines, y up; a
    /// filled path's dithered areas as arrays of their tile. Only outlines
    /// are written, so a path's stroke changes nothing here.
    fn of(page: &Page) -> Self {
        let mut colours = Vec::new();
        let mut layer_of: HashMap<[u8; 3], usize> = HashMap::new();
        let mut items = Vec::new();
        let mut tiles = Tiles::default();
        for shape in &page.shapes {
            let Some(colour) = shape.fill.or(shape.stroke) else {
                continue;
            };
            let layer = *layer_of.entry(colour).or_insert_with(|| {
                colours.push(colour);
                colours.len() - 1
            });
            let filled = shape.fill.is_some();
            let split = filled
                .then(|| crate::dither::split(&shape.segments))
                .flatten();
            let segments = split.as_ref().map_or(&shape.segments[..], |s| &s.0[..]);
            items.extend(
                outlines(segments, layer, filled, page.height)
                    .into_iter()
                    .map(Item::Outline),
            );
            for run in split.iter().flat_map(|s| &s.1) {
                tiles.place(run, layer, page.height, &mut items);
            }
        }
        Self {
            layers: colours.into_iter().map(Layer::of).collect(),
            items,
            blocks: tiles.blocks,
        }
    }
}

/// The outlines of one path's `segments` on `layer`, y up on a page
/// `height` high.
fn outlines(segments: &[Segment], layer: usize, filled: bool, height: f64) -> Vec<Outline> {
    let flip = |p: Xy| (p.0, height - p.1);
    let fresh = |start: Xy| Outline {
        layer,
        start,
        pieces: Vec::new(),
        closed: filled,
    };
    let mut out = Vec::new();
    let mut current: Option<Outline> = None;
    // Where a piece without a move before it starts: after a close, the
    // subpath's start, as in SVG.
    let mut at = flip((0., 0.));
    for segment in segments {
        let piece = match *segment {
            Segment::Move(p) => {
                out.extend(current.take().filter(|o| !o.pieces.is_empty()));
                at = flip(p);
                current = Some(fresh(at));
                continue;
            }
            Segment::Close => {
                if let Some(mut outline) = current.take() {
                    outline.closed = true;
                    at = outline.start;
                    out.extend(Some(outline).filter(|o| !o.pieces.is_empty()));
                }
                continue;
            }
            Segment::Line(p) => Piece::Line(flip(p)),
            Segment::Cubic(a, b, p) => Piece::Cubic(flip(a), flip(b), flip(p)),
        };
        current.get_or_insert_with(|| fresh(at)).pieces.push(piece);
        at = piece.end();
    }
    out.extend(current.filter(|o| !o.pieces.is_empty()));
    out
}

/// A block's layer, square side (its bits) and inked cells, each as its
/// column and its row counted from the bottom.
type TileKey = (usize, u64, Vec<(usize, usize)>);

/// The blocks made so far, each made once for what it draws.
#[derive(Default)]
struct Tiles {
    blocks: Vec<Block>,
    known: HashMap<TileKey, usize>,
}

impl Tiles {
    /// `run` (page space, y down) as arrays on `layer`: its whole tiles,
    /// then the columns and the rows it ends in short of a whole tile, and
    /// the corner where both meet, each an array of the part of the tile it
    /// holds.
    fn place(&mut self, run: &Run, layer: usize, height: f64, items: &mut Vec<Item>) {
        let side = run.side;
        let (px, py) = run.period;
        let cells = |length: f64| (length / side).round() as usize;
        let (w, h) = (cells(run.width), cells(run.height));
        let (nx, ny) = (w / px, h / py);
        let (rx, ry) = (w % px, h % py);
        for (window, counts, offset) in [
            ((px, py), (nx, ny), (0, 0)),
            ((rx, py), (1, ny), (nx * px, 0)),
            ((px, ry), (nx, 1), (0, ny * py)),
            ((rx, ry), (1, 1), (nx * px, ny * py)),
        ] {
            // Row j of the tile, counted from its top, is row
            // window.1 - 1 - j of the block, counted from its bottom.
            let mut inked: Vec<(usize, usize)> = run
                .inked
                .iter()
                .filter(|&&(i, j)| i < window.0 && j < window.1)
                .map(|&(i, j)| (i, window.1 - 1 - j))
                .collect();
            if inked.is_empty() || counts.0 == 0 || counts.1 == 0 {
                continue;
            }
            inked.sort_unstable();
            let block = self.block((layer, side.to_bits(), inked));
            let bottom = run.y + (offset.1 + counts.1 * window.1) as f64 * side;
            items.push(Item::Array(Array {
                layer,
                block,
                at: (run.x + offset.0 as f64 * side, height - bottom),
                columns: counts.0,
                rows: counts.1,
                step: (window.0 as f64 * side, window.1 as f64 * side),
            }));
        }
    }

    /// The block that draws `key`'s squares, made when it is new.
    fn block(&mut self, key: TileKey) -> usize {
        if let Some(&index) = self.known.get(&key) {
            return index;
        }
        let (layer, side) = (key.0, f64::from_bits(key.1));
        let outlines = key
            .2
            .iter()
            .map(|&(i, j)| {
                let (left, right) = (i as f64 * side, (i + 1) as f64 * side);
                let (bottom, top) = (j as f64 * side, (j + 1) as f64 * side);
                Outline {
                    layer,
                    start: (left, top),
                    pieces: vec![
                        Piece::Line((right, top)),
                        Piece::Line((right, bottom)),
                        Piece::Line((left, bottom)),
                    ],
                    closed: true,
                }
            })
            .collect();
        self.blocks.push(Block {
            name: format!("VM_DITHER_{}", self.blocks.len() + 1),
            outlines,
        });
        self.known.insert(key, self.blocks.len() - 1);
        self.blocks.len() - 1
    }
}

/// A colour's layer: its name, nearest colour index and true colour.
struct Layer {
    name: String,
    index: u8,
    colour: u32,
}

impl Layer {
    fn of(c: [u8; 3]) -> Self {
        Self {
            name: format!("COLOR_{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
            index: nearest_index(c),
            // Group 420: red, green and blue in one integer.
            colour: u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]),
        }
    }
}

/// A DXF being written: group code and value pairs, CR LF lines, the codes
/// right-aligned in three places as AutoCAD writes them.
#[derive(Default)]
struct Dxf {
    text: String,
}

impl Dxf {
    fn pair(&mut self, code: u16, value: impl std::fmt::Display) {
        let _ = write!(self.text, "{code:>3}\r\n{value}\r\n");
    }

    /// A point's x and y under `code` and `code + 10`.
    fn xy(&mut self, code: u16, p: Xy) {
        self.pair(code, num(p.0));
        self.pair(code + 10, num(p.1));
    }

    /// A point with z 0.
    fn xyz(&mut self, code: u16, p: Xy) {
        self.xy(code, p);
        self.pair(code + 20, "0.0");
    }

    /// The layer and colours every entity carries: the exact colour too
    /// when the version has a code for it.
    fn paint(&mut self, layer: &Layer, true_colour: bool) {
        self.pair(8, &layer.name);
        self.pair(62, layer.index);
        if true_colour {
            self.pair(420, layer.colour);
        }
    }

    /// An `INSERT`'s block, place and array after its head; a count of one
    /// and its spacing are left to their defaults.
    fn array(&mut self, array: &Array, blocks: &[Block]) {
        self.pair(2, &blocks[array.block].name);
        self.xyz(10, array.at);
        if array.columns > 1 {
            self.pair(70, array.columns);
        }
        if array.rows > 1 {
            self.pair(71, array.rows);
        }
        if array.columns > 1 {
            self.pair(44, num(array.step.0));
        }
        if array.rows > 1 {
            self.pair(45, num(array.step.1));
        }
    }
}

/// Layer 0 and the colours' layers, as names and colour indexes.
fn layer_entries(layers: &[Layer]) -> impl Iterator<Item = (&str, u8)> {
    std::iter::once(("0", 7)).chain(layers.iter().map(|l| (l.name.as_str(), l.index)))
}

/// A coordinate with at most six decimals, no trailing zeros.
fn num(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".to_owned(),
        _ => text.to_owned(),
    }
}

/// The colour AutoCAD shows for index 1 to 255: seven named colours and two
/// greys, then 24 hues 15 degrees apart in ten shades each (value 100%, 80%,
/// 60%, 50% and 30%, each also half-way to white first), then six greys.
/// Read back from Illustrator's AutoCAD import, one line per index, all 255
/// alike (September 24, 2026).
fn index_colour(index: u8) -> [u8; 3] {
    match index {
        0 | 7 => [255, 255, 255],
        1 => [255, 0, 0],
        2 => [255, 255, 0],
        3 => [0, 255, 0],
        4 => [0, 255, 255],
        5 => [0, 0, 255],
        6 => [255, 0, 255],
        8 => [128, 128, 128],
        9 => [192, 192, 192],
        10..=249 => {
            let hue = f64::from((index - 10) / 10) * 15.;
            let shade = (index - 10) % 10;
            let sector = (hue / 60.).floor();
            let f = hue / 60. - sector;
            let (r, g, b) = match sector as u8 {
                0 => (1., f, 0.),
                1 => (1. - f, 1., 0.),
                2 => (0., 1., f),
                3 => (0., 1. - f, 1.),
                4 => (f, 0., 1.),
                _ => (1., 0., 1. - f),
            };
            let value = [1., 0.8, 0.6, 0.5, 0.3][usize::from(shade / 2)];
            let channel = |c: f64| {
                let c = 255. * c;
                let c = if shade % 2 == 1 {
                    c + (255. - c) / 2.
                } else {
                    c
                };
                (c * value) as u8
            };
            [channel(r), channel(g), channel(b)]
        }
        250..=255 => {
            let grey = [51, 91, 132, 173, 214, 255][usize::from(index - 250)];
            [grey, grey, grey]
        }
    }
}

/// The AutoCAD colour index nearest `c`; index 7 draws black on a light
/// background and white on a dark one, so it stands for both.
fn nearest_index(c: [u8; 3]) -> u8 {
    let distance = |a: [u8; 3]| {
        (0..3)
            .map(|i| (i32::from(a[i]) - i32::from(c[i])).pow(2))
            .sum::<i32>()
    };
    (1..=255u8)
        .min_by_key(|&i| match i {
            7 => distance([0, 0, 0]).min(distance([255, 255, 255])),
            _ => distance(index_colour(i)),
        })
        .unwrap_or(7)
}

/// How many equal steps of t keep a cubic's chords within `tolerance` of
/// it: Wang's formula, n = ceil(sqrt(3 * 2 / 8 * L / tolerance)), L the
/// larger second difference of the control points. The bound holds for
/// every cubic, so the flattening is within the tolerance everywhere, and
/// short or flat pieces get few steps.
fn steps(p0: Xy, p1: Xy, p2: Xy, p3: Xy, tolerance: f64) -> Result<usize, String> {
    let second = |a: Xy, b: Xy, c: Xy| (a.0 - 2. * b.0 + c.0).hypot(a.1 - 2. * b.1 + c.1);
    let bend = second(p0, p1, p2).max(second(p1, p2, p3));
    let n = (0.75 * bend / tolerance).sqrt().ceil();
    if n.is_nan() || n > MAX_STEPS {
        return Err("The drawing is too large to write as lines".into());
    }
    Ok((n as usize).max(1))
}

fn cubic_at(p0: Xy, p1: Xy, p2: Xy, p3: Xy, t: f64) -> Xy {
    let u = 1. - t;
    let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// Whether two points are the same to well under a written decimal.
fn same(a: Xy, b: Xy) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
}

/// The outline as polyline vertices within `tolerance` of its curves; a
/// closed outline's last vertex is dropped when it repeats the first.
/// `budget` is how many vertices the file may still take.
fn flattened(outline: &Outline, tolerance: f64, budget: &mut usize) -> Result<Vec<Xy>, String> {
    let mut take = |n: usize| -> Result<(), String> {
        *budget = budget.checked_sub(n).ok_or(TOO_MANY_VERTICES)?;
        Ok(())
    };
    take(1)?;
    let mut points = vec![outline.start];
    let mut at = outline.start;
    for piece in &outline.pieces {
        match *piece {
            Piece::Line(p) => {
                take(1)?;
                points.push(p);
            }
            Piece::Cubic(a, b, p) => {
                let n = steps(at, a, b, p, tolerance)?;
                take(n)?;
                points.extend((1..n).map(|i| cubic_at(at, a, b, p, i as f64 / n as f64)));
                points.push(p);
            }
        }
        at = piece.end();
    }
    if outline.closed && points.len() > 1 && same(points[0], at) {
        points.pop();
    }
    Ok(points)
}

const TOO_MANY_VERTICES: &str = "The drawing has too many curve points to write as lines";

/// The header's extents: the page, y up.
fn extents(dxf: &mut Dxf, page: &Page) {
    dxf.pair(9, "$INSBASE");
    dxf.xyz(10, (0., 0.));
    dxf.pair(9, "$EXTMIN");
    dxf.xyz(10, (0., 0.));
    dxf.pair(9, "$EXTMAX");
    dxf.xyz(10, (page.width, page.height));
    dxf.pair(9, "$LIMMIN");
    dxf.xy(10, (0., 0.));
    dxf.pair(9, "$LIMMAX");
    dxf.xy(10, (page.width, page.height));
}

/// One outline as an R12 `POLYLINE` within `tolerance` of its curves.
fn polyline12(
    dxf: &mut Dxf,
    layer: &Layer,
    outline: &Outline,
    tolerance: f64,
    budget: &mut usize,
) -> Result<(), String> {
    let points = flattened(outline, tolerance, budget)?;
    if points.len() < 2 {
        return Ok(());
    }
    dxf.pair(0, "POLYLINE");
    dxf.paint(layer, false);
    dxf.pair(66, 1);
    dxf.xyz(10, (0., 0.));
    dxf.pair(70, u8::from(outline.closed));
    for p in points {
        dxf.pair(0, "VERTEX");
        dxf.pair(8, &layer.name);
        dxf.xyz(10, p);
    }
    dxf.pair(0, "SEQEND");
    dxf.pair(8, &layer.name);
    Ok(())
}

/// The R12 file of the line modes.
fn r12(page: &Page, drawing: &Drawing, tolerance: f64) -> Result<String, String> {
    let layers = &drawing.layers;
    let mut dxf = Dxf::default();
    dxf.pair(0, "SECTION");
    dxf.pair(2, "HEADER");
    dxf.pair(9, "$ACADVER");
    dxf.pair(1, "AC1009");
    extents(&mut dxf, page);
    dxf.pair(0, "ENDSEC");
    dxf.pair(0, "SECTION");
    dxf.pair(2, "TABLES");
    dxf.pair(0, "TABLE");
    dxf.pair(2, "LTYPE");
    dxf.pair(70, 1);
    dxf.pair(0, "LTYPE");
    dxf.pair(2, "CONTINUOUS");
    dxf.pair(70, 0);
    dxf.pair(3, "Solid line");
    dxf.pair(72, 65);
    dxf.pair(73, 0);
    dxf.pair(40, "0.0");
    dxf.pair(0, "ENDTAB");
    dxf.pair(0, "TABLE");
    dxf.pair(2, "LAYER");
    dxf.pair(70, layers.len() + 1);
    for (name, index) in layer_entries(layers) {
        dxf.pair(0, "LAYER");
        dxf.pair(2, name);
        dxf.pair(70, 0);
        dxf.pair(62, index);
        dxf.pair(6, "CONTINUOUS");
    }
    dxf.pair(0, "ENDTAB");
    dxf.pair(0, "ENDSEC");
    let mut budget = MAX_VERTICES;
    dxf.pair(0, "SECTION");
    dxf.pair(2, "BLOCKS");
    for block in &drawing.blocks {
        dxf.pair(0, "BLOCK");
        dxf.pair(8, "0");
        dxf.pair(2, &block.name);
        dxf.pair(70, 0);
        dxf.xyz(10, (0., 0.));
        dxf.pair(3, &block.name);
        for outline in &block.outlines {
            polyline12(
                &mut dxf,
                &layers[outline.layer],
                outline,
                tolerance,
                &mut budget,
            )?;
        }
        dxf.pair(0, "ENDBLK");
        dxf.pair(8, "0");
    }
    dxf.pair(0, "ENDSEC");
    dxf.pair(0, "SECTION");
    dxf.pair(2, "ENTITIES");
    for item in &drawing.items {
        match item {
            Item::Outline(outline) => polyline12(
                &mut dxf,
                &layers[outline.layer],
                outline,
                tolerance,
                &mut budget,
            )?,
            Item::Array(array) => {
                dxf.pair(0, "INSERT");
                dxf.paint(&layers[array.layer], false);
                dxf.array(array, &drawing.blocks);
            }
        }
    }
    dxf.pair(0, "ENDSEC");
    dxf.pair(0, "EOF");
    Ok(dxf.text)
}

/// Handles in the order objects are made, as hexadecimal text.
struct Handles(u32);

impl Handles {
    fn next(&mut self) -> String {
        self.0 += 1;
        format!("{:X}", self.0)
    }
}

/// A symbol table's head: its name, handle and entry count.
fn table(dxf: &mut Dxf, name: &str, handle: &str, entries: usize) {
    dxf.pair(0, "TABLE");
    dxf.pair(2, name);
    dxf.pair(5, handle);
    dxf.pair(330, "0");
    dxf.pair(100, "AcDbSymbolTable");
    dxf.pair(70, entries);
}

/// A symbol table entry's head up to its name.
fn entry(dxf: &mut Dxf, kind: &str, handle: &str, owner: &str, class: &str, name: &str) {
    dxf.pair(0, kind);
    dxf.pair(if kind == "DIMSTYLE" { 105 } else { 5 }, handle);
    dxf.pair(330, owner);
    dxf.pair(100, "AcDbSymbolTableRecord");
    dxf.pair(100, class);
    dxf.pair(2, name);
    dxf.pair(70, 0);
}

/// The R2000 file of the spline mode.
fn r2000(page: &Page, drawing: &Drawing) -> String {
    let layers = &drawing.layers;
    let mut h = Handles(0);
    let (vport, ltype, layer, style, view) = (h.next(), h.next(), h.next(), h.next(), h.next());
    let (ucs, appid, dimstyle, records) = (h.next(), h.next(), h.next(), h.next());
    let (plot_styles, normal) = (h.next(), h.next());
    let mut dxf = Dxf::default();
    dxf.pair(0, "SECTION");
    dxf.pair(2, "CLASSES");
    dxf.pair(0, "ENDSEC");
    dxf.pair(0, "SECTION");
    dxf.pair(2, "TABLES");
    table(&mut dxf, "VPORT", &vport, 0);
    dxf.pair(0, "ENDTAB");
    table(&mut dxf, "LTYPE", &ltype, 3);
    for name in ["ByBlock", "ByLayer", "Continuous"] {
        let class = "AcDbLinetypeTableRecord";
        entry(&mut dxf, "LTYPE", &h.next(), &ltype, class, name);
        dxf.pair(
            3,
            if name == "Continuous" {
                "Solid line"
            } else {
                ""
            },
        );
        dxf.pair(72, 65);
        dxf.pair(73, 0);
        dxf.pair(40, "0.0");
    }
    dxf.pair(0, "ENDTAB");
    table(&mut dxf, "LAYER", &layer, layers.len() + 1);
    for (name, index) in layer_entries(layers) {
        let class = "AcDbLayerTableRecord";
        entry(&mut dxf, "LAYER", &h.next(), &layer, class, name);
        dxf.pair(62, index);
        dxf.pair(6, "Continuous");
        dxf.pair(370, -3);
        dxf.pair(390, &normal);
    }
    dxf.pair(0, "ENDTAB");
    table(&mut dxf, "STYLE", &style, 1);
    let class = "AcDbTextStyleTableRecord";
    entry(&mut dxf, "STYLE", &h.next(), &style, class, "Standard");
    for (code, value) in [
        (40, "0.0"),
        (41, "1.0"),
        (50, "0.0"),
        (71, "0"),
        (42, "2.5"),
    ] {
        dxf.pair(code, value);
    }
    dxf.pair(3, "txt");
    dxf.pair(4, "");
    dxf.pair(0, "ENDTAB");
    table(&mut dxf, "VIEW", &view, 0);
    dxf.pair(0, "ENDTAB");
    table(&mut dxf, "UCS", &ucs, 0);
    dxf.pair(0, "ENDTAB");
    table(&mut dxf, "APPID", &appid, 1);
    let class = "AcDbRegAppTableRecord";
    entry(&mut dxf, "APPID", &h.next(), &appid, class, "ACAD");
    dxf.pair(0, "ENDTAB");
    let standard = h.next();
    table(&mut dxf, "DIMSTYLE", &dimstyle, 1);
    dxf.pair(100, "AcDbDimStyleTable");
    dxf.pair(71, 1);
    dxf.pair(340, &standard);
    let class = "AcDbDimStyleTableRecord";
    entry(
        &mut dxf, "DIMSTYLE", &standard, &dimstyle, class, "Standard",
    );
    dxf.pair(0, "ENDTAB");
    let (model, paper) = (h.next(), h.next());
    let spaces = [(model.clone(), "*Model_Space"), (paper, "*Paper_Space")];
    let tiles: Vec<(String, &str)> = drawing
        .blocks
        .iter()
        .map(|b| (h.next(), b.name.as_str()))
        .collect();
    table(&mut dxf, "BLOCK_RECORD", &records, 2 + tiles.len());
    for (handle, name) in spaces.iter().chain(&tiles) {
        dxf.pair(0, "BLOCK_RECORD");
        dxf.pair(5, handle);
        dxf.pair(330, &records);
        dxf.pair(100, "AcDbSymbolTableRecord");
        dxf.pair(100, "AcDbBlockTableRecord");
        dxf.pair(2, name);
    }
    dxf.pair(0, "ENDTAB");
    dxf.pair(0, "ENDSEC");
    dxf.pair(0, "SECTION");
    dxf.pair(2, "BLOCKS");
    for (k, (owner, name)) in spaces.iter().chain(&tiles).enumerate() {
        let paper_space = *name == "*Paper_Space";
        dxf.pair(0, "BLOCK");
        dxf.pair(5, h.next());
        dxf.pair(330, owner);
        dxf.pair(100, "AcDbEntity");
        if paper_space {
            dxf.pair(67, 1);
        }
        dxf.pair(8, "0");
        dxf.pair(100, "AcDbBlockBegin");
        dxf.pair(2, name);
        dxf.pair(70, 0);
        dxf.xyz(10, (0., 0.));
        dxf.pair(3, name);
        dxf.pair(1, "");
        for outline in k
            .checked_sub(spaces.len())
            .map_or(&[][..], |tile| &drawing.blocks[tile].outlines)
        {
            spline_outline(&mut dxf, &mut h, owner, &layers[outline.layer], outline);
        }
        dxf.pair(0, "ENDBLK");
        dxf.pair(5, h.next());
        dxf.pair(330, owner);
        dxf.pair(100, "AcDbEntity");
        if paper_space {
            dxf.pair(67, 1);
        }
        dxf.pair(8, "0");
        dxf.pair(100, "AcDbBlockEnd");
    }
    dxf.pair(0, "ENDSEC");
    dxf.pair(0, "SECTION");
    dxf.pair(2, "ENTITIES");
    for item in &drawing.items {
        match item {
            Item::Outline(outline) => {
                spline_outline(&mut dxf, &mut h, &model, &layers[outline.layer], outline);
            }
            Item::Array(array) => {
                // AutoCAD's subclass for an insert with more than one copy.
                let class = if array.columns > 1 || array.rows > 1 {
                    "AcDbMInsertBlock"
                } else {
                    "AcDbBlockReference"
                };
                let layer = &layers[array.layer];
                entity_head(&mut dxf, &mut h, "INSERT", &model, layer, class);
                dxf.array(array, &drawing.blocks);
            }
        }
    }
    dxf.pair(0, "ENDSEC");
    let (root, groups) = (h.next(), h.next());
    dxf.pair(0, "SECTION");
    dxf.pair(2, "OBJECTS");
    dxf.pair(0, "DICTIONARY");
    dxf.pair(5, &root);
    dxf.pair(330, "0");
    dxf.pair(100, "AcDbDictionary");
    dxf.pair(281, 1);
    dxf.pair(3, "ACAD_GROUP");
    dxf.pair(350, &groups);
    dxf.pair(3, "ACAD_PLOTSTYLENAME");
    dxf.pair(350, &plot_styles);
    dxf.pair(0, "DICTIONARY");
    dxf.pair(5, &groups);
    dxf.pair(330, &root);
    dxf.pair(100, "AcDbDictionary");
    dxf.pair(281, 1);
    dxf.pair(0, "ACDBDICTIONARYWDFLT");
    dxf.pair(5, &plot_styles);
    dxf.pair(330, &root);
    dxf.pair(100, "AcDbDictionary");
    dxf.pair(281, 1);
    dxf.pair(3, "Normal");
    dxf.pair(350, &normal);
    dxf.pair(100, "AcDbDictionaryWithDefault");
    dxf.pair(340, &normal);
    dxf.pair(0, "ACDBPLACEHOLDER");
    dxf.pair(5, &normal);
    dxf.pair(330, &plot_styles);
    dxf.pair(0, "ENDSEC");
    dxf.pair(0, "EOF");

    let mut head = Dxf::default();
    head.pair(0, "SECTION");
    head.pair(2, "HEADER");
    head.pair(9, "$ACADVER");
    head.pair(1, "AC1015");
    head.pair(9, "$HANDSEED");
    head.pair(5, h.next());
    head.pair(9, "$INSUNITS");
    head.pair(70, 0);
    extents(&mut head, page);
    head.pair(0, "ENDSEC");
    head.text + &dxf.text
}

/// An R2000 entity's head: its kind, handle, owner, paint and class.
fn entity_head(
    dxf: &mut Dxf,
    h: &mut Handles,
    kind: &str,
    owner: &str,
    layer: &Layer,
    class: &str,
) {
    dxf.pair(0, kind);
    dxf.pair(5, h.next());
    dxf.pair(330, owner);
    dxf.pair(100, "AcDbEntity");
    dxf.paint(layer, true);
    dxf.pair(100, class);
}

/// One outline as the spline mode writes it: an `LWPOLYLINE` alone when it
/// is straight throughout, else runs of cubics as `SPLINE`s and runs of
/// lines as open `LWPOLYLINE`s, end to end, a closed outline's closing edge
/// included.
fn spline_outline(dxf: &mut Dxf, h: &mut Handles, owner: &str, layer: &Layer, outline: &Outline) {
    let mut pieces = outline.pieces.clone();
    let end = pieces.last().map_or(outline.start, |p| p.end());
    let head = |dxf: &mut Dxf, h: &mut Handles, kind: &str, class: &str| {
        entity_head(dxf, h, kind, owner, layer, class);
    };
    let polyline = |dxf: &mut Dxf, h: &mut Handles, points: &[Xy], closed: bool| {
        head(dxf, h, "LWPOLYLINE", "AcDbPolyline");
        dxf.pair(90, points.len());
        dxf.pair(70, u8::from(closed));
        dxf.pair(43, "0.0");
        for p in points {
            dxf.xy(10, *p);
        }
    };
    if pieces.iter().all(|p| matches!(p, Piece::Line(_))) {
        let mut points: Vec<Xy> = std::iter::once(outline.start)
            .chain(pieces.iter().map(|p| p.end()))
            .collect();
        if outline.closed && points.len() > 2 && same(points[0], end) {
            points.pop();
        }
        polyline(dxf, h, &points, outline.closed);
        return;
    }
    if outline.closed && !same(end, outline.start) {
        pieces.push(Piece::Line(outline.start));
    }
    let mut at = outline.start;
    let mut rest = &pieces[..];
    while let Some(first) = rest.first() {
        let cubic = matches!(first, Piece::Cubic(..));
        let run = rest
            .iter()
            .position(|p| matches!(p, Piece::Cubic(..)) != cubic)
            .unwrap_or(rest.len());
        let (these, after) = rest.split_at(run);
        if cubic {
            head(dxf, h, "SPLINE", "AcDbSpline");
            dxf.pair(210, "0.0");
            dxf.pair(220, "0.0");
            dxf.pair(230, "1.0");
            // Planar, not rational, not periodic.
            dxf.pair(70, 8);
            dxf.pair(71, 3);
            dxf.pair(72, 3 * these.len() + 5);
            dxf.pair(73, 3 * these.len() + 1);
            dxf.pair(74, 0);
            dxf.pair(42, "0.0000001");
            dxf.pair(43, "0.0000001");
            for knot in 0..=these.len() {
                let repeats = if knot == 0 || knot == these.len() {
                    4
                } else {
                    3
                };
                for _ in 0..repeats {
                    dxf.pair(40, format!("{knot}.0"));
                }
            }
            dxf.xyz(10, at);
            for piece in these {
                if let Piece::Cubic(a, b, p) = *piece {
                    dxf.xyz(10, a);
                    dxf.xyz(10, b);
                    dxf.xyz(10, p);
                }
            }
        } else {
            let points: Vec<Xy> = std::iter::once(at)
                .chain(these.iter().map(|p| p.end()))
                .collect();
            polyline(dxf, h, &points, false);
        }
        at = these.last().map_or(at, |p| p.end());
        rest = after;
    }
}

#[cfg(test)]
mod tests;
