//! Part of `desktop_ui`: the widgets, theme, fonts, dialogs and worker threads.

use super::*;

/// Derive the shown document off the UI thread. The latest request wins, so
/// dragging the tolerance slider never queues up stale work.
pub(super) fn spawn_deriver(ctx: egui::Context) -> (Sender<DeriveRequest>, Receiver<DeriveResult>) {
    platform::serve(ctx, |request: DeriveRequest| {
        let auto = request.job == DeriveJob::Auto;
        let settings = &request.settings;
        let outcome = (|| {
            let raw = request.raw.without_islands(&settings.deleted)?;
            let (tolerance, base) = match request.job {
                DeriveJob::Derive => {
                    let base = match settings.simplify {
                        Some(t) if settings.simplify_by_hand => raw.simplified_by_hand(t)?,
                        Some(t) => raw.simplified(t)?,
                        None => raw,
                    };
                    (settings.simplify, base)
                }
                DeriveJob::Auto => {
                    let (t, doc) = crate::auto_simplify_tolerance(&raw)?;
                    (Some(t), doc)
                }
            };
            let document = Desktop::finish(base, settings)?;
            // Painted stacked, so never Auto's own rendering of the cut-out
            // shapes, whose edges show the background between colours.
            let presented = Presented::of(&document, settings.sticker.as_ref())?;
            // Counted here too, so the UI thread parses nothing per result.
            Ok((tolerance, Derived::new(document, presented)))
        })();
        Some(DeriveResult {
            serial: request.serial,
            raw_version: request.raw_version,
            auto,
            outcome,
        })
    })
}

/// Render crisp tiles of the vector off the UI thread. The latest request
/// wins; the parsed tree is kept between requests for the same document.
pub(super) fn spawn_tiler(ctx: egui::Context) -> (Sender<TileRequest>, Receiver<TileResult>) {
    let mut cached: Option<(u64, crate::PreviewTree)> = None;
    platform::serve(ctx, move |request: TileRequest| {
        if cached.as_ref().is_none_or(|(v, _)| *v != request.version) {
            cached = crate::preview_tree(&request.svg)
                .ok()
                .map(|tree| (request.version, tree));
        }
        let (_, tree) = cached.as_ref()?;
        let image = crate::render_region(
            tree,
            request.document_width,
            request.scale * request.ppp,
            request.origin,
            request.size,
        )
        .ok()?;
        Some(TileResult {
            version: request.version,
            scale: request.scale,
            ppp: request.ppp,
            origin: request.origin,
            image,
        })
    })
}

/// Pick the orientation that shows the larger picture: side by side for square
/// and tall images, one above the other for wide ones or portrait windows.
pub(crate) fn stacked_layout(avail: Vec2, aspect: f32) -> bool {
    let gap = 10.;
    let header = 34.;
    let aspect = aspect.max(0.01);
    let side_by_side = (((avail.x - gap) / 2. - 26.) / aspect).min(avail.y - header - 26.);
    let stacked = ((avail.x - 26.) / aspect).min((avail.y - gap) / 2. - header - 26.);
    stacked > side_by_side * 1.15
}

/// A rail card: a rounded surface with a small accent heading that folds it
/// to one line and back, the body sliding open and shut. The heading shows
/// `summary` while the card is folded, and also while it is open when
/// `summary_open` (a card whose switch is off shows only the switch, so the
/// heading says what is in force).
#[allow(
    clippy::too_many_arguments,
    reason = "one argument per part of the card; every card passes all of them"
)]
pub(super) fn card(
    ui: &mut egui::Ui,
    family: &FontFamily,
    glyph: &str,
    title: &str,
    open: &mut bool,
    summary: &str,
    summary_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    card_frame()
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.;
            let fold_id = ui.make_persistent_id(("card-fold", title));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                fold_id,
                *open,
            );
            state.set_open(*open);
            let heading = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.;
                    let p = pal();
                    ui.label(RichText::new(glyph).size(13.).color(p.accent));
                    // Titled the way a Mac sidebar titles its sections.
                    let heading = RichText::new(title).size(13.);
                    ui.add(
                        egui::Label::new(heading.family(family.clone()).color(p.heading))
                            .selectable(false),
                    );
                    if (!*open || summary_open) && !summary.is_empty() {
                        ui.add(
                            egui::Label::new(RichText::new(summary).size(11.5).color(pal().dim))
                                .selectable(false)
                                .truncate(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(12., 12.), egui::Sense::hover());
                        chevron(ui, rect, state.openness(ui.ctx()));
                    });
                })
                .response;
            let heading = ui.interact(heading.rect, heading.id.with("fold"), egui::Sense::click());
            if heading.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if heading
                .on_hover_text(if *open {
                    "Click to fold this card"
                } else {
                    "Click to open this card"
                })
                .clicked()
            {
                *open = !*open;
            }
            state.set_open(*open);
            state.show_body_unindented(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.;
                ui.add_space(7.);
                body(ui);
            });
        });
}

/// Settings most pictures never need, folded under a quiet "More options"
/// row that opens them in place; open or folded is remembered for the
/// session.
pub(super) fn more_options(ui: &mut egui::Ui, name: &str, body: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(("more-options", name));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    ui.add_space(2.);
    let label = RichText::new(if open {
        "Fewer options"
    } else {
        "More options"
    })
    .size(12.)
    .color(pal().accent);
    if ui
        .add(egui::Label::new(label).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    reveal(ui, name, open, |ui| {
        ui.add_space(2.);
        body(ui);
    });
}

/// The frame of a card: the rail's cards and the two picture cards.
pub(super) fn card_frame() -> egui::Frame {
    let p = pal();
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1_f32, p.card_stroke))
        .corner_radius(p.card_radius)
        .shadow(p.card_shadow)
}

/// The fold mark of a card heading: a small triangle pointing right when
/// folded and down when open, turning with the fold's animation.
fn chevron(ui: &egui::Ui, rect: egui::Rect, openness: f32) {
    let angle = (openness - 1.) * std::f32::consts::FRAC_PI_2;
    let (sin, cos) = angle.sin_cos();
    let turn = |x: f32, y: f32| rect.center() + Vec2::new(x * cos - y * sin, x * sin + y * cos);
    // Pointing down at rest; turned a quarter back when folded.
    let points = vec![turn(-4., -2.), turn(4., -2.), turn(0., 3.)];
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        pal().faint,
        Stroke::NONE,
    ));
}

/// Part of a card shown only while `open` (a switch is on), sliding open and
/// shut as it changes; its value while shown.
pub(super) fn reveal<R>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    open: bool,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let id = ui.make_persistent_id(("reveal", id));
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    state.set_open(open);
    state
        .show_body_unindented(ui, body)
        .map(|shown| shown.inner)
}

/// A grip in the middle of the rail's resize line while the line is hovered
/// or dragged, so it reads as a handle. The rail's panel is "controls";
/// egui keeps its resize interaction under `__resize`.
pub(super) fn rail_grip(ctx: &egui::Context, panel: egui::Rect) {
    let resize = egui::Id::new("controls").with("__resize");
    let Some(response) = ctx.read_response(resize) else {
        return;
    };
    if !(response.hovered() || response.dragged()) {
        return;
    }
    let center = egui::pos2(panel.right(), panel.center().y);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("rail-grip"),
    ));
    painter.rect(
        egui::Rect::from_center_size(center, Vec2::new(8., 34.)),
        4.,
        pal().surface_high,
        Stroke::new(1_f32, pal().text),
        StrokeKind::Inside,
    );
    for dy in [-7., 0., 7.] {
        painter.circle_filled(center + Vec2::new(0., dy), 1.5, pal().text);
    }
}

/// A subtle rounded box inside a card.
pub(super) fn inset(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(pal().chip)
        .corner_radius(8)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}

/// A switch on the right with its label (and optional subtitle) on the left;
/// both are clickable.
pub(super) fn toggle_row(
    ui: &mut egui::Ui,
    on: &mut bool,
    label: &str,
    subtitle: Option<&str>,
) -> egui::Response {
    let mut flip = false;
    let (_, mut response) = egui::Sides::new().show(
        ui,
        |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.;
                let text = ui.add(
                    egui::Label::new(RichText::new(label).color(pal().text))
                        .selectable(false)
                        .sense(egui::Sense::click()),
                );
                if let Some(subtitle) = subtitle {
                    ui.label(RichText::new(subtitle).size(11.).color(pal().dim));
                }
                flip = text.clicked();
            });
        },
        |ui| toggle(ui, on),
    );
    if flip {
        *on = !*on;
        response.mark_changed();
    }
    response
}

/// Black, White or a custom colour with its picker, as the Background
/// control offers; `custom` remembers the last custom choice. True when the
/// colour changed.
pub(super) fn color_choice(
    ui: &mut egui::Ui,
    id: &str,
    rgb: &mut [u8; 3],
    custom: &mut [u8; 3],
) -> bool {
    let mut changed = false;
    let is_custom = *rgb != [0, 0, 0] && *rgb != [255, 255, 255];
    if is_custom {
        let mut picked = *rgb;
        if ui.color_edit_button_srgb(&mut picked).changed() && picked != *rgb {
            *rgb = picked;
            *custom = picked;
            changed = true;
        }
    }
    let shown = match *rgb {
        [0, 0, 0] => "Black",
        [255, 255, 255] => "White",
        _ => "Custom",
    };
    let mut choice = shown;
    egui::ComboBox::from_id_salt(("sticker-color", id))
        .width(72.)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for option in ["Black", "White", "Custom"] {
                ui.selectable_value(&mut choice, option, option);
            }
        });
    if choice != shown {
        *rgb = match choice {
            "Black" => [0, 0, 0],
            "White" => [255, 255, 255],
            _ => *custom,
        };
        changed = true;
    }
    changed
}

/// Whether the picture's edge is opaque enough to hold a background shape:
/// at least half of the border pixels are fully opaque.
pub(super) fn border_opaque(raster: &Raster) -> bool {
    let (w, h) = (raster.width, raster.height);
    if w == 0 || h == 0 {
        return false;
    }
    let mut total = 0usize;
    let mut opaque = 0usize;
    let mut count = |x: usize, y: usize| {
        total += 1;
        if raster.pixels[y * w + x].0[3] == 255 {
            opaque += 1;
        }
    };
    for x in 0..w {
        count(x, 0);
        count(x, h - 1);
    }
    for y in 0..h {
        count(0, y);
        count(w - 1, y);
    }
    opaque * 2 >= total
}

/// The picture as a texture no larger than `cap` pixels per side, the GPU's
/// limit (egui's painter aborts on anything larger); scaled down to fit when
/// it is larger, as an image the engine cannot take never is.
pub(super) fn display_image(raster: &Raster, cap: usize) -> egui::ColorImage {
    let longest = raster.width.max(raster.height);
    let fitted;
    let shown = if longest > cap {
        let factor = cap as f64 / longest as f64;
        let side = |px: usize| ((px as f64 * factor).round() as usize).clamp(1, cap);
        fitted = crate::scaled_raster(raster, side(raster.width), side(raster.height));
        &fitted
    } else {
        raster
    };
    let rgba: Vec<u8> = shown.pixels.iter().flat_map(|p| p.0).collect();
    egui::ColorImage::from_rgba_unmultiplied([shown.width, shown.height], &rgba)
}

pub(super) fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(38., 20.);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let visuals = ui.style().interact_selectable(&response, *on);
        let radius = 0.5 * rect.height();
        let track = if *on { pal().accent } else { visuals.bg_fill };
        let edge = if *on {
            pal().accent
        } else {
            visuals.bg_stroke.color
        };
        ui.painter().rect(
            rect,
            radius,
            track,
            Stroke::new(1_f32, edge),
            StrokeKind::Inside,
        );
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let knob = if *on { Color32::WHITE } else { pal().knob_off };
        let centre = egui::pos2(x, rect.center().y);
        ui.painter().circle_filled(centre, radius - 4., knob);
        if !pal().dark {
            // A white knob on a pale track needs an edge to be seen.
            ui.painter()
                .circle_stroke(centre, radius - 4., Stroke::new(1_f32, pal().edge));
        }
    }
    response
}

/// A compact toolbar button carrying one glyph.
pub(super) fn icon_button(ui: &mut egui::Ui, glyph: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(glyph).size(15.).color(pal().text))
            .min_size(Vec2::new(36., 32.))
            .frame_when_inactive(pal().framed_icons)
            .corner_radius(CornerRadius::same(if pal().floating { 16 } else { 10 })),
    )
}

/// A rounded button with `text` at `size`, `min` wide and high, `radius` corners.
pub(super) fn rounded_button(
    text: &str,
    size: f32,
    min: Vec2,
    radius: u8,
) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).size(size))
        .min_size(min)
        .corner_radius(CornerRadius::same(radius))
}

/// The pill button of the cards.
pub(super) fn pill_widget(text: &str) -> egui::Button<'static> {
    rounded_button(text, 12.5, Vec2::new(40., 24.), 12)
}

pub(super) fn pill(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(pill_widget(text))
}

/// One option of a segmented choice; the selected one is filled with the accent.
pub(super) fn choice_width(
    ui: &mut egui::Ui,
    text: &str,
    selected: bool,
    width: f32,
) -> egui::Response {
    let button = if selected {
        egui::Button::new(
            RichText::new(text)
                .size(12.5)
                .color(pal().on_accent)
                .strong(),
        )
        .fill(pal().accent)
        .stroke(Stroke::new(1_f32, pal().accent))
    } else {
        egui::Button::new(RichText::new(text).size(12.5).color(pal().text))
    };
    ui.add(
        button
            .min_size(Vec2::new(width, 24.))
            .corner_radius(CornerRadius::same(12)),
    )
}

/// A square footer button for one character.
pub(super) fn small_button_widget(text: &str) -> egui::Button<'static> {
    rounded_button(text, 14., Vec2::new(24., 24.), 8)
}

pub(super) fn small_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(small_button_widget(text))
}

/// A settings row: a dim label on the left, the control on the right.
pub(super) fn labelled_row<R>(
    ui: &mut egui::Ui,
    label: &str,
    right: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Sides::new()
        .show(
            ui,
            |ui| {
                ui.label(RichText::new(label).size(12.5).color(pal().dim));
            },
            right,
        )
        .1
}

/// A tooltip pinned to the pointer while it stays over the item: one line,
/// never wrapped into a narrow column, with room either side of the text.
pub(super) fn pointer_tip(ui: &egui::Ui, id: &str, text: String) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        ui.id().with(id),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        egui::Frame::new()
            .inner_margin(Margin::symmetric(6, 1))
            .show(ui, |ui| {
                ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
            });
    });
}

/// The heading of a popup: its width, spacing and title in the title face.
pub(super) fn popup_heading(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    size: f32,
    family: &FontFamily,
) {
    ui.set_width(width);
    ui.spacing_mut().item_spacing.y = 8.;
    ui.label(
        RichText::new(title)
            .size(size)
            .family(family.clone())
            .color(pal().text),
    );
}

/// The grab cursor of something that pans on drag.
pub(super) fn grab_cursor(ui: &egui::Ui, response: &egui::Response) {
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// The frame of the header (`top`) and status bars: edge to edge, or in the
/// Glass look a pane floating over the backdrop.
pub(super) fn bar_frame(vertical: i8, top: bool) -> egui::Frame {
    let p = pal();
    let frame = egui::Frame::new()
        .fill(p.panel)
        .inner_margin(Margin::symmetric(16, vertical));
    if !p.floating {
        return frame;
    }
    // Room for the shadow below the header and below the status bar.
    let (above, below) = if top { (10, 10) } else { (4, 10) };
    frame
        .corner_radius(22)
        .stroke(Stroke::new(1_f32, p.card_stroke))
        .shadow(p.card_shadow)
        .outer_margin(Margin {
            left: 12,
            right: 12,
            top: above,
            bottom: below,
        })
}

/// The keyboard shortcuts as a quiet two-column table: keys in a fixed-width
/// face, the action beside each, two pairs to a row.
pub(super) fn legend(ui: &mut egui::Ui, entries: &[(String, &str)]) {
    egui::Grid::new("shortcut-legend")
        .spacing(Vec2::new(7., 3.))
        .show(ui, |ui| {
            for row in entries.chunks(2) {
                for (index, (key, action)) in row.iter().enumerate() {
                    if index > 0 {
                        // A cell of its own: space added inside a grid
                        // trips egui's debug assertion ("add_space makes no
                        // sense in a grid layout"), which failed the debug
                        // build's desktop tests. 3 px and the grid's 7 on
                        // each side keep the 17 px between pairs.
                        ui.allocate_exact_size(Vec2::new(3., 0.), egui::Sense::hover());
                    }
                    ui.add(
                        egui::Label::new(
                            RichText::new(key)
                                .font(FontId::monospace(10.5))
                                .color(pal().dim),
                        )
                        .selectable(false),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(*action).size(11.).color(pal().faint))
                            .selectable(false),
                    );
                }
                ui.end_row();
            }
        });
}

/// The keyboard shortcuts folded into one line whose hover lists them: a
/// legend always on screen took three rows of the rail for what every
/// button's tooltip already says (the Dredd review of September 23, 2026).
pub(super) fn shortcuts(ui: &mut egui::Ui, entries: &[(String, &str)]) {
    ui.add(
        egui::Label::new(
            RichText::new(format!("{}  Keyboard shortcuts", super::icon::INFO))
                .size(11.)
                .color(pal().faint),
        )
        .selectable(false)
        .sense(egui::Sense::hover()),
    )
    .on_hover_ui(|ui| legend(ui, entries));
}

/// A footer stat: a chip that reacts to the pointer and can be lit up when
/// what it stands for is active.
pub(super) fn stat(ui: &mut egui::Ui, glyph: &str, text: &str, active: bool) -> egui::Response {
    let label = format!("{glyph} {text}");
    let font = FontId::proportional(12.);
    let width = ui
        .painter()
        .layout_no_wrap(label.clone(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width + 14., 22.), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let (fill, edge, color) = match (active, hovered) {
            (true, true) => (pal().accent_soft, pal().accent, pal().text),
            (true, false) => (pal().accent_soft, pal().accent_soft, pal().accent),
            (false, true) => (pal().surface_high, pal().border, pal().text),
            (false, false) => (pal().chip, pal().chip, pal().dim),
        };
        ui.painter().rect(
            rect,
            11.,
            fill,
            Stroke::new(1_f32, edge),
            StrokeKind::Inside,
        );
        ui.painter()
            .text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A popup hanging off a widget's rectangle, closed by a click outside it:
/// the Save and stat popups.
pub(super) fn anchored_popup<'a>(
    id: egui::Id,
    ctx: &egui::Context,
    anchor: egui::Rect,
    align: egui::RectAlign,
) -> egui::Popup<'a> {
    egui::Popup::new(
        id,
        ctx.clone(),
        egui::PopupAnchor::ParentRect(anchor),
        egui::LayerId::background(),
    )
    .align(align)
    .gap(8.)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .frame(popup_frame())
}

/// A context menu at a point, closed by any click: the node and shape menus.
pub(super) fn menu_popup<'a>(id: egui::Id, ctx: &egui::Context, at: egui::Pos2) -> egui::Popup<'a> {
    egui::Popup::new(
        id,
        ctx.clone(),
        egui::PopupAnchor::Position(at),
        egui::LayerId::background(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .style(egui::containers::menu::menu_style)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
}

/// The frame shared by the Save, stat and Appearance popups and the
/// first-run question.
pub(super) fn popup_frame() -> egui::Frame {
    let p = pal();
    egui::Frame::new()
        .fill(p.popup)
        .stroke(Stroke::new(
            1_f32,
            if p.floating { p.card_stroke } else { p.border },
        ))
        .corner_radius(p.card_radius.max(12))
        .inner_margin(Margin::same(14))
        .shadow(p.popup_shadow)
}

/// Two answer tiles (`choice_tile`) side by side, or one above the other
/// when `narrow` (a phone's width, where two would be cut off); whether each
/// one's button was clicked.
pub(super) fn tile_pair(
    ui: &mut egui::Ui,
    family: &FontFamily,
    narrow: bool,
    (first, first_chosen): ([&str; 4], bool),
    (second, second_chosen): ([&str; 4], bool),
) -> (bool, bool) {
    if narrow {
        let a = choice_tile(ui, family, first, first_chosen).clicked();
        let b = choice_tile(ui, family, second, second_chosen).clicked();
        return (a, b);
    }
    ui.columns(2, |columns| {
        columns[0].spacing_mut().item_spacing.y = 4.;
        columns[1].spacing_mut().item_spacing.y = 4.;
        let a = choice_tile(&mut columns[0], family, first, first_chosen).clicked();
        let b = choice_tile(&mut columns[1], family, second, second_chosen).clicked();
        (a, b)
    })
}

/// The width of a question's dialog: `widest`, or the window less a margin
/// on a phone.
pub(super) fn dialog_width(ctx: &egui::Context, widest: f32) -> f32 {
    (ctx.content_rect().width() - 56.).clamp(260., widest)
}

/// One answer of a question put as tiles: a title, a price, a line saying
/// what it is for and a button, the whole tile lit while `chosen`. The
/// button's response.
pub(super) fn choice_tile(
    ui: &mut egui::Ui,
    family: &FontFamily,
    [title, price, about, button]: [&str; 4],
    chosen: bool,
) -> egui::Response {
    let p = pal();
    egui::Frame::new()
        .fill(if chosen {
            faded(p.accent, 0.12)
        } else {
            p.surface_high
        })
        .stroke(Stroke::new(1_f32, if chosen { p.accent } else { p.border }))
        .corner_radius(p.card_radius)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.;
            ui.label(
                RichText::new(title)
                    .size(16.)
                    .family(family.clone())
                    .color(p.text),
            );
            ui.label(RichText::new(price).size(13.).color(p.accent));
            // Two lines' room, so both tiles' buttons line up.
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 34.),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(34.);
                    ui.add(egui::Label::new(RichText::new(about).size(12.).color(p.dim)).wrap());
                },
            );
            ui.add_space(4.);
            ui.add_sized(
                [ui.available_width(), 30.],
                egui::Button::new(RichText::new(button).color(if chosen {
                    p.on_accent
                } else {
                    p.text
                }))
                .fill(if chosen { p.accent } else { p.chip })
                .corner_radius(CornerRadius::same(p.control_radius.max(8))),
            )
        })
        .inner
}

/// `#rrggbb` or `#rrggbbaa` from the engine's fill attributes; anything else
/// shows as a neutral grey rather than nothing.
pub(super) fn parse_hex(text: &str) -> Color32 {
    let hex = text.trim().trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("zz"), 16).ok();
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => Color32::from_rgb(r, g, b),
        (8, Some(r), Some(g), Some(b)) => {
            Color32::from_rgba_unmultiplied(r, g, b, channel(6).unwrap_or(255))
        }
        _ => Color32::from_gray(128),
    }
}

/// The source card while a file is being dragged over the window.
pub(super) fn drop_highlight(ui: &mut egui::Ui, area: egui::Rect, family: &FontFamily) {
    let painter = ui.painter_at(area);
    let inner = area.shrink(10.);
    painter.rect_filled(area, 0., pal().scrim);
    painter.rect_filled(inner, 10., faded(pal().accent, 0.09));
    dashed_rect(&painter, inner, Stroke::new(2_f32, pal().accent));
    painter.text(
        inner.center() - Vec2::new(0., 22.),
        Align2::CENTER_CENTER,
        icon::OPEN,
        FontId::proportional(44.),
        pal().accent,
    );
    painter.text(
        inner.center() + Vec2::new(0., 24.),
        Align2::CENTER_CENTER,
        "Drop to open",
        FontId::new(16., family.clone()),
        pal().text,
    );
}

pub(super) fn checkerboard(painter: &egui::Painter, rect: egui::Rect, visible: egui::Rect) {
    if visible.is_negative() {
        return;
    }
    let tile = 12.;
    let x0 = ((visible.min.x - rect.min.x) / tile).floor().max(0.) as usize;
    let y0 = ((visible.min.y - rect.min.y) / tile).floor().max(0.) as usize;
    let x1 = ((visible.max.x - rect.min.x) / tile).ceil() as usize;
    let y1 = ((visible.max.y - rect.min.y) / tile).ceil() as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = rect.min + Vec2::new(x as f32 * tile, y as f32 * tile);
            let cell = egui::Rect::from_min_size(p, Vec2::splat(tile)).intersect(rect);
            painter.rect_filled(
                cell,
                0.,
                if (x + y) % 2 == 0 {
                    pal().checker_light
                } else {
                    pal().checker_dark
                },
            );
        }
    }
}

pub(super) fn dashed_rect(painter: &egui::Painter, rect: egui::Rect, stroke: Stroke) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    for i in 0..4 {
        painter.extend(egui::Shape::dashed_line(
            &[corners[i], corners[(i + 1) % 4]],
            stroke,
            7.,
            6.,
        ));
    }
}

/// Use the Windows UI font when it is present (it is on every Windows install)
/// and keep egui's bundled text and emoji fonts as fallbacks, so icons and
/// non-Windows builds still render. Returns the family to use for titles.
pub(super) fn install_fonts(ctx: &egui::Context) -> FontFamily {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts");
    let mut load = |name: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) if !bytes.is_empty() => {
                fonts
                    .font_data
                    .insert(name.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
                true
            }
            _ => false,
        }
    };
    let regular = load("ui-regular", "segoeui.ttf");
    let semibold = load("ui-semibold", "seguisb.ttf");
    // egui lists its thin outline emoji font before its solid icon font; the
    // solid glyphs read better as toolbar icons, so they get first pick.
    let mut fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fallbacks.sort_by_key(|name| name == "NotoEmoji-Regular");
    // Without the Windows font (Mac, Linux, a browser tab) egui's text and
    // icon fonts lack the card arrows and the arrow in text; its monospace
    // font has them, so it comes last.
    if let Some(mono) = fonts
        .families
        .get(&FontFamily::Monospace)
        .and_then(|f| f.first())
    {
        if !fallbacks.contains(mono) {
            fallbacks.push(mono.clone());
        }
    }
    let mut family = Vec::new();
    if regular {
        family.push("ui-regular".to_owned());
    }
    family.extend(fallbacks.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, family);
    let title_family = if semibold {
        let mut family = vec!["ui-semibold".to_owned()];
        family.extend(fallbacks.iter().cloned());
        fonts
            .families
            .insert(FontFamily::Name("ui-semibold".into()), family);
        FontFamily::Name("ui-semibold".into())
    } else {
        FontFamily::Proportional
    };
    ctx.set_fonts(fonts);
    title_family
}

pub(super) fn apply_theme(ctx: &egui::Context, title_family: FontFamily) {
    let p = pal();
    ctx.set_theme(if p.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    let mut visuals = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = p.backdrop;
    visuals.window_fill = p.popup;
    visuals.window_stroke = Stroke::new(1_f32, p.border);
    visuals.window_corner_radius = CornerRadius::same(p.card_radius);
    visuals.menu_corner_radius = CornerRadius::same(p.card_radius);
    visuals.window_shadow = p.popup_shadow;
    visuals.popup_shadow = p.popup_shadow;
    visuals.extreme_bg_color = p.field;
    visuals.faint_bg_color = p.surface_high;
    visuals.code_bg_color = p.field;
    visuals.hyperlink_color = p.accent;
    visuals.error_fg_color = p.err;
    visuals.warn_fg_color = p.warn;
    visuals.selection.bg_fill = p.accent_soft;
    visuals.selection.stroke = Stroke::new(1_f32, p.accent);
    visuals.slider_trailing_fill = true;
    visuals.striped = false;
    visuals.text_cursor.stroke.color = p.accent;

    // Text on a lit control: white on the dark looks, the ink on the light.
    let lit = if p.dark { Color32::WHITE } else { p.text };
    let w = &mut visuals.widgets;
    for (v, fill, edge, fg) in [
        (&mut w.noninteractive, p.surface, p.border, p.label),
        (&mut w.inactive, p.chip, p.edge, p.text),
        (&mut w.hovered, p.hover, p.hover_edge, lit),
        (&mut w.active, p.accent_soft, p.accent, lit),
        (&mut w.open, p.open, p.hover_edge, p.text),
    ] {
        v.weak_bg_fill = fill;
        v.bg_fill = fill;
        v.bg_stroke = Stroke::new(1_f32, edge);
        v.fg_stroke = Stroke::new(1.2_f32, fg);
        v.corner_radius = CornerRadius::same(p.control_radius);
    }
    w.hovered.expansion = 1.;
    w.active.expansion = 1.;

    ctx.all_styles_mut(|style| {
        style.visuals = visuals.clone();
        style.interaction.tooltip_delay = 0.15;
        style.interaction.tooltip_grace_time = 0.05;
        // Cards slide open and shut, and switches slide, a little slower
        // than egui's twelfth of a second, so the motion reads.
        style.animation_time = 0.18;
        style.spacing.item_spacing = Vec2::new(8., 6.);
        style.spacing.button_padding = Vec2::new(12., 5.);
        style.spacing.interact_size = Vec2::new(40., 24.);
        style.spacing.icon_width = 18.;
        style.spacing.icon_width_inner = 10.;
        style.spacing.indent = 18.;
        style.spacing.combo_width = 120.;
        style.spacing.tooltip_width = 340.;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.)),
            (TextStyle::Button, FontId::proportional(14.)),
            (TextStyle::Heading, FontId::new(20., title_family.clone())),
            (TextStyle::Monospace, FontId::monospace(13.)),
        ]
        .into();
    });
}

/// Premultiplied RGBA pixels of the logo at `size` square, or `None` when the
/// SVG renderer cannot allocate.
pub(super) fn render_logo(size: u32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_str(LOGO_SVG, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(pixmap.take())
}

#[cfg(feature = "desktop")]
pub(super) fn window_icon() -> Option<Arc<egui::IconData>> {
    let mut rgba = render_logo(64)?;
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(Arc::new(egui::IconData {
        rgba,
        width: 64,
        height: 64,
    }))
}

/// The Open dialog's image filter: every extension `load_raster` decodes,
/// which is what the image crate's png, jpeg, bmp, gif, tiff, tga and pnm features read
/// (`tests::the_open_dialog_lists_every_format_the_loader_decodes`).
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows system dialogs use it")
)]
pub(super) const OPEN_FILTER: &str =
    "*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.tif;*.tiff;*.tga;*.pnm;*.pbm;*.pgm;*.ppm;*.pam";

/// The Open dialog's other files: Photoshop documents, traced like any
/// picture, and vector files, which are asked about (`vector_ui.rs`).
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows system dialogs use it")
)]
pub(super) const IMPORT_FILTER: &str = "*.psd;*.psb;*.svg;*.pdf;*.ai;*.eps;*.ps";

/// The system dialog's process while one is open. The window no longer
/// waits for it, so it could be closed first; `close_open_dialog` then closes
/// the dialog too rather than leave it behind with nothing to answer.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows system dialogs use it")
)]
static OPEN_DIALOG: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);

/// Close a system dialog still open, as the window closes.
#[cfg(feature = "desktop")]
pub(super) fn close_open_dialog() {
    if let Some(mut child) = OPEN_DIALOG.lock().ok().and_then(|mut open| open.take()) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(super) fn file_dialog(save: bool, name: &str, format: Format) -> Option<(String, u32)> {
    dialog(save, name, false, format)
}
pub(super) fn file_dialog_png() -> Option<(String, u32)> {
    dialog(true, "app-preview", true, Format::Svg)
}
/// The system dialogs, run by the local PowerShell so no dialog crate is
/// needed. `format` preselects the save filter and default extension; the
/// Open dialog filters to `OPEN_FILTER`. Blocks until the dialog closes, so
/// the window calls it on an errand thread.
pub(super) fn dialog(save: bool, name: &str, png: bool, format: Format) -> Option<(String, u32)> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let script = if png {
            r#"Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.SaveFileDialog; $d.Filter='PNG app preview|*.png'; $d.DefaultExt='png'; $d.FileName=$env:VM_EXPORT_NAME; if ($d.ShowDialog() -eq 'OK') { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [Console]::Write("1`n" + $d.FileName) }"#
        } else if save {
            r#"Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.SaveFileDialog; $d.Filter='SVG vector|*.svg|PDF vector|*.pdf|EPS vector (white background)|*.eps|Adobe Illustrator|*.ai|DXF drawing|*.dxf|Windows metafile|*.emf|PNG picture|*.png'; $d.FilterIndex=[int]$env:VM_EXPORT_FILTER; $d.DefaultExt=$env:VM_EXPORT_EXT; $d.AddExtension=$true; $d.FileName=$env:VM_EXPORT_NAME; if ($d.ShowDialog() -eq 'OK') { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [Console]::Write([string]$d.FilterIndex + "`n" + $d.FileName) }"#
        } else {
            r#"Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Filter='Pictures and vector files|' + $env:VM_OPEN_FILTER; if ($d.ShowDialog() -eq 'OK') { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [Console]::Write([string]$d.FilterIndex + "`n" + $d.FileName) }"#
        };
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", script])
            .env("VM_EXPORT_NAME", name)
            .env("VM_EXPORT_FILTER", format.filter_index().to_string())
            .env("VM_EXPORT_EXT", format.extension())
            .env("VM_OPEN_FILTER", format!("{OPEN_FILTER};{IMPORT_FILTER}"))
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        if let Ok(mut open) = OPEN_DIALOG.lock() {
            *open = Some(child);
        }
        // The answer, or nothing once the dialog is cancelled or closed with
        // the window.
        let mut text = String::new();
        let read = std::io::Read::read_to_string(&mut stdout, &mut text);
        if let Some(mut child) = OPEN_DIALOG.lock().ok().and_then(|mut open| open.take()) {
            let _ = child.wait();
        }
        read.ok()?;
        let (filter, path) = text.trim_start_matches('\u{feff}').split_once('\n')?;
        if path.is_empty() {
            None
        } else {
            Some((path.to_owned(), filter.parse().ok()?))
        }
    }
    #[cfg(not(windows))]
    {
        // macOS asks AppleScript's chooser; elsewhere zenity (GNOME and most
        // desktops), else kdialog (KDE). The saved format follows the name's
        // extension, which the Windows dialog's filter sets.
        let extension = if png { "png" } else { format.extension() };
        let default = format!("{name}.{extension}");
        let path = unix_dialog(save, &default)?;
        if !save {
            return Some((path, 1));
        }
        let typed = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let known = typed.and_then(|e| Format::ALL.iter().position(|f| f.extension() == e));
        match known {
            Some(i) if !png => Some((path, i as u32 + 1)),
            Some(_) => Some((path, 1)),
            None => Some((format!("{path}.{extension}"), format.filter_index())),
        }
    }
}

/// The path a system chooser returns (`save`: a new file named `default`
/// to start with), or None when it is cancelled or no chooser is installed.
#[cfg(not(windows))]
fn unix_dialog(save: bool, default: &str) -> Option<String> {
    let patterns = format!("{OPEN_FILTER};{IMPORT_FILTER}").replace(';', " ");
    let choosers: Vec<(&str, Vec<String>)> = if cfg!(target_os = "macos") {
        let script = if save {
            "on run argv\nPOSIX path of (choose file name with prompt \"Save as\" default name (item 1 of argv))\nend run"
        } else {
            "on run argv\nPOSIX path of (choose file with prompt \"Open a picture or a vector file\")\nend run"
        };
        vec![(
            "osascript",
            vec!["-e".into(), script.into(), default.into()],
        )]
    } else if save {
        vec![
            (
                "zenity",
                vec![
                    "--file-selection".into(),
                    "--save".into(),
                    "--title=Save as".into(),
                    format!("--filename={default}"),
                ],
            ),
            ("kdialog", vec!["--getsavefilename".into(), default.into()]),
        ]
    } else {
        vec![
            (
                "zenity",
                vec![
                    "--file-selection".into(),
                    "--title=Open a picture or a vector file".into(),
                    format!("--file-filter=Pictures and vector files | {patterns}"),
                    "--file-filter=All files | *".into(),
                ],
            ),
            (
                "kdialog",
                vec![
                    "--getopenfilename".into(),
                    ".".into(),
                    format!("{patterns}|Pictures and vector files"),
                ],
            ),
        ]
    };
    for (program, args) in choosers {
        let Ok(mut child) = std::process::Command::new(program)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        let mut stdout = child.stdout.take()?;
        if let Ok(mut open) = OPEN_DIALOG.lock() {
            *open = Some(child);
        }
        let mut text = String::new();
        let read = std::io::Read::read_to_string(&mut stdout, &mut text);
        if let Some(mut child) = OPEN_DIALOG.lock().ok().and_then(|mut open| open.take()) {
            let _ = child.wait();
        }
        read.ok()?;
        let path = text.trim_end_matches(['\n', '\r']).to_owned();
        return (!path.is_empty()).then_some(path);
    }
    None
}
