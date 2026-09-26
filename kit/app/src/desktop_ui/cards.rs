//! Part of `desktop_ui`: the header, the rail and its cards.

use super::*;

impl Desktop {
    pub(super) fn header(&mut self, ctx: &egui::Context, actions: &mut Actions) {
        let family = self.title_family.clone();
        egui::TopBottomPanel::top("header")
            .frame(bar_frame(9, true))
            .show_separator_line(false)
            .show(ctx, |ui| {
                let idle = self.idle();
                ui.horizontal(|ui| {
                    let full = ui.max_rect();
                    let gap = 8.;
                    // What the toolbar needs besides the picture's name: the
                    // three icon buttons, Convert, the two view chips, the
                    // Appearance button and the gaps. In a narrow window the
                    // app's name gives way first, then the picture's.
                    let fixed = 36. * 3. + 112. + 96. + 70. + 36. + gap * 8. + 6.;
                    let title = 30. + 10. + 110. + 12.;
                    let show_title = full.width() - title - fixed >= 160.;
                    let lead = if show_title { title } else { 30. + 12. };
                    let name_width = (full.width() * 0.24)
                        .clamp(140., 360.)
                        .min(full.width() - lead - fixed - 8.)
                        .max(80.);
                    ui.spacing_mut().item_spacing.x = 10.;
                    if let Some(logo) = &self.logo {
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            logo.id(),
                            Vec2::splat(30.),
                        )));
                    }
                    if show_title {
                        ui.label(
                            RichText::new("VectorMagik")
                                .size(17.)
                                .family(family.clone())
                                .color(pal().text),
                        );
                    }
                    // The toolbar sits centred in the window, not in what is
                    // left beside the title: Open, the picture's name, Convert,
                    // Save, Close and the two view chips as one group.
                    let group_width = 36. * 3. + 112. + name_width + 96. + 70. + gap * 7.;
                    let start = (full.center().x - group_width / 2.).max(ui.cursor().min.x + 12.);
                    ui.add_space((start - ui.cursor().min.x).max(0.));
                    ui.spacing_mut().item_spacing.x = gap;
                    if icon_button(ui, icon::OPEN, idle)
                        .on_hover_text(format!(
                            "Open an image  ({})",
                            ctx.format_shortcut(&SC_OPEN)
                        ))
                        .clicked()
                    {
                        actions.open = true;
                    }
                    // The picture's name where a document window shows its
                    // title (a typed path field before September 25, 2026;
                    // pictures come from Open or a drop), its path on hover.
                    let has_image = self.raster.is_some();
                    let name = std::path::Path::new(&self.loaded_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .filter(|_| has_image);
                    let (text, color) = match &name {
                        Some(name) => (name.as_str(), pal().text),
                        None => ("No image", pal().faint),
                    };
                    ui.allocate_ui_with_layout(
                        Vec2::new(name_width, 32.),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(name_width);
                            let label = ui.add(
                                egui::Label::new(RichText::new(text).size(13.5).color(color))
                                    .truncate(),
                            );
                            if name.is_some() && !platform::IN_BROWSER {
                                label.on_hover_text(&self.loaded_path);
                            }
                        },
                    );
                    let can_convert =
                        idle && self.raster.is_some() && self.path == self.loaded_path;
                    if self.worker.is_some() {
                        // While converting, the button stops the conversion.
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{}  Cancel", icon::CLOSE))
                                        .color(pal().text),
                                )
                                .stroke(Stroke::new(1_f32, pal().warn))
                                .corner_radius(CornerRadius::same(16))
                                .min_size(Vec2::new(112., 32.)),
                            )
                            .on_hover_text(
                                "Stop this conversion; its result is discarded and your edits \
                                 stay for the next Convert.  (Esc)",
                            )
                            .clicked()
                        {
                            actions.cancel = true;
                        }
                    } else if self.convert_button(ui, ctx, can_convert) {
                        actions.convert = true;
                    }
                    let can_save = (self.document.is_some() || self.foreign.is_some()) && idle;
                    let save = icon_button(ui, icon::SAVE, can_save)
                        .on_hover_text(format!(
                            "Save as SVG, PDF or EPS, {}  ({})",
                            if platform::IN_BROWSER {
                                "as a download"
                            } else {
                                "or drag the file out"
                            },
                            ctx.format_shortcut(&SC_SAVE)
                        ))
                        .on_disabled_hover_text(if self.document.is_some() {
                            "Waiting for the conversion, dialog or save to finish"
                        } else {
                            "Convert an image first"
                        });
                    self.save_anchor = save.rect;
                    if save.clicked() {
                        actions.save = true;
                    }
                    if icon_button(ui, icon::CLOSE, idle && has_image)
                        .on_hover_text(format!(
                            "Close the image  ({})",
                            ctx.format_shortcut(&SC_CLOSE)
                        ))
                        .clicked()
                    {
                        actions.close = true;
                    }
                    ui.add_space(6.);
                    if choice_width(ui, "Side by side", self.view == View::SideBySide, 96.)
                        .on_hover_text("The source and the vector next to each other")
                        .clicked()
                    {
                        self.view = View::SideBySide;
                    }
                    if choice_width(ui, "Overlay", self.view == View::Overlay, 70.)
                        .on_hover_text("One picture: B shows the bitmap, V the vector")
                        .clicked()
                    {
                        self.view = View::Overlay;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = icon_button(ui, icon::APPEARANCE, true)
                            .on_hover_text("Appearance: the look, light or dark");
                        self.appearance_anchor = button.rect;
                        if button.clicked() {
                            self.appearance_open = !self.appearance_open;
                        }
                    });
                });
            });
    }

    /// The Convert button, lit when it can run; true when clicked.
    fn convert_button(&self, ui: &mut egui::Ui, ctx: &egui::Context, can_convert: bool) -> bool {
        let convert = if can_convert {
            egui::Button::new(
                RichText::new(format!("{}  Convert", icon::CONVERT))
                    .color(pal().on_accent)
                    .strong(),
            )
            .fill(pal().accent)
            .stroke(Stroke::new(1_f32, pal().accent))
            .corner_radius(CornerRadius::same(16))
        } else {
            egui::Button::new(
                RichText::new(format!("{}  Convert", icon::CONVERT)).color(pal().text),
            )
            .corner_radius(CornerRadius::same(16))
        };
        ui.add_enabled(can_convert, convert.min_size(Vec2::new(112., 32.)))
            .on_hover_text(format!(
                "Convert the loaded image  ({})",
                ctx.format_shortcut(&SC_CONVERT)
            ))
            .on_disabled_hover_text(if self.errand.is_some() || self.stopping.is_some() {
                "Waiting for the dialog, the save or the cancelled conversion"
            } else if self.raster.is_some() && self.path != self.loaded_path {
                "Press Enter to load the path you typed first"
            } else {
                "Open an image first"
            })
            .clicked()
    }

    pub(super) fn rail(&mut self, ctx: &egui::Context) {
        let family = self.title_family.clone();
        let panel = egui::SidePanel::left("controls")
            .frame(egui::Frame::new().inner_margin(Margin {
                left: 12,
                right: SHADOW_REACH,
                top: 12 - SHADOW_REACH,
                bottom: 10,
            }))
            .resizable(true)
            .default_width(296.)
            .width_range(250.0..=400.)
            .show_separator_line(false)
            .show(ctx, |ui| {
                egui::TopBottomPanel::bottom("rail-hints")
                    .frame(egui::Frame::new().inner_margin(Margin {
                        left: 4,
                        right: 0,
                        top: 6,
                        bottom: 2,
                    }))
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        shortcuts(
                            ui,
                            &[
                                (ctx.format_shortcut(&SC_OPEN), "Open"),
                                (ctx.format_shortcut(&SC_CONVERT), "Convert"),
                                (ctx.format_shortcut(&SC_SAVE), "Save"),
                                ("Ctrl+Scroll".to_owned(), "Zoom"),
                                (ctx.format_shortcut(&SC_UNDO), "Undo"),
                                (ctx.format_shortcut(&SC_REDO), "Redo"),
                            ],
                        );
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        // egui's own bar widens the moment the pointer
                        // touches it, and it sits beside the rail's resize
                        // handle; this one waits (`rail_scroll_bar`).
                        let out = egui::ScrollArea::vertical()
                            .id_salt("rail-scroll")
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                            )
                            .show(ui, |ui| {
                                // The cards' shadows paint into the rail's
                                // margins beside the list, which clips at
                                // the cards' own edges otherwise.
                                let clip = ui.clip_rect();
                                ui.set_clip_rect(egui::Rect::from_min_max(
                                    egui::pos2(clip.min.x - 12., clip.min.y),
                                    egui::pos2(clip.max.x + f32::from(SHADOW_REACH), clip.max.y),
                                ));
                                self.rail_cards(ui, &family)
                            });
                        self.rail_scroll_bar(
                            ui,
                            out.id,
                            out.state,
                            out.content_size,
                            out.inner_rect,
                        );
                    });
            });
        rail_grip(ctx, panel.response.rect);
    }

    /// The rail's scroll bar: a thin line at rest that widens and takes
    /// drags and clicks only once the pointer has rested on it for
    /// `RAIL_BAR_DELAY`, so a pointer passing over it on the way to the
    /// rail's resize handle never grabs it. Dragging the thumb scrolls; a
    /// click on the track pages.
    fn rail_scroll_bar(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        mut state: egui::scroll_area::State,
        content: Vec2,
        inner: egui::Rect,
    ) {
        let view = inner.height();
        let range = content.y - view;
        if range <= 0.5 {
            self.rail_bar_since = None;
            return;
        }
        let track = egui::Rect::from_min_max(
            egui::pos2(inner.right() - 10., inner.top() + 2.),
            egui::pos2(inner.right(), inner.bottom() - 2.),
        );
        let bar_id = id.with("delayed-bar");
        let ctx = ui.ctx().clone();
        let dragging = ctx.is_being_dragged(bar_id);
        let over = ui.rect_contains_pointer(track);
        let now = ctx.input(|i| i.time);
        if over || dragging {
            self.rail_bar_since.get_or_insert(now);
        } else {
            self.rail_bar_since = None;
        }
        let rested = self
            .rail_bar_since
            .map(|since| Duration::from_secs_f64((now - since).max(0.)));
        let ready = dragging || rested.is_some_and(|t| t >= RAIL_BAR_DELAY);
        if let (Some(rested), false) = (rested, ready) {
            ctx.request_repaint_after(RAIL_BAR_DELAY.saturating_sub(rested));
        }
        let wide = ctx.animate_bool_responsive(bar_id, ready);
        let width = egui::lerp(3.0..=8.0, wide);
        let thumb_len = (view / content.y * track.height()).clamp(28., track.height());
        let travel = (track.height() - thumb_len).max(1.);
        let top = track.top() + state.offset.y / range * travel;
        let thumb = egui::Rect::from_min_max(
            egui::pos2(track.right() - 1. - width, top),
            egui::pos2(track.right() - 1., top + thumb_len),
        );
        let mut offset = state.offset.y;
        if ready {
            let response = ui.interact(track, bar_id, egui::Sense::click_and_drag());
            if response.dragged() {
                offset += response.drag_delta().y * range / travel;
            } else if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if pointer.y < thumb.top() {
                        offset -= view;
                    } else if pointer.y > thumb.bottom() {
                        offset += view;
                    }
                }
            }
        }
        let color = if dragging {
            pal().accent
        } else if ready {
            pal().dim
        } else {
            faded(pal().faint, 0.6)
        };
        ui.painter().rect_filled(thumb, width / 2., color);
        let offset = offset.clamp(0., range);
        if offset != state.offset.y {
            state.offset.y = offset;
            state.store(&ctx, id);
            ctx.request_repaint();
        }
    }

    pub(super) fn rail_cards(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        // Room above the first card and below the last for their shadows,
        // at the list's top and scrolled to its end.
        ui.add_space(f32::from(SHADOW_REACH));
        ui.spacing_mut().item_spacing.y = 10.;
        self.conversion_card(ui, family);
        self.curves_card(ui, family);
        self.nodes_card(ui, family);
        self.shapes_card(ui, family);
        self.sticker_card(ui, family);
        self.advanced_card(ui, family);
        self.licence_card(ui, family);
        ui.add_space(1.);
    }

    /// The original's advanced dialog: segmentation complexity, contour
    /// smoothness and curve complexity from 1 to 12 and its sharp-corner
    /// detection, replacing the preset at the next conversion.
    pub(super) fn advanced_card(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        let idle = self.worker.is_none();
        let mut open = !self.collapsed[5];
        // Off, the card shows the settings the conversion runs anyway (a
        // blended logo at high quality runs 11, 3, 6, not the dialog's
        // 5, 6, 6), the ones switching it on starts from.
        if !self.advanced_on {
            self.seed_sliders();
        }
        let values = format!(
            "{} \u{00B7} {} \u{00B7} {}{}",
            self.sliders.segmentation,
            self.sliders.smoothness,
            self.sliders.curves,
            if self.sliders.corners {
                ""
            } else {
                " \u{00B7} no corners"
            }
        );
        let summary = if self.advanced_on {
            values
        } else if self.pending_advanced().flatten().is_some() {
            format!("Preset \u{00B7} {values}")
        } else {
            "Preset".to_owned()
        };
        let on = self.advanced_on;
        card(
            ui,
            family,
            icon::SLIDERS,
            "Advanced",
            &mut open,
            &summary,
            !on,
            |ui| {
                ui.add_enabled_ui(idle, |ui| {
                    let switched = toggle_row(ui, &mut self.advanced_on, "Use the sliders", None)
                        .on_hover_text(
                            "The original program's advanced mode: its three sliders replace the \
                     image type's preset (the type still decides anti-aliasing and photo \
                     seams). Starts from the settings the picture would get anyway, which \
                     the heading shows while this is off.",
                        )
                        .changed();
                    if switched && self.advanced_on {
                        self.seed_sliders();
                    }
                    reveal(ui, "advanced-sliders", self.advanced_on, |ui| {
                        ui.add_space(2.);
                        for (label, value, hint) in [
                            (
                                "Detail",
                                &mut self.sliders.segmentation,
                                "How finely the image is cut into regions. 12 is the most \
                                 detail; photographs start there.",
                            ),
                            (
                                "Smoothness",
                                &mut self.sliders.smoothness,
                                "How strongly the outlines between regions are smoothed \
                                 before curves are drawn along them.",
                            ),
                            (
                                "Curve fit",
                                &mut self.sliders.curves,
                                "How closely the curves follow the smoothed outlines; higher \
                                 means more curve pieces.",
                            ),
                        ] {
                            egui::Sides::new().show(
                                ui,
                                |ui| {
                                    ui.label(RichText::new(label).size(12.5).color(pal().dim));
                                },
                                |ui| {
                                    ui.label(
                                        RichText::new(value.to_string())
                                            .size(12.5)
                                            .color(pal().text),
                                    );
                                },
                            );
                            ui.spacing_mut().slider_width = ui.available_width();
                            ui.add(egui::Slider::new(value, 1..=12).show_value(false))
                                .on_hover_text(hint);
                        }
                        ui.add_space(4.);
                        toggle_row(ui, &mut self.sliders.corners, "Sharp corners", None)
                            .on_hover_text(
                                "The original's corner detection: off skips its middle \
                             smoothing phase and rounds every join.",
                            );
                    });
                    if self.advanced_stale() && !self.conversion_stale() && self.worker.is_none() {
                        ui.label(
                            RichText::new(self.stale_hint())
                                .size(11.5)
                                .color(pal().warn),
                        );
                    }
                });
            },
        );
        self.collapsed[5] = !open;
    }

    /// A die-cut sticker around the shapes: a border hugging them, a rim
    /// outside it, an optional shadow, and a one-click cut of the background
    /// shapes so the outline follows the object.
    pub(super) fn sticker_card(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        let mut open = !self.collapsed[4];
        let summary = if self.sticker_on {
            format!(
                "{:.0} + {:.0} px{}",
                self.sticker.border,
                self.sticker.edge,
                if self.sticker.shadow {
                    " \u{00B7} shadow"
                } else {
                    ""
                }
            )
        } else {
            "Off".to_owned()
        };
        let on = self.sticker_on;
        card(
            ui,
            family,
            icon::STICKER,
            "Sticker",
            &mut open,
            &summary,
            !on,
            |ui| {
                let mut changed = toggle_row(ui, &mut self.sticker_on, "Cut a sticker", None)
                    .on_hover_text(
                        "Paints a border around the outside of everything in the vector, with a \
                 rim outside it like a die-cut sticker, and grows the picture so the \
                 outline fits. The shapes themselves do not change; the outline is \
                 saved with them.",
                    )
                    .changed();
                let mut cut = false;
                let shown = self.sticker_on;
                reveal(ui, "sticker-settings", shown, |ui| {
                    ui.add_space(2.);
                    let longest = self
                        .raster
                        .as_ref()
                        .map_or(250., |r| r.width.max(r.height) as f64);
                    let widest = (longest * 0.1).clamp(16., vector_rebuild::sticker::MAX_WIDTH);
                    for (index, label, hint) in [
                        (
                            0,
                            "Border",
                            "The line hugging the shapes, in source pixels. 0 leaves only the rim.",
                        ),
                        (
                            1,
                            "Rim",
                            "The band outside the border, in source pixels: the white edge of a \
                         sticker. 0 leaves only the border.",
                        ),
                    ] {
                        let (width, rgb) = if index == 0 {
                            (&mut self.sticker.border, &mut self.sticker.border_rgb)
                        } else {
                            (&mut self.sticker.edge, &mut self.sticker.edge_rgb)
                        };
                        let custom = &mut self.sticker_custom[index];
                        egui::Sides::new().show(
                            ui,
                            |ui| {
                                ui.label(RichText::new(label).size(12.5).color(pal().dim));
                            },
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 6.;
                                changed |= color_choice(ui, label, rgb, custom);
                                ui.label(
                                    RichText::new(format!("{width:.0} px"))
                                        .size(12.5)
                                        .color(pal().text),
                                );
                            },
                        );
                        ui.spacing_mut().slider_width = ui.available_width();
                        let mut value = *width;
                        let slider = ui
                            .add(
                                egui::Slider::new(&mut value, 0.0..=widest)
                                    .step_by(1.)
                                    .show_value(false),
                            )
                            .on_hover_text(hint);
                        if slider.changed() && value != *width {
                            *width = value;
                            changed = true;
                        }
                    }
                    ui.add_space(2.);
                    changed |= toggle_row(ui, &mut self.sticker.shadow, "Drop shadow", None)
                        .on_hover_text(
                            "A hard shadow a little down and right of the sticker, so a white \
                         rim still shows on a white page.",
                        )
                        .changed();
                    ui.add_space(4.);
                    let can_cut = self.document.is_some() && self.border_opaque;
                    if ui
                        .add_enabled(
                            can_cut,
                            egui::Button::new(RichText::new("Cut out background").size(12.))
                                .corner_radius(CornerRadius::same(10)),
                        )
                        .on_hover_text(
                            "Remove the shapes of the color that fills the picture's edge, so \
                         the outline hugs the object instead of the rectangle. They go with \
                         the deleted shapes: Restore deleted in the Shapes card brings them \
                         back.",
                        )
                        .on_disabled_hover_text(if self.document.is_none() {
                            "Convert an image first."
                        } else {
                            "The picture's edge is transparent already."
                        })
                        .clicked()
                    {
                        cut = true;
                    }
                    let note = if self.sticker.border + self.sticker.edge <= 0. {
                        "Give the border or the rim a width.".to_owned()
                    } else {
                        let margin = self.sticker.margin();
                        format!(
                            "Picture grows by {margin:.0} px on every side{}.",
                            if self.border_opaque && self.document.is_some() {
                                " \u{00B7} cut the background first"
                            } else {
                                ""
                            }
                        )
                    };
                    ui.add(
                        egui::Label::new(RichText::new(note).size(11.5).color(pal().faint)).wrap(),
                    );
                });
                if cut {
                    self.cut_background();
                } else if changed {
                    self.reapply();
                }
            },
        );
        self.collapsed[4] = !open;
    }

    pub(super) fn conversion_card(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        let idle = self.worker.is_none();
        let mut open = !self.collapsed[0];
        let summary = match (self.automatic, self.detected) {
            (true, Some(d)) => format!(
                "Auto \u{00B7} {} \u{00B7} {:?}",
                crate::auto::category_name(d.category),
                d.quality
            ),
            (true, None) => "Auto settings".to_owned(),
            (false, _) => format!(
                "{} \u{00B7} {:?}",
                crate::auto::category_name(self.options.category),
                self.options.quality
            ),
        };
        card(
            ui,
            family,
            icon::SETTINGS,
            "Conversion",
            &mut open,
            &summary,
            false,
            |ui| {
                ui.add_enabled_ui(idle, |ui| {
                    toggle_row(ui, &mut self.automatic, "Auto settings", None).on_hover_text(
                        "Estimated locally from colors, edges and resolution. Turn off to \
                     choose the image type and source quality.",
                    );
                    ui.add_space(4.);
                    if self.automatic {
                        // What Auto found, once there is a picture to look at.
                        if let Some(d) = self.detected {
                            let text = format!(
                                "{} \u{00B7} {:?} quality",
                                crate::auto::category_name(d.category),
                                d.quality
                            );
                            inset(ui, |ui| {
                                ui.label(RichText::new(text).size(12.5).color(pal().dim));
                            });
                        }
                    } else {
                        ui.label(RichText::new("Image type").size(12.).color(pal().dim));
                        egui::ComboBox::from_id_salt("category")
                            .width(ui.available_width())
                            .selected_text(crate::auto::category_name(self.options.category))
                            .show_ui(ui, |ui| {
                                for category in [
                                    ImageCategory::AntiAliasedArtwork,
                                    ImageCategory::AliasedArtwork,
                                    ImageCategory::Photograph,
                                ] {
                                    ui.selectable_value(
                                        &mut self.options.category,
                                        category,
                                        crate::auto::category_name(category),
                                    );
                                }
                            });
                        ui.add_space(4.);
                        ui.label(RichText::new("Source quality").size(12.).color(pal().dim));
                        egui::ComboBox::from_id_salt("quality")
                            .width(ui.available_width())
                            .selected_text(format!("{:?}", self.options.quality))
                            .show_ui(ui, |ui| {
                                for quality in [Quality::High, Quality::Medium, Quality::Low] {
                                    ui.selectable_value(
                                        &mut self.options.quality,
                                        quality,
                                        format!("{quality:?}"),
                                    );
                                }
                            });
                    }
                    if self.effective_category() == Some(ImageCategory::Photograph) {
                        ui.add_space(4.);
                        toggle_row(
                            ui,
                            &mut self.options.overlap_opaque_photos,
                            "Reduce export seams",
                            None,
                        )
                        .on_hover_text(
                            "Opaque photographs only: a half-pixel same-color overlap hides \
                         renderer seams between fills.",
                        );
                    }
                    ui.add_space(6.);
                    labelled_row(ui, "Colors", |ui| {
                        let shown = match self.prep.colors {
                            None => "All".to_owned(),
                            Some(n) => n.to_string(),
                        };
                        egui::ComboBox::from_id_salt("colors")
                            .width(72.)
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.prep.colors, None, "All");
                                for n in COLOR_CHOICES {
                                    ui.selectable_value(
                                        &mut self.prep.colors,
                                        Some(n),
                                        n.to_string(),
                                    );
                                }
                            })
                            .response
                            .on_hover_text(
                                "Limit the image to this many colors before tracing; the \
                                 colors are found in the image itself. All keeps every \
                                 color the engine finds.",
                            );
                    });
                    ui.add_space(2.);
                    labelled_row(ui, "Background", |ui| {
                        ui.add_enabled_ui(self.has_alpha, |ui| {
                            ui.spacing_mut().item_spacing.x = 6.;
                            let custom = self
                                .prep
                                .background
                                .is_some_and(|c| c != [255, 255, 255] && c != [0, 0, 0]);
                            if custom {
                                let mut rgb = self.background_custom;
                                if ui.color_edit_button_srgb(&mut rgb).changed() {
                                    self.background_custom = rgb;
                                    self.prep.background = Some(rgb);
                                }
                            }
                            let shown = match self.prep.background {
                                None => "Keep",
                                Some([255, 255, 255]) => "White",
                                Some([0, 0, 0]) => "Black",
                                Some(_) => "Custom",
                            };
                            let mut choice = shown;
                            egui::ComboBox::from_id_salt("background")
                                .width(72.)
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    for option in ["Keep", "White", "Black", "Custom"] {
                                        ui.selectable_value(&mut choice, option, option);
                                    }
                                })
                                .response
                                .on_hover_text(
                                    "Keep leaves the transparent parts transparent. White, \
                                     Black or a custom color flatten the image onto that \
                                     color first, so the vector gets a background shape.",
                                )
                                .on_disabled_hover_text("The source has no transparency.");
                            if choice != shown {
                                self.prep.background = match choice {
                                    "White" => Some([255, 255, 255]),
                                    "Black" => Some([0, 0, 0]),
                                    "Custom" => Some(self.background_custom),
                                    _ => None,
                                };
                            }
                        });
                    });
                });
                more_options(ui, "conversion", |ui| {
                    ui.add_enabled_ui(idle, |ui| {
                        toggle_row(
                            ui,
                            &mut self.options.optional_optimizer,
                            "Smooth joins",
                            None,
                        )
                        .on_hover_text(
                            "Experimental. Curve pieces meet without a kink, for a little \
                             less accuracy (one extra fitting step). The original program \
                             shipped with it off; off is the original result.",
                        );
                        if let Some(note) = self.raw_document.as_deref().and_then(optimizer_note) {
                            ui.label(RichText::new(note).size(11.5).color(pal().dim));
                        }
                    });
                    // Not greyed while a conversion runs: switching it off is
                    // how to stop the next one starting by itself.
                    toggle_row(ui, &mut self.auto_convert, "Convert automatically", None)
                        .on_hover_text(
                            "Once the image has been converted, changing a setting converts \
                             it again by itself, a moment after the last change; a \
                             conversion still running with the old settings is stopped. \
                             Opening an image never converts it.",
                        );
                });
                if self.conversion_stale() && self.worker.is_none() {
                    ui.label(
                        RichText::new(self.stale_hint())
                            .size(11.5)
                            .color(pal().warn),
                    );
                }
            },
        );
        self.collapsed[0] = !open;
    }

    /// The line under a card whose settings the shown result was not made
    /// with.
    pub(super) fn stale_hint(&self) -> &'static str {
        if self.auto_convert && self.converted_inputs.is_some() && !self.held() {
            "Converting again in a moment\u{2026}"
        } else {
            "Convert again to apply."
        }
    }

    pub(super) fn curves_card(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        let mut open = !self.collapsed[1];
        let summary = format!(
            "{}{}",
            if self.simplify {
                format!("Simplify {:.2} px", self.simplify_tolerance)
            } else {
                "Exact curves".to_owned()
            },
            if self.straighten {
                " \u{00B7} straightened"
            } else {
                ""
            }
        );
        card(
            ui,
            family,
            icon::SEGMENTS,
            "Curves",
            &mut open,
            &summary,
            false,
            |ui| {
                let toggled = toggle_row(ui, &mut self.simplify, "Simplify curves", None)
                    .on_hover_text(
                        "Merges neighboring curve pieces wherever one curve stays within the \
                     tolerance of the engine's fit. Edges shared by two fills stay sealed; \
                     corners and junctions are kept. Off shows the engine's exact output.",
                    )
                    .changed();
                let mut moved = false;
                let mut auto = false;
                let simplify = self.simplify;
                reveal(ui, "simplify-tolerance", simplify, |ui| {
                    ui.add_space(2.);
                    labelled_row(ui, "Tolerance", |ui| {
                        ui.spacing_mut().item_spacing.x = 8.;
                        ui.label(
                            RichText::new(format!("{:.2} px", self.simplify_tolerance))
                                .size(12.5)
                                .color(pal().text),
                        );
                        let label = if self.auto_pending {
                            "Auto\u{2026}"
                        } else {
                            "Auto"
                        };
                        if ui
                            .add_enabled(
                                !self.auto_pending && self.raw_document.is_some(),
                                egui::Button::new(RichText::new(label).size(12.))
                                    .min_size(Vec2::new(44., 22.))
                                    .corner_radius(CornerRadius::same(11)),
                            )
                            .on_hover_text(
                                "Pick the largest tolerance that keeps the traced picture: \
                                 fewer nodes, no visible change.",
                            )
                            .on_disabled_hover_text(if self.auto_pending {
                                "Picking the tolerance\u{2026}"
                            } else {
                                "Convert an image first"
                            })
                            .clicked()
                        {
                            auto = true;
                        }
                    });
                    ui.spacing_mut().slider_width = ui.available_width();
                    let slider = ui
                        .add(
                            egui::Slider::new(&mut self.simplify_tolerance, SIMPLIFY_RANGE)
                                .logarithmic(true)
                                .show_value(false),
                        )
                        .on_hover_text(
                            "How far, in source pixels, a merged curve may stray from the \
                         traced curves. Larger removes more nodes and smooths the small kinks \
                         left between them; circles stay round. The vector follows as you \
                         drag.",
                        );
                    moved = slider.changed();
                });
                let mut regular_toggled = false;
                let mut shapes_toggled = false;
                let mut straight_toggled = false;
                let mut straight_moved = false;
                let mut straight_auto = false;
                more_options(ui, "curves", |ui| {
                    regular_toggled =
                        toggle_row(ui, &mut self.regularize, "True lines and circles", None)
                            .on_hover_text(
                                "A run of pieces that all lie within 0.8 source pixels of one \
                     straight line is drawn as that line, and a run on one circle as \
                     circle arcs (a full ring becomes a four-piece circle), so traced \
                     edges stop wobbling. Shared edges stay sealed; nodes where three \
                     fills meet never move.",
                            )
                            .changed();
                    // Photographs are left as traced: the switch would do nothing.
                    let photo = self.effective_category() == Some(ImageCategory::Photograph);
                    shapes_toggled = !photo
                        && toggle_row(
                            ui,
                            &mut self.primitives,
                            "True shapes from the pixels",
                            None,
                        )
                        .on_hover_text(
                            "A closed outline that the picture's own pixels show to be a \
                             circle, an ellipse, a rectangle or a rounded rectangle is drawn \
                             as that shape, when it explains those pixels at least as well as \
                             the traced outline: small dots and rounded squares come out round \
                             and square-on. Artwork only; photographs are left as traced.",
                        )
                        .changed();
                    straight_toggled = toggle_row(
                        ui,
                        &mut self.straighten,
                        "Straighten lines",
                        None,
                    )
                    .on_hover_text(
                        "A curve piece that never bows further than the tolerance from the \
                     straight line between its ends is drawn as that line, and lines within \
                     three degrees of horizontal or vertical are snapped to it (no node moves \
                     more than a pixel), so pixel edges come out straight and corners square. \
                     Right-click a node for the same by hand.",
                    )
                    .changed();
                    let straighten = self.straighten;
                    reveal(ui, "straighten-bow", straighten, |ui| {
                        egui::Sides::new().show(
                            ui,
                            |ui| {
                                ui.label(RichText::new("Bow").size(12.5).color(pal().dim));
                            },
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 8.;
                                let shown = if self.straighten_auto {
                                    format!("Auto \u{00B7} {:.2} px", self.straighten_tolerance)
                                } else {
                                    format!("{:.2} px", self.straighten_tolerance)
                                };
                                ui.label(RichText::new(shown).size(12.5).color(pal().text));
                                if ui
                                    .add_enabled(
                                        !self.straighten_auto,
                                        egui::Button::new(RichText::new("Auto").size(12.))
                                            .min_size(Vec2::new(44., 22.))
                                            .corner_radius(CornerRadius::same(11)),
                                    )
                                    .on_hover_text(
                                        "The bow measured best for the kind of picture: 0.2 px on \
                                     smooth-edged artwork, 0.65 on pixel art, 1.2 on photos.",
                                    )
                                    .on_disabled_hover_text(
                                        "Auto is on; moving the slider turns it off",
                                    )
                                    .clicked()
                                {
                                    straight_auto = true;
                                }
                            },
                        );
                        ui.spacing_mut().slider_width = ui.available_width();
                        straight_moved = ui
                        .add(
                            egui::Slider::new(&mut self.straighten_tolerance, STRAIGHTEN_RANGE)
                                .logarithmic(true)
                                .show_value(false),
                        )
                        .on_hover_text(
                            "How far, in source pixels, a curve may bow from the straight line \
                         between its ends and still be drawn as that line.",
                        )
                        .changed();
                    });
                });
                ui.add_space(2.);
                let counts = match (self.node_counts, self.simplify) {
                    (Some((before, after)), true) if after != before => {
                        format!("{before} \u{2192} {after} nodes")
                    }
                    (Some((before, _)), _) => format!("{before} nodes"),
                    (None, _) => String::new(),
                };
                let note = if counts.is_empty() {
                    String::new()
                } else if self.derive_pending.is_some() {
                    format!("{counts} \u{00B7} updating\u{2026}")
                } else {
                    counts
                };
                if !note.is_empty() {
                    ui.label(RichText::new(note).size(11.5).color(pal().faint));
                }
                if straight_moved {
                    self.straighten_auto = false;
                }
                if straight_auto {
                    self.straighten_auto = true;
                    if let Some(bow) = self.auto_bow() {
                        self.straighten_tolerance = bow;
                    }
                }
                if auto {
                    self.request_derive(DeriveJob::Auto);
                } else if toggled
                    || moved
                    || regular_toggled
                    || shapes_toggled
                    || straight_toggled
                    || straight_moved
                    || straight_auto
                {
                    self.reapply();
                }
            },
        );
        self.collapsed[1] = !open;
    }

    pub(super) fn nodes_card(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        let mut open = !self.collapsed[2];
        let count = self.shown_counts().0.map(|(_, shown)| shown);
        // Hidden, the card shows only its switch; the heading still says
        // there are hand edits to restore (the review of September 23, 2026).
        let by_hand = self.rounded.len()
            + self.straightened.len()
            + self.moves_shown()
            + self.deleted_nodes.len();
        let summary = match (self.nodes, count) {
            (true, Some(n)) => format!("{n} shown"),
            (true, None) => "Shown".to_owned(),
            (false, _) if by_hand > 0 => format!("Hidden \u{00B7} {by_hand} edited by hand"),
            (false, _) => "Hidden".to_owned(),
        };
        let shown = self.nodes;
        card(
            ui,
            family,
            icon::NODES,
            "Nodes",
            &mut open,
            &summary,
            !shown,
            |ui| {
                toggle_row(ui, &mut self.nodes, "Show curve nodes", None)
                    .on_hover_text("Marks every anchor node on the vector.  (N)");
                let shown = self.nodes;
                reveal(ui, "node-edits", shown, |ui| {
                    let mut done = Vec::new();
                    let (rounded, squared, moved, deleted) = (
                        self.rounded.len(),
                        self.straightened.len(),
                        self.moves_shown(),
                        self.deleted_nodes.len(),
                    );
                    let plural = |n: usize| if n == 1 { "" } else { "s" };
                    if rounded > 0 {
                        done.push(format!("{rounded} rounded"));
                    }
                    if squared > 0 {
                        done.push(format!("{squared} squared"));
                    }
                    if moved > 0 {
                        done.push(format!("{moved} moved"));
                    }
                    if deleted > 0 {
                        done.push(format!("{deleted} deleted"));
                    }
                    let note = match (rounded, squared, moved, deleted) {
                        (0, 0, 0, 0) => "Click a node to round its corner, drag it to move it, \
                                         right-click to delete it and more. Ctrl+Z undoes."
                            .to_owned(),
                        (1, 0, 0, 0) => {
                            "1 corner rounded. Click it again to restore it.".to_owned()
                        }
                        (n, 0, 0, 0) => {
                            format!("{n} corners rounded. Click one again to restore it.")
                        }
                        (0, 0, m, 0) => format!(
                            "{m} node{} moved. Right-click one to put it back.",
                            plural(m)
                        ),
                        (0, 0, 0, d) => format!(
                            "{d} node{} deleted. Ctrl+Z brings the last back.",
                            plural(d)
                        ),
                        _ => format!("{} by hand.", done.join(" \u{00B7} ")),
                    };
                    ui.add(
                        egui::Label::new(RichText::new(note).size(11.5).color(pal().faint)).wrap(),
                    );
                    let button = |text: &str| {
                        egui::Button::new(RichText::new(text).size(12.))
                            .corner_radius(CornerRadius::same(10))
                    };
                    if rounded > 1 || squared > 0 || moved > 0 || deleted > 0 {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(6., 6.);
                            if rounded > 1 && ui.add(button("Restore all corners")).clicked() {
                                self.rounded.clear();
                                self.reapply();
                            }
                            if squared > 0 && ui.add(button("Undo squaring")).clicked() {
                                self.straightened.clear();
                                self.reapply();
                            }
                            if moved > 0
                                && ui
                                    .add(button(if moved == 1 {
                                        "Put the node back"
                                    } else {
                                        "Put all nodes back"
                                    }))
                                    .on_hover_text("Every moved node back where the trace put it.")
                                    .clicked()
                            {
                                self.put_all_back();
                            }
                            if deleted > 0
                                && ui
                                    .add(button(if deleted == 1 {
                                        "Bring the node back"
                                    } else {
                                        "Bring deleted nodes back"
                                    }))
                                    .on_hover_text("Every deleted node back in the drawing.")
                                    .clicked()
                            {
                                self.restore_deleted_nodes();
                            }
                        });
                    }
                });
            },
        );
        self.collapsed[2] = !open;
    }

    pub(super) fn shapes_card(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        let mut open = !self.collapsed[3];
        let summary = match (
            self.shapes_mode,
            self.deleted.len(),
            self.prep.recolors.len(),
        ) {
            (true, _, _) => "Selecting".to_owned(),
            (false, 0, 0) => String::new(),
            (false, r, 0) => format!("{r} deleted"),
            (false, 0, m) => format!("{m} merged"),
            (false, r, m) => format!("{r} deleted \u{00B7} {m} merged"),
        };
        card(
            ui,
            family,
            icon::SHAPES,
            "Shapes",
            &mut open,
            &summary,
            false,
            |ui| {
                // Merging and undoing merges convert again, so they wait for
                // a running conversion, dialog or save.
                let idle = self.idle();
                let mut changed = false;
                let selecting = self.shapes_mode;
                let label = if selecting {
                    "Done selecting"
                } else {
                    "Select shapes"
                };
                let button =
                    egui::Button::new(RichText::new(label).size(12.5).color(if selecting {
                        pal().on_accent
                    } else {
                        pal().text
                    }))
                    .fill(if selecting { pal().accent } else { pal().chip })
                    .stroke(Stroke::new(
                        1_f32,
                        if selecting {
                            pal().accent
                        } else {
                            pal().border
                        },
                    ))
                    .corner_radius(CornerRadius::same(13))
                    .min_size(Vec2::new(ui.available_width(), 26.));
                if ui
                    .add_enabled(self.document.is_some(), button)
                    .on_hover_text(
                        "Click shapes on the vector to select them, then Delete or Merge; \
                     right-click a shape for more. Click again when done.",
                    )
                    .on_disabled_hover_text("Convert an image first.")
                    .clicked()
                {
                    self.shapes_mode = !self.shapes_mode;
                    changed = true;
                }
                let mut delete = false;
                let mut merge = false;
                if self.shapes_mode {
                    ui.add_space(2.);
                    changed |= ui
                        .checkbox(
                            &mut self.delete_on_click,
                            RichText::new("Delete on click").size(12.5),
                        )
                        .on_hover_text(
                            "Clicking a shape removes it at once instead of selecting it.",
                        )
                        .changed();
                    let resolved = self.resolve_selection();
                    if !resolved.is_empty() {
                        let mut colors: Vec<&str> =
                            resolved.iter().map(|i| i.color.as_str()).collect();
                        colors.dedup();
                        let distinct = {
                            let mut set = std::collections::BTreeSet::new();
                            for c in &colors {
                                set.insert(c.to_ascii_lowercase());
                            }
                            set.len()
                        };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.;
                            ui.label(
                                RichText::new(format!("{} selected", resolved.len()))
                                    .size(12.5)
                                    .color(pal().text),
                            );
                            if pill(ui, "Delete")
                                .on_hover_text("Remove the selected shapes from the vector.")
                                .clicked()
                            {
                                delete = true;
                            }
                            if ui
                                .add_enabled(distinct >= 2 && idle, pill_widget("Merge"))
                                .on_hover_text(
                                    "Recolor the other selected shapes with the first one's \
                                 color in the image and convert again, so they trace as \
                                 one shape.",
                                )
                                .on_disabled_hover_text(if distinct >= 2 {
                                    "Wait for the conversion, dialog or save to finish"
                                } else {
                                    "Select shapes of two different colors"
                                })
                                .clicked()
                            {
                                merge = true;
                            }
                        });
                    }
                }
                if changed {
                    self.selected.clear();
                    self.shape_menu = None;
                }
                let removed = self.deleted.len();
                let merges = self.prep.recolors.len();
                let note = match (removed, merges) {
                    (0, 0) => String::new(),
                    (r, 0) => format!("{r} shape{} deleted.", if r == 1 { "" } else { "s" }),
                    (0, m) => format!("{m} merge{} applied.", if m == 1 { "" } else { "s" }),
                    (r, m) => format!("{r} deleted \u{00B7} {m} merged."),
                };
                if !note.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(note).size(11.5).color(pal().faint)).wrap(),
                    );
                }
                let mut undelete = false;
                let mut unmerge = false;
                if removed > 0 || merges > 0 {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.;
                        if removed > 0
                            && ui
                                .add(
                                    egui::Button::new(RichText::new("Restore deleted").size(12.))
                                        .corner_radius(CornerRadius::same(10)),
                                )
                                .clicked()
                        {
                            undelete = true;
                        }
                        if merges > 0
                            && ui
                                .add_enabled(
                                    idle,
                                    egui::Button::new(RichText::new("Undo merges").size(12.))
                                        .corner_radius(CornerRadius::same(10)),
                                )
                                .on_hover_text("Convert the unmerged image again.")
                                .on_disabled_hover_text("Wait for the conversion to finish.")
                                .clicked()
                        {
                            unmerge = true;
                        }
                    });
                }
                if delete {
                    self.delete_selected();
                }
                if merge {
                    self.merge_selected(None);
                }
                if undelete {
                    self.deleted.clear();
                    self.reapply();
                }
                if unmerge {
                    self.prep.recolors.clear();
                    self.start();
                }
            },
        );
        self.collapsed[3] = !open;
    }
}

/// The Conversion card's line under "Smooth joins" when the converted
/// document ran the optional pass: whether its step stands, or the improved
/// defaults' guard put the plain curves back because the step raised the
/// fitting objective (round two of the Opus 5.5 review: the outcome was
/// computed and never shown).
pub(super) fn optimizer_note(document: &VectorDocument) -> Option<&'static str> {
    if !document.optional_optimizer {
        return None;
    }
    Some(
        match (document.optimizer_kept, document.optimizer_objective) {
            (true, _) => "Smooth joins applied.",
            (false, Some(_)) => "Smooth joins would have worsened the fit; kept the plain curves.",
            // Asked for, and no step came back: the pass stopped (a singular
            // system, a zero-length tangent) or had nothing to join. The note
            // said nothing then (round three of the Opus 5.5 review).
            (false, None) => "Smooth joins could not run on this picture; kept the plain curves.",
        },
    )
}
