//! The sticker outline through the command line and the saved formats: the
//! shapes are untouched, the outline layers sit under them, the canvas grows,
//! PDF keeps the shadow's transparency and EPS refuses it by name.
use std::path::PathBuf;
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work/app-tests")
        .join(format!("{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .args(args)
        .output()
        .unwrap()
}

/// A handled refusal: exit code 2 and an `Error:` line naming the problem,
/// never a panic.
fn assert_refused(run: &std::process::Output, needle: &str) {
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(2), "{stderr}");
    assert!(stderr.starts_with("Error: "), "{stderr}");
    assert!(stderr.contains(needle), "{needle:?} not in {stderr}");
}

#[test]
fn sticker_and_background_cut_reach_every_saved_format() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = scratch("sticker");
    let source = root.join("kit/fixtures/samples/logo-without-blending.png");
    let source = source.to_str().unwrap();
    let plain = dir.join("plain.svg");
    let run = cli(&[source, "-o", plain.to_str().unwrap()]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let plain = std::fs::read_to_string(&plain).unwrap();
    assert!(plain.contains("viewBox=\"0 0 298 268\""));

    let cut = dir.join("cut.svg");
    let run = cli(&[
        source,
        "-o",
        cut.to_str().unwrap(),
        "--sticker",
        "4,8,shadow",
        "--cut-background",
        "on",
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let cut = std::fs::read_to_string(&cut).unwrap();
    // 4 + 8 + a shadow offset of 5 = 17 on every side.
    assert!(
        cut.contains("<svg width=\"332pt\" height=\"302pt\" viewBox=\"-17 -17 332 302\""),
        "{}",
        &cut[..300]
    );
    let shadow = cut.find("<g id=\"sticker-shadow\"").unwrap();
    let edge = cut.find("<g id=\"sticker-edge\"").unwrap();
    let border = cut.find("<g id=\"sticker-border\"").unwrap();
    let shapes = cut.find("<g id=\"#").unwrap();
    assert!(
        shadow < edge && edge < border && border < shapes,
        "layers under the shapes"
    );
    assert!(cut.contains("stroke-width=\"24.00\" stroke-linejoin=\"round\" stroke-linecap=\"round\" opacity=\"0.35\" transform=\"translate(5 5)\""));
    assert!(cut.contains(
        "<g id=\"sticker-border\" fill=\"#000000\" stroke=\"#000000\" stroke-width=\"8.00\""
    ));
    // The white page shape is gone from the shapes: the document's own paths
    // are fewer than the plain conversion's, and each layer copies them all.
    let plain_paths = plain.matches("<path ").count();
    let shape_paths = cut[shapes..].matches("<path ").count();
    assert!(shape_paths < plain_paths, "{shape_paths} vs {plain_paths}");
    assert_eq!(cut[..shapes].matches("<path d=\"").count(), 3 * shape_paths);

    // PDF keeps the shadow; EPS cannot hold its transparency and says so,
    // and works without it.
    let pdf = dir.join("cut.pdf");
    let run = cli(&[
        source,
        "-o",
        pdf.to_str().unwrap(),
        "--sticker",
        "4,8,shadow",
        "--cut-background",
        "on",
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    let eps = dir.join("cut.eps");
    let run = cli(&[
        source,
        "-o",
        eps.to_str().unwrap(),
        "--sticker",
        "4,8,shadow",
    ]);
    assert_refused(&run, "transparency");
    assert!(!eps.exists());
    let run = cli(&[
        source,
        "-o",
        eps.to_str().unwrap(),
        "--sticker",
        "4,8",
        "--cut-background",
        "on",
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let text = std::fs::read_to_string(&eps).unwrap();
    assert!(text.starts_with("%!PS-Adobe-3.0 EPSF-3.0"));
    assert!(
        text.contains("%%BoundingBox: 0 0 322 292"),
        "12 px more on every side, in points"
    );
    assert!(!text.contains("/ImageType"));

    // `on` sizes the widths for the image; a transparent image has no
    // background to cut; bad specs are refused.
    let sized = dir.join("sized.svg");
    let transparent = root.join("kit/fixtures/samples/logo-with-transparency.png");
    let run = cli(&[
        transparent.to_str().unwrap(),
        "-o",
        sized.to_str().unwrap(),
        "--sticker",
        "on",
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let sized = std::fs::read_to_string(&sized).unwrap();
    assert!(
        sized.contains("viewBox=\"-9 -9 268 268\""),
        "3 + 6 for a 250 px picture"
    );
    assert!(sized.contains("stroke-width=\"18.00\"") && sized.contains("stroke-width=\"6.00\""));
    let run = cli(&[
        transparent.to_str().unwrap(),
        "-o",
        dir.join("none.svg").to_str().unwrap(),
        "--cut-background",
        "on",
    ]);
    assert_refused(&run, "No background shape");
    for bad in ["4", "4,x", "0,0", "4,8,blur", "4,8,shadow,more"] {
        let run = cli(&[
            source,
            "-o",
            dir.join("bad.svg").to_str().unwrap(),
            "--sticker",
            bad,
        ]);
        assert_refused(&run, "sticker");
    }
}

#[cfg(feature = "desktop")]
#[test]
fn preview_snapshot_draws_the_sticker_card_and_the_grown_picture() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(format!(
        "work/app-sticker-snapshot-{}.png",
        std::process::id()
    ));
    let source = root.join("kit/fixtures/samples/logo-without-blending.png");
    let result = Command::new(env!("CARGO_BIN_EXE_vector-magic-preview"))
        .arg("--image")
        .arg(&source)
        .args([
            "--convert",
            "--sticker",
            "4,8,shadow",
            "--cut-background",
            "--height",
            "1400",
            // A picture is shown at most at its own size; 2x brings the
            // 250 px mark back to the size these pixel counts were set at.
            "--zoom",
            "2",
            "--output",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let image = image::open(&output).unwrap().to_rgb8();
    assert_eq!(image.dimensions(), (1200, 1400));
    // The vector card (lower half in the stacked layout) shows the mark on a
    // checkerboard with a white rim around it: plenty of pure white pixels
    // and plenty of pure black, and the checkerboard greys where the page
    // used to be.
    let lower = (320..1180).flat_map(|x| (760..1340).map(move |y| (x, y)));
    let count = |test: &dyn Fn(&image::Rgb<u8>) -> bool| {
        lower
            .clone()
            .filter(|&(x, y)| test(image.get_pixel(x, y)))
            .count()
    };
    let white = count(&|p| p.0.iter().all(|v| *v > 245));
    let black = count(&|p| p.0.iter().all(|v| *v < 12));
    let checker =
        count(&|p| p.0.iter().all(|v| (36..=56).contains(v)) && p[0] == p[1] && p[1] == p[2]);
    assert!(white > 8000, "white rim missing ({white})");
    assert!(black > 20000, "the mark is missing ({black})");
    assert!(checker > 20000, "the page was not cut out ({checker})");
    let _ = std::fs::remove_file(&output);
}
