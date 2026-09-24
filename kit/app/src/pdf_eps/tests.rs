use super::*;

/// The content stream of a PDF this module wrote (one page, deflated).
fn content(pdf: &[u8]) -> String {
    let text = String::from_utf8_lossy(pdf);
    let start = text.find("5 0 obj").unwrap();
    let stream = pdf[start..]
        .windows(7)
        .position(|w| w == b"stream\n")
        .unwrap()
        + start
        + 7;
    let end = pdf[stream..]
        .windows(10)
        .position(|w| w == b"\nendstream")
        .unwrap()
        + stream;
    let data = miniz_oxide::inflate::decompress_to_vec_zlib(&pdf[stream..end]).unwrap();
    String::from_utf8(data).unwrap()
}

#[test]
fn every_colour_level_reads_back_truncated_or_rounded() {
    for c in 0..=255u8 {
        let text = rgb([c, c, c]);
        let v: f64 = text.split(' ').next().unwrap().parse().unwrap();
        assert_eq!(((v as f32) * 255.) as u8, c, "{text}");
        assert_eq!((v * 255.).round() as u8, c, "{text}");
    }
}

#[test]
fn relative_implicit_and_quadratic_path_data_become_absolute_operators() {
    let ops = path_ops("m10,10 20 0v5h-20zM0 0Q10 0 10 10T20 20").unwrap();
    assert_eq!(
        ops,
        "10 10 m\n30 10 l\n30 15 l\n10 15 l\nh\n0 0 m\n6.6667 0 10 3.3333 10 10 c\n\
         10 16.6667 13.3333 20 20 20 c\n"
    );
    assert!(path_ops("M 0 0 A 5 5 0 0 1 10 10").is_err());
}

#[test]
fn the_view_box_is_mapped_onto_a_page_of_the_declared_size() {
    // 200 px at 96 per inch is 150 pt; y runs down in SVG and up in PDF.
    let svg = "<svg width=\"200\" height=\"100\" viewBox=\"0 0 400 200\"><path fill=\"#ff0000\" d=\"M 0 0 L 400 0 L 400 200 Z\" /></svg>";
    let pdf = to_pdf(svg).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 150 75]"));
    let body = content(&pdf);
    assert!(body.starts_with("0.375 0 0 -0.375 0 75 cm\n"), "{body}");
    assert!(
        body.contains("1 0 0 rg\n0 0 m\n400 0 l\n400 200 l\nh\nf\n"),
        "{body}"
    );
}

#[test]
fn a_translucent_group_is_one_transparency_group_and_eps_refuses_it() {
    let svg = "<svg width=\"10pt\" height=\"10pt\" viewBox=\"0 0 10 10\"><g opacity=\"0.35\" transform=\"translate(1 1)\" fill=\"#000000\" stroke=\"#000000\" stroke-width=\"2\"><path d=\"M 1 1 L 5 1 L 5 5 Z\" /></g><path fill=\"#00ff00\" opacity=\"0.50\" d=\"M 0 0 L 1 0 L 1 1 Z\" /></svg>";
    let pdf = to_pdf(svg).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/G0 << /ca 0.35 /CA 0.35 >>"), "{text}");
    assert!(
        text.contains("/X0 6 0 R") && text.contains("/Subtype /Form"),
        "{text}"
    );
    let body = content(&pdf);
    assert!(body.contains("/G0 gs\n/X0 Do\n"), "{body}");
    // A fill alone takes its opacity directly.
    assert!(body.contains("/G1 gs\n0 1 0 rg\n"), "{body}");
    let error = to_eps(svg).unwrap_err();
    assert!(error.contains("transparency"), "{error}");
}

#[test]
fn eps_fills_then_strokes_one_path_and_starts_on_white() {
    let svg = "<svg width=\"8\" height=\"4\" viewBox=\"0 0 8 4\"><path fill=\"#ffffff\" stroke=\"#000000\" stroke-width=\"0.5\" stroke-linejoin=\"round\" d=\"M 1 1 L 7 1 L 7 3 Z\" /></svg>";
    let eps = String::from_utf8(to_eps(svg).unwrap()).unwrap();
    assert!(
        eps.starts_with("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 6 3\n"),
        "{eps}"
    );
    assert!(eps.contains("1 1 1 rg 0 0 6 3 re f\n"), "{eps}");
    assert!(
        eps.contains("1 1 m\n7 1 l\n7 3 l\nh\nq 1 1 1 rg f Q\n0 0 0 rg 0.5 w 1 j 0 J 4 M\nS\n"),
        "{eps}"
    );
}

#[test]
fn anything_but_the_app_s_own_shapes_is_refused() {
    for svg in [
        "<svg width=\"1\" height=\"1\"><image href=\"x.png\" /></svg>",
        "<svg width=\"1\" height=\"1\"><text>hi</text></svg>",
        "<svg width=\"1\" height=\"1\"><path fill=\"url(#g)\" d=\"M 0 0\" /></svg>",
        "<svg width=\"1\" height=\"1\"><g transform=\"rotate(45)\"></g></svg>",
    ] {
        assert!(to_pdf(svg).is_err(), "{svg}");
    }
}
