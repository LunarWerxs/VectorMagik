//! Shape layers (`psd/vector.rs`) and the action descriptors they keep their
//! settings in (`psd/descriptor.rs`), in documents built byte by byte with
//! the builders of `psd.rs`'s tests, and in VectorMojo's sample.

use super::*;
use std::collections::HashSet;

/// A descriptor value as these tests write one.
enum D {
    Obj(&'static str, Vec<(&'static str, D)>),
    List(Vec<D>),
    Doub(f64),
    Unit(&'static [u8; 4], f64),
    Long(i32),
    Bool(bool),
    Enum(&'static str, &'static str),
    Text(&'static str),
}

/// A key: a four-character code after a zero length, or its length and
/// bytes.
fn key(out: &mut Vec<u8>, key: &str) {
    let n = if key.len() == 4 { 0 } else { key.len() as u32 };
    out.extend(n.to_be_bytes());
    out.extend(key.as_bytes());
}

fn unicode(out: &mut Vec<u8>, text: &str) {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    out.extend((units.len() as u32).to_be_bytes());
    out.extend(units.iter().flat_map(|unit| unit.to_be_bytes()));
}

fn descriptor(out: &mut Vec<u8>, class: &str, items: &[(&str, D)]) {
    unicode(out, "");
    key(out, class);
    out.extend((items.len() as u32).to_be_bytes());
    for (k, v) in items {
        key(out, k);
        value(out, v);
    }
}

/// A value: its type, then its data.
fn value(out: &mut Vec<u8>, v: &D) {
    match v {
        D::Obj(class, items) => {
            out.extend(b"Objc");
            descriptor(out, class, items);
        }
        D::List(items) => {
            out.extend(b"VlLs");
            out.extend((items.len() as u32).to_be_bytes());
            for item in items {
                value(out, item);
            }
        }
        D::Doub(v) => {
            out.extend(b"doub");
            out.extend(v.to_be_bytes());
        }
        D::Unit(unit, v) => {
            out.extend(b"UntF");
            out.extend(*unit);
            out.extend(v.to_be_bytes());
        }
        D::Long(v) => {
            out.extend(b"long");
            out.extend(v.to_be_bytes());
        }
        D::Bool(v) => out.extend([b'b', b'o', b'o', b'l', u8::from(*v)]),
        D::Enum(kind, v) => {
            out.extend(b"enum");
            key(out, kind);
            key(out, v);
        }
        D::Text(text) => {
            out.extend(b"TEXT");
            unicode(out, text);
        }
    }
}

/// A block holding a version (16) and a descriptor, after `prefix`.
fn versioned(name: &[u8; 4], prefix: &[u8], class: &str, items: &[(&str, D)]) -> Vec<u8> {
    let mut data = prefix.to_vec();
    data.extend(16u32.to_be_bytes());
    descriptor(&mut data, class, items);
    block(name, &data)
}

fn rgb(r: f64, g: f64, b: f64) -> D {
    D::Obj(
        "RGBC",
        vec![
            ("Rd  ", D::Doub(r)),
            ("Grn ", D::Doub(g)),
            ("Bl  ", D::Doub(b)),
        ],
    )
}

/// A solid colour fill.
fn soco(colour: D) -> Vec<u8> {
    versioned(b"SoCo", &[], "null", &[("Clr ", colour)])
}

/// A linear gradient fill through `stops`.
fn gradient(stops: Vec<D>) -> Vec<u8> {
    let stops = stops
        .into_iter()
        .enumerate()
        .map(|(i, colour)| {
            D::Obj(
                "Clrt",
                vec![
                    ("Clr ", colour),
                    ("Type", D::Enum("Clry", "UsrS")),
                    ("Lctn", D::Long(i as i32 * 4096)),
                    ("Mdpn", D::Long(50)),
                ],
            )
        })
        .collect();
    let grad = D::Obj(
        "Grdn",
        vec![
            ("Nm  ", D::Text("Two stops")),
            ("GrdF", D::Enum("GrdF", "CstS")),
            ("Intr", D::Doub(4096.)),
            ("Clrs", D::List(stops)),
            ("Trns", D::List(Vec::new())),
        ],
    );
    versioned(
        b"GdFl",
        &[],
        "null",
        &[
            ("Angl", D::Unit(b"#Ang", 90.)),
            ("Type", D::Enum("GrdT", "Lnr ")),
            ("Grad", grad),
        ],
    )
}

/// A stroke `width` pixels wide in `colour` at `percent` opacity, round at
/// its corners and ends, with the fill turned off.
fn stroke(width: f64, colour: D, percent: f64) -> Vec<u8> {
    versioned(
        b"vstk",
        &[],
        "strokeStyle",
        &[
            ("strokeStyleVersion", D::Long(2)),
            ("strokeEnabled", D::Bool(true)),
            ("fillEnabled", D::Bool(false)),
            ("strokeStyleLineWidth", D::Unit(b"#Pxl", width)),
            (
                "strokeStyleLineCapType",
                D::Enum("strokeStyleLineCapType", "strokeStyleRoundCap"),
            ),
            (
                "strokeStyleLineJoinType",
                D::Enum("strokeStyleLineJoinType", "strokeStyleRoundJoin"),
            ),
            (
                "strokeStyleLineAlignment",
                D::Enum("strokeStyleLineAlignment", "strokeStyleAlignCenter"),
            ),
            ("strokeStyleOpacity", D::Unit(b"#Prc", percent)),
            (
                "strokeStyleContent",
                D::Obj("solidColorLayer", vec![("Clr ", colour)]),
            ),
        ],
    )
}

/// A knot: the control point before it, its anchor and the control point
/// after it, each x then y in pixels.
type Knot = [[f64; 2]; 3];

/// A corner: a knot whose control points are its anchor.
fn corner(x: f64, y: f64) -> Knot {
    [[x, y]; 3]
}

/// A circle of four knots, clockwise from its top.
fn circle(cx: f64, cy: f64, r: f64) -> Vec<Knot> {
    let k = 0.552_284_75 * r;
    vec![
        [[cx - k, cy - r], [cx, cy - r], [cx + k, cy - r]],
        [[cx + r, cy - k], [cx + r, cy], [cx + r, cy + k]],
        [[cx + k, cy + r], [cx, cy + r], [cx - k, cy + r]],
        [[cx - r, cy + k], [cx - r, cy], [cx - r, cy - k]],
    ]
}

/// A subpath of a vector mask: whether it is closed, its operation, the
/// flags of its length record (2 nonzero, 1 even-odd) and its knots.
struct Sub {
    closed: bool,
    operation: i16,
    rule: u16,
    knots: Vec<Knot>,
}

fn closed(knots: Vec<Knot>) -> Sub {
    Sub {
        closed: true,
        operation: 1,
        rule: 2,
        knots,
    }
}

/// A vector mask with `flags` in a document `size` pixels.
fn vmsk(flags: u32, size: (f64, f64), subpaths: &[Sub]) -> Vec<u8> {
    let mut data = 3u32.to_be_bytes().to_vec();
    data.extend(flags.to_be_bytes());
    let record = |data: &mut Vec<u8>, selector: u16, body: &[u8]| {
        data.extend(selector.to_be_bytes());
        let mut body = body.to_vec();
        body.resize(24, 0);
        data.extend(body);
    };
    let fixed = |v: f64| ((v * 16_777_216.).round() as i32).to_be_bytes();
    record(&mut data, 6, &[]);
    record(&mut data, 8, &[0, 0]);
    for sub in subpaths {
        let mut body = (sub.knots.len() as u16).to_be_bytes().to_vec();
        body.extend(sub.operation.to_be_bytes());
        body.extend(sub.rule.to_be_bytes());
        record(&mut data, if sub.closed { 0 } else { 3 }, &body);
        for knot in &sub.knots {
            let body: Vec<u8> = knot
                .iter()
                .flat_map(|[x, y]| [fixed(y / size.1), fixed(x / size.0)])
                .flatten()
                .collect();
            record(&mut data, if sub.closed { 1 } else { 4 }, &body);
        }
    }
    block(b"vmsk", &data)
}

/// A layer made of `blocks` and no pixels.
fn shape(blocks: Vec<Vec<u8>>) -> TestLayer {
    let mut layer = layer([0, 0, 0, 0], Vec::new());
    layer.blocks = blocks.concat();
    layer
}

const SIZE: (f64, f64) = (100., 50.);

/// The layers of the test document, bottom first: a white background, a
/// green-to-blue gradient everywhere but a circle (an inverted even-odd
/// mask), a red rectangle at half its fill opacity with a drop shadow, a
/// blue line stroked 4 pixels wide at half opacity, a text layer, and two
/// shapes that do not show (one hidden, one in a hidden group).
fn layers() -> Vec<TestLayer> {
    let spot = circle(75., 25., 15.);
    let mut rectangle = shape(vec![
        soco(rgb(255., 0., 0.)),
        vmsk(
            0,
            SIZE,
            &[closed(vec![
                corner(10., 10.),
                corner(50., 10.),
                corner(50., 30.),
                corner(10., 30.),
            ])],
        ),
        block(b"iOpa", &[128, 0, 0, 0]),
        versioned(
            b"lfx2",
            &[0; 4],
            "null",
            &[
                ("Scl ", D::Unit(b"#Prc", 100.)),
                ("masterFXSwitch", D::Bool(true)),
                ("DrSh", D::Obj("DrSh", vec![("enab", D::Bool(true))])),
            ],
        ),
    ]);
    rectangle.rect = [10, 10, 30, 50];
    let line = Sub {
        closed: false,
        operation: 1,
        rule: 2,
        knots: vec![corner(10., 40.), corner(90., 40.)],
    };
    let mut text = solid([0, 95, 5, 100], [0, 0, 0]);
    text.blocks = block(b"TySh", &[0; 4]);
    let mut hidden = shape(vec![
        soco(rgb(18., 52., 86.)),
        vmsk(
            0,
            SIZE,
            &[closed(vec![
                corner(0., 0.),
                corner(100., 0.),
                corner(100., 50.),
            ])],
        ),
    ]);
    hidden.flags = 2;
    let grouped = shape(vec![
        soco(rgb(101., 67., 33.)),
        vmsk(
            0,
            SIZE,
            &[closed(vec![
                corner(0., 0.),
                corner(100., 0.),
                corner(100., 50.),
            ])],
        ),
    ]);
    vec![
        solid([0, 0, 50, 100], [255, 255, 255]),
        shape(vec![
            gradient(vec![rgb(0., 255., 0.), rgb(0., 0., 255.)]),
            vmsk(
                1,
                SIZE,
                &[Sub {
                    closed: true,
                    operation: 1,
                    rule: 1,
                    knots: spot,
                }],
            ),
        ]),
        rectangle,
        shape(vec![
            soco(rgb(255., 255., 255.)),
            vmsk(0, SIZE, &[line]),
            stroke(4., rgb(0., 0., 255.), 50.),
        ]),
        text,
        hidden,
        group(3, 0),
        grouped,
        group(1, 2),
    ]
}

/// The test document at 144 pixels an inch, with a white composite that the
/// version info says is real.
fn document() -> Vec<u8> {
    let mut psd = Psd::new(3, 8, 3, (100, 50));
    let mut resolution = (144u32 << 16).to_be_bytes().to_vec();
    resolution.extend([0, 1, 0, 1]);
    resolution.extend((144u32 << 16).to_be_bytes());
    resolution.extend([0, 1, 0, 1]);
    psd.resources = resource(1005, &resolution);
    psd.resources.extend(resource(1057, &[0, 0, 0, 1, 1]));
    psd.layers = layer_section(&layer_info(&layers()), false);
    let white = vec![255; 5000];
    psd.image = raw(&[&white, &white, &white]);
    psd.bytes()
}

#[test]
fn shape_layers_come_back_as_flat_paths_in_points() {
    let imported = crate::import::photoshop_shapes(&document())
        .unwrap()
        .unwrap();
    let svg = &imported.svg;
    // 100 by 50 pixels at 144 pixels an inch.
    assert!(svg.starts_with(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"50pt\" height=\"25pt\" viewBox=\"0 0 50 25\">\n"
    ));
    assert_eq!(svg.matches("<path").count(), 3, "{svg}");
    // The inverted circle: the page with the circle cut out, even-odd, in
    // the gradient's first colour.
    assert!(
        svg.contains("<path d=\"M0 0L50 0L50 25L0 25ZM37.5 5C"),
        "{svg}"
    );
    assert!(
        svg.contains("Z\" fill=\"#00ff00\" fill-rule=\"evenodd\"/>"),
        "{svg}"
    );
    // The rectangle at its fill opacity.
    assert!(
        svg.contains("<path d=\"M5 5L25 5L25 15L5 15Z\" fill=\"#ff0000\" fill-opacity=\"0.502\"/>"),
        "{svg}"
    );
    // The line: 4 pixels are 2 points.
    assert!(
        svg.contains(
            "<path d=\"M5 20L45 20\" fill=\"none\" stroke=\"#0000ff\" stroke-width=\"2\" \
             stroke-linejoin=\"round\" stroke-linecap=\"round\" stroke-opacity=\"0.5\"/>"
        ),
        "{svg}"
    );
    assert!(!svg.contains("#123456") && !svg.contains("#654321"));
    assert_eq!(
        imported.skipped,
        [
            "1 raster layer",
            "text layers",
            "layer effects",
            "gradients drawn in one colour"
        ]
    );
    assert_eq!(imported.pages, 1);
    // The flat paths are what the app writes as PDF.
    assert!(crate::pdf_eps::to_pdf(svg).is_ok());
}

#[test]
fn the_composite_draws_shape_layers_in_their_place() {
    let raster = decode(&without_composite((100, 50), &layers()), 10_000).unwrap();
    let at = |x: usize, y: usize| raster.pixels[y * 100 + x].0;
    let (white, green) = ([255; 4], [0, 255, 0, 255]);
    // The inverted circle leaves the background inside it; the hidden
    // shapes do not show.
    assert_eq!(at(75, 25), white);
    assert_eq!(at(2, 2), green);
    assert_eq!(at(97, 47), green);
    // The red rectangle at half its fill over the green.
    let [r, g, b, a] = at(20, 20);
    assert!(r.abs_diff(128) <= 1 && g.abs_diff(127) <= 1 && b == 0 && a == 255);
    // The blue line at half opacity, round past its ends.
    let [r, g, b, _] = at(50, 40);
    assert!(r == 0 && g.abs_diff(128) <= 1 && b.abs_diff(127) <= 1);
    assert_ne!(at(91, 40), green);
    assert_eq!(at(94, 40), green);
    assert_eq!(at(50, 43), green);
    // The circle's edge is anti-aliased.
    let edge: Vec<[u8; 4]> = (55..65).map(|x| at(x, 25)).collect();
    assert!(edge.iter().any(|p| p[0] > 0 && p[0] < 255), "{edge:?}");
}

#[test]
fn documents_without_shape_layers_have_none() {
    let layers = [solid([0, 0, 2, 2], [1, 2, 3])];
    assert_eq!(shapes(&without_composite((2, 2), &layers)), Ok(None));
    let mut psd = Psd::new(3, 8, 3, (1, 1));
    psd.image = raw(&[&[1], &[2], &[3]]);
    assert_eq!(shapes(&psd.bytes()), Ok(None));
    // Only hidden shapes, and a fill layer without an outline, are none too.
    let mut hidden = shape(vec![
        soco(rgb(1., 2., 3.)),
        vmsk(
            0,
            (2., 2.),
            &[closed(vec![corner(0., 0.), corner(2., 0.), corner(2., 2.)])],
        ),
    ]);
    hidden.flags = 2;
    let fill = shape(vec![soco(rgb(9., 9., 9.))]);
    assert_eq!(
        shapes(&without_composite((2, 2), &[hidden, fill])),
        Ok(None)
    );
    assert!(shapes(b"8BPS").is_err());
}

/// A 20 by 10 document of one red rectangle whose fill is `fill`.
fn one_shape(fill: Vec<u8>) -> Vec<u8> {
    let outline = vmsk(
        0,
        (20., 10.),
        &[closed(vec![
            corner(2., 2.),
            corner(18., 2.),
            corner(18., 8.),
            corner(2., 8.),
        ])],
    );
    without_composite((20, 10), &[shape(vec![fill, outline])])
}

#[test]
fn hostile_settings_are_refused() {
    let good = one_shape(soco(rgb(255., 0., 0.)));
    assert!(shapes(&good).unwrap().is_some());
    assert_eq!(
        decode(&good, 1000).unwrap().pixels[5 * 20 + 10].0,
        [255, 0, 0, 255]
    );
    let mut refused = Vec::new();
    // An unknown value type.
    let mut data = 16u32.to_be_bytes().to_vec();
    descriptor(&mut data, "null", &[]);
    data.truncate(data.len() - 4);
    data.extend(1u32.to_be_bytes());
    key(&mut data, "Clr ");
    data.extend(b"XXXX");
    refused.push(block(b"SoCo", &data));
    // Nested far deeper than Photoshop nests.
    let mut deep = rgb(1., 2., 3.);
    for _ in 0..40 {
        deep = D::Obj("null", vec![("Clr ", deep)]);
    }
    refused.push(soco(deep));
    // Four billion items, and a list as long.
    let mut data = 16u32.to_be_bytes().to_vec();
    descriptor(&mut data, "null", &[]);
    data.truncate(data.len() - 4);
    data.extend(u32::MAX.to_be_bytes());
    refused.push(block(b"SoCo", &data));
    let mut data = 16u32.to_be_bytes().to_vec();
    descriptor(&mut data, "null", &[("Clr ", D::List(Vec::new()))]);
    let at = data.len() - 4;
    data[at..].copy_from_slice(&u32::MAX.to_be_bytes());
    refused.push(block(b"SoCo", &data));
    // A key and a string longer than the data, and another version.
    let mut data = 16u32.to_be_bytes().to_vec();
    data.extend(0x7FFF_FFFFu32.to_be_bytes());
    refused.push(block(b"SoCo", &data));
    let mut data = soco(rgb(1., 2., 3.));
    data[15] = 15;
    refused.push(data);
    for fill in refused {
        let bytes = one_shape(fill);
        let error = shapes(&bytes).unwrap_err();
        assert!(
            error.contains("damaged") || error.contains("cut short"),
            "{error}"
        );
        // The picture draws the layer as a fill layer it cannot read:
        // nothing.
        let raster = decode(&bytes, 1000).unwrap();
        assert!(raster.pixels.iter().all(|p| p.0[3] == 0));
    }
    // Vector masks: a knot before any subpath, and an unknown record.
    for selector in [1u16, 9] {
        let mut data = 3u32.to_be_bytes().to_vec();
        data.extend([0; 4]);
        data.extend(selector.to_be_bytes());
        data.extend([0; 24]);
        let bytes = without_composite(
            (20, 10),
            &[shape(vec![soco(rgb(1., 2., 3.)), block(b"vmsk", &data)])],
        );
        assert!(shapes(&bytes).unwrap_err().contains("vector mask"));
    }
    // An outline far too detailed to draw: thousands of knots whose control
    // points lie a hundred documents away.
    let knots: Vec<Knot> = (0..45_000)
        .map(|i| {
            let x = f64::from(i % 20);
            [[x, -1000.], [x, 5.], [x, 1000.]]
        })
        .collect();
    let bytes = one_shape_with(vmsk(0, (20., 10.), &[closed(knots)]));
    assert!(shapes(&bytes).unwrap().is_some());
    let error = decode(&bytes, 1000).unwrap_err();
    assert!(error.contains("too detailed"), "{error}");
}

/// A 20 by 10 document of one red shape with `outline`.
fn one_shape_with(outline: Vec<u8>) -> Vec<u8> {
    without_composite((20, 10), &[shape(vec![soco(rgb(255., 0., 0.)), outline])])
}

#[test]
fn damaged_shape_layers_never_panic() {
    let mut line = shape(vec![
        soco(rgb(1., 2., 3.)),
        vmsk(
            1,
            (20., 10.),
            &[Sub {
                closed: false,
                operation: 2,
                rule: 1,
                knots: circle(10., 5., 4.),
            }],
        ),
        stroke(3., rgb(4., 5., 6.), 80.),
    ]);
    line.blocks.extend(gradient(vec![rgb(7., 8., 9.)]));
    let bytes = without_composite((20, 10), &[line]);
    assert!(shapes(&bytes).unwrap().is_some());
    for at in 0..bytes.len() {
        for value in [0, 1, 0x7F, 0x80, 0xFE, 0xFF] {
            let mut damaged = bytes.clone();
            damaged[at] = value;
            let _ = shapes(&damaged);
            let _ = decode(&damaged, 1 << 12);
        }
    }
}

/// VectorMojo's demo document: its eight shape layers come back as paths,
/// and its picture is them rather than its composite of one navy.
#[test]
fn the_vector_mojo_sample_gives_its_shapes() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/samples/vector-mojo-sample.psd");
    let bytes = std::fs::read(path).unwrap();
    let imported = shapes(&bytes).unwrap().unwrap();
    let svg = &imported.svg;
    // 1200 by 800 pixels without a resolution: 0.75 points a pixel.
    assert!(svg.starts_with(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"900pt\" height=\"600pt\" viewBox=\"0 0 900 600\">\n"
    ));
    let paint = |attribute: &str| -> Vec<String> {
        svg.lines()
            .filter(|line| line.starts_with("<path"))
            .map(|line| {
                line.split(&format!(" {attribute}=\""))
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .unwrap_or("-")
                    .to_owned()
            })
            .collect()
    };
    let (fills, strokes) = (paint("fill"), paint("stroke"));
    eprintln!(
        "fills {fills:?}\nstrokes {strokes:?}\nskipped {:?}",
        imported.skipped
    );
    // Bottom first: the canvas, the ribbon, the glow and the spark in their
    // gradients' first colours, the orbit, the route (a gradient stroke
    // alone) and the two nodes.
    assert_eq!(
        fills,
        ["#111827", "#7c3aed", "#67e8f9", "#e0f2fe", "none", "#fbbf24", "#67e8f9", "#fde68a"]
    );
    assert_eq!(strokes, ["-", "-", "-", "-", "#fbbf24", "-", "-", "-"]);
    // The route's 28 pixels are 21 points.
    assert!(svg.contains(
        "fill=\"none\" stroke=\"#fbbf24\" stroke-width=\"21\" stroke-linejoin=\"round\" \
         stroke-linecap=\"round\"/>"
    ));
    // The orbit at half opacity.
    assert!(svg.contains("fill=\"#e0f2fe\" fill-rule=\"evenodd\" fill-opacity=\"0.502\""));
    // The orbit is a ring: a subtracted circle, filled even-odd.
    assert_eq!(svg.matches("fill-rule=\"evenodd\"").count(), 1);
    assert_eq!(
        imported.skipped,
        [
            "gradients drawn in one colour",
            "blend modes drawn as normal"
        ]
    );
    let raster = decode(&bytes, 16_000_000).unwrap();
    let colours: HashSet<[u8; 4]> = raster.pixels.iter().map(|p| p.0).collect();
    eprintln!("{} colours in the composite", colours.len());
    assert!(colours.len() > 100, "{} colours", colours.len());
}
