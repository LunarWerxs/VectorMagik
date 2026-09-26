//! The desktop's own tests.
use super::*;
use std::time::Instant;
fn run_frame(ctx: &egui::Context, app: &mut Desktop) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 800.),
            )),
            ..Default::default()
        },
        |ctx| app.ui(ctx),
    );
}
/// Pump frames until the derive thread has answered the latest request.
fn settle(ctx: &egui::Context, app: &mut Desktop) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.derive_pending.is_some() {
        assert!(Instant::now() < deadline, "derivation stalled");
        run_frame(ctx, app);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn conversion_finishes_at_preview_without_opening_a_save_dialog() {
    let ctx = egui::Context::default();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-with-blending-small.png");
    let mut app = Desktop::snapshot_state(
        &ctx,
        Some(&source),
        false,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        false,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    assert!(ctx.style().visuals.dark_mode && app.automatic && app.document.is_none());
    app.start();
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.worker.is_some() {
        assert!(Instant::now() < deadline, "Preview completion stalled");
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.document.is_some() && app.preview.is_some() && app.snapshot_path.is_none(),
        "{}",
        app.status
    );
    assert_eq!(app.status_kind, StatusKind::Done);
    assert!(app.elapsed.is_some());
    assert_eq!(
        app.document.as_ref().unwrap().preset,
        app.raw_document.as_ref().unwrap().preset
    );
    // The shown document is the simplified one; the engine's own output is
    // kept. Simplifying never adds a node; the passes after it may (true
    // lines and circles draw a whole circle in four quarters, true shapes a
    // rounded rectangle in eight), so the count that must not grow is the
    // simplified one's.
    let (raw_nodes, _) = app.node_counts.unwrap();
    let mut settings = app.derive_settings();
    settings.regularize = None;
    settings.straighten = None;
    settings.primitives = false;
    let simplified = Desktop::derive(app.raw_document.as_ref().unwrap(), &settings)
        .unwrap()
        .nodes()
        .len();
    assert!(simplified <= raw_nodes, "{simplified} of {raw_nodes} nodes");
    assert_eq!(app.raw_document.as_ref().unwrap().nodes().len(), raw_nodes);
    assert!(app
        .raw_document
        .as_ref()
        .unwrap()
        .simplify_tolerance
        .is_none());
    assert!(app.document.as_ref().unwrap().simplify_tolerance.is_some());
    // Switching simplification, the true lines and circles and the true
    // shapes off restores the engine's document without reconverting; the
    // derive thread answers within a few frames.
    app.simplify = false;
    app.regularize = false;
    app.primitives = false;
    app.reapply();
    assert!(app.derive_pending.is_some());
    settle(&ctx, &mut app);
    assert_eq!(app.node_counts, Some((raw_nodes, raw_nodes)));
    assert!(app.document.as_ref().unwrap().simplify_tolerance.is_none());
    // A click rounds a node tightly: the corner node goes and the two cut
    // points come, so one node more; widening it to the whole pieces
    // leaves one node fewer; a second click restores the document exactly.
    let before = app.document.as_ref().unwrap().svg().to_owned();
    let node = app.document.as_ref().unwrap().nodes()[3];
    let version = app.document_version;
    app.toggle_node(node);
    settle(&ctx, &mut app);
    assert_eq!(app.rounding_of(&node), Some(Reach::Tight));
    assert!(app.document_version > version);
    let rounded = app.document.as_ref().unwrap();
    assert_eq!(rounded.nodes().len(), raw_nodes + 1);
    assert!(!rounded.nodes().iter().any(|n| same_point(n, &node)));
    assert_ne!(rounded.svg(), before);
    let tight = rounded.svg().to_owned();
    app.round_node(node, Reach::Wide);
    settle(&ctx, &mut app);
    assert_eq!(app.rounding_of(&node), Some(Reach::Wide));
    // Wide cuts further: the whole pieces when the cap allows (one node
    // fewer), or capped cuts that still add the two cut points.
    let wide = app.document.as_ref().unwrap();
    assert!(
        wide.nodes().len() == raw_nodes - 1 || wide.nodes().len() == raw_nodes + 1,
        "{} nodes",
        wide.nodes().len()
    );
    assert_ne!(wide.svg(), tight);
    // A second node keeps its own reach; the first stays wide.
    let other = app.document.as_ref().unwrap().nodes()[9];
    app.round_node(other, Reach::Medium);
    settle(&ctx, &mut app);
    assert_eq!(app.rounded.len(), 2);
    assert_eq!(app.rounding_of(&node), Some(Reach::Wide));
    assert_eq!(app.rounding_of(&other), Some(Reach::Medium));
    app.restore_node(other);
    app.toggle_node(node);
    settle(&ctx, &mut app);
    assert!(app.rounded.is_empty());
    assert_eq!(app.document.as_ref().unwrap().svg(), before);
    // The saved document is the shown one declared in pixels: the source
    // size at 1x (the engine's file says points), the chosen size
    // otherwise; nothing but the declaration changes. The name follows
    // the source.
    let raster = app.raster.as_ref().unwrap();
    let declared = format!(
        "width=\"{}pt\" height=\"{}pt\"",
        raster.width, raster.height
    );
    assert!(before.contains(&declared), "the engine declares points");
    // Saved as shown: stacked (vector_rebuild::stacking).
    let stacked = vector_rebuild::stacking::stack_svg(&before).unwrap().0;
    let declared_as = |w: u32, h: u32| {
        crate::export::compact_svg(&stacked.replacen(
            &declared,
            &format!("width=\"{w}\" height=\"{h}\""),
            1,
        ))
    };
    assert!(!app.enlarged());
    assert_eq!(
        app.output_size(),
        Some((raster.width as u32, raster.height as u32))
    );
    assert_eq!(
        app.export_svg().unwrap().unwrap(),
        declared_as(raster.width as u32, raster.height as u32)
    );
    assert_eq!(app.export_name(), "logo-with-blending-small.svg");
    app.output_scale = 4.;
    assert!(app.enlarged());
    let (w, h) = app.output_size().unwrap();
    assert_eq!((w, h), (raster.width as u32 * 4, raster.height as u32 * 4));
    assert_eq!(app.export_svg().unwrap().unwrap(), declared_as(w, h));
    app.save_format = Format::Pdf;
    assert_eq!(app.export_name(), "logo-with-blending-small.pdf");
    app.output_scale = 1.;
    // The palette lists every fill, most used first.
    let colors = app.document.as_ref().unwrap().colors();
    assert_eq!(colors.len(), app.document.as_ref().unwrap().color_count());
    assert!(colors.windows(2).all(|w| w[0].1 >= w[1].1));
    assert!(colors.iter().all(|(hex, _)| hex.starts_with('#')));
    // Auto picks a candidate tolerance and turns simplification on.
    app.request_derive(DeriveJob::Auto);
    assert!(app.auto_pending);
    settle(&ctx, &mut app);
    assert!(!app.auto_pending && app.simplify);
    assert!(crate::AUTO_TOLERANCE_CANDIDATES
        .iter()
        .any(|t| (*t as f32 - app.simplify_tolerance).abs() < 1e-6));
    assert!(app.node_counts.unwrap().1 <= raw_nodes);
    // Closing clears everything back to the empty state.
    app.close();
    assert!(app.raster.is_none() && app.document.is_none() && app.path.is_empty());
    assert!(app.deleted.is_empty() && app.rounded.is_empty() && app.working.is_none());
    run_frame(&ctx, &mut app);
}

/// A converted app on the small blended sample, settled.
fn converted(ctx: &egui::Context) -> Desktop {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-with-blending-small.png");
    Desktop::snapshot_state(
        ctx,
        Some(&source),
        true,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        false,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap()
}

#[test]
fn smooth_joins_switch_runs_the_optional_pass_and_changes_the_curves() {
    let ctx = egui::Context::default();
    let plain = converted(&ctx);
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-with-blending-small.png");
    let smooth = Desktop::snapshot_state(
        &ctx,
        Some(&source),
        true,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        true,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    let (off, on) = (
        plain.raw_document.as_ref().unwrap(),
        smooth.raw_document.as_ref().unwrap(),
    );
    assert!(!off.optional_optimizer && off.optimizer_unknowns == 0);
    assert!(on.optional_optimizer && on.optimizer_unknowns > 0);
    assert_ne!(off.svg, on.svg, "the pass moved no control point");
    // The guard's outcome reaches the card and the statistics.
    assert_eq!(super::cards::optimizer_note(off), None);
    assert!(on.optimizer_kept && on.optimizer_objective.is_some());
    assert_eq!(
        super::cards::optimizer_note(on),
        Some("Smooth joins applied.")
    );
    // Asked for with no step back (a pass that stopped): the card says so.
    let mut failed = (**on).clone();
    failed.optimizer_kept = false;
    failed.optimizer_objective = None;
    assert_eq!(
        super::cards::optimizer_note(&failed),
        Some("Smooth joins could not run on this picture; kept the plain curves.")
    );
    assert!(on.statistics_json().contains("\"optimizer_kept\":true"));
    assert!(!off.statistics_json().contains("optimizer_kept"));
    assert_eq!(off.segment_count(), on.segment_count());
    assert!(smooth.document.as_ref().unwrap().optional_optimizer);
}

#[test]
fn sticker_outlines_the_shapes_grows_the_picture_and_cuts_the_background() {
    let ctx = egui::Context::default();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-without-blending.png");
    let sticker = Sticker {
        border: 4.,
        edge: 8.,
        shadow: true,
        ..Sticker::default()
    };
    let mut app = Desktop::snapshot_state(
        &ctx,
        Some(&source),
        true,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        false,
        StickerChoice::Custom(sticker),
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    assert!(
        app.sticker_on && app.sticker == sticker,
        "the widths asked for win"
    );
    let raster = app.raster.as_ref().unwrap();
    let (w, h) = (raster.width as u32, raster.height as u32);
    // The sticker is painted and saved, never edited: the document keeps
    // the engine's shapes alone while the shown picture has the three
    // layers under them, and the picture grows by the sticker's reach.
    let document = app.document.as_ref().unwrap().svg().to_owned();
    let shown = app.shown.as_ref().unwrap().to_string();
    assert!(!document.contains("sticker-"));
    for layer in ["sticker-shadow", "sticker-edge", "sticker-border"] {
        assert!(shown.contains(layer), "{layer} missing");
    }
    assert!(shown.find("sticker-border").unwrap() < shown.find("<g id=\"#").unwrap());
    assert_eq!(sticker.margin(), 17., "4 + 8 + a shadow offset of 5");
    assert_eq!(app.picture_margin(), 17.);
    assert_eq!(app.output_size(), Some((w + 34, h + 34)));
    let exported = app.export_svg().unwrap().unwrap();
    assert!(exported.contains(&format!("width=\"{}\" height=\"{}\"", w + 34, h + 34)));
    assert!(exported.contains(&format!("viewBox=\"-17 -17 {} {}\"", w + 34, h + 34)));
    // The sticker's three layers copy every path of the stacked drawing.
    let stacked = vector_rebuild::stacking::stack_svg(&document).unwrap().0;
    assert_eq!(
        shown.matches("<path d=\"").count(),
        3 * stacked.matches(" d=\"").count()
    );
    // The white page around the mark is a background shape touching the
    // edge; cutting it out removes it like a deleted shape, so the
    // outline hugs the mark, and Restore deleted would bring it back.
    assert!(app.border_opaque);
    let before = app.current_islands().len();
    let cut = app.cut_background_now(&ctx).unwrap();
    assert!(cut >= 1);
    assert_eq!(app.deleted.len(), cut);
    let after = app.current_islands();
    assert_eq!(after.len(), before - cut);
    let touches_edge = |island: &Island| {
        island.min.x <= 1.
            || island.min.y <= 1.
            || island.max.x >= w as f64 - 1.
            || island.max.y >= h as f64 - 1.
    };
    assert!(
        !after
            .iter()
            .any(|i| i.color.eq_ignore_ascii_case("#ffffff") && touches_edge(i)),
        "no white shape touches the edge any more"
    );
    assert!(app.cut_background_now(&ctx).is_err(), "nothing left to cut");
    // Off again: the shown picture is the document itself at its own size.
    app.sticker_on = false;
    app.rederive_now(&ctx).unwrap();
    assert_eq!(app.picture_margin(), 0.);
    assert_eq!(app.output_size(), Some((w, h)));
    assert_eq!(
        app.shown.as_ref().unwrap().as_str(),
        vector_rebuild::stacking::stack_svg(app.document.as_ref().unwrap().svg())
            .unwrap()
            .0
    );
    // A transparent picture has no background to cut.
    let transparent = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-with-transparency.png");
    let mut app = Desktop::snapshot_state(
        &ctx,
        Some(&transparent),
        true,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        false,
        StickerChoice::Sized,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    assert!(!app.border_opaque);
    assert!(app.cut_background_now(&ctx).is_err());
    assert_eq!(
        app.sticker,
        Sticker::for_size(250, 250),
        "sized for the picture"
    );
    assert_eq!(app.picture_margin(), 9.);
    // The card draws with the sticker on and the tile thread renders the
    // widened picture at the widened width.
    run_frame(&ctx, &mut app);
    app.zoom = 8.;
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.tile.is_none() && Instant::now() < deadline {
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(app.tile.is_some(), "no tile arrived");
}

#[test]
fn shapes_can_be_deleted_restored_and_merged() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    // Shapes: the shape under the picture's centre can be selected and
    // deleted without reconverting, and restored; a merge recolours the
    // image the engine sees and converts again with fewer colours.
    let centre = Point {
        x: app.raster.as_ref().unwrap().width as f64 / 2.,
        y: app.raster.as_ref().unwrap().height as f64 / 2.,
    };
    let islands = app.current_islands();
    let index = shapes::island_at(&islands, centre).expect("a shape at the centre");
    let shape = Removal {
        color: islands[index].color.clone(),
        at: centre,
    };
    app.shapes_mode = true;
    app.toggle_selected(shape.clone());
    assert_eq!(app.resolve_selection().len(), 1);
    let nodes_before = app.document.as_ref().unwrap().nodes().len();
    app.delete_selected();
    settle(&ctx, &mut app);
    assert!(app.selected.is_empty() && app.deleted.len() == 1);
    assert!(app.document.as_ref().unwrap().nodes().len() < nodes_before);
    let islands = app.current_islands();
    assert!(shapes::island_at(&islands, centre)
        .is_none_or(|i| !islands[i].color.eq_ignore_ascii_case(&shape.color)));
    app.deleted.clear();
    app.reapply();
    settle(&ctx, &mut app);
    assert_eq!(app.document.as_ref().unwrap().nodes().len(), nodes_before);
    // Merge the centre shape into a neighbouring colour.
    let islands = app.current_islands();
    let other = islands
        .iter()
        .find(|i| !i.color.eq_ignore_ascii_case(&shape.color))
        .expect("a second colour");
    let inside = Point {
        x: (other.min.x + other.max.x) / 2.,
        y: (other.min.y + other.max.y) / 2.,
    };
    let other_shape = Removal {
        color: other.color.clone(),
        at: if other.contains(inside) {
            inside
        } else {
            other.outline[0]
        },
    };
    app.selected = vec![other_shape.clone(), shape.clone()];
    app.merge_selected(Some(other_shape.color.clone()));
    assert!(app.worker.is_some() && app.prep.recolors.len() == 1);
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.worker.is_some() {
        assert!(Instant::now() < deadline, "merge conversion stalled");
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(&ctx, &mut app);
    let islands = app.current_islands();
    let merged = shapes::island_at(&islands, centre).expect("a shape at the centre");
    assert!(
        islands[merged]
            .color
            .eq_ignore_ascii_case(&other_shape.color),
        "the centre now traces as {} (wanted {})",
        islands[merged].color,
        other_shape.color
    );
    assert_eq!(app.converted_prep.as_ref(), Some(&app.prep));
    // The image the engine saw carries the recolouring; the loaded one does not.
    let recolored = app.prep.recolors[0].pixels[0] as usize;
    assert_ne!(
        app.working.as_ref().unwrap().pixels[recolored].0[..3],
        app.raster.as_ref().unwrap().pixels[recolored].0[..3]
    );
    assert!(app.prep.recolors.len() == 1);
    app.prep.recolors.clear();
    app.start();
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.worker.is_some() {
        assert!(Instant::now() < deadline, "reconversion stalled");
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.converted_prep, Some(Preparation::default()));
}

#[test]
fn save_popup_stages_the_file_and_the_convert_hint_follows_the_preparation() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    // Ctrl+S opens the popup; it draws with the Save button's anchor and
    // stays open across frames.
    assert!(!app.save_open);
    app.open_overlay(Overlay::Save, egui::pos2(0., 0.));
    assert!(app.save_open);
    run_frame(&ctx, &mut app);
    run_frame(&ctx, &mut app);
    assert!(app.save_open && app.save_anchor != egui::Rect::NOTHING);
    // The staged file for a drag is the export, written off the UI thread
    // while the popup is open, where the drag module keeps it; a new size
    // writes it again.
    app.save_format = Format::Svg;
    app.output_scale = 2.;
    let deadline = Instant::now() + Duration::from_secs(30);
    let staged = loop {
        run_frame(&ctx, &mut app);
        match app.stage_state() {
            StageState::Ready(path) => break path,
            StageState::Failed(error) => panic!("{error}"),
            StageState::Writing => {}
        }
        assert!(Instant::now() < deadline, "the drag file was never written");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(staged.ends_with("logo-with-blending-small.svg"));
    let written = std::fs::read_to_string(&staged).unwrap();
    assert_eq!(written, app.export_svg().unwrap().unwrap());
    assert!(written.contains(&format!(
        "width=\"{}\"",
        app.raster.as_ref().unwrap().width * 2
    )));
    assert!(crate::dragout::is_staged(&staged));
    app.output_scale = 1.;
    assert!(matches!(app.stage_state(), StageState::Writing));
    std::fs::remove_file(&staged).unwrap();
    // Changing the preparation after a result asks for another Convert.
    assert_eq!(app.converted_prep.as_ref(), Some(&app.prep));
    assert!(!app.conversion_stale());
    app.prep.colors = Some(4);
    assert_ne!(app.converted_prep.as_ref(), Some(&app.prep));
    assert!(app.conversion_stale());
    app.prep.colors = None;
    assert_eq!(app.converted_prep.as_ref(), Some(&app.prep));
    assert!(!app.conversion_stale());
    // So do the image type, the quality and the optional pass: switching
    // Auto off with other choices asks for it; the choices Auto made, or
    // Auto on again, do not.
    let detected = app.detected.unwrap();
    app.automatic = false;
    app.options.category = detected.category;
    app.options.quality = detected.quality;
    assert!(!app.conversion_stale());
    app.options.quality = Quality::Low;
    assert!(app.conversion_stale(), "quality");
    app.options.quality = detected.quality;
    app.options.category = ImageCategory::AliasedArtwork;
    assert!(app.conversion_stale(), "image type");
    app.automatic = true;
    assert!(!app.conversion_stale());
    app.options.optional_optimizer = true;
    assert!(app.conversion_stale(), "optional pass");
    app.options.optional_optimizer = false;
    // Seams only matter for an opaque photograph: not for this logo.
    app.options.overlap_opaque_photos = !app.options.overlap_opaque_photos;
    assert!(!app.conversion_stale());
    app.options.overlap_opaque_photos = !app.options.overlap_opaque_photos;
}

#[test]
fn colour_limit_and_background_reach_the_engine() {
    let ctx = egui::Context::default();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-with-transparency.png");
    let prep = Preparation {
        colors: Some(2),
        background: Some([255, 255, 255]),
        recolors: Vec::new(),
    };
    let app = Desktop::snapshot_state(
        &ctx,
        Some(&source),
        true,
        false,
        1.,
        None,
        None,
        prep.clone(),
        false,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    assert!(app.has_alpha, "the sample is transparent");
    let working = app.working.as_ref().unwrap();
    assert!(working.pixels.iter().all(|p| p.0[3] == 255), "flattened");
    let mut distinct: Vec<[u8; 3]> = Vec::new();
    for p in &working.pixels {
        let c = [p.0[0], p.0[1], p.0[2]];
        if !distinct.contains(&c) {
            distinct.push(c);
        }
    }
    assert_eq!(distinct.len(), 2, "{distinct:?}");
    assert!(app.document.as_ref().unwrap().color_count() <= 2);
    assert_eq!(app.converted_prep, Some(prep));
    assert!(app
        .raster
        .as_ref()
        .unwrap()
        .pixels
        .iter()
        .any(|p| p.0[3] < 255));
}

#[test]
fn auto_tolerance_straightening_overlay_and_dropping_a_colour_work_together() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    // Auto settings on: Convert picked the simplification tolerance too.
    assert!(app.automatic && app.simplify);
    assert!(crate::AUTO_TOLERANCE_CANDIDATES
        .iter()
        .any(|t| (*t as f32 - app.simplify_tolerance).abs() < 1e-6));
    // Straightening is on by default at Auto, the bow for anti-aliased
    // artwork, with the settings of the CLI's `--straighten auto` (the
    // engine crate's defaults otherwise, round three of the Opus 5.5
    // review); moving the slider leaves Auto.
    assert!(app.straighten_auto);
    assert_eq!(app.straighten_tolerance, 0.2);
    assert_eq!(
        app.straighten_settings(),
        Some(StraightenOptions {
            auto: true,
            ..StraightenOptions::default()
        })
    );
    // It changes the picture; off restores the engine's curves; a node
    // forced square changes it again.
    let straightened = app.document.as_ref().unwrap().svg().to_owned();
    app.straighten = false;
    app.rederive_now(&ctx).unwrap();
    let plain = app.document.as_ref().unwrap().svg().to_owned();
    assert_ne!(plain, straightened);
    assert!(plain.matches(" L ").count() < straightened.matches(" L ").count());
    app.straighten = true;
    app.rederive_now(&ctx).unwrap();
    assert_eq!(app.document.as_ref().unwrap().svg(), straightened);
    // A node inside the picture (the canvas corners are square already).
    let node = app
        .document
        .as_ref()
        .unwrap()
        .nodes()
        .into_iter()
        .find(|n| n.x > 1. && n.y > 1. && n.x < 249. && n.y < 249.)
        .unwrap();
    app.straightened.push(node);
    app.rederive_now(&ctx).unwrap();
    assert_ne!(app.document.as_ref().unwrap().svg(), straightened);
    app.straightened.clear();
    app.rederive_now(&ctx).unwrap();
    // The source card shows the original until the engine input is asked
    // for; the working textures exist after a conversion.
    assert!(app.working_source.is_some() && !app.show_engine_input);
    // One picture, switched by B and V, draws in both states; folded
    // cards and the selecting button draw too.
    app.set_view(View::Overlay, false);
    run_frame(&ctx, &mut app);
    app.set_view(View::Overlay, true);
    app.collapsed = [true; 7];
    app.shapes_mode = true;
    run_frame(&ctx, &mut app);
    app.collapsed = [false; 7];
    app.set_view(View::SideBySide, true);
    // Dropping the least used colour recolours its shapes to the nearest
    // other colour and converts again with one colour fewer.
    let before = app.palette.clone();
    assert!(before.len() >= 2);
    let dropped = before.last().unwrap().0.clone();
    app.remove_color(&dropped);
    // The pixels are worked out on the conversion's thread: nothing is
    // recoloured on this one, and the result brings the recolouring back.
    assert!(app.worker.is_some() && app.prep.recolors.is_empty());
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.worker.is_some() {
        assert!(Instant::now() < deadline, "drop conversion stalled");
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(&ctx, &mut app);
    assert!(app.document.is_some(), "{}", app.status);
    assert!(!app.prep.recolors.is_empty(), "{}", app.status);
    assert_eq!(app.converted_prep.as_ref(), Some(&app.prep));
    assert!(
        app.status.starts_with(&format!("Dropped {dropped}")),
        "{}",
        app.status
    );
    assert!(
        app.palette.len() < before.len(),
        "{:?} -> {:?}",
        before,
        app.palette
    );
    assert!(!app
        .palette
        .iter()
        .any(|(c, _)| c.eq_ignore_ascii_case(&dropped)));
    // The hover outline comes from the exact pieces.
    let islands = app.current_islands();
    assert!(islands.iter().all(|i| i.pieces.len() == 1 + i.holes.len()));
    assert!(islands.iter().all(|i| !i.pieces[0].is_empty()));
}

#[test]
fn zoomed_vector_gets_a_crisp_tile_from_the_render_thread() {
    let ctx = egui::Context::default();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-with-blending-small.png");
    let mut app = Desktop::snapshot_state(
        &ctx,
        Some(&source),
        true,
        false,
        8.,
        None,
        None,
        Preparation::default(),
        false,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.tile.is_none() {
        assert!(Instant::now() < deadline, "no crisp tile arrived");
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    let tile = app.tile.as_ref().unwrap();
    assert_eq!(tile.key.version, app.document_version);
    assert!(tile.key.size[0] > 0 && tile.key.size[1] > 0);
    // Fitting the picture again drops the tile: the base preview is enough.
    app.set_zoom(1.);
    run_frame(&ctx, &mut app);
    assert!(app.tile.is_none());
}

#[test]
fn a_scaled_display_gets_a_tile_in_its_own_pixels() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    // The 250 px picture's preview has 4 pixels per picture pixel; at 6
    // points per picture pixel a display at 100% wants a tile of the view.
    let scale = 6.;
    let (size, viewport) = (Vec2::splat(250. * scale), Vec2::new(400., 300.));
    app.request_tile(scale, 1., size, viewport, 8192);
    let one = app.tile_pending.take().expect("a tile at 100%");
    // The same view on a display at 200%: twice the pixels each way, the
    // same picture area.
    app.request_tile(scale, 2., size, viewport, 8192);
    let two = app.tile_pending.take().expect("a tile at 200%");
    assert_eq!((one.ppp, two.ppp), (1., 2.));
    assert_eq!(one.origin, two.origin);
    for axis in 0..2 {
        let (a, b) = (one.size[axis] as f32, two.size[axis] as f32);
        assert!(
            (b - 2. * a).abs() <= 2.,
            "{:?} against {:?}",
            one.size,
            two.size
        );
    }
    // At half the zoom the preview is enough at 100% and not at 200%.
    app.request_tile(scale / 2., 1., size / 2., viewport, 8192);
    assert!(app.tile_pending.is_none());
    app.request_tile(scale / 2., 2., size / 2., viewport, 8192);
    assert!(app.tile_pending.is_some());
}

#[test]
fn every_icon_has_a_glyph() {
    let ctx = egui::Context::default();
    let _app = Desktop::blank(&ctx);
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    ctx.fonts_mut(|fonts| {
        for glyph in icon::ALL {
            assert!(
                fonts.has_glyphs(&FontId::proportional(14.), glyph),
                "no glyph for {glyph:?}"
            );
        }
        assert!(fonts.has_glyphs(
            &FontId::proportional(14.),
            "\u{00D7}\u{00B7}\u{2026}\u{2212}\u{2014}\u{2192}"
        ));
        assert!(fonts.has_glyphs(&FontId::monospace(10.5), "Ctrl+Enter"));
    });
}

#[test]
fn layout_stacks_only_when_it_shows_a_larger_picture() {
    // Square images stay side by side in a landscape window.
    assert!(!stacked_layout(Vec2::new(910., 690.), 1.));
    assert!(!stacked_layout(Vec2::new(560., 520.), 1.));
    // Tall images never stack.
    assert!(!stacked_layout(Vec2::new(910., 690.), 0.5));
    // Wide images and portrait windows stack.
    assert!(stacked_layout(Vec2::new(910., 690.), 3.));
    assert!(stacked_layout(Vec2::new(600., 1200.), 1.));
}

#[test]
fn fill_colors_parse_and_the_rest_is_grey() {
    assert_eq!(parse_hex("#4fd1c5"), Color32::from_rgb(0x4f, 0xd1, 0xc5));
    assert_eq!(
        parse_hex("#4fd1c580"),
        Color32::from_rgba_unmultiplied(0x4f, 0xd1, 0xc5, 0x80)
    );
    assert_eq!(parse_hex("none"), Color32::from_gray(128));
    assert_eq!(Format::from_filter_index(2), Format::Pdf);
    assert_eq!(Format::Eps.filter_index(), 3);
}

#[test]
fn logo_renders_with_transparent_corners_and_an_opaque_center() {
    let rgba = render_logo(64).unwrap();
    assert_eq!(rgba.len(), 64 * 64 * 4);
    assert_eq!(rgba[3], 0, "corner must stay transparent");
    let center = (32 * 64 + 32) * 4;
    assert_eq!(rgba[center + 3], 255);
    let icon = window_icon().unwrap();
    assert_eq!((icon.width, icon.height), (64, 64));
}

#[test]
fn the_advanced_card_runs_the_sliders_and_seeds_from_the_picture() {
    let ctx = egui::Context::default();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples/logo-with-blending-small.png");
    let sliders = Sliders {
        segmentation: 5,
        smoothness: 9,
        curves: 6,
        corners: false,
    };
    let convert = |sliders: Option<Sliders>| {
        Desktop::snapshot_state(
            &ctx,
            Some(&source),
            true,
            false,
            1.,
            None,
            None,
            Preparation::default(),
            false,
            StickerChoice::Off,
            None,
            false,
            false,
            sliders,
        )
        .unwrap()
    };
    let plain = convert(None);
    let advanced = convert(Some(sliders));
    let doc = advanced.document.as_ref().unwrap();
    let expected =
        crate::engine::advanced_settings(ImageCategory::AntiAliasedArtwork, sliders).unwrap();
    assert_eq!(doc.advanced, Some(expected));
    assert!(expected.contour_anti_alias && !expected.detect_corners);
    assert_eq!(
        plain.document.as_ref().unwrap().advanced,
        Some(crate::engine::blended_defaults())
    );
    assert_ne!(doc.svg(), plain.document.as_ref().unwrap().svg());
    assert!(advanced.advanced_on && advanced.pending_advanced() == Some(Some(expected)));
    // Off, the card shows the settings in force (11, 3, 6 for a blended
    // logo at high quality, not the dialog's 5, 6, 6).
    let mut app = plain;
    run_frame(&ctx, &mut app);
    assert!(!app.advanced_on);
    assert_eq!(
        app.sliders,
        Sliders::of(Some(crate::engine::blended_defaults()))
    );
    // Switched on, the card starts from the settings in force: the advanced
    // defaults for a blended logo, the detail ceiling for a photograph.
    app.advanced_on = true;
    app.seed_sliders();
    assert_eq!(
        app.sliders,
        Sliders::of(Some(crate::engine::blended_defaults()))
    );
    assert_eq!(app.sliders.smoothness, 3);
    app.automatic = false;
    app.options.category = ImageCategory::Photograph;
    app.options.quality = Quality::High;
    app.seed_sliders();
    assert_eq!(
        app.sliders,
        Sliders::of(Some(crate::engine::photo_detail_ceiling()))
    );
    assert_eq!(app.sliders.segmentation, 12);
}

/// A sample picture from the kit's fixtures.
fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../kit/fixtures/samples")
        .join(name)
}

/// A scratch file of one test, in the system's temporary folder.
fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("vm-desktop-{}-{name}", std::process::id()))
}

/// `path` loaded and converted on this thread with the desktop's defaults:
/// Auto settings, simplification, true lines and circles, straightening.
fn converted_from(ctx: &egui::Context, path: &Path) -> Result<Desktop, String> {
    Desktop::snapshot_state(
        ctx,
        Some(path),
        true,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        false,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
}

/// Pump frames until the running conversion has arrived.
fn finish(ctx: &egui::Context, app: &mut Desktop) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while app.worker.is_some() {
        assert!(Instant::now() < deadline, "conversion stalled");
        run_frame(ctx, app);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Pump frames until the errand (a dialog or a save) has answered.
fn run_errand(ctx: &egui::Context, app: &mut Desktop) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.errand.is_some() {
        assert!(Instant::now() < deadline, "the errand never answered");
        run_frame(ctx, app);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One frame with these key presses and dropped files.
fn frame_with(ctx: &egui::Context, app: &mut Desktop, keys: &[Key], dropped: &[PathBuf]) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 800.),
            )),
            events: keys
                .iter()
                .map(|&key| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                })
                .collect(),
            dropped_files: dropped
                .iter()
                .map(|path| egui::DroppedFile {
                    path: Some(path.clone()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
        |ctx| app.ui(ctx),
    );
}

#[test]
fn the_advanced_card_expects_the_quality_auto_picks_for_a_small_picture() {
    let ctx = egui::Context::default();
    // The small blended logo at 120 px: Auto calls it blended artwork at
    // medium quality (its sides are under 160 px), which runs the basic
    // preset, not the blended advanced default of high quality.
    let small = scratch("small.png");
    let logo = image::open(sample("logo-with-blending-small.png"))
        .unwrap()
        .to_rgba8();
    image::imageops::resize(&logo, 120, 120, image::imageops::FilterType::Triangle)
        .save(&small)
        .unwrap();
    let app = converted_from(&ctx, &small);
    std::fs::remove_file(&small).unwrap();
    let mut app = app.unwrap();
    let detected = app.detected.unwrap();
    assert_eq!(
        (detected.category, detected.quality),
        (ImageCategory::AntiAliasedArtwork, Quality::Medium)
    );
    assert_eq!(app.document.as_ref().unwrap().advanced, None);
    // The card expects what ran, so no "Convert again to apply" stays up.
    assert_eq!(app.pending_advanced(), Some(None));
    assert!(!app.advanced_stale());
    // Switched on, the sliders start from the dialog's defaults, not from
    // the high-quality default the picture never got.
    app.advanced_on = true;
    app.seed_sliders();
    assert_eq!(app.sliders, Sliders::default());
    assert_ne!(
        app.sliders,
        Sliders::of(Some(crate::engine::blended_defaults()))
    );
    // Sliders on do change the next result, and the card says so.
    assert!(app.advanced_stale());
}

#[test]
fn edits_made_while_converting_apply_to_its_result() {
    let ctx = egui::Context::default();
    let mut app = Desktop::snapshot_state(
        &ctx,
        Some(&sample("logo-with-blending-small.png")),
        false,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        false,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    app.start();
    // While it runs, as the Curves and Sticker cards do: simplification
    // off, a sticker on. There is no document to derive from yet.
    app.simplify = false;
    app.reapply();
    app.sticker_on = true;
    app.reapply();
    assert!(app.derive_pending.is_none());
    finish(&ctx, &mut app);
    // The result came derived with the settings it started with; the ones
    // asked for since are derived at once. Meanwhile the picture, the nodes
    // over it and the saved size agree: no sticker painted, no margin.
    assert!(app.derive_pending.is_some());
    assert!(!app.shown.as_ref().unwrap().contains("sticker-"));
    assert_eq!(app.picture_margin(), 0.);
    settle(&ctx, &mut app);
    assert!(!app.simplify, "Auto's tolerance did not switch it back on");
    assert!(app.document.as_ref().unwrap().simplify_tolerance.is_none());
    assert!(app.shown.as_ref().unwrap().contains("sticker-"));
    let margin = app.sticker.margin() as f32;
    assert!(margin > 0.);
    assert_eq!(app.picture_margin(), margin);
    let raster = app.raster.as_ref().unwrap();
    let side = |px: usize| (px as f32 + 2. * margin).round() as u32;
    let (w, h) = (side(raster.width), side(raster.height));
    assert_eq!(app.output_size(), Some((w, h)));
    assert!(app
        .export_svg()
        .unwrap()
        .unwrap()
        .contains(&format!("width=\"{w}\" height=\"{h}\"")));
    // The counts came from the derive thread with the document: the nodes
    // the markers use, the segments and the palette, most used first.
    let document = app.document.as_ref().unwrap();
    assert_eq!(
        app.nodes_cache.as_ref().map(|(v, n)| (*v, n.len())),
        Some((app.document_version, document.nodes().len()))
    );
    let raw = app.raw_document.as_ref().unwrap();
    assert_eq!(
        app.node_counts,
        Some((raw.nodes().len(), document.nodes().len()))
    );
    let mut counted: Vec<(String, usize)> = Vec::new();
    for fill in document
        .svg()
        .split("fill=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
    {
        match counted.iter_mut().find(|(c, _)| c == fill) {
            Some((_, n)) => *n += 1,
            None => counted.push((fill.to_owned(), 1)),
        }
    }
    counted.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    assert_eq!(app.palette, counted);
}

#[test]
fn one_to_one_is_in_reach_whatever_the_fit() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    // A phone photo in the default window fits at about 0.1, a 32 px icon
    // at about 13; the 1 key shows either at one pixel per pixel.
    for fit in [0.106_f32, 1., 13.] {
        app.fit = fit;
        app.zoom = 1.;
        frame_with(&ctx, &mut app, &[Key::Num1], &[]);
        assert!(
            (app.fit * app.zoom - 1.).abs() < 1e-4,
            "1:1 at fit {fit} shows {}",
            app.fit * app.zoom
        );
        app.set_zoom(f32::MAX);
        assert!(
            app.fit * app.zoom >= ZOOM_IN_PIXELS - 1e-3,
            "fit {fit}: in to {}",
            app.fit * app.zoom
        );
        app.set_zoom(0.);
        assert!(
            (app.fit * app.zoom - fit.min(1.) * ZOOM_OUT).abs() < 1e-5,
            "fit {fit}: out to {}",
            app.fit * app.zoom
        );
        app.set_zoom(1.);
        assert_eq!(app.zoom, 1., "fit is always in range");
    }
}

#[test]
fn a_picture_over_the_engine_limit_loads_scaled_and_converts() {
    let ctx = egui::Context::default();
    let wide = scratch("wide.png");
    image::RgbaImage::from_fn(4500, 12, |x, y| {
        image::Rgba([(x / 18) as u8, if y < 6 { 40 } else { 200 }, 90, 255])
    })
    .save(&wide)
    .unwrap();
    let mut app = Desktop::blank(&ctx);
    app.path = wide.to_string_lossy().into_owned();
    app.load(&ctx);
    std::fs::remove_file(&wide).unwrap();
    let raster = app
        .raster
        .as_ref()
        .unwrap_or_else(|| panic!("{}", app.status));
    assert_eq!((raster.width, raster.height), (4096, 11));
    assert!(
        app.status.contains("4500 \u{00D7} 12 px") && app.status.contains("4096 \u{00D7} 11 px"),
        "{}",
        app.status
    );
    // Its textures fit egui's texture limit (the GPU's in the window); the
    // painter aborts on a larger one.
    let limit = ctx.input(|i| i.max_texture_side);
    for texture in [&app.source, &app.source_sharp] {
        assert!(texture.as_ref().unwrap().size().iter().all(|&s| s <= limit));
    }
    // It converts, where the engine used to refuse it.
    app.start();
    finish(&ctx, &mut app);
    assert!(app.document.is_some(), "{}", app.status);
    assert!(app
        .working_source
        .as_ref()
        .unwrap()
        .size()
        .iter()
        .all(|&s| s <= limit));
    // A strip too thin once scaled is refused, with the reason.
    let thin = scratch("thin.png");
    image::RgbaImage::from_pixel(9000, 3, image::Rgba([10, 20, 30, 255]))
        .save(&thin)
        .unwrap();
    let opened = app.loaded_path.clone();
    app.path = thin.to_string_lossy().into_owned();
    app.load(&ctx);
    std::fs::remove_file(&thin).unwrap();
    assert_eq!(app.status_kind, StatusKind::Error);
    assert!(app.status.contains("too narrow"), "{}", app.status);
    // The picture already open stays, with its result and its path, so
    // Convert still works on it.
    assert!(app.raster.is_some() && app.document.is_some());
    assert_eq!(
        (app.path.as_str(), app.loaded_path.as_str()),
        (opened.as_str(), opened.as_str())
    );
    // With nothing open, the refusal leaves nothing open.
    let mut empty = Desktop::blank(&ctx);
    let thin = scratch("thin-alone.png");
    image::RgbaImage::from_pixel(9000, 3, image::Rgba([10, 20, 30, 255]))
        .save(&thin)
        .unwrap();
    empty.path = thin.to_string_lossy().into_owned();
    empty.load(&ctx);
    std::fs::remove_file(&thin).unwrap();
    assert!(empty.raster.is_none() && empty.status.contains("too narrow"));
}

#[test]
fn saving_runs_off_the_ui_thread_and_the_toolbar_waits_for_it() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    let saved = scratch("saved.svg");
    app.save_to(saved.clone());
    assert!(!app.idle(), "the toolbar waits");
    assert_eq!(app.status_kind, StatusKind::Busy, "{}", app.status);
    run_errand(&ctx, &mut app);
    assert!(app.idle());
    assert_eq!(app.status_kind, StatusKind::Done, "{}", app.status);
    assert_eq!(
        std::fs::read_to_string(&saved).unwrap(),
        app.export_svg().unwrap().unwrap()
    );
    std::fs::remove_file(&saved).unwrap();
    // A save that fails says why and frees the toolbar.
    app.save_to(scratch("missing-folder").join("saved.svg"));
    run_errand(&ctx, &mut app);
    assert!(app.idle());
    assert_eq!(app.status_kind, StatusKind::Error, "{}", app.status);
}

#[test]
fn dropping_a_colour_beside_transparency_leaves_transparency() {
    let ctx = egui::Context::default();
    let mut app = converted_from(&ctx, &sample("logo-with-transparency.png")).unwrap();
    let working = app.working.clone().unwrap();
    let (width, height) = (working.width, working.height);
    let islands = app.current_islands();
    // The colour whose shapes border the transparent background most.
    let (hex, touching) = app
        .palette
        .iter()
        .map(|(hex, _)| {
            let touching = islands
                .iter()
                .filter(|island| island.color.eq_ignore_ascii_case(hex))
                .flat_map(|island| island.covered(width, height, true))
                .filter(|&i| working.pixels[i as usize].0[3] == 0)
                .count();
            (hex.clone(), touching)
        })
        .max_by_key(|(_, touching)| *touching)
        .unwrap();
    assert!(touching > 0, "no shape borders the transparency");
    // The transparent pixels hide white. Seeded with that, the flood painted
    // a white band the picture never showed; now the pixels nearest the
    // transparency turn transparent themselves.
    assert!(working
        .pixels
        .iter()
        .filter(|p| p.0[3] == 0)
        .all(|p| p.0[..3] == [255, 255, 255]));
    let recolors = state::colour_drop(&islands, &working, &hex);
    assert!(recolors
        .iter()
        .any(|r| r.alpha == Some(0) && !r.pixels.is_empty()));
    assert!(
        !recolors
            .iter()
            .any(|r| r.alpha.is_none() && r.rgb == [255, 255, 255]),
        "a white band"
    );
    // Converted again, the vector has no fill far from the colours it had.
    let before: Vec<[u8; 3]> = app
        .palette
        .iter()
        .filter_map(|(hex, _)| crate::parse_rgb(hex))
        .collect();
    app.remove_color(&hex);
    finish(&ctx, &mut app);
    settle(&ctx, &mut app);
    assert!(app.document.is_some(), "{}", app.status);
    assert!(
        !app.palette
            .iter()
            .any(|(c, _)| c.eq_ignore_ascii_case(&hex)),
        "{hex} is still there: {:?}",
        app.palette
    );
    for (fill, _) in &app.palette {
        let rgb = crate::parse_rgb(fill).unwrap();
        assert!(
            before
                .iter()
                .any(|b| b.iter().zip(rgb).all(|(x, y)| x.abs_diff(y) <= 24)),
            "new fill {fill} in {:?}",
            app.palette
        );
    }
    // The image the engine saw has those pixels transparent.
    let working = app.working.as_ref().unwrap();
    for recolor in app.prep.recolors.iter().filter(|r| r.alpha == Some(0)) {
        assert!(recolor
            .pixels
            .iter()
            .all(|&i| working.pixels[i as usize].0[3] == 0));
    }
}

#[test]
fn a_running_conversion_can_be_cancelled_and_a_drop_meanwhile_is_explained() {
    let ctx = egui::Context::default();
    let mut app = Desktop::snapshot_state(
        &ctx,
        Some(&sample("logo-with-blending-small.png")),
        false,
        false,
        1.,
        None,
        Some(DEFAULT_SIMPLIFY_TOLERANCE as f64),
        Preparation::default(),
        false,
        StickerChoice::Off,
        Some(Bow::Auto),
        true,
        DEFAULT_PRIMITIVES,
        None,
    )
    .unwrap();
    // A conversion that has not answered yet.
    let (sender, receiver) = mpsc::channel::<JobResult>();
    app.worker = Some(receiver);
    // A file dropped meanwhile is not opened, and the status says why.
    let other = sample("logo-without-blending.png");
    frame_with(&ctx, &mut app, &[], std::slice::from_ref(&other));
    assert!(app.loaded_path.ends_with("logo-with-blending-small.png"));
    assert!(
        app.status
            .starts_with("logo-without-blending.png was not opened")
            && app.status.contains("cancel"),
        "{}",
        app.status
    );
    // Escape cancels: the window stops waiting for the result, but a new
    // conversion waits until the cancelled one has stopped, so Convert and
    // Cancel cannot pile conversions up.
    frame_with(&ctx, &mut app, &[Key::Escape], &[]);
    assert!(app.worker.is_none() && app.stopping.is_some() && !app.idle());
    assert!(
        app.status.starts_with("Conversion cancelled"),
        "{}",
        app.status
    );
    // Its late answer goes nowhere, and the window is idle again.
    assert!(sender.send(Err("late".into())).is_ok());
    run_frame(&ctx, &mut app);
    assert!(app.stopping.is_none() && app.idle());
    assert!(app.status.starts_with("Conversion cancelled"));
    // A real one cancelled at once leaves no result, and a drop opens once
    // it has stopped.
    app.start();
    app.cancel();
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.stopping.is_some() {
        assert!(
            Instant::now() < deadline,
            "the cancelled conversion never stopped"
        );
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.document.is_none() && app.worker.is_none());
    frame_with(&ctx, &mut app, &[], &[other]);
    assert!(
        app.loaded_path.ends_with("logo-without-blending.png"),
        "{}",
        app.status
    );
}

/// Round two of the Opus 5.5 review: Cancel puts the result a conversion
/// replaced back, and a superseded Auto result leaves the slider alone.
#[test]
fn cancel_puts_the_previous_result_back() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    let before = app.document.as_ref().unwrap().svg().to_owned();
    let counts = app.node_counts;
    app.start();
    assert!(app.document.is_none() && app.worker.is_some());
    app.cancel();
    assert_eq!(app.document.as_ref().unwrap().svg(), before);
    assert_eq!(app.node_counts, counts);
    assert!(
        app.status.contains("previous result is back"),
        "{}",
        app.status
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.stopping.is_some() {
        assert!(
            Instant::now() < deadline,
            "the cancelled conversion never stopped"
        );
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    // Its result never replaced the restored one.
    assert_eq!(app.document.as_ref().unwrap().svg(), before);
    assert!(app.idle());
}

#[test]
fn a_superseded_auto_result_leaves_the_slider_alone() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    // The test is the derive thread, so Auto's answer comes after the newer
    // request every time (the real one drops a superseded request unread,
    // and the answer that matters only came when Auto won that race; round
    // three of the Opus 5.5 review).
    let (requests, _read) = std::sync::mpsc::channel();
    let (answer, answers) = std::sync::mpsc::channel();
    app.deriver = Some((requests, answers));
    app.request_derive(DeriveJob::Auto);
    let auto_serial = app.derive_serial;
    // The user moves the slider before Auto answers: a newer request.
    app.simplify_tolerance = 1.7;
    app.reapply();
    let raw = app.raw_document.clone().unwrap();
    let settings = DeriveSettings {
        simplify: Some(0.3),
        simplify_by_hand: false,
        deleted: Vec::new(),
        regularize: None,
        primitives: false,
        straighten: None,
        straightened: Vec::new(),
        moved: Vec::new(),
        deleted_nodes: Vec::new(),
        rounded: Vec::new(),
        sticker: None,
    };
    let picked = Desktop::derive(&raw, &settings).unwrap();
    let presented = Presented::of(&picked, None).unwrap();
    answer
        .send(DeriveResult {
            serial: auto_serial,
            raw_version: app.raw_version,
            auto: true,
            outcome: Ok((Some(0.3), Derived::new(picked, presented))),
        })
        .unwrap();
    app.receive_derivations(&ctx);
    assert_eq!(app.simplify_tolerance, 1.7);
    assert_eq!(app.derive_pending, Some(auto_serial + 1));
}

#[test]
fn a_picture_scaled_for_the_engine_saves_at_its_own_size() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/app-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}-wide.png", std::process::id()));
    // An odd height: one ratio for both sides saved it one pixel short.
    image::RgbaImage::from_fn(5000, 401, |x, _| {
        image::Rgba(if x < 2500 {
            [200, 30, 30, 255]
        } else {
            [30, 30, 200, 255]
        })
    })
    .save(&path)
    .unwrap();
    let ctx = egui::Context::default();
    let app = converted_from(&ctx, &path).unwrap();
    let _ = std::fs::remove_file(&path);
    let raster = app.raster.as_ref().unwrap();
    assert_eq!((raster.width, raster.height), (4096, 328));
    // 1x is the picture opened, not the copy the engine traced, and the
    // size controls show what is saved.
    assert_eq!(app.output_size(), Some((5000, 401)));
    let (w, h) = app.unit_size().unwrap();
    assert_eq!((w.round(), h.round()), (5000., 401.));
}

#[test]
fn the_open_dialog_lists_every_format_the_loader_decodes() {
    let mut listed: Vec<&str> = OPEN_FILTER
        .split(';')
        .map(|pattern| pattern.strip_prefix("*.").unwrap())
        .collect();
    let mut decoded: Vec<&str> = image::ImageFormat::all()
        .filter(|format| format.reading_enabled())
        .flat_map(|format| format.extensions_str().iter().copied())
        .collect();
    listed.sort_unstable();
    decoded.sort_unstable();
    assert_eq!(listed, decoded);
    // The other files the dialog offers are ones the app reads itself.
    for pattern in IMPORT_FILTER.split(';') {
        let extension = pattern.strip_prefix("*.").unwrap();
        let kind = match extension {
            "psd" | "psb" => crate::import::InputKind::Photoshop,
            "svg" => crate::import::InputKind::Svg,
            "pdf" | "ai" => crate::import::InputKind::Pdf,
            _ => crate::import::InputKind::PostScript,
        };
        assert!(
            kind == crate::import::InputKind::Photoshop || kind.is_vector(),
            "{extension}"
        );
    }
}

/// One frame with these raw events (keys with their modifiers, releases).
fn frame_events(ctx: &egui::Context, app: &mut Desktop, events: Vec<egui::Event>) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 800.),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.ui(ctx),
    );
}

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

/// A shortcut pressed and let go on one frame, then the derivation it asked
/// for.
fn shortcut(ctx: &egui::Context, app: &mut Desktop, pressed: Key, modifiers: Modifiers) {
    frame_events(
        ctx,
        app,
        vec![
            key(pressed, true, modifiers),
            key(pressed, false, modifiers),
        ],
    );
    settle(ctx, app);
}

#[test]
fn undo_and_redo_walk_back_through_roundings_moves_and_settings() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    run_frame(&ctx, &mut app);
    let start = app.document.as_ref().unwrap().svg().to_owned();
    let nodes = app.document.as_ref().unwrap().nodes();
    let (corner, other) = (nodes[3], nodes[9]);
    app.round_node(corner, Reach::Tiny);
    settle(&ctx, &mut app);
    assert_eq!(app.rounding_of(&corner), Some(Reach::Tiny));
    let rounded = app.document.as_ref().unwrap().svg().to_owned();
    assert_ne!(rounded, start);
    let to = vector_rebuild::nodes::written(Point {
        x: other.x + 3.,
        y: other.y + 2.,
    });
    app.move_node(other, to);
    settle(&ctx, &mut app);
    assert!(app
        .document
        .as_ref()
        .unwrap()
        .nodes()
        .iter()
        .any(|n| same_point(n, &to)));
    let moved = app.document.as_ref().unwrap().svg().to_owned();
    app.regularize = false;
    app.reapply();
    settle(&ctx, &mut app);
    assert_eq!(app.undo.len(), 3, "one step per edit");
    // Ctrl+Z walks back one edit at a time, the picture with it.
    shortcut(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
    assert!(app.regularize);
    assert_eq!(app.document.as_ref().unwrap().svg(), moved);
    shortcut(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
    assert!(app.moved.is_empty());
    assert_eq!(app.document.as_ref().unwrap().svg(), rounded);
    shortcut(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
    assert!(app.rounded.is_empty());
    assert_eq!(app.document.as_ref().unwrap().svg(), start);
    shortcut(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
    assert_eq!(app.status, "Nothing to undo.");
    // Ctrl+Y and Ctrl+Shift+Z step forward again.
    shortcut(&ctx, &mut app, Key::Y, Modifiers::COMMAND);
    assert_eq!(app.document.as_ref().unwrap().svg(), rounded);
    shortcut(
        &ctx,
        &mut app,
        Key::Z,
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
    );
    assert_eq!(app.document.as_ref().unwrap().svg(), moved);
    // A new edit drops what was undone.
    app.round_node(other, Reach::Wide);
    settle(&ctx, &mut app);
    assert!(app.redo.is_empty());
}

#[test]
fn a_moved_node_carries_its_rounding_and_is_put_back() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    let node = app.document.as_ref().unwrap().nodes()[5];
    app.round_node(node, Reach::Tight);
    settle(&ctx, &mut app);
    let rounded_here = app.document.as_ref().unwrap().svg().to_owned();
    // The rounded corner's marker dragged: the node moves, then rounds
    // where it landed.
    let to = vector_rebuild::nodes::written(Point {
        x: node.x + 2.5,
        y: node.y - 1.5,
    });
    app.move_node(node, to);
    settle(&ctx, &mut app);
    assert_eq!(app.rounding_of(&to), Some(Reach::Tight));
    assert_eq!(app.moved_from(&to), Some(node));
    assert_ne!(app.document.as_ref().unwrap().svg(), rounded_here);
    // Carried on, it keeps where it came from.
    let further = vector_rebuild::nodes::written(Point {
        x: to.x + 1.,
        y: to.y,
    });
    app.move_node(to, further);
    settle(&ctx, &mut app);
    assert_eq!(app.moved.len(), 1);
    assert_eq!(app.moved_from(&further), Some(node));
    // Put back, the picture is the one rounded in place.
    app.put_back(further);
    settle(&ctx, &mut app);
    assert!(app.moved.is_empty());
    assert_eq!(app.rounding_of(&node), Some(Reach::Tight));
    assert_eq!(app.document.as_ref().unwrap().svg(), rounded_here);
    // Carried back onto its own place, a node is simply not moved.
    app.move_node(node, to);
    app.move_node(to, node);
    assert!(app.moved.is_empty());
}

#[test]
fn a_rounded_corner_dragged_shows_the_outline_it_will_round() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    let node = app.document.as_ref().unwrap().nodes()[5];
    app.round_node(node, Reach::Tight);
    settle(&ctx, &mut app);
    // The rounding cut the corner out of the shown document; the drag
    // pulls the two pieces it had before.
    let shown = app.document.as_ref().unwrap().svg().to_owned();
    assert!(vector_rebuild::nodes::pieces_at(&shown, node)
        .unwrap()
        .is_empty());
    app.begin_node_drag(node);
    let drag = app.node_drag.take().unwrap();
    assert_eq!(drag.pieces.len(), 2, "{:?}", drag.pieces);
    let (reach, frame) = drag.rounding.unwrap();
    assert_eq!(reach, Reach::Tight.fraction());
    // Drawn where it will go, the corner there is round again.
    let to = vector_rebuild::nodes::written(Point {
        x: node.x + 2.,
        y: node.y + 1.,
    });
    let outline = vector_rebuild::nodes::rounded_at(&drag.pieces, node, to, reach, frame);
    let ends: Vec<Point> = outline
        .iter()
        .flat_map(|p| [p.cubic.points[0], p.cubic.points[3]])
        .collect();
    assert!(outline.len() > 2, "{outline:?}");
    assert!(!ends.contains(&to), "{outline:?}");
    // Dropped there, the document has exactly those pieces.
    app.move_node(node, to);
    settle(&ctx, &mut app);
    let dropped = app.document.as_ref().unwrap().svg().to_owned();
    for piece in &outline {
        let [a, _, _, b] = piece.cubic.points;
        for end in [a, b] {
            let end = vector_rebuild::nodes::written(end);
            assert!(
                !vector_rebuild::nodes::pieces_at(&dropped, end)
                    .unwrap()
                    .is_empty(),
                "{end:?} of the drag's outline is not in the dropped document"
            );
        }
    }
    // A corner that is not rounded drags its pieces as they are.
    let plain = app.document.as_ref().unwrap().nodes()[0];
    app.begin_node_drag(plain);
    assert!(app.node_drag.take().unwrap().rounding.is_none());
}

#[test]
fn converting_again_keeps_the_picture_and_its_nodes_until_the_result_arrives() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    app.nodes = true;
    app.view = View::Overlay;
    let nodes = app.current_nodes();
    let preview = app.preview.as_ref().unwrap().id();
    let counts = app.shown_counts();
    app.start();
    // The vector card goes on drawing the picture being replaced, nodes
    // and all, instead of falling back to the bitmap in between.
    assert!(app.document.is_none() && app.preview.is_none());
    assert_eq!(app.vector_texture().map(|t| t.id()), Some(preview));
    assert_eq!(app.stale().and_then(|r| r.nodes.clone()), Some(nodes));
    // The footer and the Nodes card keep their numbers too.
    assert_eq!(app.shown_counts(), counts);
    run_frame(&ctx, &mut app);
    finish(&ctx, &mut app);
    // The new result replaces it in one step.
    assert!(app.stale().is_none());
    assert!(app.preview.is_some() && app.document.is_some());
}

#[test]
fn convert_automatically_runs_again_once_a_setting_rests() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    assert!(app.auto_convert, "on by default");
    app.path = sample("logo-with-blending-small.png")
        .to_string_lossy()
        .into_owned();
    app.load(&ctx);
    // Opening never converts; the first Convert is the user's.
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_none() && app.document.is_none());
    app.start();
    finish(&ctx, &mut app);
    assert!(app.document.is_some(), "{}", app.status);
    // A setting changed: nothing until it has rested.
    app.prep.colors = Some(3);
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_none());
    assert_eq!(app.stale_hint(), "Converting again in a moment\u{2026}");
    std::thread::sleep(AUTO_CONVERT_DELAY + Duration::from_millis(60));
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_some(), "{}", app.status);
    finish(&ctx, &mut app);
    assert!(app.palette.len() <= 3, "{:?}", app.palette);
    assert!(!app.conversion_stale());
    // Changed again while converting: the old conversion stops and the new
    // settings convert.
    app.prep.colors = Some(4);
    run_frame(&ctx, &mut app);
    std::thread::sleep(AUTO_CONVERT_DELAY + Duration::from_millis(60));
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_some());
    app.prep.colors = Some(5);
    run_frame(&ctx, &mut app);
    std::thread::sleep(AUTO_CONVERT_DELAY + Duration::from_millis(60));
    let deadline = Instant::now() + Duration::from_secs(60);
    while app.converted_inputs.as_ref().map(|i| i.prep.colors) != Some(Some(5)) {
        assert!(Instant::now() < deadline, "the restart never came");
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    finish(&ctx, &mut app);
    assert_eq!(app.converted_prep.as_ref().unwrap().colors, Some(5));
    // Off, a change waits for Convert.
    app.auto_convert = false;
    app.prep.colors = Some(6);
    run_frame(&ctx, &mut app);
    std::thread::sleep(AUTO_CONVERT_DELAY + Duration::from_millis(60));
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_none() && app.conversion_stale());
    assert_eq!(app.stale_hint(), "Convert again to apply.");
}

#[test]
fn hold_shows_the_other_picture_only_while_it_is_held() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    app.set_view(View::Overlay, true);
    app.hold_compare = true;
    frame_events(&ctx, &mut app, vec![key(Key::B, true, Modifiers::NONE)]);
    assert_eq!(app.peek, Some(false));
    assert!(app.overlay_vector, "held, not switched");
    run_frame(&ctx, &mut app);
    assert_eq!(app.peek, Some(false), "still held");
    frame_events(&ctx, &mut app, vec![key(Key::B, false, Modifiers::NONE)]);
    assert_eq!(app.peek, None);
    assert!(app.overlay_vector);
    // Without Hold, B switches to the bitmap and stays.
    app.hold_compare = false;
    frame_with(&ctx, &mut app, &[Key::B], &[]);
    assert!(!app.overlay_vector && app.peek.is_none());
}

/// Held by its button, the original stays as long as the button is held:
/// a click-only button stopped counting as pressed after egui's 0.8 s click
/// limit and the vector came back by itself (the owner, September 26, 2026).
#[test]
fn holding_the_original_button_keeps_the_original_until_let_go() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    app.set_view(View::Overlay, true);
    app.hold_compare = true;
    for _ in 0..30 {
        run_frame(&ctx, &mut app);
    }
    let press = |app: &mut Desktop, at: egui::Pos2, pressed: bool| {
        frame_events(
            &ctx,
            app,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    };
    // The Original button in the overlay card's heading: where a press shows
    // the original.
    let mut button = None;
    'scan: for y in (96..460).step_by(12) {
        for x in (480..1040).step_by(16) {
            let at = egui::pos2(x as f32, y as f32);
            press(&mut app, at, true);
            run_frame(&ctx, &mut app);
            let original = app.peek == Some(false);
            press(&mut app, at, false);
            // A press on its checkbox turns Hold off.
            app.hold_compare = true;
            if original {
                button = Some(at);
                break 'scan;
            }
        }
    }
    let at = button.expect("no Original button in the overlay card");
    press(&mut app, at, true);
    // A frame is 1/60 s: 90 of them hold it past the click limit.
    for _ in 0..90 {
        run_frame(&ctx, &mut app);
    }
    assert_eq!(app.peek, Some(false), "let go by itself while held");
    press(&mut app, at, false);
    run_frame(&ctx, &mut app);
    assert_eq!(app.peek, None);
}

#[test]
fn the_window_preferences_round_trip_and_ignore_what_they_do_not_know() {
    let dark = (ThemeChoice::Dark, None);
    let text = prefs::write_prefs((true, false, true), &Default::default(), dark);
    assert_eq!(
        prefs::parse_prefs(&text, (false, true, false)),
        (true, false, true)
    );
    assert!(!text.contains("licence"));
    assert_eq!(prefs::parse_appearance(&text), dark);
    // Light or dark and the first-run answer ride along; words not
    // understood keep the defaults.
    let chosen = (ThemeChoice::System, Some(LicenceUse::Commercial));
    let text = prefs::write_prefs((true, false, false), &Default::default(), chosen);
    assert_eq!(prefs::parse_appearance(&text), chosen);
    // A free commercial trial keeps when it began.
    let trial = (ThemeChoice::Dark, Some(LicenceUse::Trial(1_758_700_000)));
    let text = prefs::write_prefs((true, false, false), &Default::default(), trial);
    assert!(text.contains("licence_use=trial:1758700000\r\n"), "{text}");
    assert_eq!(prefs::parse_appearance(&text), trial);
    assert_eq!(
        prefs::parse_appearance("look=studio\ntheme=sepia\nlicence_use=maybe\n"),
        dark
    );
    // A licence rides along; a key that is not shaped like one is dropped,
    // and its certificate with it.
    let licence = crate::licence::Stored {
        key: "esk_ABCDE-FGHIJ-KLMNO-PQRS1".into(),
        certificate: "payload.signature".into(),
    };
    let text = prefs::write_prefs((true, false, false), &licence, dark);
    assert_eq!(
        prefs::parse_prefs(&text, (false, true, true)),
        (true, false, false)
    );
    assert_eq!(prefs::parse_licence(&text), licence);
    assert_eq!(
        prefs::parse_licence("licence_key=nonsense\nlicence_certificate=a.b\n"),
        crate::licence::Stored::default()
    );
    assert_eq!(
        prefs::parse_prefs(
            "junk\nhold_compare=maybe\nauto_convert=off\nother=on\n",
            (true, true, true)
        ),
        (true, false, true)
    );
    let ctx = egui::Context::default();
    let path = scratch("prefs").join("desktop.txt");
    let mut app = Desktop::blank(&ctx);
    app.load_prefs(path.clone());
    app.hold_compare = true;
    app.save_prefs();
    let mut again = Desktop::blank(&ctx);
    again.load_prefs(path.clone());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    assert!(again.hold_compare && again.auto_convert);
    // Snapshots and tests never touch the file.
    assert!(Desktop::blank(&ctx).prefs.is_none());
}

#[test]
fn tiny_rounding_is_the_gentlest_reach() {
    assert_eq!(Reach::ALL[0], Reach::Tiny);
    assert!(Reach::ALL
        .windows(2)
        .all(|w| w[0].fraction() < w[1].fraction()));
    assert_eq!(Reach::parse(" Tiny "), Some(Reach::Tiny));
}

#[test]
fn a_node_dragged_again_before_its_picture_arrives_moves_once() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    let node = app.document.as_ref().unwrap().nodes()[5];
    app.round_node(node, Reach::Tight);
    settle(&ctx, &mut app);
    let first = Point {
        x: node.x + 2.,
        y: node.y,
    };
    let second = Point {
        x: node.x + 4.,
        y: node.y + 1.,
    };
    // The second drag starts where the marker still shows it: the old place.
    app.move_node(node, first);
    app.move_node(node, second);
    let second = vector_rebuild::nodes::written(second);
    assert_eq!(app.moved.len(), 1);
    assert!(same_point(&app.moved[0].to, &second));
    assert_eq!(app.rounding_of(&second), Some(Reach::Tight));
    settle(&ctx, &mut app);
    assert_eq!(app.moved_from(&second), Some(node));
}

#[test]
fn cancel_holds_the_settings_until_they_change() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    app.path = sample("logo-with-blending-small.png")
        .to_string_lossy()
        .into_owned();
    app.load(&ctx);
    app.start();
    finish(&ctx, &mut app);
    let shown = app.document.as_ref().unwrap().svg().to_owned();
    app.prep.colors = Some(3);
    run_frame(&ctx, &mut app);
    std::thread::sleep(AUTO_CONVERT_DELAY + Duration::from_millis(60));
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_some());
    app.cancel();
    // The earlier picture is back with the settings it was made with, and
    // the stopped settings wait: the card says Convert, not "in a moment".
    assert_eq!(app.document.as_ref().unwrap().svg(), shown);
    assert_eq!(app.converted_inputs.as_ref().unwrap().prep.colors, None);
    assert!(app.held() && app.conversion_stale());
    assert_eq!(app.stale_hint(), "Convert again to apply.");
    std::thread::sleep(AUTO_CONVERT_DELAY + Duration::from_millis(60));
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.stopping.is_some() {
        assert!(
            Instant::now() < deadline,
            "the cancelled conversion never stopped"
        );
        run_frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_none(), "held settings started again");
    // Changed again, they convert by themselves.
    app.prep.colors = Some(4);
    run_frame(&ctx, &mut app);
    std::thread::sleep(AUTO_CONVERT_DELAY + Duration::from_millis(60));
    run_frame(&ctx, &mut app);
    assert!(app.worker.is_some());
    finish(&ctx, &mut app);
    assert_eq!(app.converted_prep.as_ref().unwrap().colors, Some(4));
}

#[test]
fn a_node_is_deleted_undone_and_kept_through_a_conversion() {
    let ctx = egui::Context::default();
    let mut app = converted(&ctx);
    run_frame(&ctx, &mut app);
    let svg = |app: &Desktop| app.document.as_ref().unwrap().svg().to_owned();
    let shows = |app: &Desktop, p: &Point| {
        app.document
            .as_ref()
            .unwrap()
            .nodes()
            .iter()
            .any(|n| same_point(n, p))
    };
    let start = svg(&app);
    let nodes = app.document.as_ref().unwrap().nodes();
    let refusal = |p: &Point| vector_rebuild::nodes::deletion_refusal(&start, *p).unwrap();
    let node = *nodes
        .iter()
        .find(|n| refusal(n).is_none())
        .expect("a node joining two pieces");
    // From its menu: the node goes and the menu closes, one step of Undo.
    app.node_menu = Some((node, egui::pos2(300., 300.)));
    assert_eq!(app.deletion_refusal(&node), None);
    app.delete_node(node);
    assert!(app.node_menu.is_none());
    settle(&ctx, &mut app);
    assert!(!shows(&app, &node));
    let kept = svg(&app);
    assert_ne!(kept, start);
    shortcut(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
    assert!(app.deleted_nodes.is_empty());
    assert_eq!(svg(&app), start);
    shortcut(&ctx, &mut app, Key::Y, Modifiers::COMMAND);
    assert_eq!(svg(&app), kept);
    // Converted again, the same trace loses the same node.
    app.start();
    finish(&ctx, &mut app);
    settle(&ctx, &mut app);
    assert_eq!(svg(&app), kept);
    // A junction stays, and says why.
    if let Some(junction) = nodes.iter().find(|n| refusal(n).is_some()) {
        let before = app.deleted_nodes.clone();
        app.delete_node(*junction);
        assert_eq!(app.deleted_nodes, before);
        assert_eq!(Some(app.status.as_str()), refusal(junction));
    }
    // A rounded corner deleted takes its rounding with it. Picked in the
    // drawing as it stands: the first deletion may have left the outline it
    // was on with three nodes, which then all stay.
    let now = svg(&app);
    let other = *app
        .document
        .as_ref()
        .unwrap()
        .nodes()
        .iter()
        .find(|n| {
            vector_rebuild::nodes::deletion_refusal(&now, **n)
                .unwrap()
                .is_none()
        })
        .expect("a second node joining two pieces");
    app.round_node(other, Reach::Tight);
    settle(&ctx, &mut app);
    // The two ends of its arc are shown, but made by the rounding.
    let arc_end = {
        let shown = app.document.as_ref().unwrap();
        let before = shown.unrounded.clone().unwrap();
        shown
            .nodes()
            .into_iter()
            .find(|n| {
                vector_rebuild::nodes::deletion_refusal(&before, *n).unwrap()
                    == Some(vector_rebuild::nodes::ABSENT)
            })
            .expect("a node the rounding made")
    };
    assert!(app
        .deletion_refusal(&arc_end)
        .is_some_and(|why| why.contains("rounded corner")));
    app.delete_node(other);
    settle(&ctx, &mut app);
    assert!(app.rounding_of(&other).is_none(), "{}", app.status);
    assert_eq!(app.deleted_nodes.len(), 2);
    // Brought back, every node is where the trace put it.
    app.restore_deleted_nodes();
    settle(&ctx, &mut app);
    assert!(app.deleted_nodes.is_empty());
    assert_eq!(svg(&app), start);
}

#[test]
fn a_refused_typed_key_stays_out_and_a_refused_stored_one_ends_the_licence() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    // Nothing stored: nothing is asked of the network.
    app.licence_tick(&ctx);
    assert!(app.licence_reply.is_none() && app.licence_renewed);
    let key = "esk_ABCDE-FGHIJ-KLMNO-PQRS1".to_owned();
    let refused = r#"{"error":"unknown_or_expired_key","message":"That key does not exist, has expired, or has been revoked."}"#;
    app.take_redeem(key.clone(), true, 404, refused);
    assert_eq!(app.licence, crate::licence::Stored::default());
    assert!(app
        .licence_note
        .as_deref()
        .unwrap()
        .contains("does not exist"));
    let stored = crate::licence::Stored {
        key: key.clone(),
        certificate: "a.b".into(),
    };
    app.licence = stored.clone();
    app.take_redeem(key.clone(), false, 404, refused);
    assert_eq!(app.licence, crate::licence::Stored::default());
    assert!(app
        .licence_note
        .as_deref()
        .unwrap()
        .starts_with("The license has ended"));
    // No connection keeps what is stored.
    app.licence = stored.clone();
    app.take_redeem(key, false, 0, "");
    assert_eq!(app.licence, stored);
    // The card draws in each state, folded and open.
    run_frame(&ctx, &mut app);
    app.collapsed[6] = false;
    run_frame(&ctx, &mut app);
    app.licence = crate::licence::Stored::default();
    run_frame(&ctx, &mut app);
}

#[test]
fn light_and_dark_build_their_style_and_a_frame_draws_it() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    for (theme, dark) in [(ThemeChoice::Light, false), (ThemeChoice::Dark, true)] {
        app.set_appearance(theme);
        frame_with(&ctx, &mut app, &[], &[]);
        let style = ctx.style();
        assert_eq!(style.visuals.dark_mode, dark, "{theme:?}");
        assert!(std::ptr::eq(pal(), look::palette(dark)), "{theme:?}");
        assert_eq!(style.visuals.panel_fill, pal().backdrop);
        assert_eq!(app.applied, Some(dark));
    }
    // The popup that chooses them draws in every look.
    app.open_overlay(Overlay::Appearance, egui::pos2(600., 300.));
    frame_with(&ctx, &mut app, &[], &[]);
    assert!(app.appearance_open);
}

#[test]
fn the_first_run_question_is_asked_once_and_an_answer_or_a_licence_ends_it() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    // Tests and snapshots start without it.
    assert!(!app.ask_licence);
    app.ask_licence_if_new();
    assert!(app.ask_licence);
    frame_with(&ctx, &mut app, &[], &[]);
    // The Commercial tile opens the key field; the question stays.
    app.prompt_key = true;
    frame_with(&ctx, &mut app, &[], &[]);
    assert!(app.ask_licence);
    // An answer kept in the preferences, or a stored licence, means it is
    // not asked again.
    app.licence_use = Some(LicenceUse::Personal);
    app.ask_licence_if_new();
    assert!(!app.ask_licence);
    // A free day of commercial use is not asked about while it runs, and
    // once it has ended the question comes back (without a second trial).
    let now = vector_rebuild::clock::unix_seconds();
    app.licence_use = Some(LicenceUse::Trial(now - 3600));
    app.ask_licence_if_new();
    assert!(!app.ask_licence);
    assert_eq!(app.licence_use.unwrap().trial_left(now), Some(23 * 3600));
    app.licence_use = Some(LicenceUse::Trial(now - TRIAL_SECONDS - 1));
    app.ask_licence_if_new();
    assert!(app.ask_licence);
    frame_with(&ctx, &mut app, &[], &[]);
    app.ask_licence = false;
    app.licence_use = None;
    app.licence.key = "esk_ABCDE-FGHIJ-KLMNO-PQRS1".into();
    app.ask_licence_if_new();
    assert!(!app.ask_licence);
    // A key Pay refuses leaves the question open with the reason shown.
    app.licence = Default::default();
    app.ask_licence_if_new();
    app.take_redeem(
        "esk_ABCDE-FGHIJ-KLMNO-PQRS1".into(),
        true,
        404,
        r#"{"ok":false,"error":"unknown_or_expired_key","message":"That key is not known."}"#,
    );
    assert!(app.ask_licence && app.licence_note.is_some());
}

#[test]
fn a_portable_copy_keeps_its_settings_beside_itself() {
    let folder = scratch("portable");
    std::fs::create_dir_all(&folder).unwrap();
    let path = prefs::portable_prefs(&folder);
    assert_eq!(path, Some(folder.join(prefs::PORTABLE_PREFS)));
    // The check that the folder takes files leaves nothing behind.
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    app.load_prefs(path.clone().unwrap());
    app.theme = ThemeChoice::Light;
    app.save_prefs();
    let mut again = Desktop::blank(&ctx);
    again.load_prefs(path.unwrap());
    let _ = std::fs::remove_dir_all(&folder);
    assert_eq!(again.theme, ThemeChoice::Light);
}

#[test]
fn a_vector_file_is_asked_about_then_converted_as_it_is_or_traced() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="#ff0000"/><rect x="20" width="20" height="20" fill="#0000ff"/></svg>"##;
    app.load_bytes(&ctx, "logo.svg".into(), svg);
    let offer = app
        .vector_offer
        .as_ref()
        .expect("a vector file is asked about");
    assert_eq!(offer.kind, crate::import::InputKind::Svg);
    assert!(app.raster.is_none());
    frame_with(&ctx, &mut app, &[], &[]);
    // Converted: its shapes kept, shown, and saved at its own size (40 by
    // 20 CSS pixels) through the same Save path.
    app.convert_offer(&ctx);
    assert!(app.vector_offer.is_none() && app.raster.is_none() && app.document.is_none());
    assert!(app.foreign.is_some());
    assert_eq!(app.output_size(), Some((40, 20)));
    let saved = app.export_svg().unwrap().unwrap();
    assert!(saved.contains("fill=\"#ff0000\"") && saved.contains("fill=\"#0000ff\""));
    assert!(saved.contains("width=\"40\"") && saved.contains("height=\"20\""));
    assert!(crate::pdf_eps::to_pdf(&saved).unwrap().starts_with(b"%PDF"));
    frame_with(&ctx, &mut app, &[], &[]);
    // Traced instead: drawn at the chosen side and opened as a picture.
    app.load_bytes(&ctx, "logo.svg".into(), svg);
    app.trace_offer(&ctx);
    assert!(app.foreign.is_none());
    let raster = app.raster.as_ref().expect("traced from pixels");
    assert_eq!((raster.width, raster.height), (2000, 1000));
    // Cancel leaves what was open alone.
    app.load_bytes(&ctx, "other.svg".into(), svg);
    app.vector_offer = None;
    assert!(app.raster.is_some());
}

#[test]
fn a_photoshop_document_with_shape_layers_is_asked_about_and_traced_from_its_composite() {
    let ctx = egui::Context::default();
    let mut app = Desktop::blank(&ctx);
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/samples/vector-mojo-sample.psd"),
    )
    .unwrap();
    app.load_bytes(&ctx, "vector-mojo-sample.psd".into(), &bytes);
    let offer = app
        .vector_offer
        .as_ref()
        .expect("shape layers are asked about");
    assert_eq!(offer.kind, crate::import::InputKind::Photoshop);
    assert!(offer.imported.svg.matches("<path").count() >= 8);
    let picture = offer
        .picture
        .as_ref()
        .expect("its composite is the picture to trace");
    let (width, height) = (picture.width, picture.height);
    frame_with(&ctx, &mut app, &[], &[]);
    app.trace_offer(&ctx);
    let raster = app.raster.as_ref().expect("traced from its composite");
    assert_eq!((raster.width, raster.height), (width, height));
}
