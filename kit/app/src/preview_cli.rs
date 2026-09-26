use crate::{engine::Options as EngineOptions, snapshot};
use std::path::PathBuf;
use vector_rebuild::{ImageCategory, Quality};
pub fn run() -> Result<(), String> {
    let mut input = None;
    let mut output = None;
    let mut options = snapshot::Options::default();
    let mut simplify_given = false;
    let mut args = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--image" => input = Some(PathBuf::from(args.next().ok_or("Missing image path")?)),
            "--output" | "--snapshot" => {
                output = Some(PathBuf::from(args.next().ok_or("Missing PNG path")?))
            }
            "--convert" => options.convert = true,
            "--look" => {
                options.look = args
                    .next()
                    .as_deref()
                    .and_then(crate::desktop_ui::Look::from_word)
                    .ok_or("--look takes classic, glass or studio")?;
            }
            "--theme" => {
                options.light = match args.next().as_deref() {
                    Some("light") => true,
                    Some("dark") => false,
                    _ => return Err("--theme takes light or dark".into()),
                };
            }
            "--nodes" => options.nodes = true,
            "--show" => {
                options.overlay = Some(match args.next().as_deref() {
                    Some("save") => crate::desktop_ui::Overlay::Save,
                    Some("size") => crate::desktop_ui::Overlay::Size,
                    Some("colors") => crate::desktop_ui::Overlay::Colors,
                    Some("node-menu") => crate::desktop_ui::Overlay::NodeMenu,
                    Some("shapes") => crate::desktop_ui::Overlay::Shapes,
                    Some("licence" | "license") => crate::desktop_ui::Overlay::Licence,
                    Some("appearance") => crate::desktop_ui::Overlay::Appearance,
                    _ => return Err(
                        "--show takes save, size, colors, node-menu, shapes, license or appearance"
                            .into(),
                    ),
                });
            }
            "--round" => {
                let value = args
                    .next()
                    .ok_or("--round takes X,Y[:tiny|tight|medium|wide]")?;
                let (point, reach) = match value.split_once(':') {
                    Some((point, reach)) => (
                        point,
                        crate::desktop_ui::Reach::parse(reach)
                            .ok_or("--round reach is tiny, tight, medium or wide")?,
                    ),
                    None => (value.as_str(), crate::desktop_ui::Reach::Tight),
                };
                let (x, y) = point
                    .split_once(',')
                    .and_then(|(x, y)| {
                        Some((x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?))
                    })
                    .ok_or("--round takes X,Y[:tiny|tight|medium|wide]")?;
                options.rounds.push((x, y, reach));
            }
            "--delete" => {
                const TAKES: &str = "--delete takes X,Y";
                let value = args.next().ok_or(TAKES)?;
                let (x, y) = value
                    .split_once(',')
                    .and_then(|(x, y)| {
                        Some((x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?))
                    })
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .ok_or(TAKES)?;
                options.deletes.push((x, y));
            }
            "--colors" => {
                options.prep.colors = Some(
                    args.next()
                        .and_then(|v| v.parse::<usize>().ok())
                        .filter(|n| (1..=256).contains(n))
                        .ok_or("--colors takes 1 to 256")?,
                );
            }
            "--background" => {
                let value = args
                    .next()
                    .ok_or("--background takes white, black or #rrggbb")?;
                options.prep.background = Some(
                    crate::parse_rgb(&value).ok_or("--background takes white, black or #rrggbb")?,
                );
            }
            "--optimizer" => {
                options.optimizer = match args.next().as_deref() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err("--optimizer takes on or off".into()),
                };
            }
            "--sticker" => {
                // on, or BORDER,RIM[,shadow] in source pixels; off removes it.
                let value = args
                    .next()
                    .ok_or("--sticker takes on, off or BORDER,RIM[,shadow]")?;
                use crate::desktop_ui::StickerChoice;
                options.sticker = match value.as_str() {
                    "off" => StickerChoice::Off,
                    "on" => StickerChoice::Sized,
                    spec => StickerChoice::Custom(crate::parse_sticker(spec)?),
                };
            }
            "--cut-background" => options.cut_background = true,
            "--pointer" => {
                const TAKES: &str = "--pointer takes X,Y in window pixels";
                let value = args.next().ok_or(TAKES)?;
                let (x, y) = value.split_once(',').ok_or(TAKES)?;
                let (x, y): (f32, f32) = (
                    x.trim().parse().map_err(|_| TAKES)?,
                    y.trim().parse().map_err(|_| TAKES)?,
                );
                if !(x.is_finite() && y.is_finite()) {
                    return Err(TAKES.into());
                }
                options.pointer = Some((x, y));
            }
            "--drag" => {
                const TAKES: &str = "--drag takes X1,Y1,X2,Y2 in window pixels";
                let value = args.next().ok_or(TAKES)?;
                let values: Vec<f32> = value
                    .split(',')
                    .map(|v| v.trim().parse::<f32>().map_err(|_| TAKES))
                    .collect::<Result<_, _>>()?;
                let [x0, y0, x1, y1] = values[..] else {
                    return Err(TAKES.into());
                };
                if !values.iter().all(|v| v.is_finite()) {
                    return Err(TAKES.into());
                }
                options.drag = Some(((x0, y0), (x1, y1)));
            }
            "--reconvert" => options.reconvert = true,
            "--straighten" => {
                use crate::desktop_ui::Bow;
                const TAKES: &str = "--straighten takes off, auto or a bow tolerance in pixels";
                let value = args.next().ok_or(TAKES)?;
                options.straighten = match value.as_str() {
                    "off" => None,
                    "auto" => Some(Bow::Auto),
                    bow => Some(Bow::Pixels(
                        bow.parse::<f64>()
                            .ok()
                            .filter(|t| t.is_finite() && *t >= 0.)
                            .ok_or(TAKES)?,
                    )),
                };
            }
            "--advanced" => {
                let value = args
                    .next()
                    .ok_or("--advanced takes SEG,SMOOTH,CURVE[,corners=on|off]")?;
                options.sliders = Some(crate::engine::parse_sliders(&value)?);
            }
            "--regularize" => {
                options.regularize = match args.next().as_deref() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err("--regularize takes on or off".into()),
                };
            }
            "--primitives" => {
                options.primitives = match args.next().as_deref() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err("--primitives takes on or off".into()),
                };
            }
            "--overlay" => {
                options.single_view = match args.next().as_deref() {
                    Some("bitmap") => Some(false),
                    Some("vector") => Some(true),
                    _ => return Err("--overlay takes bitmap or vector".into()),
                };
            }
            "--simplify" => {
                let value = args.next().ok_or("Missing simplify tolerance or off")?;
                options.simplify = if value == "off" {
                    None
                } else {
                    simplify_given = true;
                    Some(
                        value
                            .parse::<f64>()
                            .ok()
                            .filter(|t| t.is_finite() && *t > 0.)
                            .ok_or("Simplify tolerance must be a positive number or off")?,
                    )
                };
            }
            "--width" => {
                options.width = args
                    .next()
                    .ok_or("Missing width")?
                    .parse()
                    .map_err(|_| "Invalid width")?
            }
            "--height" => {
                options.height = args
                    .next()
                    .ok_or("Missing height")?
                    .parse()
                    .map_err(|_| "Invalid height")?
            }
            "--zoom" => {
                options.zoom = args
                    .next()
                    .ok_or("Missing zoom")?
                    .parse()
                    .map_err(|_| "Invalid zoom")?
            }
            "--category" => {
                let value = args.next().ok_or("Missing category")?;
                let manual = options.manual.get_or_insert(EngineOptions::default());
                manual.category = match value.as_str() {
                    "blended" => ImageCategory::AntiAliasedArtwork,
                    "unblended" => ImageCategory::AliasedArtwork,
                    "photo" => ImageCategory::Photograph,
                    _ => return Err("Unknown category".into()),
                };
            }
            "--quality" => {
                let value = args.next().ok_or("Missing quality")?;
                let manual = options.manual.get_or_insert(EngineOptions::default());
                manual.quality = match value.as_str() {
                    "high" => Quality::High,
                    "medium" => Quality::Medium,
                    "low" => Quality::Low,
                    _ => return Err("Unknown quality".into()),
                };
            }
            "--help" => {
                println!("Render the app's own interface to PNG without opening or capturing any window.\nvector-magic-preview --output preview.png [--image source.png --convert --nodes] [--zoom 2] [--width 1200 --height 800] [--simplify 0.5|off] [--show save|size|colors|node-menu|shapes|license|appearance] [--look classic|glass|studio] [--theme dark|light] [--colors N] [--background white|black|#rrggbb] [--round X,Y[:tiny|tight|medium|wide]] [--delete X,Y] [--optimizer on|off] [--sticker on|off|BORDER,RIM[,shadow]] [--cut-background] [--straighten off|auto|PX] [--regularize on|off] [--primitives on|off] [--advanced SEG,SMOOTH,CURVE[,corners=on|off]] [--overlay bitmap|vector] [--pointer X,Y] [--drag X1,Y1,X2,Y2] [--reconvert] [--category blended|unblended|photo --quality high|medium|low]\nAuto settings are used unless manual category/quality is supplied. Size is 850..2400 by 600..1600. Curve simplification defaults to 0.5 px, as in the desktop; under Auto settings a conversion picks its own tolerance, so --simplify PX with --convert needs --category and --quality. --sticker on uses the widths the desktop picks for the image; --cut-background presses the Sticker card's button after converting. --pointer rests the pointer there for the hover states (the rail's resize grip and scroll bar, a node's tooltip); --drag presses at the first point and holds the button at the second (a node dragged); --reconvert converts again after converting and renders while that runs; --delete deletes the shown node nearest X,Y after converting (its two pieces refitted as one curve).");
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }
    let output = output.ok_or("Specify --output preview.png")?;
    // Under Auto settings a conversion picks the simplify tolerance itself,
    // as the desktop's Convert does, so a number given here would be quietly
    // replaced (round two of the Opus 5.5 review).
    if simplify_given && options.convert && options.manual.is_none() {
        return Err("--simplify PX needs --category and --quality: under Auto settings the conversion picks its own tolerance (use --simplify off to show the engine's curves)".into());
    }
    snapshot::save(input.as_deref(), &output, options)?;
    println!("Saved app-only snapshot: {}", output.display());
    Ok(())
}
