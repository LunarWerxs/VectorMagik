use super::*;

/// A PDF of `objects` (object 1 first) with a classic cross-reference table
/// and `trailer` entries besides `/Size`.
fn pdf(objects: &[Vec<u8>], trailer: &str) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        table += &format!("{offset:010} 00000 n \n");
    }
    table += &format!(
        "trailer\n<< /Size {} {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    );
    out.extend_from_slice(table.as_bytes());
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn deflated(dict: &str, data: &[u8]) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
    stream(&format!("{dict} /Filter /FlateDecode"), &packed)
}

/// Catalog 1, pages 2, a 200 by 100 page 3 (with `page` entries added),
/// its content 4, and `extra` objects from 5 on.
fn page_objects(page: &str, content: &[u8], extra: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R {page} >>")
            .into_bytes(),
        deflated("", content),
    ];
    objects.extend_from_slice(extra);
    objects
}

fn page_pdf(page: &str, content: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
    pdf(&page_objects(page, content, extra), "/Root 1 0 R")
}

fn paths(svg: &str) -> Vec<&str> {
    svg.lines().filter(|l| l.starts_with("<path")).collect()
}

fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = element.find(&key)? + key.len();
    Some(&element[start..start + element[start..].find('"')?])
}

/// The numbers of path data, in order.
fn coordinates(d: &str) -> Vec<f64> {
    d.split(|c: char| c.is_ascii_alphabetic() || c == ' ' || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| t.parse().unwrap())
        .collect()
}

/// Where `needle` last starts in `data`.
fn last(data: &[u8], needle: &[u8]) -> usize {
    data.windows(needle.len())
        .rposition(|w| w == needle)
        .unwrap()
}

fn commands(d: &str) -> String {
    d.chars().filter(char::is_ascii_alphabetic).collect()
}

#[test]
fn a_classic_file_gives_its_rectangle_in_page_space_y_down() {
    let file = page_pdf("", b"1 0 0 rg 10 20 30 40 re f", &[]);
    let imported = to_svg(&file, 0).unwrap();
    assert!(
        imported.svg.starts_with(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200pt\" height=\"100pt\" \
             viewBox=\"0 0 200 100\">\n"
        ),
        "{}",
        imported.svg
    );
    assert_eq!(
        paths(&imported.svg),
        ["<path fill=\"#ff0000\" d=\"M10 80L40 80L40 40L10 40Z\"/>"]
    );
    assert_eq!((imported.pages, imported.skipped.len()), (1, 0));
    // What comes out is what the app's own writer reads.
    crate::pdf_eps::to_pdf(&imported.svg).unwrap();
    let error = to_svg(&file, 1).unwrap_err();
    assert!(error.contains("1 page;"), "{error}");
}

#[test]
fn an_xref_stream_with_an_object_stream_and_a_png_predictor_is_read() {
    let members = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>",
    ];
    let (mut header, mut body) = (String::new(), String::new());
    for (i, member) in members.iter().enumerate() {
        header += &format!("{} {} ", i + 1, body.len());
        body += member;
        body.push(' ');
    }
    let object_stream = deflated(
        &format!("/Type /ObjStm /N 3 /First {}", header.len()),
        format!("{header}{body}").as_bytes(),
    );
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offsets = Vec::new();
    for (number, object) in [
        (4, deflated("", b"0 0 1 rg 0 0 10 10 re f")),
        (5, object_stream),
    ] {
        offsets.push(out.len());
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        out.extend_from_slice(&object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    // Rows of type, offset or object stream (2 bytes), generation or index.
    let rows: Vec<[u8; 4]> = vec![
        [0, 0, 0, 255],
        [2, 0, 5, 0],
        [2, 0, 5, 1],
        [2, 0, 5, 2],
        [1, (offsets[0] >> 8) as u8, offsets[0] as u8, 0],
        [1, (offsets[1] >> 8) as u8, offsets[1] as u8, 0],
        [1, (xref_at >> 8) as u8, xref_at as u8, 0],
    ];
    // PNG "Up": each byte less the one above it, after a filter byte of 2.
    let mut predicted = Vec::new();
    let mut above = [0u8; 4];
    for row in &rows {
        predicted.push(2);
        predicted.extend(row.iter().zip(above).map(|(b, a)| b.wrapping_sub(a)));
        above = *row;
    }
    let xref = deflated(
        "/Type /XRef /Size 7 /Index [0 7] /W [1 2 1] /Root 1 0 R \
         /DecodeParms << /Predictor 12 /Columns 4 >>",
        &predicted,
    );
    out.extend_from_slice(b"6 0 obj\n");
    out.extend_from_slice(&xref);
    out.extend_from_slice(format!("\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let imported = to_svg(&out, 0).unwrap();
    assert_eq!(
        paths(&imported.svg),
        ["<path fill=\"#0000ff\" d=\"M0 100L10 100L10 90L0 90Z\"/>"]
    );
}

#[test]
fn an_incremental_update_wins_over_what_it_replaces() {
    let mut file = page_pdf("", b"1 0 0 rg 0 0 10 10 re f", &[]);
    let text = String::from_utf8_lossy(&file).into_owned();
    let previous = text[text.rfind("startxref").unwrap() + 10..]
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let replaced = file.len();
    file.extend_from_slice(b"4 0 obj\n");
    file.extend_from_slice(&stream("", b"0 0 1 rg 0 0 10 10 re f"));
    file.extend_from_slice(b"\nendobj\n");
    let xref = file.len();
    file.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \n4 1\n{replaced:010} 00000 n \n\
             trailer\n<< /Size 5 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let imported = to_svg(&file, 0).unwrap();
    assert_eq!(attr(paths(&imported.svg)[0], "fill"), Some("#0000ff"));
}

#[test]
fn a_broken_cross_reference_is_rebuilt_by_scanning() {
    let expected = ["<path fill=\"#ff0000\" d=\"M0 100L10 100L10 90L0 90Z\"/>"];
    let file = page_pdf("", b"1 0 0 rg 0 0 10 10 re f", &[]);
    // No table and no startxref, only a trailer.
    let cut = last(&file, b"\nxref\n") + 1;
    let mut no_table = file[..cut].to_vec();
    no_table.extend_from_slice(b"trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    assert_eq!(paths(&to_svg(&no_table, 0).unwrap().svg), expected);
    // Every offset in the table six bytes off, and a /Length that lies.
    let mut shifted = file[..15].to_vec();
    shifted.extend_from_slice(b"%junk\n");
    shifted.extend_from_slice(&file[15..]);
    let xref = last(&shifted, b"\nxref\n") + 1;
    let start = last(&shifted, b"startxref\n") + 10;
    let mut fixed = shifted[..start].to_vec();
    fixed.extend_from_slice(format!("{xref}\n%%EOF\n").as_bytes());
    let length = last(&fixed, b"/Length ") + 8;
    fixed[length] = if fixed[length] == b'9' { b'8' } else { b'9' };
    assert_eq!(paths(&to_svg(&fixed, 0).unwrap().svg), expected);
    // Junk before the header, offsets counted from the header.
    let mut prefixed = b"JUNK JUNK\n".to_vec();
    prefixed.extend_from_slice(&file);
    assert_eq!(paths(&to_svg(&prefixed, 0).unwrap().svg), expected);
}

#[test]
fn a_form_is_drawn_through_its_matrix_and_a_curve_through_the_ctm() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Matrix [2 0 0 2 10 10]",
        b"0 1 0 rg 0 0 5 5 re f",
    );
    let file = page_pdf(
        "/Resources << /XObject << /Fm1 5 0 R >> >>",
        b"q 1 0 0 1 50 0 cm /Fm1 Do Q\n\
          2 0 0 2 0 0 cm 1.5 w 1 J 1 j 0 0 m 10 0 5 5 v 10 10 5 5 y S",
        &[form],
    );
    let imported = to_svg(&file, 0).unwrap();
    assert_eq!(
        paths(&imported.svg),
        [
            "<path fill=\"#00ff00\" d=\"M60 90L70 90L70 80L60 80Z\"/>",
            "<path fill=\"none\" stroke=\"#000000\" stroke-width=\"3\" stroke-linejoin=\"round\" \
             stroke-linecap=\"round\" d=\"M0 100C0 100 20 100 10 90C20 80 10 90 10 90\"/>"
        ]
    );
}

#[test]
fn text_images_clipping_dashes_and_gradients_are_left_out_and_said() {
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 \
         /ColorSpace /DeviceGray",
        &[0x80],
    );
    // The first inline image's data holds " EI " itself: its size says
    // where it ends. The second is filtered and found by its EI.
    let mut content = b"BT /F1 12 Tf 10 10 Td (Hi) Tj ET\nq 10 0 0 10 0 0 cm /Im1 Do Q\n\
        BI /W 2 /H 2 /BPC 8 /CS /G ID "
        .to_vec();
    content.extend_from_slice(b" EI ");
    content.extend_from_slice(b"\nEI\nBI /W 1 /H 1 /F /AHx ID 80> EI\n");
    content.extend_from_slice(b"0 0 10 10 re W n [3 2] 0 d /Sh0 sh\n0 0 1 rg 1 1 2 2 re f\n");
    let file = page_pdf(
        "/Resources << /XObject << /Im1 5 0 R >> >>",
        &content,
        &[image],
    );
    let imported = to_svg(&file, 0).unwrap();
    assert_eq!(
        paths(&imported.svg),
        ["<path fill=\"#0000ff\" d=\"M1 99L3 99L3 97L1 97Z\"/>"]
    );
    assert_eq!(
        imported.skipped,
        [TEXT, "3 images", CLIPPING, DASHES, SHADINGS]
    );
}

#[test]
fn device_indexed_and_spot_colours_become_rgb() {
    let resources = "/Resources << /ColorSpace << \
        /CS0 [/Separation /Spot /DeviceCMYK << /FunctionType 2 /Domain [0 1] \
        /C0 [0 0 0 0] /C1 [0 1 0 0] /N 1 >>] \
        /CS1 [/Indexed /DeviceRGB 1 <000000 0000ff>] \
        /CS2 [/Separation /Other /DeviceRGB 5 0 R] >> >>";
    let sampled = stream("/FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1]", b"");
    let content = b"0 1 1 0 k 0 0 1 1 re f\n0.5 g 0 0 1 1 re f\n\
        1 0 0 0 K 2 w 0 0 1 1 re S\n/CS0 cs 0.5 scn 0 0 1 1 re f\n\
        /CS1 cs 1 sc 0 0 1 1 re f\n/CS2 cs 1 scn 0 0 1 1 re f\n\
        /Pattern cs /P0 scn 0 0 1 1 re f\n";
    let imported = to_svg(&page_pdf(resources, content, &[sampled]), 0).unwrap();
    let colours: Vec<_> = paths(&imported.svg)
        .iter()
        .map(|p| (attr(p, "fill").unwrap(), attr(p, "stroke")))
        .collect();
    assert_eq!(
        colours,
        [
            ("#ff0000", None),
            ("#808080", None),
            ("none", Some("#00ffff")),
            ("#ff80ff", None),
            ("#0000ff", None),
            ("#000000", None),
            ("#808080", None),
        ]
    );
    assert_eq!(imported.skipped, [SPOT, PATTERNS]);
}

#[test]
fn opacity_comes_from_the_graphics_state() {
    let file = page_pdf(
        "/Resources << /ExtGState << /GS0 << /ca 0.5 /CA 0.25 /LW 4 >> >> >>",
        b"/GS0 gs 1 0 0 rg 0 0 1 RG 0 0 10 10 re B",
        &[],
    );
    let imported = to_svg(&file, 0).unwrap();
    assert_eq!(
        paths(&imported.svg),
        [
            "<path fill=\"#ff0000\" fill-opacity=\"0.5\" stroke=\"#0000ff\" stroke-width=\"4\" \
             stroke-opacity=\"0.25\" d=\"M0 100L10 100L10 90L0 90Z\"/>"
        ]
    );
}

#[test]
fn a_transparency_group_keeps_the_opacity_it_was_drawn_with() {
    // Illustrator's translucent groups: `/Half gs /G Do`, and inside the
    // group `/Full gs`, which starts its own content at full opacity.
    let own = "/Resources << /ExtGState << /Full << /ca 1 /CA 1 >> >> >>";
    let group = stream(
        &format!("/Subtype /Form /BBox [0 0 9 9] /Group << /S /Transparency >> {own}"),
        b"/Full gs 0 0 1 1 re f",
    );
    let plain = stream(
        &format!("/Subtype /Form /BBox [0 0 9 9] {own}"),
        b"/Full gs 0 0 1 1 re f",
    );
    let file = page_pdf(
        "/Resources << /ExtGState << /Half << /ca 0.5 /CA 0.5 >> >> \
         /XObject << /G 5 0 R /P 6 0 R >> >>",
        b"/Half gs /G Do /Half gs /P Do",
        &[group, plain],
    );
    let imported = to_svg(&file, 0).unwrap();
    let opacities: Vec<_> = paths(&imported.svg)
        .iter()
        .map(|p| attr(p, "fill-opacity"))
        .collect();
    assert_eq!(opacities, [Some("0.5"), None]);
}

#[test]
fn a_turned_page_and_inherited_boxes_are_placed() {
    let turned = page_pdf("/Rotate 90", b"0 0 10 10 re f", &[]);
    let imported = to_svg(&turned, 0).unwrap();
    assert!(imported.svg.contains("width=\"100pt\" height=\"200pt\""));
    assert_eq!(
        paths(&imported.svg),
        ["<path fill=\"#000000\" d=\"M0 0L0 10L10 10L10 0Z\"/>"]
    );
    // The MediaBox and rotation from the page tree, the CropBox the page's.
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 /MediaBox [0 0 300 300] /Rotate 180 >>"
            .to_vec(),
        b"<< /Type /Page /Parent 2 0 R /CropBox [10 10 110 60] /Contents 4 0 R >>".to_vec(),
        stream("", b"10 10 m 20 10 l S"),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Rotate 0 >>".to_vec(),
    ];
    let file = pdf(&objects, "/Root 1 0 R");
    let first = to_svg(&file, 0).unwrap();
    assert_eq!(first.pages, 2);
    assert!(first.svg.contains("width=\"100pt\" height=\"50pt\""));
    assert_eq!(attr(paths(&first.svg)[0], "d"), Some("M100 0L90 0"));
    let second = to_svg(&file, 1).unwrap();
    assert!(second.svg.contains("width=\"300pt\" height=\"300pt\""));
    assert_eq!(attr(paths(&second.svg)[0], "d"), Some("M10 290L20 290"));
}

#[test]
fn a_protected_file_is_refused_in_plain_words() {
    let objects = page_objects("", b"0 0 10 10 re f", &[]);
    let file = pdf(
        &objects,
        "/Root 1 0 R /Encrypt << /Filter /Standard /V 1 /R 2 >>",
    );
    let error = to_svg(&file, 0).unwrap_err();
    assert!(
        error.starts_with("This PDF is protected by a password"),
        "{error}"
    );
}

#[test]
fn garbage_and_every_truncation_or_corruption_is_refused_or_read_without_panicking() {
    for bytes in [
        &b""[..],
        b"hello",
        b"\x89PNG\r\n",
        b"%PDF-1.4\n1 0 obj << /Type",
    ] {
        assert!(to_svg(bytes, 0).is_err());
    }
    assert!(to_svg(b"%PDF-1.4\n", 0).unwrap_err().contains("damaged"));
    let form = stream("/Subtype /Form /BBox [0 0 1 1]", b"0 0 1 1 re f");
    let file = page_pdf(
        "/Resources << /XObject << /F 5 0 R >> >>",
        b"q /F Do Q BI /W 1 /H 1 ID x EI 1 0 0 rg 0 0 5 5 re f",
        &[form],
    );
    for end in 0..file.len() {
        let _ = to_svg(&file[..end], 0);
    }
    for at in 0..file.len() {
        for b in [b'(', b'[', b'<', b'0', b' ', 0xff] {
            let mut bad = file.clone();
            bad[at] = b;
            let _ = to_svg(&bad, 0);
        }
    }
}

#[test]
fn hostile_nesting_and_expansion_end() {
    // A form that draws itself twice: 2^32 copies if nothing stopped it.
    let form = stream(
        "/Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /F 5 0 R >> >>",
        b"/F Do /F Do",
    );
    let file = page_pdf(
        "/Resources << /XObject << /F 5 0 R >> >>",
        b"/F Do",
        &[form],
    );
    let imported = to_svg(&file, 0).unwrap();
    assert_eq!(imported.skipped, [DEEP, TOO_MUCH]);
    // Ten thousand nested arrays, and unbalanced q and Q.
    let mut content = vec![b'['; 10_000];
    content.extend(vec![b']'; 10_000]);
    content.extend_from_slice(b" Q Q Q q q 0 0 1 1 re f");
    let imported = to_svg(&page_pdf("", &content, &[]), 0).unwrap();
    assert_eq!(paths(&imported.svg).len(), 1);
    // Content that expands past what is read (16 MB under test) is noted,
    // not read: each two bytes of run-length data are 128 spaces.
    let bomb: Vec<u8> = [129, b' '].repeat(140_000);
    let mut objects = page_objects("", b"", &[]);
    objects[3] = stream("/Filter /RunLengthDecode", &bomb);
    let imported = to_svg(&pdf(&objects, "/Root 1 0 R"), 0).unwrap();
    assert!(
        imported.skipped[0].starts_with("part of the page, which could not be read"),
        "{:?}",
        imported.skipped
    );
}

/// `svg` through the app's own PDF writer and back: the same paths, in
/// order, with the same colours, at `scale` points per user unit, each
/// path moved by its group's `offsets`.
fn round_trip(svg: &str, scale: f64, offsets: &[(f64, f64)]) {
    let pdf = crate::pdf_eps::to_pdf(svg).unwrap();
    let imported = to_svg(&pdf, 0).unwrap();
    assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
    let originals: Vec<&str> = svg
        .split("<path")
        .skip(1)
        .map(|p| &p[..p.find("/>").unwrap()])
        .collect();
    let read = paths(&imported.svg);
    assert_eq!(read.len(), originals.len(), "{}", imported.svg);
    for ((original, back), offset) in originals.iter().zip(&read).zip(offsets) {
        let original = format!(" {}", original.trim_start());
        let (d0, d1) = (attr(&original, "d").unwrap(), attr(back, "d").unwrap());
        assert_eq!(commands(d0), commands(d1));
        let (c0, c1) = (coordinates(d0), coordinates(d1));
        assert_eq!(c0.len(), c1.len());
        for (i, (a, b)) in c0.iter().zip(&c1).enumerate() {
            let shift = if i % 2 == 0 { offset.0 } else { offset.1 };
            assert!(((a + shift) * scale - b).abs() < 0.01, "{d0} -> {d1}");
        }
        for key in [
            "fill",
            "stroke",
            "fill-rule",
            "stroke-linejoin",
            "stroke-linecap",
        ] {
            let expected = attr(&original, key).or(match key {
                "fill" => Some("#000000"),
                _ => None,
            });
            assert_eq!(attr(back, key), expected, "{key} of {back}");
        }
        if let Some(width) = attr(&original, "stroke-width") {
            let width: f64 = width.parse().unwrap();
            let back: f64 = attr(back, "stroke-width").unwrap().parse().unwrap();
            assert!((width * scale - back).abs() < 0.01);
        }
    }
}

#[test]
fn the_apps_own_pdfs_read_back_as_the_same_paths() {
    round_trip(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\" \
         viewBox=\"0 0 200 100\"><path fill=\"#1a2b3c\" d=\"M 10 10 L 50 10 C 60 20 70 30 80 10 Z\"/>\
         <path fill=\"#ffffff\" fill-rule=\"evenodd\" d=\"M 0 0 L 200 0 L 200 100 L 0 100 Z \
         M 20 20 L 40 20 L 40 40 Z\"/></svg>",
        0.75,
        &[(0., 0.); 2],
    );
    round_trip(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120pt\" height=\"60pt\" \
         viewBox=\"0 0 240 120\"><path fill=\"none\" stroke=\"#ff8000\" stroke-width=\"4\" \
         stroke-linejoin=\"round\" stroke-linecap=\"round\" d=\"M 10 10 C 20 40 60 40 100 10\"/>\
         <path fill=\"#00ff00\" stroke=\"#0000ff\" stroke-width=\"2\" \
         d=\"M 100 100 L 150 100 L 150 50 Z\"/></svg>",
        0.5,
        &[(0., 0.); 2],
    );
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100pt\" height=\"100pt\" \
         viewBox=\"0 0 100 100\"><g transform=\"translate(10 5)\"><path fill=\"#800000\" \
         d=\"M 0 0 L 10 0 L 10 10 Z\"/></g><g opacity=\"0.5\"><path fill=\"#008000\" \
         d=\"M 50 50 L 60 50 L 60 60 Z\"/></g><path fill=\"#000080\" fill-opacity=\"0.25\" \
         d=\"M 70 70 L 80 70 L 80 80 Z\"/></svg>";
    round_trip(svg, 1., &[(10., 5.), (0., 0.), (0., 0.)]);
    let back = to_svg(&crate::pdf_eps::to_pdf(svg).unwrap(), 0).unwrap();
    let opacities: Vec<_> = paths(&back.svg)
        .iter()
        .map(|p| attr(p, "fill-opacity"))
        .collect();
    assert_eq!(opacities, [None, Some("0.5"), Some("0.25")]);
}

#[test]
fn a_page_with_no_shapes_offers_its_largest_picture_with_its_mask() {
    // A 2 by 2 RGB image under a 2 by 2 soft mask, and a smaller indexed one.
    let rgb: Vec<u8> = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let big = deflated(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 \
         /ColorSpace /DeviceRGB /SMask 7 0 R",
        &rgb,
    );
    let small = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 \
         /ColorSpace [/Indexed /DeviceRGB 1 <000000ff8000>]",
        &[1],
    );
    let mask = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 \
         /ColorSpace /DeviceGray",
        &[255, 128, 0, 255],
    );
    let file = page_pdf(
        "/Resources << /XObject << /Im1 5 0 R /Im2 6 0 R >> >>",
        b"q 50 0 0 50 0 0 cm /Im2 Do Q q 100 0 0 100 0 0 cm /Im1 Do Q",
        &[big, small, mask],
    );
    let imported = to_svg(&file, 0).unwrap();
    assert!(paths(&imported.svg).is_empty());
    let raster = picture(&file, 0)
        .unwrap()
        .expect("the page draws a picture");
    assert_eq!((raster.width, raster.height), (2, 2));
    let px: Vec<[u8; 4]> = raster.pixels.iter().map(|p| p.0).collect();
    assert_eq!(
        px,
        vec![
            [255, 0, 0, 255],
            [0, 255, 0, 128],
            [0, 0, 255, 0],
            [255, 255, 255, 255]
        ]
    );
    // A page with no image offers none; an unreadable one says why.
    assert!(picture(&page_pdf("", b"0 0 1 rg 0 0 5 5 re f", &[]), 0)
        .unwrap()
        .is_none());
    let broken = stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /BitsPerComponent 8 \
         /ColorSpace /DeviceRGB",
        &[1, 2, 3],
    );
    let file = page_pdf(
        "/Resources << /XObject << /Im1 5 0 R >> >>",
        b"/Im1 Do",
        &[broken],
    );
    assert!(picture(&file, 0)
        .unwrap_err()
        .contains("shorter than its size"));
}
