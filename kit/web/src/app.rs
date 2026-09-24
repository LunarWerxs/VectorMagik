//! The desktop app itself in a browser tab: the same `Desktop` window code,
//! run by egui with a canvas instead of a window. There is no wasm-bindgen
//! (the offline rule), so the page and the module talk through two byte
//! buffers per frame, read and written by `kit/web/js/app.mjs`.
//!
//! All numbers are little-endian. `str` is a u32 byte length and UTF-8;
//! `bytes` is a u32 length and raw bytes. Positions are in points (CSS
//! pixels) from the canvas' top-left corner.
//!
//! **Input**, written by the page into memory from `vm_alloc` and passed to
//! `vm_app_frame(ptr, len)`:
//!
//! ```text
//! f64 time in seconds          f32 width, f32 height (points)
//! f32 pixels per point         u32 largest texture side
//! u8 focused                   u8 modifiers: 1 alt, 2 ctrl, 4 shift, 8 mac_cmd, 16 command
//! u32 event count, then per event a u8 tag and its payload:
//!   1 pointer moved   f32 x, f32 y
//!   2 pointer button  f32 x, f32 y, u8 button (0 primary, 1 secondary,
//!                     2 middle, 3 extra1, 4 extra2), u8 pressed
//!   3 pointer gone
//!   4 wheel           u8 unit (0 point, 1 line, 2 page), f32 dx, f32 dy
//!                     (already negated: how far to move the content)
//!   5 key             str key (KeyboardEvent.key), str code
//!                     (KeyboardEvent.code), u8 pressed, u8 repeat
//!   6 text            str
//!   7 copy            8 cut            9 paste str
//!   10 zoom           f32 factor
//!   11 file           str name, bytes data (dropped or picked)
//!   12 hovering file  u8 on
//!   13 screenshot     u32 width, u32 height, bytes RGBA (top row first)
//!   14 reply          u32 id, u32 HTTP status (0: no response), str body:
//!                     the answer to command 5 `id`
//!   15 theme          u8 dark: the page's light and dark switch, sent at
//!                     start and whenever it changes
//! ```
//!
//! **Output**, at `vm_out_ptr()` / `vm_out_len()` after `vm_app_frame`
//! returned 0 (1: the message is the output, as UTF-8):
//!
//! ```text
//! u32 textures to set, each: u8 kind (0 managed, 1 user), u64 id,
//!     u8 partial, u32 x, u32 y, u32 width, u32 height,
//!     u8 magnify linear, u8 minify linear, u8 wrap (0 clamp, 1 repeat,
//!     2 mirrored), then width * height * 4 bytes of premultiplied sRGBA
//! u32 textures to free, each: u8 kind, u64 id
//! u32 meshes, each: f32 clip min x, min y, max x, max y, u8 kind, u64
//!     texture id, u32 vertex count and 20 bytes per vertex (f32 x, f32 y,
//!     f32 u, f32 v, u8 r, g, b, a: premultiplied sRGBA, egui's Vertex),
//!     u32 index count and u32 indices
//! str CSS cursor
//! str text to copy ("" for none)
//! str URL to open ("" for none), u8 in a new tab
//! f64 seconds until the next frame is wanted (negative: none asked for)
//! u32 commands, each a u8 tag and its payload:
//!   1 open picker     show the file chooser; the file comes back as event 11
//!   2 download        str name, str MIME type, bytes data
//!   3 store prefs     str text, kept for `vm_app_start` next time
//!   4 screenshot      read the canvas once this frame is painted and send
//!                     it back as event 13
//!   5 post JSON       u32 id, str URL, str body: fetch it and send the
//!                     answer back as event 14 (the licence's redeem)
//! ```
//!
//! The module imports `env.vm_now_ms`: milliseconds since 1970 on the page's
//! monotonic clock, `performance.timeOrigin + performance.now()`.

use crate::slice;
use std::cell::RefCell;
use std::sync::Arc;
use vector_magic_rebuild::desktop_ui::{platform, Desktop};

struct Running {
    ctx: egui::Context,
    app: Desktop,
}

thread_local! {
    static RUNNING: RefCell<Option<Running>> = const { RefCell::new(None) };
}

/// Start the app with the preferences the page kept (`store prefs`).
///
/// # Safety
/// `prefs` must point at `prefs_len` readable bytes (or be null with 0).
#[no_mangle]
pub unsafe extern "C" fn vm_app_start(prefs: *const u8, prefs_len: usize) -> u32 {
    // SAFETY: the caller hands over a buffer it allocated with `vm_alloc`.
    let prefs = String::from_utf8_lossy(unsafe { slice(prefs, prefs_len) }).into_owned();
    let ctx = egui::Context::default();
    let mut app = Desktop::in_browser(&ctx, &prefs);
    app.ask_licence_if_new();
    RUNNING.with(|running| *running.borrow_mut() = Some(Running { ctx, app }));
    0
}

/// Run one frame on the input at `input` and leave its output for the page.
///
/// # Safety
/// `input` must point at `input_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn vm_app_frame(input: *const u8, input_len: usize) -> u32 {
    // SAFETY: as above.
    let input = unsafe { slice(input, input_len) };
    let result = RUNNING.with(|running| {
        let mut running = running.borrow_mut();
        let running = running.as_mut().ok_or("vm_app_start was not called")?;
        let raw = read_input(input).ok_or("the frame's input is cut short")?;
        platform::run_pending();
        let app = &mut running.app;
        let output = running.ctx.run(raw, |ctx| app.ui(ctx));
        Ok::<_, String>(write_output(&running.ctx, output))
    });
    match result {
        Ok(bytes) => {
            crate::set_out(bytes);
            0
        }
        Err(message) => {
            crate::set_out(message.into_bytes());
            1
        }
    }
}

/// The drawing the app shows now, as the SVG it saves, into the output; for
/// the Node check that the app in the browser draws what the desktop does.
#[no_mangle]
pub extern "C" fn vm_app_svg() -> u32 {
    let svg = RUNNING.with(|running| {
        running
            .borrow()
            .as_ref()
            .and_then(|running| running.app.settled_svg())
    });
    match svg {
        Some(svg) => {
            crate::set_out(svg.into_bytes());
            0
        }
        None => {
            crate::set_out(Vec::new());
            1
        }
    }
}

/// The app's status line, into the output.
#[no_mangle]
pub extern "C" fn vm_app_status() -> u32 {
    let status = RUNNING.with(|running| {
        running
            .borrow()
            .as_ref()
            .map(|running| running.app.status_line())
    });
    crate::set_out(status.unwrap_or_default().into_bytes());
    0
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.bytes.len() < n {
            return None;
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Option<String> {
        Some(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
    fn pos(&mut self) -> Option<egui::Pos2> {
        Some(egui::pos2(self.f32()?, self.f32()?))
    }
}

pub(crate) fn read_input(bytes: &[u8]) -> Option<egui::RawInput> {
    let mut r = Reader { bytes };
    let time = r.f64()?;
    let size = egui::vec2(r.f32()?, r.f32()?);
    let pixels_per_point = r.f32()?;
    let max_texture_side = r.u32()? as usize;
    let focused = r.u8()? != 0;
    let bits = r.u8()?;
    let modifiers = egui::Modifiers {
        alt: bits & 1 != 0,
        ctrl: bits & 2 != 0,
        shift: bits & 4 != 0,
        mac_cmd: bits & 8 != 0,
        command: bits & 16 != 0,
    };
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        max_texture_side: Some(max_texture_side),
        time: Some(time),
        focused,
        modifiers,
        ..Default::default()
    };
    if let Some(viewport) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
        viewport.native_pixels_per_point = Some(pixels_per_point);
        viewport.inner_rect = raw.screen_rect;
        viewport.focused = Some(focused);
    }
    for _ in 0..r.u32()? {
        match r.u8()? {
            1 => raw.events.push(egui::Event::PointerMoved(r.pos()?)),
            2 => {
                let pos = r.pos()?;
                let button = match r.u8()? {
                    1 => egui::PointerButton::Secondary,
                    2 => egui::PointerButton::Middle,
                    3 => egui::PointerButton::Extra1,
                    4 => egui::PointerButton::Extra2,
                    _ => egui::PointerButton::Primary,
                };
                let pressed = r.u8()? != 0;
                raw.events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                });
            }
            3 => raw.events.push(egui::Event::PointerGone),
            4 => {
                let unit = match r.u8()? {
                    1 => egui::MouseWheelUnit::Line,
                    2 => egui::MouseWheelUnit::Page,
                    _ => egui::MouseWheelUnit::Point,
                };
                let delta = egui::vec2(r.f32()?, r.f32()?);
                raw.events.push(egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                });
            }
            5 => {
                let (key, code) = (r.str()?, r.str()?);
                let (pressed, repeat) = (r.u8()? != 0, r.u8()? != 0);
                let physical_key = egui::Key::from_name(code.strip_prefix("Key").unwrap_or(&code));
                if let Some(key) = egui::Key::from_name(&key).or(physical_key) {
                    raw.events.push(egui::Event::Key {
                        key,
                        physical_key,
                        pressed,
                        repeat,
                        modifiers,
                    });
                }
            }
            6 => raw.events.push(egui::Event::Text(r.str()?)),
            7 => raw.events.push(egui::Event::Copy),
            8 => raw.events.push(egui::Event::Cut),
            9 => raw.events.push(egui::Event::Paste(r.str()?)),
            10 => raw.events.push(egui::Event::Zoom(r.f32()?)),
            11 => {
                let name = r.str()?;
                let data: Arc<[u8]> = r.bytes()?.into();
                raw.dropped_files.push(egui::DroppedFile {
                    name,
                    bytes: Some(data),
                    ..Default::default()
                });
            }
            12 => {
                if r.u8()? != 0 {
                    raw.hovered_files.push(egui::HoveredFile::default());
                }
            }
            13 => {
                let (width, height) = (r.u32()? as usize, r.u32()? as usize);
                let rgba = r.bytes()?;
                if rgba.len() == width * height * 4 {
                    raw.events.push(egui::Event::Screenshot {
                        viewport_id: egui::ViewportId::ROOT,
                        user_data: Default::default(),
                        image: Arc::new(egui::ColorImage::from_rgba_unmultiplied(
                            [width, height],
                            rgba,
                        )),
                    });
                }
            }
            14 => {
                let (id, status) = (r.u32()?, r.u32()?);
                let body = r.str()?;
                platform::deliver_reply(id, u16::try_from(status).unwrap_or(0), body);
            }
            15 => platform::deliver_theme(r.u8()? != 0),
            _ => return None,
        }
    }
    Some(raw)
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }
    fn u32(&mut self, v: usize) {
        self.bytes.extend_from_slice(&(v as u32).to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len());
        self.bytes.extend_from_slice(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    fn texture(&mut self, id: egui::TextureId) {
        match id {
            egui::TextureId::Managed(id) => {
                self.u8(0);
                self.u64(id);
            }
            egui::TextureId::User(id) => {
                self.u8(1);
                self.u64(id);
            }
        }
    }
}

fn write_output(ctx: &egui::Context, output: egui::FullOutput) -> Vec<u8> {
    let mut w = Writer { bytes: Vec::new() };
    let textures = &output.textures_delta;
    w.u32(textures.set.len());
    for (id, delta) in &textures.set {
        w.texture(*id);
        let egui::ImageData::Color(image) = &delta.image;
        let [x, y] = delta.pos.unwrap_or([0, 0]);
        w.u8(u8::from(delta.pos.is_some()));
        w.u32(x);
        w.u32(y);
        w.u32(image.size[0]);
        w.u32(image.size[1]);
        let linear = |filter| u8::from(filter == egui::TextureFilter::Linear);
        w.u8(linear(delta.options.magnification));
        w.u8(linear(delta.options.minification));
        w.u8(match delta.options.wrap_mode {
            egui::TextureWrapMode::ClampToEdge => 0,
            egui::TextureWrapMode::Repeat => 1,
            egui::TextureWrapMode::MirroredRepeat => 2,
        });
        for pixel in &image.pixels {
            w.bytes.extend_from_slice(&pixel.to_array());
        }
    }
    w.u32(textures.free.len());
    for id in &textures.free {
        w.texture(*id);
    }
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    let meshes: Vec<_> = primitives
        .iter()
        .filter_map(|p| match &p.primitive {
            egui::epaint::Primitive::Mesh(mesh) if !mesh.indices.is_empty() => {
                Some((p.clip_rect, mesh))
            }
            _ => None,
        })
        .collect();
    w.u32(meshes.len());
    for (clip, mesh) in meshes {
        w.f32(clip.min.x);
        w.f32(clip.min.y);
        w.f32(clip.max.x);
        w.f32(clip.max.y);
        w.texture(mesh.texture_id);
        w.u32(mesh.vertices.len());
        for v in &mesh.vertices {
            w.f32(v.pos.x);
            w.f32(v.pos.y);
            w.f32(v.uv.x);
            w.f32(v.uv.y);
            w.bytes.extend_from_slice(&v.color.to_array());
        }
        w.u32(mesh.indices.len());
        for i in &mesh.indices {
            w.bytes.extend_from_slice(&i.to_le_bytes());
        }
    }
    let platform_output = &output.platform_output;
    w.str(css_cursor(platform_output.cursor_icon));
    let mut copied = String::new();
    let mut url: Option<(String, bool)> = None;
    for command in &platform_output.commands {
        match command {
            egui::OutputCommand::CopyText(text) => copied.clone_from(text),
            egui::OutputCommand::OpenUrl(open) => url = Some((open.url.clone(), open.new_tab)),
            _ => {}
        }
    }
    w.str(&copied);
    let (url, new_tab) = url.unwrap_or_default();
    w.str(&url);
    w.u8(u8::from(new_tab));
    let root = output.viewport_output.get(&egui::ViewportId::ROOT);
    let mut commands = platform::take_commands();
    if root.is_some_and(|root| {
        root.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Screenshot(_)))
    }) {
        commands.push(platform::Command::Screenshot);
    }
    let repaint = match root {
        _ if platform::has_pending() => 0.,
        Some(root) if root.repaint_delay < std::time::Duration::from_secs(3600) => {
            root.repaint_delay.as_secs_f64()
        }
        _ => -1.,
    };
    w.f64(repaint);
    w.u32(commands.len());
    for command in commands {
        match command {
            platform::Command::OpenPicker => w.u8(1),
            platform::Command::Download { name, mime, data } => {
                w.u8(2);
                w.str(&name);
                w.str(mime);
                w.bytes(&data);
            }
            platform::Command::StorePrefs(text) => {
                w.u8(3);
                w.str(&text);
            }
            platform::Command::Screenshot => w.u8(4),
            platform::Command::PostJson { id, url, body } => {
                w.u8(5);
                w.u32(id as usize);
                w.str(&url);
                w.str(&body);
            }
        }
    }
    w.bytes
}

fn css_cursor(icon: egui::CursorIcon) -> &'static str {
    use egui::CursorIcon as C;
    match icon {
        C::Default => "default",
        C::None => "none",
        C::ContextMenu => "context-menu",
        C::Help => "help",
        C::PointingHand => "pointer",
        C::Progress => "progress",
        C::Wait => "wait",
        C::Cell => "cell",
        C::Crosshair => "crosshair",
        C::Text => "text",
        C::VerticalText => "vertical-text",
        C::Alias => "alias",
        C::Copy => "copy",
        C::Move => "move",
        C::NoDrop => "no-drop",
        C::NotAllowed => "not-allowed",
        C::Grab => "grab",
        C::Grabbing => "grabbing",
        C::AllScroll => "all-scroll",
        C::ResizeHorizontal => "ew-resize",
        C::ResizeNeSw => "nesw-resize",
        C::ResizeNwSe => "nwse-resize",
        C::ResizeVertical => "ns-resize",
        C::ResizeEast => "e-resize",
        C::ResizeSouthEast => "se-resize",
        C::ResizeSouth => "s-resize",
        C::ResizeSouthWest => "sw-resize",
        C::ResizeWest => "w-resize",
        C::ResizeNorthWest => "nw-resize",
        C::ResizeNorth => "n-resize",
        C::ResizeNorthEast => "ne-resize",
        C::ResizeColumn => "col-resize",
        C::ResizeRow => "row-resize",
        C::ZoomIn => "zoom-in",
        C::ZoomOut => "zoom-out",
    }
}
