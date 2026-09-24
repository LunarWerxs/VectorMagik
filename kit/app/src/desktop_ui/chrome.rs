//! Part of `desktop_ui`: the status bar, popups and menus.

use super::*;

impl Desktop {
    /// The footer: the status on the left; on the right the result stats
    /// (each one clickable) and the zoom controls.
    pub(super) fn status_bar(&mut self, ctx: &egui::Context) {
        let (node_counts, segment_counts, palette) = self.shown_counts();
        let stats = match (node_counts, segment_counts) {
            (Some((_, nodes)), Some((_, segments))) => Some((nodes, segments, palette)),
            _ => None,
        };
        let raw_stats = match (node_counts, segment_counts) {
            (Some((nodes, _)), Some((segments, _))) => Some((nodes, segments)),
            _ => None,
        };
        // The picture opened, also when the engine traced a scaled copy.
        let raster_size = self
            .raster
            .as_ref()
            .map(|r| self.source_size.unwrap_or((r.width, r.height)));
        let output = self.output_size().filter(|_| self.enlarged());
        let mut toggle_nodes = false;
        let mut toggle_simplify = false;
        let mut open: Option<StatPopup> = None;
        let mut drawn = [false; 2];
        let mut zoom_to: Option<f32> = None;
        let zoom_range = self.zoom_range();
        let mut fit = false;
        egui::TopBottomPanel::bottom("status")
            .frame(bar_frame(6, false))
            .show_separator_line(pal().separators)
            .show(ctx, |ui| {
                let width = ui.available_width();
                let (show_detail, show_size) = (width >= 820., width >= 600.);
                egui::Sides::new().spacing(16.).shrink_left().show(
                    ui,
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.;
                        let (color, text_color) = match self.status_kind {
                            StatusKind::Info => (pal().dim, pal().text),
                            StatusKind::Busy => (pal().accent, pal().text),
                            StatusKind::Done => (pal().ok, pal().text),
                            StatusKind::Error => (pal().err, pal().err),
                        };
                        match self.status_kind {
                            StatusKind::Busy => {
                                ui.add(egui::Spinner::new().size(15.).color(pal().accent));
                            }
                            kind => {
                                let glyph = match kind {
                                    StatusKind::Done => icon::DONE,
                                    StatusKind::Error => icon::ERROR,
                                    _ => icon::INFO,
                                };
                                ui.label(RichText::new(glyph).size(15.).color(color));
                            }
                        }
                        let text = if self.status_kind == StatusKind::Busy {
                            format!(
                                "{}  {:.1} s",
                                self.status,
                                self.started.elapsed().as_secs_f64()
                            )
                        } else {
                            self.status.clone()
                        };
                        ui.add(egui::Label::new(RichText::new(&text).color(text_color)).truncate())
                            .on_hover_text(text);
                    },
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.;
                        if raster_size.is_none() {
                            return;
                        }
                        // Right-to-left: the zoom group sits at the far right.
                        ui.label(
                            RichText::new(format!("{:.0}%", self.fit * self.zoom * 100.))
                                .size(12.)
                                .color(pal().dim),
                        )
                        .on_hover_text(
                            "Zoom. Ctrl+scroll over a picture zooms around the pointer.",
                        );
                        if small_button(ui, "+")
                            .on_hover_text("Zoom in  (+)")
                            .clicked()
                        {
                            zoom_to = Some(self.zoom * ZOOM_STEP);
                        }
                        ui.spacing_mut().slider_width = 74.;
                        let mut zoom = self.zoom;
                        if ui
                            .add(
                                egui::Slider::new(&mut zoom, zoom_range.clone())
                                    .logarithmic(true)
                                    .show_value(false),
                            )
                            .changed()
                        {
                            zoom_to = Some(zoom);
                        }
                        if small_button(ui, "\u{2212}")
                            .on_hover_text("Zoom out  (-)")
                            .clicked()
                        {
                            zoom_to = Some(self.zoom / ZOOM_STEP);
                        }
                        if pill(ui, "1:1")
                            .on_hover_text("One source pixel per screen pixel  (1)")
                            .clicked()
                        {
                            zoom_to = Some(1. / self.fit);
                        }
                        if pill(ui, "Fit")
                            .on_hover_text("Fit the picture to its card  (F)")
                            .clicked()
                        {
                            fit = true;
                        }
                        ui.add_space(8.);
                        if let (Some((nodes, segments, colors)), true) = (stats, show_detail) {
                            let (raw_nodes, raw_segments) = raw_stats.unwrap_or((nodes, segments));
                            let tip = if self.nodes {
                                "Curve nodes are shown. Click to hide them.  (N)"
                            } else {
                                "Click to show the curve nodes.  (N)"
                            };
                            if stat(ui, icon::NODES, &format!("{nodes} nodes"), self.nodes)
                                .on_hover_text(tip)
                                .clicked()
                            {
                                toggle_nodes = true;
                            }
                            let tip = if self.simplify {
                                format!(
                                    "Simplified from the traced {raw_segments} segments and \
                                     {raw_nodes} nodes. Click to show the exact traced curves."
                                )
                            } else {
                                "The exact traced curves. Click to simplify them.".to_owned()
                            };
                            if stat(
                                ui,
                                icon::SEGMENTS,
                                &format!("{segments} segments"),
                                self.simplify,
                            )
                            .on_hover_text(tip)
                            .clicked()
                            {
                                toggle_simplify = true;
                            }
                            let colors_chip = stat(
                                ui,
                                icon::COLORS,
                                &format!("{colors} colors"),
                                self.stat_popup == Some(StatPopup::Colors),
                            )
                            .on_hover_text("Click for the palette; a swatch copies its value.");
                            self.stat_anchors[1] = colors_chip.rect;
                            drawn[1] = true;
                            // The palette is the shown document's: none
                            // while a conversion replaces it.
                            if colors_chip.clicked() && self.document.is_some() {
                                open = Some(StatPopup::Colors);
                            }
                        }
                        if let (Some((w, h)), true) = (raster_size, show_size) {
                            let text = match output {
                                Some((ow, oh)) => {
                                    format!("{w} \u{00D7} {h} \u{2192} {ow} \u{00D7} {oh} px")
                                }
                                None => format!("{w} \u{00D7} {h} px"),
                            };
                            let size_chip = stat(
                                ui,
                                icon::SIZE,
                                &text,
                                output.is_some() || self.stat_popup == Some(StatPopup::Size),
                            )
                            .on_hover_text(if output.is_some() {
                                "Source size and the pixel size the file is saved at. Click to change."
                            } else {
                                "Source size; the file is saved at this pixel size. Click to save larger."
                            });
                            self.stat_anchors[0] = size_chip.rect;
                            drawn[0] = true;
                            if size_chip.clicked() {
                                open = Some(StatPopup::Size);
                            }
                        }
                    },
                );
            });
        if let Some(zoom) = zoom_to {
            self.set_zoom(zoom);
        }
        if fit {
            self.set_zoom(1.);
            self.scroll = Vec2::ZERO;
        }
        if toggle_nodes {
            self.nodes = !self.nodes;
        }
        if toggle_simplify {
            self.simplify = !self.simplify;
            self.reapply();
        }
        // A popup whose chip is no longer drawn (the window got narrow) has
        // nothing to hang from.
        match self.stat_popup {
            Some(StatPopup::Size) if !drawn[0] => self.stat_popup = None,
            Some(StatPopup::Colors) if !drawn[1] => self.stat_popup = None,
            _ => {}
        }
        if let Some(which) = open {
            self.stat_popup = if self.stat_popup == Some(which) {
                None
            } else {
                Some(which)
            };
        }
    }

    /// The popups hanging off the footer stats.
    pub(super) fn stat_popups(&mut self, ctx: &egui::Context) {
        let Some(which) = self.stat_popup else {
            return;
        };
        let anchor = self.stat_anchors[match which {
            StatPopup::Size => 0,
            StatPopup::Colors => 1,
        }];
        let Some(unit_size) = self.unit_size() else {
            self.stat_popup = None;
            return;
        };
        let colors = match which {
            StatPopup::Colors => Some(self.palette.clone()),
            StatPopup::Size => None,
        };
        let family = self.title_family.clone();
        let mut open = true;
        let mut copied: Option<String> = None;
        let mut removed: Option<String> = None;
        anchored_popup(
            egui::Id::new("stat-popup").with(which as u8),
            ctx,
            anchor,
            egui::RectAlign::TOP_END,
        )
        .open_bool(&mut open)
        .show(|ui| match which {
            StatPopup::Size => {
                popup_heading(ui, 250., "Output size", 13.5, &family);
                self.size_controls(ui, unit_size);
                ui.label(
                    RichText::new(
                        "The saved SVG, PDF or EPS is declared at this size in pixels; \
                         the curves scale exactly. 1\u{00D7} is the source's pixel size.",
                    )
                    .size(11.5)
                    .color(pal().faint),
                );
            }
            StatPopup::Colors => {
                let colors = colors.unwrap_or_default();
                ui.set_width(230.);
                ui.spacing_mut().item_spacing.y = 8.;
                ui.label(
                    RichText::new(format!("{} colors", colors.len()))
                        .size(13.5)
                        .family(family.clone())
                        .color(pal().text),
                );
                let removable = colors.len() > 1 && self.idle();
                egui::ScrollArea::vertical()
                    .max_height(280.)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.;
                        for (hex, paths) in &colors {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.;
                                let (rect, response) =
                                    ui.allocate_exact_size(Vec2::splat(22.), egui::Sense::click());
                                let color = parse_hex(hex);
                                ui.painter().rect(
                                    rect,
                                    5.,
                                    color,
                                    Stroke::new(
                                        1_f32,
                                        if response.hovered() {
                                            pal().text
                                        } else {
                                            pal().border
                                        },
                                    ),
                                    StrokeKind::Inside,
                                );
                                if response.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                let noun = if *paths == 1 { "path" } else { "paths" };
                                if response
                                    .on_hover_text(format!("{paths} {noun}. Click to copy {hex}."))
                                    .clicked()
                                {
                                    ui.ctx().copy_text(hex.clone());
                                    copied = Some(hex.clone());
                                }
                                ui.label(RichText::new(hex).size(12.5).color(pal().text));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                        .add_enabled(
                                            removable,
                                            small_button_widget("\u{00D7}"),
                                        )
                                        .on_hover_text(
                                            "Drop this color: its shapes take the nearest \
                                             remaining color and the image converts again.",
                                        )
                                        .on_disabled_hover_text(if colors.len() > 1 {
                                            "Wait for the conversion, dialog or save to finish"
                                        } else {
                                            "The last color stays; convert with more colors first."
                                        })
                                        .clicked()
                                    {
                                        removed = Some(hex.clone());
                                    }
                                    },
                                );
                            });
                        }
                    });
                ui.label(
                    RichText::new(
                        "Fill colors of the vector, most used first. \u{00D7} drops a color.",
                    )
                    .size(11.5)
                    .color(pal().faint),
                );
            }
        });
        if let Some(hex) = copied {
            self.set_status(StatusKind::Done, format!("Copied {hex} to the clipboard."));
        }
        if let Some(hex) = removed {
            self.stat_popup = None;
            self.remove_color(&hex);
        }
        if !open {
            self.stat_popup = None;
        }
    }

    /// One-click scales and a typed width; the height follows the source.
    /// `(w, h)` is the saved size at 1x (`unit_size`), so every number shown
    /// is the one saved (round three of the Opus 5.5 review: the scaled
    /// copy's size showed while the opened picture's was saved).
    pub(super) fn size_controls(&mut self, ui: &mut egui::Ui, (w, h): (f32, f32)) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.;
            for scale in OUTPUT_SCALES {
                let selected = (self.output_scale - scale).abs() < 1e-3;
                if choice_width(ui, &format!("{scale:.0}\u{00D7}"), selected, 46.)
                    .on_hover_text(format!(
                        "{} \u{00D7} {} px",
                        (w * scale).round(),
                        (h * scale).round()
                    ))
                    .clicked()
                {
                    self.output_scale = scale;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            ui.label(RichText::new("Width").size(12.5).color(pal().dim));
            let mut width = (w * self.output_scale).round() as u32;
            if ui
                .add(
                    egui::DragValue::new(&mut width)
                        .range(OUTPUT_WIDTH_RANGE)
                        .speed(4.)
                        .suffix(" px"),
                )
                .on_hover_text("Drag or type a width; the height keeps the source's proportions.")
                .changed()
            {
                self.output_scale = width as f32 / w.max(1.);
            }
            let height = (h * self.output_scale).round() as u32;
            ui.label(
                RichText::new(format!("\u{00D7}  {height} px"))
                    .size(12.5)
                    .color(pal().dim),
            );
        });
    }

    /// Save: the format first, the size, then either the system dialog or a
    /// file to drag straight onto the desktop or into a folder.
    /// The Appearance popup under its header button: the look, and light or
    /// dark (a tab follows the page's switch instead).
    pub(super) fn appearance_popup(&mut self, ctx: &egui::Context) {
        if !self.appearance_open {
            return;
        }
        let family = self.title_family.clone();
        let mut open = true;
        anchored_popup(
            egui::Id::new("appearance-popup"),
            ctx,
            self.appearance_anchor,
            egui::RectAlign::BOTTOM_END,
        )
        .open_bool(&mut open)
        .show(|ui| {
            popup_heading(ui, 290., "Appearance", 14., &family);
            ui.label(RichText::new("Look").size(12.5).color(pal().dim));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                for look in Look::ALL {
                    if choice_width(ui, look.label(), self.look == look, 88.)
                        .on_hover_text(look.about())
                        .clicked()
                    {
                        self.look = look;
                    }
                }
            });
            if self.look != Look::Classic {
                ui.label(
                    RichText::new("Glass and Studio preview a new design.")
                        .size(11.5)
                        .color(pal().faint),
                );
            }
            ui.add_space(2.);
            ui.label(RichText::new("Light or dark").size(12.5).color(pal().dim));
            if platform::IN_BROWSER {
                ui.label(
                    RichText::new(
                        "Follows the light and dark switch at the top right of the page.",
                    )
                    .size(11.5)
                    .color(pal().faint),
                );
            } else {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.;
                    for theme in ThemeChoice::ALL {
                        if choice_width(ui, theme.label(), self.theme == theme, 88.)
                            .on_hover_text(match theme {
                                ThemeChoice::System => "Follow Windows' light or dark setting",
                                ThemeChoice::Dark => "Always dark",
                                ThemeChoice::Light => "Always light",
                            })
                            .clicked()
                        {
                            self.theme = theme;
                        }
                    }
                });
            }
        });
        self.appearance_open = open;
    }

    pub(super) fn save_popup(&mut self, ctx: &egui::Context) {
        if !self.save_open {
            return;
        }
        let Some(unit_size) = self.unit_size() else {
            self.save_open = false;
            return;
        };
        if self.document.is_none() && self.foreign.is_none() {
            self.save_open = false;
            return;
        }
        self.stage_export();
        let state = self.stage_state();
        if matches!(state, StageState::Writing) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let family = self.title_family.clone();
        let anchor = self.save_anchor;
        let mut open = true;
        let mut choose = false;
        let mut drag = false;
        anchored_popup(
            egui::Id::new("save-popup"),
            ctx,
            anchor,
            egui::RectAlign::BOTTOM,
        )
        .open_bool(&mut open)
        .show(|ui| {
            popup_heading(ui, 330., "Save vector", 14., &family);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6., 6.);
                for format in Format::ALL {
                    if choice_width(ui, format.label(), self.save_format == format, 42.)
                        .on_hover_text(format.hint())
                        .clicked()
                    {
                        self.save_format = format;
                    }
                }
            });
            ui.add_space(2.);
            self.size_controls(ui, unit_size);
            ui.add_space(2.);
            // The original's "Shape mode", with the app's stacked drawing
            // first: what the window shows.
            labelled_row(ui, "Shapes", |ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                let cut_plain = !self.save_stacked && !self.save_grouped;
                let cut_grouped = !self.save_stacked && self.save_grouped;
                if choice_width(ui, "Cut-outs", cut_plain, 64.)
                    .on_hover_text(
                        "Every shape cut out of the ones below it, nothing overlapping, \
                         in one list (the original's \"cut-outs in shapes below\").",
                    )
                    .clicked()
                {
                    (self.save_stacked, self.save_grouped) = (false, false);
                }
                if choice_width(ui, "By color", cut_grouped, 64.)
                    .on_hover_text(
                        "Cut-outs grouped by colour, one group per colour: the original's \
                         default, handy for recolouring in an editor.",
                    )
                    .clicked()
                {
                    (self.save_stacked, self.save_grouped) = (false, true);
                }
                if choice_width(ui, "Stacked", self.save_stacked, 64.)
                    .on_hover_text(
                        "As shown: each color runs a hair under the edges of the colors \
                         drawn after it, so no thin background line shows between two \
                         colors.",
                    )
                    .clicked()
                {
                    self.save_stacked = true;
                }
            });
            toggle_row(ui, &mut self.save_stroke, "Stroke shape boundaries", None).on_hover_text(
                "Also draw every shape's outline in its own colour, a hair wide, so \
                 viewers that leave faint seams between neighbouring shapes show \
                 none (the original's stroking mode).",
            );
            if self.save_format == Format::Dxf {
                labelled_row(ui, "DXF curves", |ui| {
                    ui.spacing_mut().item_spacing.x = 4.;
                    use crate::export::DxfMode;
                    for (mode, label, hint) in [
                        (
                            DxfMode::CoarseLines,
                            "Few lines",
                            "Lines only, curves as fewer lines: a smaller file.",
                        ),
                        (
                            DxfMode::FineLines,
                            "Many lines",
                            "Lines only, curves as more lines: a larger file.",
                        ),
                        (
                            DxfMode::Splines,
                            "Curves",
                            "Lines and spline curves (the original's default).",
                        ),
                    ] {
                        if choice_width(ui, label, self.save_dxf == mode, 64.)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            self.save_dxf = mode;
                        }
                    }
                });
            }
            ui.add_space(4.);
            let name = self.export_name();
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), 58.),
                egui::Sense::click_and_drag(),
            );
            let ready = matches!(state, StageState::Ready(_));
            let hovered = ready && (response.hovered() || response.dragged());
            ui.painter().rect(
                rect,
                10.,
                if hovered {
                    pal().surface_high
                } else {
                    pal().chip
                },
                Stroke::new(1_f32, if hovered { pal().accent } else { pal().border }),
                StrokeKind::Inside,
            );
            ui.painter().text(
                rect.left_center() + Vec2::new(16., 0.),
                Align2::LEFT_CENTER,
                icon::FILE,
                FontId::proportional(26.),
                if hovered { pal().accent } else { pal().dim },
            );
            ui.painter().text(
                rect.left_center() + Vec2::new(56., -9.),
                Align2::LEFT_CENTER,
                &name,
                FontId::proportional(13.5),
                if ready { pal().text } else { pal().dim },
            );
            // PDF and EPS take the exporter a moment; the card says so and
            // only drags once the file is there.
            let (note, color) = match &state {
                StageState::Ready(_) if platform::IN_BROWSER => {
                    ("Click to download".to_owned(), pal().dim)
                }
                StageState::Ready(_) => (
                    "Drag onto your desktop or into a folder".to_owned(),
                    pal().dim,
                ),
                StageState::Writing => (
                    format!("Writing the {}\u{2026}", self.save_format.label()),
                    pal().dim,
                ),
                StageState::Failed(error) => {
                    let line = error.lines().next().unwrap_or("");
                    let short: String = line.chars().take(44).collect();
                    if short.len() < line.len() {
                        (format!("{short}\u{2026}"), pal().err)
                    } else {
                        (short, pal().err)
                    }
                }
            };
            ui.painter().text(
                rect.left_center() + Vec2::new(56., 10.),
                Align2::LEFT_CENTER,
                note,
                FontId::proportional(11.5),
                color,
            );
            if let StageState::Failed(error) = &state {
                response.clone().on_hover_text(error);
            }
            if ready {
                grab_cursor(ui, &response);
            }
            if response.drag_started() || (platform::IN_BROWSER && response.clicked()) {
                drag = true;
            }
            if !platform::IN_BROWSER
                && ui
                    .add(
                        egui::Button::new(RichText::new("Choose a location\u{2026}").size(13.))
                            .min_size(Vec2::new(ui.available_width(), 30.))
                            .corner_radius(CornerRadius::same(10)),
                    )
                    .on_hover_text("The system save dialog, set to this format.")
                    .clicked()
            {
                choose = true;
            }
        });
        self.save_open = open;
        // Nothing starts while a dialog or a save is out: a second dialog
        // errand would replace the first and lose its answer.
        let idle = self.idle();
        if drag && idle {
            self.drag_out();
        } else if choose && idle {
            self.save_open = false;
            self.save_dialog();
        }
    }

    /// The menu of a right-clicked shape: delete it, select it, or merge the
    /// selection into its colour.
    pub(super) fn shape_menu_ui(&mut self, ctx: &egui::Context) {
        let Some((shape, at)) = self.shape_menu.clone() else {
            return;
        };
        let selected = self.is_selected(&shape);
        let idle = self.idle();
        let others = self
            .selected
            .iter()
            .filter(|s| !s.color.eq_ignore_ascii_case(&shape.color))
            .count();
        let mut open = true;
        let mut action: Option<ShapeAction> = None;
        menu_popup(
            egui::Id::new("shape-menu").with((shape.at.x.to_bits(), shape.at.y.to_bits())),
            ctx,
            at,
        )
        .open_bool(&mut open)
        .show(|ui| {
            ui.set_min_width(200.);
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Shape \u{00B7} {}", shape.color))
                        .size(11.5)
                        .color(pal().dim),
                )
                .selectable(false),
            );
            if ui.button("Delete shape").clicked() {
                action = Some(ShapeAction::Delete);
            }
            if ui
                .button(if selected { "Deselect" } else { "Select" })
                .clicked()
            {
                action = Some(ShapeAction::ToggleSelect);
            }
            if others > 0 {
                ui.separator();
                let label = format!("Merge {others} selected into {}", shape.color);
                if ui
                    .add_enabled(idle, egui::Button::new(label))
                    .on_hover_text(
                        "Recolor the selected shapes of other colors with this one's \
                         color in the image and convert again.",
                    )
                    .clicked()
                {
                    action = Some(ShapeAction::MergeInto);
                }
            }
        });
        match action {
            Some(ShapeAction::Delete) => self.delete_shape(shape),
            Some(ShapeAction::ToggleSelect) => self.toggle_selected(shape),
            Some(ShapeAction::MergeInto) => {
                if !self.is_selected(&shape) {
                    self.selected.push(shape.clone());
                }
                self.merge_selected(Some(shape.color));
            }
            _ => {}
        }
        if !open || action.is_some() {
            self.shape_menu = None;
        }
    }

    /// The menu of a right-clicked node: the reach for its rounding,
    /// restoring the corner, straightening and squaring it, putting it back
    /// where the trace had it when it was moved, and deleting it two ways.
    pub(super) fn node_menu_ui(&mut self, ctx: &egui::Context) {
        let Some((node, at)) = self.node_menu else {
            return;
        };
        let current = self.rounding_of(&node);
        // Straightening runs before the moves, so it knows a moved node by
        // where it was.
        let base = self.base_position(&node);
        let moved = self.moved_from(&node).is_some();
        let squared = self.straightened.iter().any(|p| same_point(p, &base));
        let refusal = self.deletion_refusal(&node);
        let mut open = true;
        let mut pick: Option<Option<Reach>> = None;
        let mut square: Option<bool> = None;
        let mut put_back = false;
        let mut delete: Option<bool> = None;
        menu_popup(
            egui::Id::new("node-menu").with((node.x.to_bits(), node.y.to_bits())),
            ctx,
            at,
        )
        .open_bool(&mut open)
        .show(|ui| {
            ui.set_min_width(200.);
            let heading = match (current, moved) {
                (Some(reach), false) => format!("Rounded corner \u{00B7} {}", reach.label()),
                (Some(reach), true) => {
                    format!("Moved \u{00B7} rounded corner \u{00B7} {}", reach.label())
                }
                (None, true) => "Moved node".to_owned(),
                (None, false) => "Corner node".to_owned(),
            };
            ui.add(
                egui::Label::new(RichText::new(heading).size(11.5).color(pal().dim))
                    .selectable(false),
            );
            for reach in Reach::ALL {
                if ui
                    .selectable_label(
                        current == Some(reach),
                        format!("{} rounding", reach.label()),
                    )
                    .on_hover_text(reach.hint())
                    .clicked()
                {
                    pick = Some(Some(reach));
                }
            }
            if current.is_some() {
                ui.separator();
                if ui.button("Restore corner").clicked() {
                    pick = Some(None);
                }
            }
            ui.separator();
            if squared {
                if ui.button("Undo straighten and square").clicked() {
                    square = Some(false);
                }
            } else if ui
                .button("Straighten and square")
                .on_hover_text(
                    "Both pieces meeting here become straight lines, and snap to horizontal \
                     or vertical when they are within twenty degrees of it, so the corner \
                     comes out square.",
                )
                .clicked()
            {
                square = Some(true);
            }
            if moved {
                ui.separator();
                if ui
                    .button("Put back")
                    .on_hover_text("Returns the node to where the trace put it.")
                    .clicked()
                {
                    put_back = true;
                }
            }
            ui.separator();
            let why = refusal.unwrap_or_default();
            if ui
                .add_enabled(refusal.is_none(), egui::Button::new("Delete node"))
                .on_hover_text(
                    "Takes the node out: one piece runs from the node before to the node \
                     after, keeping their outer handles.",
                )
                .on_disabled_hover_text(why)
                .clicked()
            {
                delete = Some(false);
            }
            if ui
                .add_enabled(
                    refusal.is_none(),
                    egui::Button::new("Delete, keep the shape"),
                )
                .on_hover_text(
                    "Takes the node out and fits one curve to the two pieces, so the \
                     outline stays where one curve can follow it.",
                )
                .on_disabled_hover_text(why)
                .clicked()
            {
                delete = Some(true);
            }
        });
        match pick {
            Some(Some(reach)) => self.round_node(node, reach),
            Some(None) => self.restore_node(node),
            None => {}
        }
        match square {
            Some(true) => {
                self.straightened.push(base);
                self.reapply();
            }
            Some(false) => {
                self.straightened.retain(|p| !same_point(p, &base));
                self.reapply();
            }
            None => {}
        }
        if put_back {
            self.put_back(node);
        }
        if let Some(keep_shape) = delete {
            self.delete_node(node, keep_shape);
        }
        if !open || pick.is_some() || square.is_some() || put_back || delete.is_some() {
            self.node_menu = None;
        }
    }
}
