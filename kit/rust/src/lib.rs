//! The recovered Vector Magic engine, every stage of the original's pipeline
//! ported to Rust (the `recovered_*` modules, driven by `recovered_pipeline`
//! and compared to frozen native fixtures to the bit), plus owned
//! post-processing of its output (`simplify`, `regularize`, `straighten`,
//! `shapes`, `sticker`, `prepare`) and the settings it was recovered with.
//! Provenance: docs/SETTINGS.md and analysis/disassembly in the parent package.
//! This crate contains no binary loading, FFI, networking, or proprietary executable.
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the port keeps the original's negated jumps: `!(a > b)` is true when either side is NaN, `a <= b` is not"
)]
#![allow(
    clippy::excessive_precision,
    reason = "constants are transcribed digit for digit from the original binary's doubles"
)]
#![allow(
    clippy::if_same_then_else,
    reason = "the decision trees are transcribed leaf for leaf; identical leaves are the tree's own shape"
)]
use std::collections::BTreeMap;
pub mod caps;
pub mod fitting;
pub mod geometry;
pub mod median_fills;
pub mod nodes;
pub mod palette_fills;
pub mod pixel_art;
pub mod prepare;
pub mod primitives;
pub mod profile;
pub mod raster;
pub mod recovered_colour_model;
pub mod recovered_export;
pub mod recovered_fit;
pub mod recovered_lapack;
pub mod recovered_optimizer;
pub mod recovered_pipeline;
pub mod recovered_segmentation;
pub mod recovered_smoothing;
pub mod recovered_state;
pub mod recovered_topology;
pub mod recovery;
pub mod rectilinear;
pub mod regularize;
pub mod shapes;
pub mod simplify;
pub mod stacking;
pub mod sticker;
pub mod straighten;
pub mod strokes;

pub type Parameters = BTreeMap<String, Vec<f64>>;

pub const PRESETS: [&str; 10] = [
    include_str!("../fixtures/preset-00.txt"),
    include_str!("../fixtures/preset-01.txt"),
    include_str!("../fixtures/preset-02.txt"),
    include_str!("../fixtures/preset-03.txt"),
    include_str!("../fixtures/preset-04.txt"),
    include_str!("../fixtures/preset-05.txt"),
    include_str!("../fixtures/preset-06.txt"),
    include_str!("../fixtures/preset-07.txt"),
    include_str!("../fixtures/preset-08.txt"),
    include_str!("../fixtures/preset-09.txt"),
];

/// Parses the recovered dotted-leader text format, rejecting malformed or duplicate entries.
/// Numeric values are represented as f64 here; legacy storage types are in parameter-bindings.json.
pub fn parse_preset(text: &str) -> Result<Parameters, String> {
    let mut values = BTreeMap::new();
    for (i, raw) in text.lines().enumerate() {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let tokens: Vec<_> = raw.split_whitespace().collect();
        if tokens.len() < 3 || !tokens[1].bytes().all(|b| b == b'.') {
            return Err(format!("Malformed preset line {}", i + 1));
        }
        let mut numbers = Vec::new();
        for token in &tokens[2..] {
            let number: f64 = token
                .parse()
                .map_err(|_| format!("Invalid number on line {}", i + 1))?;
            if !number.is_finite() {
                return Err(format!("Non-finite number on line {}", i + 1));
            }
            numbers.push(number);
        }
        if values.insert(tokens[0].to_owned(), numbers).is_some() {
            return Err(format!("Duplicate key {}", tokens[0]));
        }
    }
    Ok(values)
}

pub fn preset(code: usize) -> Result<Parameters, String> {
    PRESETS
        .get(code)
        .ok_or_else(|| "Preset code must be 0..9".to_owned())
        .and_then(|s| parse_preset(s))
}

/// Observed numeric enum codes. Labels follow the UI string switch; see docs/SETTINGS.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageCategory {
    AliasedArtwork,
    AntiAliasedArtwork,
    Photograph,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    High,
    Medium,
    Low,
}

/// 0x0046BD78 and pointer table 0x00A21D08.
pub fn basic_preset_code(category: ImageCategory, quality: Quality) -> usize {
    let c = match category {
        ImageCategory::Photograph => 0,
        ImageCategory::AntiAliasedArtwork => 1,
        ImageCategory::AliasedArtwork => 2,
    };
    let q = match quality {
        Quality::High => 0,
        Quality::Medium => 1,
        Quality::Low => 2,
    };
    [[8, 7, 6], [5, 4, 3], [2, 1, 0]][c][q]
}

/// Advanced GUI values from the settings snapshot. This deliberately does not mimic C++ object layout.
/// The fields are in the order of the original's 44-byte advanced block (UI object +0x10, plain
/// settings snapshot +0x08; 0x00440190 copies it as eleven DWORDs): field k in the four-byte
/// slot at offset 4k, `palette_anti_alias`, `cluster_colors` and `contour_anti_alias` as one-byte
/// booleans at the start of theirs, `detect_corners` as a whole DWORD. Nothing in the port reads
/// or writes that block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvancedSettings {
    pub colors: i32,
    pub color_sensitivity: i32,
    pub palette_anti_alias: bool,
    pub segmentation_complexity: i32,
    pub min_pixels: i32,
    pub anti_alias_rejection: i32,
    pub cluster_colors: bool,
    pub contour_smoothness: i32,
    pub detect_corners: bool,
    pub curve_complexity: i32,
    pub contour_anti_alias: bool,
}

/// Defaults from 0x0046CD90. These are reset defaults, before later GUI/preset choices.
impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            colors: 2,
            color_sensitivity: 1,
            palette_anti_alias: true,
            segmentation_complexity: 5,
            min_pixels: 0,
            anti_alias_rejection: 0,
            cluster_colors: true,
            contour_smoothness: 6,
            detect_corners: true,
            curve_complexity: 6,
            contour_anti_alias: true,
        }
    }
}

impl AdvancedSettings {
    /// This crate constrains continuous sliders to the analyzed 1..=12 domain.
    /// This is an API boundary, not proof that every original input path clamps values.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("segmentation_complexity", self.segmentation_complexity),
            ("contour_smoothness", self.contour_smoothness),
            ("curve_complexity", self.curve_complexity),
        ] {
            if !(1..=12).contains(&value) {
                return Err(format!("{name} must be 1..=12"));
            }
        }
        if !(0..=2).contains(&self.color_sensitivity)
            || !(0..=2).contains(&self.anti_alias_rejection)
        {
            return Err("Sensitivity and rejection codes must be 0..=2".to_owned());
        }
        if self.colors < 1 || self.min_pixels < 0 {
            return Err(
                "Color count must be positive; minimum pixels must be nonnegative".to_owned(),
            );
        }
        Ok(())
    }
}

fn put(p: &mut Parameters, key: &str, value: f64) {
    p.insert(key.to_owned(), vec![value]);
}
fn put_vec(p: &mut Parameters, key: &str, value: [f64; 3]) {
    p.insert(key.to_owned(), value.to_vec());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Palette,
    Segmentation,
    Smoothing,
    Fitting,
}

/// Recoverable portion of 0x0046B9A0, which starts each task by loading a baseline preset.
/// Custom palette copying, foreground/background state, and progress callbacks are excluded.
pub fn advanced_parameters(s: &AdvancedSettings, stage: Stage) -> Result<Parameters, String> {
    s.validate()?;
    let mut p = preset(if stage == Stage::Palette { 9 } else { 4 })?;
    if stage == Stage::Palette {
        put(
            &mut p,
            "Shared::is_anti_aliased",
            f64::from(s.palette_anti_alias),
        );
        let (hist, peaks) = match s.color_sensitivity {
            0 => (1., 0.),
            1 => (1., 1.),
            _ => (0., 1.),
        };
        put(&mut p, "PaletteFinder::use_16_hist_bins", hist);
        put(&mut p, "PaletteFinder::use_26_connected_peaks", peaks);
    } else if stage == Stage::Segmentation {
        let reverse = f64::from(12 - s.segmentation_complexity);
        let initial = ((reverse + 1.) * 0.0001) as f32;
        put(&mut p, "Segmenter::lambda_pre", f64::from(initial));
        put(&mut p, "Segmenter::lambda_initial", f64::from(initial));
        put(
            &mut p,
            "Segmenter::lambda_final",
            f64::from((0.05 * 400_f64.powf(reverse / 11.)) as f32),
        );
        put(&mut p, "Segmenter::min_num_pixels", f64::from(s.min_pixels));
        // Engine fields +0x214 / +0x20C are not registered under both names.
        // Expose the named anti-alias field; the additional raw flags are returned separately below.
        put(
            &mut p,
            "Shared::is_anti_aliased",
            f64::from(s.anti_alias_rejection != 2),
        );
    } else {
        let t = f64::from(s.contour_smoothness - 1) / 11.;
        let prior_min = [0.05, 0.01, 0.45];
        let prior_max = if s.contour_anti_alias {
            [10.; 3]
        } else {
            [20.; 3]
        };
        let length_min = if s.contour_anti_alias {
            [0.25, 0.05, 0.005]
        } else {
            [0.5; 3]
        };
        let length_max = if s.contour_anti_alias {
            [10., 1., 1.]
        } else {
            [40.; 3]
        };
        let interpolate =
            |lo: [f64; 3], hi: [f64; 3]| std::array::from_fn(|i| lo[i] * (hi[i] / lo[i]).powf(t));
        put_vec(
            &mut p,
            "ContourSmoother::measurement_types",
            [f64::from(s.contour_anti_alias); 3],
        );
        put_vec(&mut p, "ContourSmoother::prior_types", [3.; 3]);
        put_vec(
            &mut p,
            "ContourSmoother::prior_strengths",
            interpolate(prior_min, prior_max),
        );
        put_vec(
            &mut p,
            "ContourSmoother::length_penalty_weights",
            interpolate(length_min, length_max),
        );
        put_vec(
            &mut p,
            "ContourSmoother::do_puncture_corners",
            [0., f64::from(s.detect_corners), 0.],
        );
        put_vec(
            &mut p,
            "ContourSmoother::is_enabled",
            [1., f64::from(s.detect_corners), 1.],
        );
        let reverse = f64::from(12 - s.curve_complexity);
        put(
            &mut p,
            "BezierFitter::stat_thresh_initial",
            (reverse + 1.) * 0.0001,
        );
        put(
            &mut p,
            "BezierFitter::stat_thresh_final",
            0.1 * 200_f64.powf(reverse / 11.),
        );
        put(
            &mut p,
            "Shared::is_anti_aliased",
            f64::from(s.contour_anti_alias),
        );
    }
    Ok(p)
}

/// The one parameter set advanced mode gives the engine: 0x0046B9A0 loads
/// preset 4 for the segmentation, smoothing and fitting stages and applies
/// the slider formulas of `advanced_parameters` to each; the port's stages
/// read one map, so the three stages' keys are merged here. The palette
/// stage's keys (`PaletteFinder::*`) have no consumer in the port and are
/// not carried; the port reads one `Shared::is_anti_aliased`, so the
/// segmentation's anti-alias rejection and the contour anti-aliasing must
/// agree. Neither are the segmentation's two switches without a registered
/// name (engine +0x214, set when anti-alias rejection is 0, and +0x218 from
/// colour clustering; docs/SETTINGS.md), which no stage of the port reads.
/// Nothing on the default path calls this.
pub fn advanced_preset(s: &AdvancedSettings) -> Result<Parameters, String> {
    if (s.anti_alias_rejection != 2) != s.contour_anti_alias {
        return Err(
            "Advanced mode: the engine reads one anti-aliasing flag, so anti-alias rejection \
             (0 or 1 with contour anti-aliasing on, 2 with it off) must agree with contour \
             anti-aliasing"
                .to_owned(),
        );
    }
    let mut p = advanced_parameters(s, Stage::Fitting)?;
    let segmentation = advanced_parameters(s, Stage::Segmentation)?;
    for key in [
        "Segmenter::lambda_pre",
        "Segmenter::lambda_initial",
        "Segmenter::lambda_final",
        "Segmenter::min_num_pixels",
    ] {
        p.insert(key.to_owned(), segmentation[key].clone());
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_extracted_presets_have_the_same_100_keys() {
        let first = preset(0).unwrap();
        assert_eq!(first.len(), 100);
        for code in 0..10 {
            let p = preset(code).unwrap();
            assert_eq!(
                p.keys().collect::<Vec<_>>(),
                first.keys().collect::<Vec<_>>()
            );
            assert_eq!(p["Shared::image_type_code"], vec![code as f64]);
            for key in [
                "measurement_types",
                "prior_types",
                "prior_strengths",
                "length_penalty_weights",
                "is_enabled",
            ] {
                assert_eq!(p[&format!("ContourSmoother::{key}")].len(), 3);
            }
        }
    }
    #[test]
    fn parser_rejects_corrupt_configuration() {
        for text in [
            "x .. NaN",
            "x .. 1\nx .. 2",
            "x ..",
            "x ... nope",
            "x value 2",
        ] {
            assert!(parse_preset(text).is_err(), "{text}");
        }
        assert_eq!(parse_preset("a . 1e-005").unwrap()["a"], [1e-5]);
    }
    #[test]
    fn arithmetic_matches_144_instruction_interpreter_cases() {
        let mut count = 0;
        for line in include_str!("../fixtures/settings-golden.csv")
            .lines()
            .skip(1)
        {
            let v: Vec<f64> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let s = AdvancedSettings {
                segmentation_complexity: v[0] as i32,
                contour_smoothness: v[1] as i32,
                curve_complexity: v[2] as i32,
                contour_anti_alias: v[3] != 0.,
                detect_corners: v[4] != 0.,
                anti_alias_rejection: v[5] as i32,
                cluster_colors: v[6] != 0.,
                ..Default::default()
            };
            let seg = advanced_parameters(&s, Stage::Segmentation).unwrap();
            let fit = advanced_parameters(&s, Stage::Fitting).unwrap();
            let actual: Vec<_> = [
                seg["Segmenter::lambda_initial"].clone(),
                seg["Segmenter::lambda_final"].clone(),
                fit["ContourSmoother::prior_strengths"].clone(),
                fit["ContourSmoother::length_penalty_weights"].clone(),
                fit["BezierFitter::stat_thresh_initial"].clone(),
                fit["BezierFitter::stat_thresh_final"].clone(),
            ]
            .concat();
            for (i, (a, expected)) in actual.iter().zip(&v[7..]).enumerate() {
                assert!(
                    (a - expected).abs() <= 1e-7 * expected.abs().max(1.),
                    "case {count} field {i}: {a} != {expected}"
                );
            }
            assert_eq!(fit["ContourSmoother::do_puncture_corners"], [0., v[4], 0.]);
            assert_eq!(fit["ContourSmoother::is_enabled"], [1., v[4], 1.]);
            count += 1;
        }
        assert_eq!(count, 144);
    }
    #[test]
    fn invalid_controls_do_not_silently_produce_parameters() {
        for value in [i32::MIN, 0, 13, i32::MAX] {
            let s = AdvancedSettings {
                contour_smoothness: value,
                ..Default::default()
            };
            assert!(advanced_parameters(&s, Stage::Smoothing).is_err());
        }
    }
    #[test]
    fn basic_selection_matches_the_extracted_pointer_table() {
        let categories = [
            ImageCategory::Photograph,
            ImageCategory::AntiAliasedArtwork,
            ImageCategory::AliasedArtwork,
        ];
        let qualities = [Quality::High, Quality::Medium, Quality::Low];
        let mut actual = Vec::new();
        for c in categories {
            for q in qualities {
                actual.push(basic_preset_code(c, q));
            }
        }
        assert_eq!(actual, [8, 7, 6, 5, 4, 3, 2, 1, 0]);
    }
    #[test]
    fn advanced_preset_changes_only_stage_keys_and_needs_one_anti_alias_flag() {
        let merged = advanced_preset(&AdvancedSettings::default()).unwrap();
        let base = preset(4).unwrap();
        assert_eq!(
            merged.keys().collect::<Vec<_>>(),
            base.keys().collect::<Vec<_>>()
        );
        let changed: Vec<_> = merged
            .iter()
            .filter(|(k, v)| base[*k] != **v)
            .map(|(k, _)| k.as_str())
            .collect();
        for key in [
            "Segmenter::lambda_pre",
            "Segmenter::lambda_final",
            "ContourSmoother::prior_strengths",
            "BezierFitter::stat_thresh_final",
        ] {
            assert!(merged.contains_key(key), "{key}");
        }
        assert!(
            changed.iter().all(|k| {
                k.starts_with("Segmenter::")
                    || k.starts_with("ContourSmoother::")
                    || k.starts_with("BezierFitter::")
            }),
            "{changed:?}"
        );
        let disagreeing = AdvancedSettings {
            anti_alias_rejection: 2,
            contour_anti_alias: true,
            ..AdvancedSettings::default()
        };
        assert!(advanced_preset(&disagreeing).is_err());
    }
}
