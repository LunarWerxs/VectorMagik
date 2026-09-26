//! The engine as an MCP server (`vector-magic-rebuild --mcp`): an AI
//! assistant starts this program and talks to it over stdin and stdout, one
//! JSON-RPC message per line (the Model Context Protocol's stdio transport).
//!
//! Three tools: `vectorize` runs the command line's conversion (the desktop
//! app's Auto settings unless told otherwise) and shows the result, `inspect`
//! says what a file is and what Auto would choose for it, and `view` draws
//! any picture or vector file as a PNG the assistant can look at. The tool
//! arguments become the command line's own flags, so the two can never
//! disagree about an option. Everything runs on this computer; stdout carries
//! only protocol messages, and the notes the command line writes to stderr go
//! into the replies instead.

use serde_json::{json, Map, Value};
use std::io::{BufRead, Write};
use std::path::Path;
use vector_magic_rebuild::import::{self, InputKind};
use vector_rebuild::{ImageCategory, Quality};

/// The protocol revisions this server speaks, newest first: a client asking
/// for one of them gets it, any other gets the newest.
const PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The longer side of the PNG `vectorize` and `view` show, in pixels,
/// unless `view` is asked for another (64 to 2048).
const PREVIEW_SIDE: u32 = 1024;

const INSTRUCTIONS: &str = "VectorMagik traces pictures (PNG, JPEG, GIF, BMP, TIFF, TGA, PNM, \
Photoshop PSD) into clean vectors and saves them as SVG, PDF, EPS, AI, DXF, EMF or PNG, on this \
computer. Call vectorize with an absolute input path; it uses the desktop app's Auto settings and \
returns the saved file's statistics and a picture of the result. Call inspect first when unsure \
what kind of picture it is, and view to look at any picture or vector file. SVG, PDF, AI and EPS \
inputs are vector files: vectorize needs vector convert (save their shapes as they are) or trace.";

/// Serve until stdin closes.
pub fn serve() -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => answer(&message),
            Err(e) => Some(error(Value::Null, -32700, &format!("Parse error: {e}"))),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The reply to one message; none to a notification or a response.
fn answer(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response to a request this server never sends, or no method.
        return (message.get("result").is_none() && message.get("error").is_none())
            .then(|| error(id, -32600, "Invalid request: no method"));
    };
    let empty = Value::Object(Map::new());
    let params = message.get("params").unwrap_or(&empty);
    let result = match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call(params),
        _ => return Some(error(id, -32601, &format!("Unknown method: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(message) => error(id, -32602, &message),
    })
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = PROTOCOLS
        .iter()
        .find(|known| Some(**known) == asked)
        .unwrap_or(&PROTOCOLS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "vectormagik",
            "title": "VectorMagik",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

fn tools() -> Value {
    let input = json!({
        "type": "string",
        "description": "Absolute path of the file: a picture (PNG, JPEG, GIF, BMP, TIFF, TGA, PNM, \
                        Photoshop PSD) or a vector file (SVG, PDF, AI, EPS).",
    });
    let number_or = |words: &[&str], text: &str| {
        json!({
            "anyOf": [{ "type": "number" }, { "type": "string", "enum": words }],
            "description": text,
        })
    };
    json!([
        {
            "name": "vectorize",
            "title": "Trace a picture into a vector file",
            "description": "Trace a picture into clean vector shapes and save them. The output's \
                extension picks the format (SVG, PDF, EPS, AI, DXF, EMF or PNG). With the default \
                settings this is exactly what the desktop app's Auto settings produce; any option \
                below overrides one step. Returns the statistics (shapes, nodes, seconds) and, \
                unless preview is false, a picture of the result.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input": input,
                    "output": {
                        "type": "string",
                        "description": "Absolute path to save to; its extension picks the format: \
                            .svg, .pdf, .eps, .ai, .dxf, .emf or .png. Defaults to the input's \
                            name with .svg beside it. An existing file is replaced.",
                    },
                    "settings": {
                        "type": "string",
                        "enum": ["app", "engine"],
                        "default": "app",
                        "description": "app: the desktop app's Auto settings (image type and \
                            quality detected, curves simplified, true lines and circles, near-straight \
                            curves straightened, true shapes, shapes stacked); engine: the bare \
                            engine at high quality for blended artwork, every step off unless set.",
                    },
                    "category": {
                        "type": "string",
                        "enum": ["auto", "blended", "unblended", "photo"],
                        "description": "The kind of picture: blended (artwork with smooth, \
                            anti-aliased edges), unblended (hard pixel edges), photo; auto detects it.",
                    },
                    "quality": {
                        "type": "string",
                        "enum": ["auto", "high", "medium", "low"],
                        "description": "The source picture's quality (low for blurry or \
                            compressed pictures); auto detects it.",
                    },
                    "colors": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 256,
                        "description": "Limit the picture to this many colours before tracing.",
                    },
                    "background": {
                        "type": "string",
                        "description": "Flatten transparency onto white, black or #rrggbb first.",
                    },
                    "simplify": number_or(&["auto", "off"],
                        "Merge neighbouring curve pieces within this many source pixels (above 0, \
                         at most 3), auto (the app's choice) or off."),
                    "straighten": number_or(&["auto", "off"],
                        "Draw curves that bow at most this many source pixels as straight lines and \
                         snap near-level lines level; auto or off."),
                    "regularize": number_or(&["off"],
                        "Draw runs within this many source pixels of a line or a circle as that \
                         line or circle (the app uses 0.8), or off."),
                    "primitives": {
                        "type": "boolean",
                        "description": "Draw outlines the pixels show to be circles, ellipses, \
                            rectangles or rounded rectangles as those shapes.",
                    },
                    "stack": {
                        "type": "boolean",
                        "description": "Join each shape with those painted after it, so no \
                            background shows between two colours.",
                    },
                    "sticker": {
                        "type": "boolean",
                        "description": "Add a die-cut sticker outline (black border, white rim) \
                            under the shapes.",
                    },
                    "cut_background": {
                        "type": "boolean",
                        "description": "Remove the background shapes touching the picture's edge \
                            first (for an opaque picture with a plain background).",
                    },
                    "vector": {
                        "type": "string",
                        "enum": ["convert", "trace"],
                        "description": "For a vector input only: convert saves its own shapes as \
                            they are in the output's format; trace draws it and traces that.",
                    },
                    "trace_size": {
                        "type": "integer",
                        "minimum": 16,
                        "maximum": 4096,
                        "description": "With vector trace: the drawing's longer side in pixels \
                            (2000 unless given).",
                    },
                    "advanced": {
                        "type": "string",
                        "description": "The engine's advanced mode instead of its preset: \
                            SEG,SMOOTH,CURVE from 1 to 12 (segmentation detail, contour \
                            smoothness, curve detail), optionally ,corners=on|off,aa=on|off,minpix=N.",
                    },
                    "color_groups": {
                        "type": "boolean",
                        "description": "Group the saved shapes by colour (true unless set false).",
                    },
                    "stroke_boundaries": {
                        "type": "boolean",
                        "description": "Stroke every shape with its own colour, hiding hairline seams.",
                    },
                    "dxf": {
                        "type": "string",
                        "enum": ["splines", "fine", "coarse"],
                        "description": "A DXF's curves as spline curves, or as fine or coarse lines.",
                    },
                    "preview": {
                        "type": "boolean",
                        "default": true,
                        "description": "Return a picture of the result.",
                    },
                },
                "required": ["input"],
            },
        },
        {
            "name": "inspect",
            "title": "Describe a picture or vector file",
            "description": "What a file is: for a picture its size, whether it has transparency, \
                how many colours it has and the image type and quality the app's Auto settings \
                would choose; for a vector file its shapes, pages and what a conversion would \
                leave out.",
            "inputSchema": {
                "type": "object",
                "properties": { "input": input },
                "required": ["input"],
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "view",
            "title": "Look at a picture or vector file",
            "description": "Draw a picture or a vector file (SVG, PDF, AI, EPS, including the files \
                vectorize saves) as a PNG image to look at.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input": input,
                    "size": {
                        "type": "integer",
                        "minimum": 64,
                        "maximum": 2048,
                        "description": "The image's longer side in pixels (1024 unless given; a \
                            picture is never enlarged).",
                    },
                },
                "required": ["input"],
            },
            "annotations": { "readOnlyHint": true },
        },
    ])
}

/// `tools/call`: a tool's failure is its result (`isError`), so the assistant
/// reads why; only a malformed call is a protocol error.
fn call(params: &Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("tools/call needs a tool name")?;
    let empty = Value::Object(Map::new());
    let args = params.get("arguments").unwrap_or(&empty);
    let args = args
        .as_object()
        .ok_or("The tool's arguments must be an object")?;
    let run = match name {
        "vectorize" => vectorize as fn(&Map<String, Value>) -> Result<Vec<Value>, String>,
        "inspect" => inspect,
        "view" => view,
        _ => return Err(format!("Unknown tool: {name}")),
    };
    // The engine refuses bad input with an error; a panic is a defect, and
    // it must not take the server down with it.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(args)))
        .unwrap_or_else(|_| Err(format!("{name} stopped on an internal error")));
    Ok(match outcome {
        Ok(content) => json!({ "content": content, "isError": false }),
        Err(message) => json!({ "content": [text(&message)], "isError": true }),
    })
}

fn text(message: &str) -> Value {
    json!({ "type": "text", "text": message })
}

fn image(png: &[u8]) -> Value {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    json!({ "type": "image", "data": STANDARD.encode(png), "mimeType": "image/png" })
}

/// Refuse argument names a tool does not take, so a misspelt option is
/// never silently ignored.
fn only(args: &Map<String, Value>, known: &[&str]) -> Result<(), String> {
    match args.keys().find(|key| !known.contains(&key.as_str())) {
        Some(key) => Err(format!(
            "Unknown argument: {key} (this tool takes {})",
            known.join(", ")
        )),
        None => Ok(()),
    }
}

fn string<'a>(args: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn boolean(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}

fn integer(args: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{key} must be a whole number")),
    }
}

/// A number, or one of the words the option takes, as the flag's value.
fn number_or_word(args: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{key} must be a number or a word")),
    }
}

fn required<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    string(args, key)?.ok_or_else(|| format!("{key} is required"))
}

/// The desktop app's Auto chain, as the command line spells it (its help:
/// "The desktop's chain is ...").
const APP_CHAIN: &[(&str, &str)] = &[
    ("--category", "auto"),
    ("--quality", "auto"),
    ("--simplify", "auto"),
    ("--regularize", "0.8"),
    ("--straighten", "auto"),
    ("--primitives", "on"),
    ("--stack", "on"),
];

/// `vectorize`'s arguments as the command line's: the input, `-o`, then each
/// flag once, the chosen settings' value unless an argument overrides it.
fn command_line(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    only(
        args,
        &[
            "input",
            "output",
            "settings",
            "category",
            "quality",
            "colors",
            "background",
            "simplify",
            "straighten",
            "regularize",
            "primitives",
            "stack",
            "sticker",
            "cut_background",
            "vector",
            "trace_size",
            "advanced",
            "color_groups",
            "stroke_boundaries",
            "dxf",
            "preview",
        ],
    )?;
    let input = required(args, "input")?;
    let output = match string(args, "output")? {
        Some(output) => output.to_owned(),
        None => beside(input),
    };
    let mut flags: Vec<(&str, String)> = match string(args, "settings")?.unwrap_or("app") {
        "app" => APP_CHAIN
            .iter()
            .map(|(f, v)| (*f, (*v).to_owned()))
            .collect(),
        "engine" => Vec::new(),
        other => return Err(format!("settings must be app or engine, not {other}")),
    };
    let mut set = |flag: &'static str, value: Option<String>| {
        flags.retain(|(f, _)| *f != flag);
        if let Some(value) = value {
            flags.push((flag, value));
        }
    };
    let on_off = |b: bool| if b { "on" } else { "off" }.to_owned();
    for (key, flag) in [
        ("category", "--category"),
        ("quality", "--quality"),
        ("background", "--background"),
        ("advanced", "--advanced"),
        ("dxf", "--dxf"),
    ] {
        if let Some(value) = string(args, key)? {
            set(flag, Some(value.to_owned()));
        }
    }
    if let Some(n) = integer(args, "colors")? {
        set("--colors", Some(n.to_string()));
    }
    for (key, flag) in [
        ("simplify", "--simplify"),
        ("straighten", "--straighten"),
        ("regularize", "--regularize"),
    ] {
        if let Some(value) = number_or_word(args, key)? {
            // Off is the command line's default: the flag left out.
            set(flag, (value != "off").then_some(value));
        }
    }
    for (key, flag) in [
        ("primitives", "--primitives"),
        ("stack", "--stack"),
        ("sticker", "--sticker"),
        ("cut_background", "--cut-background"),
    ] {
        if let Some(b) = boolean(args, key)? {
            set(flag, Some(on_off(b)));
        }
    }
    let side = integer(args, "trace_size")?;
    match (string(args, "vector")?, side) {
        (Some("convert"), Some(_)) => return Err("trace_size goes with vector trace".into()),
        (Some("convert"), None) => set("--vector", Some("convert".into())),
        (Some("trace") | None, Some(side)) => set("--vector", Some(format!("trace:{side}"))),
        (Some("trace"), None) => set("--vector", Some("trace".into())),
        (Some(other), _) => return Err(format!("vector must be convert or trace, not {other}")),
        (None, None) => {}
    }
    let mut line = vec![input.to_owned(), "-o".to_owned(), output];
    for (flag, value) in flags {
        line.push(flag.to_owned());
        line.push(value);
    }
    if boolean(args, "color_groups")? == Some(false) {
        line.push("--no-color-groups".into());
    }
    if boolean(args, "stroke_boundaries")? == Some(true) {
        line.push("--stroke-boundaries".into());
    }
    Ok(line)
}

/// The default output: the input's name with `.svg`, beside it (`-traced.svg`
/// for an SVG input, never the input itself).
fn beside(input: &str) -> String {
    let path = Path::new(input);
    let svg = path.with_extension("svg");
    if svg != path {
        return svg.to_string_lossy().into_owned();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{stem}-traced.svg"))
        .to_string_lossy()
        .into_owned()
}

fn vectorize(args: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let line = command_line(args)?;
    let mut notes = Vec::new();
    let outcome = crate::run(&line, &mut notes);
    let mut report: String = notes.iter().map(|note| format!("{note}\n")).collect();
    let outcome = match outcome {
        Ok(outcome) => outcome,
        // The command line's refusal names its flag; say this tool's too.
        Err(error) if error.contains("--vector convert") => {
            return Err(report
                + &error
                + "\nWith this tool: vector \"convert\", or vector \"trace\" with trace_size.")
        }
        Err(error) => return Err(report + &error),
    };
    report.push_str(&outcome.report);
    let mut content = vec![text(report.trim_end())];
    if boolean(args, "preview")? != Some(false) {
        match preview(&outcome.svg, PREVIEW_SIDE) {
            Ok(png) => content.push(image(&png)),
            Err(why) => content.push(text(&format!("No preview: {why}"))),
        }
    }
    Ok(content)
}

#[cfg(feature = "render")]
fn preview(svg: &str, side: u32) -> Result<Vec<u8>, String> {
    vector_magic_rebuild::export::preview_png(svg, side)
}

#[cfg(not(feature = "render"))]
fn preview(_svg: &str, _side: u32) -> Result<Vec<u8>, String> {
    Err("this build has no renderer (the render or desktop feature)".into())
}

fn category_word(category: ImageCategory) -> &'static str {
    match category {
        ImageCategory::AntiAliasedArtwork => "blended",
        ImageCategory::AliasedArtwork => "unblended",
        ImageCategory::Photograph => "photo",
    }
}

fn quality_word(quality: Quality) -> &'static str {
    match quality {
        Quality::High => "high",
        Quality::Medium => "medium",
        Quality::Low => "low",
    }
}

fn read(input: &str) -> Result<(Vec<u8>, InputKind), String> {
    let bytes = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    let kind = import::input_kind(&bytes, input);
    Ok((bytes, kind))
}

fn inspect(args: &Map<String, Value>) -> Result<Vec<Value>, String> {
    only(args, &["input"])?;
    let input = required(args, "input")?;
    let (bytes, kind) = read(input)?;
    let described = if kind.is_vector() {
        let imported = import::read_vector(kind, &bytes)?;
        let shapes = imported.svg.matches("<path").count();
        let holds_picture = shapes == 0 && import::picture_in(kind, &bytes)?.is_some();
        json!({
            "kind": kind.label(),
            "vector": true,
            "shapes": shapes,
            "pages": imported.pages,
            "left_out": imported.skipped,
            "holds_a_picture": holds_picture,
        })
    } else {
        let raster = vector_magic_rebuild::decode_raster_up_to(
            &bytes,
            vector_magic_rebuild::DESKTOP_MAX_PIXELS,
        )?;
        let (width, height) = (raster.width, raster.height);
        let (fitted, loaded_as) = vector_magic_rebuild::fit_for_engine(raster)?;
        let detected = vector_magic_rebuild::auto::detect(&fitted);
        let transparent = fitted.pixels.iter().any(|p| p.0[3] < 255);
        let mut colors = std::collections::HashSet::new();
        for pixel in &fitted.pixels {
            colors.insert(pixel.0);
            if colors.len() > 256 {
                break;
            }
        }
        let colors = if colors.len() > 256 {
            json!("more than 256")
        } else {
            json!(colors.len())
        };
        json!({
            "kind": kind.label(),
            "vector": false,
            "width": width,
            "height": height,
            "traced_at": loaded_as.map(|_| [fitted.width, fitted.height]),
            "transparent": transparent,
            "colors": colors,
            "auto_category": category_word(detected.category),
            "auto_quality": quality_word(detected.quality),
        })
    };
    Ok(vec![text(&described.to_string())])
}

fn view(args: &Map<String, Value>) -> Result<Vec<Value>, String> {
    only(args, &["input", "size"])?;
    let input = required(args, "input")?;
    let side = match integer(args, "size")? {
        Some(side @ 64..=2048) => side as u32,
        Some(_) => return Err("size must be 64 to 2048".into()),
        None => PREVIEW_SIDE,
    };
    let (bytes, kind) = read(input)?;
    let png = if kind.is_vector() {
        preview(&import::read_vector(kind, &bytes)?.svg, side)?
    } else {
        let raster = vector_magic_rebuild::decode_raster_up_to(
            &bytes,
            vector_magic_rebuild::DESKTOP_MAX_PIXELS,
        )?;
        let rgba: Vec<u8> = raster.pixels.iter().flat_map(|p| p.0).collect();
        let picture = image::RgbaImage::from_raw(raster.width as u32, raster.height as u32, rgba)
            .ok_or("The picture's pixels do not fill its size")?;
        let longer = picture.width().max(picture.height());
        let picture = if longer > side {
            let scale = f64::from(side) / f64::from(longer);
            let size = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
            image::imageops::resize(
                &picture,
                size(picture.width()),
                size(picture.height()),
                image::imageops::FilterType::Triangle,
            )
        } else {
            picture
        };
        vector_magic_rebuild::export::png_bytes(
            picture.as_raw(),
            picture.width(),
            picture.height(),
        )?
    };
    Ok(vec![image(&png)])
}
