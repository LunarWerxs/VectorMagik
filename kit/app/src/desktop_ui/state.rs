//! Part of `desktop_ui`: the document state, loading, conversion, editing and saving.

use super::*;

impl Desktop {
    #[cfg(feature = "desktop")]
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::blank(&cc.egui_ctx);
        if let Some(path) = prefs::prefs_path() {
            app.load_prefs(path);
        }
        if let Some(path) = std::env::args_os()
            .nth(1)
            .map(|a| a.to_string_lossy().into_owned())
        {
            app.path = path;
            app.load_on_first_frame = true;
        }
        app
    }
    /// The app in a browser tab (`kit/web`), with the preferences the page
    /// kept from the last visit.
    pub fn in_browser(ctx: &egui::Context, prefs: &str) -> Self {
        let mut app = Self::blank(ctx);
        (app.hold_compare, app.auto_convert) =
            prefs::parse_prefs(prefs, (app.hold_compare, app.auto_convert));
        app.prefs_saved = (app.hold_compare, app.auto_convert);
        app
    }
    /// The drawing as it is saved, once nothing is left running for it; for
    /// the browser build's check that the app there draws what the desktop
    /// does.
    pub fn settled_svg(&self) -> Option<String> {
        if !self.idle() || self.derive_pending.is_some() || platform::has_pending() {
            return None;
        }
        self.export_svg()?.ok()
    }
    /// The status line as the footer shows it.
    pub fn status_line(&self) -> String {
        self.status.clone()
    }
    pub(crate) fn blank(ctx: &egui::Context) -> Self {
        #[cfg(feature = "desktop")]
        crate::dragout::tidy();
        let title_family = install_fonts(ctx);
        apply_theme(ctx, title_family.clone());
        let logo = render_logo(64).map(|rgba| {
            ctx.load_texture(
                "logo",
                egui::ColorImage::from_rgba_premultiplied([64, 64], &rgba),
                egui::TextureOptions::LINEAR,
            )
        });
        Self {
            path: String::new(),
            loaded_path: String::new(),
            options: VectorizeOptions::default(),
            automatic: true,
            detected: None,
            raster: None,
            working: None,
            working_opaque: false,
            has_alpha: false,
            load_on_first_frame: false,
            prep: Preparation::default(),
            converted_prep: None,
            background_custom: [255, 255, 255],
            deleted: Vec::new(),
            shapes_mode: false,
            delete_on_click: false,
            selected: Vec::new(),
            islands: None,
            nodes_cache: None,
            shape_menu: None,
            raw_document: None,
            raw_version: 0,
            document: None,
            shown: None,
            shown_margin: 0.,
            document_version: 0,
            sticker_on: false,
            sticker: Sticker::default(),
            sticker_custom: [[79, 209, 197], [255, 184, 76]],
            border_opaque: false,
            raw_counts: (0, 0),
            node_counts: None,
            segment_counts: None,
            palette: Vec::new(),
            rounded: Vec::new(),
            moved: Vec::new(),
            deleted_nodes: Vec::new(),
            node_drag: None,
            node_menu: None,
            deletable: None,
            undo: Vec::new(),
            redo: Vec::new(),
            edits_seen: None,
            auto_convert: true,
            converted_inputs: None,
            inputs_changed: None,
            held_inputs: None,
            hold_compare: false,
            peek: None,
            peek_button: None,
            rail_bar_since: None,
            prefs: None,
            prefs_saved: (false, true),
            deriver: Some(spawn_deriver(ctx.clone())),
            derive_serial: 0,
            derive_pending: None,
            auto_pending: false,
            source: None,
            source_sharp: None,
            working_source: None,
            working_sharp: None,
            show_engine_input: false,
            // One picture, switched between the vector and the bitmap, with
            // the nodes shown: how the owner works (September 23, 2026).
            view: View::Overlay,
            overlay_vector: true,
            collapsed: [false; 6],
            advanced_on: false,
            sliders: Sliders::default(),
            regularize: true,
            primitives: DEFAULT_PRIMITIVES,
            straighten: true,
            straighten_tolerance: DEFAULT_STRAIGHTEN_TOLERANCE,
            straighten_auto: true,
            straightened: Vec::new(),
            preview: None,
            tile: None,
            tile_pending: None,
            tile_wanted: None,
            tiler: Some(spawn_tiler(ctx.clone())),
            logo,
            worker: None,
            stopping: None,
            stop: Arc::new(AtomicBool::new(false)),
            replaced: None,
            errand: None,
            staged: None,
            status: "Open an image, or drop one onto the source card.".into(),
            status_kind: StatusKind::Info,
            started: Stopwatch::start(),
            elapsed: None,
            zoom: 1.,
            fit: 1.,
            scroll: Vec2::ZERO,
            nodes: true,
            simplify: true,
            simplify_tolerance: DEFAULT_SIMPLIFY_TOLERANCE,
            simplify_pick: None,
            output_scale: 1.,
            source_size: None,
            save_format: Format::Svg,
            save_stacked: true,
            save_open: false,
            save_anchor: egui::Rect::NOTHING,
            stat_popup: None,
            stat_anchors: [egui::Rect::NOTHING; 2],
            snapshot_path: None,
            title_family,

            #[cfg(feature = "ui-test")]
            frames: 0,
        }
    }
    #[cfg(feature = "desktop")]
    #[allow(
        clippy::too_many_arguments,
        reason = "one argument per headless snapshot input; the CLI passes them straight through"
    )]
    pub(crate) fn snapshot_state(
        ctx: &egui::Context,
        input: Option<&std::path::Path>,
        convert: bool,
        nodes: bool,
        zoom: f32,
        manual: Option<VectorizeOptions>,
        simplify: Option<f64>,
        prep: Preparation,
        optimizer: bool,
        sticker: StickerChoice,
        straighten: Option<Bow>,
        regularize: bool,
        primitives: bool,
        sliders: Option<Sliders>,
    ) -> Result<Self, String> {
        let mut app = Self::blank(ctx);
        // Snapshots show both pictures unless asked for one (`set_view`).
        app.view = View::SideBySide;
        app.nodes = nodes;
        app.zoom = zoom;
        app.prep = prep;
        app.regularize = regularize;
        app.primitives = primitives;
        if let Some(sliders) = sliders {
            app.advanced_on = true;
            app.sliders = sliders;
        }
        app.straighten = straighten.is_some();
        if let Some(Bow::Pixels(tolerance)) = straighten {
            app.straighten_tolerance = tolerance as f32;
            app.straighten_auto = false;
        }
        app.simplify = simplify.is_some();
        if let Some(tolerance) = simplify {
            app.simplify_tolerance = tolerance as f32;
        }
        if let Some(options) = manual {
            app.options = options;
            app.automatic = false;
        }
        app.options.optional_optimizer = optimizer;
        if let Some(input) = input {
            app.path = input.to_string_lossy().into_owned();
            app.load(ctx);
            if app.raster.is_none() {
                return Err(app.status);
            }
            // After loading, so widths asked for win over the picture's own.
            match sticker {
                StickerChoice::Off => {}
                StickerChoice::Sized => app.sticker_on = true,
                StickerChoice::Custom(sticker) => {
                    app.sticker_on = true;
                    app.sticker = sticker;
                }
            }
            if convert {
                app.started = Stopwatch::start();
                let result = Self::process(
                    app.raster.clone().unwrap(),
                    &app.prep,
                    app.options,
                    app.automatic,
                    app.slider_settings(),
                    &app.derive_settings(),
                    &AtomicBool::new(false),
                )?;
                app.accept(ctx, result);
            }
        }
        app.zoom = zoom;
        Ok(app)
    }
    /// Cut the background shapes out and re-derive on this thread, for a
    /// snapshot; the window uses the derive thread instead.
    #[cfg(feature = "desktop")]
    pub(crate) fn cut_background_now(&mut self, ctx: &egui::Context) -> Result<usize, String> {
        let removals = self.background_removals();
        if removals.is_empty() {
            return Err("No background shape touches the edge of the picture".into());
        }
        let count = removals.len();
        self.deleted.extend(removals);
        self.rederive_now(ctx)?;
        Ok(count)
    }
    #[cfg(feature = "desktop")]
    pub(super) fn rederive_now(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let Some(raw) = self.raw_document.clone() else {
            return Err("Convert an image first".into());
        };
        let settings = self.derive_settings();
        let shown = Self::derive(&raw, &settings)?;
        let presented = Presented::of(&shown, settings.sticker.as_ref())?;
        self.set_document(ctx, Derived::new(shown, presented));
        Ok(())
    }
    pub(super) fn set_status(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.status = text.into();
        self.status_kind = kind;
    }
    /// Side by side, or one picture showing the vector (`true`) or the bitmap.
    #[cfg(feature = "desktop")]
    pub(crate) fn set_view(&mut self, view: View, vector: bool) {
        self.view = view;
        self.overlay_vector = vector;
    }
    /// Convert again with the settings in force and leave it running, for a
    /// snapshot of the view while a conversion replaces the shown result.
    #[cfg(feature = "desktop")]
    pub(crate) fn convert_again(&mut self) {
        self.start();
    }
    /// Open one of the popups as a user would, for a snapshot. The node menu
    /// takes the first node of the shown document and opens at `at`.
    #[cfg(feature = "desktop")]
    pub(crate) fn open_overlay(&mut self, overlay: Overlay, at: egui::Pos2) {
        match overlay {
            Overlay::Save => self.save_open = self.document.is_some(),
            Overlay::Size => self.stat_popup = Some(StatPopup::Size),
            Overlay::Colors => self.stat_popup = Some(StatPopup::Colors),
            Overlay::NodeMenu => {
                if let Some(node) = self
                    .document
                    .as_ref()
                    .and_then(|d| d.nodes().first().copied())
                {
                    self.node_menu = Some((node, at));
                }
            }
            Overlay::Shapes => {
                self.shapes_mode = true;
                let Some(raster) = &self.raster else {
                    return;
                };
                let centre = Point {
                    x: raster.width as f64 / 2.,
                    y: raster.height as f64 / 2.,
                };
                let islands = self.current_islands();
                if let Some(index) = shapes::island_at(&islands, centre) {
                    let shape = Removal {
                        color: islands[index].color.clone(),
                        at: centre,
                    };
                    self.selected.push(shape.clone());
                    self.shape_menu = Some((shape, at));
                }
            }
        }
    }
    /// Round the node nearest `at` with `reach` and re-derive the shown
    /// document on this thread, for a snapshot; the window uses the derive
    /// thread instead.
    #[cfg(feature = "desktop")]
    pub(crate) fn round_nearest_now(
        &mut self,
        ctx: &egui::Context,
        at: Point,
        reach: Reach,
    ) -> Result<(), String> {
        let Some(raw) = self.raw_document.clone() else {
            return Err("Convert an image before rounding a node".into());
        };
        let nodes = raw.nodes();
        let nearest = nodes
            .iter()
            .min_by(|a, b| {
                let da = (a.x - at.x).powi(2) + (a.y - at.y).powi(2);
                let db = (b.x - at.x).powi(2) + (b.y - at.y).powi(2);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .ok_or("The document has no nodes")?;
        self.rounded.push(Rounded { at: nearest, reach });
        self.rederive_now(ctx)
    }
    /// Delete the shown node nearest `at` (keeping the shape when asked) and
    /// re-derive the shown document on this thread, for a snapshot.
    #[cfg(feature = "desktop")]
    pub(crate) fn delete_nearest_now(
        &mut self,
        ctx: &egui::Context,
        at: Point,
        keep_shape: bool,
    ) -> Result<(), String> {
        let nodes = self
            .document
            .as_ref()
            .ok_or("Convert an image before deleting a node")?
            .nodes();
        let nearest = nodes
            .iter()
            .min_by(|a, b| {
                let da = (a.x - at.x).powi(2) + (a.y - at.y).powi(2);
                let db = (b.x - at.x).powi(2) + (b.y - at.y).powi(2);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .ok_or("The document has no nodes")?;
        if let Some(reason) = self.deletion_refusal(&nearest) {
            return Err(format!(
                "The node at {:.2}, {:.2} cannot be deleted: {reason}",
                nearest.x, nearest.y
            ));
        }
        self.deleted_nodes.push(NodeDeletion {
            at: nearest,
            keep_shape,
        });
        self.rederive_now(ctx)
    }
    /// The shown document's nodes, parsed once per document version (on the
    /// derive or conversion thread, for every document those make).
    pub(super) fn current_nodes(&mut self) -> Arc<Vec<Point>> {
        if let Some((version, nodes)) = &self.nodes_cache {
            if *version == self.document_version {
                return Arc::clone(nodes);
            }
        }
        let nodes = Arc::new(
            self.document
                .as_ref()
                .map(|d| d.nodes())
                .unwrap_or_default(),
        );
        self.nodes_cache = Some((self.document_version, Arc::clone(&nodes)));
        nodes
    }
    /// The shown document's shapes, computed once per document version.
    pub(super) fn current_islands(&mut self) -> Arc<Vec<Island>> {
        if let Some((version, islands)) = &self.islands {
            if *version == self.document_version {
                return Arc::clone(islands);
            }
        }
        let islands = Arc::new(
            self.document
                .as_ref()
                .and_then(|d| shapes::islands(d.svg()).ok())
                .unwrap_or_default(),
        );
        self.islands = Some((self.document_version, Arc::clone(&islands)));
        islands
    }
    /// The selected shapes as they exist in the shown document; a selection
    /// whose shape is gone (deleted, merged away) is dropped.
    pub(super) fn resolve_selection(&mut self) -> Vec<Island> {
        let islands = self.current_islands();
        let mut resolved = Vec::new();
        self.selected.retain(|s| match find_shape(&islands, s) {
            Some(i) => {
                resolved.push(islands[i].clone());
                true
            }
            None => false,
        });
        resolved
    }
    pub(super) fn is_selected(&self, shape: &Removal) -> bool {
        self.selected.iter().any(|s| same_shape(s, shape))
    }
    pub(super) fn toggle_selected(&mut self, shape: Removal) {
        if let Some(index) = self.selected.iter().position(|s| same_shape(s, &shape)) {
            self.selected.remove(index);
        } else {
            self.selected.push(shape);
        }
    }
    pub(super) fn delete_shape(&mut self, shape: Removal) {
        self.selected.retain(|s| !same_shape(s, &shape));
        self.deleted.push(shape);
        self.reapply();
    }
    pub(super) fn delete_selected(&mut self) {
        let taken: Vec<Removal> = std::mem::take(&mut self.selected);
        if !taken.is_empty() {
            self.deleted.extend(taken);
            self.reapply();
        }
    }
    /// Recolour every selected shape of another colour to `target` (the first
    /// selected shape's colour unless given) in the image the engine sees, and
    /// convert again, so they trace as one shape with no boundary between.
    pub(super) fn merge_selected(&mut self, target: Option<String>) {
        let resolved = self.resolve_selection();
        let Some(target) = target.or_else(|| resolved.first().map(|island| island.color.clone()))
        else {
            return;
        };
        let Some(rgb) = crate::parse_rgb(&target) else {
            self.set_status(StatusKind::Error, format!("Cannot merge into {target}"));
            return;
        };
        let Some((width, height)) = self.working.as_ref().map(|w| (w.width, w.height)) else {
            return;
        };
        let mut merged = 0;
        for island in resolved
            .iter()
            .filter(|island| !island.color.eq_ignore_ascii_case(&target))
        {
            let pixels = island.covered(width, height, true);
            if !pixels.is_empty() {
                self.prep.recolors.push(Recolor {
                    pixels,
                    rgb,
                    alpha: None,
                });
                merged += 1;
            }
        }
        self.selected.clear();
        if merged == 0 {
            self.set_status(
                StatusKind::Info,
                "Select shapes of two colors to merge them.",
            );
            return;
        }
        self.start();
        self.set_status(
            StatusKind::Busy,
            format!(
                "Merging {merged} shape{} into {target} and converting again\u{2026}",
                if merged == 1 { "" } else { "s" }
            ),
        );
    }
    /// Drop one fill colour (see `colour_drop`) and convert again; the pixels
    /// are worked out on the conversion's thread, so a photograph's thousands
    /// of shapes never stop the window. Undone with the Shapes card's Undo
    /// merges.
    pub(super) fn remove_color(&mut self, hex: &str) {
        if self.palette.len() < 2 {
            self.set_status(StatusKind::Info, "The last color stays.");
            return;
        }
        let Some(working) = self.working.clone() else {
            return;
        };
        let islands = self.current_islands();
        if !islands
            .iter()
            .any(|island| island.color.eq_ignore_ascii_case(hex))
        {
            self.set_status(StatusKind::Info, format!("No shape of {hex} to drop."));
            return;
        }
        self.start_with(Some(ColourDrop {
            hex: hex.to_owned(),
            islands,
            working,
        }));
        self.set_status(
            StatusKind::Busy,
            format!("Dropping {hex}: its shapes take their neighbours' colors and the image converts again\u{2026}"),
        );
    }
    pub(super) fn simplify_settings(&self) -> Option<f64> {
        self.simplify.then_some(match self.simplify_pick {
            Some(pick) if pick as f32 == self.simplify_tolerance => pick,
            _ => self.simplify_tolerance as f64,
        })
    }
    /// The settings the shown document is derived with now.
    pub(super) fn derive_settings(&self) -> DeriveSettings {
        DeriveSettings {
            simplify: self.simplify_settings(),
            // By hand: the slider no longer shows what the app chose itself,
            // Auto's pick or the tolerance it starts at.
            simplify_by_hand: self.simplify_tolerance
                != self
                    .simplify_pick
                    .map_or(DEFAULT_SIMPLIFY_TOLERANCE, |pick| pick as f32),
            deleted: self.deleted.clone(),
            regularize: self.regularize_settings(),
            primitives: self.primitives,
            straighten: self.straighten_settings(),
            straightened: self.straightened.clone(),
            moved: self.moved.clone(),
            deleted_nodes: self.deleted_nodes.clone(),
            rounded: self.rounded.iter().map(|r| r.rounding()).collect(),
            sticker: self.sticker_settings(),
        }
    }
    /// The shown document from the engine's output: the deleted shapes taken
    /// out, simplified when asked, then the rounded corners applied.
    pub(super) fn derive(
        raw: &VectorDocument,
        settings: &DeriveSettings,
    ) -> Result<VectorDocument, String> {
        let raw = raw.without_islands(&settings.deleted)?;
        let shown = match settings.simplify {
            Some(tolerance) if settings.simplify_by_hand => raw.simplified_by_hand(tolerance)?,
            Some(tolerance) => raw.simplified(tolerance)?,
            None => raw,
        };
        Self::finish(shown, settings)
    }
    /// The steps after simplification: true lines and circles,
    /// straightening and the shapes from the pixels (`post_passes`), then
    /// the nodes moved by hand, then the nodes deleted by hand, then the
    /// rounded corners (both keyed where the moves left their nodes).
    pub(super) fn finish(
        mut shown: VectorDocument,
        settings: &DeriveSettings,
    ) -> Result<VectorDocument, String> {
        shown = shown.post_passes(
            settings.regularize,
            settings.straighten,
            settings.primitives,
            &settings.straightened,
        )?;
        shown = shown.moved(&settings.moved)?;
        shown = shown.without_nodes(&settings.deleted_nodes)?;
        let rounded = &settings.rounded;
        if !rounded.is_empty() {
            shown = shown.smoothed(rounded)?;
        }
        Ok(shown)
    }
    /// The Advanced card's sliders when it is on.
    pub(super) fn slider_settings(&self) -> Option<Sliders> {
        self.advanced_on.then_some(self.sliders)
    }
    /// The advanced settings the next conversion will run, for the image
    /// type and quality in force (none while Auto has not looked at the
    /// picture). Auto's quality follows the picture's size, so a picture of
    /// 48 to 159 px on its shorter side (Medium) runs the basic preset even
    /// with Auto on; smaller and larger ones trace at High.
    pub(super) fn pending_advanced(&self) -> Option<Option<AdvancedSettings>> {
        let category = self.effective_category()?;
        let quality = self.effective_quality()?;
        let options = VectorizeOptions {
            category,
            quality,
            advanced: match self.slider_settings() {
                Some(sliders) => Some(crate::engine::advanced_settings(category, sliders).ok()?),
                None => None,
            },
            ..self.options
        };
        Some(crate::engine::effective_advanced(&options))
    }
    /// Whether the next conversion would run other advanced settings than
    /// the result shown did: the Advanced card's "Convert again" hint.
    pub(super) fn advanced_stale(&self) -> bool {
        match (&self.document, self.pending_advanced()) {
            (Some(document), Some(next)) => document.advanced != next,
            _ => false,
        }
    }
    /// What the next conversion would record in its document: the preset
    /// (image type and quality), whether opaque-photo seams are hidden, and
    /// the optional pass. `None` while Auto has not looked at the picture.
    pub(super) fn pending_conversion(&self) -> Option<(usize, bool, bool)> {
        let category = self.effective_category()?;
        let quality = self.effective_quality()?;
        Some((
            vector_rebuild::basic_preset_code(category, quality),
            self.options.overlap_opaque_photos
                && category == ImageCategory::Photograph
                && self.working_opaque,
            self.options.optional_optimizer,
        ))
    }
    /// Whether another Convert would trace differently from the result shown:
    /// another preparation, image type, quality, seam treatment or optional
    /// pass (the Advanced card has its own hint).
    pub(super) fn conversion_stale(&self) -> bool {
        let Some(document) = &self.document else {
            return false;
        };
        self.converted_prep.as_ref() != Some(&self.prep)
            || self.pending_conversion()
                != Some((
                    document.preset,
                    document.photo_overlap,
                    document.optional_optimizer,
                ))
    }
    /// The sliders the card starts from when switched on: the ones behind
    /// the conversion the picture would get anyway (the photo detail
    /// ceiling for a high-quality photograph, the dialog's defaults else).
    pub(super) fn seed_sliders(&mut self) {
        let was_on = std::mem::replace(&mut self.advanced_on, false);
        let effective = self.pending_advanced().flatten();
        self.advanced_on = was_on;
        self.sliders = Sliders::of(effective);
    }
    pub(super) fn regularize_settings(&self) -> Option<RegularizeOptions> {
        self.regularize.then_some(RegularizeOptions {
            band: DEFAULT_REGULARIZE_BAND,
        })
    }
    /// The engine crate's defaults (the CLI's `--straighten`), with the
    /// slider's bow or Auto; the kind's bow, `grid` and the snap limit are
    /// set by `finish` from the preset the document was traced with. Under
    /// Auto the slider only shows the bow, so it is left out of the settings
    /// and a new picture's bow does not count as a change to derive again.
    pub(super) fn straighten_settings(&self) -> Option<StraightenOptions> {
        self.straighten.then_some(StraightenOptions {
            flatness: if self.straighten_auto {
                StraightenOptions::default().flatness
            } else {
                self.straighten_tolerance as f64
            },
            auto: self.straighten_auto,
            ..StraightenOptions::default()
        })
    }
    /// The bow Auto takes for the picture shown, if one is.
    pub(super) fn auto_bow(&self) -> Option<f32> {
        self.raw_document
            .as_ref()
            .map(|raw| vector_rebuild::straighten::auto_flatness(raw.preset) as f32)
    }
    /// Show a derived document; its counts, palette and nodes came with it,
    /// so nothing is parsed here.
    pub(super) fn set_document(&mut self, ctx: &egui::Context, derived: Derived) {
        let Derived {
            document,
            presented,
            nodes,
            segments,
            palette,
        } = derived;
        self.preview =
            Some(ctx.load_texture("vector", presented.image, egui::TextureOptions::LINEAR));
        let (raw_nodes, raw_segments) = self.raw_counts;
        self.node_counts = Some((raw_nodes, nodes.len()));
        self.segment_counts = Some((raw_segments, segments));
        self.palette = palette;
        self.document = Some(document);
        self.shown = Some(presented.svg);
        self.shown_margin = presented.margin;
        self.document_version += 1;
        self.nodes_cache = Some((self.document_version, nodes));
        self.tile = None;
        self.tile_pending = None;
    }
    /// The Sticker card's settings when the sticker is on and has a width;
    /// both widths at zero paint nothing rather than stopping the slider.
    pub(super) fn sticker_settings(&self) -> Option<Sticker> {
        (self.sticker_on && self.sticker.validate().is_ok()).then_some(self.sticker)
    }
    /// How much wider than the source the painted picture is on every side:
    /// the margin of the sticker it was painted with, or nothing.
    pub(super) fn picture_margin(&self) -> f32 {
        if self.document.is_some() {
            self.shown_margin
        } else if let Some(replaced) = self.stale() {
            replaced.shown_margin
        } else {
            0.
        }
    }
    /// The result a running conversion is replacing, while the vector card
    /// still draws it: from the start until the new result arrives (or the
    /// old one comes back on Cancel).
    pub(super) fn stale(&self) -> Option<&Replaced> {
        self.replaced.as_deref().filter(|_| self.preview.is_none())
    }
    /// The node and segment counts (the engine's, then the shown ones) and
    /// the number of fill colors on screen: the shown document's, or while a
    /// conversion runs the ones of the result it replaces, so the footer and
    /// the Nodes card change their numbers in place.
    pub(super) fn shown_counts(&self) -> (Counts, Counts, usize) {
        match self.stale() {
            Some(r) if self.document.is_none() => {
                (r.node_counts, r.segment_counts, r.palette.len())
            }
            _ => (self.node_counts, self.segment_counts, self.palette.len()),
        }
    }
    /// The vector picture the card draws: the shown document's, or the one
    /// a running conversion is replacing.
    pub(super) fn vector_texture(&self) -> Option<egui::TextureHandle> {
        self.preview
            .clone()
            .or_else(|| self.stale().and_then(|r| r.preview.clone()))
    }
    /// The background shapes of the shown document, as removals, when the
    /// image the engine saw has an opaque border.
    pub(super) fn background_removals(&mut self) -> Vec<Removal> {
        let Some(working) = self.working.clone() else {
            return Vec::new();
        };
        if !self.border_opaque {
            return Vec::new();
        }
        let islands = self.current_islands();
        vector_rebuild::sticker::background_removals(
            &islands,
            working.width,
            working.height,
            |x, y| working.pixels[y * working.width + x].0[3] == 255,
        )
    }
    /// Cut the background shapes out of the vector, so the sticker hugs the
    /// object rather than the picture's rectangle. They go with the deleted
    /// shapes, so Restore deleted brings them back.
    pub(super) fn cut_background(&mut self) {
        let removals = self.background_removals();
        if removals.is_empty() {
            self.set_status(
                StatusKind::Info,
                "No background shape touches the edge of the picture.",
            );
            return;
        }
        let count = removals.len();
        self.selected.clear();
        self.deleted.extend(removals);
        self.reapply();
        self.set_status(
            StatusKind::Done,
            format!(
                "Cut out {count} background shape{}. Restore deleted brings {} back.",
                if count == 1 { "" } else { "s" },
                if count == 1 { "it" } else { "them" }
            ),
        );
    }
    /// Ask the derive thread for the shown document after the Curves or Nodes
    /// controls change. No conversion is repeated; the latest request wins.
    /// While a conversion runs there is nothing to derive from yet: `accept`
    /// derives again when the settings changed meanwhile.
    pub(super) fn request_derive(&mut self, job: DeriveJob) {
        let settings = self.derive_settings();
        let (Some(raw), Some((sender, _))) = (&self.raw_document, &self.deriver) else {
            return;
        };
        self.derive_serial += 1;
        let request = DeriveRequest {
            serial: self.derive_serial,
            raw_version: self.raw_version,
            raw: Arc::clone(raw),
            settings,
            job,
        };
        if sender.send(request).is_ok() {
            self.derive_pending = Some(self.derive_serial);
            self.auto_pending = job == DeriveJob::Auto;
        }
    }
    pub(super) fn reapply(&mut self) {
        self.request_derive(DeriveJob::Derive);
    }
    pub(super) fn receive_derivations(&mut self, ctx: &egui::Context) {
        let Some((_, receiver)) = &self.deriver else {
            return;
        };
        let mut latest = None;
        while let Ok(result) = receiver.try_recv() {
            latest = Some(result);
        }
        let Some(result) = latest else {
            return;
        };
        if result.raw_version != self.raw_version {
            return;
        }
        // A result some later request has superseded (the slider moved while
        // Auto ran) still shows, but its Auto tolerance must not overwrite
        // the slider the user has moved since.
        let current = self
            .derive_pending
            .is_none_or(|pending| pending == result.serial);
        if self.derive_pending == Some(result.serial) {
            self.derive_pending = None;
        }
        if result.auto {
            self.auto_pending = false;
        }
        match result.outcome {
            Ok((tolerance, derived)) => {
                if result.auto && current {
                    if let Some(tolerance) = tolerance {
                        self.simplify = true;
                        self.simplify_tolerance = tolerance as f32;
                        self.simplify_pick = Some(tolerance);
                    }
                }
                self.set_document(ctx, derived);
            }
            Err(error) => self.set_status(StatusKind::Error, error),
        }
    }
    /// Round `node` with `reach`, or change the reach of an already rounded one.
    pub(super) fn round_node(&mut self, node: Point, reach: Reach) {
        match self.rounded.iter_mut().find(|r| same_point(&r.at, &node)) {
            Some(existing) if existing.reach == reach => return,
            Some(existing) => existing.reach = reach,
            None => self.rounded.push(Rounded { at: node, reach }),
        }
        self.reapply();
    }
    pub(super) fn restore_node(&mut self, node: Point) {
        if let Some(index) = self.rounded.iter().position(|r| same_point(&r.at, &node)) {
            self.rounded.remove(index);
            self.reapply();
        }
    }
    /// A click rounds tightly; a second click restores the corner.
    pub(super) fn toggle_node(&mut self, node: Point) {
        if self.rounding_of(&node).is_some() {
            self.restore_node(node);
        } else {
            self.round_node(node, Reach::Tight);
        }
    }
    pub(super) fn rounding_of(&self, node: &Point) -> Option<Reach> {
        self.rounded
            .iter()
            .find(|r| same_point(&r.at, node))
            .map(|r| r.reach)
    }
    /// Forget the conversion result; the user's edits (rounded corners,
    /// deleted shapes, merges) stay and apply to the next result.
    pub(super) fn clear_result(&mut self) {
        self.raw_document = None;
        self.raw_version += 1;
        self.raw_counts = (0, 0);
        self.document = None;
        self.shown = None;
        self.shown_margin = 0.;
        self.document_version += 1;
        self.node_counts = None;
        self.segment_counts = None;
        self.palette.clear();
        self.node_menu = None;
        self.shape_menu = None;
        self.selected.clear();
        self.islands = None;
        self.nodes_cache = None;
        self.save_open = false;
        self.stat_popup = None;
        self.derive_pending = None;
        self.auto_pending = false;
        self.preview = None;
        self.working_source = None;
        self.working_sharp = None;
        self.tile = None;
        self.tile_pending = None;
        self.elapsed = None;
    }
    /// Forget the edits too: a different image is coming.
    pub(super) fn clear_edits(&mut self) {
        self.rounded.clear();
        self.moved.clear();
        self.deleted_nodes.clear();
        self.straightened.clear();
        self.deleted.clear();
        self.prep.recolors.clear();
        self.converted_prep = None;
        self.working = None;
        self.working_opaque = false;
        self.has_alpha = false;
        self.border_opaque = false;
    }
    pub(super) fn close(&mut self) {
        self.clear_result();
        self.clear_edits();
        self.forget_edits();
        self.path.clear();
        self.loaded_path.clear();
        self.raster = None;
        self.source = None;
        self.source_sharp = None;
        self.detected = None;
        self.zoom = 1.;
        self.fit = 1.;
        self.scroll = Vec2::ZERO;
        self.set_status(
            StatusKind::Info,
            "Open an image, or drop one onto the source card.",
        );
    }
    /// Open the picture at `path`. A file that cannot be opened leaves the
    /// picture already open as it was, with its result, edits and Undo (the
    /// review of September 23, 2026: a bad file dropped on the window
    /// cleared them all first).
    pub(super) fn load(&mut self, ctx: &egui::Context) {
        let loaded =
            crate::load_raster_up_to(std::path::Path::new(&self.path), crate::DESKTOP_MAX_PIXELS);
        self.take_loaded(ctx, loaded);
    }
    /// Open the picture `name` from its file's bytes: a file dropped on or
    /// picked in the browser tab, which has no paths.
    pub(super) fn load_bytes(&mut self, ctx: &egui::Context, name: String, bytes: &[u8]) {
        self.path = name;
        let loaded = crate::decode_raster_up_to(bytes, crate::BROWSER_MAX_PIXELS);
        self.take_loaded(ctx, loaded);
    }
    fn take_loaded(&mut self, ctx: &egui::Context, loaded: Result<Raster, String>) {
        let loaded = loaded.and_then(crate::fit_for_engine);
        if let Err(error) = &loaded {
            if self.raster.is_some() {
                self.path.clone_from(&self.loaded_path);
                self.set_status(
                    StatusKind::Error,
                    format!("{error} The open image stays as it was."),
                );
                return;
            }
        }
        self.clear_result();
        self.clear_edits();
        self.forget_edits();
        self.source = None;
        self.source_sharp = None;
        self.raster = None;
        self.loaded_path.clear();
        self.detected = None;
        self.zoom = 1.;
        self.fit = 1.;
        self.scroll = Vec2::ZERO;
        self.source_size = None;
        match loaded {
            Ok((raster, scaled_from)) => {
                self.source_size = scaled_from;
                self.load_source_textures(ctx, &raster);
                self.detected = Some(crate::auto::detect(&raster));
                let shortcut = ctx.format_shortcut(&SC_CONVERT);
                let size = match scaled_from {
                    Some((width, height)) => format!(
                        "{width} \u{00D7} {height} px scaled to {} \u{00D7} {} px, the largest \
                         size it converts ({} px)",
                        raster.width,
                        raster.height,
                        crate::engine::SIDE_LIMITS.end()
                    ),
                    None => format!("{} \u{00D7} {} px loaded", raster.width, raster.height),
                };
                self.set_status(
                    StatusKind::Info,
                    format!("{size}. Convert ({shortcut}) traces it."),
                );
                self.loaded_path = self.path.clone();
                self.has_alpha = vector_rebuild::prepare::has_transparency(&raster);
                // The sticker's widths follow the picture; the switch, colours
                // and shadow stay as the user left them.
                let sized = Sticker::for_size(raster.width, raster.height);
                self.sticker.border = sized.border;
                self.sticker.edge = sized.edge;
                self.raster = Some(raster);
            }
            Err(error) => self.set_status(StatusKind::Error, error),
        }
    }
    /// The source's textures, no larger per side than the GPU takes.
    pub(super) fn load_source_textures(&mut self, ctx: &egui::Context, raster: &Raster) {
        let image = display_image(raster, ctx.input(|i| i.max_texture_side));
        self.source_sharp =
            Some(ctx.load_texture("source-sharp", image.clone(), egui::TextureOptions::NEAREST));
        self.source = Some(ctx.load_texture("source", image, egui::TextureOptions::LINEAR));
    }
    /// The image the engine saw; the source card keeps showing the original
    /// unless asked for this one.
    pub(super) fn load_working_textures(&mut self, ctx: &egui::Context, raster: &Raster) {
        let image = display_image(raster, ctx.input(|i| i.max_texture_side));
        self.working_sharp = Some(ctx.load_texture(
            "working-sharp",
            image.clone(),
            egui::TextureOptions::NEAREST,
        ));
        self.working_source =
            Some(ctx.load_texture("working", image, egui::TextureOptions::LINEAR));
    }
    /// Prepare the image, let Auto look at what the engine will see, trace it,
    /// and derive the shown document with the user's edits applied.
    pub(super) fn process(
        raster: Raster,
        prep: &Preparation,
        mut options: VectorizeOptions,
        automatic: bool,
        sliders: Option<Sliders>,
        settings: &DeriveSettings,
        stop: &AtomicBool,
    ) -> JobResult {
        let working = if prep.is_identity() {
            raster
        } else {
            prep.apply(&raster)
        };
        let detected = automatic.then(|| crate::auto::detect(&working));
        if let Some(d) = detected {
            options.category = d.category;
            options.quality = d.quality;
        }
        if let Some(sliders) = sliders {
            options.advanced = Some(crate::engine::advanced_settings(options.category, sliders)?);
        }
        let mut raw = crate::engine::vectorize(&working, options)?;
        if stop.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        if let Some(palette) = prep.palette_of(&working) {
            raw = raw.snapped(&palette, None);
        }
        // A merged region traces in its neighbour's colour give or take a
        // level of the engine's own estimate; make it the colour asked for.
        // Pixels made transparent trace as no fill at all.
        let targets: Vec<[u8; 3]> = prep
            .recolors
            .iter()
            .filter(|r| r.alpha.is_none())
            .map(|r| r.rgb)
            .collect();
        if !targets.is_empty() {
            raw = raw.snapped(&targets, Some(MERGE_SNAP));
        }
        // With Auto settings on, a simplification that is on gets its Auto
        // tolerance too, as if the button had been pressed.
        let (shown, auto_tolerance) = if automatic && settings.simplify.is_some() {
            let base = raw.without_islands(&settings.deleted)?;
            let (tolerance, simplified) = crate::auto_simplify_tolerance(&base)?;
            (Self::finish(simplified, settings)?, Some(tolerance))
        } else {
            (Self::derive(&raw, settings)?, None)
        };
        let presented = Presented::of(&shown, settings.sticker.as_ref())?;
        Ok(Converted {
            working_opaque: working.pixels.iter().all(|p| p.0[3] == 255),
            working,
            detected,
            raw_counts: (raw.nodes().len(), raw.segment_count()),
            raw,
            derived: Derived::new(shown, presented),
            auto_tolerance,
            prep: prep.clone(),
            derived_with: settings.clone(),
            dropped: None,
        })
    }
    pub(super) fn start(&mut self) {
        self.start_with(None);
    }
    /// Convert on a thread of its own; with `drop`, work out the recolouring
    /// that drops that colour there first and convert with it added to the
    /// merges. The result arrives in `ui`, or never once cancelled.
    fn start_with(&mut self, drop: Option<ColourDrop>) {
        let Some(raster) = self.raster.clone() else {
            return;
        };
        let sliders = self.slider_settings();
        let options = self.options;
        let automatic = self.automatic;
        let mut prep = self.prep.clone();
        let settings = self.derive_settings();
        let previous_inputs = self.converted_inputs.replace(self.conversion_inputs());
        self.inputs_changed = None;
        self.held_inputs = None;
        let (sender, receiver) = mpsc::channel();
        self.worker = Some(receiver);
        self.started = Stopwatch::start();
        // Kept so that Cancel can put it back, and drawn until the result
        // arrives.
        let shown_nodes = (self.nodes && self.document.is_some()).then(|| self.current_nodes());
        let shown_tile = self.tile.take();
        self.replaced = self.document.is_some().then(|| {
            Box::new(Replaced {
                raw_document: self.raw_document.clone(),
                raw_counts: self.raw_counts,
                document: self.document.clone(),
                shown: self.shown.clone(),
                shown_margin: self.shown_margin,
                node_counts: self.node_counts,
                segment_counts: self.segment_counts,
                palette: self.palette.clone(),
                preview: self.preview.clone(),
                working_source: self.working_source.clone(),
                working_sharp: self.working_sharp.clone(),
                elapsed: self.elapsed,
                inputs: previous_inputs,
                nodes: shown_nodes,
                tile: shown_tile,
            })
        });
        self.clear_result();
        self.set_status(StatusKind::Busy, "Creating vector curves\u{2026}");
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        platform::spawn(move || {
            let result = std::panic::catch_unwind(move || -> JobResult {
                let dropped = drop.map(|drop| {
                    let recolors = colour_drop(&drop.islands, &drop.working, &drop.hex);
                    let pixels = recolors.iter().map(|r| r.pixels.len()).sum();
                    prep.recolors.extend(recolors);
                    (drop.hex, pixels)
                });
                if stop.load(Ordering::Relaxed) {
                    return Err(CANCELLED.into());
                }
                let mut converted =
                    Self::process(raster, &prep, options, automatic, sliders, &settings, &stop)?;
                converted.dropped = dropped;
                Ok(converted)
            })
            .unwrap_or_else(|_| {
                Err("Conversion failed unexpectedly; the source file was not changed.".into())
            });
            let _ = sender.send(result);
        });
    }
    /// Stop the running conversion: it skips its remaining stages (the
    /// engine itself runs its current stage to the end), its result is
    /// dropped, the result it replaced comes back and the edits stay for the
    /// next Convert. Until its thread has stopped, `idle` is false, so
    /// cancelling and converting again never runs two conversions at once
    /// (round two of the Opus 5.5 review: every Convert and Cancel left one
    /// more engine running).
    pub(super) fn cancel(&mut self) {
        if let Some(restored) = self.stop_conversion() {
            // Convert automatically leaves the settings as they are until
            // they change again, rather than start what was just stopped;
            // the result put back keeps the settings it was made with.
            self.held_inputs = Some(self.conversion_inputs());
            self.inputs_changed = None;
            self.set_status(
                StatusKind::Info,
                if restored {
                    "Conversion cancelled; the previous result is back."
                } else {
                    "Conversion cancelled. Convert starts it again."
                },
            );
        }
    }
    /// Stop the running conversion and put back the result it replaced;
    /// whether there was one, or `None` when nothing was running.
    pub(super) fn stop_conversion(&mut self) -> Option<bool> {
        let receiver = self.worker.take()?;
        self.stop.store(true, Ordering::Relaxed);
        self.stopping = Some(receiver);
        self.raw_version += 1;
        let restored = self.restore_replaced();
        // Curves, Nodes or Sticker changes made while it ran had no
        // document to apply to (round three of the Opus 5.5 review).
        if restored {
            self.reapply();
        }
        Some(restored)
    }
    /// Put back the result a conversion replaced; whether there was one.
    pub(super) fn restore_replaced(&mut self) -> bool {
        let Some(replaced) = self.replaced.take() else {
            return false;
        };
        let Replaced {
            raw_document,
            raw_counts,
            document,
            shown,
            shown_margin,
            node_counts,
            segment_counts,
            palette,
            preview,
            working_source,
            working_sharp,
            elapsed,
            inputs,
            nodes: _,
            tile: _,
        } = *replaced;
        self.raw_document = raw_document;
        self.raw_counts = raw_counts;
        self.document = document;
        self.shown = shown;
        self.shown_margin = shown_margin;
        self.document_version += 1;
        self.node_counts = node_counts;
        self.segment_counts = segment_counts;
        self.palette = palette;
        self.preview = preview;
        self.working_source = working_source;
        self.working_sharp = working_sharp;
        self.elapsed = elapsed;
        self.converted_inputs = inputs;
        true
    }
    pub(super) fn accept(&mut self, ctx: &egui::Context, converted: Converted) {
        let Converted {
            working,
            working_opaque,
            detected,
            raw,
            raw_counts,
            derived,
            auto_tolerance,
            prep,
            derived_with,
            dropped,
        } = converted;
        self.load_working_textures(ctx, &working);
        // Auto settings picked the tolerance, unless simplification was
        // changed while the conversion ran.
        let simplify_kept = self.simplify_settings() == derived_with.simplify;
        let mut made_with = derived_with;
        if let Some(tolerance) = auto_tolerance {
            made_with.simplify = Some(tolerance);
            if simplify_kept {
                self.simplify_tolerance = tolerance as f32;
                self.simplify_pick = Some(tolerance);
            }
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        self.elapsed = Some(elapsed);
        match &dropped {
            Some((hex, 0)) => self.set_status(
                StatusKind::Info,
                format!("No pixel of {hex} could change; the vector is as it was."),
            ),
            Some((hex, pixels)) => self.set_status(
                StatusKind::Done,
                format!("Dropped {hex} ({pixels} pixels changed). Vector ready in {elapsed:.2} s."),
            ),
            None => self.set_status(StatusKind::Done, format!("Vector ready in {elapsed:.2} s.")),
        }
        if detected.is_some() {
            self.detected = detected;
        }
        self.border_opaque = border_opaque(&working);
        self.working_opaque = working_opaque;
        self.working = Some(Arc::new(working));
        // A dropped colour's recolouring joins the merges, and the inputs the
        // conversion counts as started with, so Convert automatically does
        // not take it for a change.
        self.prep.recolors.clone_from(&prep.recolors);
        if let Some(inputs) = &mut self.converted_inputs {
            inputs.prep.recolors.clone_from(&prep.recolors);
        }
        self.converted_prep = Some(prep);
        self.raw_document = Some(Arc::new(raw));
        self.raw_counts = raw_counts;
        if self.straighten_auto {
            if let Some(bow) = self.auto_bow() {
                self.straighten_tolerance = bow;
            }
        }
        self.set_document(ctx, derived);
        self.replaced = None;
        // Curves, Nodes, Shapes or Sticker changes made while it ran had no
        // document to apply to; derive them now.
        let mut asked = self.derive_settings();
        if simplify_kept {
            asked.simplify = made_with.simplify;
        }
        if asked != made_with {
            self.reapply();
        }
        // Auto's tolerance and bow are no step of Undo; a dropped colour is.
        self.settle_edits();
    }
    /// Nothing the toolbar waits for: no conversion, no dialog and no file
    /// being written.
    pub(super) fn idle(&self) -> bool {
        self.worker.is_none() && self.errand.is_none() && self.stopping.is_none()
    }
    /// Run `job` off the UI thread as the errand the toolbar waits for;
    /// `receive_errand` acts on its answer.
    fn send_errand(&mut self, kind: ErrandKind, job: impl FnOnce() -> Fetched + Send + 'static) {
        // One errand at a time: a second (a keyboard shortcut while a dialog
        // is open) would replace the first and drop its answer.
        if self.errand.is_some() {
            return;
        }
        self.save_open = false;
        let (sender, receiver) = mpsc::channel();
        platform::spawn(move || {
            let _ = sender.send(job());
        });
        self.errand = Some(Errand {
            kind,
            receiver,
            before: (self.status.clone(), self.status_kind),
        });
    }
    pub(super) fn open_dialog(&mut self) {
        if platform::IN_BROWSER {
            platform::ask(platform::Command::OpenPicker);
            self.set_status(StatusKind::Info, "Choose an image in the file chooser.");
            return;
        }
        self.send_errand(ErrandKind::Open, || {
            Fetched::Picked(file_dialog(false, "", Format::Svg))
        });
        self.set_status(StatusKind::Info, "Choose an image in the Open dialog.");
    }
    /// The system dialog, filtered to the chosen format; the format picked in
    /// the dialog wins if the user changes it there.
    pub(super) fn save_dialog(&mut self) {
        if platform::IN_BROWSER {
            self.download();
            return;
        }
        let (stem, format) = (self.export_stem(), self.save_format);
        self.send_errand(ErrandKind::Save, move || {
            Fetched::Picked(file_dialog(true, &stem, format))
        });
        self.set_status(StatusKind::Info, "Choose where to save in the Save dialog.");
    }
    pub(super) fn save_preview_png(&mut self) {
        if platform::IN_BROWSER {
            self.snapshot_path = Some(PathBuf::from(format!(
                "{}-app-preview.png",
                self.export_stem()
            )));
            platform::ask(platform::Command::Screenshot);
            return;
        }
        self.send_errand(ErrandKind::Preview, || Fetched::Picked(file_dialog_png()));
        self.set_status(
            StatusKind::Info,
            "Choose where to save the app preview in the dialog.",
        );
    }
    /// Write the document as saved to `path` off the UI thread: an SVG, or a
    /// PDF or EPS through the local exporter.
    pub(super) fn save_to(&mut self, path: PathBuf) {
        let svg = match self.export_svg() {
            Some(Ok(svg)) => svg,
            Some(Err(error)) => {
                self.set_status(StatusKind::Error, error);
                return;
            }
            None => {
                self.set_status(
                    StatusKind::Error,
                    "Nothing to save: convert the image first.",
                );
                return;
            }
        };
        let source = PathBuf::from(&self.loaded_path);
        let target = path.clone();
        self.send_errand(ErrandKind::Write(path.clone()), move || {
            Fetched::Written(crate::export::write_vector(&source, &target, &svg))
        });
        // Nothing converts while an errand is out, so the clock is free.
        self.started = Stopwatch::start();
        self.set_status(
            StatusKind::Busy,
            format!("Saving {}\u{2026}", path.display()),
        );
    }
    /// Act on the errand's answer once it arrives.
    pub(super) fn receive_errand(&mut self, ctx: &egui::Context) {
        let Some(errand) = &self.errand else {
            return;
        };
        let fetched = match errand.receiver.try_recv() {
            Ok(fetched) => fetched,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Fetched::Written(Err(
                "The dialog or the exporter stopped unexpectedly.".into(),
            )),
        };
        let Some(Errand { kind, before, .. }) = self.errand.take() else {
            return;
        };
        match (kind, fetched) {
            (ErrandKind::Write(path), Fetched::Written(Ok(()))) => {
                self.set_status(StatusKind::Done, format!("Saved {}", path.display()));
            }
            (_, Fetched::Written(Err(error))) => self.set_status(StatusKind::Error, error),
            (_, Fetched::Written(Ok(()))) | (ErrandKind::Write(_), Fetched::Picked(_)) => {}
            (_, Fetched::Picked(None)) => self.set_status(before.1, before.0),
            (ErrandKind::Open, Fetched::Picked(Some((path, _)))) => {
                self.path = path;
                self.load(ctx);
            }
            (ErrandKind::Save, Fetched::Picked(Some((path, filter)))) => {
                let mut path = PathBuf::from(path);
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !matches!(ext.as_str(), "svg" | "pdf" | "eps") {
                    path.set_extension(Format::from_filter_index(filter).extension());
                }
                self.save_to(path);
            }
            (ErrandKind::Preview, Fetched::Picked(Some((path, _)))) => {
                let mut path = PathBuf::from(path);
                path.set_extension("png");
                if crate::same_file(std::path::Path::new(&self.loaded_path), &path) {
                    self.set_status(
                        StatusKind::Error,
                        "Preview must not replace the source image.",
                    );
                } else {
                    self.set_status(before.1, before.0);
                    self.snapshot_path = Some(path);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
            }
        }
    }
    /// The source file's name without its extension: what the saved file is
    /// called unless the user renames it.
    pub(super) fn export_stem(&self) -> String {
        Path::new(&self.loaded_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("vector")
            .to_owned()
    }
    pub(super) fn export_name(&self) -> String {
        format!("{}.{}", self.export_stem(), self.save_format.extension())
    }
    /// The saved size at 1x, in pixels: the picture opened, also when it was
    /// scaled down for the engine (the curves scale exactly), plus the
    /// sticker's margin on every side. Each side scales by its own ratio, so
    /// 1x is the opened size to the pixel (one ratio for both made 5000 by
    /// 401 save as 5000 by 400; round three of the Opus 5.5 review).
    pub(super) fn unit_size(&self) -> Option<(f32, f32)> {
        let raster = self.raster.as_ref()?;
        let margin = 2. * self.picture_margin();
        let (w, h) = self.source_size.unwrap_or((raster.width, raster.height));
        let side = |opened: usize, traced: usize| {
            (traced as f32 + margin) * opened as f32 / traced.max(1) as f32
        };
        Some((side(w, raster.width), side(h, raster.height)))
    }
    /// The saved size in pixels: `unit_size` times the output scale.
    pub(super) fn output_size(&self) -> Option<(u32, u32)> {
        let (w, h) = self.unit_size()?;
        let side = |px: f32| ((px * self.output_scale).round() as u32).max(1);
        Some((side(w), side(h)))
    }
    pub(super) fn enlarged(&self) -> bool {
        (self.output_scale - 1.).abs() > 1e-6
    }
    /// The document as it is saved: the shown curves, declared at the output
    /// size in pixels, its numbers written short. The engine's own file declares the same numbers in
    /// points, which is why it opened a third larger in browsers and looked
    /// small in editors; the app always saves what the footer shows.
    pub(super) fn export_svg(&self) -> Option<Result<String, String>> {
        let shown = self.shown.as_ref()?;
        let (width, height) = self.output_size()?;
        // Its numbers written short (`crate::export::compact_svg`).
        let short = |svg: String| crate::export::compact_svg(&svg);
        if self.save_stacked {
            return Some(crate::resize_svg(shown, width, height).map(short));
        }
        let document = self.document.as_ref()?;
        let sticker = self.sticker_settings();
        Some(
            Presented::drawing(document, sticker.as_ref(), false)
                .and_then(|cut| crate::resize_svg(&cut, width, height))
                .map(short),
        )
    }
    /// What the drag file holds when written for what is shown now.
    fn stage_key(&self) -> Option<StageKey> {
        self.document.as_ref()?;
        Some(StageKey {
            version: self.document_version,
            format: self.save_format,
            size: self.output_size()?,
            stacked: self.save_stacked,
        })
    }
    /// Keep the Save popup's drag file written for what is shown. A PDF or
    /// EPS runs the local exporter, which takes a moment, so it is written
    /// off the UI thread before the drag (which must start while the button
    /// is still held); a write already running finishes before the next.
    pub(super) fn stage_export(&mut self) {
        #[cfg(not(feature = "desktop"))]
        {
            self.staged = self.stage_key().map(|key| Staged {
                key,
                path: PathBuf::new(),
                writing: None,
                error: None,
            });
        }
        #[cfg(feature = "desktop")]
        self.stage_drag_file();
    }
    #[cfg(feature = "desktop")]
    fn stage_drag_file(&mut self) {
        if let Some(staged) = &mut self.staged {
            if let Some(receiver) = &staged.writing {
                match receiver.try_recv() {
                    Ok(result) => {
                        staged.writing = None;
                        staged.error = result.err();
                    }
                    Err(mpsc::TryRecvError::Empty) => return,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        staged.writing = None;
                        staged.error = Some("The exporter stopped unexpectedly.".into());
                    }
                }
            }
        }
        let Some(key) = self.stage_key() else {
            return;
        };
        if self.staged.as_ref().is_some_and(|s| s.key == key) {
            return;
        }
        let file = crate::dragout::staging_path(&self.export_name()).and_then(|path| {
            match self.export_svg() {
                Some(svg) => svg.map(|svg| (path, svg)),
                None => Err("Convert an image first.".into()),
            }
        });
        self.staged = Some(match file {
            // Every format is written on a thread of its own, the SVG too:
            // written here, a size typed or dragged in the popup rewrote and
            // synced the whole file on the UI thread every frame (round two
            // of the Opus 5.5 review). While one write is out, the next waits
            // (the early return above), so a drag writes a handful, not one
            // per frame.
            Ok((path, svg)) => {
                let (sender, receiver) = mpsc::channel();
                let (source, target) = (PathBuf::from(&self.loaded_path), path.clone());
                platform::spawn(move || {
                    let _ = sender.send(crate::export::write_vector(&source, &target, &svg));
                });
                Staged {
                    key,
                    path,
                    writing: Some(receiver),
                    error: None,
                }
            }
            Err(error) => Staged {
                key,
                path: PathBuf::new(),
                writing: None,
                error: Some(error),
            },
        });
    }
    /// Whether the drag file is written for what is shown now.
    pub(super) fn stage_state(&self) -> StageState {
        match &self.staged {
            Some(staged) if Some(staged.key) == self.stage_key() && staged.writing.is_none() => {
                match &staged.error {
                    Some(error) => StageState::Failed(error.clone()),
                    None => StageState::Ready(staged.path.clone()),
                }
            }
            _ => StageState::Writing,
        }
    }
    /// Hand the staged file to the system drag: the user drops it on the
    /// desktop, a folder or a program, which copies it from the staging folder.
    pub(super) fn drag_out(&mut self) {
        #[cfg(not(feature = "desktop"))]
        self.download();
        #[cfg(feature = "desktop")]
        self.drag_file();
    }
    /// In a browser tab: hand the page the saved file to download.
    pub(super) fn download(&mut self) {
        let name = self.export_name();
        let saved = match self.export_svg() {
            Some(svg) => {
                svg.and_then(|svg| crate::export::vector_bytes(self.save_format.kind(), &svg))
            }
            None => Err("Nothing to save: convert the image first.".into()),
        };
        match saved {
            Ok(data) => {
                self.save_open = false;
                let size = match self.output_size() {
                    Some((w, h)) => format!(" at {w} \u{00D7} {h} px"),
                    None => String::new(),
                };
                self.set_status(StatusKind::Done, format!("Downloaded {name}{size}."));
                platform::ask(platform::Command::Download {
                    name,
                    mime: self.save_format.mime(),
                    data,
                });
            }
            Err(error) => self.set_status(StatusKind::Error, error),
        }
    }
    /// In a browser tab: hand the page the app preview to download.
    pub(super) fn download_png(&mut self, path: &Path, rgba: &[u8], width: usize, height: usize) {
        let mut data = Vec::new();
        let encoded = image::codecs::png::PngEncoder::new(&mut data);
        match image::ImageEncoder::write_image(
            encoded,
            rgba,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        ) {
            Ok(()) => {
                let name = path.to_string_lossy().into_owned();
                self.set_status(StatusKind::Done, format!("Downloaded app preview {name}"));
                platform::ask(platform::Command::Download {
                    name,
                    mime: "image/png",
                    data,
                });
            }
            Err(e) => self.set_status(StatusKind::Error, e.to_string()),
        }
    }
    #[cfg(feature = "desktop")]
    fn drag_file(&mut self) {
        let name = self.export_name();
        let staged = match self.stage_state() {
            StageState::Ready(path) => path,
            StageState::Failed(error) => {
                self.set_status(StatusKind::Error, error);
                return;
            }
            StageState::Writing => {
                self.set_status(
                    StatusKind::Info,
                    format!("{name} is still being written; drag it again in a moment."),
                );
                return;
            }
        };
        match crate::dragout::drag_file(&staged) {
            Ok(crate::dragout::DragOutcome::Dropped) => {
                self.save_open = false;
                let size = match self.output_size() {
                    Some((w, h)) => format!(" at {w} \u{00D7} {h} px"),
                    None => String::new(),
                };
                self.set_status(StatusKind::Done, format!("Dropped {name}{size}."));
            }
            Ok(crate::dragout::DragOutcome::Cancelled) => {}
            Err(error) => self.set_status(StatusKind::Error, error),
        }
    }
    pub(super) fn effective_category(&self) -> Option<ImageCategory> {
        if self.automatic {
            self.detected.map(|d| d.category)
        } else {
            Some(self.options.category)
        }
    }
    /// The source quality the next conversion uses: Auto's, from the
    /// picture's size, or the one chosen.
    pub(super) fn effective_quality(&self) -> Option<Quality> {
        if self.automatic {
            self.detected.map(|d| d.quality)
        } else {
            Some(self.options.quality)
        }
    }
    /// The zoom factors (times fit) that keep the display scale within the
    /// limits `ZOOM_OUT` and `ZOOM_IN_*` set; 1:1 is always among them.
    pub(super) fn zoom_range(&self) -> std::ops::RangeInclusive<f32> {
        let fit = self.fit.max(1e-3);
        (fit.min(1.) * ZOOM_OUT / fit)..=(ZOOM_IN_PIXELS.max(ZOOM_IN_FIT * fit) / fit)
    }
    pub(super) fn set_zoom(&mut self, zoom: f32) {
        let range = self.zoom_range();
        self.zoom = zoom.clamp(*range.start(), *range.end());
    }
    pub(super) fn files_hovering(ctx: &egui::Context) -> bool {
        ctx.input(|i| !i.raw.hovered_files.is_empty())
    }
}

/// The recolouring that drops the fill colour `hex` from `working`, the image
/// the engine saw, given the shown document's shapes in paint order. The
/// dropped shapes' pixels, widened by two so the blended edge goes with them,
/// take the colour of the nearest shape of another colour, or turn
/// transparent where the transparent background is nearest (its hidden RGB,
/// often black, would otherwise paint a band the picture never showed); every
/// pixel within three more takes its own shape's colour, so the blend around
/// goes crisp. Pixels that hold their target already are left out. The masks
/// cost each shape's box, the rest a few passes over the image.
pub(super) fn colour_drop(islands: &[Island], working: &Raster, hex: &str) -> Vec<Recolor> {
    let (width, height) = (working.width, working.height);
    let gone = crate::parse_rgb(hex);
    // Which colour owns each pixel, in paint order; the dropped shapes' pixels.
    let mut owner: Vec<Option<[u8; 3]>> = vec![None; width * height];
    let mut dropped = vec![false; width * height];
    for island in islands {
        let Some(rgb) = crate::parse_rgb(&island.color) else {
            continue;
        };
        let mine = island.color.eq_ignore_ascii_case(hex);
        for index in island.covered(width, height, mine) {
            let index = index as usize;
            owner[index] = Some(rgb);
            dropped[index] |= mine;
        }
    }
    let near = |i: usize| -> [Option<usize>; 4] {
        let (x, y) = (i % width, i / width);
        [
            (x > 0).then(|| i - 1),
            (x + 1 < width).then(|| i + 1),
            (y > 0).then(|| i - width),
            (y + 1 < height).then(|| i + width),
        ]
    };
    let dilated = |mask: &[bool]| -> Vec<bool> {
        (0..width * height)
            .map(|i| mask[i] || near(i).iter().flatten().any(|&n| mask[n]))
            .collect()
    };
    let dropped = dilated(&dropped);
    // The blend around the dropped shapes goes crisp too: every pixel
    // within a few more of them takes its own shape's colour.
    let mut band = dropped.clone();
    for _ in 0..3 {
        band = dilated(&band);
    }
    // Flood the nearest other colour, or the transparency, into the dropped
    // pixels: a colour, with alpha 0 where the pixel turns transparent.
    let mut fill: Vec<Option<([u8; 3], Option<u8>)>> = vec![None; width * height];
    let mut queue = std::collections::VecDeque::new();
    // Only a pixel next to a dropped one can fill anything; seeding just
    // those, still in index order, gives the same flood as seeding every
    // pixel of the image (round two of the Opus 5.5 review).
    let borders_drop = |i: usize| near(i).into_iter().flatten().any(|n| dropped[n]);
    for i in (0..width * height).filter(|&i| !dropped[i] && borders_drop(i)) {
        let seed = match owner[i] {
            Some(rgb) => (Some(rgb) != gone).then_some((rgb, None)),
            None => {
                let [r, g, b, a] = working.pixels[i].0;
                if a == 0 {
                    Some(([r, g, b], Some(0)))
                } else {
                    (Some([r, g, b]) != gone).then_some(([r, g, b], None))
                }
            }
        };
        if seed.is_some() {
            fill[i] = seed;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let target = fill[i];
        for n in near(i).into_iter().flatten() {
            if dropped[n] && fill[n].is_none() {
                fill[n] = target;
                queue.push_back(n);
            }
        }
    }
    let mut by_target: std::collections::BTreeMap<([u8; 3], Option<u8>), Vec<u32>> =
        std::collections::BTreeMap::new();
    for i in 0..width * height {
        let target = if dropped[i] {
            fill[i]
        } else if band[i] {
            owner[i].map(|rgb| (rgb, None))
        } else {
            None
        };
        if let Some((rgb, alpha)) = target {
            let [r, g, b, a] = working.pixels[i].0;
            let changes = match alpha {
                Some(alpha) => a != alpha,
                None => [r, g, b] != rgb,
            };
            if changes {
                by_target.entry((rgb, alpha)).or_default().push(i as u32);
            }
        }
    }
    by_target
        .into_iter()
        .map(|((rgb, alpha), pixels)| Recolor { pixels, rgb, alpha })
        .collect()
}
