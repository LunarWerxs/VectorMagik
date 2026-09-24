//! Straight lines and square corners the picture asks for: a curve piece
//! that never leaves a thin band around its chord becomes a line, a line
//! that runs within a small angle of horizontal or vertical is snapped to
//! it (so two such lines meeting at a node make an exact 90-degree corner),
//! and the user can force both on the pieces meeting at a chosen node.
//! Owned post-processing of the engine's output, like `simplify`: pieces
//! shared by two fills are judged from the same geometry on both sides and
//! nodes move in every path that holds them, so shared edges stay sealed.
//! No snap may fold a piece (draw it backwards or with no length), and the
//! result does not depend on hash-map order, so the same document always
//! gives the same bytes.
use crate::geometry::Point;
use crate::regularize::is_bend;
use crate::simplify::{
    arriving, dot, junction_keys, key, leaving, normalized, parse_all_paths, splice, sub, Edge,
    EdgeKey, Key,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StraightenOptions {
    /// A curve whose handles stay within this many source pixels of its
    /// chord (or 3% of the chord's length per pixel of this, whichever is
    /// more) becomes a line.
    pub flatness: f64,
    /// A line within this many degrees of horizontal or vertical is snapped
    /// to it, when no node has to move further than `snap`.
    pub angle: f64,
    /// The furthest a node may move for a snap, in source pixels.
    pub snap: f64,
    /// A snapped group within `GRID_PULL` of a whole pixel lands on it:
    /// right for aliased artwork, whose edges are pixel edges, and kept for
    /// photographs (without it the astronaut's kinks rose 8.46 -> 8.77 per
    /// 100 px). An anti-aliased edge lies between pixels: pulled, a 1.7 px
    /// outline came out 1.375 px and a band's edge moved 0.3 px
    /// (kit/tools/shape_set.py, September 22, 2026), so `for_preset` turns
    /// it off for anti-aliased artwork.
    pub grid: bool,
    /// `flatness` is the picture kind's own (`auto_flatness`), set by
    /// `for_preset` once the preset the document was traced with is known.
    pub auto: bool,
}
impl Default for StraightenOptions {
    fn default() -> Self {
        // The engine's own tracing of a straight anti-aliased edge bows
        // about 0.6 px; a line this close to its chord reads as straight.
        Self {
            flatness: 0.8,
            angle: 3.,
            snap: 1.,
            grid: true,
            auto: false,
        }
    }
}

/// The bow tolerance Auto straightening gives a document traced with the
/// basic preset `preset` (`crate::basic_preset_code`): 0.2 px on
/// anti-aliased artwork, 0.65 on aliased artwork, 1.2 on photographs.
/// Measured on September 22, 2026 on the desktop's chain against the
/// straighten-aa, decide-straighten and straighten-photo rounds of
/// kit/tools/quality_round.py, picked by kit/tools/pick_variant.py per kind
/// of picture: the one 0.8 for everything drew lines where anti-aliased
/// curves bow (the gear logo's colour error 1.93 against 1.65 at 0.2, the
/// rounded rectangle 0.58 px out of shape against 0.47) and where the
/// aliased logo's did (4.30 against 3.95 at 0.65), while on the photograph
/// 1.2 drew fewer wobbles (inflections 1.20 -> 1.11 per 100 px) at the
/// same colour error.
pub fn auto_flatness(preset: usize) -> f64 {
    use crate::{basic_preset_code, ImageCategory, Quality};
    let kind = |category| {
        [Quality::High, Quality::Medium, Quality::Low]
            .into_iter()
            .any(|q| basic_preset_code(category, q) == preset)
    };
    if kind(ImageCategory::AntiAliasedArtwork) {
        0.2
    } else if kind(ImageCategory::AliasedArtwork) {
        0.65
    } else {
        1.2
    }
}
impl StraightenOptions {
    /// These options for a document traced with the basic preset `preset`
    /// (`crate::basic_preset_code`): the kind's bow when `auto`, and for
    /// anti-aliased artwork's presets no whole-pixel pull (`grid`) and no
    /// snap moving a node further than `ANTI_ALIASED_SNAP`.
    pub fn for_preset(self, preset: usize) -> Self {
        use crate::{basic_preset_code, ImageCategory, Quality};
        let anti_aliased = [Quality::High, Quality::Medium, Quality::Low]
            .into_iter()
            .any(|q| basic_preset_code(ImageCategory::AntiAliasedArtwork, q) == preset);
        Self {
            flatness: if self.auto {
                auto_flatness(preset)
            } else {
                self.flatness
            },
            grid: !anti_aliased,
            snap: if anti_aliased {
                self.snap.min(ANTI_ALIASED_SNAP)
            } else {
                self.snap
            },
            ..self
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(self.flatness.is_finite() && self.flatness >= 0.)
            || !(self.angle.is_finite() && (0. ..45.).contains(&self.angle))
            || !(self.snap.is_finite() && self.snap >= 0.)
        {
            return Err("Straightening needs a non-negative flatness and snap and an angle below 45 degrees".into());
        }
        Ok(())
    }
}

/// How forcing works at a chosen node: both pieces meeting there become
/// lines whatever their bow, and snap to an axis within this angle and
/// move (at a tip, where both leave the node on one side of that axis,
/// only the one nearer it).
const FORCED_ANGLE: f64 = 20.;
const FORCED_SNAP: f64 = f64::INFINITY;
/// A snapped group this close to a whole pixel lands on it.
const GRID_PULL: f64 = 0.34;
/// The furthest a snap may move a node of anti-aliased artwork, whose edges
/// carry their position between pixels: a 42 px edge of the blended logo
/// slanting 0.76 degrees snapped upright moved both nodes 0.27 px.
const ANTI_ALIASED_SNAP: f64 = 0.25;
/// The smallest turn at a node that shows as a kink (the outline metric's,
/// kit/tools/outline_metrics.py `KINK_DEGREES`), in degrees.
const VISIBLE_KINK: f64 = 3.;
/// A long piece may bow this fraction of its chord, per pixel of flatness,
/// and still be a line (2.4% at the default 0.8 px): the engine's fit over a
/// straight anti-aliased run wobbles by a percent or two of its length.
const RELATIVE_BOW_PER_PX: f64 = 0.03;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StraightenStats {
    pub lines_made: usize,
    /// Lines that lie exactly on an axis now and did not before.
    pub edges_snapped: usize,
    pub nodes_moved: usize,
}

/// The largest distance of a curve's handles from its chord, with handles
/// that overshoot the chord's ends counted by their overshoot too.
fn bow(edge: &Edge) -> f64 {
    let [p0, p1, p2, p3] = edge.cubic.points;
    let (dx, dy) = (p3.x - p0.x, p3.y - p0.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        return f64::INFINITY;
    }
    let mut worst: f64 = 0.;
    for p in [p1, p2] {
        let (vx, vy) = (p.x - p0.x, p.y - p0.y);
        let across = (vx * dy - vy * dx).abs() / len;
        let along = (vx * dx + vy * dy) / len;
        let overshoot = (-along).max(along - len).max(0.);
        worst = worst.max(across).max(overshoot);
    }
    worst
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Axis {
    Horizontal,
    Vertical,
}

/// Whether drawing `stretch[own]` as its chord would put a visible kink in
/// the bend the stretch makes: the piece bends the way the stretch turns
/// (both handles on the side its tangent turns away from; a piece that
/// snakes across its chord is wobble, whatever its neighbours do), and its
/// chord leaves a neighbour as `drawn` (the stretch with every other flat
/// piece its chord) by `VISIBLE_KINK` or more. Keeping every flat piece of
/// a bend a curve joined runs of cubics across what had been lines, and
/// zig-zags along the logos' outlines went up (the gear's inflections
/// 0.105 -> 0.179 per 100 px) where no kink had shown.
fn kinks_a_bend(stretch: &[Edge], drawn: &[Edge], own: usize) -> bool {
    let [p0, p1, p2, p3] = stretch[own].cubic.points;
    let chord = sub(p3, p0);
    let side = |p: Point| {
        let v = sub(p, p0);
        chord.x * v.y - chord.y * v.x
    };
    let (Some(first), Some(last), Some(along)) = (
        leaving(&stretch[0]),
        arriving(&stretch[stretch.len() - 1]),
        normalized(chord),
    ) else {
        return false;
    };
    // A piece with its handles on the left of its chord turns right.
    let turn = first.x * last.y - first.y * last.x;
    let (a, b) = (side(p1), side(p2));
    if !((a > 0. && b > 0. && turn < 0.) || (a < 0. && b < 0. && turn > 0.)) {
        return false;
    }
    let visible = VISIBLE_KINK.to_radians().cos();
    let before = own.checked_sub(1).and_then(|k| arriving(&drawn[k]));
    let after = drawn.get(own + 1).and_then(leaving);
    before.is_some_and(|t| dot(t, along) <= visible)
        || after.is_some_and(|t| dot(along, t) <= visible)
}

/// Whether a flat piece with no smooth neighbour is a bend its chord would
/// kink: both handles on one side of the chord, and at a node the piece
/// meets its neighbour smoothly (under a corner's 25 degrees), the chord
/// would turn it `VISIBLE_KINK` or more further than the piece does. A
/// 30-degree arc of radius 15 px bows 0.68 px, under the default flatness,
/// and meeting its neighbours at 5 degrees its chord turned each end 20.
fn lone_bend_kinks(edge: &Edge, before: Option<Edge>, after: Option<Edge>) -> bool {
    let [p0, p1, p2, p3] = edge.cubic.points;
    let chord = sub(p3, p0);
    let side = |p: Point| {
        let v = sub(p, p0);
        chord.x * v.y - chord.y * v.x
    };
    let (a, b) = (side(p1), side(p2));
    if !((a > 0. && b > 0.) || (a < 0. && b < 0.)) {
        return false;
    }
    let Some(along) = normalized(chord) else {
        return false;
    };
    let turn = |u: Point, v: Point| dot(u, v).clamp(-1., 1.).acos().to_degrees();
    let corner = 25.;
    let kinks = |now: f64, drawn: f64| now < corner && drawn >= now + VISIBLE_KINK;
    let at_start = match (before.as_ref().and_then(arriving), leaving(edge)) {
        (Some(t), Some(own)) => kinks(turn(t, own), turn(t, along)),
        _ => false,
    };
    let at_end = match (arriving(edge), after.as_ref().and_then(leaving)) {
        (Some(own), Some(u)) => kinks(turn(own, u), turn(along, u)),
        _ => false,
    };
    at_start || at_end
}

/// Which axis a line is close to, within `angle` degrees.
fn near_axis(edge: &Edge, angle: f64) -> Option<Axis> {
    let (a, b) = (edge.start(), edge.end());
    let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
    if dx < 1e-9 && dy < 1e-9 {
        return None;
    }
    let limit = angle.to_radians().tan();
    if dy <= dx * limit {
        Some(Axis::Horizontal)
    } else if dx <= dy * limit {
        Some(Axis::Vertical)
    } else {
        None
    }
}

/// How far a line runs from `node` to `other` along `axis`, and how far
/// across it.
fn leg(axis: Axis, node: Point, other: Point) -> (f64, f64) {
    match axis {
        Axis::Horizontal => (other.x - node.x, other.y - node.y),
        Axis::Vertical => (other.y - node.y, other.x - node.x),
    }
}

/// A piece's two node keys in sorted order.
type Ends = (Key, Key);

/// Where a forced tip is judged: the forced node, an axis, and whether a
/// line leaves the node towards that axis's positive direction.
type Side = (Key, Axis, bool);

/// A piece's ends: the same for both fills that share it, whichever way
/// each walks it.
fn ends_of(edge: &Edge) -> Ends {
    let (a, b) = (key(edge.start()), key(edge.end()));
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// For each node the lines leave (only nodes `at` accepts), axis and side
/// along it, the line nearest the axis within `angle` degrees: the smallest
/// ratio of its run across the axis to its run along it, the smaller ends
/// on a tie, so both fills that share a line agree.
fn nearest_lines<'a>(
    lines: impl Iterator<Item = &'a Edge>,
    angle: f64,
    at: impl Fn(Key) -> bool,
) -> HashMap<Side, (f64, Ends)> {
    let mut nearest: HashMap<Side, (f64, Ends)> = HashMap::new();
    for edge in lines {
        let Some(axis) = near_axis(edge, angle) else {
            continue;
        };
        for (node, other) in [(edge.start(), edge.end()), (edge.end(), edge.start())] {
            if !at(key(node)) {
                continue;
            }
            let (along, across) = leg(axis, node, other);
            let candidate = (across.abs() / along.abs(), ends_of(edge));
            let best = nearest
                .entry((key(node), axis, along > 0.))
                .or_insert(candidate);
            if candidate
                .0
                .total_cmp(&best.0)
                .then(candidate.1.cmp(&best.1))
                .is_lt()
            {
                *best = candidate;
            }
        }
    }
    nearest
}

/// Whether `edge` is the nearest line at both of its ends (a node `skip`
/// accepts counts as passed).
fn is_nearest(
    nearest: &HashMap<Side, (f64, Ends)>,
    edge: &Edge,
    axis: Axis,
    skip: impl Fn(Key) -> bool,
) -> bool {
    [(edge.start(), edge.end()), (edge.end(), edge.start())]
        .into_iter()
        .all(|(node, other)| {
            skip(key(node))
                || nearest
                    .get(&(key(node), axis, leg(axis, node, other).0 > 0.))
                    .is_some_and(|best| best.1 == ends_of(edge))
        })
}

/// Union-find over node keys.
struct Groups {
    parent: HashMap<Key, Key>,
}
impl Groups {
    fn find(&mut self, k: Key) -> Key {
        let p = *self.parent.entry(k).or_insert(k);
        if p == k {
            return k;
        }
        let root = self.find(p);
        self.parent.insert(k, root);
        root
    }
    fn unite(&mut self, a: Key, b: Key) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent.insert(ra, rb);
        }
    }
}

/// The document with its near-straight pieces made straight and its
/// near-axis lines snapped; `forced` names nodes whose two pieces are made
/// straight and snapped regardless of the tolerances.
pub fn straighten_svg(
    svg: &str,
    options: StraightenOptions,
    forced: &[Point],
) -> Result<(String, StraightenStats), String> {
    options.validate()?;
    let (ranges, mut paths) = parse_all_paths(svg)?;
    let forced: HashSet<Key> = forced.iter().map(|p| key(*p)).collect();
    let mut stats = StraightenStats::default();
    let touches_forced = |edge: &Edge| {
        !forced.is_empty()
            && (forced.contains(&key(edge.start())) || forced.contains(&key(edge.end())))
    };

    // Pass 1: curves that never leave the band around their chord.
    let flat = |edge: &Edge| {
        let chord = {
            let (a, b) = (edge.start(), edge.end());
            ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
        };
        let limit = if touches_forced(edge) {
            f64::INFINITY
        } else {
            options
                .flatness
                .max(chord * options.flatness * RELATIVE_BOW_PER_PX)
        };
        // A piece from a node back to itself has no chord (its bow is
        // infinite) and is never flat, forced or not: as a line it would
        // be a single point.
        let bent = bow(edge);
        bent.is_finite() && bent <= limit
    };
    // A flat piece that a smooth neighbour continues into one bend is part
    // of that bend, not a straight run: drawn as its chord, a gentle bend
    // or a small circle split into short pieces became a polyline with a
    // kink at every node (the two 8-degree pieces of a radius-100 bend bow
    // 0.33 px and kinked 8 degrees; a radius-10 circle's 36-degree pieces
    // bow 0.65 and made a decagon). Such a piece stays a curve where its
    // chord would kink visibly (`kinks_a_bend`). Judged on the pieces as
    // traced, before any becomes a line, from the canonical direction of
    // the piece, and only across nodes that are neither a junction nor a
    // kink (a turn of `VISIBLE_KINK` or more), whose two pieces are the
    // same for both fills that share it.
    let junctions = junction_keys(&paths);
    let visible = VISIBLE_KINK.to_radians().cos();
    let mut in_bend: HashSet<EdgeKey> = HashSet::new();
    for subpath in paths.iter().flatten() {
        let n = subpath.edges.len();
        let cyclic = subpath.cyclic();
        // Only the pieces drawn as their chord count: a D's flat side opposite
        // its arc is no lens, and becomes a line (round three of the Opus 5.5
        // review). A forced piece counts, as it becomes a line whatever its
        // bow (`flat` takes it as flat).
        let mut between: HashMap<Ends, usize> = HashMap::new();
        for edge in &subpath.edges {
            if edge.line || flat(edge) {
                *between.entry(ends_of(edge)).or_default() += 1;
            }
        }
        for (i, edge) in subpath.edges.iter().enumerate() {
            if edge.line || touches_forced(edge) || !flat(edge) {
                continue;
            }
            // Two pieces of one outline between the same two nodes (a thin
            // leaf, a crescent, a tapered stroke: M A C..B C..A Z) that would
            // both be drawn as their chord are one line drawn twice, and the
            // shape would vanish; its flat sides stay curves (round two of
            // the Opus 5.5 review).
            if between[&ends_of(edge)] > 1 {
                in_bend.insert(edge.key().0);
                continue;
            }
            // Smooth as the eye sees it: a node that already kinks has no
            // bend running through it for a line to break.
            let smooth = |a: &Edge, b: &Edge| {
                !junctions.contains(&key(b.start()))
                    && matches!((arriving(a), leaving(b)), (Some(t), Some(u)) if dot(t, u) > visible)
            };
            let before = (i > 0 || cyclic).then(|| subpath.edges[(i + n - 1) % n]);
            let after = (i + 1 < n || cyclic).then(|| subpath.edges[(i + 1) % n]);
            let mut stretch = Vec::with_capacity(3);
            stretch.extend(before.filter(|b| n > 2 && smooth(b, edge)));
            let mut own = stretch.len();
            stretch.push(*edge);
            stretch.extend(after.filter(|a| n > 1 && smooth(edge, a)));
            if stretch.len() < 2 {
                // No neighbour continues the bend (a junction or a slight
                // kink at both ends, as where regularize's arc meets its
                // neighbours within its 8-degree tangent guard): the piece
                // still stays a curve when its chord would turn a smooth
                // node by a visible kink more than the piece does.
                if lone_bend_kinks(edge, before.filter(|_| n > 1), after.filter(|_| n > 1)) {
                    in_bend.insert(edge.key().0);
                }
                continue;
            }
            let (edge_key, reversed) = edge.key();
            if reversed {
                stretch = stretch.iter().rev().map(Edge::reversed).collect();
                own = stretch.len() - 1 - own;
            }
            let drawn: Vec<Edge> = stretch
                .iter()
                .enumerate()
                .map(|(k, e)| {
                    if k != own && !e.line && flat(e) {
                        Edge::line(e.start(), e.end(), false)
                    } else {
                        *e
                    }
                })
                .collect();
            if is_bend(&stretch, options.flatness) && kinks_a_bend(&stretch, &drawn, own) {
                in_bend.insert(edge_key);
            }
        }
    }
    let mut made: HashSet<Ends> = HashSet::new();
    for subpath in paths.iter_mut().flatten() {
        for edge in subpath.edges.iter_mut() {
            if edge.line {
                continue;
            }
            if flat(edge) && !in_bend.contains(&edge.key().0) {
                let (a, b) = (edge.start(), edge.end());
                *edge = Edge::line(a, b, false);
                if made.insert(ends_of(edge)) {
                    stats.lines_made += 1;
                }
            }
        }
    }

    // At a forced node, two lines that leave it on the same side along one
    // axis form a tip; snapping both would fold the tip shut (a triangle
    // forced at its apex became a line). Only the line nearer that axis
    // takes the forced tolerances, the others the ordinary ones; lines that
    // leave on opposite sides run straight through the node and snap
    // together.
    let lines = || {
        paths
            .iter()
            .flatten()
            .flat_map(|s| s.edges.iter())
            .filter(|e| e.line)
    };
    let nearest = nearest_lines(lines().filter(|e| touches_forced(e)), FORCED_ANGLE, |k| {
        forced.contains(&k)
    });
    let nearest_at_forced =
        |edge: &Edge, axis: Axis| is_nearest(&nearest, edge, axis, |k| !forced.contains(&k));
    // The same holds at every node under the ordinary tolerances (round two
    // of the Opus 5.5 review): two near-axis lines that leave one node on
    // the same side along the axis are a thin spike (a sword tip, a serif,
    // a tapered stroke), and snapping both flattened it to nothing with no
    // chord reversed, so the fold check below never saw it. Only the line
    // nearer the axis snaps there.
    let tips = nearest_lines(lines(), options.angle, |_| true);
    let nearest_at_every_node = |edge: &Edge, axis: Axis| is_nearest(&tips, edge, axis, |_| false);

    // Pass 2: lines near an axis. Nodes joined by near-horizontal lines
    // share one y, nodes joined by near-vertical lines one x; a group whose
    // members would have to move further than allowed is left alone.
    let mut groups: HashMap<Axis, Groups> = HashMap::new();
    let mut allowed: HashMap<(Axis, Key), f64> = HashMap::new();
    let mut positions: HashMap<Key, Point> = HashMap::new();
    // Every piece's ends as they were, to check that no snap folds it.
    let mut chords: HashMap<Ends, (Point, Point)> = HashMap::new();
    let mut near: HashMap<Ends, Axis> = HashMap::new();
    for subpath in paths.iter().flatten() {
        for edge in &subpath.edges {
            let (a, b) = (edge.start(), edge.end());
            let ends = ends_of(edge);
            chords
                .entry(ends)
                .or_insert(if key(a) <= key(b) { (a, b) } else { (b, a) });
            if !edge.line {
                continue;
            }
            positions.insert(key(a), a);
            positions.insert(key(b), b);
            let forced_axis = if touches_forced(edge) {
                near_axis(edge, FORCED_ANGLE).filter(|axis| nearest_at_forced(edge, *axis))
            } else {
                None
            };
            let (axis, snap) = match forced_axis {
                Some(axis) => (Some(axis), FORCED_SNAP),
                None => (
                    near_axis(edge, options.angle)
                        .filter(|axis| nearest_at_every_node(edge, *axis)),
                    options.snap,
                ),
            };
            let Some(axis) = axis else {
                continue;
            };
            // A line already on its axis moves nothing but still ties its ends.
            let exact = match axis {
                Axis::Horizontal => a.y == b.y,
                Axis::Vertical => a.x == b.x,
            };
            if !exact {
                near.entry(ends).or_insert(axis);
            }
            let group = groups.entry(axis).or_insert_with(|| Groups {
                parent: HashMap::new(),
            });
            group.unite(key(a), key(b));
            for k in [key(a), key(b)] {
                let entry = allowed.entry((axis, k)).or_insert(snap);
                *entry = entry.max(snap);
            }
        }
    }
    let value = |axis: Axis, p: Point| match axis {
        Axis::Horizontal => p.y,
        Axis::Vertical => p.x,
    };
    // Each group that may move: its axis, its members in key order and the
    // coordinate they all take.
    let mut snaps: Vec<(Axis, Vec<Key>, f64)> = Vec::new();
    for (axis, group) in groups.iter_mut() {
        let axis = *axis;
        let keys: Vec<Key> = group.parent.keys().copied().collect();
        let mut members: HashMap<Key, Vec<Key>> = HashMap::new();
        for k in keys {
            let root = group.find(k);
            members.entry(root).or_default().push(k);
        }
        for mut members in members.into_values() {
            // The union-find's map hands the members out in a different
            // order in every process; summed in key order, the mean is the
            // same bits every run, and so is a mean that lands on a rounding
            // tie of the two decimals written. (Round two of the Opus 5.5
            // review tried the lines' traced positions weighted by length
            // instead, so that a side traced as one C-shaped piece keeps its
            // place: the unblended logo 4.297 -> 4.269 and the thin outline
            // 1.275 -> 1.224, but the blended logo 1.504 -> 1.512 and the
            // exact rounded rectangle's spread 0.58 -> 0.66 px; not kept,
            // testing/quality-round/straighten-mean.md.)
            members.sort_unstable();
            let target = members
                .iter()
                .map(|k| value(axis, positions[k]))
                .sum::<f64>()
                / members.len() as f64;
            // Pixel edges sit on whole pixels: take the whole pixel when the
            // group is within a third of one and every member may move that
            // far, otherwise the mean.
            let fits = |target: f64| {
                members
                    .iter()
                    .all(|k| (value(axis, positions[k]) - target).abs() <= allowed[&(axis, *k)])
            };
            let rounded = target.round();
            let target = if options.grid && (rounded - target).abs() <= GRID_PULL && fits(rounded) {
                rounded
            } else {
                target
            };
            if fits(target) {
                snaps.push((axis, members, target));
            }
        }
    }
    // A snap must not fold a piece: moving the ends of a short piece onto
    // one point, or past each other, draws it backwards or not at all.
    // Every group that moved an end of a folded piece stays where it was,
    // and the check runs again until nothing folds; each round leaves at
    // least one more group alone, so it ends.
    let mut groups_of: HashMap<Key, Vec<usize>> = HashMap::new();
    for (index, (_, members, _)) in snaps.iter().enumerate() {
        for k in members {
            groups_of.entry(*k).or_default().push(index);
        }
    }
    let mut active = vec![true; snaps.len()];
    let mut moves = loop {
        let mut moves: HashMap<Key, Point> = HashMap::new();
        for (axis, members, target) in snaps
            .iter()
            .zip(&active)
            .filter_map(|(snap, on)| on.then_some(snap))
        {
            for k in members {
                let p = moves.entry(*k).or_insert(positions[k]);
                match axis {
                    Axis::Horizontal => p.y = *target,
                    Axis::Vertical => p.x = *target,
                }
            }
        }
        let mut folded = false;
        for (ends, (a, b)) in &chords {
            let before = sub(*b, *a);
            let (a2, b2) = (
                moves.get(&ends.0).copied().unwrap_or(*a),
                moves.get(&ends.1).copied().unwrap_or(*b),
            );
            if dot(before, before) == 0. || dot(before, sub(b2, a2)) > 0. {
                continue;
            }
            for (k, from) in [(ends.0, *a), (ends.1, *b)] {
                for &index in groups_of.get(&k).into_iter().flatten() {
                    let (axis, target) = (snaps[index].0, snaps[index].2);
                    if active[index] && value(axis, from) != target {
                        active[index] = false;
                        folded = true;
                    }
                }
            }
        }
        if !folded {
            break moves;
        }
    };
    moves.retain(|k, p| key(*p) != *k);
    stats.nodes_moved = moves.len();
    // A line counts as snapped when it now lies exactly on its axis.
    let at = |k: &Key| moves.get(k).copied().unwrap_or(positions[k]);
    stats.edges_snapped = near
        .iter()
        .filter(|(ends, axis)| value(**axis, at(&ends.0)) == value(**axis, at(&ends.1)))
        .count();

    // Apply: every occurrence of a moved node, with a curve's neighbouring
    // handle carried along so the tangent there keeps its direction.
    if !moves.is_empty() {
        for subpath in paths.iter_mut().flatten() {
            for edge in subpath.edges.iter_mut() {
                let [p0, p1, p2, p3] = edge.cubic.points;
                let (mut q0, mut q1, mut q2, mut q3) = (p0, p1, p2, p3);
                if let Some(to) = moves.get(&key(p0)) {
                    let d = Point {
                        x: to.x - p0.x,
                        y: to.y - p0.y,
                    };
                    q0 = *to;
                    q1 = Point {
                        x: p1.x + d.x,
                        y: p1.y + d.y,
                    };
                }
                if let Some(to) = moves.get(&key(p3)) {
                    let d = Point {
                        x: to.x - p3.x,
                        y: to.y - p3.y,
                    };
                    q3 = *to;
                    q2 = Point {
                        x: p2.x + d.x,
                        y: p2.y + d.y,
                    };
                }
                if edge.line {
                    let implicit = edge.implicit;
                    *edge = Edge::line(q0, q3, implicit);
                } else {
                    edge.cubic.points = [q0, q1, q2, q3];
                }
            }
        }
    }
    Ok((splice(svg, &ranges, &paths), stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    const DOC: &str = "<svg width=\"100pt\" height=\"100pt\" viewBox=\"0 0 100 100\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"#000000ff\">\n<path fill=\"#000000\" opacity=\"1.00\" d=\" M 10.00 10.20 C 30.00 10.30 50.00 10.40 70.00 9.90 L 70.30 40.00 C 60.00 60.00 40.00 60.00 30.00 40.10 L 10.00 40.00 Z\" />\n</g>\n<g id=\"#ff0000ff\">\n<path fill=\"#ff0000\" opacity=\"1.00\" d=\" M 70.00 9.90 L 90.00 10.00 L 90.00 40.00 L 70.30 40.00 Z\" />\n</g>\n</svg>\n";

    #[test]
    fn flat_curves_become_lines_and_near_axis_lines_snap_on_both_sides() {
        let (out, stats) = straighten_svg(DOC, StraightenOptions::default(), &[]).unwrap();
        // The top curve bows 0.4 px: a line. The big bottom curve stays.
        assert_eq!(stats.lines_made, 1, "{out}");
        assert!(
            out.contains("M 10.00 10.00 L 70.00 10.00 L 70.00 40.00"),
            "{out}"
        );
        // The big bottom curve stays a curve; its handles follow the nodes
        // that snapped so the tangents keep their direction.
        assert!(
            out.contains(" C 59.70 60.00 40.00 59.90 30.00 40.00 L 10.00 40.00"),
            "{out}"
        );
        // The shared edge (70,10)-(70,40) moved identically in the red path.
        assert!(
            out.contains("M 70.00 10.00 L 90.00 10.00 L 90.00 40.00 L 70.00 40.00"),
            "{out}"
        );
        assert!(stats.nodes_moved >= 3 && stats.edges_snapped >= 3);
        // Off means off.
        let (same, stats) = straighten_svg(
            DOC,
            StraightenOptions {
                flatness: 0.,
                angle: 0.,
                snap: 0.,
                grid: true,
                auto: false,
            },
            &[],
        )
        .unwrap();
        assert_eq!(stats, StraightenStats::default());
        assert!(same.contains("C 30.00 10.30 50.00 10.40 70.00 9.90"));
    }

    #[test]
    fn a_forced_node_straightens_and_squares_its_two_pieces() {
        let options = StraightenOptions {
            flatness: 0.,
            angle: 0.,
            snap: 0.,
            grid: true,
            auto: false,
        };
        let forced = [Point { x: 30., y: 40.1 }];
        let (out, stats) = straighten_svg(DOC, options, &forced).unwrap();
        // The bowed curve into (30, 40.1) is now a line to (30, 40) and the
        // line on to (10, 40) is exactly horizontal.
        assert_eq!(stats.lines_made, 1);
        assert!(out.contains("L 30.00 40.00 L 10.00 40.00"), "{out}");
        assert!(out.contains("L 70.30 40.00 L 30.00 40.00"), "{out}");
        assert!(straighten_svg(
            DOC,
            StraightenOptions {
                angle: 50.,
                ..Default::default()
            },
            &[]
        )
        .is_err());
    }

    fn wrap(d: &str) -> String {
        format!("<svg width=\"120pt\" height=\"120pt\" viewBox=\"0 0 120 120\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"#000000ff\">\n<path fill=\"#000000\" opacity=\"1.00\" d=\"{d}\" />\n</g>\n</svg>\n")
    }

    const ONLY_FORCED: StraightenOptions = StraightenOptions {
        flatness: 0.,
        angle: 0.,
        snap: 0.,
        grid: true,
        auto: false,
    };

    #[test]
    fn a_snapped_mean_is_the_same_in_every_run() {
        // Eight nodes a hair off y = 20.5, joined by near-horizontal lines.
        // Their mean, summed in key (here x) order, writes as 20.49; in most
        // other orders it rounds to 20.50. Every call builds new hash maps
        // with new seeds, so without the fixed order the bytes varied.
        let ys = [
            "20.48", "20.48", "20.48", "20.53", "20.53", "20.50", "20.48", "20.48",
        ];
        let mut d = String::new();
        for (k, y) in ys.iter().enumerate() {
            let command = if k == 0 { "M" } else { "L" };
            d.push_str(&format!(" {command} {}.00 {y}", 10 + 10 * k));
        }
        d.push_str(" L 80.00 60.00 L 10.00 60.00 Z");
        let svg = wrap(&d);
        let (first, _) = straighten_svg(&svg, StraightenOptions::default(), &[]).unwrap();
        assert!(
            first.contains(" M 10.00 20.49 L 20.00 20.49 L 30.00 20.49"),
            "{first}"
        );
        assert!(first.contains(" L 80.00 20.49 L 80.00 60.00"), "{first}");
        for _ in 0..16 {
            let (again, _) = straighten_svg(&svg, StraightenOptions::default(), &[]).unwrap();
            assert_eq!(again, first);
        }
    }

    #[test]
    fn forcing_the_tip_of_a_triangle_squares_one_side_and_keeps_its_area() {
        // Both sides at the apex lean less than 20 degrees from vertical and
        // leave it downwards: snapping both put all three corners on one x
        // and the triangle vanished. Only the side nearer vertical (here a
        // tie, broken by the node keys) is squared.
        let svg = wrap(" M 10.00 10.00 L 20.00 10.00 L 15.00 40.00 Z");
        let (out, stats) = straighten_svg(&svg, ONLY_FORCED, &[Point { x: 15., y: 40. }]).unwrap();
        assert!(
            out.contains(" M 12.50 10.00 L 20.00 10.00 L 12.50 40.00 Z"),
            "{out}"
        );
        assert_eq!((stats.nodes_moved, stats.edges_snapped), (2, 1), "{out}");
        // A piece from a forced node back to itself has no chord: it stays a
        // curve instead of becoming a line of no length.
        let lobe = wrap(" M 30.00 30.00 C 50.00 10.00 50.00 50.00 30.00 30.00 Z");
        let (out, stats) = straighten_svg(&lobe, ONLY_FORCED, &[Point { x: 30., y: 30. }]).unwrap();
        assert_eq!(stats, StraightenStats::default(), "{out}");
        assert!(
            out.contains(" M 30.00 30.00 C 50.00 10.00 50.00 50.00 30.00 30.00 Z"),
            "{out}"
        );
    }

    #[test]
    fn a_thin_leaf_and_a_thin_spike_keep_their_area_under_the_defaults() {
        // Round two of the Opus 5.5 review. A 100 by 3 px leaf: each side
        // bows 1.5 px, flat by the relative limit, and as lines both sides
        // were the same chord drawn twice.
        let leaf = wrap(" M 10.00 60.00 C 40.00 58.50 80.00 58.50 110.00 60.00 C 80.00 61.50 40.00 61.50 10.00 60.00 Z");
        let (out, _) = straighten_svg(&leaf, StraightenOptions::default(), &[]).unwrap();
        assert_eq!(out.matches(" C ").count(), 2, "{out}");
        // A D, whose flat side (bowing 0.4 px) runs between the same two
        // nodes as its arc: no lens, as only one of them is drawn as its
        // chord, so the flat side becomes a line (round three).
        let d = wrap(" M 10.00 60.00 C 40.00 20.00 80.00 20.00 110.00 60.00 C 80.00 60.40 40.00 60.40 10.00 60.00 Z");
        let (out, stats) = straighten_svg(&d, StraightenOptions::default(), &[]).unwrap();
        assert_eq!(
            (stats.lines_made, out.matches(" C ").count()),
            (1, 1),
            "{out}"
        );
        // A 40 px spike whose two sides leave its tip along +x within 1.2
        // degrees of horizontal: all three nodes snapped to one y and it
        // vanished. Only the side nearer horizontal snaps at the tip.
        let spike =
            wrap(" M 0.00 30.00 L 40.00 30.80 L 40.00 50.00 L 60.00 50.00 L 60.00 10.00 L 40.00 10.00 L 40.00 29.30 Z");
        let (out, _) = straighten_svg(&spike, StraightenOptions::default(), &[]).unwrap();
        assert!(out.contains("L 40.00 30.80"), "{out}");
        assert!(
            !out.contains("L 40.00 29.30"),
            "the nearer side still snaps: {out}"
        );
    }

    #[test]
    fn a_lone_arc_between_slight_kinks_stays_a_curve() {
        // A 30-degree arc of radius 15 px (bow 0.68 px, flat at the default
        // 0.8) whose lines meet it at 5 degrees on both ends: as its chord,
        // each end would turn 20 degrees.
        let (r, k) = (15.0f64, 4. / 3. * 7.5f64.to_radians().tan() * 15.);
        let at = |deg: f64| {
            let a = deg.to_radians();
            (60. + r * a.cos(), 40. + r * a.sin(), -a.sin(), a.cos())
        };
        let turn = |x: f64, y: f64, deg: f64| {
            let (s, c) = deg.to_radians().sin_cos();
            (x * c - y * s, x * s + y * c)
        };
        let (x0, y0, tx0, ty0) = at(-105.);
        let (x3, y3, tx3, ty3) = at(-75.);
        let (ix, iy) = turn(tx0, ty0, 5.);
        let (ox, oy) = turn(tx3, ty3, -5.);
        let d = format!(
            " M {:.2} {:.2} L {x0:.2} {y0:.2} C {:.2} {:.2} {:.2} {:.2} {x3:.2} {y3:.2} L {:.2} {:.2} L {:.2} 80.00 L {:.2} 80.00 Z",
            x0 - 20. * ix,
            y0 - 20. * iy,
            x0 + k * tx0,
            y0 + k * ty0,
            x3 - k * tx3,
            y3 - k * ty3,
            x3 + 20. * ox,
            y3 + 20. * oy,
            x3 + 20. * ox,
            x0 - 20. * ix,
        );
        let options = StraightenOptions {
            angle: 0.,
            ..StraightenOptions::default()
        };
        let (out, stats) = straighten_svg(&wrap(&d), options, &[]).unwrap();
        assert_eq!(stats.lines_made, 0, "{out}");
        assert_eq!(out.matches(" C ").count(), 1, "{out}");
    }

    #[test]
    fn pieces_of_a_bend_stay_curves_and_wobble_on_a_straight_run_does_not() {
        // A circle of radius 10 in ten exact pieces: each bows 0.65 px, under
        // the default 0.8, and as lines they made a decagon. A 16-degree
        // bend of radius 100 in two pieces between corners: each bows 0.33
        // px, and as lines they kinked 8 degrees. Three pieces wobbling 0.2
        // px either side of y = 10 are a straight run and still become
        // lines.
        let circle = Point { x: 30., y: 80. };
        let step = std::f64::consts::TAU / 10.;
        let kappa = 4. / 3. * (step / 4.).tan() * 10.;
        let at = |a: f64| Point {
            x: circle.x + 10. * a.cos(),
            y: circle.y + 10. * a.sin(),
        };
        let mut ring = format!(" M {:.2} {:.2}", at(0.).x, at(0.).y);
        for k in 0..10 {
            let (a0, a1) = (step * k as f64, step * (k + 1) as f64);
            let (p0, p3) = (at(a0), at(a1));
            ring.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                p0.x - kappa * a0.sin(),
                p0.y + kappa * a0.cos(),
                p3.x + kappa * a1.sin(),
                p3.y - kappa * a1.cos(),
                p3.x,
                p3.y
            ));
        }
        let bend = Point { x: 60., y: 140. };
        let arc = |a: f64| Point {
            x: bend.x + 100. * a.cos(),
            y: bend.y + 100. * a.sin(),
        };
        let (a0, half) = (-FRAC_PI_2 - 0.14, 0.14f64);
        let k = 4. / 3. * (half / 4.).tan() * 100.;
        let mut gentle = String::new();
        for s in 0..2 {
            let (b0, b1) = (a0 + half * s as f64, a0 + half * (s + 1) as f64);
            let (p0, p3) = (arc(b0), arc(b1));
            gentle.push_str(&format!(
                " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                p0.x - k * b0.sin(),
                p0.y + k * b0.cos(),
                p3.x + k * b1.sin(),
                p3.y - k * b1.cos(),
                p3.x,
                p3.y
            ));
        }
        let start = arc(a0);
        let d = format!(
            " M 10.00 10.00 C 20.00 10.20 30.00 10.20 40.00 10.00 C 50.00 9.80 60.00 9.80 70.00 10.00 C 80.00 10.20 90.00 10.20 100.00 10.00 L 100.00 30.00 L 10.00 30.00 Z{ring} Z M {:.2} {:.2}{gentle} L 110.00 90.00 Z",
            start.x, start.y
        );
        let svg = wrap(&d);
        let (out, stats) = straighten_svg(&svg, StraightenOptions::default(), &[]).unwrap();
        assert_eq!(stats.lines_made, 3, "{out}");
        assert!(
            out.contains(" M 10.00 10.00 L 40.00 10.00 L 70.00 10.00 L 100.00 10.00"),
            "{out}"
        );
        assert_eq!(out.matches(" C ").count(), 12, "{out}");
    }

    #[test]
    fn anti_aliased_artwork_keeps_its_edges_between_pixels() {
        // An edge at y = 20.3 lands on the whole pixel for aliased artwork
        // and photographs, and stays where the anti-aliasing put it for
        // anti-aliased artwork.
        use crate::{basic_preset_code, ImageCategory, Quality};
        let svg = wrap(" M 10.00 20.30 L 60.00 20.30 L 60.00 60.00 L 10.00 60.00 Z");
        for (category, top) in [
            (ImageCategory::AliasedArtwork, "20.00"),
            (ImageCategory::Photograph, "20.00"),
            (ImageCategory::AntiAliasedArtwork, "20.30"),
        ] {
            for quality in [Quality::High, Quality::Low] {
                let options =
                    StraightenOptions::default().for_preset(basic_preset_code(category, quality));
                let (out, _) = straighten_svg(&svg, options, &[]).unwrap();
                assert!(
                    out.contains(&format!(" M 10.00 {top} L 60.00 {top} L 60.00 60.00")),
                    "{category:?}: {out}"
                );
            }
        }
    }

    #[test]
    fn anti_aliased_artwork_snaps_only_what_moves_a_quarter_pixel() {
        // A left edge slanting 0.55 px over 40 snaps upright for aliased
        // artwork (and lands on the whole pixel); for anti-aliased artwork
        // the snap would move both nodes 0.275 px, so the slant stays, while
        // a 0.4 px slant (0.2 px moves) still snaps.
        use crate::{basic_preset_code, ImageCategory, Quality};
        let options = |category| {
            StraightenOptions::default().for_preset(basic_preset_code(category, Quality::High))
        };
        let slanted = |bottom: &str| {
            wrap(&format!(
                " M 10.00 20.00 L 60.00 20.00 L 60.00 60.00 L {bottom} 60.00 Z"
            ))
        };
        let (aliased, _) = straighten_svg(
            &slanted("10.55"),
            options(ImageCategory::AliasedArtwork),
            &[],
        )
        .unwrap();
        assert!(aliased.contains(" L 10.00 60.00"), "{aliased}");
        let (kept, _) = straighten_svg(
            &slanted("10.55"),
            options(ImageCategory::AntiAliasedArtwork),
            &[],
        )
        .unwrap();
        assert!(
            kept.contains(" M 10.00 20.00 L 60.00 20.00 L 60.00 60.00 L 10.55 60.00"),
            "{kept}"
        );
        let (snapped, _) = straighten_svg(
            &slanted("10.40"),
            options(ImageCategory::AntiAliasedArtwork),
            &[],
        )
        .unwrap();
        assert!(!snapped.contains("10.40"), "{snapped}");
    }

    #[test]
    fn snaps_that_would_fold_a_short_line_stay_where_they_were() {
        // Two near-vertical lines end at the two ends of a 0.8 px horizontal
        // line. Each snaps within the default pixel, one to x = 11 and the
        // other to x = 10, which would draw the short line backwards: both
        // snaps are left out and the short line keeps its direction.
        let svg = wrap(" M 11.80 110.00 L 10.00 60.00 L 10.80 60.00 L 9.20 10.00 L 40.00 10.00 L 40.00 110.00 Z");
        let (out, stats) = straighten_svg(&svg, StraightenOptions::default(), &[]).unwrap();
        assert!(
            out.contains(" L 10.00 60.00 L 10.80 60.00 L 9.20 10.00"),
            "{out}"
        );
        assert_eq!((stats.nodes_moved, stats.edges_snapped), (0, 0), "{out}");
    }

    #[test]
    fn auto_takes_the_bow_of_the_picture_kind() {
        use crate::{basic_preset_code, ImageCategory, Quality};
        let auto = StraightenOptions {
            auto: true,
            ..Default::default()
        };
        for (category, bow) in [
            (ImageCategory::AntiAliasedArtwork, 0.2),
            (ImageCategory::AliasedArtwork, 0.65),
            (ImageCategory::Photograph, 1.2),
        ] {
            for quality in [Quality::High, Quality::Medium, Quality::Low] {
                let preset = basic_preset_code(category, quality);
                assert_eq!(auto.for_preset(preset).flatness, bow);
                // A bow given in pixels stays the one given.
                assert_eq!(
                    StraightenOptions::default().for_preset(preset).flatness,
                    0.8
                );
            }
        }
    }
}
