//! The tests of the browser build, run natively.

use super::*;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bytes(relative: &str) -> Vec<u8> {
    std::fs::read(root().join(relative)).unwrap()
}

#[test]
fn the_browser_draws_what_the_command_line_draws() {
    // tests/expected holds the command line's own output of the desktop's
    // chain (--category auto --quality auto --simplify auto --regularize 0.8
    // --straighten auto --primitives on --stack on) for a blended logo, a
    // pixel-edged logo and a photograph.
    for (sample, expected) in [
        (
            "kit/fixtures/samples/logo-with-blending-small.png",
            "logo-with-blending-small",
        ),
        (
            "kit/fixtures/samples/logo-without-blending.png",
            "logo-without-blending",
        ),
        ("kit/fixtures/photos/chelsea.png", "chelsea"),
    ] {
        let session = convert(&bytes(sample), Settings::default()).unwrap();
        let want =
            String::from_utf8(bytes(&format!("kit/web/tests/expected/{expected}.svg"))).unwrap();
        assert!(
            session.svg() == want,
            "{expected} differs from the command line's drawing"
        );
        assert!(
            !session.stats().contains("\"colors\":0"),
            "{}",
            session.stats()
        );
        assert!(
            session.stats().contains("\"simplify\":"),
            "{}",
            session.stats()
        );
    }
}

#[test]
fn settings_parse_and_refuse_what_they_should() {
    let settings = Settings::parse(
        "category=photo; quality=low; colors=8; background=white; simplify=1.5; straighten=off; regularize=off; primitives=off; stack=off",
    )
    .unwrap();
    assert_eq!(settings.category, Some(ImageCategory::Photograph));
    assert_eq!(settings.quality, Some(Quality::Low));
    assert_eq!(settings.colors, Some(8));
    assert_eq!(settings.background, Some([255, 255, 255]));
    assert_eq!(settings.simplify, Simplify::Tolerance(1.5));
    assert!(
        settings.straighten.is_none()
            && !settings.regularize
            && !settings.primitives
            && !settings.stack
    );
    assert_eq!(Settings::parse("").unwrap(), Settings::default());
    for bad in [
        "colors=0",
        "simplify=9",
        "category=painting",
        "stack=maybe",
        "nonsense=1",
        "simplify",
    ] {
        assert!(Settings::parse(bad).is_err(), "{bad} was accepted");
    }
}

#[test]
fn a_new_setting_draws_again_or_asks_for_a_new_trace() {
    let mut session = convert(
        &bytes("kit/fixtures/samples/logo-with-blending-small.png"),
        Settings::default(),
    )
    .unwrap();
    let auto_svg = session.svg().to_owned();
    derive(&mut session, Settings::parse("simplify=3").unwrap()).unwrap();
    assert_ne!(session.svg(), auto_svg);
    assert!(
        session.stats().contains("\"simplify\":3"),
        "{}",
        session.stats()
    );
    derive(&mut session, Settings::default()).unwrap();
    assert_eq!(
        session.svg(),
        auto_svg,
        "back to the same settings, the same drawing"
    );
    assert!(derive(&mut session, Settings::parse("colors=4").unwrap()).is_err());
}

#[test]
fn the_drawing_saves_as_pdf_and_eps_and_a_bad_file_is_refused() {
    let session = convert(
        &bytes("kit/fixtures/samples/logo-with-blending-small.png"),
        Settings::default(),
    )
    .unwrap();
    assert!(output(&session, 2).unwrap().starts_with(b"%PDF"));
    assert!(output(&session, 3).unwrap().starts_with(b"%!PS"));
    assert_eq!(output(&session, 1).unwrap(), session.svg().as_bytes());
    assert!(convert(b"not a picture", Settings::default()).is_err());
}
