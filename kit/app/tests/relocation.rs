#![cfg(windows)]
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn copy(source: &Path, destination: &Path) {
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(source, destination).unwrap();
}

#[test]
fn moved_binary_resolves_local_engine_and_exporter_from_an_unrelated_working_directory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let bundle = root.join(format!(
        "work/relocation-tests/{unique}/moved project with spaces"
    ));
    // Nothing travels with the binary: the engine and the PDF and EPS
    // writers are inside the executable, and no Python or original binary
    // is started (PATH is emptied for the run).
    let exe = bundle.join("bin/vector-magic-rebuild.exe");
    copy(Path::new(env!("CARGO_BIN_EXE_vector-magic-rebuild")), &exe);
    let source = bundle.join("source image.png");
    copy(
        &root.join("kit/fixtures/samples/logo-without-blending.png"),
        &source,
    );
    let before = fs::read(&source).unwrap();
    for extension in ["svg", "pdf", "eps"] {
        let output = bundle.join(format!("result.{extension}"));
        let result = Command::new(&exe)
            .current_dir(std::env::temp_dir())
            .env("PATH", "")
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .args(["--category", "unblended", "--photo-seams", "native", "--defaults", "original"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("\"backend\":\"recovered-rust\""));
        let bytes = fs::read(output).unwrap();
        match extension {
            "svg" => assert_eq!(
                bytes,
                fs::read(root.join("kit/fixtures/reference/logo-without-blending-high.svg"))
                    .unwrap()
            ),
            "pdf" => assert!(bytes.starts_with(b"%PDF-")),
            _ => assert!(bytes.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0")),
        }
    }
    assert!(
        !bundle.join("work/native-jobs").exists(),
        "The engine spawns no helper process and needs no job storage"
    );
    assert_eq!(fs::read(source).unwrap(), before);
}
