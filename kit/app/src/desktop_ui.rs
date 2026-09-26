//! The desktop window: a lean workspace (Glass, light or dark: `look.rs`)
//! with a settings rail of cards,
//! two comparison cards that pick the orientation giving the largest picture,
//! synchronized zoom and pan, crisp re-rendering of the vector at any zoom,
//! live curve simplification with an automatic tolerance, drag-and-drop onto
//! the source card, a per-node "round this corner" tool (click for a tight
//! rounding, right-click for the reach), nodes dragged to a new place and
//! put back, Undo and Redo of every edit and setting, converting again by
//! itself when a conversion setting changes, a hold-to-compare overlay, a
//! footer with clickable result stats and the zoom controls, an output
//! size, and a Save popup that asks for the format first and lets the file
//! be dragged straight onto the desktop. Rail cards slide open and shut,
//! and a card whose switch is off shows only the switch. Conversions,
//! derivations, tiles, system dialogs and the PDF/EPS exporter all run off
//! the UI thread, so the window keeps painting; a running conversion can be
//! cancelled. The same code renders the headless PNG snapshots in
//! `snapshot.rs`.
use crate::engine::{Document as VectorDocument, Options as VectorizeOptions, Sliders};
use crate::{Preparation, Recolor};
use egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Key, KeyboardShortcut, Margin,
    Modifiers, RichText, Stroke, StrokeKind, TextStyle, Vec2,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use vector_rebuild::clock::Stopwatch;
use vector_rebuild::geometry::Point;
use vector_rebuild::nodes::{NodeMove, NodePiece};
use vector_rebuild::raster::Raster;
use vector_rebuild::regularize::RegularizeOptions;
use vector_rebuild::shapes::{self, Island, Removal};
use vector_rebuild::simplify::Rounding;
use vector_rebuild::sticker::Sticker;
use vector_rebuild::straighten::StraightenOptions;
use vector_rebuild::{AdvancedSettings, ImageCategory, Quality};

// The window's colours come from the palette in force (`look.rs`); these
// few are drawn over the picture itself and stay the same light or dark.
const NODE: Color32 = Color32::from_rgb(64, 205, 255);
const NODE_SMOOTH: Color32 = Color32::from_rgb(140, 230, 120);
const SHAPE_HOVER: Color32 = Color32::from_rgb(255, 255, 255);
const SHAPE_SELECTED: Color32 = Color32::from_rgb(255, 184, 76);
/// Under a node marker's outline, so it reads on white as well as on black.
const NODE_SHADOW: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 120);
/// Half the side of a node marker's square, the radius of its circle.
const NODE_MARKER_RADIUS: f32 = 3.5;

/// A window narrower than this is a phone's: the rail becomes a sheet
/// behind the toolbar's settings button, the view chips go and Convert is
/// its icon (SUE's phone visitor, September 25, 2026: held upright, the app
/// was cut off on the right).
const NARROW_WIDTH: f32 = 760.;

/// Whether the window is phone-narrow (`NARROW_WIDTH`).
fn narrow_window(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < NARROW_WIDTH
}

/// A picture to try the app on without one of one's own: the SVG format's
/// logo (free for any use; kit/fixtures/samples/LICENSE.txt). Every visitor
/// SUE sent on September 25, 2026 left with nothing traced, having no image
/// to hand.
const SAMPLE_PNG: &[u8] = include_bytes!("../../fixtures/samples/logo-with-blending.png");

/// How far zoom goes, as display scales (screen pixels per source pixel):
/// out to a quarter of fit or of 1:1, whichever is smaller, and in to 32
/// screen pixels per source pixel or eight times fit, whichever is larger.
/// Limits relative to fit alone put 1:1 out of reach for a phone photo (fit
/// about 0.1) and for a small icon (fit above 4).
const ZOOM_OUT: f32 = 0.25;
const ZOOM_IN_PIXELS: f32 = 32.;
const ZOOM_IN_FIT: f32 = 8.;
const ZOOM_STEP: f32 = 1.25;
/// Source pixels a simplified curve may stray from the engine's fit.
pub const DEFAULT_SIMPLIFY_TOLERANCE: f32 = 0.5;
const SIMPLIFY_RANGE: std::ops::RangeInclusive<f32> = 0.05..=3.;
/// How far a curve may bow from its chord and still be drawn as a line:
/// where the slider sits until Auto (the default) takes the bow of the
/// picture's kind from its first conversion
/// (`vector_rebuild::straighten::auto_flatness`).
pub const DEFAULT_STRAIGHTEN_TOLERANCE: f32 = 0.8;
/// The Curves card's straightening bow for a snapshot: the picture kind's
/// own (Auto, as the desktop starts) or this many source pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bow {
    Auto,
    Pixels(f64),
}
/// The Curves card's true lines and circles: how far a run may stray from
/// the line or circle that replaces it, in source pixels.
pub const DEFAULT_REGULARIZE_BAND: f64 = 0.8;
/// The Curves card's true shapes: the closed outlines the pixels show to be
/// circles, ellipses, rectangles and rounded rectangles drawn as those
/// shapes (`VectorDocument::refitted`); on, picked by
/// kit/tools/pick_variant.py on September 23, 2026
/// (testing/quality-round/primitives.md).
pub const DEFAULT_PRIMITIVES: bool = true;
const STRAIGHTEN_RANGE: std::ops::RangeInclusive<f32> = 0.1..=2.;

/// How the two pictures are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    SideBySide,
    /// One picture; B shows the bitmap, V the vector.
    Overlay,
}
/// Screen pixels within which a click picks a curve node.
const NODE_PICK_RADIUS: f32 = 8.;
/// Output sizes offered as one click; anything else is typed as a width.
const OUTPUT_SCALES: [f32; 4] = [1., 2., 4., 8.];
const OUTPUT_WIDTH_RANGE: std::ops::RangeInclusive<u32> = 16..=16384;
/// How far (levels per channel) a traced fill may sit from a merge's colour
/// and still be written as that colour.
const MERGE_SNAP: u8 = 6;
/// Colour limits offered in the Conversion card.
const COLOR_CHOICES: [usize; 13] = [1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 24, 32, 64];
/// Longest side of one crisp tile, in screen pixels.
const MAX_TILE_SIDE: u32 = 4096;

const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SC_CONVERT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SC_PREVIEW: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const SC_CLOSE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
const SC_UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const SC_REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const SC_REDO_SHIFT: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
/// How many edits Undo remembers.
const UNDO_LIMIT: usize = 200;
/// How long settings must rest (and the pointer be up) before Convert
/// automatically runs again, so a slider dragged or a list scrolled through
/// converts once at the end, not at every step.
const AUTO_CONVERT_DELAY: Duration = Duration::from_millis(400);
/// How long the pointer must rest on the rail's scroll bar before it widens
/// and takes clicks, so passing over it on the way to the rail's resize
/// handle next to it never grabs it.
const RAIL_BAR_DELAY: Duration = Duration::from_millis(350);

/// Icons come from the emoji fonts egui bundles, so they render offline and in
/// the headless snapshot. Plain code points only: a variation selector would
/// draw as a missing-glyph box. `tests::every_icon_has_a_glyph` guards the list.
pub mod icon {
    pub const OPEN: &str = "\u{1F5C1}";
    pub const CONVERT: &str = "\u{25B6}";
    pub const SAVE: &str = "\u{1F4BE}";
    pub const CLOSE: &str = "\u{1F5D9}";
    pub const SETTINGS: &str = "\u{2699}";
    // A trigram in egui's icon font on every system (U+2637 is in no font the
    // app loads, Segoe UI included, and drew as a box).
    pub const SLIDERS: &str = "\u{2630}";
    pub const NODES: &str = "\u{25A3}";
    pub const SOURCE: &str = "\u{1F5BC}";
    pub const VECTOR: &str = "\u{1F58A}";
    pub const INFO: &str = "\u{2139}";
    pub const DONE: &str = "\u{2714}";
    pub const ERROR: &str = "\u{26A0}";
    pub const SIZE: &str = "\u{1F4CF}";
    pub const COLORS: &str = "\u{1F3A8}";
    pub const SEGMENTS: &str = "\u{2731}";
    pub const FILE: &str = "\u{1F4C4}";
    pub const SHAPES: &str = "\u{1F4A0}";
    pub const STICKER: &str = "\u{2B23}";
    pub const APPEARANCE: &str = "\u{1F313}";
    pub const ALL: [&str; 19] = [
        OPEN, CONVERT, SAVE, CLOSE, SETTINGS, SLIDERS, NODES, SOURCE, VECTOR, INFO, DONE, ERROR,
        SIZE, COLORS, SEGMENTS, FILE, SHAPES, STICKER, APPEARANCE,
    ];
}

/// The window and taskbar mark, also drawn beside the title.
/// The logo, as a file so other tools (the Launchpad's icon builder) can
/// copy the same artwork.
const LOGO_SVG: &str = include_str!("../assets/logo.svg");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatusKind {
    Info,
    Busy,
    Done,
    Error,
}

/// How much of each neighbouring piece a rounded node may reshape. Chosen
/// per node: a click rounds tightly, the node's menu offers the others.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reach {
    Tiny,
    Tight,
    Medium,
    Wide,
}
impl Reach {
    pub const ALL: [Reach; 4] = [Reach::Tiny, Reach::Tight, Reach::Medium, Reach::Wide];
    pub fn fraction(self) -> f64 {
        match self {
            Reach::Tiny => 0.1,
            Reach::Tight => 0.25,
            Reach::Medium => 0.5,
            Reach::Wide => 1.,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Reach::Tiny => "Tiny",
            Reach::Tight => "Tight",
            Reach::Medium => "Medium",
            Reach::Wide => "Wide",
        }
    }
    pub fn parse(text: &str) -> Option<Reach> {
        match text.trim().to_ascii_lowercase().as_str() {
            "tiny" => Some(Reach::Tiny),
            "tight" => Some(Reach::Tight),
            "medium" => Some(Reach::Medium),
            "wide" => Some(Reach::Wide),
            _ => None,
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Reach::Tiny => {
                "Reshapes only the tenth of each neighboring piece nearest the node: \
                 the corner just loses its point."
            }
            Reach::Tight => "Reshapes only the quarter of each neighboring piece nearest the node.",
            Reach::Medium => "Reshapes the half of each neighboring piece nearest the node.",
            Reach::Wide => "Reshapes both neighboring pieces entirely.",
        }
    }
}

/// A node the user rounded, with the reach chosen for it.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Rounded {
    at: Point,
    reach: Reach,
}

/// A node marker being dragged: where the node was when the drag began,
/// where the pointer has taken it, and the pieces it pulls along, drawn
/// over the picture until the drop re-derives the document. A rounded
/// corner pulls the pieces it had before rounding, drawn rounded again with
/// its reach (the fraction) in the document's view box.
#[derive(Clone)]
struct NodeDrag {
    from: Point,
    to: Point,
    pieces: Vec<NodePiece>,
    rounding: Option<(f64, (f64, f64))>,
}

/// Everything Undo and Redo put back: the hand edits of nodes and shapes,
/// the merges, and every card's settings. Taken at the end of each frame in
/// which the pointer is up, so a slider dragged is one step.
#[derive(Clone, Debug, PartialEq)]
struct Edits {
    rounded: Vec<Rounded>,
    straightened: Vec<Point>,
    moved: Vec<NodeMove>,
    deleted_nodes: Vec<Point>,
    deleted: Vec<Removal>,
    prep: Preparation,
    conversion: ConversionSettings,
    simplify: bool,
    simplify_tolerance: f32,
    regularize: bool,
    primitives: bool,
    straighten: bool,
    straighten_tolerance: f32,
    straighten_auto: bool,
    sticker_on: bool,
    sticker: Sticker,
}

/// The Conversion and Advanced cards' settings, apart from the preparation:
/// what a conversion is run with.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ConversionSettings {
    automatic: bool,
    /// The chosen image type and quality; ignored under Auto settings, so
    /// left out then.
    manual: Option<(ImageCategory, Quality)>,
    overlap_opaque_photos: bool,
    optional_optimizer: bool,
    /// The Advanced card's sliders when it is on.
    sliders: Option<Sliders>,
}

/// What a conversion was started with: the preparation and the settings.
/// Convert automatically runs again when these differ from the ones the
/// shown (or running) conversion was started with.
#[derive(Clone, Debug, PartialEq)]
struct ConversionInputs {
    prep: Preparation,
    settings: ConversionSettings,
}
impl Rounded {
    fn rounding(self) -> Rounding {
        Rounding {
            at: self.at,
            reach: self.reach.fraction(),
        }
    }
}
fn same_point(a: &Point, b: &Point) -> bool {
    a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits()
}
/// The same shape reference: colour and the point it was picked at.
fn same_shape(a: &Removal, b: &Removal) -> bool {
    a.color.eq_ignore_ascii_case(&b.color) && same_point(&a.at, &b.at)
}
/// The island a shape reference stands for now, if it still exists.
fn find_shape(islands: &[Island], shape: &Removal) -> Option<usize> {
    shapes::island_at(islands, shape.at)
        .filter(|&i| islands[i].color.eq_ignore_ascii_case(&shape.color))
}

/// The file formats Save offers; chosen before any dialog opens. The
/// original offered EPS, SVG, PDF, AI, EMF and DXF; PNG is the drawing as
/// pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Svg,
    Pdf,
    Eps,
    Ai,
    Dxf,
    Emf,
    Png,
}
impl Format {
    pub const ALL: [Format; 7] = [
        Format::Svg,
        Format::Pdf,
        Format::Eps,
        Format::Ai,
        Format::Dxf,
        Format::Emf,
        Format::Png,
    ];
    fn label(self) -> &'static str {
        match self {
            Format::Svg => "SVG",
            Format::Pdf => "PDF",
            Format::Eps => "EPS",
            Format::Ai => "AI",
            Format::Dxf => "DXF",
            Format::Emf => "EMF",
            Format::Png => "PNG",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Pdf => "pdf",
            Format::Eps => "eps",
            Format::Ai => "ai",
            Format::Dxf => "dxf",
            Format::Emf => "emf",
            Format::Png => "png",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Format::Svg => "The curves exactly as shown. Opens in every editor and browser.",
            Format::Pdf => "One vector page, transparency kept. Made by the local exporter.",
            Format::Eps => {
                "PostScript on white. Partial transparency is refused rather than rasterized."
            }
            Format::Ai => {
                "Adobe Illustrator: a PDF-compatible file every current Illustrator opens."
            }
            Format::Dxf => {
                "AutoCAD and cutting machines: the outlines, as curves or as lines (below)."
            }
            Format::Emf => {
                "Windows metafile, for Office and other Windows programs. No partial transparency."
            }
            Format::Png => "The drawing as pixels, at the size above, on transparency.",
        }
    }
    /// The position in the save dialog's filter list.
    fn filter_index(self) -> u32 {
        Format::ALL
            .iter()
            .position(|f| *f == self)
            .map_or(1, |i| i as u32 + 1)
    }
    fn from_filter_index(index: u32) -> Format {
        (index as usize)
            .checked_sub(1)
            .and_then(|i| Format::ALL.get(i).copied())
            .unwrap_or(Format::Svg)
    }
    fn kind(self) -> crate::export::OutputKind {
        match self {
            Format::Svg => crate::export::OutputKind::Svg,
            Format::Pdf => crate::export::OutputKind::Pdf,
            Format::Eps => crate::export::OutputKind::Eps,
            Format::Ai => crate::export::OutputKind::Ai,
            Format::Dxf => crate::export::OutputKind::Dxf,
            Format::Emf => crate::export::OutputKind::Emf,
            Format::Png => crate::export::OutputKind::Png,
        }
    }
    fn mime(self) -> &'static str {
        match self {
            Format::Svg => "image/svg+xml",
            Format::Pdf => "application/pdf",
            Format::Eps => "application/postscript",
            Format::Ai => "application/illustrator",
            Format::Dxf => "image/vnd.dxf",
            Format::Emf => "image/emf",
            Format::Png => "image/png",
        }
    }
}

/// What the shape menu asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShapeAction {
    Delete,
    ToggleSelect,
    MergeInto,
}

/// Which footer stat has its popup open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatPopup {
    Size,
    Colors,
}

/// What a headless snapshot may open on top of the workspace, so the popups
/// can be inspected without a window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    Save,
    Size,
    Colors,
    NodeMenu,
    /// Shape editing on, the shape at the centre selected and its menu open.
    Shapes,
    /// The first-run question: personal or commercial use.
    Licence,
    /// The Appearance popup: light, dark or the system's.
    Appearance,
}

/// How the person said they use VectorMagik when the app first asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenceUse {
    Personal,
    Commercial,
    /// For work, free for `TRIAL_SECONDS` from this Unix time: a business
    /// tries the app without claiming it is for personal use (the owner,
    /// September 24, 2026: "I'd rather give them a chance").
    Trial(i64),
}

/// How long the free commercial trial runs: a day.
pub const TRIAL_SECONDS: i64 = 24 * 60 * 60;

impl LicenceUse {
    fn word(self) -> String {
        match self {
            LicenceUse::Personal => "personal".into(),
            LicenceUse::Commercial => "commercial".into(),
            LicenceUse::Trial(start) => format!("trial:{start}"),
        }
    }
    fn from_word(word: &str) -> Option<Self> {
        match word.trim() {
            "personal" => Some(LicenceUse::Personal),
            "commercial" => Some(LicenceUse::Commercial),
            other => other
                .strip_prefix("trial:")
                .and_then(|start| start.parse().ok())
                .map(LicenceUse::Trial),
        }
    }
    /// The seconds of a trial left at `now` (0 once it has ended); None when
    /// this is no trial.
    pub fn trial_left(self, now: i64) -> Option<i64> {
        match self {
            LicenceUse::Trial(start) => Some((start + TRIAL_SECONDS - now).clamp(0, TRIAL_SECONDS)),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Actions {
    open: bool,
    convert: bool,
    cancel: bool,
    save: bool,
    preview: bool,
    close: bool,
    undo: bool,
    redo: bool,
}

/// A system dialog or a file being written, off the UI thread so the window
/// keeps painting while it is out; the toolbar waits for it.
struct Errand {
    kind: ErrandKind,
    receiver: Receiver<Fetched>,
    /// The status line put back when a dialog is cancelled.
    before: (String, StatusKind),
}
enum ErrandKind {
    Open,
    Save,
    Preview,
    Write(PathBuf),
}
/// What an errand brought back: the file a dialog picked with its filter
/// index (`None` when cancelled), or how writing a file went.
enum Fetched {
    Picked(Option<(String, u32)>),
    Written(Result<(), String>),
}

/// The file the Save popup offers for dragging out, written off the UI
/// thread for one document version, format and saved size.
struct Staged {
    key: StageKey,
    path: PathBuf,
    /// Answers once the file is written.
    writing: Option<Receiver<Result<(), String>>>,
    error: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Debug)]
struct StageKey {
    version: u64,
    format: Format,
    size: (u32, u32),
    stacked: bool,
    options: crate::export::ExportOptions,
}
/// Whether the staged file can be dragged out yet.
#[cfg_attr(
    not(feature = "desktop"),
    allow(
        dead_code,
        reason = "the browser downloads the file instead of dragging it"
    )
)]
enum StageState {
    Writing,
    Ready(PathBuf),
    Failed(String),
}

/// A colour to drop before a conversion, worked out on its thread: the
/// colour, the shown document's shapes and the image the engine last saw.
struct ColourDrop {
    hex: String,
    islands: Arc<Vec<Island>>,
    working: Arc<Raster>,
}

/// One crisp rendering of part of the vector at a display scale (`scale`
/// points per picture pixel), in the display's own pixels: `ppp` of them per
/// point, so a display scaled to 150% or 200% gets a sharp picture rather
/// than one stretched from a texture of one pixel per point. `size` is in
/// those pixels.
struct TileRequest {
    version: u64,
    svg: Arc<String>,
    document_width: f32,
    scale: f32,
    ppp: f32,
    origin: [f32; 2],
    size: [u32; 2],
}
struct TileResult {
    version: u64,
    scale: f32,
    ppp: f32,
    origin: [f32; 2],
    image: egui::ColorImage,
}
#[derive(Clone, Copy, PartialEq)]
struct TileKey {
    version: u64,
    scale: f32,
    ppp: f32,
    origin: [f32; 2],
    size: [u32; 2],
}
struct Tile {
    key: TileKey,
    texture: egui::TextureHandle,
}

/// Re-deriving the shown document off the UI thread, so the tolerance slider
/// updates live and Auto can try several tolerances.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DeriveJob {
    /// Simplify with the settings' tolerance, if any.
    Derive,
    /// Pick the tolerance too.
    Auto,
}
/// What a conversion cleared from the view, kept until its result arrives
/// so that Cancel can put it back. Until then the vector card goes on
/// drawing it, nodes and all (not clickable), so converting again swaps the
/// picture in place instead of flashing the bitmap in between (the owner,
/// September 23, 2026: "it can't hide the nodes and the vector and then
/// reshow").
struct Replaced {
    raw_document: Option<Arc<VectorDocument>>,
    raw_nodes: usize,
    document: Option<VectorDocument>,
    shown: Option<Arc<String>>,
    shown_margin: f32,
    node_counts: Option<(usize, usize)>,
    palette: Vec<(String, usize)>,
    preview: Option<egui::TextureHandle>,
    working_source: Option<egui::TextureHandle>,
    working_sharp: Option<egui::TextureHandle>,
    elapsed: Option<f64>,
    /// What the replaced result was converted with.
    inputs: Option<ConversionInputs>,
    /// Its nodes (when they were shown) and sharp tile, for that drawing.
    nodes: Option<Arc<Vec<Point>>>,
    tile: Option<Tile>,
}
/// A count as the engine gave it and as shown after the passes.
type Counts = Option<(usize, usize)>;
/// The error a cancelled conversion ends with; nobody shows it.
const CANCELLED: &str = "Conversion cancelled.";
/// Everything the shown document is derived with from the engine's output:
/// the Curves, Nodes, Shapes and Sticker cards, none of which converts again.
#[derive(Clone, Debug, PartialEq)]
struct DeriveSettings {
    simplify: Option<f64>,
    /// The tolerance is the slider's, not Auto's pick: the kinks the merges
    /// keep are smoothed too (`VectorDocument::simplified_by_hand`).
    simplify_by_hand: bool,
    deleted: Vec<Removal>,
    regularize: Option<RegularizeOptions>,
    primitives: bool,
    straighten: Option<StraightenOptions>,
    straightened: Vec<Point>,
    moved: Vec<NodeMove>,
    deleted_nodes: Vec<Point>,
    rounded: Vec<Rounding>,
    sticker: Option<Sticker>,
}
struct DeriveRequest {
    serial: u64,
    raw_version: u64,
    raw: Arc<VectorDocument>,
    settings: DeriveSettings,
    job: DeriveJob,
}
struct DeriveResult {
    serial: u64,
    raw_version: u64,
    auto: bool,
    outcome: Result<(Option<f64>, Derived), String>,
}

/// The Sticker card for a snapshot: off, on with the widths the desktop picks
/// for the picture, or on with these settings.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StickerChoice {
    #[default]
    Off,
    Sized,
    Custom(Sticker),
}

/// The shown document as it is painted and saved: the sticker (if any) under
/// the document's shapes, and the rendered preview of that. The document
/// itself stays the editing surface for nodes and shapes.
#[derive(Clone)]
struct Presented {
    svg: Arc<String>,
    image: egui::ColorImage,
    /// How far the sticker reaches past the document on every side, in
    /// source pixels; 0 without one.
    margin: f32,
}
impl Presented {
    /// The drawing of a document as painted and saved: stacked (a strip of
    /// each colour under the edges later ones share, so no background line
    /// shows between two colours; `vector_rebuild::stacking`) unless cut
    /// out, then the sticker's layers under it.
    fn drawing(
        document: &VectorDocument,
        sticker: Option<&Sticker>,
        stacked: bool,
    ) -> Result<String, String> {
        let planar = if stacked {
            vector_rebuild::stacking::stack_svg(document.svg())?.0
        } else {
            document.svg().to_owned()
        };
        match sticker {
            Some(sticker) => vector_rebuild::sticker::apply(&planar, sticker),
            None => Ok(planar),
        }
    }
    fn of(document: &VectorDocument, sticker: Option<&Sticker>) -> Result<Self, String> {
        let svg = Self::drawing(document, sticker, true)?;
        let image = crate::preview(&svg)?;
        Ok(Self {
            svg: Arc::new(svg),
            image,
            margin: sticker.map_or(0., |s| s.margin() as f32),
        })
    }
}

/// A shown document with how it is painted and what the footer and cards
/// count in it, all made off the UI thread: its nodes (for the markers),
/// segments and fills, most used first.
struct Derived {
    document: VectorDocument,
    presented: Presented,
    nodes: Arc<Vec<Point>>,
    palette: Vec<(String, usize)>,
}
impl Derived {
    fn new(document: VectorDocument, presented: Presented) -> Self {
        Self {
            nodes: Arc::new(document.nodes()),
            palette: document.colors(),
            document,
            presented,
        }
    }
}

/// The image the engine saw and whether it is opaque, what Auto detected in
/// it, the engine's own document with its node and segment counts, the
/// document as shown (simplified, straightened, with the user's shape and
/// corner edits) and as painted, the tolerance Auto settings picked for the
/// simplification if it did, the preparation and derive settings it was
/// made with, and a dropped colour with the pixels that changed for it.
struct Converted {
    working: Raster,
    working_opaque: bool,
    detected: Option<crate::auto::Detection>,
    raw: VectorDocument,
    raw_nodes: usize,
    derived: Derived,
    auto_tolerance: Option<f64>,
    prep: Preparation,
    derived_with: DeriveSettings,
    dropped: Option<(String, usize)>,
}
type JobResult = Result<Converted, String>;
pub struct Desktop {
    path: String,
    loaded_path: String,
    options: VectorizeOptions,
    automatic: bool,
    detected: Option<crate::auto::Detection>,
    raster: Option<Raster>,
    /// The image the engine was given: `raster` after the preparation. Shown
    /// in the source card once converted.
    working: Option<Arc<Raster>>,
    /// Whether every pixel of `working` is opaque, as the photo seam
    /// treatment asks.
    working_opaque: bool,
    has_alpha: bool,
    /// A picture named on the command line, loaded on the first frame once
    /// the GPU's texture limit is known.
    load_on_first_frame: bool,
    /// Colour limit, background and merges applied before the engine.
    prep: Preparation,
    /// The preparation the current result was made with.
    converted_prep: Option<Preparation>,
    background_custom: [u8; 3],
    /// Shapes the user removed, each by its colour and a point inside it.
    deleted: Vec<Removal>,
    shapes_mode: bool,
    delete_on_click: bool,
    selected: Vec<Removal>,
    /// The shown document's shapes, for hit tests; keyed by document version.
    islands: Option<(u64, Arc<Vec<Island>>)>,
    /// The shown document's nodes, for the markers; keyed by document version.
    nodes_cache: Option<(u64, Arc<Vec<Point>>)>,
    /// The right-clicked shape and where its menu opened.
    shape_menu: Option<(Removal, egui::Pos2)>,
    /// The engine's output before any simplification.
    raw_document: Option<Arc<VectorDocument>>,
    /// Bumped per conversion or close, so late derivations are discarded.
    raw_version: u64,
    /// What is edited: the raw document, simplified and with the rounded
    /// nodes applied.
    document: Option<VectorDocument>,
    /// What is painted and saved: `document` with the sticker under it.
    shown: Option<Arc<String>>,
    /// The margin of the sticker `shown` was painted with, so the picture,
    /// the nodes over it and the saved size agree while a Sticker change is
    /// still being derived.
    shown_margin: f32,
    /// Bumped whenever `document` or `shown` changes, so stale tiles are
    /// discarded.
    document_version: u64,
    /// The Sticker card: an outline cut around the shapes, applied last.
    sticker_on: bool,
    sticker: Sticker,
    /// The last custom colour of the border and of the rim.
    sticker_custom: [[u8; 3]; 2],
    /// Whether the image the engine saw has an opaque border, so there is a
    /// background shape the Sticker card can cut out.
    border_opaque: bool,
    /// The engine's document's nodes, counted once per conversion on its
    /// thread.
    raw_nodes: usize,
    /// (engine nodes, shown nodes) for the Curves card.
    node_counts: Option<(usize, usize)>,
    /// The shown fills, kept with the document so the footer and card
    /// headings never parse it per frame.
    palette: Vec<(String, usize)>,
    /// Nodes the user rounded, each with its own reach; applied on top of
    /// simplification.
    rounded: Vec<Rounded>,
    /// Nodes the user dragged elsewhere, applied after the Curves card's
    /// passes and before the rounding (`Desktop::finish`).
    moved: Vec<NodeMove>,
    /// Nodes the user deleted from their right-click menu, keyed where the
    /// moves left them: applied after the moves and before the rounding.
    deleted_nodes: Vec<Point>,
    /// The node marker being dragged, if one is.
    node_drag: Option<NodeDrag>,
    /// The right-clicked node and where its menu opened.
    node_menu: Option<(Point, egui::Pos2)>,
    /// Whether the node with the menu open may be deleted, worked out once
    /// per node and document version: the reason it may not, or `None`.
    deletable: Option<(u64, Point, Option<&'static str>)>,
    /// Undo and Redo: the edits before each change, the ones undone, and the
    /// edits as last recorded.
    undo: Vec<Edits>,
    redo: Vec<Edits>,
    edits_seen: Option<Edits>,
    /// Convert again by itself when a conversion setting changes (after the
    /// picture has been converted once).
    auto_convert: bool,
    /// What the shown or running conversion was started with; `None` until
    /// the picture is converted.
    converted_inputs: Option<ConversionInputs>,
    /// Settings that differ from `converted_inputs`, and since when they
    /// have been as they are: Convert automatically waits for them to rest.
    inputs_changed: Option<(ConversionInputs, Stopwatch)>,
    /// The settings of a conversion the user cancelled: Convert
    /// automatically leaves them alone until they change.
    held_inputs: Option<ConversionInputs>,
    /// Overlay view: the Bitmap and Vector buttons (and B and V) show the
    /// other picture only while held, instead of switching to it.
    hold_compare: bool,
    /// The picture shown while a button or key is held under `hold_compare`
    /// (`Some(true)` the vector), and the button held on the last frame.
    peek: Option<bool>,
    peek_button: Option<bool>,
    /// Since when (egui's clock, seconds) the pointer has rested on the
    /// rail's scroll bar.
    rail_bar_since: Option<f64>,
    /// Where the window's preferences (Hold, Convert automatically) are
    /// kept; `None` for snapshots and tests, which never read or write them.
    prefs: Option<PathBuf>,
    /// The preferences as last written, so they are written when changed.
    prefs_saved: (bool, bool, bool),
    /// The commercial licence (`crate::licence`): the key and its latest
    /// certificate, kept with the preferences, and as last saved.
    licence: crate::licence::Stored,
    licence_saved: crate::licence::Stored,
    /// What the Licence card's key field holds.
    licence_input: String,
    /// A redeem on its way: the reply, the key presented, and whether it was
    /// typed (a refusal then leaves the stored licence alone) or the stored
    /// key renewing its certificate (a refusal then ends it).
    licence_reply: Option<(Receiver<platform::Reply>, String, bool)>,
    /// The card's last word about the licence: a refusal, no connection.
    licence_note: Option<String>,
    /// Whether this run has offered the stored key for renewal yet.
    licence_renewed: bool,
    /// The answer to the first-run question, once given.
    licence_use: Option<LicenceUse>,
    /// Whether the first-run question is on screen (`ask_licence_if_new`).
    ask_licence: bool,
    /// The question's key field is open: commercial use was chosen.
    prompt_key: bool,
    /// The checkout was opened from the question, which then says so.
    licence_opened: bool,
    /// The design and the light or dark the person chose.
    theme: ThemeChoice,
    /// Light or dark and the first-run answer as last written with the
    /// preferences.
    appearance_saved: (ThemeChoice, Option<LicenceUse>),
    /// Whether the style was last built dark.
    applied: Option<bool>,
    appearance_open: bool,
    /// In a phone-narrow window the rail is a sheet over the whole window,
    /// shown while this is set (the toolbar's settings button).
    rail_open: bool,
    /// Where the Appearance button was drawn, so its popup hangs below it.
    appearance_anchor: egui::Rect,
    /// A vector file just opened, waiting for "trace it or convert it"
    /// (`vector_ui.rs`).
    vector_offer: Option<vector_ui::VectorOffer>,
    /// A vector file converted as it is, shown and saved in place of a
    /// traced document.
    foreign: Option<vector_ui::Foreign>,
    deriver: Option<(Sender<DeriveRequest>, Receiver<DeriveResult>)>,
    derive_serial: u64,
    /// The serial of the latest request not yet answered, if any.
    derive_pending: Option<u64>,
    auto_pending: bool,
    source: Option<egui::TextureHandle>,
    /// The source again, unfiltered, for magnified inspection.
    source_sharp: Option<egui::TextureHandle>,
    /// The image the engine saw (after the preparation), shown in the source
    /// card on request.
    working_source: Option<egui::TextureHandle>,
    working_sharp: Option<egui::TextureHandle>,
    show_engine_input: bool,
    /// Side by side, or one picture switched between bitmap and vector.
    view: View,
    overlay_vector: bool,
    /// Which rail cards are folded to their heading.
    collapsed: [bool; 7],
    /// The Advanced card: the original's three sliders and its corner
    /// detection instead of the preset, applied at the next conversion.
    advanced_on: bool,
    sliders: Sliders,
    /// The Curves card's true lines and circles: a run of pieces along one
    /// line becomes that line, a run on one circle its arcs.
    regularize: bool,
    /// The Curves card's true shapes: a closed outline the source's pixels
    /// show to be a circle, an ellipse, a rectangle or a rounded rectangle
    /// becomes that shape.
    primitives: bool,
    /// The Curves card's straightening: flat curves become lines, lines near
    /// an axis snap to it; `straightened` nodes get both regardless.
    straighten: bool,
    straighten_tolerance: f32,
    /// The bow is the picture kind's own until the slider moves.
    straighten_auto: bool,
    straightened: Vec<Point>,
    preview: Option<egui::TextureHandle>,
    tile: Option<Tile>,
    tile_pending: Option<TileKey>,
    /// The tile the view last wanted and since when, while it rests before
    /// asking (`platform::TILE_REST`).
    tile_wanted: Option<(TileKey, Stopwatch)>,
    tiler: Option<(Sender<TileRequest>, Receiver<TileResult>)>,
    logo: Option<egui::TextureHandle>,
    worker: Option<Receiver<JobResult>>,
    /// A cancelled conversion still running its current engine stage to the
    /// end: Convert waits for it, so conversions never pile up.
    stopping: Option<Receiver<JobResult>>,
    /// Set by Cancel: the running conversion skips its remaining stages.
    stop: Arc<AtomicBool>,
    /// The result a running conversion replaced, put back by Cancel.
    replaced: Option<Box<Replaced>>,
    /// A dialog or a file being written; the toolbar waits for it.
    errand: Option<Errand>,
    /// The Save popup's file to drag out.
    staged: Option<Staged>,
    status: String,
    status_kind: StatusKind,
    started: Stopwatch,
    elapsed: Option<f64>,
    zoom: f32,
    fit: f32,
    /// The body a picture card would have at its full size this frame: what
    /// the fit is measured against, so a card that hugs a small picture does
    /// not shrink its own fit.
    fit_area: Option<Vec2>,
    scroll: Vec2,
    nodes: bool,
    simplify: bool,
    simplify_tolerance: f32,
    /// Auto's last pick exactly: the slider holds it as an f32, and 0.3 is
    /// not one (round three of the Opus 5.5 review), so while the slider
    /// still shows it the derive uses this.
    simplify_pick: Option<f64>,
    /// The saved file's size relative to the source; 1 keeps the engine's
    /// declaration byte for byte.
    output_scale: f32,
    /// The size of the picture opened when it was scaled down for the engine.
    source_size: Option<(usize, usize)>,
    save_format: Format,
    /// Save the stacked drawing (the default, as shown) or the shapes cut
    /// out exactly, for cutting machines.
    save_stacked: bool,
    /// Cut-out shapes grouped by colour (the original's default) or not.
    save_grouped: bool,
    /// The original's "Stroke shape boundaries".
    save_stroke: bool,
    /// How a DXF writes curves.
    save_dxf: crate::export::DxfMode,
    save_open: bool,
    /// Where the Save button was drawn, so its popup hangs below it.
    save_anchor: egui::Rect,
    stat_popup: Option<StatPopup>,
    /// Where the Size and Colors stats were drawn this frame.
    stat_anchors: [egui::Rect; 2],
    snapshot_path: Option<PathBuf>,
    title_family: FontFamily,

    #[cfg(feature = "ui-test")]
    frames: u32,
}
mod cards;
mod chrome;
mod edits;
mod headless;
mod licence_ui;
mod look;
pub mod platform;
mod prefs;
mod state;
#[cfg(test)]
mod tests;
mod vector_ui;
mod widgets;
mod workspace;

pub use look::ThemeChoice;
use look::{faded, pal, SHADOW_REACH};
use widgets::*;

#[cfg(feature = "desktop")]
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
    /// A system dialog still open closes with the window.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        close_open_dialog();
    }
}

#[cfg(feature = "desktop")]
pub fn run() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("VectorMagik")
        .with_inner_size([1280., 860.])
        .with_min_inner_size([760., 540.]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "VectorMagik",
        options,
        Box::new(|cc| Ok(Box::new(Desktop::new(cc)))),
    )
}
