use std::path::PathBuf;
use vector_magic_rebuild::export::write_vector;

fn folder(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work/export-tests")
        .join(format!("{name}-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80"><path fill="#ff0000" fill-rule="evenodd" d="M10 10H90V70H10Z M30 30V50H70V30Z"/></svg>"##;

#[test]
fn save_formats_preserve_svg_and_emit_vector_pdf_eps() {
    let dir = folder("formats");
    let source = dir.join("source image.png");
    std::fs::write(&source, b"unchanged source").unwrap();
    for extension in ["svg", "PDF", "eps"] {
        let output = dir.join(format!("vector with spaces.{extension}"));
        write_vector(&source, &output, SVG, &Default::default()).unwrap();
        let result = std::fs::read(&output).unwrap();
        match extension {
            "svg" => assert_eq!(result, SVG.as_bytes()),
            "PDF" => assert!(result.starts_with(b"%PDF-")),
            _ => {
                let text = String::from_utf8(result).unwrap();
                assert!(text.starts_with("%!PS-Adobe-3.0 EPSF-3.0"));
                assert!(text.contains("%%BoundingBox: 0 0 75 60"));
                assert!(!text.contains("/ImageType"));
                assert!(text.contains("1 0 0 rg"));
            }
        }
    }
    assert_eq!(std::fs::read(source).unwrap(), b"unchanged source");
}

#[test]
fn export_failures_preserve_source_and_existing_output() {
    let dir = folder("failures");
    let source = dir.join("source.svg");
    std::fs::write(&source, SVG).unwrap();
    assert!(write_vector(&source, &source, SVG, &Default::default()).is_err());
    let output = dir.join("keep.eps");
    std::fs::write(&output, b"previous output").unwrap();
    let alpha = SVG.replace("fill-rule=", "opacity=\"0.5\" fill-rule=");
    let error = write_vector(&source, &output, &alpha, &Default::default()).unwrap_err();
    assert!(error.contains("transparency"));
    assert_eq!(std::fs::read(output).unwrap(), b"previous output");
    assert!(write_vector(&source, &dir.join("wrong.tif"), SVG, &Default::default()).is_err());
    assert_eq!(std::fs::read_to_string(source).unwrap(), SVG);
}

#[test]
fn the_output_format_is_known_before_any_work() {
    use std::path::Path;
    use vector_magic_rebuild::export::{output_kind, OutputKind};
    assert_eq!(output_kind(Path::new("a.svg")), Ok(OutputKind::Svg));
    assert_eq!(output_kind(Path::new("dir.v2/A.PDF")), Ok(OutputKind::Pdf));
    assert_eq!(output_kind(Path::new("a.Eps")), Ok(OutputKind::Eps));
    for bad in ["a.tif", "a", "a.svg.bak", ".svg"] {
        assert!(output_kind(Path::new(bad)).is_err(), "{bad}");
    }
}

#[test]
fn exporter_rejects_external_resources_and_embedded_bitmaps() {
    let dir = folder("resources");
    let source = dir.join("source.png");
    for (i, svg) in [
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="https://invalid.example/image.png"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><path fill="url(https://invalid.example/fill)"/></svg>"#,
    ].iter().enumerate() {
        let output = dir.join(format!("blocked-{i}.pdf"));
        assert!(write_vector(&source, &output, svg, &Default::default()).is_err());
        assert!(!output.exists());
    }
}
