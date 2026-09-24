use super::*;

/// The records of an EMF as their type and the bytes after their size,
/// each checked to lie whole inside the file on a 4-byte boundary.
fn records(emf: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < emf.len() {
        let word = |i: usize| u32::from_le_bytes(emf[i..i + 4].try_into().unwrap());
        let (kind, size) = (word(at), word(at + 4) as usize);
        assert!(
            size >= 8 && size % 4 == 0 && at + size <= emf.len(),
            "record {kind} at {at}"
        );
        out.push((kind, emf[at + 8..at + size].to_vec()));
        at += size;
    }
    out
}

fn words(bytes: &[u8]) -> Vec<i64> {
    bytes
        .chunks(4)
        .map(|c| i64::from(i32::from_le_bytes(c.try_into().unwrap())))
        .collect()
}

/// A poly record's points after its bounds and count, 16 or 32-bit.
fn poly_points(data: &[u8], wide: bool) -> Vec<[i64; 2]> {
    let count = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;
    let values: Vec<i64> = if wide {
        words(&data[20..])
    } else {
        data[20..]
            .chunks(2)
            .map(|c| i64::from(i16::from_le_bytes(c.try_into().unwrap())))
            .collect()
    };
    assert_eq!(values.len(), 2 * count);
    values.chunks(2).map(|p| [p[0], p[1]]).collect()
}

/// 100 x 50 pt over a 200 x 100 view box: 8 logical units per unit.
const SVG: &str = "<svg width=\"100pt\" height=\"50pt\" viewBox=\"0 0 200 100\"><g id=\"#ff0000ff\"><path fill=\"#ff0000\" d=\"M 20 50 C 20 10 80 10 80 50 L 20 50 Z M 40 40 L 60 40 L 60 45 Z\" /><path fill=\"#ff0000\" fill-rule=\"evenodd\" d=\"M 100 10 L 190 10 L 190 90 Z\" /></g><path fill=\"none\" stroke=\"#0000ff\" stroke-width=\"4\" stroke-linejoin=\"round\" d=\"M 10 90 L 190 90\" /></svg>";

#[test]
fn every_path_is_one_gdi_path_in_sixteenths_of_a_point() {
    let emf = to_emf(SVG).unwrap();
    let records = records(&emf);
    let header = words(&records[0].1);
    assert_eq!(records[0].0, EMR_HEADER);
    // Bounds in device units and the frame in 0.01 mm, both inclusive.
    assert_eq!(header[..8], [0, 0, 1599, 799, 0, 0, 3527, 1763]);
    assert_eq!(header[8] as u32, ENHMETA_SIGNATURE);
    assert_eq!(header[10] as usize, emf.len());
    assert_eq!(header[11] as usize, records.len());
    // Device 23040 x 17280 px in 508 x 381 mm: 1152 dpi, 16 per point.
    assert_eq!(header[16..20], [23040, 17280, 508, 381]);
    let kinds: Vec<u32> = records.iter().map(|r| r.0).collect();
    assert_eq!(
        kinds,
        [
            EMR_HEADER,
            EMR_SETMAPMODE,
            EMR_SETWINDOWORGEX,
            EMR_SETWINDOWEXTEX,
            EMR_SETVIEWPORTORGEX,
            EMR_SETVIEWPORTEXTEX,
            EMR_SETBKMODE,
            EMR_SETMITERLIMIT,
            // The red outline with its notch, nonzero.
            EMR_SETPOLYFILLMODE,
            EMR_CREATEBRUSHINDIRECT,
            EMR_SELECTOBJECT,
            EMR_SELECTOBJECT,
            EMR_BEGINPATH,
            EMR_MOVETOEX,
            EMR_POLYBEZIERTO16,
            EMR_LINETO,
            EMR_CLOSEFIGURE,
            EMR_MOVETOEX,
            EMR_POLYLINETO16,
            EMR_CLOSEFIGURE,
            EMR_ENDPATH,
            EMR_FILLPATH,
            // The red triangle, evenodd, with the same brush.
            EMR_SETPOLYFILLMODE,
            EMR_BEGINPATH,
            EMR_MOVETOEX,
            EMR_POLYLINETO16,
            EMR_CLOSEFIGURE,
            EMR_ENDPATH,
            EMR_FILLPATH,
            // The blue line, stroked only.
            EMR_SELECTOBJECT,
            EMR_EXTCREATEPEN,
            EMR_SELECTOBJECT,
            EMR_BEGINPATH,
            EMR_MOVETOEX,
            EMR_LINETO,
            EMR_ENDPATH,
            EMR_STROKEPATH,
            EMR_SELECTOBJECT,
            EMR_SELECTOBJECT,
            EMR_DELETEOBJECT,
            EMR_DELETEOBJECT,
            EMR_EOF,
        ]
    );
    let data = |i: usize| words(&records[i].1);
    assert_eq!(data(1), [8]);
    assert_eq!(data(3), [1600, 800]);
    assert_eq!(data(5), [1600, 800]);
    assert_eq!(data(8), [2]);
    // Brush 1, solid, red as 0x0000ff.
    assert_eq!(data(9), [1, 0, 0xff, 0]);
    assert_eq!(data(11)[0] as u32, NULL_PEN);
    assert_eq!(data(13), [160, 400]);
    assert_eq!(
        poly_points(&records[14].1, false),
        [[160, 80], [640, 80], [640, 400]]
    );
    assert_eq!(data(14)[..4], [160, 80, 640, 400]);
    assert_eq!(data(15), [160, 400]);
    assert_eq!(poly_points(&records[18].1, false), [[480, 320], [480, 360]]);
    assert_eq!(data(21), [160, 80, 640, 400]);
    assert_eq!(data(22), [1]);
    assert_eq!(data(29)[0] as u32, NULL_BRUSH);
    // Pen 3: geometric, round join, flat cap, 2 pt wide, blue.
    assert_eq!(
        data(30),
        [3, 0, 0, 0, 0, 0x0001_0200, 32, 0, 0x00ff_0000, 0, 0]
    );
    // The stroke's bounds reach half its width past the line.
    assert_eq!(data(36), [64, 704, 1536, 736]);
    assert_eq!(data(39), [1]);
    assert_eq!(data(40), [3]);
    assert_eq!(data(41), [0, 16, 20]);
}

#[test]
fn objects_are_made_selected_and_deleted_in_order_on_a_frozen_document() {
    let svg = include_str!("../../../fixtures/reference/logo-with-blending-small-high.svg");
    let emf = to_emf(svg).unwrap();
    let records = records(&emf);
    let handles = u16::from_le_bytes(records[0].1[48..50].try_into().unwrap()) as i64;
    let (mut made, mut selected_brush, mut selected_pen) = (Vec::new(), None, None);
    for (kind, data) in &records {
        let data = words(data);
        match *kind {
            EMR_CREATEBRUSHINDIRECT | EMR_EXTCREATEPEN => {
                assert!(data[0] > 0 && data[0] < handles && !made.contains(&data[0]));
                made.push(data[0]);
            }
            EMR_SELECTOBJECT if data[0] >= 0 => {
                assert!(made.contains(&data[0]));
                if data[0] <= 2 {
                    selected_brush = Some(data[0]);
                } else {
                    selected_pen = Some(data[0]);
                }
            }
            EMR_SELECTOBJECT => {
                if [WHITE_BRUSH, NULL_BRUSH].contains(&(data[0] as u32)) {
                    selected_brush = None;
                } else {
                    selected_pen = None;
                }
            }
            EMR_DELETEOBJECT => {
                assert!(Some(data[0]) != selected_brush && Some(data[0]) != selected_pen);
                made.retain(|h| *h != data[0]);
            }
            _ => {}
        }
    }
    assert!(made.is_empty(), "{made:?} left undeleted");
    // One brush per colour group.
    let brushes = records
        .iter()
        .filter(|r| r.0 == EMR_CREATEBRUSHINDIRECT)
        .count();
    assert_eq!(brushes, svg.matches("<g id=").count());
    assert_eq!(
        records.iter().filter(|r| r.0 == EMR_BEGINPATH).count(),
        svg.matches("<path").count()
    );
}

#[test]
fn coordinates_beyond_sixteen_bits_take_the_wide_records() {
    let svg = "<svg width=\"3000pt\" height=\"10pt\" viewBox=\"0 0 3000 10\"><path fill=\"#000000\" d=\"M 0 0 L 2500 0 L 2500 5 C 2600 5 2900 10 2990 10 Z\" /></svg>";
    let records = records(&to_emf(svg).unwrap());
    let find = |kind: u32| records.iter().find(|r| r.0 == kind).unwrap();
    assert_eq!(
        poly_points(&find(EMR_POLYLINETO).1, true),
        [[40000, 0], [40000, 80]]
    );
    assert_eq!(
        poly_points(&find(EMR_POLYBEZIERTO).1, true),
        [[41600, 80], [46400, 160], [47840, 160]]
    );
    assert!(!records
        .iter()
        .any(|r| r.0 == EMR_POLYLINETO16 || r.0 == EMR_POLYBEZIERTO16));
}

#[test]
fn transparency_and_unreachable_coordinates_are_refused() {
    let translucent = SVG.replace("fill=\"#ff0000\" d=", "fill=\"#ff0000\" opacity=\"0.5\" d=");
    assert!(to_emf(&translucent).unwrap_err().contains("transparency"));
    let far = "<svg width=\"10pt\" height=\"10pt\"><path fill=\"#000000\" d=\"M 0 0 L 1e30 0 L 0 1 Z\" /></svg>";
    assert!(to_emf(far).unwrap_err().contains("too far"));
    for svg in [
        "",
        "<svg",
        "<svg width=\"1\" height=\"1\"><path d=\"M 0 0 A 1 1 0 0 0 1 1\" /></svg>",
    ] {
        assert!(to_emf(svg).is_err(), "{svg}");
    }
    // An empty page is still a whole metafile.
    let empty = to_emf("<svg width=\"10pt\" height=\"10pt\"></svg>").unwrap();
    assert_eq!(records(&empty).last().unwrap().0, EMR_EOF);
}

/// Writes `NAME.svg`, `NAME.emf` and `NAME.png` (resvg's rendering at the
/// declared size, 96 per inch) of four frozen references into the folder
/// `EMF_CHECK_DIR` names (`work/emf-check` without it), for
/// `kit/tools/emf_check.py`, which draws each EMF with GDI+ and compares.
#[cfg(feature = "render")]
#[test]
#[ignore = "writes files for kit/tools/emf_check.py"]
fn write_emf_check_pairs() {
    use crate::export::{vector_bytes, ExportOptions, OutputKind};
    let dir = std::env::var_os("EMF_CHECK_DIR").map_or_else(
        || std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/emf-check"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let references = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/reference/");
    for name in [
        "logo-with-blending-small-high",
        "logo-without-blending-high",
        "coffee-low",
        "astronaut-low",
    ] {
        let svg = std::fs::read_to_string(format!("{references}{name}.svg")).unwrap();
        std::fs::write(dir.join(format!("{name}.svg")), &svg).unwrap();
        std::fs::write(dir.join(format!("{name}.emf")), to_emf(&svg).unwrap()).unwrap();
        let png = vector_bytes(OutputKind::Png, &svg, &ExportOptions::default()).unwrap();
        std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
        // The same drawing aliased at four times the size: GDI+ plays EMF
        // records aliased, and resvg's antialiasing leaves a seam of the
        // background along every edge two shapes share, which aliased
        // rendering does not; box-filtered alike, the two compare like for
        // like.
        let crisp = svg.replacen("<svg ", "<svg shape-rendering=\"crispEdges\" ", 1);
        let tree = resvg::usvg::Tree::from_str(&crisp, &resvg::usvg::Options::default()).unwrap();
        let size = tree.size().to_int_size();
        let (width, height) = (size.width() * 4, size.height() * 4);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).unwrap();
        let transform = resvg::tiny_skia::Transform::from_scale(4., 4.);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        let rgba = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        image::RgbaImage::from_raw(width, height, rgba)
            .unwrap()
            .save(dir.join(format!("{name}.aliased4.png")))
            .unwrap();
    }
}
