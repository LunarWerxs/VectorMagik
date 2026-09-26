//! Part of `desktop_ui`: Undo and Redo, moving nodes by hand, and converting
//! again by itself when a conversion setting changes.

use super::*;

impl Desktop {
    /// The Conversion and Advanced cards' settings as a conversion takes them.
    pub(super) fn conversion_settings(&self) -> ConversionSettings {
        ConversionSettings {
            automatic: self.automatic,
            manual: (!self.automatic).then_some((self.options.category, self.options.quality)),
            overlap_opaque_photos: self.options.overlap_opaque_photos,
            optional_optimizer: self.options.optional_optimizer,
            sliders: self.slider_settings(),
        }
    }
    pub(super) fn conversion_inputs(&self) -> ConversionInputs {
        ConversionInputs {
            prep: self.prep.clone(),
            settings: self.conversion_settings(),
        }
    }

    /// The edits as they stand.
    pub(super) fn edits(&self) -> Edits {
        Edits {
            rounded: self.rounded.clone(),
            straightened: self.straightened.clone(),
            moved: self.moved.clone(),
            deleted_nodes: self.deleted_nodes.clone(),
            deleted: self.deleted.clone(),
            prep: self.prep.clone(),
            conversion: self.conversion_settings(),
            simplify: self.simplify,
            simplify_tolerance: self.simplify_tolerance,
            regularize: self.regularize,
            primitives: self.primitives,
            straighten: self.straighten,
            straighten_tolerance: self.straighten_tolerance,
            straighten_auto: self.straighten_auto,
            sticker_on: self.sticker_on,
            sticker: self.sticker,
        }
    }
    /// Whether the edits still stand as `seen`, compared in place: a colour
    /// drop's recolouring can hold a photograph's worth of pixel indices, so
    /// nothing is copied per frame to find out.
    fn edits_are(&self, seen: &Edits) -> bool {
        seen.rounded == self.rounded
            && seen.straightened == self.straightened
            && seen.moved == self.moved
            && seen.deleted_nodes == self.deleted_nodes
            && seen.deleted == self.deleted
            && seen.prep == self.prep
            && seen.conversion == self.conversion_settings()
            && seen.simplify == self.simplify
            && seen.simplify_tolerance == self.simplify_tolerance
            && seen.regularize == self.regularize
            && seen.primitives == self.primitives
            && seen.straighten == self.straighten
            && seen.straighten_tolerance == self.straighten_tolerance
            && seen.straighten_auto == self.straighten_auto
            && seen.sticker_on == self.sticker_on
            && seen.sticker == self.sticker
    }
    /// Make whatever changed since the last call one step of Undo. Called at
    /// the end of every frame in which no pointer button is down, so a slider
    /// dragged or a node carried is one step, not one per frame.
    pub(super) fn record_edits(&mut self) {
        match &self.edits_seen {
            Some(seen) if self.edits_are(seen) => {}
            Some(_) => {
                if let Some(before) = self.edits_seen.replace(self.edits()) {
                    self.push_undo(before);
                }
            }
            None => self.edits_seen = Some(self.edits()),
        }
    }
    fn push_undo(&mut self, before: Edits) {
        self.undo.push(before);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// Take the edits as they are without a step of Undo: what a finished
    /// conversion changed by itself (Auto's tolerance and bow). A colour
    /// dropped comes back with the conversion; that is the user's step.
    pub(super) fn settle_edits(&mut self) {
        let dropped = self
            .edits_seen
            .as_ref()
            .is_some_and(|seen| seen.prep.recolors != self.prep.recolors);
        if dropped {
            if let Some(before) = self.edits_seen.take() {
                self.push_undo(before);
            }
        }
        self.edits_seen = Some(self.edits());
    }
    /// A new picture, or none: nothing to undo or redo.
    pub(super) fn forget_edits(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.edits_seen = None;
        self.converted_inputs = None;
        self.inputs_changed = None;
        self.held_inputs = None;
        self.node_drag = None;
    }
    /// Put back the edits before the last change.
    pub(super) fn undo(&mut self) {
        self.record_edits();
        let Some(previous) = self.undo.pop() else {
            self.set_status(StatusKind::Info, "Nothing to undo.");
            return;
        };
        if previous.prep.recolors != self.prep.recolors && !self.idle() {
            self.undo.push(previous);
            self.set_status(
                StatusKind::Info,
                "That undo converts again: press it once the conversion has finished.",
            );
            return;
        }
        let current = self.edits();
        self.redo.push(current);
        self.restore_edits(previous);
        self.set_status(StatusKind::Info, "Undone. Ctrl+Y redoes it.");
    }
    /// Put back the change Undo took away.
    pub(super) fn redo(&mut self) {
        self.record_edits();
        let Some(next) = self.redo.pop() else {
            self.set_status(StatusKind::Info, "Nothing to redo.");
            return;
        };
        if next.prep.recolors != self.prep.recolors && !self.idle() {
            self.redo.push(next);
            self.set_status(
                StatusKind::Info,
                "That redo converts again: press it once the conversion has finished.",
            );
            return;
        }
        let current = self.edits();
        self.undo.push(current);
        self.restore_edits(next);
        self.set_status(StatusKind::Info, "Redone.");
    }
    /// Put `edits` in force: derive the shown document again when a hand
    /// edit or a Curves or Sticker setting changed, convert again when the
    /// merges did (they were converted when made). Other conversion settings
    /// wait for Convert, or for Convert automatically, as when they are set
    /// by hand.
    fn restore_edits(&mut self, edits: Edits) {
        let derive_before = self.derive_settings();
        let merges_before = std::mem::take(&mut self.prep.recolors);
        let Edits {
            rounded,
            straightened,
            moved,
            deleted_nodes,
            deleted,
            prep,
            conversion,
            simplify,
            simplify_tolerance,
            regularize,
            primitives,
            straighten,
            straighten_tolerance,
            straighten_auto,
            sticker_on,
            sticker,
        } = edits.clone();
        self.rounded = rounded;
        self.straightened = straightened;
        self.moved = moved;
        self.deleted_nodes = deleted_nodes;
        self.deleted = deleted;
        self.prep = prep;
        self.automatic = conversion.automatic;
        if let Some((category, quality)) = conversion.manual {
            self.options.category = category;
            self.options.quality = quality;
        }
        self.options.overlap_opaque_photos = conversion.overlap_opaque_photos;
        self.options.optional_optimizer = conversion.optional_optimizer;
        self.advanced_on = conversion.sliders.is_some();
        if let Some(sliders) = conversion.sliders {
            self.sliders = sliders;
        }
        self.simplify = simplify;
        self.simplify_tolerance = simplify_tolerance;
        self.regularize = regularize;
        self.primitives = primitives;
        self.straighten = straighten;
        self.straighten_tolerance = straighten_tolerance;
        self.straighten_auto = straighten_auto;
        self.sticker_on = sticker_on;
        self.sticker = sticker;
        self.edits_seen = Some(edits);
        self.node_menu = None;
        self.shape_menu = None;
        self.node_drag = None;
        self.selected.clear();
        let merges_changed = merges_before != self.prep.recolors;
        let convertible = self.raster.is_some()
            && self.path == self.loaded_path
            && self.converted_inputs.is_some();
        if merges_changed && convertible {
            self.start();
        } else if self.derive_settings() != derive_before {
            self.reapply();
        }
    }

    /// Where the Curves card's passes left a node now shown at `node`: the
    /// node itself, or where it was before the user moved it.
    pub(super) fn moved_from(&self, node: &Point) -> Option<Point> {
        self.moved
            .iter()
            .find(|m| same_point(&m.to, node))
            .map(|m| m.from)
    }
    pub(super) fn base_position(&self, node: &Point) -> Point {
        self.moved_from(node).unwrap_or(*node)
    }
    /// Move the node shown at `at` to `to` (written as the document writes
    /// numbers). A node moved before keeps where it first came from, and a
    /// node carried back onto that spot is simply not moved any more. Its
    /// rounding goes with it: rounding runs after the moves, so it is keyed
    /// where the node now is.
    /// A node dragged again before the picture from its last move arrived
    /// still shows where it came from, so a move is also found by that
    /// place (the review of September 23, 2026: the second drag was added as
    /// a second move of the same node, which the document ignores).
    pub(super) fn move_node(&mut self, at: Point, to: Point) {
        let to = vector_rebuild::nodes::written(to);
        if same_point(&at, &to) {
            return;
        }
        let index = self
            .moved
            .iter()
            .position(|m| same_point(&m.to, &at))
            .or_else(|| self.moved.iter().position(|m| same_point(&m.from, &at)));
        let was = index.map_or(at, |i| self.moved[i].to);
        match index {
            Some(i) if same_point(&self.moved[i].from, &to) => {
                self.moved.remove(i);
            }
            Some(i) => self.moved[i].to = to,
            None => self.moved.push(NodeMove { from: at, to }),
        }
        for rounded in &mut self.rounded {
            if same_point(&rounded.at, &at) || same_point(&rounded.at, &was) {
                rounded.at = to;
            }
        }
        self.reapply();
    }
    /// Put the node shown at `node` back where the trace had it.
    pub(super) fn put_back(&mut self, node: Point) {
        if let Some(index) = self.moved.iter().position(|m| same_point(&m.to, &node)) {
            let moved = self.moved.remove(index);
            for rounded in &mut self.rounded {
                if same_point(&rounded.at, &moved.to) {
                    rounded.at = moved.from;
                }
            }
            self.reapply();
        }
    }
    /// Put every moved node back. A node moved and then deleted stays
    /// moved: its deletion is keyed where the move took it, and deleting it
    /// kept the shape it had there.
    pub(super) fn put_all_back(&mut self) {
        let (deleted, back): (Vec<NodeMove>, Vec<NodeMove>) = std::mem::take(&mut self.moved)
            .into_iter()
            .partition(|m| self.is_deleted(&m.to));
        self.moved = deleted;
        for moved in back {
            for rounded in &mut self.rounded {
                if same_point(&rounded.at, &moved.to) {
                    rounded.at = moved.from;
                }
            }
        }
        self.reapply();
    }
    /// The moved nodes still in the drawing, which Put back can return.
    pub(super) fn moves_shown(&self) -> usize {
        self.moved
            .iter()
            .filter(|m| !self.is_deleted(&m.to))
            .count()
    }
    fn is_deleted(&self, node: &Point) -> bool {
        self.deleted_nodes.iter().any(|d| same_point(d, node))
    }
    /// Why the node shown at `node` cannot be deleted, or `None`: worked out
    /// on the document the deletions apply to (before the rounding, which
    /// cuts a rounded corner's node out), once per node and document.
    pub(super) fn deletion_refusal(&mut self, node: &Point) -> Option<&'static str> {
        if let Some((version, at, refusal)) = self.deletable {
            if version == self.document_version && same_point(&at, node) {
                return refusal;
            }
        }
        let refusal = match &self.document {
            Some(document) => {
                let svg = document
                    .unrounded
                    .as_deref()
                    .map_or(document.svg(), String::as_str);
                match vector_rebuild::nodes::deletion_refusal(svg, *node) {
                    // Shown but not there before the rounding: one of the
                    // two ends of a rounded corner's arc.
                    Ok(Some(vector_rebuild::nodes::ABSENT))
                        if document.unrounded.is_some()
                            && document.nodes().iter().any(|n| same_point(n, node)) =>
                    {
                        Some(
                            "This node ends a rounded corner's arc: restore the corner to \
                             delete nodes there.",
                        )
                    }
                    Ok(refusal) => refusal,
                    Err(_) => Some("The drawing could not be read."),
                }
            }
            None => Some("Wait for the conversion to finish."),
        };
        self.deletable = Some((self.document_version, *node, refusal));
        refusal
    }
    /// Delete the node shown at `node` from the drawing: its two pieces
    /// become one cubic fitted to the curve they drew. A rounded corner's
    /// rounding goes with it. Refused, with the reason in the status line, at
    /// a junction, at an open outline's end and on an outline of three nodes.
    pub(super) fn delete_node(&mut self, node: Point) {
        if let Some(reason) = self.deletion_refusal(&node) {
            self.set_status(StatusKind::Info, reason);
            return;
        }
        if self.is_deleted(&node) {
            return;
        }
        self.rounded.retain(|r| !same_point(&r.at, &node));
        self.deleted_nodes.push(node);
        self.node_menu = None;
        self.reapply();
        self.set_status(
            StatusKind::Info,
            "Node deleted, its curve refitted. Ctrl+Z brings it back.",
        );
    }
    /// Bring every deleted node back.
    pub(super) fn restore_deleted_nodes(&mut self) {
        if !self.deleted_nodes.is_empty() {
            self.deleted_nodes.clear();
            self.reapply();
        }
    }
    /// Begin dragging the node marker shown at `at`, with the pieces it
    /// pulls along for the preview drawn while it moves.
    pub(super) fn begin_node_drag(&mut self, at: Point) {
        // A rounded corner is cut out of the shown document; its pieces are
        // the ones the document had before rounding.
        let reach = self.rounding_of(&at);
        let document = self.document.as_ref();
        let pieces = document
            .and_then(|d| {
                let svg = match (reach, &d.unrounded) {
                    (Some(_), Some(before)) => before.as_str(),
                    _ => d.svg(),
                };
                vector_rebuild::nodes::pieces_at(svg, at).ok()
            })
            .unwrap_or_default();
        let frame = document.and_then(|d| vector_rebuild::simplify::viewbox_size(d.svg()));
        let rounding = reach
            .zip(frame)
            .map(|(reach, frame)| (reach.fraction(), frame));
        self.node_menu = None;
        self.node_drag = Some(NodeDrag {
            from: at,
            to: at,
            pieces,
            rounding,
        });
    }

    /// Whether the settings are the ones a Cancel stopped, or a conversion
    /// failed with: Convert automatically waits for them to change.
    pub(super) fn held(&self) -> bool {
        self.held_inputs.as_ref().is_some_and(|held| {
            held.prep == self.prep && held.settings == self.conversion_settings()
        })
    }

    /// Convert again by itself once the conversion settings (preparation,
    /// image type and quality, the Advanced card) differ from the ones the
    /// shown or running conversion was started with and have rested for
    /// `AUTO_CONVERT_DELAY` with no pointer button down. A running
    /// conversion of settings that are gone is stopped first. Only after the
    /// picture has been converted once: opening a picture never converts.
    pub(super) fn auto_convert_tick(&mut self, ctx: &egui::Context) {
        let ready = self.auto_convert && self.raster.is_some() && self.path == self.loaded_path;
        let Some(started) = self.converted_inputs.as_ref().filter(|_| ready) else {
            self.inputs_changed = None;
            return;
        };
        if (started.prep == self.prep && started.settings == self.conversion_settings())
            || self.held()
        {
            self.inputs_changed = None;
            return;
        }
        // Shown and not running: settings that trace the same picture (Auto
        // switched off on the type it detected) need no conversion.
        if self.worker.is_none()
            && self.document.is_some()
            && !self.conversion_stale()
            && !self.advanced_stale()
        {
            self.converted_inputs = Some(self.conversion_inputs());
            self.inputs_changed = None;
            return;
        }
        let inputs = self.conversion_inputs();
        let since = match &self.inputs_changed {
            Some((seen, at)) if *seen == inputs => *at,
            _ => {
                let now = Stopwatch::start();
                self.inputs_changed = Some((inputs, now));
                now
            }
        };
        let waited = since.elapsed();
        if ctx.input(|i| i.pointer.any_down()) || waited < AUTO_CONVERT_DELAY {
            ctx.request_repaint_after(
                AUTO_CONVERT_DELAY
                    .saturating_sub(waited)
                    .max(Duration::from_millis(30)),
            );
            return;
        }
        if self.worker.is_some() {
            self.stop_conversion();
        }
        if !self.idle() {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        self.inputs_changed = None;
        self.start();
        self.set_status(
            StatusKind::Busy,
            "Settings changed; converting again\u{2026}",
        );
    }
}
