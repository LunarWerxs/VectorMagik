//! Part of `desktop_ui`: how the window looks. Three looks, each light or
//! dark: Classic (the teal-on-charcoal design the app shipped with) and two
//! previews of a design after Apple's current one, asked for on September 24,
//! 2026 ("make it feel modern, something Apple would make"): Glass, whose bars
//! and cards float as translucent panes over coloured light, after Liquid
//! Glass, and Studio, the opaque sidebar-and-toolbar layout of a Mac pro app.
//! Every colour the window paints comes from the palette in force; the UI
//! thread sets it once a frame (`set`), so a switch shows on the next frame.

use egui::{Color32, Shadow};
use std::cell::Cell;

/// The design of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Classic,
    Glass,
    Studio,
}

impl Look {
    pub const ALL: [Look; 3] = [Look::Classic, Look::Glass, Look::Studio];
    pub fn label(self) -> &'static str {
        match self {
            Look::Classic => "Classic",
            Look::Glass => "Glass",
            Look::Studio => "Studio",
        }
    }
    pub fn about(self) -> &'static str {
        match self {
            Look::Classic => "The original design: charcoal and teal.",
            Look::Glass => {
                "Preview: bars and cards float as frosted glass over soft colour, \
                 after Apple's Liquid Glass."
            }
            Look::Studio => {
                "Preview: a quiet sidebar and toolbar in system grey and blue, \
                 like a Mac pro app."
            }
        }
    }
    /// The word kept in the preferences.
    pub fn word(self) -> &'static str {
        match self {
            Look::Classic => "classic",
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
    /// Behind everything: the canvas area (and the Classic rail).
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
    /// Card headings in small capitals of the accent (Classic), or in the
    /// title face in `heading`.
    pub heading_upper: bool,
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

const NO_SHADOW: Shadow = Shadow {
    offset: [0, 0],
    blur: 0,
    spread: 0,
    color: Color32::TRANSPARENT,
};

// Everything outside the pictures in Classic dark stays darker than 60/255,
// which the snapshot test samples.
static CLASSIC_DARK: Palette = Palette {
    look: Look::Classic,
    dark: true,
    backdrop: rgb(13, 16, 20),
    panel: rgb(23, 27, 32),
    surface: rgb(23, 27, 32),
    surface_high: rgb(31, 36, 42),
    border: rgb(44, 50, 58),
    edge: rgb(52, 58, 66),
    chip: rgb(36, 41, 48),
    field: rgb(16, 19, 23),
    text: rgb(225, 227, 228),
    label: rgb(200, 206, 210),
    dim: rgb(160, 168, 174),
    faint: rgb(104, 112, 120),
    accent: rgb(79, 209, 197),
    accent_soft: rgb(0, 80, 74),
    on_accent: rgb(0, 44, 40),
    ok: rgb(112, 214, 128),
    warn: rgb(255, 184, 76),
    err: rgb(255, 112, 112),
    checker_light: Color32::from_gray(52),
    checker_dark: Color32::from_gray(40),
    hover: rgb(46, 52, 60),
    hover_edge: rgb(76, 84, 94),
    open: rgb(42, 48, 56),
    knob_off: rgb(160, 168, 174),
    popup: rgb(23, 27, 32),
    scrim: rgba(13, 16, 20, 200),
    card_radius: 12,
    control_radius: 8,
    card_stroke: rgb(44, 50, 58),
    card_shadow: NO_SHADOW,
    popup_shadow: shadow(6, 18, 140),
    floating: false,
    separators: false,
    framed_icons: true,
    heading_upper: true,
    heading: rgb(79, 209, 197),
    glows: &[],
};

static CLASSIC_LIGHT: Palette = Palette {
    look: Look::Classic,
    dark: false,
    backdrop: rgb(234, 238, 240),
    panel: rgb(255, 255, 255),
    surface: rgb(255, 255, 255),
    surface_high: rgb(243, 246, 247),
    border: rgb(212, 218, 222),
    edge: rgb(204, 211, 216),
    chip: rgb(236, 240, 242),
    field: rgb(247, 249, 250),
    text: rgb(24, 30, 34),
    label: rgb(40, 48, 54),
    dim: rgb(82, 92, 100),
    faint: rgb(132, 142, 150),
    accent: rgb(0, 148, 138),
    accent_soft: rgb(200, 238, 234),
    on_accent: rgb(255, 255, 255),
    ok: rgb(30, 140, 62),
    warn: rgb(176, 94, 0),
    err: rgb(200, 40, 40),
    checker_light: Color32::from_gray(252),
    checker_dark: Color32::from_gray(232),
    hover: rgb(228, 233, 236),
    hover_edge: rgb(176, 186, 192),
    open: rgb(230, 235, 238),
    knob_off: rgb(255, 255, 255),
    popup: rgb(255, 255, 255),
    scrim: rgba(234, 238, 240, 200),
    card_radius: 12,
    control_radius: 8,
    card_stroke: rgb(212, 218, 222),
    card_shadow: NO_SHADOW,
    popup_shadow: shadow(6, 18, 40),
    floating: false,
    separators: false,
    framed_icons: true,
    heading_upper: true,
    heading: rgb(0, 148, 138),
    glows: &[],
};

// Apple's system colours (label, secondary and tertiary label, system blue,
// green, orange and red) in their light and dark forms.
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
    heading_upper: false,
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
    heading_upper: false,
    heading: rgb(108, 108, 114),
    glows: &[],
};

static GLASS_DARK: Palette = Palette {
    look: Look::Glass,
    dark: true,
    backdrop: rgb(9, 11, 19),
    panel: rgba(30, 34, 48, 170),
    surface: rgba(34, 38, 52, 150),
    surface_high: rgba(255, 255, 255, 30),
    border: rgba(255, 255, 255, 40),
    edge: rgba(255, 255, 255, 30),
    chip: rgba(255, 255, 255, 20),
    field: rgba(0, 0, 0, 80),
    text: rgb(246, 246, 250),
    label: rgb(236, 236, 242),
    dim: rgb(170, 174, 188),
    faint: rgb(114, 118, 134),
    accent: rgb(10, 132, 255),
    accent_soft: rgba(10, 132, 255, 80),
    on_accent: rgb(255, 255, 255),
    ok: rgb(48, 209, 88),
    warn: rgb(255, 159, 10),
    err: rgb(255, 69, 58),
    checker_light: rgb(58, 60, 70),
    checker_dark: rgb(46, 48, 58),
    hover: rgba(255, 255, 255, 38),
    hover_edge: rgba(255, 255, 255, 76),
    open: rgba(255, 255, 255, 30),
    knob_off: rgb(255, 255, 255),
    popup: rgba(32, 36, 50, 244),
    scrim: rgba(9, 11, 19, 190),
    card_radius: 18,
    control_radius: 12,
    card_stroke: rgba(255, 255, 255, 40),
    card_shadow: shadow(10, 28, 110),
    popup_shadow: shadow(12, 32, 150),
    floating: true,
    separators: false,
    framed_icons: true,
    heading_upper: false,
    heading: rgb(246, 246, 250),
    glows: &[
        Glow {
            x: 0.1,
            y: 0.05,
            reach: 0.6,
            color: rgba(96, 72, 236, 170),
        },
        Glow {
            x: 0.92,
            y: 0.9,
            reach: 0.65,
            color: rgba(0, 150, 190, 150),
        },
        Glow {
            x: 0.78,
            y: 0.02,
            reach: 0.45,
            color: rgba(214, 60, 160, 110),
        },
        Glow {
            x: 0.18,
            y: 0.98,
            reach: 0.5,
            color: rgba(36, 104, 255, 110),
        },
    ],
};

static GLASS_LIGHT: Palette = Palette {
    look: Look::Glass,
    dark: false,
    backdrop: rgb(232, 236, 245),
    panel: rgba(255, 255, 255, 160),
    surface: rgba(255, 255, 255, 150),
    surface_high: rgba(255, 255, 255, 215),
    border: rgba(0, 0, 0, 28),
    edge: rgba(0, 0, 0, 22),
    chip: rgba(255, 255, 255, 150),
    field: rgba(255, 255, 255, 200),
    text: rgb(28, 28, 32),
    label: rgb(28, 28, 32),
    dim: rgb(86, 88, 100),
    faint: rgb(138, 142, 156),
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
    popup: rgba(250, 251, 254, 246),
    scrim: rgba(232, 236, 245, 190),
    card_radius: 18,
    control_radius: 12,
    card_stroke: rgba(255, 255, 255, 215),
    card_shadow: shadow(8, 26, 34),
    popup_shadow: shadow(12, 32, 60),
    floating: true,
    separators: false,
    framed_icons: true,
    heading_upper: false,
    heading: rgb(28, 28, 32),
    glows: &[
        Glow {
            x: 0.08,
            y: 0.06,
            reach: 0.62,
            color: rgba(120, 170, 255, 170),
        },
        Glow {
            x: 0.94,
            y: 0.92,
            reach: 0.62,
            color: rgba(255, 160, 205, 150),
        },
        Glow {
            x: 0.86,
            y: 0.08,
            reach: 0.45,
            color: rgba(120, 226, 206, 130),
        },
        Glow {
            x: 0.14,
            y: 0.94,
            reach: 0.5,
            color: rgba(188, 160, 255, 130),
        },
    ],
};

impl Palette {
    /// Behind the rail's cards: the backdrop, the sidebar's grey, or nothing
    /// in front of the Glass backdrop.
    pub fn rail_fill(&self) -> Color32 {
        match self.look {
            Look::Classic => self.backdrop,
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
    static CURRENT: Cell<&'static Palette> = const { Cell::new(&CLASSIC_DARK) };
}

/// The palette in force on this thread.
pub(super) fn pal() -> &'static Palette {
    CURRENT.with(Cell::get)
}

/// The palette of `look`, light or dark.
pub(super) fn palette(look: Look, dark: bool) -> &'static Palette {
    match (look, dark) {
        (Look::Classic, true) => &CLASSIC_DARK,
        (Look::Classic, false) => &CLASSIC_LIGHT,
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
