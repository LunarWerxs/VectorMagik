//! The whole conversion without any original code: the stage sequence the
//! engine's vectorize entry runs (import 0x473c20, preprocessing 0x473850,
//! segmentation 0x488d20, contour construction 0x499eb0, smoothing 0x497c30,
//! fitting 0x4a1030, export preparation 0x47d4c0, size 0x4745d0 and the SVG
//! writer 0x47ca40) over the recovered Rust stages, with the record hand-offs
//! the engine made through its own memory done here through owned values.
//! Every constant that was not a registered parameter is named where it is
//! used, with the engine field it came from.
use crate::geometry::{Cubic, Point};
use crate::recovered_export::{self, Document, ExportSettings, ExportShape};
use crate::recovered_fit::ScheduleSettings;
use crate::recovered_optimizer::OptimizerSettings;
use crate::recovered_segmentation::{self, SegmenterParams};
use crate::recovered_smoothing::{
    self, AaImage, AaInput, Canvas, CgParams, Contour as SmoothContour, Edge as SmoothEdge,
    Node as SmoothNode, Params as SmoothParams, Report as SmoothReport,
};
use crate::recovered_state::{self, FittedContours};
use crate::recovered_topology;
use crate::Parameters;

/// What the application chooses per conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// export+0x2c: 0 paths only, 1 hole loops, 2 hole loops in colour groups.
    pub layering: i32,
    /// export+0x34: 0 strokes every path with its fill.
    pub stroking: i32,
    /// BezierFitter+0x1c: the optional pass 0x49fc80, registered by no preset.
    pub optional_optimizer: bool,
    /// Owned, for the improved defaults: the optional pass keeps the plain
    /// fit when it fails or its step raises its own objective, rather than
    /// failing the conversion or keeping the step
    /// (`recovered_state::fit_contours_optimized_guarded`). Off runs the
    /// original's pass as it is (the `-high-optimizer` references).
    pub guard_optimizer: bool,
    /// Owned, for the improved defaults: a translucent region's fill is its
    /// straight colour rather than the premultiplied bytes
    /// (`recovered_export::ExportSettings::straight_fills`). Off writes the
    /// original's text.
    pub straight_fills: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            layering: 2,
            stroking: 1,
            optional_optimizer: false,
            guard_optimizer: false,
            straight_fills: false,
        }
    }
}

/// Wall-clock seconds each stage of one conversion took; a measurement,
/// never an input to any stage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StageSeconds {
    pub preprocess: f64,
    pub segment: f64,
    pub topology: f64,
    pub smooth: f64,
    pub fit: f64,
    pub export: f64,
}

/// The document and what the stages reported on the way.
#[derive(Clone, Debug)]
pub struct Conversion {
    pub document: Document,
    pub regions: usize,
    pub nodes: usize,
    pub contours: usize,
    /// The curves the fitter made (its filled slots; a curve between two
    /// regions is drawn by both). The fitter allocates more slots than it
    /// fills: a terminal slot per scheduled run and the one-step intervals,
    /// which export as lines.
    pub curves: usize,
    pub smoothing: SmoothReport,
    /// The optional pass's unknown count, 0 when it did not run or, guarded,
    /// failed.
    pub optimizer_unknowns: usize,
    /// The objective 0x49ccb0 before and after the optional pass's step when
    /// the pass ran to the end (the step's value even when the guard put the
    /// plain fit back), None otherwise.
    pub optimizer_objective: Option<[f64; 2]>,
    /// The optional pass's step stands: always when it ran unguarded, and
    /// under `Options::guard_optimizer` when it neither failed nor raised
    /// its objective. False when the pass did not run.
    pub optimizer_kept: bool,
    pub stages: StageSeconds,
    #[cfg(test)]
    debug_points: Vec<Point>,
    #[cfg(test)]
    debug_curves: Vec<Cubic>,
    #[cfg(test)]
    debug_shapes: Vec<ExportShape>,
}

/// The generator seed a fresh engine process carries into smoothing: the
/// initialised value of 0xa21d2c, consumed only by 0x495a30.
const INITIAL_SEED: i32 = 1;
/// The node-set pass period at smoother+0xb0.
const NODE_SET_PERIOD: i32 = 10;
/// The two diagonal unit vectors at smoother+0x620: the engine's double is
/// one unit below Rust's FRAC_1_SQRT_2, and the placement decisions feel it.
const DIAGONAL_UNITS: [f64; 4] = [
    -0.707_106_781_186_547_46,
    -0.707_106_781_186_547_46,
    -0.707_106_781_186_547_46,
    0.707_106_781_186_547_46,
];
/// BezierFitter+0x10, constructed 0.5 and registered by no preset.
const RAMP_FRACTION: f64 = 0.5;
/// export+0x0, the resolution the coordinates are in; 0x4745d0 is called
/// with 72 as well.
const EXPORT_DPI: i32 = 72;

fn value(p: &Parameters, key: &str) -> Result<f64, String> {
    p.get(key)
        .and_then(|v| v.first().copied())
        .ok_or_else(|| format!("missing parameter {key}"))
}

fn triple(p: &Parameters, key: &str) -> Result<[f64; 3], String> {
    let v = p
        .get(key)
        .ok_or_else(|| format!("missing parameter {key}"))?;
    if v.len() != 3 {
        return Err(format!("parameter {key} needs three values"));
    }
    Ok([v[0], v[1], v[2]])
}

/// The smoother's parameters from a preset: the registered triples, doubles
/// and per-phase optimizer values, plus the four fields no preset registers
/// (smoother constructor 0x496740: backup_state 1, eps_decay 0.5, fd_eps the
/// single-precision 1e-4, reserved 0).
pub fn smoothing_params(p: &Parameters) -> Result<SmoothParams, String> {
    let ints = |key: &str| -> Result<[i32; 3], String> { Ok(triple(p, key)?.map(|x| x as i32)) };
    let mut phases = Vec::with_capacity(3);
    for phase in 0..3 {
        let get = |name: &str| value(p, &format!("ContourSmoother::phase_{phase}.cg_{name}"));
        phases.push(CgParams {
            iter_for_restart: get("iter_for_restart")? as i32,
            min_iter: get("min_iter")? as i32,
            max_iter: get("max_iter")? as i32,
            knock_out_count_down: get("knock_out_count_down")? as i32,
            backup_state: 1,
            use_eps_from_step: get("use_eps_from_step")? as i32,
            min_eps: get("min_eps")?,
            max_eps: get("max_eps")?,
            eps_decay: 0.5,
            step_fraction: get("step_fraction")?,
            fd_eps: 1e-4_f32 as f64,
            reserved: 0,
            num_blind_steps: get("num_blind_steps")? as i32,
            num_seeing_steps: get("num_seeing_steps")? as i32,
            blind_blend_latest: get("blind_blend_latest")?,
            rel_tol: get("rel_tol")?,
            abs_tol: get("abs_tol")?,
            line_search_type: get("line_search_type")? as i32,
            quad_line_srch_rel_tol: get("quad_line_srch_rel_tol")?,
            quad_line_srch_max_iter: get("quad_line_srch_max_iter")? as i32,
            max_step_size: get("max_step_size")?,
        });
    }
    Ok(SmoothParams {
        measurement_types: ints("ContourSmoother::measurement_types")?,
        prior_types: ints("ContourSmoother::prior_types")?,
        prior_strengths: triple(p, "ContourSmoother::prior_strengths")?,
        length_penalty_weights: triple(p, "ContourSmoother::length_penalty_weights")?,
        do_puncture_corners: ints("ContourSmoother::do_puncture_corners")?,
        is_enabled: ints("ContourSmoother::is_enabled")?,
        optimizer_type: ints("ContourSmoother::optimizer_type")?,
        air_pressure_weight: value(p, "ContourSmoother::air_pressure_weight")?,
        perturbation_range: value(p, "ContourSmoother::perturbation_range")?,
        length_barrier_weight: value(p, "ContourSmoother::length_barrier_weight")?,
        puncture_flatness_thresh: value(p, "ContourSmoother::puncture_flatness_thresh")?,
        puncture_corner_thresh: value(p, "ContourSmoother::puncture_corner_thresh")?,
        anti_inv_pot_meas_scale: value(p, "ContourSmoother::anti_inv_pot_meas_scale")?,
        anti_inv_pot_prior_scale: value(p, "ContourSmoother::anti_inv_pot_prior_scale")?,
        phases: [phases[0], phases[1], phases[2]],
    })
}

/// The fitter's schedule from a preset (BezierFitter+0x0, +0x8, +0x18) with
/// the constructed ramp fraction.
pub fn fitting_settings(p: &Parameters) -> Result<ScheduleSettings, String> {
    let passes = value(p, "BezierFitter::max_iterations")?;
    if !(0.0..=1000.0).contains(&passes) || passes.fract() != 0.0 {
        return Err("BezierFitter::max_iterations must be a whole number up to 1000".into());
    }
    Ok(ScheduleSettings {
        initial_threshold: value(p, "BezierFitter::stat_thresh_initial")?,
        final_threshold: value(p, "BezierFitter::stat_thresh_final")?,
        ramp_fraction: RAMP_FRACTION,
        passes: passes as usize,
    })
}

/// Everything the smoother reads, built from the raster by the stages before
/// it (kept apart so tests can compare it with the original's records).
pub(crate) struct SmoothingInputs {
    /// The anti-aliased image model's input (the prepared pixels, the label
    /// image and the region colours, moved out of the segmentation), None
    /// for the presets that do not run it.
    pub(crate) image: Option<AaImage>,
    /// The segmentation's contour records.
    pub(crate) regions: usize,
    pub(crate) nodes: Vec<SmoothNode>,
    pub(crate) contours: Vec<SmoothContour>,
    pub(crate) canvas: Canvas,
    pub(crate) params: SmoothParams,
    /// Seconds of the three stages this ran.
    pub(crate) stages: StageSeconds,
}

pub(crate) fn prepare_smoothing(
    bgra: &[u8],
    width: usize,
    height: usize,
    preset: &Parameters,
) -> Result<SmoothingInputs, String> {
    let anti_aliased = value(preset, "Shared::is_anti_aliased")? != 0.0;

    // 0x473c20 and 0x473850.
    let order = value(preset, "Preprocessor::reduce_noise_order")? as i32;
    let clock = std::time::Instant::now();
    let prepared = recovered_segmentation::preprocess(bgra, width, height, order);
    let preprocess_seconds = clock.elapsed().as_secs_f64();

    // 0x488d20: canonical labels 0..regions, one contour record per region.
    let segmentation = recovered_segmentation::segment(
        &prepared,
        width,
        height,
        &SegmenterParams::from_parameters(preset)?,
    )?;
    let regions = segmentation.records.len();
    let segment_seconds = clock.elapsed().as_secs_f64() - preprocess_seconds;

    // 0x499eb0 on the label image. The builder's "extra array" pointer
    // (builder+0x1c) is the shared block at engine+0x208, whose second word is
    // Shared::is_anti_aliased: anti-aliased conversions skip the straight-run
    // thinning, the others run it.
    let topology =
        recovered_topology::build(&segmentation.labels, width, height, regions, !anti_aliased)?;
    if topology.contours.len() != regions {
        return Err("Contour construction did not produce one contour per region".into());
    }
    let topology_seconds = clock.elapsed().as_secs_f64() - preprocess_seconds - segment_seconds;

    // 0x497c30 over the engine's records: the node set's single-precision
    // copies are the integer grid positions, the entry flags start at zero.
    let nodes: Vec<SmoothNode> = topology
        .nodes
        .iter()
        .map(|n| SmoothNode {
            x: n.x,
            y: n.y,
            fx: n.x as f32,
            fy: n.y as f32,
            state: n.state,
            flags: n.flags,
            aux: 0,
        })
        .collect();
    let contours: Vec<SmoothContour> = topology
        .contours
        .iter()
        .zip(&segmentation.records)
        .map(|(c, record)| SmoothContour {
            pixels: record.pixels,
            area: 0.0,
            nodes: c.nodes.clone(),
            edges: c
                .edges
                .iter()
                .map(|e| SmoothEdge {
                    flag: 0,
                    other: e.other,
                    steps: e.steps,
                })
                .collect(),
            region: record.colour,
            color: record.bytes,
            parent: c.enclosing,
            dir: [0.0; 2],
            parity: 0,
        })
        .collect();
    let canvas = Canvas {
        width: width as i32,
        height: height as i32,
        border: topology.border,
    };
    let params = smoothing_params(preset)?;
    // Only the anti-aliased image model reads the pixels, labels and colours
    // past this point; the other presets let them go here.
    let recovered_segmentation::Segmentation {
        labels, colours, ..
    } = segmentation;
    let image = anti_aliased.then_some(AaImage {
        width: width as i32,
        height: height as i32,
        pixels: prepared,
        labels,
        region_colors: colours,
    });
    Ok(SmoothingInputs {
        image,
        regions,
        nodes,
        contours,
        canvas,
        params,
        stages: StageSeconds {
            preprocess: preprocess_seconds,
            segment: segment_seconds,
            topology: topology_seconds,
            ..StageSeconds::default()
        },
    })
}

/// Converts `bgra` (`width * height * 4` bytes, blue, green, red, alpha, as
/// 0x473c20 imports them) under `preset` into the SVG text the original
/// engine wrote for it.
pub fn vectorize(
    bgra: &[u8],
    width: usize,
    height: usize,
    preset: &Parameters,
    options: &Options,
) -> Result<Conversion, String> {
    vectorize_with_seed(bgra, width, height, preset, options, INITIAL_SEED)
}

/// `vectorize` with the generator seed the smoothing starts from.
pub fn vectorize_with_seed(
    bgra: &[u8],
    width: usize,
    height: usize,
    preset: &Parameters,
    options: &Options,
    seed: i32,
) -> Result<Conversion, String> {
    if width < 2 || height < 2 || width > 16_384 || height > 16_384 {
        return Err("Conversion needs an image of 2 to 16384 pixels a side".into());
    }
    if bgra.len() != width * height * 4 {
        return Err("Pixel data does not match the image size".into());
    }
    if value(preset, "Preprocessor::filter_type")? != 1.0 {
        return Err("Only preprocessing filter type 1 was recovered".into());
    }
    let prepared = prepare_smoothing(bgra, width, height, preset)?;
    let SmoothingInputs {
        image,
        regions,
        mut nodes,
        mut contours,
        canvas,
        params,
        mut stages,
    } = prepared;
    let aa = image.as_ref().map(|image| AaInput {
        image,
        seed,
        every: NODE_SET_PERIOD,
        units: DIAGONAL_UNITS,
        prepare: true,
    });
    let clock = std::time::Instant::now();
    let smoothing = recovered_smoothing::smooth(&mut nodes, &mut contours, canvas, &params, aa)
        .map_err(|e| format!("Smoothing: {}", e.0))?;
    stages.smooth = clock.elapsed().as_secs_f64();

    // 0x4a1030 over the smoothed records: node ids, the label across each
    // leaving edge, the corner states, and curve indices still unset (-1).
    let ids: Vec<Vec<usize>> = contours
        .iter()
        .map(|c| c.nodes.iter().map(|&i| i as usize).collect())
        .collect();
    let edges: Vec<Vec<u32>> = contours
        .iter()
        .map(|c| c.edges.iter().map(|e| e.other as u32).collect())
        .collect();
    let points: Vec<Point> = nodes.iter().map(|n| Point { x: n.x, y: n.y }).collect();
    let flags: Vec<u8> = nodes.iter().map(|n| n.state).collect();
    let indices = vec![u32::MAX as usize; nodes.len()];
    let settings = fitting_settings(preset)?;
    let clock = std::time::Instant::now();
    let (fitted, optimizer_report, optimizer_kept): (FittedContours, Option<_>, bool) =
        if options.optional_optimizer {
            let corners: Vec<Vec<bool>> = contours
                .iter()
                .map(|c| c.edges.iter().map(|e| e.flag & 1 != 0).collect())
                .collect();
            if options.guard_optimizer {
                let (fitted, outcome) = recovered_state::fit_contours_optimized_guarded(
                    &ids,
                    &edges,
                    &corners,
                    &points,
                    &flags,
                    &indices,
                    settings,
                    OptimizerSettings::default(),
                )?;
                (fitted, outcome.report, outcome.kept)
            } else {
                let (fitted, report) = recovered_state::fit_contours_optimized(
                    &ids,
                    &edges,
                    &corners,
                    &points,
                    &flags,
                    &indices,
                    settings,
                    OptimizerSettings::default(),
                )?;
                (fitted, Some(report), true)
            }
        } else {
            (
                recovered_state::fit_contours(&ids, &edges, &points, &flags, &indices, settings)?,
                None,
                false,
            )
        };

    stages.fit = clock.elapsed().as_secs_f64();

    // 0x47d4c0, 0x4745d0 and 0x47ca40 over the fitter's arrays.
    let fitted_curves = fitted.curves.iter().flatten().count();
    let shapes: Vec<ExportShape> = contours
        .iter()
        .zip(&fitted.parts)
        .map(|(c, parts)| ExportShape {
            colour: c.color,
            parent: c.parent,
            nodes: c.nodes.iter().map(|&i| i as usize).collect(),
            pieces: parts.clone(),
        })
        .collect();
    let curves: Vec<Cubic> = fitted
        .curves
        .iter()
        .map(|c| {
            c.unwrap_or(Cubic {
                points: [Point {
                    x: f64::NAN,
                    y: f64::NAN,
                }; 4],
            })
        })
        .collect();
    let export = ExportSettings {
        straight_fills: options.straight_fills,
        ..ExportSettings::new(options.layering, options.stroking, EXPORT_DPI)
    };
    let clock = std::time::Instant::now();
    let document = recovered_export::export(&shapes, &points, &curves, &export)?;
    stages.export = clock.elapsed().as_secs_f64();
    #[cfg(feature = "profile")]
    eprintln!("{}", crate::profile::report());
    Ok(Conversion {
        document,
        regions,
        nodes: nodes.len(),
        contours: contours.len(),
        curves: fitted_curves,
        smoothing,
        optimizer_unknowns: optimizer_report.as_ref().map_or(0, |r| r.unknowns),
        optimizer_objective: optimizer_report.map(|r| [r.objective_before, r.objective_after]),
        optimizer_kept,
        stages,
        #[cfg(test)]
        debug_points: points,
        #[cfg(test)]
        debug_curves: curves,
        #[cfg(test)]
        debug_shapes: shapes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 126 inputs of `--segmentation-fixtures` (k = preset * 14 + image)
    /// and the documents `--export-fixtures` wrote for the same k, with
    /// layering k % 3 and stroking (k / 3) % 2.
    fn inputs() -> Vec<(usize, usize, usize, Vec<u8>)> {
        include_str!("../fixtures/native-segmentation.csv")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let f: Vec<&str> = line.split(',').collect();
                let preset: usize = f[2].parse().unwrap();
                let width: usize = f[4].parse().unwrap();
                let height: usize = f[5].parse().unwrap();
                let pixels = f[6..6 + 4 * width * height]
                    .iter()
                    .map(|x| x.parse::<f64>().unwrap() as u8)
                    .collect();
                (preset, width, height, pixels)
            })
            .collect()
    }

    fn documents() -> Vec<(i32, i32, String)> {
        include_str!("../fixtures/native-export.csv")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let f: Vec<&str> = line.split(',').collect();
                let hex = f[f.len() - 1].trim();
                let bytes: Vec<u8> = (0..hex.len() / 2)
                    .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                    .collect();
                (
                    f[6].parse().unwrap(),
                    f[7].parse().unwrap(),
                    String::from_utf8(bytes).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn whole_conversion_matches_the_original_on_126_cases() {
        let inputs = inputs();
        let documents = documents();
        assert_eq!(inputs.len(), 126);
        assert_eq!(documents.len(), 126);
        for (k, ((preset, width, height, pixels), (layering, stroking, svg))) in
            inputs.iter().zip(&documents).enumerate()
        {
            assert_eq!(k / 14, *preset);
            let parameters = crate::preset(*preset).unwrap();
            let options = Options {
                layering: *layering,
                stroking: *stroking,
                optional_optimizer: false,
                guard_optimizer: false,
                straight_fills: false,
            };
            let conversion = vectorize(pixels, *width, *height, &parameters, &options)
                .unwrap_or_else(|e| panic!("case {k}: {e}"));
            if conversion.document.svg != *svg {
                // Locate the stage: compare every export input with the
                // original's for this case.
                let case = &crate::recovered_export::tests::cases()[k];
                let bits = |p: &Point| (p.x.to_bits(), p.y.to_bits());
                for (i, (a, b)) in conversion.debug_points.iter().zip(&case.nodes).enumerate() {
                    if bits(a) != bits(b) {
                        eprintln!(
                            "node {i}: rust ({:e}, {:e}) original ({:e}, {:e})",
                            a.x, a.y, b.x, b.y
                        );
                    }
                }
                eprintln!(
                    "nodes {} vs {}",
                    conversion.debug_points.len(),
                    case.nodes.len()
                );
                for (i, (a, b)) in conversion.debug_shapes.iter().zip(&case.shapes).enumerate() {
                    if a != b {
                        eprintln!("shape {i}: rust {:?}\n original {:?}", a, b);
                    }
                }
                for (i, (a, b)) in conversion.debug_curves.iter().zip(&case.curves).enumerate() {
                    if a.points
                        .iter()
                        .zip(&b.points)
                        .any(|(p, q)| bits(p) != bits(q) && !(p.x.is_nan() && q.x.is_nan()))
                    {
                        eprintln!(
                            "curve {i}: rust {:?}\n original {:?}",
                            a.points
                                .map(|p| (format!("{:e}", p.x), format!("{:e}", p.y))),
                            b.points
                                .map(|p| (format!("{:e}", p.x), format!("{:e}", p.y)))
                        );
                    }
                }
            }
            assert_eq!(conversion.document.svg, *svg, "case {k} (preset {preset})");
            assert_eq!(conversion.contours, conversion.regions);
        }
    }

    /// The records the smoother receives from the Rust stages, against the
    /// original's records for the same images (`--aa-smoothing-fixtures`
    /// ran the original import, preprocessing, segmentation and contour
    /// construction on the 42 blended images; the export cases 42 to 83 use
    /// the same images under the same presets).
    #[test]
    fn anti_aliased_smoothing_inputs_match_the_original_records() {
        let inputs = inputs();
        let fixture = crate::recovered_smoothing::aa_tests::cases(
            include_str!("../fixtures/native-aa-smoothing.csv"),
            "aasmoothing",
        );
        let mut matched = 0;
        for (k, (preset, width, height, pixels)) in inputs.iter().enumerate() {
            let parameters = crate::preset(*preset).unwrap();
            let mine = prepare_smoothing(pixels, *width, *height, &parameters).unwrap();
            let Some(image) = &mine.image else {
                continue;
            };
            let Some(theirs) = fixture.iter().find(|c| {
                c.preset == *preset as i64
                    && c.image.width == *width as i32
                    && c.image.height == *height as i32
                    && c.image.pixels == image.pixels
            }) else {
                continue;
            };
            matched += 1;
            assert_eq!(image.labels, theirs.image.labels, "case {k} labels");
            assert_eq!(
                image.region_colors, theirs.image.region_colors,
                "case {k} region colours"
            );
            assert_eq!(mine.canvas.border, theirs.border, "case {k} border");
            assert_eq!(mine.nodes.len(), theirs.nodes.len(), "case {k} node count");
            for (i, (a, b)) in mine.nodes.iter().zip(&theirs.nodes).enumerate() {
                assert_eq!(
                    (a.x, a.y, a.fx, a.fy, a.state, a.flags, a.aux),
                    (b.x, b.y, b.fx, b.fy, b.state, b.flags, b.aux),
                    "case {k} node {i}"
                );
            }
            assert_eq!(
                mine.contours.len(),
                theirs.contours.len(),
                "case {k} contour count"
            );
            for (i, (a, b)) in mine.contours.iter().zip(&theirs.contours).enumerate() {
                assert_eq!(
                    (a.pixels, &a.nodes, a.region, a.color, a.parent),
                    (b.pixels, &b.nodes, b.region, b.color, b.parent),
                    "case {k} contour {i}"
                );
                let ea: Vec<_> = a.edges.iter().map(|e| (e.flag, e.other, e.steps)).collect();
                let eb: Vec<_> = b.edges.iter().map(|e| (e.flag, e.other, e.steps)).collect();
                assert_eq!(ea, eb, "case {k} contour {i} edges");
            }
            assert_eq!(mine.params, theirs.params, "case {k} params");
            assert_eq!(
                (NODE_SET_PERIOD, DIAGONAL_UNITS),
                (theirs.every, theirs.units)
            );
        }
        // The export cases use every third blended image; fourteen of them
        // fall under the same preset as their fixture capture.
        assert_eq!(matched, 14);
    }

    #[test]
    fn preset_parameters_build_every_stage_setting() {
        for code in 0..10 {
            let p = crate::preset(code).unwrap();
            let smoothing = smoothing_params(&p).unwrap();
            assert_eq!(smoothing.phases[0].backup_state, 1);
            assert_eq!(smoothing.phases[2].eps_decay, 0.5);
            let fitting = fitting_settings(&p).unwrap();
            assert_eq!(fitting.ramp_fraction, 0.5);
            assert!(fitting.passes > 0);
            SegmenterParams::from_parameters(&p).unwrap();
        }
    }

    /// `guard_optimizer` through the whole conversion on every third of the
    /// 126 synthetic inputs (all nine presets): the guarded document is the unguarded pass's where the step
    /// stands and the plain fit's where it rose; the pass fills no curve
    /// slot the plain fit left empty, so the curve count stays.
    #[test]
    fn guarded_optional_pass_keeps_the_plain_document_when_its_step_rises() {
        let (mut kept, mut rose) = (0, 0);
        for (k, (preset, width, height, pixels)) in inputs().iter().enumerate().step_by(3) {
            let parameters = crate::preset(*preset).unwrap();
            let run = |optional_optimizer, guard_optimizer| {
                let options = Options {
                    optional_optimizer,
                    guard_optimizer,
                    ..Options::default()
                };
                vectorize(pixels, *width, *height, &parameters, &options)
                    .unwrap_or_else(|e| panic!("case {k}: {e}"))
            };
            let plain = run(false, false);
            let stepped = run(true, false);
            let guarded = run(true, true);
            assert!(!plain.optimizer_kept && plain.optimizer_objective.is_none());
            assert!(stepped.optimizer_kept);
            assert_eq!(guarded.optimizer_objective, stepped.optimizer_objective);
            assert_eq!(guarded.optimizer_unknowns, stepped.optimizer_unknowns);
            let [before, after] = stepped.optimizer_objective.unwrap();
            if after > before {
                assert!(!guarded.optimizer_kept, "case {k}");
                assert_eq!(guarded.document.svg, plain.document.svg, "case {k}");
                rose += 1;
            } else {
                assert!(guarded.optimizer_kept, "case {k}");
                assert_eq!(guarded.document.svg, stepped.document.svg, "case {k}");
                kept += 1;
            }
            assert_eq!(plain.curves, stepped.curves, "case {k}");
        }
        assert_eq!(kept + rose, 42);
        assert!(kept > 0 && rose > 0, "{kept} kept, {rose} rose");
    }

    #[test]
    fn rejects_bad_sizes_and_data() {
        let p = crate::preset(2).unwrap();
        let options = Options::default();
        assert!(vectorize(&[0; 4], 1, 1, &p, &options).is_err());
        assert!(vectorize(&[0; 12], 2, 2, &p, &options).is_err());
    }
}
