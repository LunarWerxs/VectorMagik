//! Part of `desktop_ui`: vector files opened in the app (`crate::import`).
//! The app traces pictures; a file that already holds shapes (SVG, PDF, AI,
//! EPS) is not refused but asked about (the owner, September 24, 2026:
//! "instead of saying 'This is unsupported,' it says ... do you want us to
//! convert this to a raster, then trace it ... or do you want us to just
//! cross-convert?"). Tracing draws it as a picture and traces that like any
//! image; converting keeps its shapes exactly and saves them in any format
//! the app writes.

use super::*;
use crate::import::{Imported, InputKind};

/// A vector file just opened, waiting for the answer.
pub(super) struct VectorOffer {
    pub name: String,
    pub kind: InputKind,
    pub imported: Imported,
    /// The longer side, in pixels, the picture is drawn at to be traced.
    pub side: u32,
    /// The file's own picture, traced in place of a drawing of its shapes:
    /// a Photoshop document's composite, raster layers and all.
    pub picture: Option<Raster>,
}

/// A vector file kept as it is: shown, and saved in place of a traced
/// document (a cross-conversion).
pub(super) struct Foreign {
    pub kind: InputKind,
    pub imported: Imported,
    pub texture: egui::TextureHandle,
    /// Its page in pixels at 96 per inch: the saved size at 1x.
    pub size: (f32, f32),
}

/// The sides a vector file can be traced at.
const TRACE_SIDES: [u32; 3] = [1000, 2000, 4000];

enum Answer {
    Trace,
    Convert,
    Cancel,
}

/// How many shapes a flat SVG holds.
fn shape_count(imported: &Imported) -> usize {
    imported.svg.matches("<path").count()
}

/// The page of a flat SVG in points, from its viewBox.
fn page_points(imported: &Imported) -> (f32, f32) {
    let view = imported
        .svg
        .split_once("viewBox=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(view, _)| {
            view.split_whitespace()
                .filter_map(|v| v.parse::<f32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    match view.as_slice() {
        [_, _, w, h] if *w > 0. && *h > 0. => (*w, *h),
        _ => (100., 100.),
    }
}

impl Desktop {
    /// Take a vector file's artwork, read from `name`: ask what to do with it.
    pub(super) fn offer_vector(
        &mut self,
        name: &str,
        kind: InputKind,
        imported: Imported,
        picture: Option<Raster>,
    ) {
        let name = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name)
            .to_owned();
        self.vector_offer = Some(VectorOffer {
            name,
            kind,
            imported,
            side: TRACE_SIDES[1],
            picture,
        });
    }

    /// The question about a vector file just opened.
    pub(super) fn vector_prompt(&mut self, ctx: &egui::Context) {
        let Some(offer) = &mut self.vector_offer else {
            return;
        };
        let family = self.title_family.clone();
        let p = pal();
        let shapes = shape_count(&offer.imported);
        let (width, height) = page_points(&offer.imported);
        let mut answer: Option<Answer> = None;
        let dialog = dialog_width(ctx, 520.);
        egui::Modal::new(egui::Id::new("vector-offer"))
            .frame(popup_frame().inner_margin(Margin::same(22)))
            .backdrop_color(p.scrim)
            .show(ctx, |ui| {
                ui.set_width(dialog);
                ui.spacing_mut().item_spacing.y = 8.;
                ui.label(
                    RichText::new(format!("{} is vector artwork", offer.name))
                        .size(19.)
                        .family(family.clone())
                        .color(p.text),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "VectorMagik traces pictures, and this {} already holds shapes: \
                             {shapes} on a page {} \u{00D7} {} pt{}.",
                            offer.kind.label(),
                            width.round(),
                            height.round(),
                            if offer.imported.pages > 1 {
                                format!(" (the first of {} pages)", offer.imported.pages)
                            } else {
                                String::new()
                            }
                        ))
                        .size(13.)
                        .color(p.dim),
                    )
                    .wrap(),
                );
                if !offer.imported.skipped.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "Left out: {}.",
                                offer.imported.skipped.join("; ")
                            ))
                            .size(12.)
                            .color(p.warn),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(4.);
                let (trace, convert) = tile_pair(
                    ui,
                    &family,
                    dialog < 440.,
                    (
                        [
                            "Trace it",
                            "From pixels",
                            "Draw it as a picture and trace that, like any image: a clean redraw.",
                            "Trace it",
                        ],
                        false,
                    ),
                    (
                        [
                            "Convert it",
                            "Shapes kept exactly",
                            "Save its own shapes as SVG, PDF, EPS and the other formats, untraced.",
                            "Convert it",
                        ],
                        false,
                    ),
                );
                if trace {
                    answer = Some(Answer::Trace);
                }
                if convert {
                    answer = Some(Answer::Convert);
                }
                let own_picture = offer.picture.is_some();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.;
                    if !own_picture {
                        ui.label(RichText::new("Traced at").size(12.).color(p.dim));
                    }
                    for side in TRACE_SIDES.into_iter().filter(|_| !own_picture) {
                        if choice_width(ui, &format!("{side} px"), offer.side == side, 70.)
                            .on_hover_text(
                                "The drawing's longer side in pixels: larger keeps finer \
                                 detail and takes longer to trace.",
                            )
                            .clicked()
                        {
                            offer.side = side;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(pill_widget("Cancel")).clicked() {
                            answer = Some(Answer::Cancel);
                        }
                    });
                });
            });
        match answer {
            Some(Answer::Trace) => self.trace_offer(ctx),
            Some(Answer::Convert) => self.convert_offer(ctx),
            Some(Answer::Cancel) => {
                self.vector_offer = None;
                self.path.clone_from(&self.loaded_path);
            }
            None => {}
        }
    }

    /// Draw the offered file as a picture and trace it.
    pub(super) fn trace_offer(&mut self, ctx: &egui::Context) {
        let Some(offer) = self.vector_offer.take() else {
            return;
        };
        let drawn = match offer.picture {
            Some(picture) => Ok(picture),
            #[cfg(feature = "render")]
            None => crate::import::rasterize(&offer.imported, offer.side),
            #[cfg(not(feature = "render"))]
            None => Err("This build draws no vector files.".into()),
        };
        self.path = offer.name;
        self.take_loaded(ctx, drawn);
        if self.raster.is_some() && self.idle() {
            self.start();
        }
    }

    /// Keep the offered file's shapes as they are, shown and ready to save.
    pub(super) fn convert_offer(&mut self, ctx: &egui::Context) {
        let Some(offer) = self.vector_offer.take() else {
            return;
        };
        #[cfg(feature = "render")]
        let drawn = crate::import::rasterize(
            &offer.imported,
            (ctx.input(|i| i.max_texture_side) as u32).min(2048),
        );
        #[cfg(not(feature = "render"))]
        let drawn: Result<Raster, String> = Err("This build draws no vector files.".into());
        let raster = match drawn {
            Ok(raster) => raster,
            Err(error) => {
                self.path.clone_from(&self.loaded_path);
                self.set_status(StatusKind::Error, error);
                return;
            }
        };
        // What was open goes, as when a picture is opened.
        self.close();
        let texture = ctx.load_texture(
            "foreign",
            display_image(&raster, ctx.input(|i| i.max_texture_side)),
            egui::TextureOptions::LINEAR,
        );
        let (width, height) = page_points(&offer.imported);
        let shapes = shape_count(&offer.imported);
        self.path.clone_from(&offer.name);
        self.loaded_path = offer.name;
        self.document_version += 1;
        self.set_status(
            StatusKind::Done,
            format!(
                "Converted: the {}'s {shapes} shapes, kept exactly. Save them as any format.",
                offer.kind.label()
            ),
        );
        self.foreign = Some(Foreign {
            kind: offer.kind,
            imported: offer.imported,
            texture,
            size: (width * 4. / 3., height * 4. / 3.),
        });
    }

    /// The workspace while a converted file is open: its drawing, fitted.
    pub(super) fn foreign_workspace(&mut self, ctx: &egui::Context) {
        let Some(foreign) = &self.foreign else {
            return;
        };
        let family = self.title_family.clone();
        let (texture, size) = (foreign.texture.id(), foreign.texture.size_vec2());
        let title = format!(
            "Converted {} \u{00B7} {} shapes",
            foreign.kind.label(),
            shape_count(&foreign.imported)
        );
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal().canvas_fill())
                    .inner_margin(Margin {
                        left: 6,
                        right: 12,
                        top: 12,
                        bottom: 12,
                    }),
            )
            .show(ctx, |ui| {
                card_frame().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon::VECTOR).size(14.).color(pal().accent));
                                ui.label(
                                    RichText::new(title)
                                        .size(14.)
                                        .family(family)
                                        .color(pal().text),
                                );
                                ui.label(
                                    RichText::new("Not traced: the file's own shapes")
                                        .size(12.)
                                        .color(pal().dim),
                                );
                            });
                        });
                    let area = ui.available_rect_before_wrap().shrink(12.);
                    let fit = (area.width() / size.x).min(area.height() / size.y).min(1.);
                    let rect = egui::Rect::from_center_size(area.center(), size * fit);
                    let painter = ui.painter_at(area);
                    checkerboard(&painter, rect, rect);
                    painter.image(
                        texture,
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                });
            });
    }
}
