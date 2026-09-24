#![cfg(feature = "desktop")]
use std::path::PathBuf;
use std::process::Command;

#[test]
fn headless_app_snapshot_uses_real_conversion_and_draws_nodes_on_dark_ui() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(format!("work/app-snapshot-test-{}.png", std::process::id()));
    let source = root.join("kit/fixtures/samples/logo-with-transparency.png");
    let before = std::fs::read(&source).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_vector-magic-preview"))
        .arg("--image")
        .arg(&source)
        .args(["--convert", "--nodes", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let image = image::open(&output).unwrap().to_rgb8();
    assert_eq!(image.dimensions(), (1200, 800));
    // The whole frame is a dark theme: no region brighter than the checkerboard
    // except the pictures themselves, so the corner outside them stays dark.
    let background = image.get_pixel(1190, 745);
    assert!(background.0.iter().all(|v| *v < 60), "{background:?}");
    // The vector card is the right half of the workspace.
    let right_half = (620..1200).flat_map(|x| (90..760).map(move |y| (x, y)));
    let blue_nodes = right_half
        .clone()
        .filter(|&(x, y)| {
            let p = image.get_pixel(x, y);
            p[2] > 180 && p[1] > 140 && p[0] < 120
        })
        .count();
    assert!(blue_nodes > 150, "node markers are missing");
    let gear = right_half
        .filter(|&(x, y)| {
            let p = image.get_pixel(x, y);
            (70..170).contains(&p[0]) && (70..170).contains(&p[1]) && p[2] > 170
        })
        .count();
    assert!(gear > 2000, "converted gear is missing ({gear} pixels)");
    assert_eq!(before, std::fs::read(source).unwrap());
    let _ = std::fs::remove_file(&output);
}

#[test]
fn snapshot_rejects_source_overwrite_and_invalid_dimensions() {
    use vector_magic_rebuild::snapshot::{self, Options};
    let path = PathBuf::from("nonexistent-source.png");
    assert!(snapshot::save(Some(&path), &path, Options::default())
        .unwrap_err()
        .contains("source"));
    assert!(snapshot::render(
        None,
        Options {
            width: 0,
            ..Default::default()
        }
    )
    .is_err());
}

#[test]
fn desktop_binary_can_snapshot_without_entering_the_window_loop() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(format!(
        "work/desktop-headless-test-{}.png",
        std::process::id()
    ));
    let result = Command::new(env!("CARGO_BIN_EXE_vector-magic-desktop"))
        .arg("--snapshot")
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(image::open(&output).unwrap().width(), 1200);
    let _ = std::fs::remove_file(&output);
}
