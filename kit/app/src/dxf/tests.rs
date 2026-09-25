use super::*;

type Pairs = Vec<(u16, String)>;

/// A DXF read back as group code and value pairs, its layout checked: CR
/// LF lines, a number on every code line, the file's last line ended.
fn pairs(dxf: &[u8]) -> Pairs {
    let text = std::str::from_utf8(dxf).unwrap();
    let lines: Vec<&str> = text.split("\r\n").collect();
    assert_eq!(lines.last(), Some(&""));
    let lines = &lines[..lines.len() - 1];
    assert_eq!(lines.len() % 2, 0);
    lines
        .chunks(2)
        .map(|pair| (pair[0].trim().parse().unwrap(), pair[1].to_owned()))
        .collect()
}

/// The entities section as each entity's type and its pairs.
fn entities(pairs: &Pairs) -> Vec<(String, Pairs)> {
    let start = pairs
        .iter()
        .position(|p| p.0 == 2 && p.1 == "ENTITIES")
        .unwrap()
        + 1;
    let length = pairs[start..]
        .iter()
        .position(|p| p.0 == 0 && p.1 == "ENDSEC")
        .unwrap();
    let mut out: Vec<(String, Pairs)> = Vec::new();
    for (code, value) in &pairs[start..start + length] {
        match code {
            0 => out.push((value.clone(), Vec::new())),
            _ => out.last_mut().unwrap().1.push((*code, value.clone())),
        }
    }
    out
}

fn value(pairs: &Pairs, code: u16) -> &str {
    &pairs.iter().find(|p| p.0 == code).unwrap().1
}

fn numbers(pairs: &Pairs, code: u16) -> Vec<f64> {
    pairs
        .iter()
        .filter(|p| p.0 == code)
        .map(|p| p.1.parse().unwrap())
        .collect()
}

fn points(pairs: &Pairs) -> Vec<Xy> {
    numbers(pairs, 10)
        .into_iter()
        .zip(numbers(pairs, 20))
        .collect()
}

/// Each polyline of an R12 file: its own pairs and its vertices.
fn polylines(entities: &[(String, Pairs)]) -> Vec<(Pairs, Vec<Xy>)> {
    let mut out: Vec<(Pairs, Vec<Xy>)> = Vec::new();
    for (kind, pairs) in entities {
        match kind.as_str() {
            "POLYLINE" => out.push((pairs.clone(), Vec::new())),
            "VERTEX" => out.last_mut().unwrap().1.extend(points(pairs)),
            _ => assert_eq!(kind, "SEQEND"),
        }
    }
    out
}

fn distance_to_segment(p: Xy, a: Xy, b: Xy) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0. {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0., 1.)
    } else {
        0.
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// The distance from `p` to a closed polyline.
fn distance_to_ring(p: Xy, ring: &[Xy]) -> f64 {
    (0..ring.len())
        .map(|i| distance_to_segment(p, ring[i], ring[(i + 1) % ring.len()]))
        .fold(f64::INFINITY, f64::min)
}

/// 100 x 50 pt: a red lens of two cubics (open in the data, closed by its
/// fill) with a square hole, a blue triangle, an open green stroke.
const SVG: &str = "<svg width=\"100pt\" height=\"50pt\" viewBox=\"0 0 100 50\"><g id=\"#ff0000ff\"><path fill=\"#ff0000\" d=\" M 10 25 C 10 5 40 5 40 25 C 40 45 10 45 10 25 M 20 20 L 30 20 L 30 30 L 20 30 L 20 20 Z\" /></g><path fill=\"#0000ff\" d=\"M 50 10 L 90 10 L 70 40 Z\" /><path fill=\"none\" stroke=\"#00aa00\" stroke-width=\"2\" d=\"M 50 45 C 60 35 80 55 95 45\" /></svg>";

/// The lens's two cubics, y up.
const LENS: [[Xy; 4]; 2] = [
    [(10., 25.), (10., 45.), (40., 45.), (40., 25.)],
    [(40., 25.), (40., 5.), (10., 5.), (10., 25.)],
];

#[test]
fn the_line_modes_stay_within_their_tolerance_of_every_curve() {
    let mut vertices = Vec::new();
    for (mode, tolerance) in [
        (DxfMode::FineLines, FINE_TOLERANCE),
        (DxfMode::CoarseLines, COARSE_TOLERANCE),
    ] {
        let pairs = pairs(&to_dxf(SVG, mode).unwrap());
        assert_eq!(
            pairs[2..4],
            [(9, "$ACADVER".to_owned()), (1, "AC1009".to_owned())]
        );
        assert_eq!(pairs.last(), Some(&(0, "EOF".to_owned())));
        let polylines = polylines(&entities(&pairs));
        assert_eq!(polylines.len(), 4);
        let (head, lens) = &polylines[0];
        assert_eq!(
            [value(head, 8), value(head, 62), value(head, 70)],
            ["COLOR_FF0000", "1", "1"]
        );
        // R12 has no true colour code, and Illustrator's AutoCAD import
        // refuses a file that carries one.
        assert!(pairs.iter().all(|p| p.0 != 420));
        // Every point of the curves within the tolerance of the polyline,
        // and every vertex on a curve.
        for [p0, p1, p2, p3] in LENS {
            for i in 0..=2000 {
                let p = cubic_at(p0, p1, p2, p3, f64::from(i) / 2000.);
                let off = distance_to_ring(p, lens);
                assert!(off <= tolerance + 1e-6, "{mode:?}: {p:?} is {off} off");
            }
        }
        let dense: Vec<Xy> = LENS
            .iter()
            .flat_map(|[p0, p1, p2, p3]| {
                (0..4000).map(move |i| cubic_at(*p0, *p1, *p2, *p3, f64::from(i) / 4000.))
            })
            .collect();
        for v in lens {
            assert!(distance_to_ring(*v, &dense) < 1e-4, "{v:?}");
        }
        vertices.push(lens.len());
        // The hole's corners, its repeated first corner left to the flag.
        assert_eq!(
            polylines[1].1,
            [(20., 30.), (30., 30.), (30., 20.), (20., 20.)]
        );
        assert_eq!(value(&polylines[1].0, 70), "1");
        assert_eq!(polylines[2].1, [(50., 40.), (90., 40.), (70., 10.)]);
        assert_eq!(value(&polylines[2].0, 8), "COLOR_0000FF");
        // The stroke is open.
        assert_eq!(value(&polylines[3].0, 70), "0");
        assert_eq!(value(&polylines[3].0, 8), "COLOR_00AA00");
        assert_eq!(polylines[3].1.first(), Some(&(50., 5.)));
        assert_eq!(polylines[3].1.last(), Some(&(95., 5.)));
        // One layer per colour after layer 0.
        let layers: Vec<&str> = pairs
            .windows(2)
            .filter(|w| w[0] == (0, "LAYER".to_owned()))
            .map(|w| w[1].1.as_str())
            .collect();
        assert_eq!(
            layers,
            ["0", "COLOR_FF0000", "COLOR_0000FF", "COLOR_00AA00"]
        );
    }
    assert!(vertices[0] > 2 * vertices[1], "{vertices:?}");
}

#[test]
fn the_spline_mode_keeps_every_cubic_and_every_handle_resolves() {
    let pairs = pairs(&to_dxf(SVG, DxfMode::Splines).unwrap());
    assert_eq!(
        pairs[2..4],
        [(9, "$ACADVER".to_owned()), (1, "AC1015".to_owned())]
    );
    let entities = entities(&pairs);
    let kinds: Vec<&str> = entities.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(kinds, ["SPLINE", "LWPOLYLINE", "LWPOLYLINE", "SPLINE"]);
    let lens = &entities[0].1;
    assert_eq!(
        [
            value(lens, 70),
            value(lens, 71),
            value(lens, 72),
            value(lens, 73)
        ],
        ["8", "3", "11", "7"]
    );
    assert_eq!(
        numbers(lens, 40),
        [0., 0., 0., 0., 1., 1., 1., 2., 2., 2., 2.]
    );
    let expected: Vec<Xy> = [&LENS[0][..], &LENS[1][1..]].concat();
    assert_eq!(points(lens), expected);
    assert_eq!(value(lens, 420), "16711680");
    let hole = &entities[1].1;
    assert_eq!([value(hole, 90), value(hole, 70)], ["4", "1"]);
    assert_eq!(
        points(hole),
        [(20., 30.), (30., 30.), (30., 20.), (20., 20.)]
    );
    let stroke = &entities[3].1;
    assert_eq!(
        points(stroke),
        [(50., 5.), (60., 15.), (80., -5.), (95., 5.)]
    );
    // Every handle unique and below the seed; every owner and pointer a
    // handle (or 0, the file itself).
    let seed = pairs
        .iter()
        .position(|p| p.1 == "$HANDSEED")
        .map(|i| u32::from_str_radix(&pairs[i + 1].1, 16).unwrap())
        .unwrap();
    let body = &pairs[pairs.iter().position(|p| p.1 == "ENDSEC").unwrap()..];
    let handles: Vec<u32> = body
        .iter()
        .filter(|p| p.0 == 5 || p.0 == 105)
        .map(|p| u32::from_str_radix(&p.1, 16).unwrap())
        .collect();
    let mut unique = handles.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), handles.len());
    assert!(handles.iter().all(|h| *h < seed));
    for (code, pointer) in body.iter().filter(|p| [330, 340, 350, 390].contains(&p.0)) {
        let pointer = u32::from_str_radix(pointer, 16).unwrap();
        assert!(
            (*code == 330 && pointer == 0) || handles.contains(&pointer),
            "{code} {pointer:X}"
        );
    }
    // Every layer's plot style is the Normal placeholder: without one
    // Illustrator's AutoCAD import refuses the file.
    let placeholder = pairs
        .iter()
        .position(|p| *p == (0, "ACDBPLACEHOLDER".to_owned()))
        .map(|i| pairs[i + 1].1.as_str())
        .unwrap();
    let layers: Vec<&[(u16, String)]> = pairs
        .iter()
        .enumerate()
        .filter(|(_, p)| **p == (0, "LAYER".to_owned()))
        .map(|(i, _)| {
            let end = pairs[i + 1..].iter().position(|p| p.0 == 0).unwrap() + i + 1;
            &pairs[i..end]
        })
        .collect();
    assert_eq!(layers.len(), 4);
    for layer in layers {
        assert_eq!(value(&layer.to_vec(), 390), placeholder);
    }
    let sections: Vec<&str> = pairs
        .windows(2)
        .filter(|w| w[0] == (0, "SECTION".to_owned()))
        .map(|w| w[1].1.as_str())
        .collect();
    assert_eq!(
        sections,
        ["HEADER", "CLASSES", "TABLES", "BLOCKS", "ENTITIES", "OBJECTS"]
    );
}

#[test]
fn a_mixed_outline_is_splines_and_lines_end_to_end_and_closed() {
    let svg = "<svg width=\"20pt\" height=\"10pt\" viewBox=\"0 0 20 10\"><path fill=\"#808080\" d=\"M 0 0 L 10 0 C 20 0 20 10 10 10 L 0 10\" /></svg>";
    let pairs = pairs(&to_dxf(svg, DxfMode::Splines).unwrap());
    let entities = entities(&pairs);
    let kinds: Vec<&str> = entities.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(kinds, ["LWPOLYLINE", "SPLINE", "LWPOLYLINE"]);
    let runs: Vec<Vec<Xy>> = entities.iter().map(|e| points(&e.1)).collect();
    assert_eq!(runs[0], [(0., 10.), (10., 10.)]);
    assert_eq!(runs[1], [(10., 10.), (20., 10.), (20., 0.), (10., 0.)]);
    // The fill closes the outline: its closing edge ends the last run.
    assert_eq!(runs[2], [(10., 0.), (0., 0.), (0., 10.)]);
    assert!(entities.iter().all(|e| value(&e.1, 62) == "8"));
}

#[test]
fn colours_take_their_nearest_autocad_index() {
    // AutoCAD's palette as Illustrator's AutoCAD import reads it.
    assert_eq!(index_colour(13), [204, 102, 102]);
    assert_eq!(index_colour(23), [204, 127, 102]);
    assert_eq!(index_colour(92), [0, 204, 0]);
    assert_eq!(index_colour(240), [255, 0, 63]);
    assert_eq!(index_colour(251), [91, 91, 91]);
    for (colour, index) in [
        ([255, 0, 0], 1),
        ([0, 0, 0], 7),
        ([255, 255, 255], 7),
        ([128, 128, 128], 8),
        ([204, 102, 102], 13),
        ([0, 170, 0], 94),
    ] {
        assert_eq!(nearest_index(colour), index, "{colour:?}");
    }
}

#[test]
fn hostile_and_empty_drawings_are_refused_or_written_whole() {
    let huge = "<svg width=\"10\" height=\"10\"><path fill=\"#000000\" d=\"M 0 0 C 1e300 0 0 1e300 1 1 Z\" /></svg>";
    for mode in [DxfMode::FineLines, DxfMode::CoarseLines] {
        assert!(to_dxf(huge, mode).unwrap_err().contains("too large"));
    }
    assert!(to_dxf("<svg><path d=\"M 0 0\"", DxfMode::Splines).is_err());
    assert!(to_dxf(
        "<svg width=\"1\" height=\"1\"><image href=\"a.png\" /></svg>",
        DxfMode::FineLines
    )
    .is_err());
    for mode in [DxfMode::Splines, DxfMode::FineLines] {
        let pairs = pairs(&to_dxf("<svg width=\"5pt\" height=\"5pt\"></svg>", mode).unwrap());
        assert!(entities(&pairs).is_empty());
        assert_eq!(pairs.last(), Some(&(0, "EOF".to_owned())));
    }
}
