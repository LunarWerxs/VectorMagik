//! A strip of each region's colour under the edges it shares with the
//! regions painted after it, so no background shows between two colours.
//!
//! The engine writes a cut-out map: every region is its own outline with
//! holes where its neighbours lie, and two neighbours share each edge piece
//! for piece. A renderer anti-aliases each fill on its own, so a pixel the
//! edge halves gets half of one colour over the background and then half of
//! the other over that: about a quarter of the background shows through, a
//! light hairline round every region at 1x (the defect sweep of September
//! 23, 2026: 1904 of the checker's 1950 two-colour edge pixels more than a
//! tenth background; the original's export does the same).
//!
//! Here every region is followed, in the paint order, by a stroke in its own
//! colour `STRIP` px wide along each run of its outline that a later opaque
//! region shares, so the later region's anti-aliased edge blends over the
//! earlier colour, where the background showed. The strip reaches half its
//! width under the later region, which covers it, and half into the region
//! itself, the same colour; the regions' outlines are untouched and no
//! coordinate moves. A strip ends square at a junction. Joining the whole
//! of each later neighbour under a region instead was built first and lost:
//! it put the lower colour under the neighbour's other edges too (the shape
//! set's black bar blended over orange where it met white, colour error
//! 1.28 -> 1.50). A translucent neighbour gets no strip under it (its colour
//! would blend with the strip), and a document whose paths are stroked
//! already (an opaque photograph's seam overlap) is returned as it is.
//!
//! Export-time and owned: the desktop shows and saves stacked documents, the
//! CLI with `--stack on`; the planar document the passes and the node
//! editing work on is unchanged.
use crate::simplify::{parse_all_paths, write_path_data, Edge, EdgeKey, Subpath};
use std::collections::HashMap;

/// The strip's width in source pixels: half of it under the later region.
const STRIP: f64 = 1.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackStats {
    /// Regions painted, and of those the ones given a strip.
    pub regions: usize,
    pub stripped: usize,
}

/// The `<path` element holding a `d` attribute: where it ends (after `/>`),
/// its fill when that is `#rrggbb`, whether it is opaque and whether it is
/// stroked.
struct Element {
    end: usize,
    fill: Option<String>,
    opaque: bool,
    stroked: bool,
}

fn element(svg: &str, d_start: usize, d_end: usize) -> Option<Element> {
    let start = svg[..d_start].rfind("<path")?;
    let head = &svg[start..d_start];
    let attribute = |name: &str| -> Option<&str> {
        let at = head.find(&format!(" {name}=\""))? + name.len() + 3;
        head[at..].split('"').next()
    };
    let fill = attribute("fill")
        .filter(|f| f.len() == 7 && f.starts_with('#'))
        .map(str::to_owned);
    let opaque = attribute("opacity").map_or(Some(1.), |v| v.parse::<f64>().ok()) == Some(1.);
    let close = d_end + svg[d_end..].find("/>")? + 2;
    Some(Element {
        end: close,
        fill,
        opaque,
        stroked: head.contains(" stroke="),
    })
}

/// The runs of a subpath's pieces that `shared` marks, in walk order; a
/// closed outline marked all round is one closed run.
fn runs(subpath: &Subpath, shared: &dyn Fn(&Edge) -> bool) -> Vec<Subpath> {
    let edges = &subpath.edges;
    let n = edges.len();
    let marked: Vec<bool> = edges.iter().map(shared).collect();
    let plain = |e: &Edge| Edge {
        implicit: false,
        ..*e
    };
    if n > 0 && marked.iter().all(|&m| m) && subpath.cyclic() {
        return vec![Subpath {
            edges: edges.iter().map(plain).collect(),
            closed: true,
        }];
    }
    // A closed outline is walked from after an unmarked piece, so no run is
    // cut in two where the walk wraps.
    let first = if subpath.cyclic() {
        (0..n).find(|&i| !marked[i]).map_or(0, |i| i + 1)
    } else {
        0
    };
    let mut found = Vec::new();
    let mut current: Vec<Edge> = Vec::new();
    for k in 0..n {
        let i = (first + k) % n;
        if marked[i] {
            current.push(plain(&edges[i]));
        } else if !current.is_empty() {
            found.push(Subpath {
                edges: std::mem::take(&mut current),
                closed: false,
            });
        }
    }
    if !current.is_empty() {
        found.push(Subpath {
            edges: current,
            closed: false,
        });
    }
    found
}

/// The document with a strip of each region's colour under the runs of its
/// outline that later opaque regions share, written right after it;
/// everything else verbatim.
pub fn stack_svg(svg: &str) -> Result<(String, StackStats), String> {
    let (ranges, paths) = parse_all_paths(svg)?;
    let mut stats = StackStats {
        regions: paths.len(),
        ..StackStats::default()
    };
    let elements: Vec<Option<Element>> = ranges
        .iter()
        .map(|&(start, end)| element(svg, start, end))
        .collect();
    if elements
        .iter()
        .any(|e| e.as_ref().is_none_or(|e| e.stroked))
    {
        return Ok((svg.to_owned(), stats));
    }
    let elements: Vec<Element> = elements.into_iter().flatten().collect();
    let mut holders: HashMap<EdgeKey, Vec<usize>> = HashMap::new();
    for (index, path) in paths.iter().enumerate() {
        for edge in path.iter().flat_map(|s| s.edges.iter()) {
            let found = holders.entry(edge.key().0).or_default();
            if !found.contains(&index) {
                found.push(index);
            }
        }
    }
    let newline = if svg.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(svg.len() + svg.len() / 2);
    let mut copied = 0;
    for (index, path) in paths.iter().enumerate() {
        let element = &elements[index];
        let Some(fill) = &element.fill else {
            continue;
        };
        let shared = |edge: &Edge| {
            holders[&edge.key().0].iter().any(|&other| {
                other > index && elements[other].opaque && elements[other].fill.is_some()
            })
        };
        let strips: Vec<Subpath> = path.iter().flat_map(|s| runs(s, &shared)).collect();
        if strips.is_empty() {
            continue;
        }
        stats.stripped += 1;
        out.push_str(&svg[copied..element.end]);
        out.push_str(&format!(
            "{newline}<path fill=\"none\" stroke=\"{fill}\" stroke-width=\"{STRIP:.2}\" stroke-linejoin=\"round\" d=\"{}\" />",
            write_path_data(&strips)
        ));
        copied = element.end;
    }
    out.push_str(&svg[copied..]);
    Ok((out, stats))
}

#[cfg(test)]
mod tests;
