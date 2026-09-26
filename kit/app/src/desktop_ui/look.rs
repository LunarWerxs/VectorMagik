//! Part of `desktop_ui`: how the window looks. One design after Apple's
//! current one (asked for on September 24, 2026: "make it feel modern,
//! something Apple would make"), light or dark: Glass, whose bars and cards
//! float as frosted panes over the window, after Liquid Glass. In the dark
//! the cards are a little lighter than a flat near-black behind them, light
//! Glass turned dark; the owner chose it on September 25, 2026 from four dark
//! shades clicked through in the app ("Lifted should be the default theme
//! ... that'll be the only theme"), after Classic, Studio, a gradient behind
//! the dark panes ("not properly Apple") and the other shades went. Every
//! colour the window paints comes from the palette in force; the UI thread
//! sets it once a frame (`set`), so a switch shows on the next frame.

use egui::{Color32, Shadow};
use std::cell::Cell;

/// Light or dark: chosen, or the system's.
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

/// A patch of light behind the panes: its centre as a fraction of the
/// window, its reach as a fraction of the window's longer side, and its
/// colour, whose alpha is how strongly it tints. Only light Glass has any.
pub(super) struct Glow {
    pub x: f32,
    pub y: f32,
    pub reach: f32,
    pub color: Color32,
}

/// Every colour and shape the window's design decides.
pub(super) struct Palette {
    pub dark: bool,
    /// Behind everything, showing through the rail and the canvas.
    pub backdrop: Color32,
    /// The header and the status bar, panes floating over the backdrop.
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
    /// The rim of a card and of the floating bars.
    pub card_stroke: Color32,
    pub card_shadow: Shadow,
    pub popup_shadow: Shadow,
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

/// How far past a card's sides its shadow reaches: half its 12 px blur, with
/// three above and nine below, which every margin around a card leaves room
/// for (the first Glass drew 28 px of blur 10 px down, and the rail and the
/// bars cut it off at the sides; the owner, September 25, 2026).
pub(super) const SHADOW_REACH: i8 = 6;

// Apple's system colours (label, secondary and tertiary label, system blue,
// green, orange and red) in their light and dark forms. Everything outside
// the pictures in the dark stays darker than 60/255, which the snapshot test
// samples.
static DARK: Palette = Palette {
    dark: true,
    // A flat near-black, no light behind it; the cards white at low opacity,
    // so a little lighter than it, with crisp rims.
    backdrop: rgb(12, 12, 14),
    panel: rgba(255, 255, 255, 18),
    surface: rgba(255, 255, 255, 14),
    surface_high: rgba(255, 255, 255, 24),
    border: rgba(255, 255, 255, 30),
    edge: rgba(255, 255, 255, 30),
    chip: rgba(255, 255, 255, 16),
    field: rgba(0, 0, 0, 120),
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
    checker_light: Color32::from_gray(48),
    checker_dark: Color32::from_gray(38),
    hover: rgba(255, 255, 255, 32),
    hover_edge: rgba(255, 255, 255, 76),
    open: rgba(255, 255, 255, 26),
    knob_off: rgb(255, 255, 255),
    popup: rgba(34, 34, 38, 246),
    scrim: rgba(12, 12, 14, 190),
    card_radius: 18,
    control_radius: 12,
    card_stroke: rgba(255, 255, 255, 36),
    card_shadow: shadow(3, 12, 150),
    popup_shadow: shadow(12, 32, 150),
    heading: rgb(246, 246, 250),
    glows: &[],
};

static LIGHT: Palette = Palette {
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
    card_shadow: shadow(3, 12, 30),
    popup_shadow: shadow(12, 32, 60),
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

thread_local! {
    static CURRENT: Cell<&'static Palette> = const { Cell::new(&DARK) };
}

/// The palette in force on this thread.
pub(super) fn pal() -> &'static Palette {
    CURRENT.with(Cell::get)
}

/// The palette, light or dark.
pub(super) fn palette(dark: bool) -> &'static Palette {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// Put the light or the dark palette in force; true when it changed.
pub(super) fn set(dark: bool) -> bool {
    let next = palette(dark);
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
