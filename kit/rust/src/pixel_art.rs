//! Pixel art scaled up by a whole number, traced as the squares it is.
//!
//! A picture made by scaling a small drawing up k times with nearest-
//! neighbour sampling is k-by-k blocks of one colour: its outlines are the
//! blocks' edges, exactly, and nothing else is what it shows. The engine
//! smooths them like any aliased picture: on the shape set's 16x sprite the
//! red fill's steps met the black outline with slants up to 0.93 px, and on
//! the defect sweep's heart16-x8 the outline's outer side became a row of
//! diagonals while its inner side kept its steps (3% of the pixels wrong;
//! September 23, 2026).
//!
//! This traces every 4-connected area of one colour of the small drawing
//! along its pixel edges, keeps a node wherever an outline turns or three
//! areas meet (so two areas share every edge piece for piece, as the
//! engine's documents do), scales by k and writes the engine's own layout:
//! one path per area, its outer outline walked with a positive shoelace
//! area and its holes the other way, grouped by colour in order of first
//! appearance. Fully transparent areas are left out. Owned; the app runs it
//! under the improved defaults for pixel-edged pictures with an exact
//! palette that are such an upscale (`crate` users decide; see
//! `upscale_factor`).
use std::collections::HashMap;

/// The whole-number factor a picture was scaled up by with nearest-neighbour
/// sampling, when it is at least `min`: both sides divisible by it and every
/// block of that size one colour. `pixels` are RGBA in row order.
pub fn upscale_factor(
    pixels: &[[u8; 4]],
    width: usize,
    height: usize,
    min: usize,
) -> Option<usize> {
    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let mut g = gcd(width, height);
    let mut run = |len: usize| g = gcd(g, len);
    for y in 0..height {
        let mut start = 0;
        for x in 1..=width {
            if x == width || pixels[y * width + x] != pixels[y * width + start] {
                run(x - start);
                start = x;
            }
        }
    }
    for x in 0..width {
        let mut start = 0;
        for y in 1..=height {
            if y == height || pixels[y * width + x] != pixels[start * width + x] {
                run(y - start);
                start = y;
            }
        }
    }
    if g < min || width / g < 2 || height / g < 2 {
        return None;
    }
    // Runs that are all multiples of g along both axes still need their
    // blocks aligned: every block one colour.
    for by in 0..height / g {
        for bx in 0..width / g {
            let first = pixels[by * g * width + bx * g];
            for y in by * g..(by + 1) * g {
                for x in bx * g..(bx + 1) * g {
                    if pixels[y * width + x] != first {
                        return None;
                    }
                }
            }
        }
    }
    Some(g)
}

/// A lattice vertex of the small drawing.
type Vertex = (i64, i64);

/// The engine-layout SVG of the small drawing `pixels` (`width` by
/// `height`, RGBA) scaled by `scale`, and the number of areas drawn and of
/// nodes written.
pub fn trace(
    pixels: &[[u8; 4]],
    width: usize,
    height: usize,
    scale: usize,
) -> (String, usize, usize) {
    let colour = |i: usize| {
        let p = pixels[i];
        if p[3] == 0 {
            [0; 4]
        } else {
            p
        }
    };
    // 4-connected areas of one colour, numbered in scan order.
    let n = width * height;
    let mut label = vec![usize::MAX; n];
    let mut areas: Vec<[u8; 4]> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..n {
        if label[start] != usize::MAX {
            continue;
        }
        let own = colour(start);
        let id = areas.len();
        areas.push(own);
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % width, i / width);
            let mut visit = |j: usize| {
                if label[j] == usize::MAX && colour(j) == own {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < width {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - width);
            }
            if y + 1 < height {
                visit(i + width);
            }
        }
    }
    let at = |x: i64, y: i64| -> Option<usize> {
        (x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height)
            .then(|| label[y as usize * width + x as usize])
    };
    // Every unit edge between two areas (or an area and the frame), directed
    // with its area on the walk's positive side: a pixel's top edge left to
    // right, right edge down, bottom edge right to left, left edge up.
    let mut edges: Vec<Vec<(Vertex, Vertex)>> = vec![Vec::new(); areas.len()];
    let mut degree: HashMap<Vertex, usize> = HashMap::new();
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let own = at(x, y).unwrap();
            let sides = [
                ((x, y - 1), (x, y), (x + 1, y)),
                ((x + 1, y), (x + 1, y), (x + 1, y + 1)),
                ((x, y + 1), (x + 1, y + 1), (x, y + 1)),
                ((x - 1, y), (x, y + 1), (x, y)),
            ];
            for ((nx, ny), a, b) in sides {
                if at(nx, ny) != Some(own) {
                    edges[own].push((a, b));
                }
            }
        }
    }
    // A vertex's degree is the number of distinct unit edges meeting there.
    let mut seen: std::collections::HashSet<(Vertex, Vertex)> = std::collections::HashSet::new();
    for list in &edges {
        for &(a, b) in list {
            let key = if a <= b { (a, b) } else { (b, a) };
            if seen.insert(key) {
                *degree.entry(a).or_default() += 1;
                *degree.entry(b).or_default() += 1;
            }
        }
    }
    let mut groups: Vec<([u8; 4], Vec<String>)> = Vec::new();
    let mut nodes = 0;
    let mut drawn = 0;
    for (id, list) in edges.iter().enumerate() {
        let fill = areas[id];
        if fill[3] == 0 || list.is_empty() {
            continue;
        }
        let mut leaving: HashMap<Vertex, Vec<usize>> = HashMap::new();
        for (k, &(a, _)) in list.iter().enumerate() {
            leaving.entry(a).or_default().push(k);
        }
        let mut used = vec![false; list.len()];
        let mut loops: Vec<Vec<Vertex>> = Vec::new();
        for first in 0..list.len() {
            if used[first] {
                continue;
            }
            let mut walk = vec![list[first].0];
            let mut k = first;
            loop {
                used[k] = true;
                let (a, b) = list[k];
                if b == list[first].0 {
                    break;
                }
                walk.push(b);
                // At a vertex the area touches itself across a corner, turn
                // the way that keeps the loop tight (right, in picture
                // coordinates, of the direction arrived along).
                let dir = (b.0 - a.0, b.1 - a.1);
                let candidates = &leaving[&b];
                let pick = candidates
                    .iter()
                    .copied()
                    .filter(|&c| !used[c])
                    .min_by_key(|&c| {
                        let (_, e) = list[c];
                        let d = (e.0 - b.0, e.1 - b.1);
                        // Right turn first, then straight, then left.
                        let cross = dir.0 * d.1 - dir.1 * d.0;
                        let along = dir.0 * d.0 + dir.1 * d.1;
                        match (cross.signum(), along.signum()) {
                            (1, _) => 0,
                            (0, 1) => 1,
                            _ => 2,
                        }
                    });
                let Some(next) = pick else {
                    break;
                };
                k = next;
            }
            // Keep a vertex where the outline turns or three edges meet.
            let m = walk.len();
            let kept: Vec<Vertex> = (0..m)
                .filter(|&i| {
                    let (p, q, r) = (walk[(i + m - 1) % m], walk[i], walk[(i + 1) % m]);
                    let straight = (q.0 - p.0) * (r.1 - q.1) - (q.1 - p.1) * (r.0 - q.0) == 0;
                    !straight || degree.get(&q).copied().unwrap_or(0) > 2
                })
                .map(|i| walk[i])
                .collect();
            if kept.len() >= 3 {
                loops.push(kept);
            }
        }
        if loops.is_empty() {
            continue;
        }
        // The outer outline first (the largest positive area), then holes.
        let area = |l: &Vec<Vertex>| -> i64 {
            (0..l.len())
                .map(|i| {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    a.0 * b.1 - b.0 * a.1
                })
                .sum()
        };
        loops.sort_by_key(|l| std::cmp::Reverse(area(l)));
        let s = scale as f64;
        let mut d = String::new();
        for l in &loops {
            nodes += l.len();
            d.push_str(&format!(
                " M {:.2} {:.2}",
                l[0].0 as f64 * s,
                l[0].1 as f64 * s
            ));
            for v in l.iter().skip(1).chain(std::iter::once(&l[0])) {
                d.push_str(&format!(" L {:.2} {:.2}", v.0 as f64 * s, v.1 as f64 * s));
            }
        }
        d.push_str(" Z");
        let path = format!(
            "<path fill=\"#{:02x}{:02x}{:02x}\" opacity=\"{}\" d=\"{d}\" />",
            fill[0],
            fill[1],
            fill[2],
            if fill[3] > 0xfa {
                "1.00".to_owned()
            } else {
                format!("{:.2}", fill[3] as f64 / 255.)
            }
        );
        drawn += 1;
        match groups.iter_mut().find(|(c, _)| *c == fill) {
            Some((_, paths)) => paths.push(path),
            None => groups.push((fill, vec![path])),
        }
    }
    let (w, h) = (width * scale, height * scale);
    let mut svg = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\r\n");
    svg.push_str(&format!(
        "<svg width=\"{w}pt\" height=\"{h}pt\" viewBox=\"0 0 {w} {h}\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">"
    ));
    for (c, paths) in &groups {
        svg.push_str(&format!(
            "\r\n<g id=\"#{:02x}{:02x}{:02x}{:02x}\">",
            c[0], c[1], c[2], c[3]
        ));
        for p in paths {
            svg.push_str("\r\n");
            svg.push_str(p);
        }
        svg.push_str("\r\n</g>");
    }
    svg.push_str("\r\n</svg>\r\n");
    (svg, drawn, nodes)
}

#[cfg(test)]
mod tests;
