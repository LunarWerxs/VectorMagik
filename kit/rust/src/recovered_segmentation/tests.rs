//! Preprocessing and segmentation against `--segmentation-fixtures`: 126
//! cases, fourteen synthetic blended images under all nine presets through
//! the original import, preprocessing and segmentation.

use super::*;

/// One contour record: pixels, region id, parent, four colour bytes, four colour floats.
pub type ContourRecord = (i32, i32, i32, [u8; 4], [f32; 4]);

pub(super) struct Case {
    pub id: i64,
    pub preset: i64,
    pub width: usize,
    pub height: usize,
    pub input: Vec<u8>,
    pub filter_type: i32,
    pub order: i32,
    pub prepared: Vec<u8>,
    pub tables: Vec<i32>,
    pub labels: Vec<i32>,
    pub label_count: i32,
    pub parents: Vec<i32>,
    pub region_count: i32,
    /// Per contour record: pixels, region id, parent, four colour bytes,
    /// four colour floats.
    pub records: Vec<ContourRecord>,
    pub roots: i32,
}

fn take(v: &[f64], at: &mut usize) -> f64 {
    let x = v[*at];
    *at += 1;
    x
}
fn take_i(v: &[f64], at: &mut usize) -> i32 {
    take(v, at) as i32
}

pub(super) fn parse(line: &str) -> Case {
    let mut fields = line.split(',');
    fields.next();
    let v: Vec<f64> = fields.map(|f| f.parse().unwrap()).collect();
    let mut at = 0;
    let id = take(&v, &mut at) as i64;
    let preset = take(&v, &mut at) as i64;
    let _pattern = take(&v, &mut at);
    let width = take(&v, &mut at) as usize;
    let height = take(&v, &mut at) as usize;
    let count = width * height;
    let input: Vec<u8> = (0..4 * count).map(|_| take_i(&v, &mut at) as u8).collect();
    // The shared block's seven words follow the input; the engine derives them.
    for _ in 0..7 {
        take_i(&v, &mut at);
    }
    take(&v, &mut at);
    let filter_type = take_i(&v, &mut at);
    let order = take_i(&v, &mut at);
    assert_eq!(take_i(&v, &mut at), -1);
    assert_eq!(take(&v, &mut at) as usize, width);
    assert_eq!(take(&v, &mut at) as usize, height);
    let prepared: Vec<u8> = (0..4 * count).map(|_| take_i(&v, &mut at) as u8).collect();
    let tables: Vec<i32> = (0..88).map(|_| take_i(&v, &mut at)).collect();
    assert_eq!(take_i(&v, &mut at), -2);
    assert_eq!(take(&v, &mut at) as usize, width);
    assert_eq!(take(&v, &mut at) as usize, height);
    let labels: Vec<i32> = (0..count).map(|_| take_i(&v, &mut at)).collect();
    let label_count = take_i(&v, &mut at);
    let max_label = take_i(&v, &mut at);
    let parents: Vec<i32> = (0..=max_label).map(|_| take_i(&v, &mut at)).collect();
    let region_count = take_i(&v, &mut at);
    take(&v, &mut at);
    let records_len = take(&v, &mut at) as usize;
    let records = (0..records_len)
        .map(|_| {
            let pixels = take_i(&v, &mut at);
            let region = take_i(&v, &mut at);
            let parent = take_i(&v, &mut at);
            let mut bytes = [0u8; 4];
            for b in bytes.iter_mut() {
                *b = take_i(&v, &mut at) as u8;
            }
            let mut floats = [0f32; 4];
            for f in floats.iter_mut() {
                *f = take(&v, &mut at) as f32;
            }
            (pixels, region, parent, bytes, floats)
        })
        .collect();
    let roots = take_i(&v, &mut at);
    assert_eq!(at, v.len(), "case {id}: trailing values");
    Case {
        id,
        preset,
        width,
        height,
        input,
        filter_type,
        order,
        prepared,
        tables,
        labels,
        label_count,
        parents,
        region_count,
        records,
        roots,
    }
}

pub(super) fn cases() -> Vec<Case> {
    include_str!("../../fixtures/native-segmentation.csv")
        .lines()
        .filter(|l| l.starts_with("segmentation,"))
        .map(parse)
        .collect()
}

#[test]
fn preprocessing_matches_126_native_images() {
    let cases = cases();
    assert_eq!(cases.len(), 126);
    let mut filtered = 0;
    for case in &cases {
        assert_eq!(case.filter_type, 1, "case {}: filter type", case.id);
        assert_eq!(&case.tables[28..36], &NEIGHBOUR_DX, "case {}: dx", case.id);
        assert_eq!(&case.tables[55..63], &NEIGHBOUR_DY, "case {}: dy", case.id);
        let mine = preprocess(&case.input, case.width, case.height, case.order);
        for (i, (a, b)) in mine.iter().zip(&case.prepared).enumerate() {
            assert_eq!(
                a,
                b,
                "case {} (preset {}, order {}): pixel ({}, {}) channel {}: {} vs native {} (input {:?})",
                case.id,
                case.preset,
                case.order,
                (i / 4) % case.width,
                (i / 4) / case.width,
                i % 4,
                a,
                b,
                &case.input[i / 4 * 4..i / 4 * 4 + 4]
            );
        }
        if case.order > 0 {
            filtered += 1;
        }
    }
    assert_eq!(filtered, 70);
}

/// One stage of `--segmentation-aa-stages`: the label image, the highest
/// label, the live count, the parents and the sub-pixel region records
/// (count, interior, colour, flags, colour floats) up to the highest label.
pub(super) struct AaStage {
    pub labels: Vec<i32>,
    pub max_label: i32,
    pub label_count: i32,
    pub parents: Vec<i32>,
    pub regions: Vec<(i32, i32, i32, u8, [f32; 4])>,
    /// After stage 5: every region's 42 features.
    pub features: Vec<[f64; 42]>,
}

pub(super) struct AaCase {
    pub id: i64,
    pub preset: i64,
    pub width: usize,
    pub height: usize,
    pub stages: Vec<AaStage>,
    /// The two edge images of 0x483150 and the mask of 0x481f60(2) before
    /// stage 5, with the shared block's type code.
    pub edge: (Vec<f32>, Vec<f32>),
    pub mask: Vec<u8>,
    pub type_code: i32,
}

pub(super) fn parse_aa(line: &str) -> AaCase {
    let mut fields = line.split(',');
    fields.next();
    let v: Vec<f64> = fields.map(|f| f.parse().unwrap()).collect();
    let mut at = 0;
    let id = take(&v, &mut at) as i64;
    let preset = take(&v, &mut at) as i64;
    let _pattern = take(&v, &mut at);
    let width = take(&v, &mut at) as usize;
    let height = take(&v, &mut at) as usize;
    assert_eq!(take_i(&v, &mut at), 0x104, "case {id}: segmentation object");
    assert_eq!(take_i(&v, &mut at), 0x130, "case {id}: sub-pixel object");
    let count = width * height;
    let mut stages = Vec::new();
    let mut edge = (Vec::new(), Vec::new());
    let mut mask = Vec::new();
    let mut type_code = 0;
    for stage in 1..=9 {
        if stage == 5 {
            assert_eq!(take_i(&v, &mut at), -50, "case {id}: edge marker");
            take(&v, &mut at);
            assert_eq!(take(&v, &mut at), 42.0);
            assert_eq!(take(&v, &mut at), 0.0032);
            type_code = take_i(&v, &mut at);
            edge.0 = (0..count).map(|_| take(&v, &mut at) as f32).collect();
            edge.1 = (0..count).map(|_| take(&v, &mut at) as f32).collect();
            mask = (0..count).map(|_| take_i(&v, &mut at) as u8).collect();
        }
        assert_eq!(take_i(&v, &mut at), -stage, "case {id}: stage marker");
        let labels: Vec<i32> = (0..count).map(|_| take_i(&v, &mut at)).collect();
        let max_label = take_i(&v, &mut at);
        let label_count = take_i(&v, &mut at);
        let parents: Vec<i32> = (0..=max_label).map(|_| take_i(&v, &mut at)).collect();
        let region_count = take_i(&v, &mut at);
        let mut regions = Vec::new();
        for _ in 0..region_count {
            let pixels = take_i(&v, &mut at);
            let interior = take_i(&v, &mut at);
            let colour = take_i(&v, &mut at);
            let flags = take_i(&v, &mut at) as u8;
            let mut floats = [0f32; 4];
            for f in floats.iter_mut() {
                *f = take(&v, &mut at) as f32;
            }
            regions.push((pixels, interior, colour, flags, floats));
        }
        regions.truncate((max_label + 1) as usize);
        let mut features = Vec::new();
        if stage == 5 {
            let n = take_i(&v, &mut at);
            for _ in 0..n {
                let mut f = [0f64; 42];
                for x in f.iter_mut() {
                    *x = take(&v, &mut at);
                }
                features.push(f);
            }
        }
        stages.push(AaStage {
            labels,
            max_label,
            label_count,
            parents,
            regions,
            features,
        });
    }
    assert_eq!(at, v.len(), "case {id}: trailing values");
    AaCase {
        id,
        preset,
        width,
        height,
        stages,
        edge,
        mask,
        type_code,
    }
}

pub(super) fn aa_cases() -> Vec<AaCase> {
    include_str!("../../fixtures/native-segmentation-aa-stages.csv")
        .lines()
        .filter(|l| l.starts_with("segaa,"))
        .map(parse_aa)
        .collect()
}

/// One step of `--segmentation-aa-sweeps`: the sweep's merge count (-1
/// the initialisation, -2 a rebuild, -3 the beach pass, -4 the diagonal
/// extraction), the stage state and the cached costs.
pub(super) struct AaSweepStep {
    pub result: i32,
    pub state: AaStage,
    pub costs: Vec<f32>,
}

pub(super) fn parse_aa_sweeps(line: &str) -> (i64, Vec<AaSweepStep>) {
    let mut fields = line.split(',');
    fields.next();
    let v: Vec<f64> = fields.map(|f| f.parse().unwrap()).collect();
    let mut at = 0;
    let id = take(&v, &mut at) as i64;
    let _preset = take(&v, &mut at);
    let _pattern = take(&v, &mut at);
    let width = take(&v, &mut at) as usize;
    let height = take(&v, &mut at) as usize;
    let count = width * height;
    let mut steps = Vec::new();
    loop {
        let marker = take_i(&v, &mut at);
        if marker == -99 {
            break;
        }
        assert_eq!(
            marker,
            -(100 + steps.len() as i32),
            "case {id}: step marker"
        );
        let result = take_i(&v, &mut at);
        assert_eq!(take_i(&v, &mut at), 0);
        let labels: Vec<i32> = (0..count).map(|_| take_i(&v, &mut at)).collect();
        let max_label = take_i(&v, &mut at);
        let label_count = take_i(&v, &mut at);
        let parents: Vec<i32> = (0..=max_label).map(|_| take_i(&v, &mut at)).collect();
        let region_count = take_i(&v, &mut at);
        let mut regions = Vec::new();
        for _ in 0..region_count {
            let pixels = take_i(&v, &mut at);
            let interior = take_i(&v, &mut at);
            let colour = take_i(&v, &mut at);
            let flags = take_i(&v, &mut at) as u8;
            let mut floats = [0f32; 4];
            for f in floats.iter_mut() {
                *f = take(&v, &mut at) as f32;
            }
            regions.push((pixels, interior, colour, flags, floats));
        }
        regions.truncate((max_label + 1) as usize);
        let costs: Vec<f32> = (0..count).map(|_| take(&v, &mut at) as f32).collect();
        steps.push(AaSweepStep {
            result,
            state: AaStage {
                labels,
                max_label,
                label_count,
                parents,
                regions,
                features: Vec::new(),
            },
            costs,
        });
    }
    assert_eq!(at, v.len(), "case {id}: trailing values");
    (id, steps)
}

fn check_aa_state(case: i64, step: usize, mine: &sub_pixels::AaSnapshot, native: &AaStage) {
    let w = mine.labels.len() / native.labels.len().max(1);
    let _ = w;
    let mismatches: Vec<String> = mine
        .labels
        .iter()
        .zip(&native.labels)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .take(8)
        .map(|(i, (a, b))| format!("{i}: {a} vs {b}"))
        .collect();
    assert!(
        mismatches.is_empty(),
        "case {case} step {step}: labels differ at {}",
        mismatches.join(", ")
    );
    assert_eq!(
        mine.max_label, native.max_label,
        "case {case} step {step}: max"
    );
    assert_eq!(
        mine.label_count, native.label_count,
        "case {case} step {step}: count"
    );
    assert_eq!(
        mine.parents, native.parents,
        "case {case} step {step}: parents"
    );
    for (i, (a, b)) in mine.regions.iter().zip(&native.regions).enumerate() {
        assert_eq!(a, b, "case {case} step {step}: region {i}");
    }
    assert_eq!(
        mine.regions.len(),
        native.regions.len(),
        "case {case} step {step}: regions"
    );
}

/// Busy pixel-edged pictures for the sweep's term cache: a one-pixel
/// checker, a two-colour random dither and a grid of soft lines.
fn busy_pictures() -> Vec<(&'static str, usize, usize, Vec<u8>)> {
    let mut out = Vec::new();
    let (w, h) = (20, 20);
    let checker: Vec<u8> = (0..w * h)
        .flat_map(|p| {
            let v = if (p % w + p / w) % 2 == 0 { 0 } else { 255 };
            [v, v, v, 255]
        })
        .collect();
    out.push(("checker", w, h, checker));
    let mut seed = 12345u32;
    let dither: Vec<u8> = (0..24 * 24)
        .flat_map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if seed >> 31 == 0 {
                [30, 60, 200, 255]
            } else {
                [240, 220, 40, 255]
            }
        })
        .collect();
    out.push(("dither", 24, 24, dither));
    let (w, h) = (33, 33);
    let grid: Vec<u8> = (0..w * h)
        .flat_map(|p| {
            let (x, y) = (p % w, p / w);
            let line = |c: usize| match c % 8 {
                0 => 255,
                1 | 7 => 110,
                _ => 0,
            };
            let v = 255 - line(x).max(line(y)) as u8;
            [v, v / 2 + 60, 255 - v / 3, 255]
        })
        .collect();
    out.push(("grid", w, h, grid));
    out
}

/// `sweep`'s term cache and joined costs (`sub_pixels::TermCache`) leave the
/// labels, parents, regions and cached costs of every sweep of the
/// sub-pixel run as costing every term the original's way does.
#[test]
fn sweep_term_cache_equals_the_original_costs() {
    let params = SegmenterParams::from_parameters(&crate::preset(5).unwrap()).unwrap();
    for (name, w, h, rgba) in busy_pictures() {
        let image = preprocess(&rgba, w, h, 0);
        let run = |reference: bool| {
            const INF: i32 = 0x7fff_ffff;
            let mut sp = super_pixels::SuperPixels::for_test(&image, w, h, &params);
            sp.aa_stage(1).unwrap();
            sp.sub.terms.reference = reference;
            sp.sub.use_interior_size = false;
            sp.sub.visited.clear();
            sp.sub_init();
            // 0x4a9ac0's three sweep phases, each state kept.
            let mut states = Vec::new();
            for phase in 0..3 {
                if phase > 0 {
                    sp.sub_rebuild(false, false);
                }
                for i in 0..21 {
                    let limit = if phase == 2 { INF } else { i + 1 };
                    let merges = sp.sweep(if phase == 0 { i + 1 } else { INF }, limit);
                    states.push((merges, sp.aa_snapshot(), sp.sub.cost.clone()));
                    if merges == 0 {
                        break;
                    }
                }
            }
            states
        };
        let (cached, original) = (run(false), run(true));
        assert!(original.iter().any(|s| s.0 > 0), "{name}: no merges");
        for (k, (a, b)) in cached.iter().zip(&original).enumerate() {
            assert_eq!(a.0, b.0, "{name} sweep {k}: merges");
            assert_eq!(a.1, b.1, "{name} sweep {k}: state");
            let costs =
                a.2.iter()
                    .zip(&b.2)
                    .all(|(x, y)| x.to_bits() == y.to_bits());
            assert!(costs, "{name} sweep {k}: costs");
        }
    }
}

#[test]
fn sub_pixel_sweeps_match_native_step_by_step() {
    let inputs = cases();
    let lines: Vec<&str> = include_str!("../../fixtures/native-segmentation-aa-sweeps.csv")
        .lines()
        .filter(|l| l.starts_with("segsweep,"))
        .collect();
    assert_eq!(lines.len(), 42);
    for line in lines {
        let (id, steps) = parse_aa_sweeps(line);
        let input = inputs.iter().find(|c| c.id == id).unwrap();
        let params =
            SegmenterParams::from_parameters(&crate::preset(input.preset as usize).unwrap())
                .unwrap();
        let image = preprocess(&input.input, input.width, input.height, input.order);
        let mut sp =
            super_pixels::SuperPixels::for_test(&image, input.width, input.height, &params);
        sp.aa_stage(1).unwrap();
        let mut step = 0usize;
        let check = |sp: &super_pixels::SuperPixels, result: i32, step: &mut usize| {
            let native = &steps[*step];
            check_aa_state(id, *step, &sp.aa_snapshot(), &native.state);
            assert_eq!(result, native.result, "case {id} step {}: result", *step);
            let bad: Vec<String> = sp
                .sub
                .cost
                .iter()
                .zip(&native.costs)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .take(6)
                .map(|(i, (a, b))| format!("{i}: {a:e} vs {b:e}"))
                .collect();
            assert!(
                bad.is_empty(),
                "case {id} step {}: costs differ at {}",
                *step,
                bad.join(", ")
            );
            *step += 1;
        };
        sp.sub.use_interior_size = false;
        sp.sub.visited.clear();
        sp.sub_init();
        check(&sp, -1, &mut step);
        const INF: i32 = 0x7fff_ffff;
        let mut r = sp.sweep(1, 1);
        check(&sp, r, &mut step);
        if r != 0 {
            for i in 0..20 {
                r = sp.sweep(i + 2, i + 2);
                check(&sp, r, &mut step);
                if r == 0 {
                    break;
                }
            }
        }
        sp.sub_rebuild(false, false);
        check(&sp, -2, &mut step);
        r = sp.sweep(INF, 1);
        check(&sp, r, &mut step);
        if r != 0 {
            for i in 0..20 {
                r = sp.sweep(INF, i + 2);
                check(&sp, r, &mut step);
                if r == 0 {
                    break;
                }
            }
        }
        sp.sub_rebuild(false, false);
        check(&sp, -2, &mut step);
        r = sp.sweep(INF, INF);
        check(&sp, r, &mut step);
        if r != 0 {
            for _ in 0..20 {
                r = sp.sweep(INF, INF);
                check(&sp, r, &mut step);
                if r == 0 {
                    break;
                }
            }
        }
        sp.beach();
        check(&sp, -3, &mut step);
        sp.extract_diagonals();
        check(&sp, -4, &mut step);
        sp.sub_rebuild(false, false);
        check(&sp, -2, &mut step);
        r = sp.sweep(INF, 4);
        check(&sp, r, &mut step);
        if r != 0 {
            for _ in 0..20 {
                r = sp.sweep(INF, 4);
                check(&sp, r, &mut step);
                if r == 0 {
                    break;
                }
            }
        }
        sp.sub_rebuild(false, false);
        check(&sp, -2, &mut step);
        assert_eq!(step, steps.len(), "case {id}: step count");
    }
}

/// The stages of the anti-aliased path ported so far.
const AA_STAGES_PORTED: u32 = 9;

#[test]
fn anti_aliased_stages_match_native() {
    let inputs = cases();
    let aa = aa_cases();
    assert_eq!(aa.len(), 42);
    for case in &aa {
        let input = inputs.iter().find(|c| c.id == case.id).unwrap();
        assert_eq!((input.width, input.height), (case.width, case.height));
        let params =
            SegmenterParams::from_parameters(&crate::preset(case.preset as usize).unwrap())
                .unwrap();
        let image = preprocess(&input.input, input.width, input.height, input.order);
        let mut sp = super_pixels::SuperPixels::for_test(&image, case.width, case.height, &params);
        for stage in 1..=AA_STAGES_PORTED {
            if stage == 5 {
                let mask = sp.boundary_mask();
                assert_eq!(mask, case.mask, "case {}: boundary mask", case.id);
                let blurred =
                    super_pixels::SuperPixels::gaussian_blur(&image, case.width, case.height, 0);
                let edge = sp.edge_image(&blurred);
                assert_eq!(edge.0, case.edge.0, "case {}: edge gradient", case.id);
                assert_eq!(edge.1, case.edge.1, "case {}: edge laplacian", case.id);
                sp.aa_stage(5).unwrap();
                let native = &case.stages[4];
                let features = &sp.sub.features;
                assert_eq!(
                    features.len(),
                    native.features.len(),
                    "case {}: feature rows",
                    case.id
                );
                for (i, (a, b)) in features.iter().zip(&native.features).enumerate() {
                    for k in 0..42 {
                        assert!(
                            a[k] == b[k] || (a[k].is_nan() && b[k].is_nan()),
                            "case {} region {i} feature {k}: {:e} vs native {:e}",
                            case.id,
                            a[k],
                            b[k]
                        );
                    }
                }
            } else {
                if stage == 6 {
                    assert_eq!(
                        sp.type_code(),
                        case.type_code,
                        "case {}: type code",
                        case.id
                    );
                }
                sp.aa_stage(stage).unwrap();
            }
            let mine = sp.aa_snapshot();
            let native = &case.stages[(stage - 1) as usize];
            let mismatches: Vec<String> = mine
                .labels
                .iter()
                .zip(&native.labels)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .take(8)
                .map(|(i, (a, b))| format!("({}, {}) {a} vs {b}", i % case.width, i / case.width))
                .collect();
            assert!(
                mismatches.is_empty(),
                "case {} (preset {}) stage {stage}: labels differ at {}",
                case.id,
                case.preset,
                mismatches.join(", ")
            );
            assert_eq!(
                mine.max_label, native.max_label,
                "case {} stage {stage}: max",
                case.id
            );
            assert_eq!(
                mine.label_count, native.label_count,
                "case {} stage {stage}: count",
                case.id
            );
            assert_eq!(
                mine.parents, native.parents,
                "case {} stage {stage}: parents",
                case.id
            );
            if stage >= 2 {
                for (i, (a, b)) in mine.regions.iter().zip(&native.regions).enumerate() {
                    assert_eq!(a, b, "case {} stage {stage}: region {i}", case.id);
                }
                assert_eq!(
                    mine.regions.len(),
                    native.regions.len(),
                    "case {} stage {stage}: region count",
                    case.id
                );
            }
        }
    }
}

#[test]
fn segmentation_matches_native_labels_and_records() {
    let cases = cases();
    let mut checked = 0;
    for case in &cases {
        assert_eq!(
            &case.tables[24..26],
            &[1, 0],
            "case {}: two-neighbour dx",
            case.id
        );
        assert_eq!(
            &case.tables[48..50],
            &[0, 1],
            "case {}: two-neighbour dy",
            case.id
        );
        assert_eq!(
            &case.tables[36..40],
            &[1, case.width as i32, -1, -(case.width as i32)],
            "case {}: row offsets",
            case.id
        );
        let params =
            SegmenterParams::from_parameters(&crate::preset(case.preset as usize).unwrap())
                .unwrap();
        let image = preprocess(&case.input, case.width, case.height, case.order);
        assert_eq!(image, case.prepared);
        let seg = segment(&image, case.width, case.height, &params).unwrap();
        let mismatches: Vec<String> = seg
            .labels
            .iter()
            .zip(&case.labels)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(8)
            .map(|(i, (a, b))| format!("({}, {}) {a} vs {b}", i % case.width, i / case.width))
            .collect();
        assert!(
            mismatches.is_empty(),
            "case {} (preset {}): labels differ at {}",
            case.id,
            case.preset,
            mismatches.join(", ")
        );
        assert_eq!(
            seg.label_count, case.label_count,
            "case {}: label count",
            case.id
        );
        assert_eq!(
            seg.max_label,
            case.parents.len() as i32 - 1,
            "case {}: max label",
            case.id
        );
        assert_eq!(seg.parents, case.parents, "case {}: parents", case.id);
        assert_eq!(
            seg.colours.len() as i32,
            case.region_count,
            "case {}: colours",
            case.id
        );
        assert_eq!(
            seg.records.len(),
            case.records.len(),
            "case {}: records",
            case.id
        );
        for (i, (mine, native)) in seg.records.iter().zip(&case.records).enumerate() {
            assert_eq!(
                (
                    mine.pixels,
                    mine.colour,
                    mine.bytes,
                    seg.colours[mine.colour as usize]
                ),
                (native.0, native.1, native.3, native.4),
                "case {} record {i}",
                case.id
            );
            assert_eq!(native.2, -1, "case {} record {i}: parent", case.id);
        }
        assert_eq!(case.roots, seg.max_label + 1, "case {}: roots", case.id);
        checked += 1;
    }
    assert_eq!(checked, 126);
}

/// The smallest images the pipeline accepts (2 pixels a side) run through
/// every preset: solid, transparent and checkerboard, 2x2, 2x9 and 9x2. A
/// 1-pixel side is refused rather than read at pixel -1 by the sub-pixel
/// edge image.
#[test]
fn smallest_images_segment_under_every_preset() {
    let images = |w: usize, h: usize| -> [Vec<u8>; 3] {
        let solid = [40u8, 90, 200, 255].repeat(w * h);
        let transparent = vec![0u8; 4 * w * h];
        let checker = (0..w * h)
            .flat_map(|p| {
                if (p % w + p / w).is_multiple_of(2) {
                    [0u8, 0, 0, 255]
                } else {
                    [255u8, 255, 255, 255]
                }
            })
            .collect();
        [solid, transparent, checker]
    };
    for code in 0..10 {
        let parameters = crate::preset(code).unwrap();
        let params = SegmenterParams::from_parameters(&parameters).unwrap();
        let order = parameters["Preprocessor::reduce_noise_order"][0] as i32;
        for (w, h) in [(2, 2), (2, 9), (9, 2)] {
            for (kind, image) in images(w, h).iter().enumerate() {
                let prepared = preprocess(image, w, h, order);
                let seg = segment(&prepared, w, h, &params)
                    .unwrap_or_else(|e| panic!("preset {code} {w}x{h} image {kind}: {e}"));
                assert_eq!(seg.labels.len(), w * h);
                assert!(
                    seg.labels
                        .iter()
                        .all(|&l| l >= 0 && (l as usize) < seg.records.len()),
                    "preset {code} {w}x{h} image {kind}: labels outside the records"
                );
            }
        }
        for (w, h) in [(1, 5), (5, 1), (1, 1)] {
            assert!(segment(&vec![0; 4 * w * h], w, h, &params).is_err());
        }
    }
}

/// 0x4a8640's visited-pair key keeps 16 bits of the region: region 65,536
/// files under region 0's keys, and a neighbour past 65,535 spills into the
/// region's half (0x4a86db / 0x4a86e3).
#[test]
fn sweep_pair_keys_alias_past_16_bits_as_the_original() {
    assert_eq!(sub_pixels::visited_key(3, 7), 0x0003_0007);
    assert_eq!(
        sub_pixels::visited_key(65_536, 5),
        sub_pixels::visited_key(0, 5)
    );
    assert_eq!(
        sub_pixels::visited_key(0, 0x1_0005),
        sub_pixels::visited_key(1, 5)
    );
    assert_ne!(sub_pixels::visited_key(1, 5), sub_pixels::visited_key(5, 1));
}

/// Feature 39's k-d tree against 0x4a8800's scan over every large region,
/// to the bit: colours on the byte grid (many ties), off it, and with NaN
/// and infinite channels, from empty sets to a few thousand regions.
#[test]
fn nearest_large_colour_matches_the_full_scan_to_the_bit() {
    let mut state = 0x2545_f491_u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let scan = |set: &[[f32; 4]], r: &[f32; 4]| -> f64 {
        let mut best = 1e100;
        for o in set {
            let d = [o[0] - r[0], o[1] - r[1], o[2] - r[2], o[3] - r[3]];
            let dist = ((d[0] as f64 * d[0] as f64 + d[1] as f64 * d[1] as f64)
                + d[2] as f64 * d[2] as f64)
                + d[3] as f64 * d[3] as f64;
            if best > dist {
                best = dist;
            }
        }
        best
    };
    let mut checked = 0;
    for size in [0usize, 1, 7, 8, 9, 17, 100, 1000, 4000] {
        for grid in [true, false] {
            let colour = |next: &mut dyn FnMut() -> u32| -> [f32; 4] {
                let mut c = [0.0f32; 4];
                for v in c.iter_mut() {
                    let bits = next();
                    *v = if grid {
                        (bits % 8) as f32 * 36.0 / 255.0
                    } else {
                        (bits >> 8) as f32 / (1u32 << 24) as f32
                    };
                }
                match next() % 97 {
                    0 => c[(next() % 4) as usize] = f32::NAN,
                    1 => c[(next() % 4) as usize] = f32::INFINITY,
                    _ => {}
                }
                c
            };
            let set: Vec<[f32; 4]> = (0..size).map(|_| colour(&mut next)).collect();
            let tree = sub_pixels::NearestColour::new(set.iter().copied());
            for _ in 0..300 {
                let r = colour(&mut next);
                assert_eq!(
                    tree.nearest(&r).to_bits(),
                    scan(&set, &r).to_bits(),
                    "{size} colours, query {r:?}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 9 * 2 * 300);
}

/// 0x4a9d10's erosion by heap equals the original's rescan loop: the same
/// labels on random pictures with coarse colours (many equal distances, so
/// the first-in-scan-order tie rule decides), blobs, and long thin flagged
/// lines, the case the heap exists for.
#[test]
fn erosion_heap_equals_the_scan() {
    let mut state = 0x9e37_79b9_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let mut cases = 0;
    for (w, h) in [
        (1i32, 1i32),
        (2, 2),
        (5, 3),
        (16, 16),
        (40, 9),
        (3, 120),
        (64, 48),
    ] {
        for shape in 0..3 {
            let n = (w * h) as usize;
            let colours: Vec<[f32; 4]> = (0..n)
                .map(|_| {
                    let mut c = [0.0f32; 4];
                    for v in c.iter_mut() {
                        *v = (next() % 4) as f32 * 85.0 / 255.0;
                    }
                    c
                })
                .collect();
            // Labels 0..4 unflagged, label 5 the flagged region: speckle,
            // a blob, or a one-pixel-wide line through the picture.
            let mut labels: Vec<i32> = (0..n).map(|_| (next() % 5) as i32).collect();
            for (p, label) in labels.iter_mut().enumerate() {
                let (x, y) = (p as i32 % w, p as i32 / w);
                let flagged = match shape {
                    0 => next() % 3 == 0,
                    1 => (x - w / 2).abs() + (y - h / 2).abs() <= (w.min(h) / 2).max(1),
                    _ => x == w / 2 || y == (x * h) / w.max(1),
                };
                if flagged {
                    *label = 5;
                }
            }
            let closest = |labels: &[i32], q: i32| -> Option<(f32, i32)> {
                let (x, y) = (q % w, q / w);
                let c = colours[q as usize];
                let mut best = 1.0e10f32;
                let mut label = -1;
                let mut found = false;
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    if nx < 0 || nx >= w || ny < 0 || ny >= h {
                        continue;
                    }
                    let p = (ny * w + nx) as usize;
                    if labels[p] == 5 {
                        continue;
                    }
                    let o = colours[p];
                    let d = [o[0] - c[0], o[1] - c[1], o[2] - c[2], o[3] - c[3]];
                    let dist = ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]) + d[3] * d[3];
                    if best > dist {
                        best = dist;
                        label = labels[p];
                        found = true;
                    }
                }
                found.then_some((best, label))
            };
            let pixels: Vec<i32> = (0..n as i32).filter(|&p| labels[p as usize] == 5).collect();
            let mut by_scan = labels.clone();
            sub_pixels::erode_by_scan(&mut by_scan, &pixels, closest);
            let mut by_heap = labels.clone();
            sub_pixels::erode(&mut by_heap, w, h, &pixels, closest);
            assert_eq!(by_heap, by_scan, "{w}x{h}, shape {shape}");
            cases += 1;
        }
    }
    assert_eq!(cases, 21);
}
