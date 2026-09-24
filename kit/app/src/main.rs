use std::path::PathBuf;
use vector_magic_rebuild::engine::{self, Options as EngineOptions};
use vector_rebuild::{ImageCategory, Quality};

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if args.is_empty() || args[0] == "--help" {
        println!("Vector Magic rebuild: the recovered engine, in Rust alone
Usage: vector-magic-rebuild INPUT -o OUTPUT.svg|OUTPUT.pdf|OUTPUT.eps [--category blended|unblended|photo|auto] [--quality high|medium|low|auto]
PNG, JPEG, GIF, BMP and PNM are supported. Default: blended artwork, high source quality; auto takes the image type or the source quality the desktop's Auto detects in the prepared image (both auto are the desktop's Auto settings).
Opaque photographs use seam overlap; --photo-seams native keeps the engine's own output.
--defaults improved|original: at high quality, photographs trace at the original's advanced-mode detail ceiling (12,6,6) and blended artwork at its advanced mode with segmentation detail 11 and smoothness 3 (11,3,6), both measured better than the basic presets (improved, the default); original keeps the original's basic presets byte for byte.
No original binary, host process or extracted engine is needed; nothing is downloaded.
--simplify 0.5 merges neighbouring curve pieces of the output within that many source pixels (owned post-processing, off by default); --simplify auto picks the tolerance as the desktop's Auto does: the largest candidate from 0.1 up to 0.5 (aliased artwork), 0.3 (anti-aliased artwork) or 0.1 (photographs) whose rendering changes at most 0.6% of the preview pixels (the statistics report it as \"simplify\"). The desktop's chain is --category auto --quality auto --simplify auto --regularize 0.8 --straighten auto --primitives on --stack on.
--colors N limits the image to N colours before tracing; --background white|black|#rrggbb flattens transparency onto that colour first (both owned preparation, off by default).
--optimizer on runs the engine's optional tangent-continuity Newton pass (BezierFitter+0x1c; off in every preset).
--advanced SEG,SMOOTH,CURVE[,corners=on|off][,aa=on|off][,minpix=N] runs the original's advanced mode instead of the basic preset (--advanced preset: the basic preset, where the improved defaults would run the advanced mode): segmentation complexity, contour smoothness and curve complexity from 1 to 12 (the original's defaults are 5, 6, 6), corner detection, anti-aliasing (on for blended artwork unless said otherwise) and the minimum region size in pixels.
--regularize 0.8 draws every run of pieces that lies within that many source pixels of one straight line as that line, and every run that lies within it of one circle as circle arcs (a full ring becomes a four-piece circle); junction nodes stay put, so fills stay sealed (owned post-processing, off by default; the desktop has it on).
--set KEY=V[,V...] overrides one engine parameter after the preset or the advanced mode chose it (research; the key and the number of values must be the parameter map's).
--stack on writes every region joined with the neighbours painted after it, so each anti-aliased edge blends over a solid colour and no background shows between two colours (the picture is the same; export-time, off by default; the desktop shows and saves stacked).
--primitives on draws every closed outline that the source's pixels show to be a circle, an ellipse, a rectangle or a rounded rectangle as that shape, when it explains those pixels at least as well as the traced outline (artwork only; runs after straightening; owned post-processing, off by default; the desktop has it on).
--straighten 0.8 draws curve pieces that bow at most that many source pixels (or 3% of their length per pixel of it) as straight lines and snaps lines within three degrees of horizontal or vertical, moving no node more than a pixel (owned post-processing, off by default; the desktop has it on); --straighten auto takes the bow the desktop's Auto does for the kind of picture traced: 0.2 on anti-aliased artwork, 0.65 on aliased artwork, 1.2 on photographs.
--sticker on|BORDER,RIM[,shadow] paints a die-cut sticker outline under the shapes (a black border, a white rim, an optional shadow; on sizes them for the image) and grows the canvas to fit; --cut-background on first removes the background shapes of an opaque image so the outline hugs the object (both owned post-processing).");
        return Ok(());
    }
    let cli = parse_args(&args)?;
    // Loaded as the desktop loads: up to 100 megapixels, scaled to the
    // engine's limits and a 1 px side doubled, the drawing then declared at
    // the picture's own size (the defect sweep of September 23, 2026: the
    // CLI refused a 6000 x 4000 photograph the desktop opens).
    let raster = vector_magic_rebuild::load_raster_up_to(
        &cli.input,
        vector_magic_rebuild::DESKTOP_MAX_PIXELS,
    )?;
    let (raster, loaded_as) = vector_magic_rebuild::fit_for_engine(raster)?;
    run_engine(cli, raster, loaded_as)
}

/// Everything the command line said, parsed once.
struct Cli {
    input: PathBuf,
    output: PathBuf,
    native: EngineOptions,
    /// `--category auto` / `--quality auto`: detected in the prepared image.
    auto_category: bool,
    auto_quality: bool,
    /// The `--advanced` text, parsed once the image type is known.
    advanced: Option<String>,
    simplify: Option<Simplify>,
    straighten: Option<vector_rebuild::straighten::StraightenOptions>,
    regularize: Option<f64>,
    primitives: bool,
    stack: bool,
    /// `--set`: engine parameters overridden, for measurement.
    overrides: Vec<engine::Override>,
    prep: vector_magic_rebuild::Preparation,
    sticker: StickerArg,
    cut_background: bool,
}

/// The arguments after the program name, `args[0]` being the input.
fn parse_args(args: &[String]) -> Result<Cli, String> {
    let input = PathBuf::from(&args[0]);
    let mut output = None;
    let mut native = EngineOptions::default();
    let mut advanced: Option<String> = None;
    let (mut auto_category, mut auto_quality) = (false, false);
    let mut simplify: Option<Simplify> = None;
    let mut straighten: Option<vector_rebuild::straighten::StraightenOptions> = None;
    let mut regularize: Option<f64> = None;
    let mut primitives = false;
    let mut overrides: Vec<engine::Override> = Vec::new();
    let mut stack = false;
    let mut prep = vector_magic_rebuild::Preparation::default();
    let mut sticker = StickerArg::Off;
    let mut cut_background = false;
    let mut i = 1;
    while i < args.len() {
        let key = &args[i];
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("Missing value for {key}"))?;
        match key.as_str() {
            "-o" | "--output" => output = Some(PathBuf::from(value)),
            "--category" => {
                auto_category = value == "auto";
                native.category = match value.as_str() {
                    "blended" | "auto" => ImageCategory::AntiAliasedArtwork,
                    "unblended" => ImageCategory::AliasedArtwork,
                    "photo" => ImageCategory::Photograph,
                    _ => return Err("Category must be blended, unblended, photo or auto".into()),
                }
            }
            "--quality" => {
                auto_quality = value == "auto";
                native.quality = match value.as_str() {
                    "high" | "auto" => Quality::High,
                    "medium" => Quality::Medium,
                    "low" => Quality::Low,
                    _ => return Err("Quality must be high, medium, low or auto".into()),
                }
            }
            "--photo-seams" => {
                native.overlap_opaque_photos = match value.as_str() {
                    "overlap" => true,
                    "native" => false,
                    _ => return Err("Photo seams must be overlap or native".into()),
                }
            }
            "--optimizer" => {
                native.optional_optimizer = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("Optimizer must be on or off".into()),
                }
            }
            "--advanced" => advanced = Some(value.clone()),
            "--defaults" => {
                native.owned_defaults = match value.as_str() {
                    "improved" => true,
                    "original" => false,
                    _ => return Err("Defaults must be improved or original".into()),
                }
            }
            "--simplify" => simplify = Some(match value.as_str() {
                "auto" if cfg!(feature = "render") => Simplify::Auto,
                "auto" => return Err(AUTO_SIMPLIFY_NEEDS_DESKTOP.into()),
                tolerance => Simplify::Tolerance(
                    tolerance
                        .parse::<f64>()
                        .ok()
                        .filter(|t| t.is_finite() && *t > 0. && *t <= MAX_SIMPLIFY)
                        .ok_or(
                            "Simplify tolerance must be auto or a number of source pixels above 0 and at most 3",
                        )?,
                ),
            }),
            "--regularize" => {
                regularize = Some(
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v >= 0.)
                        .ok_or("Regularize band must be a non-negative number of source pixels")?,
                )
            }
            "--stack" => {
                stack = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("Stack must be on or off".into()),
                }
            }
            "--primitives" => {
                primitives = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("Primitives must be on or off".into()),
                }
            }
            "--straighten" => {
                use vector_rebuild::straighten::StraightenOptions;
                straighten = Some(match value.as_str() {
                    "auto" => StraightenOptions {
                        auto: true,
                        ..StraightenOptions::default()
                    },
                    bow => StraightenOptions {
                        flatness: bow.parse::<f64>().ok().filter(|t| t.is_finite() && *t >= 0.).ok_or(
                            "Straighten bow tolerance must be auto or a non-negative number of source pixels",
                        )?,
                        ..StraightenOptions::default()
                    },
                })
            }
            "--sticker" => {
                sticker = match value.as_str() {
                    "off" => StickerArg::Off,
                    "on" => StickerArg::Sized,
                    spec => StickerArg::Custom(vector_magic_rebuild::parse_sticker(spec)?),
                }
            }
            "--cut-background" => {
                cut_background = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("Cut background must be on or off".into()),
                }
            }
            "--colors" => {
                prep.colors = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|n| (1..=256).contains(n))
                        .ok_or("Colors must be 1 to 256")?,
                )
            }
            "--set" => {
                let (name, values) = value
                    .split_once('=')
                    .ok_or("--set takes KEY=VALUE[,VALUE...]")?;
                let values = values
                    .split(',')
                    .map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite()))
                    .collect::<Option<Vec<f64>>>()
                    .ok_or("--set values must be finite numbers")?;
                overrides.push((name.to_owned(), values));
            }
            "--background" => {
                prep.background = Some(
                    vector_magic_rebuild::parse_rgb(value)
                        .ok_or("Background must be white, black or #rrggbb")?,
                )
            }
            _ => return Err(format!("Unknown option: {key}")),
        }
        i += 2;
    }
    if let Some(spec) = &advanced {
        // Refused now rather than after loading; parsed again for the
        // detected image type when the category is auto.
        if spec != "preset" {
            engine::parse_advanced(spec, native.category)?;
        }
    }
    let output = output.ok_or("Specify an output with -o OUTPUT.svg|OUTPUT.pdf|OUTPUT.eps")?;
    if vector_magic_rebuild::same_file(&input, &output) {
        return Err("Input and output must be different files".into());
    }
    // The format before any work.
    vector_magic_rebuild::export::output_kind(&output)?;
    Ok(Cli {
        input,
        output,
        native,
        auto_category,
        auto_quality,
        advanced,
        simplify,
        straighten,
        regularize,
        primitives,
        stack,
        overrides,
        prep,
        sticker,
        cut_background,
    })
}

/// The engine backend: the recovered pipeline plus the owned post-processing.
fn run_engine(
    cli: Cli,
    raster: vector_rebuild::raster::Raster,
    loaded_as: Option<(usize, usize)>,
) -> Result<(), String> {
    let Cli {
        input,
        output,
        mut native,
        auto_category,
        auto_quality,
        advanced,
        simplify,
        straighten,
        regularize,
        primitives,
        stack,
        overrides,
        prep,
        sticker,
        cut_background,
    } = cli;
    let raster = if prep.is_identity() {
        raster
    } else {
        prep.apply(&raster)
    };
    // The desktop's Auto: detected in the image the engine is given.
    if auto_category || auto_quality {
        let detected = vector_magic_rebuild::auto::detect(&raster);
        if auto_category {
            native.category = detected.category;
        }
        if auto_quality {
            native.quality = detected.quality;
        }
    }
    match advanced.as_deref() {
        Some("preset") => native.basic_preset = true,
        Some(spec) => native.advanced = Some(engine::parse_advanced(spec, native.category)?),
        None => {}
    }
    let mut doc = engine::vectorize_with(&raster, native, &overrides)?;
    if let Some(palette) = prep.palette_of(&raster) {
        doc = doc.snapped(&palette, None);
    }
    match simplify {
        Some(Simplify::Tolerance(tolerance)) => doc = doc.simplified(tolerance)?,
        #[cfg(feature = "render")]
        Some(Simplify::Auto) => doc = vector_magic_rebuild::auto_simplify_tolerance(&doc)?.1,
        #[cfg(not(feature = "render"))]
        Some(Simplify::Auto) => return Err(AUTO_SIMPLIFY_NEEDS_DESKTOP.into()),
        None => {}
    }
    doc = doc.post_passes(
        regularize.map(|band| vector_rebuild::regularize::RegularizeOptions { band }),
        straighten,
        primitives,
        &[],
    )?;
    if cut_background {
        let islands = vector_rebuild::shapes::islands(doc.svg())?;
        let removals = vector_rebuild::sticker::background_removals(
            &islands,
            raster.width,
            raster.height,
            |x, y| raster.pixels[y * raster.width + x].0[3] == 255,
        );
        if removals.is_empty() {
            return Err("No background shape touches the edge of the picture".into());
        }
        doc = doc.without_islands(&removals)?;
    }
    let drawn = if stack {
        vector_rebuild::stacking::stack_svg(doc.svg())?.0
    } else {
        doc.svg().to_owned()
    };
    let svg = match sticker {
        StickerArg::Off => drawn,
        StickerArg::Sized => vector_rebuild::sticker::apply(
            &drawn,
            &vector_rebuild::sticker::Sticker::for_size(raster.width, raster.height),
        )?,
        StickerArg::Custom(sticker) => vector_rebuild::sticker::apply(&drawn, &sticker)?,
    };
    // Declared at the picture's own size when the engine traced it scaled or
    // widened: the drawing's view box times the picture's size over the
    // engine's (a sticker's margin scales with it).
    let svg = match loaded_as {
        Some((width, height)) => {
            let (view_w, view_h) = vector_magic_rebuild::view_box_size(&svg)
                .ok_or("The drawing has no view box to declare its size by")?;
            let declare = |view: f64, picture: usize, traced: usize| {
                ((view * picture as f64 / traced as f64).round() as u32).max(1)
            };
            vector_magic_rebuild::resize_svg(
                &svg,
                declare(view_w, width, raster.width),
                declare(view_h, height, raster.height),
            )?
        }
        None => svg,
    };
    // Under the improved defaults the numbers are written short; the
    // original's defaults keep its exact file.
    let svg = if native.owned_defaults {
        vector_magic_rebuild::export::compact_svg(&svg)
    } else {
        svg
    };
    vector_magic_rebuild::export::write_vector(&input, &output, &svg)?;
    print!("{}", doc.statistics_json());
    if let Some((width, height)) = loaded_as {
        println!(
            "Traced {width} x {height} px at {} x {} px, the engine's limits, and saved at {width} x {height}",
            raster.width, raster.height
        );
    }
    println!("Saved {}", output.display());
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Simplify {
    Tolerance(f64),
    /// The desktop's automatic tolerance (`auto_simplify_tolerance`), which
    /// compares renderings and so needs the desktop feature's renderer.
    Auto,
}
/// The largest simplify tolerance, the desktop slider's end: at 100 px the
/// curves ran 47 px outside a 200 px logo (the defect sweep of September 23,
/// 2026).
const MAX_SIMPLIFY: f64 = 3.;
const AUTO_SIMPLIFY_NEEDS_DESKTOP: &str =
    "--simplify auto renders the drawing and needs a build with the render (or desktop) feature";

#[derive(Clone, Copy, PartialEq)]
enum StickerArg {
    Off,
    /// Widths sized for the image, as the desktop picks them.
    Sized,
    Custom(vector_rebuild::sticker::Sticker),
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(2);
    }
}
