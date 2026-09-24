use super::*;
use crate::simplify::{key, parse_all_paths};

const W: [u8; 4] = [255, 255, 255, 255];
const K: [u8; 4] = [20, 20, 20, 255];
const R: [u8; 4] = [214, 40, 40, 255];
const B: [u8; 4] = [29, 78, 216, 255];
const G: [u8; 4] = [21, 128, 61, 255];

fn scaled(rows: &[&[[u8; 4]]], k: usize) -> (Vec<[u8; 4]>, usize, usize) {
    let (w, h) = (rows[0].len() * k, rows.len() * k);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            out.push(rows[y / k][x / k]);
        }
    }
    (out, w, h)
}

#[test]
fn a_nearest_neighbour_upscale_is_found_and_a_drawing_is_not() {
    let drawing: [&[[u8; 4]]; 2] = [&[W, K, W], &[K, K, W]];
    let (pixels, w, h) = scaled(&drawing, 4);
    assert_eq!(upscale_factor(&pixels, w, h, 3), Some(4));
    // Scaled by 2 is below the minimum asked for.
    let (pixels, w, h) = scaled(&drawing, 2);
    assert_eq!(upscale_factor(&pixels, w, h, 3), None);
    // A block that is not one colour.
    let (mut pixels, w, h) = scaled(&drawing, 4);
    pixels[5] = R;
    assert_eq!(upscale_factor(&pixels, w, h, 3), None);
}

#[test]
fn a_pixel_and_the_hole_it_leaves_are_one_outline_walked_both_ways() {
    let drawing: [&[[u8; 4]]; 3] = [&[W, W, W], &[W, K, W], &[W, W, W]];
    let (pixels, w, h) = scaled(&drawing, 1);
    let (svg, areas, _) = trace(&pixels, w, h, 5);
    assert_eq!(areas, 2);
    assert!(svg.contains("viewBox=\"0 0 15 15\""), "{svg}");
    let (_, paths) = parse_all_paths(&svg).unwrap();
    // White: the frame and a hole; black: its square, the corners only.
    assert_eq!(paths[0].len(), 2, "{svg}");
    assert_eq!(paths[1][0].edges.len(), 4, "{svg}");
    assert!(paths[1][0].edges.iter().all(|e| e.line));
    let hole: Vec<_> = paths[0][1].edges.iter().map(|e| e.key().0).collect();
    let mut square: Vec<_> = paths[1][0].edges.iter().map(|e| e.key().0).collect();
    square.sort();
    let mut sorted = hole.clone();
    sorted.sort();
    assert_eq!(sorted, square);
    // The hole turns the other way.
    let turn = |edges: &[crate::simplify::Edge]| -> f64 {
        edges
            .iter()
            .map(|e| e.start().x * e.end().y - e.end().x * e.start().y)
            .sum()
    };
    assert!(turn(&paths[1][0].edges) > 0. && turn(&paths[0][1].edges) < 0.);
}

#[test]
fn a_node_stays_where_three_areas_meet_on_a_straight_edge() {
    // Red over blue and green: the red's lower side is straight but the
    // blue and green meet under it at x = 1, so the red keeps a node there
    // and shares each piece with the area below it.
    let drawing: [&[[u8; 4]]; 2] = [&[R, R, R], &[B, G, G]];
    let (pixels, w, h) = scaled(&drawing, 1);
    let (svg, _, _) = trace(&pixels, w, h, 3);
    let (_, paths) = parse_all_paths(&svg).unwrap();
    let red = &paths[0][0];
    let node = key(crate::geometry::Point { x: 3., y: 3. });
    assert!(red.edges.iter().any(|e| key(e.start()) == node), "{svg}");
    for other in &paths[1..] {
        for edge in &other[0].edges {
            let shared = red.edges.iter().any(|r| r.key().0 == edge.key().0);
            let on_red = edge.start().y == 3. && edge.end().y == 3.;
            assert_eq!(shared, on_red, "{svg}");
        }
    }
}
