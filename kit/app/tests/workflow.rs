use std::path::PathBuf;
use std::process::Command;
use vector_magic_rebuild::{load_raster, write_svg};

/// A handled refusal: exit code 2 and an `Error:` line naming the problem,
/// never a panic (which exits 101 and fails `success()` just the same).
fn assert_refused(run: &std::process::Output, needle: &str) {
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(2), "{stderr}");
    assert!(stderr.starts_with("Error: "), "{stderr}");
    assert!(stderr.contains(needle), "{needle:?} not in {stderr}");
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work/app-tests")
        .join(format!("{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cli_loads_raster_exports_svg_and_refuses_to_overwrite_source() {
    let dir = scratch("cli");
    let input = dir.join("two colors.png");
    let output = dir.join("result.svg");
    let source = image::RgbaImage::from_fn(24, 16, |x, _| {
        if x < 12 {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    source.save(&input).unwrap();
    let before = std::fs::read(&input).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([input.to_str().unwrap(), "-o", output.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let svg = std::fs::read_to_string(&output).unwrap();
    assert!(svg.contains("#000000") && svg.contains("#ffffff"));
    assert!(svg.contains("viewBox=\"0 0 24 16\""));
    let rejected = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([input.to_str().unwrap(), "-o", input.to_str().unwrap()])
        .output()
        .unwrap();
    assert_refused(&rejected, "must be different files");
    assert!(write_svg(&input, &input, &svg).is_err());
    assert_eq!(std::fs::read(&input).unwrap(), before);
    let invalid = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            input.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
            "--colors",
            "0",
        ])
        .output()
        .unwrap();
    assert_refused(&invalid, "Colors must be 1 to 256");
    assert_eq!(std::fs::read_to_string(&output).unwrap(), svg);
    // A colour limit and a background reach the engine: at most two fills, all
    // from the two-colour palette, and no transparency left to trace around.
    let limited = output.with_file_name("cli-two-colours.svg");
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            input.to_str().unwrap(),
            "-o",
            limited.to_str().unwrap(),
            "--colors",
            "2",
            "--background",
            "white",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let two = std::fs::read_to_string(&limited).unwrap();
    let fills: std::collections::BTreeSet<&str> = two
        .split("fill=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    assert!(fills.len() <= 2, "{fills:?}");
    // Colour names, non-ASCII text of six bytes, trailing text and signs
    // are refused with the usage line, never a panic.
    for background in ["teal", "#\u{4e2d}\u{6587}", "#ff0000zz", "#+f+f+f"] {
        let bad = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
            .args([
                input.to_str().unwrap(),
                "-o",
                limited.to_str().unwrap(),
                "--background",
                background,
            ])
            .output()
            .unwrap();
        assert_refused(&bad, "Background must be white, black or #rrggbb");
    }
    assert_eq!(std::fs::read_to_string(&limited).unwrap(), two);
}

/// The output's format is checked before the image is even opened: a PNG
/// target is refused naming the formats, for an input that does not exist.
#[test]
fn an_unsupported_output_is_refused_before_any_work() {
    let dir = scratch("early");
    let missing = dir.join("no such image.png");
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            missing.to_str().unwrap(),
            "-o",
            dir.join("photo.png").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_refused(&run, "Choose an SVG, PDF or EPS output filename");
    assert!(!dir.join("photo.png").exists());
    // A supported one gets as far as loading.
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            missing.to_str().unwrap(),
            "-o",
            dir.join("drawing.svg").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&run.stderr).contains("Choose an SVG"));
    // So does a PDF: it is written in-process, with no exporter to find.
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            missing.to_str().unwrap(),
            "-o",
            dir.join("drawing.pdf").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(!stderr.contains("Choose an SVG") && !stderr.contains("exporter"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn regularize_draws_the_ring_as_a_circle_and_rejects_a_bad_band() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/samples/logo-with-transparency.png"
    );
    let dir = scratch("regularize");
    let plain = dir.join("plain.svg");
    let regular = dir.join("regular.svg");
    // Under the original preset the ring's outlines are five pieces each
    // (the improved default already traces them as four).
    for (path, extra) in [(&plain, &[][..]), (&regular, &["--regularize", "0.8"][..])] {
        let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
            .args([
                source,
                "-o",
                path.to_str().unwrap(),
                "--defaults",
                "original",
            ])
            .args(extra)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
    let before = std::fs::read_to_string(&plain).unwrap();
    let after = std::fs::read_to_string(&regular).unwrap();
    let curves = |svg: &str| svg.matches(" C ").count();
    // The ring's two outlines, traced as five pieces each (the hole in the
    // black, the black disc in the purple and its hole), are four arcs each
    // afterwards; the gear's teeth are not circles and keep their pieces.
    let five_piece_loops = |svg: &str| {
        svg.split(" M ")
            .filter(|outline| outline.matches(" C ").count() == 5 && !outline.contains(" L "))
            .count()
    };
    assert_eq!(five_piece_loops(&before), 3, "{}", curves(&before));
    assert_eq!(five_piece_loops(&after), 0, "{}", curves(&after));
    assert!(curves(&after) < curves(&before));
    assert!(after.contains("<path ") && after.trim_end().ends_with("</svg>"));
    let bad = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            source,
            "-o",
            regular.to_str().unwrap(),
            "--regularize",
            "-1",
        ])
        .output()
        .unwrap();
    assert_refused(&bad, "non-negative");
}

#[test]
fn advanced_mode_runs_the_sliders_and_rejects_bad_ones() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/samples/logo-with-blending-small.png"
    );
    let dir = scratch("advanced");
    let basic = dir.join("basic.svg");
    let advanced = dir.join("sliders.svg");
    let plain = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([source, "-o", basic.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(plain.status.success());
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            source,
            "-o",
            advanced.to_str().unwrap(),
            "--advanced",
            "5,9,6,corners=off",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stats = String::from_utf8_lossy(&run.stdout);
    assert!(
        stats.contains("\"advanced\":\"5,9,6,corners=off,aa=on,minpix=0\""),
        "{stats}"
    );
    // A plain run of blended artwork reports the improved default; the
    // original preset reports no advanced settings at all.
    assert!(String::from_utf8_lossy(&plain.stdout)
        .contains("\"advanced\":\"11,3,6,corners=on,aa=on,minpix=0\""));
    let original = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            source,
            "-o",
            basic.to_str().unwrap(),
            "--defaults",
            "original",
        ])
        .output()
        .unwrap();
    assert!(original.status.success());
    assert!(!String::from_utf8_lossy(&original.stdout).contains("\"advanced\""));
    let a = std::fs::read_to_string(&advanced).unwrap();
    assert!(a.contains("<path ") && a.trim_end().ends_with("</svg>"));
    assert_ne!(a, std::fs::read_to_string(&basic).unwrap());
    let bad = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            source,
            "-o",
            advanced.to_str().unwrap(),
            "--advanced",
            "13,6,6",
        ])
        .output()
        .unwrap();
    assert_refused(&bad, "1..12");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn decoder_preserves_alpha_and_rejects_invalid_or_oversized_inputs() {
    let dir = scratch("decode");
    let path = dir.join("alpha.png");
    image::RgbaImage::from_pixel(4, 3, image::Rgba([50, 100, 150, 128]))
        .save(&path)
        .unwrap();
    let raster = load_raster(&path).unwrap();
    assert_eq!((raster.width, raster.height), (4, 3));
    assert!(raster.pixels.iter().all(|p| p.0 == [50, 100, 150, 128]));
    std::fs::write(dir.join("broken.png"), b"not an image").unwrap();
    assert!(load_raster(&dir.join("broken.png")).is_err());
    let huge = dir.join("huge.ppm");
    std::fs::write(&huge, b"P6\n20000 20000\n255\n").unwrap();
    assert!(load_raster(&huge).unwrap_err().contains("16 million"));
}

/// The advanced grammar both command lines share: parts are trimmed, the
/// CLI takes every setting, and the preview takes the desktop card's sliders
/// and refuses the settings the card derives from the image type.
#[test]
fn both_command_lines_read_advanced_settings_in_one_grammar() {
    use vector_magic_rebuild::engine::{parse_advanced, parse_sliders, Sliders};
    use vector_rebuild::ImageCategory;
    let spaced = parse_advanced("5, 6 ,6, corners=off", ImageCategory::Photograph).unwrap();
    assert_eq!(
        (
            spaced.segmentation_complexity,
            spaced.contour_smoothness,
            spaced.curve_complexity,
            spaced.detect_corners,
            spaced.contour_anti_alias
        ),
        (5, 6, 6, false, false)
    );
    let all = parse_advanced("9,4,7,aa=off,minpix=3", ImageCategory::AntiAliasedArtwork).unwrap();
    assert!(!all.contour_anti_alias && all.min_pixels == 3 && all.detect_corners);
    assert_eq!(
        parse_sliders("5, 6 ,6, corners=off").unwrap(),
        Sliders {
            segmentation: 5,
            smoothness: 6,
            curves: 6,
            corners: false,
        }
    );
    assert!(parse_sliders("12,1,3").unwrap().corners);
    for bad in [
        "5,6,6,aa=off",
        "5,6,6,minpix=2",
        "5,6,6,x=1",
        "5,6,6,corners",
        "13,6,6",
    ] {
        assert!(parse_sliders(bad).is_err(), "{bad}");
    }
    assert!(parse_sliders("5,6,6,aa=off").unwrap_err().contains("aa"));
    for bad in [
        "5,6",
        "0,6,6",
        "5,6,6,corners=maybe",
        "5,6,6,minpix=-1",
        "5,6,6,x=1",
    ] {
        assert!(
            parse_advanced(bad, ImageCategory::Photograph).is_err(),
            "{bad}"
        );
    }
}

/// `--category auto --quality auto` is the desktop's Auto: the type and
/// source quality detected in the prepared image, the same drawing as
/// naming them.
#[test]
fn auto_category_and_quality_trace_as_detected() {
    use vector_magic_rebuild::auto::detect;
    use vector_rebuild::{ImageCategory, Quality};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = scratch("auto");
    for (name, expected) in [
        ("fixtures/photos/astronaut.png", ImageCategory::Photograph),
        (
            "fixtures/samples/logo-without-blending.png",
            ImageCategory::AliasedArtwork,
        ),
    ] {
        let source = root.join("kit").join(name);
        let detected = detect(&load_raster(&source).unwrap());
        assert_eq!(detected.category, expected, "{name}");
        let category = match detected.category {
            ImageCategory::AntiAliasedArtwork => "blended",
            ImageCategory::AliasedArtwork => "unblended",
            ImageCategory::Photograph => "photo",
        };
        let quality = match detected.quality {
            Quality::High => "high",
            Quality::Medium => "medium",
            Quality::Low => "low",
        };
        let run = |out: &PathBuf, extra: &[&str]| {
            let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
                .args([source.to_str().unwrap(), "-o", out.to_str().unwrap()])
                .args(extra)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            std::fs::read_to_string(out).unwrap()
        };
        let auto = run(
            &dir.join("auto.svg"),
            &["--category", "auto", "--quality", "auto"],
        );
        let named = run(
            &dir.join("named.svg"),
            &["--category", category, "--quality", quality],
        );
        assert_eq!(auto, named, "{name}");
        assert_ne!(auto, run(&dir.join("default.svg"), &[]), "{name}");
    }
    let bad = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args(["in.png", "-o", "out.svg", "--category", "logo"])
        .output()
        .unwrap();
    assert_refused(&bad, "blended, unblended, photo or auto");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--simplify auto --regularize 0.8 --straighten auto` with Auto settings
/// is the desktop's chain: the tolerance `auto_simplify_tolerance` picks on
/// the engine's drawing, then true lines and circles, then straightening at
/// the bow Auto takes for the picture's kind; the statistics report the
/// tolerance.
#[cfg(feature = "desktop")]
#[test]
fn simplify_auto_reproduces_the_desktop_chain() {
    use vector_magic_rebuild::{auto::detect, engine, AUTO_TOLERANCE_CANDIDATES};
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/samples/logo-with-blending-small.png"
    );
    let dir = scratch("simplify-auto");
    let out = dir.join("desktop.svg");
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            source,
            "-o",
            out.to_str().unwrap(),
            "--category",
            "auto",
            "--quality",
            "auto",
            "--simplify",
            "auto",
            "--regularize",
            &vector_magic_rebuild::desktop_ui::DEFAULT_REGULARIZE_BAND.to_string(),
            "--straighten",
            "auto",
            "--primitives",
            if vector_magic_rebuild::desktop_ui::DEFAULT_PRIMITIVES {
                "on"
            } else {
                "off"
            },
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    // The desktop's job with Auto settings, simplification, true lines and
    // circles and straightening on (desktop_ui/state.rs).
    let raster = load_raster(std::path::Path::new(source)).unwrap();
    let detected = detect(&raster);
    let options = engine::Options {
        category: detected.category,
        quality: detected.quality,
        ..engine::Options::default()
    };
    let raw = engine::vectorize(&raster, options).unwrap();
    let (tolerance, simplified, _) = vector_magic_rebuild::auto_simplify_tolerance(&raw).unwrap();
    // The same function the desktop's derive runs (`Desktop::finish`), with
    // the desktop's own defaults (its straightening is the engine crate's
    // defaults with Auto on, desktop_ui/tests.rs).
    let desktop = simplified
        .post_passes(
            Some(vector_rebuild::regularize::RegularizeOptions {
                band: vector_magic_rebuild::desktop_ui::DEFAULT_REGULARIZE_BAND,
            }),
            Some(vector_rebuild::straighten::StraightenOptions {
                auto: true,
                ..Default::default()
            }),
            vector_magic_rebuild::desktop_ui::DEFAULT_PRIMITIVES,
            &[],
        )
        .unwrap();
    assert!(AUTO_TOLERANCE_CANDIDATES.contains(&tolerance));
    // Saved with its numbers written short, as the desktop saves it.
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        vector_magic_rebuild::export::compact_svg(desktop.svg())
    );
    let stats = String::from_utf8_lossy(&run.stdout);
    assert!(
        stats.contains(&format!("\"simplify\":{tolerance}")),
        "{stats}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The preview command line shares the CLI's sticker and slider parsers, so
/// what one refuses the other refuses too.
#[cfg(feature = "desktop")]
#[test]
fn the_preview_refuses_what_the_cli_refuses() {
    for args in [
        ["--sticker", "4,2,shadow,junk"],
        ["--sticker", "0,0"],
        ["--advanced", "5,6,6,aa=off"],
        ["--background", "#\u{4e2d}\u{6587}"],
    ] {
        let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-preview"))
            .args(args)
            .args(["--output", "never-written.png"])
            .output()
            .unwrap();
        // Refused the way the CLI refuses: exit 2 and an Error: line.
        assert_refused(&run, "");
        assert!(
            !std::path::Path::new("never-written.png").exists(),
            "{args:?}"
        );
    }
}

#[test]
fn stacking_and_true_shapes_run_from_the_command_line_and_refuse_bad_values() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = scratch("stack");
    // The shape set's pixel-edged small details: the turned squares come
    // back square-on with --primitives on, and --stack on adds strips.
    let source = root.join("kit/fixtures/shapes/shape-small-details-aliased.png");
    let run = |name: &str, extra: &[&str]| {
        let out = dir.join(name);
        let result = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
            .arg(&source)
            .arg("-o")
            .arg(&out)
            .args(["--category", "auto", "--quality", "auto"])
            .args(extra)
            .output()
            .unwrap();
        (result, out)
    };
    let (plain, plain_out) = run("plain.svg", &[]);
    assert!(plain.status.success(), "{}", String::from_utf8_lossy(&plain.stderr));
    let (shapes, shapes_out) = run("shapes.svg", &["--primitives", "on", "--stack", "on"]);
    assert!(shapes.status.success(), "{}", String::from_utf8_lossy(&shapes.stderr));
    let (plain, shapes) = (
        std::fs::read_to_string(plain_out).unwrap(),
        std::fs::read_to_string(shapes_out).unwrap(),
    );
    assert!(!plain.contains("fill=\"none\""));
    assert!(shapes.contains("<path fill=\"none\" stroke=\""), "{shapes}");
    assert_ne!(plain, shapes);
    for (flag, needle) in [("--stack", "Stack must be on or off"), ("--primitives", "Primitives must be on or off")] {
        let (bad, _) = run("bad.svg", &[flag, "maybe"]);
        assert_refused(&bad, needle);
    }
}

#[test]
fn engine_parameters_can_be_overridden_and_bad_overrides_are_refused() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = scratch("set");
    let source = root.join("kit/fixtures/shapes/shape-circles-low.png");
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
            .arg(&source)
            .arg("-o")
            .arg(dir.join("out.svg"))
            .args(["--category", "blended", "--quality", "high"])
            .args(extra)
            .output()
            .unwrap()
    };
    let ok = run(&["--advanced", "preset", "--set", "Preprocessor::reduce_noise_order=1"]);
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    assert_refused(&run(&["--set", "Nothing::here=1"]), "No engine parameter is called Nothing::here");
    assert_refused(&run(&["--set", "ContourSmoother::prior_strengths=1,2"]), "takes 3 values, not 2");
    assert_refused(&run(&["--set", "ContourSmoother::perturbation_range"]), "--set takes KEY=VALUE");
}

#[test]
fn the_cli_opens_what_the_desktop_opens_and_saves_it_at_its_own_size() {
    // Wider than the engine's 4096 px and 1 px wide: both traced (scaled and
    // widened) and declared at the picture's size, as the desktop does.
    let dir = scratch("fit");
    for (name, (width, height)) in [("wide", (6000, 40)), ("thin", (1, 20))] {
        let input = dir.join(format!("{name}.png"));
        let output = dir.join(format!("{name}.svg"));
        image::RgbaImage::from_fn(width, height, |x, y| {
            if (x * 2 < width) == (y * 2 < height) {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        })
        .save(&input)
        .unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
            .args([input.to_str().unwrap(), "-o", output.to_str().unwrap()])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(
            stdout.contains(&format!("saved at {width} x {height}")),
            "{stdout}"
        );
        let svg = std::fs::read_to_string(&output).unwrap();
        assert!(
            svg.contains(&format!("width=\"{width}\" height=\"{height}\"")),
            "{svg}"
        );
    }
    // A simplify tolerance past the desktop slider's 3 px is refused.
    let input = dir.join("wide.png");
    let run = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args([
            input.to_str().unwrap(),
            "-o",
            dir.join("far.svg").to_str().unwrap(),
            "--simplify",
            "4",
        ])
        .output()
        .unwrap();
    assert_refused(&run, "at most 3");
}
