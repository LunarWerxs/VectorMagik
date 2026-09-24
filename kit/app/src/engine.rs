//! The conversion the application runs: the recovered Rust engine over the
//! loaded raster, then the owned post-processing the desktop offers. No
//! original code, binary or external process is involved.
use std::sync::Arc;
use std::time::Duration;
use vector_rebuild::clock::Stopwatch;
use vector_rebuild::raster::Raster;
use vector_rebuild::recovered_pipeline;
use vector_rebuild::{basic_preset_code, AdvancedSettings, ImageCategory, Quality};

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub category: ImageCategory,
    pub quality: Quality,
    pub overlap_opaque_photos: bool,
    /// Run the optional pass 0x49fc80 (BezierFitter+0x1c, off in every
    /// preset).
    pub optional_optimizer: bool,
    /// The original's advanced mode: its sliders replace the basic preset
    /// (`vector_rebuild::advanced_preset`); the category still decides the
    /// photo seam treatment.
    pub advanced: Option<AdvancedSettings>,
    /// The improved defaults at high quality, unless `advanced` says
    /// otherwise: photographs trace at the original's advanced-mode detail
    /// ceiling (`photo_detail_ceiling`; owner-accepted September 22, 2026
    /// from the measured sheets, astronaut colour error 7.49 -> 6.19,
    /// similarity 0.845 -> 0.895), and blended artwork with the original's
    /// advanced mode at segmentation detail 11 and smoothness 3
    /// (`blended_defaults`, 5,4,6 before September 23, 2026; September 22,
    /// 2026: the basic preset pinches the S's thin outline to a wedge where
    /// it meets the band, the owner's "grey incursion", and any smoothness
    /// under 6 traces it whole, while 6 rounds off a sharp inner corner of
    /// the gear that 4 keeps; colour error 1.47 -> 1.45 on the blended logo,
    /// 1.63 -> 1.65 on the gear, 0.74 -> 0.73 on the small logo, files 4 to
    /// 24% smaller). Also, at every quality: the optional pass keeps the
    /// plain fit when its step fails or raises its objective, and a
    /// translucent region is filled with its straight colour instead of the
    /// premultiplied bytes the original writes with its opacity
    /// (`recovered_pipeline::Options::straight_fills`). Off keeps the
    /// original's presets 5 and 8 byte for byte.
    pub owned_defaults: bool,
    /// The basic preset even where the improved defaults would run the
    /// advanced mode (CLI `--advanced preset`), so the preset can be
    /// measured against them with every other improvement kept.
    pub basic_preset: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            category: ImageCategory::AntiAliasedArtwork,
            quality: Quality::High,
            overlap_opaque_photos: true,
            optional_optimizer: false,
            advanced: None,
            owned_defaults: true,
            basic_preset: false,
        }
    }
}

/// The photo default: the original's advanced mode with segmentation
/// complexity at its ceiling of 12, contour smoothness and curve complexity
/// at its defaults of 6, corner detection on, anti-aliasing off as in every
/// photo preset (`--advanced 12,6,6` on a photograph).
pub fn photo_detail_ceiling() -> AdvancedSettings {
    AdvancedSettings {
        segmentation_complexity: 12,
        contour_smoothness: 6,
        curve_complexity: 6,
        detect_corners: true,
        contour_anti_alias: false,
        anti_alias_rejection: 2,
        min_pixels: 0,
        ..AdvancedSettings::default()
    }
}

/// The blended-artwork default: the original's advanced mode at segmentation
/// detail 11 and smoothness 3, curves at the dialog's 6, corner detection and
/// anti-aliasing on (`--advanced 11,3,6` on blended artwork). 5,4,6 until
/// September 23, 2026, when lettering, thin rings and strokes, near colours
/// and pies joined the samples: its segmentation merged 22 times harder than
/// the basic preset's, broke thin rings and strokes and dropped small
/// letters. The rule picked 10,4,6, which misses the engine-quality gate on
/// the transparency logo by 0.015; 11,3,6 is the best that passes every gate
/// (testing/quality-round/blended-final.md: -1.36 against 5,4,6, thin rings'
/// overlap 0.16 -> 0.71, thin strokes' 0.83 -> 0.98), and keeps the S whole
/// and the gear sharp that the basic preset and detail 12 lose.
pub fn blended_defaults() -> AdvancedSettings {
    AdvancedSettings {
        segmentation_complexity: 11,
        contour_smoothness: 3,
        curve_complexity: 6,
        detect_corners: true,
        contour_anti_alias: true,
        anti_alias_rejection: 0,
        min_pixels: 0,
        ..AdvancedSettings::default()
    }
}

/// The settings a conversion under `options` runs: the explicit advanced
/// settings, else the improved default of the image type at high quality,
/// else none (the basic preset).
pub fn effective_advanced(options: &Options) -> Option<AdvancedSettings> {
    options.advanced.or_else(|| {
        if !options.owned_defaults || options.basic_preset || options.quality != Quality::High {
            return None;
        }
        match options.category {
            ImageCategory::Photograph => Some(photo_detail_ceiling()),
            ImageCategory::AntiAliasedArtwork => Some(blended_defaults()),
            ImageCategory::AliasedArtwork => None,
        }
    })
}

/// The three sliders of the original's advanced dialog and its corner
/// detection, as the desktop's Advanced card holds them; anti-aliasing
/// follows the image type and the minimum region size stays 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sliders {
    pub segmentation: i32,
    pub smoothness: i32,
    pub curves: i32,
    pub corners: bool,
}
impl Default for Sliders {
    /// The original dialog's defaults.
    fn default() -> Self {
        Self {
            segmentation: 5,
            smoothness: 6,
            curves: 6,
            corners: true,
        }
    }
}
impl Sliders {
    /// The sliders behind a setting: the dialog's when there is none.
    pub fn of(settings: Option<AdvancedSettings>) -> Self {
        match settings {
            Some(s) => Self {
                segmentation: s.segmentation_complexity,
                smoothness: s.contour_smoothness,
                curves: s.curve_complexity,
                corners: s.detect_corners,
            },
            None => Self::default(),
        }
    }
}

/// The advanced settings the sliders make for an image type: anti-aliasing
/// on for blended artwork, off otherwise, as the original's dialog defaulted.
pub fn advanced_settings(
    category: ImageCategory,
    sliders: Sliders,
) -> Result<AdvancedSettings, String> {
    let contour_anti_alias = category == ImageCategory::AntiAliasedArtwork;
    let settings = AdvancedSettings {
        segmentation_complexity: sliders.segmentation,
        contour_smoothness: sliders.smoothness,
        curve_complexity: sliders.curves,
        detect_corners: sliders.corners,
        contour_anti_alias,
        anti_alias_rejection: if contour_anti_alias { 0 } else { 2 },
        min_pixels: 0,
        ..AdvancedSettings::default()
    };
    settings.validate()?;
    Ok(settings)
}

/// `SEG,SMOOTH,CURVE[,corners=on|off][,aa=on|off][,minpix=N]`: the three
/// 1..12 sliders of the original's advanced dialog, its corner detection,
/// its anti-aliasing (default: on for blended artwork) and its minimum
/// region size in pixels.
pub fn parse_advanced(spec: &str, category: ImageCategory) -> Result<AdvancedSettings, String> {
    const USAGE: &str = "--advanced takes SEG,SMOOTH,CURVE (each 1..12) then optional corners=on|off, aa=on|off, minpix=N";
    let (sliders, rest) = slider_spec(spec, USAGE)?;
    let mut settings = AdvancedSettings {
        segmentation_complexity: sliders.segmentation,
        contour_smoothness: sliders.smoothness,
        curve_complexity: sliders.curves,
        detect_corners: sliders.corners,
        contour_anti_alias: category == ImageCategory::AntiAliasedArtwork,
        ..AdvancedSettings::default()
    };
    for (key, value) in rest {
        match key {
            "aa" => settings.contour_anti_alias = on_off(key, value, USAGE)?,
            "minpix" => {
                settings.min_pixels = value
                    .parse::<i32>()
                    .ok()
                    .filter(|n| *n >= 0)
                    .ok_or(format!("{USAGE}: minpix must be a non-negative integer"))?
            }
            _ => return Err(USAGE.into()),
        }
    }
    settings.anti_alias_rejection = if settings.contour_anti_alias { 0 } else { 2 };
    settings.validate()?;
    Ok(settings)
}

/// `SEG,SMOOTH,CURVE[,corners=on|off]`: the desktop Advanced card's sliders
/// as the preview command line takes them, in `parse_advanced`'s grammar.
/// The card's anti-aliasing and minimum region size follow the image type
/// (`advanced_settings`), so `aa=` and `minpix=` are refused, not dropped.
pub fn parse_sliders(spec: &str) -> Result<Sliders, String> {
    const USAGE: &str =
        "--advanced takes SEG,SMOOTH,CURVE (each 1..12) then optional corners=on|off";
    let (sliders, rest) = slider_spec(spec, USAGE)?;
    match rest.first() {
        None => Ok(sliders),
        Some(&(key @ ("aa" | "minpix"), _)) => Err(format!(
            "{USAGE}; the preview's {key} follows the image type, as in the desktop"
        )),
        Some(_) => Err(USAGE.into()),
    }
}

/// `key=value` settings after the sliders, trimmed, in order.
type Settings<'a> = Vec<(&'a str, &'a str)>;

/// The grammar both advanced parsers share: three 1..12 sliders, then
/// `key=value` settings, every part trimmed. Reads `corners=` and returns
/// the other settings in order for the caller.
fn slider_spec<'a>(spec: &'a str, usage: &str) -> Result<(Sliders, Settings<'a>), String> {
    let mut parts = spec.split(',');
    let mut slider = |name: &str| -> Result<i32, String> {
        parts
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok())
            .filter(|v| (1..=12).contains(v))
            .ok_or_else(|| format!("{usage}: {name} missing or outside 1..12"))
    };
    let mut sliders = Sliders {
        segmentation: slider("segmentation complexity")?,
        smoothness: slider("contour smoothness")?,
        curves: slider("curve complexity")?,
        corners: AdvancedSettings::default().detect_corners,
    };
    let mut rest = Vec::new();
    for part in parts {
        let (key, value) = part.split_once('=').ok_or_else(|| usage.to_owned())?;
        let (key, value) = (key.trim(), value.trim());
        if key == "corners" {
            sliders.corners = on_off(key, value, usage)?;
        } else {
            rest.push((key, value));
        }
    }
    Ok((sliders, rest))
}

fn on_off(key: &str, value: &str, usage: &str) -> Result<bool, String> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{usage}: {key} must be on or off")),
    }
}

/// The advanced settings as the statistics print them.
pub fn describe_advanced(s: &AdvancedSettings) -> String {
    format!(
        "{},{},{},corners={},aa={},minpix={}",
        s.segmentation_complexity,
        s.contour_smoothness,
        s.curve_complexity,
        if s.detect_corners { "on" } else { "off" },
        if s.contour_anti_alias { "on" } else { "off" },
        s.min_pixels
    )
}

#[derive(Clone, Debug)]
pub struct Document {
    pub svg: String,
    pub width: usize,
    pub height: usize,
    pub preset: usize,
    /// Set when the conversion ran the original's advanced mode instead of
    /// the basic preset.
    pub advanced: Option<AdvancedSettings>,
    pub elapsed: Duration,
    pub photo_overlap: bool,
    /// Set when `simplified` rewrote the curves; the engine's own output has none.
    pub simplify_tolerance: Option<f64>,
    pub optional_optimizer: bool,
    /// What the stages reported: regions segmented, nodes and curves fitted,
    /// corners punctured, unknowns of the optional pass (0 when off).
    pub regions: usize,
    pub nodes: usize,
    pub curves: usize,
    pub corners: usize,
    pub optimizer_unknowns: usize,
    /// Whether the optional pass's step stands (false when it is off, when it
    /// failed, or when the improved defaults' guard put the plain fit back
    /// because the step raised the objective), and the objective before and
    /// after the step when the pass ran.
    pub optimizer_kept: bool,
    pub optimizer_objective: Option<[f64; 2]>,
    /// Wall-clock seconds per engine stage.
    pub stages: recovered_pipeline::StageSeconds,
    /// The picture the engine traced, for the passes that judge the drawing
    /// against it (`primitives`); shared, since documents are cloned per
    /// derived view.
    pub source: Option<Arc<Raster>>,
    /// The document before its hand-rounded corners (`smoothed`), so a
    /// rounded corner being dragged can show the outline it will round again.
    pub unrounded: Option<Arc<String>>,
}
impl Document {
    pub fn svg(&self) -> &str {
        &self.svg
    }
    /// The same document with neighbouring curve pieces merged wherever one
    /// cubic stays within `tolerance` source pixels of the engine's fit. Owned
    /// post-processing; the engine's output is left untouched in `self`.
    pub fn simplified(&self, tolerance: f64) -> Result<Self, String> {
        self.simplified_with(tolerance, false)
    }
    /// `simplified` for a tolerance chosen by hand (the Simplify slider, the
    /// command line's `--simplify N`): the kinks the merges keep are
    /// smoothed within it too (`vector_rebuild::simplify::smooth_kinks`).
    pub fn simplified_by_hand(&self, tolerance: f64) -> Result<Self, String> {
        self.simplified_with(tolerance, true)
    }
    fn simplified_with(&self, tolerance: f64, smooth_kinks: bool) -> Result<Self, String> {
        let (svg, _) = vector_rebuild::simplify::simplify_svg(
            &self.svg,
            vector_rebuild::simplify::SimplifyOptions {
                tolerance,
                smooth_kinks,
            },
        )?;
        Ok(Self {
            svg,
            simplify_tolerance: Some(tolerance),
            ..self.clone()
        })
    }
    /// The same document with the given anchor nodes made smooth: the two
    /// curve pieces meeting at each node are refitted with one shared tangent,
    /// so a corner becomes a round join through the same point. Each node
    /// carries its own reach, the fraction of each neighbouring piece allowed
    /// to change (1 = whole piece). Nodes not in the document, and junctions
    /// of three or more boundaries, are ignored.
    pub fn smoothed(&self, nodes: &[vector_rebuild::simplify::Rounding]) -> Result<Self, String> {
        let (svg, _) = vector_rebuild::simplify::smooth_nodes(&self.svg, nodes)?;
        Ok(Self {
            svg,
            unrounded: Some(Arc::new(self.svg.clone())),
            ..self.clone()
        })
    }
    /// The same document with the listed nodes moved by hand, each with its
    /// handles, in every outline through it (`vector_rebuild::nodes`); a
    /// node no longer in the document is skipped.
    pub fn moved(&self, moves: &[vector_rebuild::nodes::NodeMove]) -> Result<Self, String> {
        if moves.is_empty() {
            return Ok(self.clone());
        }
        let (svg, _) = vector_rebuild::nodes::move_nodes(&self.svg, moves)?;
        Ok(Self {
            svg,
            ..self.clone()
        })
    }
    /// The same document with the listed nodes deleted by hand, their two
    /// pieces joined into one in every outline through them
    /// (`vector_rebuild::nodes::delete_nodes`); a node no longer in the
    /// document, or no longer deletable, is skipped.
    pub fn without_nodes(
        &self,
        deletions: &[vector_rebuild::nodes::NodeDeletion],
    ) -> Result<Self, String> {
        if deletions.is_empty() {
            return Ok(self.clone());
        }
        let (svg, _) = vector_rebuild::nodes::delete_nodes(&self.svg, deletions)?;
        Ok(Self {
            svg,
            ..self.clone()
        })
    }
    /// The same document with every run of pieces along one line drawn as
    /// that line and every run on one circle drawn as its arcs. Owned
    /// post-processing like `simplified`, run before straightening.
    pub fn regularized(
        &self,
        options: vector_rebuild::regularize::RegularizeOptions,
    ) -> Result<Self, String> {
        // Arcs are held against the source's pixels (a letter bowl is no
        // circle), on artwork: photographs keep the plain pass.
        use vector_rebuild::primitives::PrimitiveOptions;
        use vector_rebuild::regularize::{regularize_svg_checked, PixelCheck};
        let check = match (&self.source, PrimitiveOptions::for_preset(self.preset)) {
            (Some(source), Some(kind)) => Some(PixelCheck {
                source: source.as_ref(),
                anti_aliased: kind.anti_aliased,
            }),
            _ => None,
        };
        let (svg, _) = regularize_svg_checked(&self.svg, options, check)?;
        Ok(Self {
            svg,
            ..self.clone()
        })
    }
    /// The same document with its near-straight pieces made straight and its
    /// near-axis lines snapped to the axis; `forced` nodes get both whatever
    /// the tolerances say. Owned post-processing like `simplified`.
    pub fn straightened(
        &self,
        options: vector_rebuild::straighten::StraightenOptions,
        forced: &[vector_rebuild::geometry::Point],
    ) -> Result<Self, String> {
        let (svg, _) = vector_rebuild::straighten::straighten_svg(&self.svg, options, forced)?;
        Ok(Self {
            svg,
            ..self.clone()
        })
    }
    /// The same document with every closed outline the source's pixels show
    /// to be a circle, an ellipse, a rectangle or a rounded rectangle drawn
    /// as that shape (`vector_rebuild::primitives`); unchanged for
    /// photographs and for a document without its source. `forced` nodes
    /// keep their outlines.
    pub fn refitted(&self, forced: &[vector_rebuild::geometry::Point]) -> Result<Self, String> {
        use vector_rebuild::primitives::{refit_svg, PrimitiveOptions};
        let (Some(source), Some(options)) =
            (&self.source, PrimitiveOptions::for_preset(self.preset))
        else {
            return Ok(self.clone());
        };
        let (svg, _) = refit_svg(&self.svg, source, options, forced)?;
        Ok(Self {
            svg,
            ..self.clone()
        })
    }
    /// The owned passes after simplifying, in the desktop's order: true
    /// lines and circles, then straightening with its options fitted to the
    /// document's preset (`forced` nodes straightened and squared whatever
    /// the tolerances), then, when `primitives`, the shapes the pixels show
    /// (last, so straightening's bow cannot flatten a small rounded corner's
    /// arcs). The desktop's derive (`Desktop::finish`) and the CLI's
    /// `--regularize` / `--straighten` / `--primitives` all run this one
    /// function, so the CLI reproduces the desktop's chain by construction,
    /// not by a copy (round two of the Opus 5.5 review).
    pub fn post_passes(
        &self,
        regularize: Option<vector_rebuild::regularize::RegularizeOptions>,
        straighten: Option<vector_rebuild::straighten::StraightenOptions>,
        primitives: bool,
        forced: &[vector_rebuild::geometry::Point],
    ) -> Result<Self, String> {
        let regularized = match regularize {
            Some(options) => self.regularized(options)?,
            None => self.clone(),
        };
        let straightened = match straighten {
            Some(options) => regularized.straightened(options.for_preset(self.preset), forced)?,
            None => regularized,
        };
        if primitives {
            straightened.refitted(forced)
        } else {
            Ok(straightened)
        }
    }
    /// The same document without the listed regions (each an outer outline
    /// with its holes, found by colour and a point inside it).
    pub fn without_islands(
        &self,
        removals: &[vector_rebuild::shapes::Removal],
    ) -> Result<Self, String> {
        if removals.is_empty() {
            return Ok(self.clone());
        }
        let (svg, _) = vector_rebuild::shapes::remove_islands(&self.svg, removals)?;
        Ok(Self {
            svg,
            ..self.clone()
        })
    }
    /// The same document with its fills snapped to `palette`: every fill, or
    /// with `within` only those that many levels or closer to a palette colour.
    pub fn snapped(&self, palette: &[[u8; 3]], within: Option<u8>) -> Self {
        Self {
            svg: vector_rebuild::prepare::snap_fills(&self.svg, palette, within),
            ..self.clone()
        }
    }
    pub fn color_count(&self) -> usize {
        self.colors().len()
    }
    /// Every fill color in the document with how many paths use it, most used
    /// first; ties keep the document's order.
    pub fn colors(&self) -> Vec<(String, usize)> {
        let mut seen: Vec<(String, usize)> = Vec::new();
        // Where each fill sits in `seen`, so a photograph's thousands of
        // paths and colours cost one lookup each rather than a scan.
        let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for fill in self
            .svg
            .split("fill=\"")
            .skip(1)
            .filter_map(|s| s.split('"').next())
        {
            let at = *index.entry(fill).or_insert_with(|| {
                seen.push((fill.to_owned(), 0));
                seen.len() - 1
            });
            seen[at].1 += 1;
        }
        seen.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        seen
    }
    pub fn nodes(&self) -> Vec<vector_rebuild::geometry::Point> {
        use vector_rebuild::geometry::Point;
        let mut nodes = Vec::new();
        for path in self.svg.split(" d=\"").skip(1) {
            let mut base = nodes.len();
            let mut tokens = path.split('"').next().unwrap_or("").split_whitespace();
            while let Some(command) = tokens.next() {
                if command == "M" {
                    // The engine writes a shape and its holes as one path
                    // with a single Z at the end (M a .. a M b .. b Z): a
                    // loop that returns to its start before the next M
                    // closes there too, so its repeated start is not a node.
                    if nodes.len() > base + 1 && nodes.last() == nodes.get(base) {
                        nodes.pop();
                    }
                    base = nodes.len();
                }
                let count = match command {
                    "M" | "L" => 2,
                    "C" => 6,
                    "Z" => {
                        if nodes.len() > base + 1 && nodes.last() == nodes.get(base) {
                            nodes.pop();
                        }
                        0
                    }
                    _ => break,
                };
                let values: Vec<f64> = tokens
                    .by_ref()
                    .take(count)
                    .filter_map(|n| n.parse().ok())
                    .collect();
                if count > 0 && values.len() == count {
                    nodes.push(Point {
                        x: values[count - 2],
                        y: values[count - 1],
                    });
                }
            }
        }
        nodes
    }
    pub fn segment_count(&self) -> usize {
        self.svg
            .split(" d=\"")
            .skip(1)
            .map(|s| {
                s.split('"')
                    .next()
                    .unwrap_or("")
                    .bytes()
                    .filter(|b| matches!(b, b'L' | b'C' | b'Q' | b'A'))
                    .count()
            })
            .sum()
    }
    pub fn statistics_json(&self) -> String {
        format!(
            "{{\"backend\":\"recovered-rust\",\"width\":{},\"height\":{},\"preset\":{},\"segments\":{},\"seconds\":{:.6},\"photo_overlap\":{},\"regions\":{},\"nodes\":{},\"curves\":{},\"corners\":{},\"optimizer_unknowns\":{},\"optional_optimizer\":{},\"stages\":{{\"preprocess\":{:.6},\"segment\":{:.6},\"topology\":{:.6},\"smooth\":{:.6},\"fit\":{:.6},\"export\":{:.6}}},\"original_code\":false{}{}{}}}\n",
            self.width,
            self.height,
            self.preset,
            self.segment_count(),
            self.elapsed.as_secs_f64(),
            self.photo_overlap,
            self.regions,
            self.nodes,
            self.curves,
            self.corners,
            self.optimizer_unknowns,
            self.optional_optimizer,
            self.stages.preprocess,
            self.stages.segment,
            self.stages.topology,
            self.stages.smooth,
            self.stages.fit,
            self.stages.export,
            self.advanced
                .as_ref()
                .map(|a| format!(",\"advanced\":\"{}\"", describe_advanced(a)))
                .unwrap_or_default(),
            self.simplify_tolerance
                .map(|t| format!(",\"simplify\":{t}"))
                .unwrap_or_default(),
            self.optimizer_objective
                .map(|[before, after]| format!(
                    ",\"optimizer_kept\":{},\"optimizer_objective\":[{before:e},{after:e}]",
                    self.optimizer_kept
                ))
                .unwrap_or_default()
        )
    }
}

/// The layering and stroking the application always exported with
/// (export+0x2c = 2: hole loops in colour groups; export+0x34 = 1: no
/// stroke).
const LAYERING: i32 = 2;
const STROKING: i32 = 1;

/// The picture sides, in pixels, the engine traces.
pub const SIDE_LIMITS: std::ops::RangeInclusive<usize> = 2..=4096;

/// Pixel art scaled up by at least this whole number is traced as its
/// squares; below it a drawing's runs can be even by chance.
const MIN_UPSCALE: usize = 3;

/// A pixel-edged picture with no more colours than this has its colours as
/// its palette (`exact_palette`).
const MAX_EXACT_PALETTE: usize = 64;

/// The colours of a pixel-edged picture when they are few enough to be its
/// palette: every pixel fully opaque or fully transparent, and at most
/// `MAX_EXACT_PALETTE` colours among the opaque ones. The engine fills each
/// region with its pixels' mean, so a hairline merged with its background
/// comes back grey: two-colour pixel text traced with up to 13 greys, 91
/// levels off (the defect sweep of September 23, 2026); under the improved
/// defaults a traced aliased picture's fills snap to these.
fn exact_palette(raster: &Raster) -> Option<Vec<[u8; 3]>> {
    let mut seen = std::collections::HashSet::new();
    for pixel in &raster.pixels {
        match pixel.0[3] {
            0 => {}
            255 => {
                seen.insert([pixel.0[0], pixel.0[1], pixel.0[2]]);
                if seen.len() > MAX_EXACT_PALETTE {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let mut palette: Vec<[u8; 3]> = seen.into_iter().collect();
    palette.sort_unstable();
    (!palette.is_empty()).then_some(palette)
}

/// Converts `raster` under the basic preset for `options`.
pub fn vectorize(raster: &Raster, options: Options) -> Result<Document, String> {
    vectorize_with(raster, options, &[])
}

/// A research override of one engine parameter: its key in the preset map
/// and the values it takes (CLI `--set KEY=V[,V...]`).
pub type Override = (String, Vec<f64>);

/// `vectorize` with engine parameters overridden after the preset or the
/// advanced mode chose them, each key one the map has, with as many values;
/// for measuring a candidate setting without a build per try.
pub fn vectorize_with(
    raster: &Raster,
    options: Options,
    overrides: &[Override],
) -> Result<Document, String> {
    if !SIDE_LIMITS.contains(&raster.width)
        || !SIDE_LIMITS.contains(&raster.height)
        || raster.width * raster.height != raster.pixels.len()
        || raster.pixels.len() > 16_000_000
    {
        return Err("Input must be 2..4096 pixels per side, at most 16 million pixels, with complete RGBA data".into());
    }
    let started = Stopwatch::start();
    let preset = basic_preset_code(options.category, options.quality);
    let advanced = effective_advanced(&options);
    let mut parameters = match &advanced {
        Some(settings) => vector_rebuild::advanced_preset(settings)?,
        None => vector_rebuild::preset(preset)?,
    };
    for (key, values) in overrides {
        match parameters.get_mut(key) {
            Some(slot) if slot.len() == values.len() => slot.clone_from(values),
            Some(slot) => {
                return Err(format!(
                    "{key} takes {} values, not {}",
                    slot.len(),
                    values.len()
                ))
            }
            None => return Err(format!("No engine parameter is called {key}")),
        }
    }
    let mut bgra = Vec::with_capacity(raster.pixels.len() * 4);
    for p in &raster.pixels {
        bgra.extend_from_slice(&[p.0[2], p.0[1], p.0[0], p.0[3]]);
    }
    // Pixel-edged artwork of a few exact colours: its fills its colours (the
    // defect sweep of September 23, 2026).
    let palette = if options.owned_defaults && options.category == ImageCategory::AliasedArtwork {
        exact_palette(raster)
    } else {
        None
    };
    // Pixel art scaled up by a whole number is its squares, exactly
    // (`vector_rebuild::pixel_art`).
    if palette.is_some() {
        let rgba: Vec<[u8; 4]> = raster.pixels.iter().map(|p| p.0).collect();
        let (w, h) = (raster.width, raster.height);
        if let Some(k) = vector_rebuild::pixel_art::upscale_factor(&rgba, w, h, MIN_UPSCALE) {
            let small: Vec<[u8; 4]> = (0..h / k)
                .flat_map(|y| (0..w / k).map(move |x| (y, x)))
                .map(|(y, x)| rgba[y * k * w + x * k])
                .collect();
            let (svg, regions, nodes) = vector_rebuild::pixel_art::trace(&small, w / k, h / k, k);
            return Ok(Document {
                svg,
                width: w,
                height: h,
                preset,
                advanced,
                elapsed: started.elapsed(),
                photo_overlap: false,
                simplify_tolerance: None,
                optional_optimizer: false,
                regions,
                nodes,
                curves: 0,
                corners: nodes,
                optimizer_unknowns: 0,
                optimizer_kept: false,
                optimizer_objective: None,
                stages: recovered_pipeline::StageSeconds::default(),
                // Exact already: no true shapes refitted from the pixels (a
                // 4x sprite's stepped round took curves 0.39 px off it).
                source: None,
                unrounded: None,
            });
        }
    }
    let conversion = recovered_pipeline::vectorize(
        &bgra,
        raster.width,
        raster.height,
        &parameters,
        &recovered_pipeline::Options {
            layering: LAYERING,
            stroking: STROKING,
            optional_optimizer: options.optional_optimizer,
            guard_optimizer: options.owned_defaults,
            straight_fills: options.owned_defaults,
        },
    )?;
    let mut svg = conversion.document.svg;
    let view_box = format!("viewBox=\"0 0 {} {}\"", raster.width, raster.height);
    if !svg.contains(&view_box)
        || (!svg.contains("<path ") && raster.pixels.iter().any(|p| p.0[3] != 0))
        || !svg.trim_end().ends_with("</svg>")
    {
        return Err(
            "The engine did not produce a complete vector document with matching dimensions".into(),
        );
    }
    // Regions the segmentation merged away, drawn back from the pixels, on
    // artwork under the improved defaults (`vector_rebuild::recovery`),
    // before the palette snap so a recovered fill is snapped too.
    let mut regions = conversion.regions;
    if options.owned_defaults && options.category != ImageCategory::Photograph {
        let (recovered, stats) = vector_rebuild::recovery::recover_svg(
            &svg,
            raster,
            options.category == ImageCategory::AntiAliasedArtwork,
        )?;
        svg = recovered;
        regions += stats.recovered;
        // Thin strokes drawn in their ink at their width (anti-aliased
        // artwork: pixel-edged strokes are their palette's colour already).
        if options.category == ImageCategory::AntiAliasedArtwork {
            svg = vector_rebuild::strokes::ink_strokes(&svg, raster)?.0;
        } else {
            // Pixel-edged strokes keep square ends (`vector_rebuild::caps`).
            svg = vector_rebuild::caps::square_caps(&svg)?.0;
        }
    }
    if let Some(palette) = &palette {
        svg = vector_rebuild::prepare::snap_fills(&svg, palette, None);
        // Each region in the colour of the pixels under it where the nearest
        // colour is a blend's (`vector_rebuild::palette_fills`).
        svg = vector_rebuild::palette_fills::fills_from_pixels(&svg, raster)?.0;
        // Shapes of long straight pixel runs drawn on their pixels, now that
        // every fill is one of the pixels' colours
        // (`vector_rebuild::rectilinear`).
        svg = vector_rebuild::rectilinear::refit_rectilinear(&svg, raster)?.0;
        // Blobs of one colour still lost, however small (a dotted line's
        // dots), on their pixel edges (`vector_rebuild::recovery`).
        let (recovered, stats) = vector_rebuild::recovery::recover_exact(&svg, raster)?;
        svg = recovered;
        regions += stats.recovered;
    }
    // Each region filled with the median of the pixels it covers once
    // traced (`vector_rebuild::median_fills`): every region of a
    // photograph, on artwork without an exact palette the regions 3 px
    // wide or more (a thin stroke's pixels are mostly its blended rim).
    if options.owned_defaults && palette.is_none() {
        let min_width = if options.category == ImageCategory::Photograph {
            0.
        } else {
            3.
        };
        svg = vector_rebuild::median_fills::median_fills(&svg, raster, min_width)?.0;
    }
    let photo_overlap = options.overlap_opaque_photos
        && options.category == ImageCategory::Photograph
        && raster.pixels.iter().all(|p| p.0[3] == 255);
    if photo_overlap {
        svg = opaque_photo_export(&svg, raster)?;
    }
    Ok(Document {
        svg,
        width: raster.width,
        height: raster.height,
        preset,
        advanced,
        elapsed: started.elapsed(),
        photo_overlap,
        simplify_tolerance: None,
        optional_optimizer: options.optional_optimizer,
        regions,
        nodes: conversion.nodes,
        curves: conversion.curves,
        corners: conversion.smoothing.corners.max(0) as usize,
        optimizer_unknowns: conversion.optimizer_unknowns,
        optimizer_kept: conversion.optimizer_kept,
        optimizer_objective: conversion.optimizer_objective,
        stages: conversion.stages,
        source: Some(Arc::new(raster.clone())),
        unrounded: None,
    })
}

/// Export-only treatment for opaque photographs. Original M/L/C/Z coordinates
/// stay byte-identical. Half-pixel same-color overlap masks renderer seams; an
/// opaque mean-color underlay preserves the source's fully opaque canvas.
/// Never apply this to artwork or to a source containing any transparency.
fn opaque_photo_export(svg: &str, raster: &Raster) -> Result<String, String> {
    let mut result = String::with_capacity(svg.len() + svg.len() / 4);
    let mut paths = svg.split("<path fill=\"");
    result.push_str(paths.next().unwrap_or_default());
    for path in paths {
        let (color, rest) = path.split_once('"').ok_or("Incomplete path color")?;
        if color.len() != 7
            || !color.starts_with('#')
            || !color[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Unexpected path color".into());
        }
        result.push_str(&format!("<path fill=\"{color}\" stroke=\"{color}\" stroke-width=\"0.5\" stroke-linejoin=\"round\"{rest}"));
    }
    let start = result.find("<svg").ok_or("Missing SVG root")?;
    let end = start + result[start..].find('>').ok_or("Incomplete SVG root")? + 1;
    let mut totals = [0u64; 3];
    for pixel in &raster.pixels {
        for (sum, channel) in totals.iter_mut().zip(pixel.0) {
            *sum += channel as u64;
        }
    }
    let mean = totals.map(|sum| sum / raster.pixels.len() as u64);
    result.insert_str(
        end,
        &format!(
            "\n<rect width=\"{}\" height=\"{}\" fill=\"#{:02x}{:02x}{:02x}\"/>\n",
            raster.width, raster.height, mean[0], mean[1], mean[2]
        ),
    );
    Ok(result)
}
