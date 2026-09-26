//! The tests of the browser build, run natively: the app driven the way the
//! page drives it, a dropped file and Ctrl+Enter, until its drawing settles.

use std::path::PathBuf;
use vector_magic_rebuild::desktop_ui::{platform, Desktop};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bytes(relative: &str) -> Vec<u8> {
    std::fs::read(root().join(relative)).unwrap()
}

/// The pictures, and the command line's own drawing of each through the
/// desktop's chain (--category auto --quality auto --simplify auto
/// --regularize 0.8 --straighten auto --primitives on --stack on), kept in
/// tests/expected.
pub const SAMPLES: [(&str, &str); 3] = [
    (
        "kit/fixtures/samples/logo-with-blending-small.png",
        "logo-with-blending-small",
    ),
    (
        "kit/fixtures/samples/logo-without-blending.png",
        "logo-without-blending",
    ),
    ("kit/fixtures/photos/chelsea.png", "chelsea"),
];

struct Tab {
    ctx: egui::Context,
    app: Desktop,
}

impl Tab {
    fn new(prefs: &str) -> Self {
        let ctx = egui::Context::default();
        let app = Desktop::in_browser(&ctx, prefs);
        Self { ctx, app }
    }
    fn frame(
        &mut self,
        events: Vec<egui::Event>,
        files: Vec<egui::DroppedFile>,
    ) -> egui::FullOutput {
        platform::run_pending();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280., 860.),
            )),
            events,
            dropped_files: files,
            ..Default::default()
        };
        let app = &mut self.app;
        self.ctx.run(raw, |ctx| app.ui(ctx))
    }
    fn drop_file(&mut self, name: &str, data: Vec<u8>) {
        self.frame(
            Vec::new(),
            vec![egui::DroppedFile {
                name: name.to_owned(),
                bytes: Some(data.into()),
                ..Default::default()
            }],
        );
    }
    fn convert(&mut self) -> String {
        self.frame(
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
            Vec::new(),
        );
        for _ in 0..200 {
            self.frame(Vec::new(), Vec::new());
            if let Some(svg) = self.app.settled_svg() {
                return svg;
            }
        }
        panic!("no drawing settled: {}", self.app.status_line());
    }
}

/// The command line's drawing as the app saves it: the app declares the
/// saved size in pixels where the engine's file says points (`Desktop`'s
/// `export_svg`); nothing else differs.
fn as_saved(cli: &str) -> String {
    let (head, rest) = cli.split_once("<svg ").unwrap();
    let (tag, body) = rest.split_once('>').unwrap();
    let tag =
        tag.replacen("pt\" height=\"", "\" height=\"", 1)
            .replacen("pt\" viewBox", "\" viewBox", 1);
    format!("{head}<svg {tag}>{body}")
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn the_app_in_a_tab_draws_what_the_command_line_draws() {
    for (sample, expected) in SAMPLES {
        let mut tab = Tab::new("");
        tab.drop_file(sample.rsplit('/').next().unwrap(), bytes(sample));
        let svg = tab.convert();
        let want = as_saved(
            &String::from_utf8(bytes(&format!("kit/web/tests/expected/{expected}.svg"))).unwrap(),
        );
        if svg != want {
            let got = root().join(format!("work/web/tab-{expected}.svg"));
            let _ = std::fs::write(&got, &svg);
            panic!("{expected} differs from the command line's drawing; the tab's is {got:?}");
        }
    }
}

#[test]
fn the_tab_asks_the_page_for_files_and_downloads() {
    let mut tab = Tab::new("");
    let _ = platform::take_commands();
    // Ctrl+O opens the page's file chooser.
    tab.frame(
        vec![key(egui::Key::O, egui::Modifiers::COMMAND)],
        Vec::new(),
    );
    assert_eq!(platform::take_commands(), [platform::Command::OpenPicker]);
    tab.drop_file("logo.png", bytes(SAMPLES[1].0));
    tab.convert();
    // Ctrl+Shift+S, the app preview: the page reads the canvas back and the
    // tab hands it over as a PNG to download.
    let preview = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
    tab.frame(vec![key(egui::Key::S, preview)], Vec::new());
    assert_eq!(platform::take_commands(), [platform::Command::Screenshot]);
    let shot = egui::Event::Screenshot {
        viewport_id: egui::ViewportId::ROOT,
        user_data: Default::default(),
        image: std::sync::Arc::new(egui::ColorImage::filled([4, 3], egui::Color32::RED)),
    };
    tab.frame(vec![shot], Vec::new());
    let commands = platform::take_commands();
    let Some(platform::Command::Download { name, mime, data }) = commands.last() else {
        panic!("no download: {commands:?}");
    };
    assert_eq!(
        (name.as_str(), *mime),
        ("logo-app-preview.png", "image/png")
    );
    assert!(data.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn a_file_that_is_not_a_picture_is_refused() {
    let mut tab = Tab::new("");
    tab.drop_file("notes.txt", b"not a picture".to_vec());
    assert!(
        tab.app.status_line().contains("format") || tab.app.status_line().contains("picture"),
        "{}",
        tab.app.status_line()
    );
}

#[test]
fn the_apps_light_or_dark_holds_and_its_system_follows_the_page() {
    // The page's event 15, as `app.mjs` sends it: the input header (time,
    // size, pixels per point, largest texture, focus, modifiers), then one
    // event.
    let theme = |dark: u8| {
        let mut bytes = Vec::new();
        bytes.extend(0f64.to_le_bytes());
        for v in [1280f32, 860., 1.] {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend(4096u32.to_le_bytes());
        bytes.extend([1, 0]);
        bytes.extend(1u32.to_le_bytes());
        bytes.extend([15, dark]);
        assert!(crate::app::read_input(&bytes).is_some());
    };
    // Chosen in the app (its Appearance popup, since September 25, 2026),
    // light or dark holds whatever the page says; the page follows it
    // (app.mjs).
    let mut tab = Tab::new(
        "theme=dark
",
    );
    theme(0);
    tab.frame(Vec::new(), Vec::new());
    assert!(tab.ctx.style().visuals.dark_mode);
    // Under System the page's word, the device's, decides.
    let mut tab = Tab::new(
        "theme=system
",
    );
    theme(0);
    tab.frame(Vec::new(), Vec::new());
    assert!(!tab.ctx.style().visuals.dark_mode);
    theme(1);
    tab.frame(Vec::new(), Vec::new());
    assert!(tab.ctx.style().visuals.dark_mode);
    assert!(platform::page_dark() == Some(true));
}
