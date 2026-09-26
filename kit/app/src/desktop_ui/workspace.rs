//! Part of `desktop_ui`: the canvas, tiles, minimap and empty state.

use super::*;

impl Desktop {
    pub(super) fn workspace(&mut self, ctx: &egui::Context) {
        if self.foreign.is_some() {
            self.foreign_workspace(ctx);
            return;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(Margin {
                left: 6,
                right: 12,
                top: 12,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                // Before a picture: one compact welcome card, not two cards
                // the height of the window (the owner, September 26, 2026:
                // "the large drop box ... might be too big").
                if self.raster.is_none() {
                    match self.welcome(ui) {
                        Start::Browse => self.open_dialog(),
                        Start::Sample => self.open_sample(ui.ctx()),
                        Start::Nothing => {}
                    }
                    return;
                }
                let avail = ui.available_size();
                let aspect = self
                    .raster
                    .as_ref()
                    .map(|r| r.width as f32 / r.height.max(1) as f32)
                    .unwrap_or(1.);
                let gap = 12.;
                if self.view == View::Overlay {
                    let vector =
                        self.peek.unwrap_or(self.overlay_vector) && self.vector_texture().is_some();
                    if !vector {
                        self.node_drag = None;
                    }
                    self.card(ui, vector);
                } else if stacked_layout(avail, aspect) {
                    let height = ((avail.y - gap) / 2.).max(80.);
                    ui.allocate_ui(Vec2::new(avail.x, height), |ui| {
                        ui.set_min_size(Vec2::new(avail.x, height));
                        self.card(ui, false);
                    });
                    ui.add_space(gap);
                    ui.allocate_ui(Vec2::new(avail.x, height), |ui| {
                        ui.set_min_size(Vec2::new(avail.x, height));
                        self.card(ui, true);
                    });
                } else {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.columns(2, |columns| {
                        self.card(&mut columns[0], false);
                        self.card(&mut columns[1], true);
                    });
                }
            });
    }

    pub(super) fn card(&mut self, ui: &mut egui::Ui, vector: bool) {
        let family = self.title_family.clone();
        let has_vector = self.vector_texture().is_some();
        card_frame().show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 14,
                    right: 14,
                    top: 10,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = Vec2::new(8., 4.);
                    let overlay = self.view == View::Overlay;
                    let can_show_input = !vector
                        && self.working_source.is_some()
                        && self
                            .converted_prep
                            .as_ref()
                            .is_some_and(|p| !p.is_identity());
                    let mut switch: Option<bool> = None;
                    let mut held: Option<bool> = None;
                    let mut flip_input = false;
                    egui::Sides::new().spacing(16.).shrink_right().show(
                        ui,
                        |ui| {
                            let (glyph, title) = match (vector, overlay) {
                                (true, _) => (icon::VECTOR, "Vector"),
                                (false, _) => (icon::SOURCE, "Original"),
                            };
                            ui.label(RichText::new(glyph).size(14.).color(pal().accent));
                            ui.label(
                                RichText::new(title)
                                    .size(14.)
                                    .family(family.clone())
                                    .color(pal().text),
                            );
                            if overlay {
                                ui.add_space(6.);
                                let hold = self.hold_compare;
                                let bitmap = choice_width(ui, "B  Original", !vector, 84.)
                                    .on_hover_text(if hold {
                                        "Hold to see the original  (hold B)"
                                    } else {
                                        "Show the original  (B)"
                                    });
                                let vector_button = ui
                                    .add_enabled(
                                        has_vector,
                                        egui::Button::new(
                                            RichText::new("V  Vector").size(12.).color(if vector {
                                                pal().on_accent
                                            } else {
                                                pal().text
                                            }),
                                        )
                                        .fill(if vector { pal().accent } else { pal().chip })
                                        .stroke(Stroke::new(
                                            1_f32,
                                            if vector { pal().accent } else { pal().border },
                                        ))
                                        .corner_radius(CornerRadius::same(11))
                                        .min_size(Vec2::new(74., 22.)),
                                    )
                                    .on_hover_text(if hold {
                                        "Hold to see the vector  (hold V)"
                                    } else {
                                        "Show the vector  (V)"
                                    })
                                    .on_disabled_hover_text("Convert the image first");
                                if hold {
                                    // Shown while held; letting go puts
                                    // back the picture chosen before.
                                    held = if bitmap.is_pointer_button_down_on() {
                                        Some(false)
                                    } else if vector_button.is_pointer_button_down_on() {
                                        Some(true)
                                    } else {
                                        None
                                    };
                                } else if bitmap.clicked() {
                                    switch = Some(false);
                                } else if vector_button.clicked() {
                                    switch = Some(true);
                                }
                                ui.checkbox(
                                    &mut self.hold_compare,
                                    RichText::new("Hold").size(12.).color(pal().dim),
                                )
                                .on_hover_text(
                                    "Hold to compare: pressing Bitmap or Vector (or holding \
                                         B or V) shows that picture only while held, then goes \
                                         back. Untick to switch pictures with a click.",
                                );
                            }
                            if can_show_input
                                && choice_width(ui, "As traced", self.show_engine_input, 90.)
                                    .on_hover_text(
                                        "Show the image as it was traced (color limit, \
                                             background and merges applied) instead of the \
                                             original.",
                                    )
                                    .clicked()
                            {
                                flip_input = true;
                            }
                        },
                        |ui| {
                            let detail = if vector {
                                // The counts live in the footer's chips.
                                if self.worker.is_some() {
                                    "Converting\u{2026}".into()
                                } else {
                                    String::new()
                                }
                            } else {
                                match &self.raster {
                                    // The name is the toolbar's title and the size
                                    // the footer's chip: only what was done to the
                                    // picture before tracing.
                                    Some(_) => {
                                        let mut text = Vec::new();
                                        if let (Some(prep), true) =
                                            (&self.converted_prep, self.working.is_some())
                                        {
                                            if let Some(n) = prep.colors {
                                                text.push(format!("{n} colors"));
                                            }
                                            if prep.background.is_some() {
                                                text.push("flattened".to_owned());
                                            }
                                            if !prep.recolors.is_empty() {
                                                text.push(format!(
                                                    "{} merged",
                                                    prep.recolors.len()
                                                ));
                                            }
                                        }
                                        text.join(" \u{00B7} ")
                                    }
                                    None => String::new(),
                                }
                            };
                            ui.add(
                                egui::Label::new(RichText::new(detail).size(12.).color(pal().dim))
                                    .truncate(),
                            );
                        },
                    );
                    if let Some(vector) = switch {
                        self.overlay_vector = vector;
                    }
                    if overlay && held != self.peek_button {
                        self.peek_button = held;
                        ui.ctx().request_repaint();
                    }
                    if flip_input {
                        self.show_engine_input = !self.show_engine_input;
                    }
                });
            self.card_body(ui, vector);
        });
    }

    pub(super) fn card_body(&mut self, ui: &mut egui::Ui, vector: bool) {
        let engine_input = !vector && self.show_engine_input && self.working_source.is_some();
        let texture = if vector {
            self.vector_texture()
        } else if engine_input {
            self.working_source.clone()
        } else {
            self.source.clone()
        };
        let (Some(texture), Some(raster)) = (texture, self.raster.as_ref()) else {
            if self.empty_state(ui, vector) && self.idle() && self.path == self.loaded_path {
                self.start();
            }
            return;
        };
        // The vector grows by the sticker's margin on every side; both cards
        // fit that larger picture so the two stay at one scale. Node and shape
        // coordinates are the document's, shifted by the margin on screen.
        let margin = self.picture_margin();
        let (source_w, source_h) = (raster.width as f32, raster.height as f32);
        let (w, h) = (source_w + 2. * margin, source_h + 2. * margin);
        let offset = if vector { margin } else { 0. };
        let editing_shapes = vector && self.shapes_mode && self.document.is_some();
        let islands = if editing_shapes {
            Some(self.current_islands())
        } else {
            None
        };
        let selected_islands: Vec<usize> = match &islands {
            Some(islands) => self
                .selected
                .iter()
                .filter_map(|s| find_shape(islands, s))
                .collect(),
            None => Vec::new(),
        };
        let viewport = ui.available_size();
        let fit = ((viewport.x - 24.) / w)
            .min((viewport.y - 24.) / h)
            .max(0.01);
        if (!vector || self.view == View::Overlay) && (self.fit - fit).abs() > 1e-6 {
            // The footer read the previous fit this frame; redraw at once.
            self.fit = fit;
            ui.ctx().request_repaint();
        }
        let scale = fit * self.zoom;
        let size = if vector {
            Vec2::new(w * scale, h * scale)
        } else {
            Vec2::new(source_w * scale, source_h * scale)
        };
        // While a conversion runs, the picture it replaces stays with its
        // nodes, which answer no pointer until the new result arrives.
        let inert = vector && self.document.is_none();
        let nodes = if vector && self.nodes && !inert {
            Some(self.current_nodes())
        } else if inert && self.nodes {
            self.stale().and_then(|r| r.nodes.clone())
        } else {
            None
        };
        // Magnified source pixels are shown unfiltered so the anti-aliased
        // edge the engine traced is visible; the vector gets a crisp tile.
        let sharp = if engine_input {
            &self.working_sharp
        } else {
            &self.source_sharp
        };
        let texture = match (sharp, vector, scale >= 2.) {
            (Some(sharp), false, true) => sharp.clone(),
            _ => texture,
        };
        let tile = if !vector {
            None
        } else if inert {
            self.stale().and_then(|r| r.tile.as_ref())
        } else {
            self.tile.as_ref()
        };
        let dropping = !vector && self.idle() && Self::files_hovering(ui.ctx());
        let smoothed: Vec<bool> = nodes
            .as_ref()
            .map(|nodes| {
                nodes
                    .iter()
                    .map(|n| self.rounding_of(n).is_some())
                    .collect()
            })
            .unwrap_or_default();
        // Rounded corners are cut out of the document; their marker stays
        // where the corner was, so they can be restored or changed.
        let ghosts: Vec<Point> = match &nodes {
            Some(nodes) => self
                .rounded
                .iter()
                .map(|r| r.at)
                .filter(|at| !nodes.iter().any(|n| same_point(n, at)))
                .collect(),
            None => Vec::new(),
        };
        let mut picked: Option<Point> = None;
        // The node under the press that may become a drag, and where a node
        // being dragged has been carried to on this frame.
        let press_origin = ui.input(|i| i.pointer.press_origin());
        let mut pressed_node: Option<Point> = None;
        let drag_preview: Option<NodeDrag> = self.node_drag.clone().filter(|_| vector);
        let mut drag_to: Option<Point> = None;
        let mut hovered_shape: Option<(usize, Point)> = None;
        let out = egui::ScrollArea::both()
            .id_salt(if vector {
                "vector-scroll"
            } else {
                "source-scroll"
            })
            .auto_shrink([false, false])
            .scroll_source(egui::containers::scroll_area::ScrollSource {
                drag: false,
                scroll_bar: true,
                mouse_wheel: true,
            })
            .scroll_offset(self.scroll)
            .show(ui, |ui| {
                let content = size.max(ui.available_size());
                let (outer, response) =
                    ui.allocate_exact_size(content, egui::Sense::click_and_drag());
                let rect = egui::Rect::from_center_size(outer.center(), size);
                let visible = rect.intersect(ui.clip_rect());
                let painter = ui.painter_at(rect);
                checkerboard(&painter, rect, visible);
                let uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
                painter.image(texture.id(), rect, uv, Color32::WHITE);
                if let Some(tile) = tile {
                    // The tile's pixels are the display's: `ppp` to a point.
                    let ratio = scale / (tile.key.scale * tile.key.ppp);
                    let min = rect.min + Vec2::new(tile.key.origin[0], tile.key.origin[1]) * scale;
                    let tile_size =
                        Vec2::new(tile.key.size[0] as f32, tile.key.size[1] as f32) * ratio;
                    painter.image(
                        tile.texture.id(),
                        egui::Rect::from_min_size(min, tile_size),
                        uv,
                        Color32::WHITE,
                    );
                }
                let to_screen = |p: &Point| {
                    rect.min
                        + Vec2::new((p.x as f32 + offset) * scale, (p.y as f32 + offset) * scale)
                };
                if let Some(islands) = &islands {
                    let pointer = response.hover_pos();
                    if let Some(pointer) = pointer {
                        let at = Point {
                            x: ((pointer.x - rect.min.x) / scale - offset) as f64,
                            y: ((pointer.y - rect.min.y) / scale - offset) as f64,
                        };
                        hovered_shape = shapes::island_at(islands, at).map(|i| (i, at));
                    }
                    for &index in &selected_islands {
                        paint_island_outline(
                            &painter,
                            &islands[index],
                            Stroke::new(2_f32, SHAPE_SELECTED),
                            scale,
                            &to_screen,
                        );
                    }
                    if let Some((index, _)) = hovered_shape {
                        if !selected_islands.contains(&index) {
                            paint_island_outline(
                                &painter,
                                &islands[index],
                                Stroke::new(1.5_f32, SHAPE_HOVER),
                                scale,
                                &to_screen,
                            );
                        }
                    }
                }
                if let Some(nodes) = &nodes {
                    let reach = visible.expand(4.);
                    let pointer = if editing_shapes || inert {
                        None
                    } else {
                        response.hover_pos()
                    };
                    let origin = if editing_shapes || inert {
                        None
                    } else {
                        press_origin
                    };
                    let mut best = f32::INFINITY;
                    let mut best_press = f32::INFINITY;
                    let mut consider = |p: &Point, screen: egui::Pos2| {
                        if let Some(pointer) = pointer {
                            let d = pointer.distance(screen);
                            if d <= NODE_PICK_RADIUS && d < best {
                                best = d;
                                picked = Some(*p);
                            }
                        }
                        if let Some(origin) = origin {
                            let d = origin.distance(screen);
                            if d <= NODE_PICK_RADIUS && d < best_press {
                                best_press = d;
                                pressed_node = Some(*p);
                            }
                        }
                    };
                    for (index, p) in nodes.iter().enumerate() {
                        let screen = to_screen(p);
                        consider(p, screen);
                        if !reach.contains(screen) {
                            continue;
                        }
                        node_marker(&painter, screen, smoothed[index]);
                    }
                    for p in &ghosts {
                        let screen = to_screen(p);
                        consider(p, screen);
                        if reach.contains(screen) {
                            node_marker(&painter, screen, true);
                        }
                    }
                    if let (Some(p), None) = (picked, &drag_preview) {
                        painter.circle_stroke(
                            to_screen(&p),
                            7.,
                            Stroke::new(1.5_f32, Color32::WHITE),
                        );
                    }
                }
                // A node being dragged: the pieces it pulls along drawn where
                // they will go, its old place ringed, the marker under the
                // pointer. The picture itself changes on the drop.
                if let Some(NodeDrag {
                    from,
                    pieces,
                    rounding,
                    ..
                }) = &drag_preview
                {
                    if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
                        let to = Point {
                            x: ((pointer.x - rect.min.x) / scale - offset) as f64,
                            y: ((pointer.y - rect.min.y) / scale - offset) as f64,
                        };
                        drag_to = Some(to);
                        let outline: Vec<NodePiece> = match rounding {
                            Some((reach, frame)) => {
                                vector_rebuild::nodes::rounded_at(pieces, *from, to, *reach, *frame)
                            }
                            None => pieces.iter().map(|p| p.moved(*from, to)).collect(),
                        };
                        for piece in outline {
                            paint_piece(
                                &painter,
                                piece,
                                Stroke::new(1.5_f32, pal().accent),
                                scale,
                                &to_screen,
                            );
                        }
                        painter.circle_stroke(to_screen(from), 4., Stroke::new(1_f32, pal().faint));
                        node_marker(&painter, to_screen(&to), rounding.is_some());
                        painter.circle_stroke(
                            to_screen(&to),
                            7.,
                            Stroke::new(1.5_f32, pal().accent),
                        );
                    }
                }
                ui.painter().rect_stroke(
                    rect.expand(1.),
                    0.,
                    Stroke::new(1_f32, pal().border),
                    StrokeKind::Inside,
                );
                response
            });
        let response = out.inner;
        let area = out.inner_rect;
        let padded = size.max(viewport);
        let max_scroll = (padded - viewport).max(Vec2::ZERO);
        let mut scroll = out.state.offset;
        let pannable = size.x > viewport.x + 1. || size.y > viewport.y + 1.;
        if let (Some(islands), true) = (&islands, response.hovered()) {
            if let Some((index, at)) = hovered_shape {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                let island = &islands[index];
                self.shape_interaction(ui, &response, island, at);
            }
        }
        // A drag that starts on a node carries the node, anywhere else it
        // pans; the drop moves the node and derives the picture again.
        if vector
            && !editing_shapes
            && self.node_drag.is_none()
            && response.drag_started_by(egui::PointerButton::Primary)
        {
            if let Some(node) = pressed_node {
                self.begin_node_drag(node);
            }
        }
        let carrying = vector && self.node_drag.is_some();
        if carrying {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let released = response.drag_stopped() || !ui.input(|i| i.pointer.primary_down());
            if let Some(drag) = &mut self.node_drag {
                if let Some(to) = drag_to {
                    drag.to = to;
                }
            }
            if released {
                if let Some(drag) = self.node_drag.take() {
                    self.move_node(drag.from, drag.to);
                }
            }
        } else if let Some(node) = picked {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            self.node_interaction(ui, &response, node);
        }
        if carrying {
            // The drag moves the node, not the view.
        } else if response.dragged() {
            scroll -= response.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if picked.is_none()
            && hovered_shape.is_none()
            && response.hovered()
            && (pannable || ui.input(|i| i.modifiers.ctrl))
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        self.scroll = scroll.clamp(Vec2::ZERO, max_scroll);
        self.zoom_at_pointer(ui, &response, area);
        if dropping {
            drop_highlight(ui, area, &self.title_family);
        }
        if vector {
            let cap = ui.ctx().input(|i| i.max_texture_side) as u32;
            let ppp = ui.ctx().pixels_per_point();
            if let Some(wait) = self.request_tile(scale, ppp, size, viewport, cap) {
                ui.ctx().request_repaint_after(wait);
            }
            if pannable {
                self.minimap(ui, &texture, area, size, viewport);
            }
        }
    }

    /// The hovered shape's tooltip, selection and menu.
    fn shape_interaction(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        island: &Island,
        at: Point,
    ) {
        let menu_open = self.shape_menu.is_some();
        let shape = Removal {
            color: island.color.clone(),
            at,
        };
        if !menu_open {
            let text = if self.delete_on_click {
                format!("{}  \u{00B7}  click to delete", island.color)
            } else if self.is_selected(&shape) {
                format!(
                    "{}  \u{00B7}  selected; click to deselect \u{00B7} right-click for more",
                    island.color
                )
            } else {
                format!(
                    "{}  \u{00B7}  click to select \u{00B7} right-click for more",
                    island.color
                )
            };
            pointer_tip(ui, "shape-tip", text);
        }
        if response.secondary_clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                self.shape_menu = Some((shape, at));
            }
        } else if response.clicked() && !menu_open {
            if self.delete_on_click {
                self.delete_shape(shape);
            } else {
                self.toggle_selected(shape);
            }
        }
    }

    /// The picked node's tooltip, rounding toggle and menu.
    fn node_interaction(&mut self, ui: &egui::Ui, response: &egui::Response, node: Point) {
        let menu_open = self.node_menu.is_some();
        if !menu_open {
            let moved = self.moved_from(&node).is_some();
            let text = match (self.rounding_of(&node), moved) {
                (Some(reach), _) => format!(
                    "Rounded ({}). Click to restore \u{00B7} drag to move \u{00B7} right-click \
                     for more",
                    reach.label()
                ),
                (None, true) => "Moved. Click to round \u{00B7} drag to move \u{00B7} \
                                 right-click to put it back"
                    .into(),
                (None, false) => "Click to round this corner \u{00B7} drag to move it \
                                  \u{00B7} right-click for more"
                    .into(),
            };
            pointer_tip(ui, "node-tip", text);
        }
        if response.secondary_clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                self.node_menu = Some((node, at));
            }
        } else if response.clicked() && !menu_open {
            self.toggle_node(node);
        }
    }

    /// Zoom by the input's zoom delta, keeping the picture under the pointer still.
    fn zoom_at_pointer(&mut self, ui: &egui::Ui, response: &egui::Response, area: egui::Rect) {
        let delta = ui.input(|i| i.zoom_delta());
        if delta != 1. && response.hovered() {
            let before = self.zoom;
            self.set_zoom(self.zoom * delta);
            let ratio = self.zoom / before;
            if let Some(pointer) = response.hover_pos() {
                let local = pointer - area.min;
                self.scroll = ((self.scroll + local) * ratio - local).max(Vec2::ZERO);
            }
        }
    }

    /// Ask the render thread for a crisp tile of the visible part of the
    /// vector whenever the display scale exceeds the base preview's, and only
    /// when the current tile does not already cover the view at that scale.
    /// A tile is at most `MAX_TILE_SIDE`, and `cap` (the GPU's limit), per side,
    /// in the display's pixels, `ppp` to a point. Where the view must rest
    /// first (`platform::TILE_REST`), how much longer before asking again.
    pub(super) fn request_tile(
        &mut self,
        scale: f32,
        ppp: f32,
        size: Vec2,
        viewport: Vec2,
        cap: u32,
    ) -> Option<Duration> {
        let (Some(shown), Some(preview), Some(raster)) = (&self.shown, &self.preview, &self.raster)
        else {
            return None;
        };
        let shown = Arc::clone(shown);
        let picture_width = raster.width as f32 + 2. * self.picture_margin();
        let base_scale = preview.size()[0] as f32 / picture_width.max(1.);
        let ppp = if ppp.is_finite() && ppp > 0. { ppp } else { 1. };
        // Display pixels per picture pixel: past the preview's, a tile.
        let pixels = scale * ppp;
        if pixels <= base_scale * 1.02 {
            self.tile = None;
            self.tile_pending = None;
            return None;
        }
        let padded = size.max(viewport);
        let pad = (padded - size) / 2.;
        let view_min = ((self.scroll - pad) / scale).max(Vec2::ZERO);
        let view_max = ((self.scroll - pad + viewport) / scale).min(size / scale);
        let side = MAX_TILE_SIDE.min(cap).max(1) as f32;
        // What one tile can hold, in picture pixels; a view wider than that
        // gets its first `reach` sharp and the rest from the preview.
        let reach = Vec2::splat((side - 2.) / pixels);
        let need_max = view_max.min(view_min + reach);
        let spare = ((reach - (need_max - view_min)) / 2.).max(Vec2::ZERO);
        let margin = ((view_max - view_min) * 0.25).min(spare);
        let want_min = (view_min - margin).max(Vec2::ZERO).floor();
        let want_max = (need_max + margin).min(size / scale).ceil();
        let covered = |key: &TileKey| {
            key.version == self.document_version
                && (key.scale - scale).abs() < 1e-3
                && (key.ppp - ppp).abs() < 1e-3
                && key.origin[0] <= view_min.x
                && key.origin[1] <= view_min.y
                && key.origin[0] + key.size[0] as f32 / pixels >= need_max.x - 0.5
                && key.origin[1] + key.size[1] as f32 / pixels >= need_max.y - 0.5
        };
        if self.tile.as_ref().is_some_and(|t| covered(&t.key))
            || self.tile_pending.as_ref().is_some_and(covered)
        {
            return None;
        }
        let width = ((want_max.x - want_min.x) * pixels).ceil().clamp(1., side);
        let height = ((want_max.y - want_min.y) * pixels).ceil().clamp(1., side);
        let key = TileKey {
            version: self.document_version,
            scale,
            ppp,
            origin: [want_min.x, want_min.y],
            size: [width as u32, height as u32],
        };
        if let Some(wait) = self.tile_rest(key) {
            return Some(wait);
        }
        if let Some((sender, _)) = &self.tiler {
            let request = TileRequest {
                version: key.version,
                svg: shown,
                document_width: picture_width,
                scale: key.scale,
                ppp: key.ppp,
                origin: key.origin,
                size: key.size,
            };
            if sender.send(request).is_ok() {
                self.tile_pending = Some(key);
            }
        }
        None
    }

    /// Whether the tile `key` must wait for the view to rest, and how much
    /// longer: in a tab a tile renders before the next frame is painted, so
    /// asking on every step of a zoom or a pan would render one per step
    /// (`platform::TILE_REST`; none in the window, whose render thread keeps
    /// only the latest request).
    fn tile_rest(&mut self, key: TileKey) -> Option<Duration> {
        if platform::TILE_REST.is_zero() {
            return None;
        }
        match &self.tile_wanted {
            Some((wanted, since)) if *wanted == key => {
                let waited = since.elapsed();
                (waited < platform::TILE_REST).then(|| platform::TILE_REST - waited)
            }
            _ => {
                self.tile_wanted = Some((key, Stopwatch::start()));
                Some(platform::TILE_REST)
            }
        }
    }

    pub(super) fn receive_tiles(&mut self, ctx: &egui::Context) {
        let Some((_, receiver)) = &self.tiler else {
            return;
        };
        let mut latest = None;
        while let Ok(result) = receiver.try_recv() {
            latest = Some(result);
        }
        if let Some(result) = latest {
            if result.version != self.document_version {
                return;
            }
            let key = TileKey {
                version: result.version,
                scale: result.scale,
                ppp: result.ppp,
                origin: result.origin,
                size: [result.image.size[0] as u32, result.image.size[1] as u32],
            };
            let texture =
                ctx.load_texture("vector-tile", result.image, egui::TextureOptions::LINEAR);
            self.tile = Some(Tile { key, texture });
            if self.tile_pending == Some(key) {
                self.tile_pending = None;
            }
        }
    }

    /// A small overview in the corner of the vector card: the whole picture,
    /// the window currently shown, and click or drag to move that window.
    pub(super) fn minimap(
        &mut self,
        ui: &mut egui::Ui,
        texture: &egui::TextureHandle,
        area: egui::Rect,
        content: Vec2,
        viewport: Vec2,
    ) {
        let limit = Vec2::new(180., 140.);
        let m = (limit.x / content.x).min(limit.y / content.y);
        let map_size = content * m;
        let map = egui::Rect::from_min_size(area.max - map_size - Vec2::splat(14.), map_size);
        let plate = map.expand(5.);
        let painter = ui.painter();
        painter.rect(
            plate,
            6.,
            faded(pal().backdrop.to_opaque(), 0.93),
            Stroke::new(1_f32, pal().border),
            StrokeKind::Outside,
        );
        painter.image(
            texture.id(),
            map,
            egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
            Color32::from_rgba_unmultiplied(255, 255, 255, 235),
        );
        let padded = content.max(viewport);
        let pad = (padded - content) / 2.;
        let shown_min = (self.scroll - pad).max(Vec2::ZERO);
        let shown_max = (self.scroll - pad + viewport).min(content);
        let window = egui::Rect::from_min_max(map.min + shown_min * m, map.min + shown_max * m);
        painter.rect(
            window,
            2.,
            faded(pal().accent, 0.17),
            Stroke::new(1.5_f32, pal().accent),
            StrokeKind::Inside,
        );
        let response = ui.interact(
            plate,
            ui.id().with("minimap"),
            egui::Sense::click_and_drag(),
        );
        if response.clicked() || response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let target = (pointer - map.min) / m;
                self.scroll = (target + pad - viewport / 2.)
                    .clamp(Vec2::ZERO, (padded - viewport).max(Vec2::ZERO));
            }
        }
        grab_cursor(ui, &response);
        response.on_hover_text("Overview. Click or drag to move the view.");
    }

    /// What a picture card says with nothing to show yet: on the vector
    /// card a conversion running, with its time, or a Convert button; true
    /// when that was clicked.
    pub(super) fn empty_state(&self, ui: &mut egui::Ui, vector: bool) -> bool {
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let center = rect.center() - Vec2::new(0., 12.);
        let converting = vector && self.worker.is_some();
        let (title, hint) = if converting {
            ui.ctx().request_repaint();
            (
                "Converting\u{2026}".to_owned(),
                format!("{:.1} s", self.started.elapsed().as_secs_f64()),
            )
        } else if vector && self.raster.is_some() {
            ("Ready to convert".to_owned(), String::new())
        } else if vector {
            ("The vector appears here".to_owned(), String::new())
        } else {
            ("No picture".to_owned(), String::new())
        };
        if converting {
            ui.put(
                egui::Rect::from_center_size(center - Vec2::new(0., 26.), Vec2::splat(32.)),
                egui::Spinner::new().size(28.).color(pal().accent),
            );
        } else {
            painter.text(
                center - Vec2::new(0., 26.),
                Align2::CENTER_CENTER,
                if vector { icon::VECTOR } else { icon::OPEN },
                FontId::proportional(40.),
                pal().faint,
            );
        }
        painter.text(
            center + Vec2::new(0., 18.),
            Align2::CENTER_CENTER,
            title,
            FontId::new(16., self.title_family.clone()),
            pal().text,
        );
        painter.text(
            center + Vec2::new(0., 40.),
            Align2::CENTER_CENTER,
            hint,
            FontId::proportional(12.5),
            pal().dim,
        );
        let can_convert = vector
            && self.raster.is_some()
            && self.idle()
            && self.document.is_none()
            && self.path == self.loaded_path;
        if !can_convert {
            return false;
        }
        ui.put(
            egui::Rect::from_center_size(center + Vec2::new(0., 62.), Vec2::new(132., 34.)),
            egui::Button::new(
                RichText::new(format!("{}  Convert", icon::CONVERT))
                    .color(pal().on_accent)
                    .strong(),
            )
            .fill(pal().accent)
            .stroke(Stroke::new(1_f32, pal().accent))
            .corner_radius(CornerRadius::same(17)),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    }

    /// The welcome card before any picture: compact and a little above the
    /// middle, the whole card a drop target, with Browse and Try a sample;
    /// while a picture opens it says so instead. What was asked for.
    pub(super) fn welcome(&self, ui: &mut egui::Ui) -> Start {
        let area = ui.available_rect_before_wrap();
        let size = Vec2::new(area.width().min(540.), area.height().min(330.));
        let rect = egui::Rect::from_center_size(
            area.center() - Vec2::new(0., (area.height() - size.y) * 0.12),
            size,
        );
        let mut start = Start::Nothing;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            card_frame().show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                start = self.drop_zone(ui);
            });
        });
        start
    }

    fn drop_zone(&self, ui: &mut egui::Ui) -> Start {
        let idle = self.idle();
        let (rect, zone) = ui.allocate_exact_size(
            ui.available_size(),
            if idle {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        let painter = ui.painter_at(rect);
        let dropping = idle && Self::files_hovering(ui.ctx());
        // The target eases in as a file comes over the window or the
        // pointer over the card, and dips while pressed.
        let ctx = ui.ctx().clone();
        let lit = ctx.animate_bool_with_time(zone.id.with("lit"), dropping, 0.15);
        let near = ctx.animate_bool_with_time(zone.id.with("near"), idle && zone.hovered(), 0.15);
        let down = ctx.animate_bool_with_time(
            zone.id.with("down"),
            idle && zone.is_pointer_button_down_on(),
            0.08,
        );
        let inner = rect.shrink(12. + 2. * down);
        let glow = lit.max(near * 0.5);
        if glow > 0. {
            painter.rect_filled(inner, 12., faded(pal().accent, 0.09 * glow));
        }
        dashed_round_rect(
            &painter,
            inner,
            12.,
            Stroke::new(1.5_f32, pal().faint.lerp_to_gamma(pal().accent, glow)),
        );
        let center = inner.center() - Vec2::new(0., 22.);
        let opening = !self.idle();
        let (title, hint) = if dropping {
            ("Drop to open", "")
        } else if opening {
            (self.status.as_str(), "")
        } else {
            // What to do, not the list of formats (the picker filters by them).
            ("Drop an image here", "A logo, a drawing or a photo")
        };
        painter.text(
            center - Vec2::new(0., 34.),
            Align2::CENTER_CENTER,
            icon::OPEN,
            FontId::proportional(44.),
            pal().faint.lerp_to_gamma(pal().accent, glow),
        );
        painter.text(
            center + Vec2::new(0., 14.),
            Align2::CENTER_CENTER,
            title,
            FontId::new(16., self.title_family.clone()),
            pal().text,
        );
        painter.text(
            center + Vec2::new(0., 38.),
            Align2::CENTER_CENTER,
            hint,
            FontId::proportional(12.5),
            pal().dim,
        );
        if opening {
            ui.put(
                egui::Rect::from_center_size(center + Vec2::new(0., 76.), Vec2::splat(22.)),
                egui::Spinner::new().size(20.).color(pal().accent),
            );
            return Start::Nothing;
        }
        if dropping {
            return Start::Nothing;
        }
        // Browse first, the way most people arrive: with a picture of their
        // own; the sample beside it for those without one.
        let (browse_w, sample_w, gap) = (176., 132., 10.);
        // Side by side, or one above the other on a card a phone narrows.
        let stacked = inner.width() < browse_w + gap + sample_w + 48.;
        let top = center.y + 60.;
        let (browse_at, sample_at, sample_w) = if stacked {
            (
                egui::pos2(center.x - browse_w / 2., top),
                egui::pos2(center.x - browse_w / 2., top + 34. + gap),
                browse_w,
            )
        } else {
            let left = center.x - (browse_w + gap + sample_w) / 2.;
            (
                egui::pos2(left, top),
                egui::pos2(left + browse_w + gap, top),
                sample_w,
            )
        };
        let browse = ui
            .put(
                egui::Rect::from_min_size(browse_at, Vec2::new(browse_w, 34.)),
                egui::Button::new(
                    RichText::new(format!("{}  Browse for an image", icon::OPEN))
                        .color(pal().on_accent),
                )
                .fill(pal().accent)
                .corner_radius(CornerRadius::same(17)),
            )
            .on_hover_text("Choose a picture on this computer")
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let sample = ui
            .put(
                egui::Rect::from_min_size(sample_at, Vec2::new(sample_w, 34.)),
                egui::Button::new(RichText::new("Try a sample").color(pal().text))
                    .corner_radius(CornerRadius::same(17)),
            )
            .on_hover_text("Open a sample logo and convert it")
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if browse.clicked() {
            Start::Browse
        } else if sample.clicked() {
            Start::Sample
        } else if zone
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Click to choose a picture, or drop one here")
            .clicked()
        {
            Start::Browse
        } else {
            Start::Nothing
        }
    }
}

/// What the welcome card's buttons asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Start {
    Nothing,
    Browse,
    Sample,
}

/// A node's marker: a hollow circle for a rounded corner, a hollow square
/// for any other node, in the node colour over a faint shadow so it shows on
/// light and dark fills alike. Hollow, so the edge it sits on runs on
/// through it: the filled dot with a dark ring it replaced, centred on a
/// black and white edge, merged its ring with the black and bit into the
/// fill, and a smooth curve read as turning back at every node (the owner,
/// September 24, 2026; testing/node-markers-2026-09-24).
fn node_marker(painter: &egui::Painter, screen: egui::Pos2, smooth: bool) {
    let shadow = Stroke::new(3_f32, NODE_SHADOW);
    if smooth {
        painter.circle_stroke(screen, NODE_MARKER_RADIUS, shadow);
        painter.circle_stroke(
            screen,
            NODE_MARKER_RADIUS,
            Stroke::new(1.25_f32, NODE_SMOOTH),
        );
    } else {
        let square = egui::Rect::from_center_size(screen, Vec2::splat(2. * NODE_MARKER_RADIUS));
        painter.rect_stroke(square, 0., shadow, StrokeKind::Middle);
        painter.rect_stroke(square, 0., Stroke::new(1.25_f32, NODE), StrokeKind::Middle);
    }
}

/// One piece, flattened finely enough for this zoom: what a dragged node
/// pulls along.
fn paint_piece(
    painter: &egui::Painter,
    piece: NodePiece,
    stroke: Stroke,
    scale: f32,
    to_screen: &dyn Fn(&Point) -> egui::Pos2,
) {
    let [p0, p1, p2, p3] = piece.cubic.points;
    let points = if piece.line {
        vec![to_screen(&p0), to_screen(&p3)]
    } else {
        let extent = [(p0, p1), (p1, p2), (p2, p3)]
            .iter()
            .map(|(a, b)| (b.x - a.x).abs() + (b.y - a.y).abs())
            .sum::<f64>() as f32
            * scale;
        let steps = ((extent / 3.).ceil() as usize).clamp(2, 96);
        (0..=steps)
            .map(|step| to_screen(&piece.cubic.evaluate(step as f64 / steps as f64)))
            .collect()
    };
    painter.add(egui::Shape::line(points, stroke));
}

/// The exact pieces of one island, flattened finely enough for this zoom so
/// the highlight hugs the curve instead of showing chords.
fn paint_island_outline(
    painter: &egui::Painter,
    island: &Island,
    stroke: Stroke,
    scale: f32,
    to_screen: &dyn Fn(&Point) -> egui::Pos2,
) {
    for pieces in &island.pieces {
        let mut points: Vec<egui::Pos2> = Vec::new();
        for piece in pieces {
            let [p0, p1, p2, p3] = piece.cubic.points;
            if points.is_empty() {
                points.push(to_screen(&p0));
            }
            if piece.line {
                points.push(to_screen(&p3));
                continue;
            }
            let extent = ((p1.x - p0.x).abs()
                + (p1.y - p0.y).abs()
                + (p2.x - p1.x).abs()
                + (p2.y - p1.y).abs()
                + (p3.x - p2.x).abs()
                + (p3.y - p2.y).abs()) as f32
                * scale;
            let steps = ((extent / 3.).ceil() as usize).clamp(2, 96);
            for step in 1..=steps {
                let t = step as f64 / steps as f64;
                points.push(to_screen(&piece.cubic.evaluate(t)));
            }
        }
        painter.add(egui::Shape::closed_line(points, stroke));
    }
}
