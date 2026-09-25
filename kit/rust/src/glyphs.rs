//! Small anti-aliased lettering drawn from its pixels.
//!
//! At a cap height of about ten pixels a letter's strokes are a pixel or so
//! wide, and the engine's segmentation and smoothing have too little to hold
//! on to: on the shape set's `shape-text` the dot of the "i" in "Magik"
//! merged into its stem, "2026" smeared, and the strokes came out as wedges
//! (smallest overlap 0.84, largest edge error 1.01 px; no preset or slider
//! setting of the engine did better, and tracing the picture enlarged did
//! worse, the improvement pass's first open lead of September 23, 2026).
//!
//! The pixels settle such a shape. Each is a blend of the ink over the
//! background by how much of it the letter covers, so the letter's outline is
//! where that coverage crosses a half (marching squares on the pixel centres,
//! `recovery::contour_loops`). A stroke thinner than a pixel never reaches a
//! half, so the level follows each place's own darkest coverage nearby
//! (`LEVEL_FLOOR`): the thinnest strokes stay joined. A region is redrawn so
//! when it is thin and small (twice its area over its outline under
//! `MAX_WIDTH`, its box no longer than `MAX_SIZE`), lies
//! on one background (the pixels two away from it within
//! `BACKGROUND_SPREAD` of their median, at least `MIN_CONTRAST` from its
//! ink), is a hole of exactly one region (its host) and every hole in it is
//! a region with no holes of its own (a letter's counter), and the outline
//! keeps its ink: the area drawn within `MAX_MASS_ERROR` of the coverage it
//! stands for (a stroke that breaks up loses ink and is left to the engine).
//! Its outlines, and their copies as the host's holes and as the counters,
//! are replaced together, so every edge stays shared; the region takes the
//! ink's colour, which the coverage is measured against.
//!
//! A line of small print is written in one ink, so letters side by side
//! that the engine filled alike are a run (`letter_runs`) and take the
//! median of their redrawn letters' inks. A run with a letter the redraw
//! left (merged with the next, or ringed by a blend of its own) is redrawn
//! whole from its coverage when every region inside its box stands on one
//! host; else its letters left take the run's ink.

use crate::geometry::{turning, Cubic, Point};
use crate::raster::{hex_rgb, Raster};
use crate::recovery::{contour_loops, simplified};
use crate::shapes::{islands, rewrite_paths, Island};
use crate::simplify::{parse_all_paths, Edge, EdgeKey, Subpath};
use std::collections::{HashMap, HashSet};

/// Pixels: twice a region's area over the length of its outlines, below
/// which it is lettering this small, and the longer side of its box, at
/// most. Measured on the two pictures of small lettering (shape-text's
/// 10 px line, shape-text-small): up to 1.4 px both overlaps rise (0.8388
/// -> 0.8415, 0.2499 -> 0.3573); wider, the 16 to 24 px serif and
/// monospace letters' outlines drawn from the pixels come out a little
/// fatter than the engine's (shape-text 0.8330 at 2.6 px), whatever the
/// level's floor (0.8310 at a plain half); longer, thin rings and
/// hairlines joined in and lost overlap (0.9527 -> 0.9278). Raised to 2 px
/// on September 25, 2026, once the outline was drawn without its ripple and
/// the ink read from the pixels: small print at 13 and 14 px strokes 1.4 to
/// 1.7 px wide on average and kept the engine's blended fill (the pale inks
/// on dark scored as other inks); quality round glyph-hue-w20, total -0.0469
/// (geometry -0.0122, pixels -0.0063), holding with any sample left out (1.7
/// px: -0.0459; 1.4 px: -0.0114, resting on one picture).
const MAX_WIDTH: f64 = 2.;
const MAX_SIZE: f64 = 16.;
/// Levels (largest channel) between the ink and the background, at least.
const MIN_CONTRAST: f64 = 40.;
/// Levels (largest channel): nine in ten of the background's pixels lie
/// this close to their median.
const BACKGROUND_SPREAD: f64 = 12.;
/// The share of a region's pixels less inked than its ink.
const INK_PERCENTILE: f64 = 0.95;
/// The outline sits at half the darkest coverage within a pixel, that
/// darkest coverage taken as at least this much: a solid stroke's level is
/// a half, the faintest's 0.4 (measured on the lettering: 0.5, 0.65 and 0.8
/// kept 0.842, 0.843 and 0.843 of the small text's overlap, a plain half
/// 0.837 with its thinnest strokes broken).
const LEVEL_FLOOR: f64 = 0.8;
/// The drawn area may differ from the coverage it stands for by this share.
const MAX_MASS_ERROR: f64 = 0.15;
/// Pixels round the region whose coverage counts.
const REACH: usize = 2;
/// Pixels: the outlines' polygons, their points placed from the coverage
/// (`placed`), are simplified within this distance, and their ripple is
/// dropped within `RIPPLE` before `spline` draws them. The points wobble by a
/// few hundredths of a pixel, and a curve through every one of them (the
/// Catmull-Rom cubics of September 23 and 24, 2026) turned back and forth
/// with them: the small print's inflections 1.78 per 100 px (the engine's)
/// -> 3.96 to 4.88, and no setting passed the quality rounds' rule. Drawn by
/// `spline` (September 25, 2026, testing/quality-round/glyphs-bs-*): 1.61,
/// the dark small print 2.09 -> 2.11 and the 10 px line 1.78 -> 1.57, and the
/// round's total -0.0175 (geometry -0.0029, pixels -0.0025, smoothness
/// -0.0048, size +0.0068), holding with any one sample left out. Simplified
/// within 0.15 px with the ripple within 0.35, smoother still (-0.0190) and
/// less faithful (geometry -0.0022, pixels -0.0010); 0.25 and 0.15, or 0.15
/// and 0.25, failed without the smallest print.
const SIMPLIFY: f64 = 0.1;
const RIPPLE: f64 = 0.3;
/// Degrees an outline turns at one of its points, at least, to keep a
/// corner there; below it the outline runs smoothly through the point.
const CORNER_DEGREES: f64 = 50.;
/// Coverage at or above which pixels are one patch of ink. A patch drawn as
/// several shapes is letters the trace had merged and the pixels part again
/// ("ect" at a 10 px cap) when there are at most `MAX_PARTS` and each is at
/// least `MIN_PART` square pixels, or else a stroke broken into dashes where
/// it runs too faint (the thin ring of an "@" came out in ten pieces).
const PATCH: f64 = 0.3;
const MAX_PARTS: usize = 4;
const MIN_PART: f64 = 3.;
/// Pixels between two letters' boxes, at most, and the share of the lower
/// one's height their rows overlap by, at least, for them to be neighbours
/// in one run of small print (a word space at a 10 px cap is 3 to 4 px).
const WORD_GAP: f64 = 6.;
const ROW_OVERLAP: f64 = 1. / 3.;
/// Levels (each channel): the engine's fills of two letters of one run lie
/// this close (the pale inks on navy came out a different blend per letter).
const FILL_NEAR: f64 = 32.;
/// Pixels: the longer side of a thin region's box, at most, for it to take
/// its run's ink although too long to redraw (letters the engine merged).
const RUN_SIZE: f64 = 64.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphStats {
    /// Thin regions on one background, and those redrawn.
    pub candidates: usize,
    pub redrawn: usize,
    /// Letters the redraw left that took their run's ink (`letter_runs`).
    pub recoloured: usize,
    /// Runs of letters redrawn whole (`run_redraw` in `redraw_glyphs`).
    pub runs: usize,
}

/// Regions to redraw: the letter that leads them (for its run), every region
/// replaced (the letter and its counters, or a whole run's), the host round
/// them, the ink and the new outlines.
struct Redraw {
    lead: usize,
    members: Vec<usize>,
    host: usize,
    /// The host's holes the regions filled (a run's), when not each member's
    /// own copy.
    host_holes: Vec<usize>,
    ink: [u8; 3],
    outers: Vec<Vec<Point>>,
    holes: Vec<Vec<Point>>,
}

/// `svg` (an engine document of anti-aliased `source`) with its small thin
/// lettering drawn from its half-coverage outlines.
pub fn redraw_glyphs(svg: &str, source: &Raster) -> Result<(String, GlyphStats), String> {
    let (w, h) = (source.width, source.height);
    let found = islands(svg)?;
    let (ranges, paths) = parse_all_paths(svg)?;
    // Which island each subpath belongs to, and every subpath by its edges,
    // so a region's outline finds its copy in the region around it.
    let mut owner_of: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, island) in found.iter().enumerate() {
        for &sub in std::iter::once(&island.outer).chain(&island.holes) {
            owner_of.insert((island.path, sub), i);
        }
    }
    let mut copies: HashMap<Vec<EdgeKey>, Vec<(usize, usize)>> = HashMap::new();
    for (p, path) in paths.iter().enumerate() {
        for (s, sub) in path.iter().enumerate() {
            copies.entry(signature(sub)).or_default().push((p, s));
        }
    }
    // The one other copy of a subpath: its island and whether it is that
    // island's outer outline.
    let other_copy = |path: usize, sub: usize| -> Option<(usize, bool)> {
        let same = copies.get(&signature(&paths[path][sub]))?;
        let others: Vec<&(usize, usize)> = same.iter().filter(|&&c| c != (path, sub)).collect();
        match others.as_slice() {
            [&(p, s)] => owner_of
                .get(&(p, s))
                .map(|&i| (i, found[i].path == p && found[i].outer == s)),
            _ => None,
        }
    };
    let mut owner = vec![u32::MAX; w * h];
    for (i, island) in found.iter().enumerate() {
        for p in island.covered(w, h, false) {
            owner[p as usize] = i as u32;
        }
    }
    let colour = |p: usize| {
        let c = source.pixels[p].0;
        [c[0] as f64, c[1] as f64, c[2] as f64]
    };
    let mut stats = GlyphStats::default();
    let mut redraws: Vec<Redraw> = Vec::new();
    let mut taken: HashSet<usize> = HashSet::new();
    let mut thin: Vec<usize> = Vec::new();
    for (i, island) in found.iter().enumerate() {
        let size = (island.max.x - island.min.x).max(island.max.y - island.min.y);
        if taken.contains(&i) {
            continue;
        }
        if width(island) >= MAX_WIDTH || size > MAX_SIZE {
            // Letters the engine merged are no glyph, but take their run's
            // ink (`pooled_inks`).
            if width(island) < MAX_WIDTH && size <= RUN_SIZE {
                thin.push(i);
            }
            continue;
        }
        thin.push(i);
        let Some(fill) = hex_rgb(&island.color).map(|c| c.map(f64::from)) else {
            continue;
        };
        // One host round it (its outline is a hole there); every hole a
        // counter (a region's outer outline) with no holes of its own.
        // The host is no thin shape itself (a letter's counter is no glyph
        // on its letter).
        let Some((host, false)) = other_copy(island.path, island.outer) else {
            continue;
        };
        if width(&found[host]) < MAX_WIDTH {
            continue;
        }
        let counters: Option<Vec<usize>> = island
            .holes
            .iter()
            .map(|&hole| match other_copy(island.path, hole) {
                Some((c, true)) if found[c].holes.is_empty() && c != host => Some(c),
                _ => None,
            })
            .collect();
        let Some(counters) = counters else {
            continue;
        };
        let inside: Vec<usize> = island
            .covered(w, h, false)
            .into_iter()
            .map(|p| p as usize)
            .collect();
        if inside.is_empty() || inside.iter().any(|&p| source.pixels[p].0[3] != 255) {
            continue;
        }
        let members: HashSet<u32> = std::iter::once(i)
            .chain(std::iter::once(host))
            .chain(counters.iter().copied())
            .map(|m| m as u32)
            .collect();
        let near = dilated(&inside, w, h, REACH);
        // Lettering this small stands a pixel or two from the next letter:
        // the background is read where no other shape is within a step.
        let foreign = |p: usize| owner[p] != u32::MAX && !members.contains(&owner[p]);
        let clear = |p: usize| {
            members.contains(&owner[p]) && !dilated(&[p], w, h, 1).into_iter().any(foreign)
        };
        let Some(back) = background(&near, &inside, w, h, &colour, clear) else {
            continue;
        };
        // The ink's hue and reach from the region's own pixels (their mean
        // step along it), not from the fill the engine's colour model gave it.
        let span = crate::strokes::ink_way(&inside, colour, back)
            .map(|way| {
                let reach = inside
                    .iter()
                    .map(|&p| {
                        (0..3)
                            .map(|k| (colour(p)[k] - back[k]) * way[k])
                            .sum::<f64>()
                    })
                    .sum::<f64>()
                    / inside.len() as f64;
                way.map(|v| v * reach)
            })
            .unwrap_or([0, 1, 2].map(|k| fill[k] - back[k]));
        let span2: f64 = span.iter().map(|v| v * v).sum();
        if span2 <= 0. {
            continue;
        }
        let mut shares: Vec<f64> = inside
            .iter()
            .map(|&p| {
                (0..3)
                    .map(|k| (colour(p)[k] - back[k]) * span[k])
                    .sum::<f64>()
                    / span2
            })
            .collect();
        shares.sort_by(f64::total_cmp);
        let ink_share =
            shares[((shares.len() - 1) as f64 * INK_PERCENTILE).round() as usize].max(1.);
        let ink = [0, 1, 2].map(|k| (back[k] + ink_share * span[k]).clamp(0., 255.));
        let contrast = (0..3).map(|k| (ink[k] - back[k]).abs()).fold(0., f64::max);
        if contrast < MIN_CONTRAST {
            continue;
        }
        stats.candidates += 1;
        // The coverage of the ink over the background, on the region, the
        // pixels within reach of it that its host holds away from any other
        // shape (a pixel between two letters is a blend of both), and its
        // counters.
        let mut domain: Vec<usize> = near
            .into_iter()
            .filter(|&p| owner[p] == i as u32 || clear(p))
            .collect();
        for &c in &counters {
            domain.extend(
                found[c]
                    .covered(w, h, false)
                    .into_iter()
                    .map(|p| p as usize),
            );
        }
        domain.sort_unstable();
        domain.dedup();
        let Some((outers, holes)) = outlines(&domain, w, h, |p| coverage(colour(p), back, ink))
        else {
            continue;
        };
        taken.insert(i);
        taken.insert(host);
        taken.extend(counters.iter().copied());
        redraws.push(Redraw {
            lead: i,
            members: std::iter::once(i).chain(counters).collect(),
            host,
            host_holes: Vec::new(),
            ink: ink.map(|v| v.round() as u8),
            outers,
            holes,
        });
    }
    stats.redrawn = redraws.len();
    if redraws.is_empty() {
        return Ok((svg.to_owned(), stats));
    }
    // Letters side by side in a row that the engine filled alike are one ink
    // (`letter_runs`): each run takes the median of its redrawn letters'
    // inks. A run with a letter the redraw left (the engine had merged it
    // with the next, or ringed it with a blend of its own) is redrawn whole
    // from its coverage, every region inside its box replaced, when those
    // regions stand on its host alone; else the letters left take the run's
    // ink.
    let lead_of: HashMap<usize, usize> = redraws
        .iter()
        .enumerate()
        .map(|(k, r)| (r.lead, k))
        .collect();
    let runs = letter_runs(&found, &thin, &taken, &lead_of);
    // Which regions draw each edge: a region the engine ringed with a blend
    // shares its outline with two neighbours, so no single subpath copies it.
    let mut edge_owners: HashMap<EdgeKey, Vec<usize>> = HashMap::new();
    for (i, island) in found.iter().enumerate() {
        for &sub in std::iter::once(&island.outer).chain(&island.holes) {
            for key in signature(&paths[island.path][sub]) {
                edge_owners.entry(key).or_default().push(i);
            }
        }
    }
    // Which letter redraw took each region (the letter, its counters).
    let redraw_of: HashMap<usize, usize> = redraws
        .iter()
        .enumerate()
        .flat_map(|(k, r)| r.members.iter().map(move |&m| (m, k)))
        .collect();
    let run_redraw = |run: &[usize],
                      ink: [u8; 3],
                      claimed: &HashSet<usize>|
     -> Option<(Redraw, Vec<usize>)> {
        let own: Vec<usize> = run.iter().filter_map(|i| lead_of.get(i).copied()).collect();
        let host = redraws[*own.first()?].host;
        if own.iter().any(|&k| redraws[k].host != host) {
            return None;
        }
        let mine: HashSet<usize> = own
            .iter()
            .flat_map(|&k| redraws[k].members.iter().copied())
            .collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &i in run {
            let b = &found[i];
            (x0, y0, x1, y1) = (
                x0.min(b.min.x),
                y0.min(b.min.y),
                x1.max(b.max.x),
                y1.max(b.max.y),
            );
        }
        let members: Vec<usize> = (0..found.len())
            .filter(|&j| {
                let b = &found[j];
                j != host
                    && b.min.x >= x0 - 1.
                    && b.min.y >= y0 - 1.
                    && b.max.x <= x1 + 1.
                    && b.max.y <= y1 + 1.
            })
            .collect();
        let set: HashSet<usize> = members.iter().copied().collect();
        // A letter another run redrew inside the box goes with this run when
        // all of it lies inside; any other region taken stops it.
        let mut absorbed: Vec<usize> = own.clone();
        for &m in &members {
            if claimed.contains(&m) {
                return None;
            }
            if taken.contains(&m) && !mine.contains(&m) {
                match redraw_of.get(&m) {
                    Some(&k)
                        if redraws[k].host == host
                            && redraws[k].members.iter().all(|x| set.contains(x)) =>
                    {
                        if !absorbed.contains(&k) {
                            absorbed.push(k);
                        }
                    }
                    _ => {
                        return None;
                    }
                }
            }
            // Every edge drawn by another member or the host, and by someone.
            for &sub in std::iter::once(&found[m].outer).chain(&found[m].holes) {
                for key in signature(&paths[found[m].path][sub]) {
                    let others: Vec<usize> = edge_owners[&key]
                        .iter()
                        .copied()
                        .filter(|&o| o != m)
                        .collect();
                    if others.is_empty() || others.iter().any(|&o| o != host && !set.contains(&o)) {
                        return None;
                    }
                }
            }
        }
        // The host's holes along the run: every edge of each shared with a
        // member.
        let mut host_holes = Vec::new();
        for &hole in &found[host].holes {
            let keys = signature(&paths[found[host].path][hole]);
            let along = |key: &EdgeKey| edge_owners[key].iter().any(|o| set.contains(o));
            if keys.iter().any(along) {
                if !keys.iter().all(along) {
                    return None;
                }
                host_holes.push(hole);
            }
        }
        if host_holes.is_empty() {
            return None;
        }
        let inside: Vec<usize> = members
            .iter()
            .flat_map(|&m| found[m].covered(w, h, false))
            .map(|p| p as usize)
            .collect();
        if inside.is_empty() || inside.iter().any(|&p| source.pixels[p].0[3] != 255) {
            return None;
        }
        let near = dilated(&inside, w, h, REACH);
        let ours = |p: usize| owner[p] != u32::MAX && set.contains(&(owner[p] as usize));
        let foreign = |p: usize| owner[p] != u32::MAX && owner[p] != host as u32 && !ours(p);
        let clear =
            |p: usize| owner[p] == host as u32 && !dilated(&[p], w, h, 1).into_iter().any(foreign);
        let back = background(&near, &inside, w, h, &colour, clear)?;
        let ink = ink.map(f64::from);
        if (0..3).map(|k| (ink[k] - back[k]).abs()).fold(0., f64::max) < MIN_CONTRAST {
            return None;
        }
        let domain: Vec<usize> = near.into_iter().filter(|&p| ours(p) || clear(p)).collect();
        let (outers, holes) = outlines(&domain, w, h, |p| coverage(colour(p), back, ink))?;
        Some((
            Redraw {
                lead: run[0],
                members,
                host,
                host_holes,
                ink: ink.map(|v| v.round() as u8),
                outers,
                holes,
            },
            absorbed,
        ))
    };
    let mut pooled: Vec<(usize, [u8; 3])> = Vec::new();
    let mut recolours: Vec<(usize, [u8; 3])> = Vec::new();
    let mut replaced: HashSet<usize> = HashSet::new();
    let mut whole: Vec<Redraw> = Vec::new();
    let mut claimed: HashSet<usize> = HashSet::new();
    for run in &runs {
        let inks: Vec<[u8; 3]> = run
            .iter()
            .filter_map(|i| lead_of.get(i).map(|&k| redraws[k].ink))
            .collect();
        if inks.is_empty() {
            continue;
        }
        let ink = [0, 1, 2].map(|c| {
            let mut v: Vec<u8> = inks.iter().map(|k| k[c]).collect();
            v.sort_unstable();
            v[v.len() / 2]
        });
        let left = run.iter().any(|i| !lead_of.contains_key(i));
        if left {
            if let Some((g, absorbed)) = run_redraw(run, ink, &claimed) {
                replaced.extend(absorbed);
                claimed.extend(g.members.iter().copied());
                whole.push(g);
                continue;
            }
        }
        for &i in run {
            match lead_of.get(&i) {
                Some(&k) => pooled.push((k, ink)),
                None => recolours.push((i, ink)),
            }
        }
    }
    // A recolour inside a run redrawn whole goes with the run.
    recolours.retain(|(i, _)| !claimed.contains(i));
    for (k, ink) in pooled {
        redraws[k].ink = ink;
    }
    recolours.retain(|(i, _)| !claimed.contains(i));
    stats.runs = whole.len();
    let redraws: Vec<Redraw> = redraws
        .into_iter()
        .enumerate()
        .filter(|(k, _)| !replaced.contains(k))
        .map(|(_, r)| r)
        .chain(whole)
        .collect();
    stats.recoloured = recolours.len();
    Ok((
        write(svg, &ranges, &paths, &found, &redraws, &recolours)?,
        stats,
    ))
}

/// Runs of letters: thin, small regions side by side in a row (boxes
/// `WORD_GAP` apart at most, rows overlapping by `ROW_OVERLAP` of the lower)
/// that the engine filled alike (within `FILL_NEAR`): a line of small print
/// is written in one ink. Read one letter at a time, a pale ink on dark came
/// out nearer the next pale ink as often as not, and the letters the redraw
/// left kept the fill the engine's colour model groups pale inks under: the
/// lilac line of the dark small print was 42% white, its white line 15%
/// lilac (September 25, 2026). Every region is a redrawn letter or one the
/// redraw left (not a host or counter it took).
fn letter_runs(
    found: &[Island],
    thin: &[usize],
    taken: &HashSet<usize>,
    lead_of: &HashMap<usize, usize>,
) -> Vec<Vec<usize>> {
    let members: Vec<usize> = thin
        .iter()
        .copied()
        .filter(|i| lead_of.contains_key(i) || !taken.contains(i))
        .collect();
    let mut root: Vec<usize> = (0..members.len()).collect();
    fn find(root: &mut [usize], mut k: usize) -> usize {
        while root[k] != k {
            root[k] = root[root[k]];
            k = root[k];
        }
        k
    }
    for a in 0..members.len() {
        for b in a + 1..members.len() {
            let (p, q) = (&found[members[a]], &found[members[b]]);
            let gap = (q.min.x - p.max.x).max(p.min.x - q.max.x);
            let overlap = p.max.y.min(q.max.y) - p.min.y.max(q.min.y);
            let lower = (p.max.y - p.min.y).min(q.max.y - q.min.y);
            let near = match (hex_rgb(&p.color), hex_rgb(&q.color)) {
                (Some(a), Some(b)) => (0..3).all(|k| a[k].abs_diff(b[k]) as f64 <= FILL_NEAR),
                _ => false,
            };
            if near && gap <= WORD_GAP && overlap >= ROW_OVERLAP * lower {
                let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                root[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut runs: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, &m) in members.iter().enumerate() {
        let r = find(&mut root, k);
        runs.entry(r).or_default().push(m);
    }
    let mut roots: Vec<usize> = runs.keys().copied().collect();
    roots.sort_unstable();
    roots
        .into_iter()
        .map(|r| runs.remove(&r).unwrap_or_default())
        .collect()
}

/// A subpath's edges, whichever way it runs: its copy in the region on the
/// other side has the same.
fn signature(sub: &Subpath) -> Vec<EdgeKey> {
    let mut keys: Vec<EdgeKey> = sub
        .edges
        .iter()
        .filter(|e| e.start() != e.end())
        .map(|e| e.key().0)
        .collect();
    keys.sort_unstable();
    keys
}

/// Twice the island's area over the length of its outlines: its stroke
/// width, for a stroke.
fn width(island: &Island) -> f64 {
    let perimeter: f64 = std::iter::once(&island.outline)
        .chain(&island.hole_outlines)
        .map(|l| {
            (0..l.len())
                .map(|k| {
                    let (a, b) = (l[k], l[(k + 1) % l.len()]);
                    (b.x - a.x).hypot(b.y - a.y)
                })
                .sum::<f64>()
        })
        .sum();
    if perimeter <= 0. {
        f64::INFINITY
    } else {
        2. * island.area() / perimeter
    }
}

/// The pixels within `steps` (eight-neighbour steps) of `set`, `set` too.
fn dilated(set: &[usize], w: usize, h: usize, steps: usize) -> Vec<usize> {
    let mut on: HashSet<usize> = set.iter().copied().collect();
    let mut edge: Vec<usize> = set.to_vec();
    for _ in 0..steps {
        let mut next = Vec::new();
        for &p in &edge {
            let (x, y) = ((p % w) as i64, (p / w) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        let q = ny as usize * w + nx as usize;
                        if on.insert(q) {
                            next.push(q);
                        }
                    }
                }
            }
        }
        edge = next;
    }
    let mut all: Vec<usize> = on.into_iter().collect();
    all.sort_unstable();
    all
}

/// The background round a region: the per-channel median of the pixels
/// `REACH` steps from it (in `near`, not within one step of it, and
/// `clear`), when nine in ten lie within `BACKGROUND_SPREAD` of it.
fn background(
    near: &[usize],
    inside: &[usize],
    w: usize,
    h: usize,
    colour: &impl Fn(usize) -> [f64; 3],
    clear: impl Fn(usize) -> bool,
) -> Option<[f64; 3]> {
    let close: HashSet<usize> = dilated(inside, w, h, 1).into_iter().collect();
    let ring: Vec<usize> = near
        .iter()
        .copied()
        .filter(|&p| !close.contains(&p) && clear(p))
        .collect();
    if ring.len() < 8 {
        return None;
    }
    let median = |c: usize| {
        let mut v: Vec<f64> = ring.iter().map(|&p| colour(p)[c]).collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let back = [median(0), median(1), median(2)];
    let within = ring
        .iter()
        .filter(|&&p| (0..3).all(|c| (colour(p)[c] - back[c]).abs() <= BACKGROUND_SPREAD))
        .count();
    (within * 10 >= ring.len() * 9).then_some(back)
}

/// How much of `ink` over `back` the colour `c` is, from 0 to 1.
fn coverage(c: [f64; 3], back: [f64; 3], ink: [f64; 3]) -> f64 {
    let d = [0, 1, 2].map(|k| ink[k] - back[k]);
    let n: f64 = d.iter().map(|v| v * v).sum();
    let v: f64 = (0..3).map(|k| (c[k] - back[k]) * d[k]).sum();
    (v / n).clamp(0., 1.)
}

/// The outlines of the coverage over `domain` (zero elsewhere): the outer
/// loops and the holes, simplified, when the domain keeps a pixel
/// clear of the picture's edge, the loops nest no deeper than a hole in a
/// shape, no patch of ink breaks into dashes (`PATCH`), and the area they
/// enclose matches the coverage.
#[allow(clippy::type_complexity, reason = "the outer loops and the holes")]
fn outlines(
    domain: &[usize],
    w: usize,
    h: usize,
    alpha: impl Fn(usize) -> f64,
) -> Option<(Vec<Vec<Point>>, Vec<Vec<Point>>)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for &p in domain {
        let (x, y) = (p % w, p / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    if x0 == 0 || y0 == 0 || x1 + 1 >= w || y1 + 1 >= h {
        return None;
    }
    let (gx0, gy0) = (x0 - 1, y0 - 1);
    let (gw, gh) = (x1 - x0 + 3, y1 - y0 + 3);
    let mut value = vec![0.; gw * gh];
    let mut mass = 0.;
    for &p in domain {
        let a = alpha(p);
        value[(p / w - gy0) * gw + p % w - gx0] = a;
        mass += a;
    }
    if mass <= 0. {
        return None;
    }
    // Contoured at a half of the darkest coverage within a pixel, as the
    // level 0.5 of the coverage shifted by what that level lacks of a half.
    let shifted: Vec<f64> = (0..gw * gh)
        .map(|i| {
            let (gx, gy) = (i % gw, i / gw);
            let mut peak: f64 = 0.;
            for ny in gy.saturating_sub(1)..(gy + 2).min(gh) {
                for nx in gx.saturating_sub(1)..(gx + 2).min(gw) {
                    peak = peak.max(value[ny * gw + nx]);
                }
            }
            value[i] - 0.5 * peak.clamp(LEVEL_FLOOR, 1.) + 0.5
        })
        .collect();
    let loops: Vec<Vec<Point>> = contour_loops(&shifted, gw, gh, (gx0, gy0), 0.5)?
        .into_iter()
        .map(|l| {
            written_loop(&simplified(
                &placed(&l, &value, gw, gh, (gx0, gy0)),
                SIMPLIFY,
            ))
        })
        .filter(|l| l.len() >= 3 && signed_area(l).abs() > 1e-3)
        .collect();
    // Nesting: a loop inside an odd number of others is a hole.
    let depth = |k: usize| {
        loops
            .iter()
            .enumerate()
            .filter(|&(j, other)| j != k && encloses(other, loops[k][0]))
            .count()
    };
    let (mut outers, mut holes) = (Vec::new(), Vec::new());
    for (k, l) in loops.iter().enumerate() {
        match depth(k) {
            0 => outers.push(l.clone()),
            1 => holes.push(l.clone()),
            _ => return None,
        }
    }
    // Each shape stands on a patch of its own, and every patch's shapes
    // hold a pixel centre of it.
    let patches = patches(&value, gw, gh);
    let mut patch_of: Vec<Option<u32>> = vec![None; outers.len()];
    for (index, &patch) in patches.iter().enumerate() {
        if patch == u32::MAX || shifted[index] < 0.5 {
            continue;
        }
        let centre = Point {
            x: (gx0 + index % gw) as f64 + 0.5,
            y: (gy0 + index / gw) as f64 + 0.5,
        };
        if let Some(k) = outers.iter().position(|l| encloses(l, centre)) {
            match patch_of[k] {
                Some(seen) if seen != patch => return None,
                _ => patch_of[k] = Some(patch),
            }
        }
    }
    let mut parts: HashMap<u32, Vec<f64>> = HashMap::new();
    for (k, patch) in patch_of.into_iter().enumerate() {
        parts
            .entry(patch?)
            .or_default()
            .push(signed_area(&outers[k]).abs());
    }
    if parts
        .values()
        .any(|a| a.len() > MAX_PARTS || (a.len() > 1 && a.iter().any(|&s| s < MIN_PART)))
    {
        return None;
    }
    let drawn: f64 = outers.iter().map(|l| signed_area(l).abs()).sum::<f64>()
        - holes.iter().map(|l| signed_area(l).abs()).sum::<f64>();
    if outers.is_empty() || (drawn - mass).abs() > MAX_MASS_ERROR * mass {
        return None;
    }
    Some((outers, holes))
}

/// The eight-connected patches of samples at or above `PATCH`, numbered
/// (`u32::MAX` below it).
fn patches(value: &[f64], gw: usize, gh: usize) -> Vec<u32> {
    let mut label = vec![u32::MAX; value.len()];
    let mut next = 0;
    for start in 0..value.len() {
        if label[start] != u32::MAX || value[start] < PATCH {
            continue;
        }
        label[start] = next;
        let mut stack = vec![start];
        while let Some(index) = stack.pop() {
            let (x, y) = ((index % gw) as i64, (index / gw) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= gw as i64 || ny >= gh as i64 {
                        continue;
                    }
                    let n = ny as usize * gw + nx as usize;
                    if label[n] == u32::MAX && value[n] >= PATCH {
                        label[n] = next;
                        stack.push(n);
                    }
                }
            }
        }
        next += 1;
    }
    label
}

/// Coverage at or below which a pixel is clear of the ink, and at or above
/// which it is solid ink, when an edge is placed from the coverage.
const CLEAR: f64 = 0.04;
const SOLID: f64 = 0.96;
/// The most partly covered pixels of one column (or row) an edge is placed
/// from: more is an edge running along that line, left where marching
/// squares put it.
const MAX_RUN: usize = 4;

/// The loop with each point on the line between two pixel centres of a
/// column (or a row) moved to where the coverage puts the edge there.
/// Marching squares crosses between the two centres linearly, and the box
/// filter's coverage is not linear across an edge, so the crossing wobbles
/// with the edge's phase against the pixel grid (a straight edge traced as a
/// few hundredths of a pixel of ripple, which every curve through the points
/// carries as inflections). The coverage itself places a straight edge
/// exactly (`edge_along`).
fn placed(
    points: &[Point],
    value: &[f64],
    gw: usize,
    gh: usize,
    origin: (usize, usize),
) -> Vec<Point> {
    let (gx0, gy0) = (origin.0 as f64, origin.1 as f64);
    let grid = |v: f64| (v - v.round()).abs() < 1e-9;
    points
        .iter()
        .map(|&p| {
            let (fx, fy) = (p.x - 0.5 - gx0, p.y - 0.5 - gy0);
            let mut q = p;
            if grid(fx) && !grid(fy) && fx >= 0. && fy >= 0. {
                // Between two centres of column `fx`: the edge's height there.
                let column = fx.round() as usize;
                let line: Vec<f64> = (0..gh).map(|r| value[r * gw + column]).collect();
                if let Some(at) = edge_along(&line, fy.floor() as usize) {
                    q.y = gy0 + at;
                }
            } else if grid(fy) && !grid(fx) && fx >= 0. && fy >= 0. {
                let row = fy.round() as usize;
                if let Some(at) = edge_along(&value[row * gw..(row + 1) * gw], fx.floor() as usize)
                {
                    q.x = gx0 + at;
                }
            }
            q
        })
        .collect()
}

/// Where the ink's edge crosses a line of pixels (pixel `k` spans `k` to
/// `k + 1`) between the centres of pixels `i` and `i + 1`, from the
/// coverage alone. From the last clear pixel on the crossing's clear side
/// the pixels run toward the ink until one is solid or clear again, and the
/// pixel that ends the run counts too (a clear one's coverage under `CLEAR`
/// and a solid one's shortfall from 1 are still the edge's):
/// - run to a solid pixel, one edge: the coverage sums to the distance from
///   the edge to the solid pixel's far side, which for a straight edge is
///   exact at the line's centre;
/// - run to a clear pixel, a band of ink the coverage's sum wide, centred on
///   its first moment with each end's ink taken at the band's side of its
///   pixels (for a straight band the two ends' errors cancel); a band inside
///   one pixel says nothing of where in it the ink lies.
///
/// None when the pixels are no such edge or band within `MAX_RUN` pixels, or
/// the answer lies more than a pixel from the crossing.
fn edge_along(line: &[f64], i: usize) -> Option<f64> {
    let n = line.len();
    if i + 1 >= n {
        return None;
    }
    // The ink lies toward the larger index when `up`.
    let up = line[i + 1] > line[i];
    let step = |k: usize, toward_ink: bool| -> Option<usize> {
        let j = if toward_ink == up {
            k.checked_add(1)?
        } else {
            k.checked_sub(1)?
        };
        (j < n).then_some(j)
    };
    let mut clear = if up { i } else { i + 1 };
    for _ in 0..=MAX_RUN {
        if line[clear] <= CLEAR {
            break;
        }
        clear = step(clear, false)?;
    }
    if line[clear] > CLEAR {
        return None;
    }
    let mut run = vec![clear];
    let mut falling = false;
    let solid = loop {
        let k = step(*run.last()?, true)?;
        let (v, previous) = (line[k], line[*run.last()?]);
        run.push(k);
        if v >= SOLID {
            break true;
        }
        if v <= CLEAR {
            break false;
        }
        // A band rises to its peak and falls once; rising again is a second
        // stroke too close to part.
        if v < previous {
            falling = true;
        } else if falling && v > previous {
            return None;
        }
        if run.len() > MAX_RUN + 1 {
            return None;
        }
    };
    let c: Vec<f64> = run.iter().map(|&k| line[k]).collect();
    let sum: f64 = c.iter().sum();
    // Positions measured from the clear pixel's outer side toward the ink:
    // the run's m-th pixel spans m to m + 1.
    let along = if solid {
        run.len() as f64 - sum
    } else {
        let last = run.len() - 1;
        if last < 3 || sum <= 0. {
            return None;
        }
        let moment: f64 = c
            .iter()
            .enumerate()
            .map(|(m, &v)| {
                let m = m as f64;
                let centre = if m < 2. {
                    m + 1. - v / 2.
                } else if m > last as f64 - 2. {
                    m + v / 2.
                } else {
                    m + 0.5
                };
                v * centre
            })
            .sum();
        moment / sum - sum / 2.
    };
    let at = if up {
        clear as f64 + along
    } else {
        clear as f64 + 1. - along
    };
    near(at, i)
}

/// `at` when it lies within a pixel of the crossing between the centres of
/// pixels `i` and `i + 1` (at `i + 1`).
fn near(at: f64, i: usize) -> Option<f64> {
    ((at - (i as f64 + 1.)).abs() <= 1.).then_some(at)
}

/// The loop as the document writes it (two decimals), without a point that
/// repeats the one before.
fn written_loop(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        let q = crate::nodes::written(*p);
        if out.last() != Some(&q) {
            out.push(q);
        }
    }
    while out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    out
}

fn signed_area(l: &[Point]) -> f64 {
    let n = l.len();
    (0..n)
        .map(|k| {
            let (a, b) = (l[k], l[(k + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        / 2.
}

/// Whether `p` lies inside the closed polygon `l` (even-odd).
fn encloses(l: &[Point], p: Point) -> bool {
    let n = l.len();
    let mut inside = false;
    for k in 0..n {
        let (a, b) = (l[k], l[(k + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// The loop's closed outline turning the way asked (outer outlines turn
/// positive, holes the other way, as `recovery` writes them), drawn by
/// `spline`. The copy turning the other way is the same pieces reversed, so
/// the region and the shape round it share every edge to the bit.
fn subpath(points: &[Point], positive: bool) -> Subpath {
    let mut points = points.to_vec();
    if turning(&points) < 0. {
        points.reverse();
    }
    let mut edges = spline(&points, RIPPLE);
    if !positive {
        edges = edges.iter().rev().map(|e| e.reversed()).collect();
    }
    Subpath {
        edges,
        closed: true,
    }
}

/// A closed outline drawn from the loop (turning positive) as a quadratic
/// B-spline on it: a curve from the middle of each edge to the middle of the
/// next with the point between as its control, so it is tangent to every
/// edge at its middle, smooth at every join and bends at each point only the
/// way the loop turns there; it turns back only where the loop does (a
/// B-spline changes its bending no more often than its control polygon). So
/// the loop's ripple goes first: a point turning against a neighbour is
/// dropped, the smallest step first, when that leaves no more changes of
/// the turning's sense around it (so a reversed run of several points
/// shortens to none) and every point it and the points dropped beside it
/// stood for stays within `tolerance` of the new edge. A point
/// turning `CORNER_DEGREES` or more stays a corner: the curve runs to it
/// along the two half-edges.
fn spline(points: &[Point], tolerance: f64) -> Vec<Edge> {
    let n = points.len();
    let mut prev: Vec<usize> = (0..n).map(|k| (k + n - 1) % n).collect();
    let mut next: Vec<usize> = (0..n).map(|k| (k + 1) % n).collect();
    let mut alive = vec![true; n];
    // The points each edge (by its first point) stands for besides its ends.
    let mut stands: Vec<Vec<Point>> = vec![Vec::new(); n];
    let turn = |k: usize, prev: &[usize], next: &[usize]| {
        angle(
            sub(points[k], points[prev[k]]),
            sub(points[next[k]], points[k]),
        )
    };
    let corner_angle = CORNER_DEGREES.to_radians();
    let sense = |t: f64| {
        if t > 1e-9 {
            1
        } else if t < -1e-9 {
            -1
        } else {
            0
        }
    };
    let mut remaining = n;
    loop {
        if remaining <= 4 {
            break;
        }
        let mut best: Option<(f64, usize)> = None;
        for k in (0..n).filter(|&k| alive[k]) {
            let t = turn(k, &prev, &next);
            if t.abs() >= corner_angle {
                continue;
            }
            let (p, q) = (prev[k], next[k]);
            let (tp, tq) = (turn(p, &prev, &next), turn(q, &prev, &next));
            if sense(t) == 0 || (sense(t) == sense(tp) && sense(t) == sense(tq)) {
                continue;
            }
            // The sense changes around k now, and with k dropped.
            let (pp, qq) = (prev[p], next[q]);
            let chain = |ids: &[usize], at: &dyn Fn(usize) -> f64| {
                ids.windows(2)
                    .filter(|w| {
                        let (a, b) = (sense(at(w[0])), sense(at(w[1])));
                        a != 0 && b != 0 && a != b
                    })
                    .count()
            };
            let before = chain(&[pp, p, k, q, qq], &|i| turn(i, &prev, &next));
            let after_turn = |i: usize| {
                let (before_i, after_i) = match i {
                    i if i == p => (points[pp], points[q]),
                    i if i == q => (points[p], points[qq]),
                    i => (points[prev[i]], points[next[i]]),
                };
                angle(sub(points[i], before_i), sub(after_i, points[i]))
            };
            // Fewer changes of sense, or as many with a reversed run shorter.
            if chain(&[pp, p, q, qq], &after_turn) > before {
                continue;
            }
            let step = crate::geometry::to_segment(points[k], points[p], points[q]);
            let keeps = stands[p]
                .iter()
                .chain(&stands[k])
                .all(|&s| crate::geometry::to_segment(s, points[p], points[q]) <= tolerance);
            if step <= tolerance && keeps && best.is_none_or(|b| step < b.0) {
                best = Some((step, k));
            }
        }
        let Some((_, k)) = best else {
            break;
        };
        let (p, q) = (prev[k], next[k]);
        let absorbed: Vec<Point> = std::mem::take(&mut stands[k]);
        stands[p].push(points[k]);
        stands[p].extend(absorbed);
        next[p] = q;
        prev[q] = p;
        alive[k] = false;
        remaining -= 1;
    }
    let start = (0..n).find(|&k| alive[k]).unwrap_or(0);
    let mut order = vec![start];
    while next[*order.last().unwrap_or(&start)] != start {
        order.push(next[order[order.len() - 1]]);
    }
    let mid = |a: Point, b: Point| Point {
        x: (a.x + b.x) / 2.,
        y: (a.y + b.y) / 2.,
    };
    let mut edges = Vec::new();
    let count = order.len();
    for (i, &k) in order.iter().enumerate() {
        let (p, q) = (order[(i + count - 1) % count], order[(i + 1) % count]);
        let (from, to) = (mid(points[p], points[k]), mid(points[k], points[q]));
        if turn(k, &prev, &next).abs() >= corner_angle {
            edges.push(Edge::line(from, points[k], false));
            edges.push(Edge::line(points[k], to, false));
        } else {
            let c = points[k];
            edges.push(Edge {
                cubic: Cubic {
                    points: [
                        from,
                        Point {
                            x: from.x + 2. / 3. * (c.x - from.x),
                            y: from.y + 2. / 3. * (c.y - from.y),
                        },
                        Point {
                            x: to.x + 2. / 3. * (c.x - to.x),
                            y: to.y + 2. / 3. * (c.y - to.y),
                        },
                        to,
                    ],
                },
                line: false,
                implicit: false,
            });
        }
    }
    edges
}

fn sub(a: Point, b: Point) -> Point {
    Point {
        x: a.x - b.x,
        y: a.y - b.y,
    }
}

/// The signed angle from direction `a` to direction `b`.
fn angle(a: Point, b: Point) -> f64 {
    (a.x * b.y - a.y * b.x).atan2(a.x * b.x + a.y * b.y)
}

/// The document with every redrawn region's outlines, its host's hole and
/// its counters replaced: the host takes the new outer outlines as holes,
/// and the region (in its ink) and its counters (in the host's colour) are
/// written after everything else.
fn write(
    svg: &str,
    ranges: &[(usize, usize)],
    paths: &[Vec<Subpath>],
    found: &[Island],
    redraws: &[Redraw],
    recolours: &[(usize, [u8; 3])],
) -> Result<String, String> {
    let mut doomed: HashSet<(usize, usize)> = HashSet::new();
    let mut added: HashMap<usize, Vec<Subpath>> = HashMap::new();
    let mut appended: Vec<(String, Vec<Subpath>)> = Vec::new();
    // A recoloured region keeps its outlines, written again in its ink.
    for &(i, [r, g, b]) in recolours {
        let island = &found[i];
        let subs: Vec<usize> = std::iter::once(island.outer)
            .chain(island.holes.iter().copied())
            .collect();
        doomed.extend(subs.iter().map(|&s| (island.path, s)));
        appended.push((
            format!("{r:02x}{g:02x}{b:02x}"),
            subs.iter()
                .map(|&s| paths[island.path][s].clone())
                .collect(),
        ));
    }
    for redraw in redraws {
        let host = &found[redraw.host];
        doomed.extend(redraw.host_holes.iter().map(|&s| (host.path, s)));
        for &m in &redraw.members {
            let island = &found[m];
            doomed.insert((island.path, island.outer));
            doomed.extend(island.holes.iter().map(|&s| (island.path, s)));
            if !redraw.host_holes.is_empty() {
                continue;
            }
            // The host's copy of the region's outline, when it stands on it.
            let outer_edges = signature(&paths[island.path][island.outer]);
            if let Some(&hole) = host
                .holes
                .iter()
                .find(|&&s| signature(&paths[host.path][s]) == outer_edges)
            {
                doomed.insert((host.path, hole));
            }
        }
        added
            .entry(host.path)
            .or_default()
            .extend(redraw.outers.iter().map(|l| subpath(l, false)));
        let [r, g, b] = redraw.ink;
        let mut shape: Vec<Subpath> = redraw.outers.iter().map(|l| subpath(l, true)).collect();
        shape.extend(redraw.holes.iter().map(|l| subpath(l, false)));
        appended.push((format!("{r:02x}{g:02x}{b:02x}"), shape));
        if !redraw.holes.is_empty() {
            let counters: Vec<Subpath> = redraw.holes.iter().map(|l| subpath(l, true)).collect();
            appended.push((
                host.color.trim_start_matches('#').to_ascii_lowercase(),
                counters,
            ));
        }
    }
    let mut out = rewrite_paths(svg, ranges, paths, &doomed, &added);
    let close = out.rfind("</svg>").ok_or("Not an SVG document")?;
    let tail = out.split_off(close);
    let newline = if svg.contains("\r\n") { "\r\n" } else { "\n" };
    if !out.ends_with('\n') {
        out.push_str(newline);
    }
    for (colour, shape) in appended {
        let d = crate::simplify::write_path_data(&shape);
        out.push_str(&format!(
            "<g id=\"#{colour}ff\">{newline}<path fill=\"#{colour}\" opacity=\"1.00\" d=\"{d}\" />{newline}</g>{newline}"
        ));
    }
    out.push_str(&tail);
    Ok(out)
}

#[cfg(test)]
mod tests;
