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

/// Illustrator flattens a plain `q`/`Q` and keeps a clipped one as a clip
/// group (measured September 24, 2026), so every opaque group is clipped to
/// the page, found in its own user space under the translations above it.
#[test]
fn every_opaque_group_is_clipped_to_the_page_so_editors_keep_it() {
    let svg = "<svg width=\"20pt\" height=\"10pt\" viewBox=\"-2 0 40 20\"><g id=\"#ff0000ff\"><path fill=\"#ff0000\" d=\"M 0 0 L 10 0 L 10 10 Z\" /></g><g transform=\"translate(5 1)\"><g><path fill=\"#0000ff\" d=\"M 0 0 L 1 0 L 1 1 Z\" /></g></g></svg>";
    let body = content(&to_pdf(svg).unwrap());
    assert!(body.contains("q\n-2 0 40 20 re W n\n1 0 0 rg\n"), "{body}");
    assert!(
        body.contains("q\n-2 0 40 20 re W n\n1 0 0 1 5 1 cm\nq\n-7 -1 40 20 re W n\n0 0 1 rg\n"),
        "{body}"
    );
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

/// 64-bit FNV-1a of `bytes`: a fingerprint of a whole file for the test
/// below, which needs no cryptographic strength.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A hand-written drawing that goes through every branch of the reader and
/// both writers: a translated translucent group, a rectangle, relative,
/// implicit, smooth and quadratic path data, fill and stroke together,
/// stroke alone and both fill rules.
const EVERY_BRANCH: &str = "<svg width=\"120\" height=\"90\" viewBox=\"-5 -5 160 120\"><g transform=\"translate(3 4)\" opacity=\"0.8\" fill=\"#3366cc\"><rect x=\"1\" y=\"2\" width=\"30\" height=\"20\" /><path fill-rule=\"evenodd\" d=\"m10,10 20 0v5h-20zM0 0Q10 0 10 10T20 20\" /></g><path fill=\"#ff8800\" stroke=\"#000\" stroke-width=\"1.5\" stroke-linejoin=\"bevel\" stroke-linecap=\"round\" d=\"M 40 40 C 50 30 60 30 70 40 S 90 50 100 40 L 100 80 H 40 Z\" /><path fill=\"none\" stroke=\"#00ff00\" fill-opacity=\"0.5\" d=\"M 5 100 L 150 100\" /></svg>";

/// The PDF and EPS of the drawings above and three frozen reference
/// documents, fingerprinted before the reader was changed to keep each
/// path's segments (September 24, 2026): the writers must not move a byte.
/// The PDFs of the three references moved once since, on purpose, when
/// each colour group was clipped to the page (the same evening).
#[test]
fn pdf_and_eps_bytes_are_unchanged_by_the_structured_reader() {
    let references = [
        (
            "logo-with-blending-small-high",
            include_str!("../../../fixtures/reference/logo-with-blending-small-high.svg"),
        ),
        (
            "coffee-low",
            include_str!("../../../fixtures/reference/coffee-low.svg"),
        ),
        (
            "astronaut-low",
            include_str!("../../../fixtures/reference/astronaut-low.svg"),
        ),
    ];
    let mut seen = Vec::new();
    for (name, svg) in references {
        seen.push((name, "pdf", fingerprint(&to_pdf(svg).unwrap())));
        seen.push((name, "eps", fingerprint(&to_eps(svg).unwrap())));
    }
    seen.push((
        "every-branch",
        "pdf",
        fingerprint(&to_pdf(EVERY_BRANCH).unwrap()),
    ));
    // EPS refuses the translucent group; the same drawing opaque.
    let opaque = EVERY_BRANCH
        .replace(" opacity=\"0.8\"", "")
        .replace(" fill-opacity=\"0.5\"", "");
    seen.push((
        "every-branch",
        "eps",
        fingerprint(&to_eps(&opaque).unwrap()),
    ));
    let expected: [(&str, &str, u64); 8] = [
        ("logo-with-blending-small-high", "pdf", 12572814889323566374),
        ("logo-with-blending-small-high", "eps", 10622153357145488776),
        ("coffee-low", "pdf", 9032009065180905887),
        ("coffee-low", "eps", 8569045390153395527),
        ("astronaut-low", "pdf", 12434725446547735941),
        ("astronaut-low", "eps", 17390289305250369645),
        ("every-branch", "pdf", 303380013723139878),
        ("every-branch", "eps", 3328680109805976187),
    ];
    assert_eq!(seen, expected);
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
