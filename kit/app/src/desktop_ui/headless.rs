//! Part of `desktop_ui`: the headless snapshot state.

use super::*;

impl Desktop {
    /// One frame of the window: the desktop runs it for eframe, the browser
    /// build (`kit/web`) for its canvas.
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.apply_appearance(ctx);
        look::paint_backdrop(ctx);
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = self.snapshot_path.take() {
                    let rgba: Vec<u8> = image
                        .pixels
                        .iter()
                        .flat_map(|p| p.to_srgba_unmultiplied())
                        .collect();
                    if platform::IN_BROWSER {
                        self.download_png(&path, &rgba, image.width(), image.height());
                        continue;
                    }
                    match image::save_buffer(
                        &path,
                        &rgba,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => self.set_status(
                            StatusKind::Done,
                            format!("Saved app preview {}", path.display()),
                        ),
                        Err(e) => self.set_status(StatusKind::Error, e.to_string()),
                    }
                }
            }
        }
        #[cfg(feature = "ui-test")]
        if let Ok(path) = std::env::var("VM_PREVIEW_SCREENSHOT") {
            self.frames += 1;
            ctx.request_repaint();
            if self.frames == 10 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    image::save_buffer(
                        &path,
                        &rgba,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    )
                    .expect("save GUI test screenshot");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if std::mem::take(&mut self.load_on_first_frame) {
            self.load(ctx);
        }
        if let Some(receiver) = &self.worker {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Conversion worker stopped unexpectedly.".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.worker = None;
                match result {
                    Ok(result) => self.accept(ctx, result),
                    Err(e) => {
                        // Edits made while it ran apply to the result that
                        // comes back.
                        if self.restore_replaced() {
                            self.reapply();
                        }
                        // Convert automatically does not try the settings
                        // that failed again until they change.
                        self.held_inputs = Some(self.conversion_inputs());
                        self.set_status(StatusKind::Error, e)
                    }
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        // A cancelled conversion is gone once its thread answers or ends.
        if let Some(receiver) = &self.stopping {
            if matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                ctx.request_repaint_after(Duration::from_millis(100));
            } else {
                self.stopping = None;
            }
        }
        self.receive_errand(ctx);
        self.receive_derivations(ctx);
        self.receive_tiles(ctx);
        if let Some(dropped) = ctx.input(|i| i.raw.dropped_files.iter().find_map(Dropped::of)) {
            if self.idle() {
                match dropped {
                    Dropped::Path(path) => {
                        self.path = path.to_string_lossy().into();
                        self.load(ctx);
                    }
                    Dropped::Bytes(name, bytes) => self.load_bytes(ctx, name, &bytes),
                }
            } else {
                let name = dropped.name();
                let wait = if self.worker.is_some() {
                    "a conversion is running; cancel it (Esc) or let it finish"
                } else if self.stopping.is_some() {
                    "a cancelled conversion is still stopping"
                } else {
                    "finish with the dialog or the save first"
                };
                self.set_status(
                    StatusKind::Info,
                    format!("{name} was not opened: {wait}, then drop it again."),
                );
            }
        }

        let mut actions = Actions::default();
        let idle = self.idle();
        ctx.input_mut(|i| {
            actions.open = i.consume_shortcut(&SC_OPEN);
            actions.convert = i.consume_shortcut(&SC_CONVERT);
            actions.preview = i.consume_shortcut(&SC_PREVIEW);
            actions.save = i.consume_shortcut(&SC_SAVE);
            actions.close = i.consume_shortcut(&SC_CLOSE);
            // Escape stops a running conversion, and is left to the popups
            // otherwise.
            actions.cancel = self.worker.is_some() && i.consume_key(Modifiers::NONE, Key::Escape);
        });
        // A text field being typed in keeps Ctrl+Z for its own text.
        let mut key_peek = None;
        if !ctx.wants_keyboard_input() {
            ctx.input_mut(|i| {
                actions.redo = i.consume_shortcut(&SC_REDO_SHIFT) || i.consume_shortcut(&SC_REDO);
                actions.undo = i.consume_shortcut(&SC_UNDO);
                if self.hold_compare && self.view == View::Overlay {
                    key_peek = if i.key_down(Key::B) {
                        Some(false)
                    } else if i.key_down(Key::V) {
                        Some(true)
                    } else {
                        None
                    };
                }
            });
        }
        // Under Hold, the picture held (by its button or its key) shows until
        // let go.
        self.peek = if self.hold_compare && self.view == View::Overlay {
            key_peek.or(self.peek_button)
        } else {
            self.peek_button = None;
            None
        };
        if !ctx.wants_keyboard_input() {
            let (mut zoom_in, mut zoom_out, mut fit, mut actual, mut nodes, mut bitmap, mut vector) =
                (false, false, false, false, false, false, false);
            let mut whole = None;
            ctx.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::Num2) {
                    whole = Some(2.);
                } else if i.consume_key(Modifiers::NONE, Key::Num3) {
                    whole = Some(3.);
                }
                zoom_in = i.consume_key(Modifiers::NONE, Key::Plus)
                    || i.consume_key(Modifiers::NONE, Key::Equals)
                    || i.consume_key(Modifiers::SHIFT, Key::Plus)
                    || i.consume_key(Modifiers::SHIFT, Key::Equals);
                zoom_out = i.consume_key(Modifiers::NONE, Key::Minus);
                fit = i.consume_key(Modifiers::NONE, Key::F);
                actual = i.consume_key(Modifiers::NONE, Key::Num1);
                nodes = i.consume_key(Modifiers::NONE, Key::N);
                bitmap = i.consume_key(Modifiers::NONE, Key::B);
                vector = i.consume_key(Modifiers::NONE, Key::V);
            });
            // Under Hold, B and V in the overlay show their picture while
            // held (above) instead of switching to it.
            if (bitmap || vector) && !(self.hold_compare && self.view == View::Overlay) {
                self.view = View::Overlay;
                self.overlay_vector = vector;
            }
            if zoom_in {
                self.set_zoom(self.zoom * ZOOM_STEP);
            }
            if zoom_out {
                self.set_zoom(self.zoom / ZOOM_STEP);
            }
            if fit {
                self.set_zoom(1.);
                self.scroll = Vec2::ZERO;
            }
            if actual {
                let fit = self.fit;
                self.set_zoom(1. / fit);
            }
            // 2 and 3: the picture that many times its own size.
            if let Some(times) = whole {
                let fit = self.fit;
                self.set_zoom(times / fit);
            }
            if nodes {
                self.nodes = !self.nodes;
            }
        }

        self.header(ctx, &mut actions);
        self.rail(ctx);
        self.status_bar(ctx);
        self.workspace(ctx);
        if actions.save && idle && (self.document.is_some() || self.foreign.is_some()) {
            self.save_open = !self.save_open;
        }
        self.save_popup(ctx);
        self.appearance_popup(ctx);
        self.stat_popups(ctx);
        self.node_menu_ui(ctx);
        self.shape_menu_ui(ctx);
        self.licence_prompt(ctx);
        self.vector_prompt(ctx);

        if actions.cancel {
            self.cancel();
        }
        if actions.open && idle {
            self.open_dialog();
        }
        if actions.convert && idle && self.raster.is_some() && self.path == self.loaded_path {
            self.start();
        }
        if actions.preview && idle {
            self.save_preview_png();
        }
        if actions.close && idle && self.raster.is_some() {
            self.close();
        }
        if actions.undo {
            self.undo();
        } else if actions.redo {
            self.redo();
        }
        // A change finished this frame (no button held) is one step of Undo.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.record_edits();
        }
        self.auto_convert_tick(ctx);
        self.licence_tick(ctx);
        self.save_prefs();
    }
}

/// A file dropped on the window (a path) or on the page (its bytes).
enum Dropped {
    Path(PathBuf),
    Bytes(String, Arc<[u8]>),
}

impl Dropped {
    /// A file dragged out of the Save popup and released back over the
    /// window is ours; it is not a request to open it.
    fn of(file: &egui::DroppedFile) -> Option<Self> {
        match (&file.path, &file.bytes) {
            #[cfg(feature = "desktop")]
            (Some(path), _) if crate::dragout::is_staged(path) => None,
            (Some(path), _) => Some(Self::Path(path.clone())),
            (None, Some(bytes)) => Some(Self::Bytes(file.name.clone(), bytes.clone())),
            (None, None) => None,
        }
    }
    fn name(&self) -> String {
        match self {
            Self::Path(path) => path.file_name().map_or_else(
                || path.to_string_lossy().into_owned(),
                |n| n.to_string_lossy().into_owned(),
            ),
            Self::Bytes(name, _) => name.clone(),
        }
    }
}
