# Faithful Rust engine port

The intended product is an owned Rust implementation of the original engine's
algorithms, decisions and data flow. Recover behavior from the local machine
code, port it, and compare it against the original engine's own output frozen
before the host was removed on September 22, 2026. The port is complete: the
Vector Magic engine is rewritten in Rust and the application runs with zero
original machine code, so there is no headless host, no bridge, no native
module and no native backend any more. No original C++ source was recovered;
this is a semantic port of disassembled operations, not a literal source-line
translation. All work and builds stay offline.

## Current executable replacements

`rust/src/recovered_fit.rs` and `recovered_state.rs` own the following behavior.
Their APIs use ordinary
Rust slices, vectors and f64 values; it has no original-image, Windows, Qt, FFI
or allocator dependency. It also passes an offline Linux target compile check.

| Original routine | Recovered behavior | Rust implementation |
|---|---|---|
| 0x49c7b0 | Chord-length sample parameterization | `fitting::chord_length_samples` |
| 0x49dcf0 | Fixed-endpoint quadratic ridge solve and SSE | `fit_quadratic` |
| 0x49d360 | Fixed-endpoint cubic ridge solve | `recovered_fit::fit_cubic` |
| 0x49e960 | Seed, count-based dispatch and fit error | `fit_interval`, `fit_contour_interval` |
| 0x49a560 | Initial one-step intervals and terminal record | `schedule_contour` |
| 0x49f250 | Merged-error delta | `schedule_contour` |
| 0x49f0d0 | Signed grow/shrink error delta | `schedule_contour` |
| 0x49f310 | Strict-threshold merge, then backward/forward shift | `schedule_contour` |
| 0x49f8b0 | Sequential passes and geometric threshold schedule | `schedule_contour` |
| 0x4a0dc0 | Scheduled node states and curve-index assignment | `mark_schedule` plus `schedule_contour` |
| 0x4a0f00 | Initial traversal over contours with shared nodes | `initialize_contours` |
| 0x49b560 | Scratch capacity and numeric flag normalization | `recovered_state::prepare_fitting` |
| 0x49ed80 / 0x49e380 | Partial derivative-record writes, quadratic branch | `derivative_record` |
| 0x49ef50 | Ordered shared-contour refinement and active-count sum | `refine_contours` |
| 0x49b620 | Line/curve records, edge metadata and curve orientation | `finalize_contour` |
| 0x4a1030 ordinary branch | Preparation, initial fit, refinement, finalization | `fit_contours` |
| 0x49ccb0 | Optional-pass objective: data residuals plus weighted unit-tangent differences | `recovered_optimizer::objective` |
| 0x49fc80 assembly | Offsets, gradient vector, upper-triangular Hessian rows with junction terms (0x47e1f0, 0x47e250, 0x47e2c0, 0x47fe00, 0x47ff00, 0x49ab00) | `recovered_optimizer::assemble` |
| 0x49fc80 update | Coordinate read/write through 0x49b2b0 / 0x49b350, quadratic regeneration | `recovered_optimizer::apply` |
| 0x47f650 / 0x47f790 | Cubic Bernstein to power, quadratic power to Bernstein | `fitting::cubic_to_power`, `power_to_quadratic` |
| 0x4a1030 optional branch | The four stages with derivative records, then the pass | `fit_contours_optimized` |
| 0x498a50 | Node grid: vertices touching a label change, border-first numbering | `recovered_topology::build` |
| 0x4999d0 | Boundary tracing with the region on the right hand, checkerboard choice, neighbour labels | `recovered_topology::build` |
| 0x4993e0 / 0x498930 | Border and stair-step thinning with the constructor's pattern table, renumbering, edge steps | `recovered_topology::build` |
| 0x499eb0 | Contour construction: node records, corners, contours, enclosing contour | `recovered_topology::build` |
| 0x4941e0 / 0x495b00 | Smoothing energy (code-3 and code-0 priors, air pressure, displacement measurement) and its gradient | `recovered_smoothing::Smoother::objective`, `gradient` |
| 0x4ada10 / 0x4ad010 | Fletcher-Reeves conjugate gradients with restarts, stall test and the quadratic line search; border pin and near-border clamp after every move | `recovered_smoothing::Cg` |
| 0x494a90 / 0x4adfb0 | Corner puncturing: seven-node window, five turns, the thirteen-constant decision tree | `recovered_smoothing::Smoother::puncture`, `decide` |
| 0x4880a0 / 0x488710 / 0x487fe0 | Image measurement (anti-aliased presets): edge rasterization into pixel event loops, region weights from the loops' areas, the model colour energy and its gradient | `recovered_smoothing::Smoother::image_energy`, `image_gradient` |
| 0x4a2520 / 0x4a2040, 0x495a30 / 0x495720 | Colour model (two-region blend, constrained least squares with Cholesky and the library `dgemv`/`dgemm`), sub-pixel placement and the seeded perturbation | `recovered_smoothing::Smoother::colour_model_evaluate`, `perturb` |
| 0x482190 / 0x4822b0, 0x4849a0 / 0x485be0, 0x4939a0 | Neighbour lists over the ANN kd-tree (sliding midpoint split 0x5bb2b0, search 0x5b97b0), the inversion passes with parity and crossing tests, the after-iteration restart | `recovered_smoothing::kd`, `Smoother::node_set_pass`, `changed_pass` |
| 0x4ad3e0 | Line search type 5: seeing iterations with the quadratic search, blind steps of the remembered length | `recovered_smoothing::Cg::line_search5` |
| 0x497c30 | Setup, three phases, puncturing, both halves | `recovered_smoothing::smooth` |
| 0x473c20 / 0x473850 | Preprocessing: alpha premultiplication, the library's `EnhanceImage` filter `reduce_noise_order` times | `recovered_segmentation::preprocess` |
| 0x4aab30, 0x4aaac0 / 0x481c60, 0x4ab730 / 0x4ab3c0 / 0x4aafd0, 0x48ae30 / 0x48ac00 / 0x484100, 0x489de0 / 0x484c90 / 0x482e90, 0x4ab820, 0x489a50, 0x4ab710, 0x488c70 | The super-pixel segmenter: quadtree seeds, the lambda schedule, merges and boundary moves, component renumbering, colour registration, colour quantisation and diagonal extraction (presets 0 to 2), contour records | `recovered_segmentation::segment` (`super_pixels.rs`) |
| 0x4a9ac0 / 0x48c6f0 / 0x48b690 / 0x4a8640 / 0x4a6bb0 / 0x489f40 / 0x48bc60 / 0x4a7f20, 0x4a83a0, 0x48b160, 0x4a9890 / 0x4a8800 / 0x483150 / 0x4bab40 / 0x481f60 / 0x4ae090, 0x4a9d10 / 0x4ae1f0, 0x4a7eb0 | The sub-pixel segmenter of presets 3 to 5: rebuilds with boundary sets and cached model costs, the merge sweeps, recolouring, quantisation, the region features (edge images, Gaussian blur, boundary mask, PCA) with the two decision trees and the erosion refinement | `recovered_segmentation::segment` (`sub_pixels.rs`) |
| 0x4a2520 / 0x4a1f20 / 0x4a2040 | The colour model shared by the smoother and the segmenter | `recovered_colour_model` |
| 0x47d4c0 | Export hole loops: each shape's children's pieces that border it, runs chained end node to start node | `recovered_export` |
| 0x4745d0 | Export document size | `recovered_export` |
| 0x47c2f0 | Export colour groups: order of first appearance, tags 0 first / 3 inside / 1 last / 2 alone | `recovered_export` |
| 0x47c660 / 0x4741b0 | Export shape walk with piece emission; holes walked backwards with every piece reversed | `recovered_export` |
| vtable 0x8dcfa0: 0x476170, 0x475b40, 0x475de0 / 0x475a80, 0x475f20, 0x475b50 / 0x475c50, 0x475ac0, 0x475af0 | SVG writer: document start, visible when alpha > 10, groups `<g id="#rrggbbaa">`, path with `stroke="#fill" stroke-width="0.09375"` when export+0x34 is 0 and opacity `"1.00"` above alpha 0xfa else `"%0.2f"` of alpha/255, `" M x y"` / `" L x y"`, `" C ..."`, `' Z" />'`, `"</svg>"`; numbers are std::fixed precision 2, which is MSVCR71's `"%.2f"`, the exact binary digits rounded half away from zero, and every endl is CR LF because the original opened the file in text mode | `recovered_export` |
| 0x473850 / 0x497c30 / 0x49e960 / export | The whole conversion, `recovered_pipeline::vectorize`: preprocess, segment, contour construction with thinning skipped for anti-aliased presets (the builder's +0x1c "extra array" pointer is the shared block at engine+0x208 whose second word is `Shared::is_anti_aliased`), smoothing with generator seed 1 (a fresh process's 0xa21d2c), node-set period 10 and diagonal units -0.70710678118654746 (one unit below Rust's `FRAC_1_SQRT_2`), fitting with ramp fraction 0.5 and passes = `BezierFitter::max_iterations`, export with dpi 72, layering 2, stroking 1 | `recovered_pipeline::vectorize` |
| reference LAPACK `dgesv` (dgetf2, dlaswp, two dtrsm sweeps) | The reference LAPACK `dgesv` the original links, rewritten in Rust: dgetf2 with the pivot's reciprocal multiplied in, dlaswp, the two dtrsm sweeps | `recovered_lapack::dgesv` |
| 0x49d360 / 0x49dcf0 / 0x479e10 / 0x49aee0 | Exact arithmetic: `fit_cubic` and `fit_quadratic` build the normal equations exactly as the original's `dgemm` does (sums over the samples in order, the ridge `1e-5` added after the sum) and `Cubic::evaluate` follows 0x479e10's operation order, so the 96 interval fixtures compare to the bit | `recovered_fit::fit_cubic`, `fit_quadratic`, `Cubic::evaluate` |

The pass's sparse solve (0x49bd00 calling 0x49bb40) is **not** a translation:
the original reads its CSR pointers from absolute address 0xc and faults
(the host's `--solver-probe` recorded the access violation before the host was
removed on September 22, 2026), so `recovered_optimizer::solve` is owned
elimination with partial pivoting. Its gradient-check diagnostic 0x49f970
(printf and CSV output) is not ported. The pass is off in every preset
(fitter+0x1c is constructed as zero and never registered); `fit_contours` keeps
that default and `fit_contours_optimized` / `--optimizer on` turn it on.
Under the improved defaults (the application's `owned_defaults`, CLI
`--defaults improved`) the pass runs guarded, an owned rule since September 22,
2026: `fit_contours_optimized_guarded` keeps the plain fit when the pass fails
(a singular or non-finite system, a zero-length junction tangent), which
otherwise fails the whole conversion, or when its Newton step raises the
objective 0x49ccb0 it descends, which the original keeps. `--defaults original`
runs the pass unguarded, so the `-high-optimizer` references are unchanged;
the accepted steps are the same to the bit either way
(`guarded_optimizer_keeps_the_plain_fit_when_the_step_fails_or_rises` on the
48 native rows, `guarded_optional_pass_keeps_the_plain_document_when_its_step_rises`
on 42 synthetic conversions). Measured on the seven samples at high quality
(September 22, 2026), E before -> after the step: under the original presets
the step raises E on six of seven (astronaut 4.05e4 -> 1.42e6, chelsea
3.59e4 -> 1.32e7, coffee 2.54e4 -> 6.13e5, blended logo 255 -> 913,
transparency logo 18.3 -> 33.7, unblended logo 22.5 -> 45.0) and lowers it
only on the small blended logo (6.04 -> 3.51); under the improved defaults'
settings it raises E on the three photographs (6.34e3 -> 3.96e5, 5.13e3 ->
5.21e5, 4.18e3 -> 9.11e4) and the unblended logo, so those keep the plain fit,
and lowers it on the three blended logos (10.0 -> 8.02, 246.1 -> 243.7,
39.7 -> 31.5), which keep the step. No sample made the pass fail.

The conversion moves the prepared pixels, the label image and the region
colours into the anti-aliased image model rather than copying them, and the
other presets drop them once the contours are built (September 22, 2026; at
the 16384-pixel limit the copied labels alone were 1 GiB). Contour
construction reads the label image in place as well.

Earlier recovered primitives include cubic evaluation, Bernstein/power basis
conversions, the flag boundary walk at 0x49a7a0 and gradient/damped Hessian at
0x49d8a0. The latter is **not** a constrained solver. These primitives are
documented in [FITTING.md](FITTING.md).

## Preserved decisions and quirks

- The zero-interior branch preserves the four seed points. One or two interior
  samples produce quarter/three-quarter handles and report only the first
  sample's line residual. Three samples use a quadratic, then degree elevation;
  larger counts use the cubic solve.
- The normal equations add `1e-5` to the diagonal. Reported SSE excludes this
  penalty. Repeated, endpoint and fully collapsed samples retain the original's
  results.
  The regularizer remains origin-dependent, as in the original.
- Contour positions wrap cyclically. Equal start/end means a zero-span fit;
  a full circuit must use an explicitly unwrapped end.
- Merge requires a delta strictly below the threshold; equality does not merge.
  A failed merge tries a backward boundary shift before a forward shift. The
  traversal advances to the updated successor after a merge.
- With `p` passes and ramp fraction `r`, the multiplier is
  `exp(ln(final / initial) / (p*r))`. After pass `i`, multiply while `i < p*r`,
  otherwise assign the final threshold. The post-pass threshold is retained.
- Starts receive state 5; interiors receive state 1 and the current curve index.
  The terminal record also increments the curve index. Outer endpoints are
  overwritten with state 4. Byte-only state writes preserve old indices.
  Ordered writes preserve aliasing when contour positions share node identities.
- Initial traversal processes contours and their node positions in original
  order. For each state-zero node it walks backward and forward over state zero,
  stopping at a different flag or the original node identity. It unwraps the end,
  schedules and marks that interval immediately, so later contours see the same
  shared state changes. Curve allocation count begins at zero.

- Preparation maps states 1/2/5 to 0 and 4 to 3; all other byte values survive.
  Scratch capacity is the longest contour plus two, or one with no contours.
- Refinement processes state-one runs in contour order and changes interiors to
  state two before visiting later shared nodes. It writes only fitted curve slots.
  The original allocation also contained unused terminal slots; the owned API uses
  `Option<Cubic>` to represent them explicitly.
- Derivative records use chord parameters from the emitted cubic endpoints.
  Zero samples leave the entire record untouched; one/two samples only set active
  count zero and contour/start/end metadata. Three samples write two gradient
  values and a 2x2 Hessian, using the quadratic scratch middle control. Larger
  counts write four coordinates. Inactive matrix entries, global offset and
  reserved words remain unchanged. The gradient has no ridge term.
- Finalization rotates away from an initial state-two run. Consecutive boundary
  nodes emit lines; state-two runs emit forward/reverse curve references. Curve
  edge metadata comes from the first interior position. Orientation compares both
  coordinates independently with inclusive tolerance `1e-6_f32 as f64`, the exact
  widened value at 0x8de900. It is not an exact double 1e-6 or Euclidean tolerance.

`fit_contours` exposes the ordinary fitting flow as one owned Rust operation on
already smoothed shared contours. It has no binary, FFI, Windows or Qt dependency.
`fit_contours_optimized` adds the optional pass: derivative records during
refinement, then one Newton step on every interior control with a tangent
continuity penalty of weight fitter+0x38 (10.0) at each junction whose node
metadata word has bit 0 clear. Two-coordinate records move their quadratic
middle and regenerate the cubic; a zero-length tangent is an error rather than
the original's NaN.

Finite-input/range validation, explicit errors, safe indexing and owned storage
are Rust API choices. They do not emulate native exceptions or x87 exceptional
values. The original computes in double precision, and the port repeats its
operation order: every control point, interval error, schedule, node state
and final SVG compares to the original's to the bit (`to_bits` equality in the
interval, refinement and optimizer tests, byte equality of the documents).

## Native evidence captured before the host was removed

`native/fit_fixtures.h` (removed with the host on September 22, 2026)
constructed controlled original records and invoked the unmodified routines.
The `--interval-fixtures` host mode emitted 288 rows, frozen
in `rust/fixtures/native-intervals.csv`:

- 96 interval fits covering every count branch, zero spans, cyclic wrapping,
  reversed node order, noisy curves, duplicates and collapsed coordinates.
- 96 schedules covering 0 through 12 passes, different ramp fractions and
  thresholds, actual merges and both shift directions. Interval positions and
  lengths match exactly; post-pass thresholds match numerically.
- 96 marking cases with nonzero initial indices and preexisting node states.
  Every resulting state, index and final allocation index matches exactly.

The separate `--initial-fixtures` host mode produced `native-initial.csv`, 48
actual native initial-traversal cases. They cover up to three contours with
shared, reversed and repeated node identities, all-zero or mixed zero/three
flags, and 0 through 12 passes. All final node states, indices and curve counts
match.
Five further fixture sets bring the total to **640 actual native rows**:

| File / capture mode | Rows | Coverage |
|---|---:|---|
| native-preparation.csv / --preparation-fixtures | 16 | Every byte flag, zero/multiple contours, capacity |
| native-derivative.csv / --derivative-fixtures | 96 | Every count branch, partial writes and sentinels |
| native-refinement.csv / --refinement-fixtures | 48 | Shared/reversed/repeated nodes, optional records on/off |
| native-finalization.csv / --finalization-fixtures | 96 | All three tags, wrapping, metadata and tolerance boundaries |
| native-whole-fitting.csv / --whole-fitting-fixtures | 48 | Complete native stage sequence versus owned `fit_contours` |
| native-optimizer.csv / --optimizer-fixtures | 48 | Four stages with records, objective, assembled gradient and Hessian rows, update with a supplied solution, objective after |
| native-topology.csv / --topology-fixtures | 48 (+ tables) | Label images through the unmodified contour construction: every node and contour record, border counts |
| native-smoothing.csv / --smoothing-fixtures | 42 | Label images through the unmodified smoother with presets 0, 1, 2, 6, 7, 8, 9: positions, states, flags, corner flags, areas |
| native-smoothing-probe.csv / --smoothing-probe | 42 | The same: phase-0 energy and gradient, then each phase's counters, energy and state, and the state after puncturing |
| native-aa-smoothing.csv / --aa-smoothing-fixtures | 42 | Blended images through the original import, preprocessing, segmentation and contour construction, then the unmodified anti-aliased smoother (presets 3, 4, 5): positions, states, flags, anti-inversion counts, corner flags, areas, rays, parities, the generator seed |
| native-aa-smoothing-probe.csv / --aa-smoothing-probe | 42 | The same per step: neighbour lists, the state after placement and perturbation, the phase-0 energy with every pixel's crossed flag and model colour, the gradient, each phase, puncturing, the closing pass |
| native-aa-smoothing-iterations.csv / --aa-smoothing-iterations | 42 | Every phase-0 iteration: the optimizer's counters, energy, best energy, state and gradient |
| native-aa-kd-tree.csv / --aa-kd-tree | 42 | The node set's ANN kd-tree in pre-order: every split's dimension, value and bounds, every leaf's point |
| native-aa-colour-model.csv / --aa-colour-model | 42 | The colour model at every pixel with two or more regions around it: region ids, weights, model colour |
| native-segmentation.csv / --segmentation-fixtures | 126 | Fourteen blended images under every preset through the original import, preprocessing and segmentation: the prepared image, the label image with its parent table, the region colours and records |
| native-segmentation-aa-stages.csv / --segmentation-aa-stages | 42 | The anti-aliased cases after each of the nine stages of their segmentation: labels, parents, the sub-pixel region records, the edge images, the boundary mask and every region's 42 features |
| native-segmentation-aa-sweeps.csv / --segmentation-aa-sweeps | 42 | The same cases after every sweep, rebuild and pass of the sub-pixel segmenter, with the cached costs |
| native-export.csv / export capture | 126 | The original exporter's own text, one process per case, including the CR LF line endings |

The optimizer rows bring the total to 688, the topology rows to 736, the two
smoothing captures to 820, the five anti-aliased captures to 1,030, the
segmentation capture to 1,156, the two sub-pixel captures to 1,240 and the
export capture to **1,366 actual native rows**, held in twenty `native-*.csv`
files (see [TOPOLOGY.md](TOPOLOGY.md), [SMOOTHING.md](SMOOTHING.md) and
[SEGMENTATION.md](SEGMENTATION.md)). Their capture replaced
only the diagnostic (with a dump of the assembled system) and the faulting
solver (with a chosen solution vector) inside the mapped process; the original
offset assignment, tangent/derivative assembly, objective and update ran
unchanged. Thirty rows carry nonzero systems, ten two-coordinate records and
258 skipped corner junctions. The host's `--solver-probe` recorded the original
solver's access violation (`c0000005` at 0x49bb90 reading 0xc).

The complete-flow fixtures called the four original stages in the order verified
at 0x4a1030, with optimization disabled. They compared every node flag/index,
written curve, untouched slot and final line/curve record. Empty/invalid input
has separate Rust checks.

The core tests execute independently of the original image using these frozen
values, replaying the captured rows to the bit. The engine tests add 126
synthetic whole conversions that compare byte-identically, the 96 interval
fixtures that compare to the bit against the reference LAPACK arithmetic of the
original, and the smoother's inputs for the 14 comparable anti-aliased cases
that equal the original's records field by field. Further tests cover threshold
equality, pass traversal, terminal indexing, ordered aliased writes and invalid
input. The optimizer tests reproduce all 48 rows (offsets, gradient, every
Hessian row's keys and values, updated curves, both objective values), check the
assembled gradient against central differences of the objective, exercise the
owned solver, and reject zero-length tangents. The topology tests reproduce all
48 label images (every node and contour record, border counts), check the
constructor's tables and refuse bad images. The smoothing tests reproduce all 42
runs and all 42 per-phase probes with every double compared for equality, and
cover the decision tree's edges, the engine's random generator and bad inputs.

Before the host was removed on September 22, 2026, `--backend recovered`
redirected ten entry points **in the mapped child process only**: 0x49e960,
0x49f8b0, 0x4a0dc0, 0x4a0f00, 0x49b560, 0x49ed80, 0x49ef50, 0x49b620, the
contour construction 0x499eb0 and the contour smoothing 0x497c30 (every
conversion; the anti-aliased ones were counted as `rust_smoothing_aa_calls`).
The original EXE and DLL files were never patched. `native/fit_bridge.h`
translated the original records and x86 calling convention; `rust-bridge/`
provided a temporary 32-bit C ABI around the portable algorithms. The adapter
retained native buffer metadata initialization at 0x49a560 and published
Rust-produced ranges into that buffer. It did not silently fall back on a
rejected Rust call. The initial-traversal adapter copied shared node identities
once, published the Rust flags/indices and invoked original array allocation
helpers for native downstream consumers. It recomputed the last Rust schedule to
preserve native scratch metadata; this redundant computation was temporary
adapter overhead. Original progress callbacks were unused in the headless host
and are not part of the portable traversal API. Schedule/mark counters included
calls performed inside the Rust traversal; the initial-traversal counter was one
per image job.

The preparation/refinement/finalization adapters only copied native records and
invoked existing array allocation helpers. Finalization read only referenced
curve slots; it never copied uninitialized native curve data into Rust. Direct
interval fitting also published the original quadratic scratch state, needed by
subsequent derivative construction. The fixture assembly call had a non-inlined
C boundary so the compiler could not retain temporary values in volatile XMM
registers across the opaque call.

Seven real inputs (four supplied logos and three local photo fixtures) run
through the engine at three source qualities plus the optional pass at high
quality, 28 conversions under `--defaults original` (six more, the
`-high-detail` references, freeze the improved defaults: 34 in all), and every
SVG is byte-identical to the frozen reference
the original engine produced before the host was removed
(`kit/fixtures/reference/*.svg`; `tools/verify_engine.py` writes
`analysis/engine-conformance.json`). The 126 synthetic whole conversions in the
core tests and the 14 comparable anti-aliased smoother records are byte-identical
to the frozen fixtures. Both CairoSVG and resvg produce identical source
comparison metrics. The reference's enlarged-logo checks and analytic circle
test therefore remain applicable to the ported output too.

The same seven images also run at high, medium and low source quality with
`--photo-seams native`. All **21 original-export comparisons** match byte for
byte, covering all nine basic presets. This separate comparison disables the
wrapper overlap policy and records settings for each job.

With `--defaults original --optimizer on` the seven images run again at high
quality: the port executes the optional pass unguarded, as the original would.
(Under the improved defaults the pass is guarded; see the optional pass above.) **All seven SVGs are
byte-identical** to the frozen `-high-optimizer.svg` references, the pass changes
at least five of them against the plain run, and the statistics JSON shows the
optional optimizer and derivative records on the owned side.

Current SVGs: `examples/engine/`. Hashes, metrics and renderer fidelity:
`analysis/engine-quality.json`. Conformance against the frozen references:
`analysis/engine-conformance.json`. Verified byte spans, including every
recovered entry: `analysis/fitting-evidence.json` and `analysis/disassembly/`.

## What remains of the original

Nothing does. The port is complete: the Vector Magic engine is rewritten in Rust
and the application runs with zero original machine code. There is no headless
host, no bridge, no native module and no native backend any more.

On September 22, 2026 the host and its harness were deleted: `native/` (the host
`native_engine_host.c`, every `*_bridge.h` and `*_fixtures.h`, `build.cmd`,
`build-rust-stages.cmd`), `rust-bridge/`, `app/src/native.rs`,
`app/tests/native.rs`, the stages DLL `vector_recovered_stages.dll`,
`tools/verify_native.py`, `tools/native_quality.py`,
`tools/recovered_quality.py`, `analysis/native-quality.json`,
`analysis/recovered-quality.json`, `analysis/native-engine-evidence.json`,
`analysis/native-math.csv`, `examples/native/` and `examples/recovered/`. The
CLI options `--backend native` and `--backend recovered` are gone: the default
backend IS the engine. The statistics JSON reports
`"backend":"recovered-rust"` with fields width, height, preset, segments,
seconds, photo_overlap, regions, nodes, curves, corners, optimizer_unknowns,
optional_optimizer, the per-stage `stages` seconds and `"original_code":false`
(plus `advanced` and `simplify` when those ran); the old `rust_*_calls` counters
no longer exist. Since September 22, 2026 `curves` counts the curves the
fitter made (its filled slots, `Conversion::curves`); it counted every slot the
fitter allocated before, including one terminal slot per scheduled run and the
one-step intervals that export as lines, so the figure is lower than before for
the same SVG. The launchers (`Start VectorMagik.cmd`,
`Rebuild VectorMagik.cmd`, `Capture App Preview.cmd`,
`kit/Launch-Vector-Magic.ps1`, `kit/Capture-App-Preview.ps1`; the duplicate
Run Vector Magic and Capture App Preview command files under kit/ were
deleted on September 22, 2026) no longer build
anything native: only
`cargo build --offline --release --manifest-path kit/app/Cargo.toml --target-dir work/rust-target --features desktop`.

The reference comparison now uses the original engine's own SVG output frozen
before the host was removed: `kit/fixtures/reference/` holds `<slug>-high.svg`,
`-medium.svg`, `-low.svg` and `-high-optimizer.svg` for
logo-with-blending-small, logo-with-blending, logo-with-transparency,
logo-without-blending, astronaut, chelsea and coffee (28 files).
`app/tests/engine.rs` compares the engine with those 28 frozen references, and
`tools/verify_engine.py` runs the CLI over the seven images at three qualities
plus the optional pass at high quality and asserts byte identity, writing
`analysis/engine-conformance.json`. `tools/engine_quality.py` is the
two-renderer fidelity report, `analysis/engine-quality.json` with
`examples/engine/`. `app/src/engine.rs` (`Options`, `Document`, `vectorize`)
replaces `native.rs`.

The old `--backend rust` research pipeline was archived on September 22, 2026
(history/archives/research-vectorizer-2026-09-22.zip); its rejected enlarged
geometry was never the path to completion. `kit/fixtures/samples/` holds the four sample PNGs and `LICENSE.txt`, their
credits.
`rust/fixtures/native-export.csv` (126 cases, the original's exporter text
captured one process per case) joins the twenty frozen `native-*.csv` fixtures
the core tests replay to the bit. Running the engine, the app and the tests needs nothing from the original
program.

Contour construction (0x499eb0) is ported: [TOPOLOGY.md](TOPOLOGY.md).
Contour smoothing (0x497c30) is ported: [SMOOTHING.md](SMOOTHING.md) (the
energy, its gradient, the conjugate-gradient optimizer with its quadratic
and blind line searches, the border pin and clamp, corner puncturing, and
for the anti-aliased presets the image measurement model at smoother+0x460,
the colour model with the sub-pixel placement and perturbation, the
neighbour lists over the ANN kd-tree and the inversion passes with the
after-iteration restart).
Preprocessing and segmentation (0x473850, 0x488d20) are ported:
[SEGMENTATION.md](SEGMENTATION.md) (the super-pixel segmenter for every
preset, the sub-pixel segmenter of the anti-aliased presets with its region
features, decision trees and refinement; 126 cases whole, 42 anti-aliased
cases per stage and per sweep).
Export is ported in `rust/src/recovered_export.rs`: the hole loops, the document
size, the colour groups, the shape walk with piece emission and the SVG writer
behind vtable 0x8dcfa0, with the exact `%.2f` binary rounding and CR LF line
endings. The writer formats a coordinate with Rust's correctly rounded `{:.2}`
and takes the digit-by-digit path only for the exact ties (an odd number of
eighths) and non-finite values; the text is the same for every input
(`fast_fixed2_matches_the_exact_digits`, September 22, 2026).

For strict original photo serialization, use `--photo-seams native`. The existing
default overlap policy for opaque photographs is a separately documented export
improvement; it is applied equally across current comparisons and is
not part of the recovered fitting algorithm. [RUNNING.md](RUNNING.md) contains
the complete offline build and verification commands.

The verification gate is the ordered list in `docs/TRANSFER.md` ("First
checks"), which `kit/tools/gate.ps1` runs whole; this record does not repeat
it, so the two cannot drift apart again.

Byte evidence now includes exact normal-return spans, the recovered constants
and the five-target preparation dispatch table. Finalization's
assertion/exception tail at 0x49ba79..0x49bb3f is explicitly excluded; safe Rust
validation replaces invalid-input handling and is not claimed to emulate it.
