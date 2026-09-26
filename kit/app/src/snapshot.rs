//! Paint this application's egui frame into memory. No window, display, screen
//! capture, OS input, browser, network or GPU context is created by this module.
use eframe::egui::{self, epaint, ColorImage, Pos2, TextureId};
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
struct Painter {
    textures: HashMap<TextureId, ColorImage>,
}
impl Painter {
    fn textures(&mut self, delta: &egui::TexturesDelta) -> Result<(), String> {
        for (id, change) in &delta.set {
            let epaint::ImageData::Color(image) = &change.image;
            if let Some([x, y]) = change.pos {
                let target = self
                    .textures
                    .get_mut(id)
                    .ok_or("Missing snapshot texture")?;
                if x + image.size[0] > target.size[0] || y + image.size[1] > target.size[1] {
                    return Err("Snapshot texture update outside allocation".into());
                }
                for row in 0..image.size[1] {
                    let start = (y + row) * target.size[0] + x;
                    target.pixels[start..start + image.size[0]].copy_from_slice(
                        &image.pixels[row * image.size[0]..(row + 1) * image.size[0]],
                    );
                }
            } else {
                self.textures.insert(*id, (**image).clone());
            }
        }
        Ok(())
    }
    fn frame(
        &self,
        primitives: &[epaint::ClippedPrimitive],
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<image::RgbaImage, String> {
        let mut pixels =
            image::RgbaImage::from_pixel(width, height, image::Rgba([27, 27, 27, 255]));
        for clipped in primitives {
            let epaint::Primitive::Mesh(mesh) = &clipped.primitive else {
                return Err(
                    "GPU paint callbacks are not supported by the app snapshot renderer".into(),
                );
            };
            let texture = self
                .textures
                .get(&mesh.texture_id)
                .ok_or("Missing frame texture")?;
            let clip = clipped.clip_rect * scale;
            for indices in mesh.indices.as_chunks::<3>().0 {
                let mut v = [
                    mesh.vertices[indices[0] as usize],
                    mesh.vertices[indices[1] as usize],
                    mesh.vertices[indices[2] as usize],
                ];
                for v in &mut v {
                    v.pos *= scale;
                }
                let mut area = edge(v[0].pos, v[1].pos, v[2].pos);
                if area == 0. || !area.is_finite() {
                    continue;
                }
                if area < 0. {
                    v.swap(1, 2);
                    area = -area;
                }
                let lo_x = v
                    .iter()
                    .map(|v| v.pos.x)
                    .fold(f32::INFINITY, f32::min)
                    .max(clip.min.x)
                    .max(0.)
                    .floor() as u32;
                let lo_y = v
                    .iter()
                    .map(|v| v.pos.y)
                    .fold(f32::INFINITY, f32::min)
                    .max(clip.min.y)
                    .max(0.)
                    .floor() as u32;
                let hi_x = v
                    .iter()
                    .map(|v| v.pos.x)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .min(clip.max.x)
                    .min(width as f32)
                    .ceil() as u32;
                let hi_y = v
                    .iter()
                    .map(|v| v.pos.y)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .min(clip.max.y)
                    .min(height as f32)
                    .ceil() as u32;
                for y in lo_y..hi_y {
                    for x in lo_x..hi_x {
                        let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                        if !clip.contains(p) {
                            continue;
                        }
                        let e = [
                            edge(v[1].pos, v[2].pos, p),
                            edge(v[2].pos, v[0].pos, p),
                            edge(v[0].pos, v[1].pos, p),
                        ];
                        if (0..3).any(|i| {
                            e[i] < 0.
                                || e[i] == 0. && !top_left(v[(i + 1) % 3].pos, v[(i + 2) % 3].pos)
                        }) {
                            continue;
                        }
                        let weights = e.map(|e| e / area);
                        let u = (0..3).map(|i| weights[i] * v[i].uv.x).sum::<f32>();
                        let t = (0..3).map(|i| weights[i] * v[i].uv.y).sum::<f32>();
                        let texel = sample(texture, u, t);
                        let rgba: [f32; 4] = std::array::from_fn(|c| {
                            texel[c]
                                * (0..3)
                                    .map(|i| weights[i] * v[i].color.to_array()[c] as f32 / 255.)
                                    .sum::<f32>()
                        });
                        let old = pixels.get_pixel_mut(x, y);
                        for c in 0..3 {
                            old[c] = (rgba[c] + old[c] as f32 * (1. - rgba[3] / 255.))
                                .round()
                                .clamp(0., 255.) as u8;
                        }
                    }
                }
            }
        }
        Ok(pixels)
    }
}
fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}
fn top_left(a: Pos2, b: Pos2) -> bool {
    b.y < a.y || b.y == a.y && b.x > a.x
}
fn sample(image: &ColorImage, u: f32, v: f32) -> [f32; 4] {
    let x = (u * image.size[0] as f32 - 0.5).clamp(0., (image.size[0] - 1) as f32);
    let y = (v * image.size[1] as f32 - 0.5).clamp(0., (image.size[1] - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = (
        (x0 + 1).min(image.size[0] - 1),
        (y0 + 1).min(image.size[1] - 1),
    );
    let (a, b) = (x - x0 as f32, y - y0 as f32);
    std::array::from_fn(|c| {
        let pixel = |x: usize, y: usize| image.pixels[y * image.size[0] + x].to_array()[c] as f32;
        (pixel(x0, y0) * (1. - a) + pixel(x1, y0) * a) * (1. - b)
            + (pixel(x0, y1) * (1. - a) + pixel(x1, y1) * a) * b
    })
}

#[derive(Clone, Debug)]
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub convert: bool,
    pub nodes: bool,
    pub zoom: f32,
    pub manual: Option<crate::engine::Options>,
    /// Curve simplification tolerance in source pixels, or `None` to show the
    /// engine's own output. The desktop default is on.
    pub simplify: Option<f64>,
    /// A popup to open on top of the workspace, as a click would.
    pub overlay: Option<crate::desktop_ui::Overlay>,
    /// Colour limit, background and merges applied before the engine.
    pub prep: crate::Preparation,
    /// Corners to round after converting: the node nearest each point.
    pub rounds: Vec<(f64, f64, crate::desktop_ui::Reach)>,
    /// Nodes to delete after converting (and rounding): the shown node
    /// nearest each point, keeping the shape when the flag is set.
    pub deletes: Vec<(f64, f64)>,
    /// The Conversion card's "Smooth joins" switch (the engine's optional pass).
    pub optimizer: bool,
    /// The Sticker card: off, on as the desktop sizes it, or on with settings.
    pub sticker: crate::desktop_ui::StickerChoice,
    /// The Curves card's straightening bow (Auto, or a tolerance in pixels),
    /// or `None` for off. The desktop default is on at Auto.
    pub straighten: Option<crate::desktop_ui::Bow>,
    /// The Curves card's true lines and circles. The desktop default is on.
    pub regularize: bool,
    /// The Curves card's true shapes. The desktop default is on.
    pub primitives: bool,
    /// The Advanced card's sliders, or `None` for the preset (the default).
    pub sliders: Option<crate::engine::Sliders>,
    /// One picture instead of two: `Some(true)` shows the vector, `Some(false)`
    /// the bitmap.
    pub single_view: Option<bool>,
    /// Press the Sticker card's "Cut out background" after converting.
    pub cut_background: bool,
    /// Where the pointer rests, in window pixels, for the hover states (the
    /// rail's resize grip and scroll bar, a node's tooltip): the frames then
    /// run for two thirds of a second, past every hover delay.
    pub pointer: Option<(f32, f32)>,
    /// A left-button drag from the first point to the second, in window
    /// pixels, held at the end (a node dragged shows where it will go).
    pub drag: Option<((f32, f32), (f32, f32))>,
    /// Convert again once converted and render while that conversion runs.
    pub reconvert: bool,
    /// The look, and whether light instead of dark.
    pub look: crate::desktop_ui::Look,
    pub light: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 800,
            convert: false,
            nodes: false,
            zoom: 1.,
            manual: None,
            simplify: Some(crate::desktop_ui::DEFAULT_SIMPLIFY_TOLERANCE as f64),
            overlay: None,
            prep: crate::Preparation::default(),
            rounds: Vec::new(),
            deletes: Vec::new(),
            optimizer: false,
            sticker: crate::desktop_ui::StickerChoice::Off,
            straighten: Some(crate::desktop_ui::Bow::Auto),
            regularize: true,
            primitives: crate::desktop_ui::DEFAULT_PRIMITIVES,
            sliders: None,
            single_view: None,
            cut_background: false,
            pointer: None,
            drag: None,
            reconvert: false,
            look: crate::desktop_ui::Look::Classic,
            light: false,
        }
    }
}

pub fn render(input: Option<&Path>, options: Options) -> Result<image::RgbaImage, String> {
    if !(850..=2400).contains(&options.width)
        || !(600..=1600).contains(&options.height)
        || !options.zoom.is_finite()
        || !(0.5..=4.).contains(&options.zoom)
    {
        return Err("Snapshot size must be 850..2400 by 600..1600; zoom must be 0.5..4".into());
    }
    let ctx = egui::Context::default();
    let mut app = crate::desktop_ui::Desktop::snapshot_state(
        &ctx,
        input,
        options.convert,
        options.nodes,
        options.zoom,
        options.manual,
        options.simplify,
        options.prep,
        options.optimizer,
        options.sticker,
        options.straighten,
        options.regularize,
        options.primitives,
        options.sliders,
    )?;
    app.set_appearance(
        options.look,
        if options.light {
            crate::desktop_ui::ThemeChoice::Light
        } else {
            crate::desktop_ui::ThemeChoice::Dark
        },
    );
    if let Some(vector) = options.single_view {
        app.set_view(crate::desktop_ui::View::Overlay, vector);
    }
    if options.cut_background {
        app.cut_background_now(&ctx)?;
    }
    for (x, y, reach) in &options.rounds {
        app.round_nearest_now(
            &ctx,
            vector_rebuild::geometry::Point { x: *x, y: *y },
            *reach,
        )?;
    }
    for (x, y) in &options.deletes {
        app.delete_nearest_now(&ctx, vector_rebuild::geometry::Point { x: *x, y: *y })?;
    }
    if let Some(overlay) = options.overlay {
        app.open_overlay(
            overlay,
            egui::pos2(options.width as f32 * 0.62, options.height as f32 * 0.42),
        );
    }
    if options.reconvert {
        app.convert_again();
    }
    let mut painter = Painter::default();
    let mut image = None;
    // Warm layout/font atlas across frames exactly as an egui integration
    // does, and let popups finish fading in (a tenth of a second).
    let frames = if options.pointer.is_some() || options.drag.is_some() {
        40
    } else {
        10
    };
    for frame in 0..frames {
        // The pointer arrives once and rests, so tooltips see it still.
        let mut events: Vec<egui::Event> = options
            .pointer
            .filter(|_| frame == 0)
            .map(|(x, y)| egui::Event::PointerMoved(egui::pos2(x, y)))
            .into_iter()
            .collect();
        if let Some(((x0, y0), (x1, y1))) = options.drag {
            // Rest on the start, press, then travel to the end in ten steps
            // and stay there with the button held.
            let t = (frame as f32 - 5.).clamp(0., 10.) / 10.;
            let at = egui::pos2(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            events.push(egui::Event::PointerMoved(at));
            if frame == 4 {
                events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(options.width as f32, options.height as f32),
                )),
                time: Some(frame as f64 / 60.),
                events,
                ..Default::default()
            },
            |ctx| app.ui(ctx),
        );
        painter.textures(&output.textures_delta)?;
        let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
        image = Some(painter.frame(
            &meshes,
            options.width,
            options.height,
            output.pixels_per_point,
        )?);
        for id in output.textures_delta.free {
            painter.textures.remove(&id);
        }
    }
    image.ok_or("No app frame was rendered".into())
}

pub fn save(input: Option<&Path>, output: &Path, options: Options) -> Result<(), String> {
    if input.is_some_and(|input| crate::same_file(input, output)) {
        return Err("Snapshot output must not replace its source image".into());
    }
    if !output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err("The app preview is a PNG image; name the file .png".into());
    }
    let image = render(input, options)?;
    crate::replace_with(output, |file| {
        image
            .write_to(file, image::ImageFormat::Png)
            .map_err(std::io::Error::other)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_triangle_edges_blend_once_and_clipping_is_respected() {
        let id = TextureId::Managed(0);
        let mut painter = Painter::default();
        painter
            .textures
            .insert(id, ColorImage::filled([1, 1], egui::Color32::WHITE));
        let mut mesh = epaint::Mesh::with_texture(id);
        mesh.add_colored_rect(
            egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(8., 8.)),
            egui::Color32::from_rgba_premultiplied(128, 0, 0, 128),
        );
        let frame = painter
            .frame(
                &[epaint::ClippedPrimitive {
                    clip_rect: egui::Rect::from_min_max(egui::pos2(2., 2.), egui::pos2(6., 6.)),
                    primitive: epaint::Primitive::Mesh(mesh),
                }],
                8,
                8,
                1.,
            )
            .unwrap();
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    frame.get_pixel(x, y).0,
                    if (2..6).contains(&x) && (2..6).contains(&y) {
                        [141, 13, 13, 255]
                    } else {
                        [27, 27, 27, 255]
                    }
                );
            }
        }
    }
    #[test]
    fn texture_patch_and_bilinear_sample_keep_channels_and_alpha() {
        let mut painter = Painter::default();
        let id = TextureId::Managed(0);
        let mut delta = egui::TexturesDelta::default();
        delta.set.push((
            id,
            epaint::ImageDelta::full(
                ColorImage::filled([2, 2], egui::Color32::BLACK),
                egui::TextureOptions::LINEAR,
            ),
        ));
        painter.textures(&delta).unwrap();
        delta.set.clear();
        delta.set.push((
            id,
            epaint::ImageDelta::partial(
                [1, 1],
                ColorImage::filled([1, 1], egui::Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
        ));
        painter.textures(&delta).unwrap();
        assert_eq!(
            sample(&painter.textures[&id], 0.5, 0.5),
            [63.75, 63.75, 63.75, 255.]
        );
    }
}
