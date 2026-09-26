//! Part of `desktop_ui`: how the window looks. Two looks after Apple's current
//! design, asked for on September 24, 2026 ("make it feel modern, something
//! Apple would make"), each light or dark: Studio, the opaque
//! sidebar-and-toolbar layout of a Mac pro app and the default, and Glass,
//! whose bars and cards float as translucent panes over the window, after
//! Liquid Glass. The owner, September 25, 2026: Studio the default, Classic
//! (the teal-on-charcoal design the app shipped with) gone, and Glass in plain
//! greys ("the whole glass one feels very AI" over its purple and blue light).
//! Every colour the window paints comes from the palette in force; the UI
//! thread sets it once a frame (`set`), so a switch shows on the next frame.

use egui::{Color32, Shadow};
use std::cell::Cell;

/// The design of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Studio,
    Glass,
}

impl Look {
    pub const ALL: [Look; 2] = [Look::Studio, Look::Glass];
    pub fn label(self) -> &'static str {
        match self {
            Look::Glass => "Glass",
            Look::Studio => "Studio",
        }
    }
    pub fn about(self) -> &'static str {
        match self {
            Look::Glass => {
                "Bars and cards float as frosted glass panes over the window, after \
                 Apple's Liquid Glass."
            }
            Look::Studio => {
                "A quiet sidebar and toolbar in system grey and blue, like a Mac pro app."
            }
        }
    }
    /// The word kept in the preferences.
    pub fn word(self) -> &'static str {
        match self {
            Look::Glass => "glass",
            Look::Studio => "studio",
        }
    }
    pub fn from_word(word: &str) -> Option<Look> {
        Look::ALL
            .into_iter()
            .find(|look| look.word() == word.trim())
    }
}

/// Light or dark: chosen, or the system's (the page's switch in a tab).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    Dark,
    Light,
    System,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::Dark, ThemeChoice::Light, ThemeChoice::System];
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
            ThemeChoice::System => "System",
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
            ThemeChoice::System => "system",
        }
    }
    pub fn from_word(word: &str) -> Option<ThemeChoice> {
        ThemeChoice::ALL
            .into_iter()
            .find(|theme| theme.word() == word.trim())
    }
}

/// A patch of coloured light behind the Glass look's panes: its centre as a
/// fraction of the window, its reach as a fraction of the window's longer
/// side, and its colour, whose alpha is how strongly it tints.
pub(super) struct Glow {
    pub x: f32,
    pub y: f32,
    pub reach: f32,
    pub color: Color32,
}

/// Every colour and shape the window's design decides.
pub(super) struct Palette {
    pub look: Look,
    pub dark: bool,
    /// Behind everything: the canvas area.
    pub backdrop: Color32,
    /// The header, status bar and (Studio) sidebar.
    pub panel: Color32,
    /// Cards.
    pub surface: Color32,
    pub surface_high: Color32,
    pub border: Color32,
    /// A resting control's edge.
    pub edge: Color32,
    pub chip: Color32,
    pub field: Color32,
    pub text: Color32,
    /// Text of controls that do not react.
    pub label: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub checker_light: Color32,
    pub checker_dark: Color32,
    pub hover: Color32,
    pub hover_edge: Color32,
    /// An open combo box or menu button.
    pub open: Color32,
    /// A switch's knob while it is off.
    pub knob_off: Color32,
    pub popup: Color32,
    /// Laid over the window behind a dialog, and over a card a file is
    /// dragged onto.
    pub scrim: Color32,
    pub card_radius: u8,
    pub control_radius: u8,
    /// The rim of a card; transparent for none.
    pub card_stroke: Color32,
    pub card_shadow: Shadow,
    pub popup_shadow: Shadow,
    /// The header and status bar float as rounded panes over the backdrop,
    /// and the rail and canvas show it through (Glass).
    pub floating: bool,
    /// Hairlines part the bars and the sidebar from the canvas (Studio).
    pub separators: bool,
    /// Toolbar icons carry a button's frame at rest (off: only on hover).
    pub framed_icons: bool,
    /// Card headings, in the title face.
    pub heading: Color32,
    pub glows: &'static [Glow],
}

/// `r`, `g`, `b` at opacity `a`, premultiplied as egui keeps colours.
const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    const fn mul(c: u8, a: u8) -> u8 {
        ((c as u16 * a as u16 + 127) / 255) as u8
    }
    Color32::from_rgba_premultiplied(mul(r, a), mul(g, a), mul(b, a), a)
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const fn shadow(y: i8, blur: u8, alpha: u8) -> Shadow {
    Shadow {
        offset: [0, y],
        blur,
        spread: 0,
        color: rgba(0, 0, 0, alpha),
    }
}

/// The Glass cards' shadow: it reaches `SHADOW_REACH` px past a card's sides
/// (half its blur), three above and nine below, which every margin around a
/// card leaves room for. The first Glass drew 28 px of blur 10 px down, and
/// the rail and the bars cut it off at the sides (the owner, September 25,
/// 2026).
const GLASS_SHADOW_DARK: Shadow = shadow(3, 12, 120);
const GLASS_SHADOW_LIGHT: Shadow = shadow(3, 12, 30);
/// How far past a card's sides any look's card shadow reaches.
pub(super) const SHADOW_REACH: i8 = 6;

const NO_SHADOW: Shadow = Shadow {
    offset: [0, 0],
    blur: 0,
    spread: 0,
    color: Color32::TRANSPARENT,
};

// Apple's system colours (label, secondary and tertiary label, system blue,
// green, orange and red) in their light and dark forms. Everything outside
// the pictures in Studio dark stays darker than 60/255, which the snapshot
// test samples.
static STUDIO_DARK: Palette = Palette {
    look: Look::Studio,
    dark: true,
    backdrop: rgb(22, 22, 24),
    panel: rgb(32, 32, 34),
    surface: rgb(44, 44, 47),
    surface_high: rgb(54, 54, 58),
    border: rgb(58, 58, 62),
    edge: rgb(70, 70, 75),
    chip: rgb(56, 56, 60),
    field: rgb(28, 28, 30),
    text: rgb(245, 245, 247),
    label: rgb(232, 232, 237),
    dim: rgb(160, 160, 166),
    faint: rgb(112, 112, 118),
    accent: rgb(10, 132, 255),
    accent_soft: rgb(20, 60, 112),
    on_accent: rgb(255, 255, 255),
    ok: rgb(48, 209, 88),
    warn: rgb(255, 159, 10),
    err: rgb(255, 69, 58),
    checker_light: Color32::from_gray(48),
    checker_dark: Color32::from_gray(38),
    hover: rgb(66, 66, 70),
    hover_edge: rgb(86, 86, 92),
    open: rgb(62, 62, 66),
    knob_off: rgb(255, 255, 255),
    popup: rgb(46, 46, 49),
    scrim: rgba(22, 22, 24, 200),
    card_radius: 10,
    control_radius: 6,
    card_stroke: Color32::TRANSPARENT,
    card_shadow: NO_SHADOW,
    popup_shadow: shadow(8, 24, 150),
    floating: false,
    separators: true,
    framed_icons: false,
    heading: rgb(160, 160, 166),
    glows: &[],
};

static STUDIO_LIGHT: Palette = Palette {
    look: Look::Studio,
    dark: false,
    backdrop: rgb(232, 232, 235),
    panel: rgb(246, 246, 248),
    surface: rgb(255, 255, 255),
    surface_high: rgb(242, 242, 245),
    border: rgb(216, 216, 220),
    edge: rgb(206, 206, 211),
    chip: rgb(234, 234, 238),
    field: rgb(255, 255, 255),
    text: rgb(29, 29, 31),
    label: rgb(29, 29, 31),
    dim: rgb(108, 108, 114),
    faint: rgb(158, 158, 164),
    accent: rgb(0, 122, 255),
    accent_soft: rgb(212, 230, 255),
    on_accent: rgb(255, 255, 255),
    ok: rgb(36, 138, 61),
    warn: rgb(178, 80, 0),
    err: rgb(215, 0, 21),
    checker_light: Color32::from_gray(255),
    checker_dark: Color32::from_gray(234),
    hover: rgb(226, 226, 231),
    hover_edge: rgb(194, 194, 200),
    open: rgb(230, 230, 235),
    knob_off: rgb(255, 255, 255),
    popup: rgb(255, 255, 255),
    scrim: rgba(232, 232, 235, 200),
    card_radius: 10,
    control_radius: 6,
    card_stroke: Color32::TRANSPARENT,
    card_shadow: shadow(1, 4, 24),
    popup_shadow: shadow(10, 30, 55),
    floating: false,
    separators: true,
    framed_icons: false,
    heading: rgb(108, 108, 114),
    glows: &[],
};

static GLASS_DARK: Palette = Palette {
    look: Look::Glass,
    dark: true,
    // Smoked glass: dark panes over a lighter, softly lit graphite, so each
    // pane stands apart from what is behind it. Panes lighter than a dark
    // backdrop (the first plain-grey try) sat within a few levels of it and
    // read as mud (the owner, September 25, 2026: "looks shit in dark theme").
    backdrop: rgb(34, 34, 38),
    panel: rgba(16, 16, 18, 190),
    surface: rgba(18, 18, 20, 175),
    surface_high: rgba(255, 255, 255, 22),
    border: rgba(255, 255, 255, 34),
    edge: rgba(255, 255, 255, 30),
    chip: rgba(255, 255, 255, 20),
    field: rgba(0, 0, 0, 100),
    text: rgb(246, 246, 250),
    label: rgb(236, 236, 242),
    dim: rgb(170, 170, 176),
    faint: rgb(116, 116, 122),
    accent: rgb(10, 132, 255),
    accent_soft: rgba(10, 132, 255, 80),
    on_accent: rgb(255, 255, 255),
    ok: rgb(48, 209, 88),
    warn: rgb(255, 159, 10),
    err: rgb(255, 69, 58),
    checker_light: rgb(52, 52, 56),
    checker_dark: rgb(40, 40, 44),
    hover: rgba(255, 255, 255, 38),
    hover_edge: rgba(255, 255, 255, 76),
    open: rgba(255, 255, 255, 30),
    knob_off: rgb(255, 255, 255),
    popup: rgba(24, 24, 27, 246),
    scrim: rgba(20, 20, 22, 190),
    card_radius: 18,
    control_radius: 12,
    card_stroke: rgba(255, 255, 255, 40),
    card_shadow: GLASS_SHADOW_DARK,
    popup_shadow: shadow(12, 32, 150),
    floating: true,
    separators: false,
    framed_icons: true,
    heading: rgb(246, 246, 250),
    glows: &[
        Glow {
            x: 0.3,
            y: 0.,
            reach: 0.9,
            color: rgba(84, 84, 90, 170),
        },
        Glow {
            x: 0.85,
            y: 1.,
            reach: 0.8,
            color: rgba(58, 58, 64, 140),
        },
    ],
};

static GLASS_LIGHT: Palette = Palette {
    look: Look::Glass,
    dark: false,
    backdrop: rgb(228, 228, 232),
    panel: rgba(255, 255, 255, 160),
    surface: rgba(255, 255, 255, 150),
    surface_high: rgba(255, 255, 255, 215),
    border: rgba(0, 0, 0, 28),
    edge: rgba(0, 0, 0, 22),
    chip: rgba(255, 255, 255, 150),
    field: rgba(255, 255, 255, 200),
    text: rgb(28, 28, 32),
    label: rgb(28, 28, 32),
    dim: rgb(88, 88, 94),
    faint: rgb(140, 140, 146),
    accent: rgb(0, 122, 255),
    accent_soft: rgba(0, 122, 255, 44),
    on_accent: rgb(255, 255, 255),
    ok: rgb(36, 138, 61),
    warn: rgb(178, 80, 0),
    err: rgb(215, 0, 21),
    checker_light: Color32::from_gray(252),
    checker_dark: Color32::from_gray(232),
    hover: rgba(255, 255, 255, 220),
    hover_edge: rgba(0, 0, 0, 44),
    open: rgba(255, 255, 255, 200),
    knob_off: rgb(255, 255, 255),
    popup: rgba(252, 252, 253, 246),
    scrim: rgba(228, 228, 232, 190),
    card_radius: 18,
    control_radius: 12,
    card_stroke: rgba(255, 255, 255, 215),
    card_shadow: GLASS_SHADOW_LIGHT,
    popup_shadow: shadow(12, 32, 60),
    floating: true,
    separators: false,
    framed_icons: true,
    heading: rgb(28, 28, 32),
    glows: &[
        Glow {
            x: 0.12,
            y: 0.,
            reach: 0.8,
            color: rgba(255, 255, 255, 180),
        },
        Glow {
            x: 0.9,
            y: 1.,
            reach: 0.7,
            color: rgba(206, 206, 212, 140),
        },
    ],
};

impl Palette {
    /// Behind the rail's cards: the backdrop, the sidebar's grey, or nothing
    /// in front of the Glass backdrop.
    pub fn rail_fill(&self) -> Color32 {
        match self.look {
            Look::Studio => self.panel,
            Look::Glass => Color32::TRANSPARENT,
        }
    }
    /// Behind the picture cards.
    pub fn canvas_fill(&self) -> Color32 {
        if self.floating {
            Color32::TRANSPARENT
        } else {
            self.backdrop
        }
    }
}

thread_local! {
    static CURRENT: Cell<&'static Palette> = const { Cell::new(&STUDIO_DARK) };
}

/// The palette in force on this thread.
pub(super) fn pal() -> &'static Palette {
    CURRENT.with(Cell::get)
}

/// The palette of `look`, light or dark.
pub(super) fn palette(look: Look, dark: bool) -> &'static Palette {
    match (look, dark) {
        (Look::Glass, true) => &GLASS_DARK,
        (Look::Glass, false) => &GLASS_LIGHT,
        (Look::Studio, true) => &STUDIO_DARK,
        (Look::Studio, false) => &STUDIO_LIGHT,
    }
}

/// Put `look`, light or dark, in force; true when it changed.
pub(super) fn set(look: Look, dark: bool) -> bool {
    let next = palette(look, dark);
    let changed = !std::ptr::eq(pal(), next);
    CURRENT.with(|current| current.set(next));
    changed
}

/// `color` at `alpha` of its opacity.
pub(super) fn faded(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha.clamp(0., 1.))
}

/// The window's backdrop: the palette's colour, lit by its glows. Painted
/// first into the background layer, so every panel lies on top of it.
pub(super) fn paint_backdrop(ctx: &egui::Context) {
    let p = pal();
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    if p.glows.is_empty() {
        painter.rect_filled(screen, 0., p.backdrop);
        return;
    }
    // A grid of vertices, each coloured by the glows that reach it; the GPU
    // blends between them.
    const COLS: usize = 24;
    const ROWS: usize = 16;
    let longer = screen.width().max(screen.height()).max(1.);
    let mut mesh = egui::Mesh::default();
    for row in 0..=ROWS {
        for col in 0..=COLS {
            let (u, v) = (col as f32 / COLS as f32, row as f32 / ROWS as f32);
            let pos = egui::pos2(
                screen.left() + u * screen.width(),
                screen.top() + v * screen.height(),
            );
            let mut color = egui::Rgba::from(p.backdrop);
            for glow in p.glows {
                let dx = (u - glow.x) * screen.width() / longer;
                let dy = (v - glow.y) * screen.height() / longer;
                let d = (dx * dx + dy * dy).sqrt() / glow.reach;
                let near = (1. - d * d).max(0.);
                let strength = near * near * glow.color.a() as f32 / 255.;
                let tint = egui::Rgba::from(glow.color.to_opaque());
                color = color * (1. - strength) + tint * strength;
            }
            mesh.colored_vertex(pos, color.into());
        }
    }
    let width = (COLS + 1) as u32;
    for row in 0..ROWS as u32 {
        for col in 0..COLS as u32 {
            let i = row * width + col;
            mesh.add_triangle(i, i + 1, i + width);
            mesh.add_triangle(i + 1, i + width + 1, i + width);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}
