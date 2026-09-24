# Contour smoothing: 0x00497C30 recovered

The contour smoother at engine+0x1888 (`ContourSmoother`, vtable 0x8de68c)
takes the shared node set and the closed contours that contour
construction leaves and moves the node positions by minimising an energy
in up to three phases. `kit/rust/src/recovered_smoothing.rs` is the owned
port of everything the smoother runs, with the anti-aliased half in
`recovered_smoothing/anti_aliased.rs` and the neighbour kd-tree in
`recovered_smoothing/kd.rs`. `--smoothing-fixtures` and `--smoothing-probe`
in the host froze 42 label images through the unmodified original before
the host was removed on September 22, 2026; `--aa-smoothing-fixtures`,
`--aa-smoothing-probe`, `--aa-smoothing-iterations`, `--aa-kd-tree` and
`--aa-colour-model` froze 42 blended images through the original import,
preprocessing, segmentation and contour construction and then the
unmodified anti-aliased smoother. The port runs the entire pipeline in
Rust with zero original machine code, byte-identical to the original:
plain smoothing for the unblended presets and anti-aliased smoothing
with the image measurement for the blended ones (presets 3, 4 and 5,
`Shared::is_anti_aliased`). The anti-aliased half is described at the end.

## Records and parameters

The smoother reads the node records (stride 0x28) and contour records
(stride 0x60) of [TOPOLOGY.md](TOPOLOGY.md) through its own pointers
(+0x600 contours, +0x604 nodes, +0x61c the node set, +0x610 the source
image for the canvas size, +0x5fc the shared block at engine+0x208 whose
+4 is `is_anti_aliased`). It changes node `+0`/`+8` (position), `+0x1c`
(state 3 at a punctured corner), the first word of every twelve-byte edge
entry (bit 0 corner, bit 3 local maximum of the turn) and contour `+8`
(the signed polygon area of small regions). Contour word 0 is the region's
pixel count, a segmentation output that construction passes through;
regions of at most seven pixels are treated as small.

Measured and defined: after contour construction the entries' first word
is uninitialised heap memory, and the energy reads its bit 0 as the corner
flag in the phases before puncturing writes it. The recovered pipeline
gives it zero when it publishes the topology (that reproduced the seven
samples), and the fixtures define it the same way.

Parameters (PARAMETERS.md, at the smoother): measurement types +0x4,
prior types +0x10, prior strengths +0x20, length penalty weights +0x38,
corner puncturing +0x50, enabled phases +0x5c, optimizer types +0x68,
air pressure weight +0x78, perturbation range +0x80, length barrier
weight +0x88 (unused by the recovered code), puncture thresholds +0x90/+0x98
(unused by the recovered code), anti-inverse-potential scales +0xa0/+0xa8,
and three 0xc8-byte optimizer blocks at +0x208 copied into the optimizer
(smoother+0xb8, offset +8) by 0x4936c0 for each phase. The block's
registered fields are the `cg_*` parameters; `cg_blind_blend_latest` is at
block+0x50 (its registration, not +0). Four fields are unregistered and
come from the block constructor 0x4acb10: +0x10 = 1 (save the state before
a line search), +0x28 = 0.5 (eps decay per round), +0x38 = 1e-4f (finite
difference step, unused with the analytic gradient), +0x40 = 0. Every
preset uses prior code 3, optimizer type 0 and line search type 0 or 5.

## The energy (0x4941e0) and its gradient (0x495b00)

For phase `p` with prior strength `s`, length penalty `w`:

1. At every position of every contour, with `a` the edge arriving at the
   node (from the previous node), `b` the edge leaving it, `na`/`nb` their
   pixel step counts (entry word 2): if the node's flag byte has 0x10, the
   code-0 prior scaled by `anti_inv_pot_prior_scale / s`; then, for a small
   region the code-0 prior 0x493ee0 `|b/nb - a/na|^2`, otherwise, unless
   the entry's bit 0 is set, the code-3 prior 0x494730
   `sqrt(2 - 2 ua.ub + 0.001) + w (|b|/nb - |a|/na)^2` with unit vectors
   `ua`, `ub`. The sum is multiplied by `s`.
2. Air pressure for every small region: the signed area (0x47ee30, half the
   shoelace sum over consecutive node pairs, written to contour+8) minus
   the pixel count, squared, times `s * air_pressure_weight`.
3. The measurement term: type 0 sums the squared displacement of every node
   from its single-precision copy (0x47f4f0); type 2 sums the square of the
   part of the displacement beyond 0.7 px; type 1 is the anti-aliased image
   model (`anti_aliased.rs`, described at the end). Nodes with a non-zero
   byte +0x1e add
   `byte * anti_inv_pot_meas_scale * |displacement|^2`.

The gradient is the analytic derivative in the same order (0x494840 and
0x494580 return the derivatives with respect to the previous and next
node), with the air-pressure term using the area the last energy
evaluation stored, and finally zeroes the components the border pin fixes:
node 0's y, the left column's x, the last left and first right node's y,
the right column's x, the last right node's y, the top and bottom rows' y.

## The optimizer (0x4ada10 at smoother+0xb8)

0x496740 prepares: the canvas size, the gradient array (0x495600), the
optimizer (0x4acca0: two state variables per node, pointers from the vtable),
and the near-border list (0x482490: interior nodes with x or y within
1.1f of the canvas edge, kept by 0x47f2d0). 0x497a80 runs a phase when
it is enabled: it validates the codes, selects the prior routines, copies
the block and, for optimizer type 0, calls 0x4adf00: eps = `cg_max_eps`,
then the loop:

* energy and gradient, direction = -gradient, `gg` = its squared norm;
* per iteration: remember the best energy, `step` (0x4ad770: the largest
  |direction component| when `cg_max_step_size` > 0, then the line search),
  the after-iteration hook (progress only without anti-aliasing), the
  stall test (`rel = (best - energy) / best` when best > 0; stalled when
  `rel < cg_rel_tol`, or when `delta < cg_abs_tol` and `delta >= 0`; more
  than `cg_knock_out_count_down` stalls after `cg_min_iter` iterations
  ends the run), the new gradient, beta = new/old squared norm (0 when the
  old is below 1e-30), direction = beta * direction - gradient, and a
  restart to steepest descent (0x4ad530) when the direction is not a
  descent direction or the iteration count is a multiple of
  `cg_iter_for_restart`; `cg_max_iter` iterations at most;
* the quadratic line search 0x4ad010 (types 0 and 5): eps is capped so
  that eps times the largest direction component stays within
  `cg_max_step_size`; each round saves the state, moves eps and 2 eps
  (0x4ace50, then the state hook), fits a parabola through the three
  energies and, when it opens upwards, steps its minimum from the current
  point, capped the same way; the best of the four energies wins (the
  saved state plus k eps for k in 0..2); eps is multiplied by 0.5 and
  clamped to [`cg_min_eps`, `cg_max_eps`]; rounds stop when the first
  energy was zero, the relative decrease is within
  `cg_quad_line_srch_rel_tol`, or `cg_quad_line_srch_max_iter` is reached;
  with `cg_use_eps_from_step` the next eps is |total step| *
  `cg_step_fraction`, clamped.

After every move 0x493640 runs 0x47e480 (the border nodes back onto their
edges: x = 0 or width, y = 0 or height, the corners fully) and 0x47f2d0
(near-border nodes kept 0.01 inside the canvas). The gradient zeroes the same
components, so the port walks 0x47e480's pattern once (`Canvas::border_pins`,
in the original's write order) for both the positions and the gradient.

## Corner puncturing (0x494a90)

After a phase with `do_puncture_corners`, every contour of at least seven
nodes is walked with a window of seven nodes: six unit edges (a coincident
pair stays a zero vector), five turns `2 - 2 u_i . u_{i+1}`, and the
decision tree 0x4adfb0 with thirteen constants over the middle turn `t2`,
the sum of the four side turns, the sum of three of them with the larger
of the two adjacent turns dropped, the far turn on that side, the smallest
side turn and whether `t2` is the largest of the five:

* `t2 < 0.611194`: a corner when `t2 >= 0.282594`, the side sum is below
  0.322933 and either the side sum is below 0.0286571, or `t2 >= 0.483023`,
  or the three-sum is below 0.00209524;
* otherwise, when `t2` is the largest: a corner when the smallest side
  turn is below 0.097537 and `t2 >= 1.42496`, or the three-sum is below
  0.122841, or the side sum is below 0.445642, or the far turn is below
  0.0046405;
* otherwise a corner when `t2 >= 1.72002`.

A corner sets the node's state to 3 and bit 0 of its entry; a largest
middle turn sets bit 3. The window slides by one node, recomputing only the
newest edge and turn. (The helpers 0x493810 and 0x4938e0 fill features the
tree does not read.)

## Evidence

Spans verified byte for byte by `recover_fitting.py`: the 35 routines above
(orchestration, setup, phase runner, energy, gradient, both priors and
their gradients, area, displacement, near-border list, pin, clamp, the
optimizer's init, release, fresh run, loop, step, line search, move and
restart, puncturing, the tree and its two helpers, the state and restart
hooks, block copy, gradient resize, the four vtable accessors and the
random generator 0x468bc0), 36 constants including the tree's thresholds.

`native-smoothing.csv`: 42 label images (12 to 32 pixels a side: discs,
diagonal splits, blocks, rings, specks and blocky noise) through the
unmodified contour construction and smoother with presets 0, 1, 2, 6, 7,
8 and 9, the parameters read from the smoother, and every node position,
state, flag byte, entry flag and contour area the original leaves.
`native-smoothing-probe.csv`: the same images with the phase-0 energy and
gradient at the prepared state, then each enabled phase's iteration,
line-search and restart counts, final energy and state, and the state after
puncturing, separately. The core tests reproduce both files exactly (every
double compared for equality). In the pipeline the Rust engine reproduces
the original records and export byte for byte.

## The anti-aliased half

With `is_anti_aliased` (presets 3, 4, 5; measurement type 1) the original
also runs, all gated on the shared block's +4, and `anti_aliased.rs` ports
in operation order:

- **Setup.** 0x4880a0 builds the image measurement object at smoother+0x460:
  per pixel a list of edge events (32-byte records: code, edge, contour, x,
  y as floats, the clockwise perimeter parameter, the contour's winding
  angle) and a model colour that starts as the pixel's region colour.
  0x482190 puts every (contour, position) into an ANN kd-tree (two
  dimensions, bucket size one, split rule 5, which the constructor's table
  at 0x5b92f0 resolves to the sliding midpoint split 0x5bb2b0) and 0x4822b0
  lists, for every node not yet flagged, the min(node count, 50) nearest
  positions within distance 5 that are not the node itself (eps 1e-12; the
  original keeps the search order, the port compares the lists as sets
  because only sums over them are read). With `with_prepare`, 0x495a30 runs
  0x495720 on every interior node: the colour model 0x4a2520 is evaluated at
  the 2x2 pixels around the node's grid position, the weight of every region
  but the first is accumulated per pixel, and the node moves by half the
  diagonal unit vectors at smoother+0x620 (-0.70710678118654746, one unit
  below Rust's FRAC_1_SQRT_2) scaled by the differences, clamped to +-0.333;
  then two draws of the engine's generator 0x468bc0 (started from seed 1
  at a fresh process's 0xa21d2c, which segmentation also advances) perturb
  it by `perturbation_range`.
- **The colour model** (engine+0xa58, `ColourModel`): 0x4a1de0 collects the
  pixel's region and the distinct regions of its eight neighbours (offsets
  set by preprocessing). Two regions: the projection of the pixel colour on
  the segment between the region colours (float arithmetic, double divide)
  pulled towards the middle by 0.01/(d.d + 0.02), clamped to [0, 1]. More:
  0x4a1f20 forms the Gram matrix of the three colour channels with a 0.01
  ridge and its Cholesky factor (0x4a1980, column-major, the factor in the
  upper triangle, stride 9), and 0x4a2040 solves the constrained least
  squares by an active set (the sum-to-one row first, a zero row for the
  most negative weight below -1e-4 each round), with the library's
  `dgemv`/`dgemm` 0x5bc210 / 0x5bbd40 and the solve 0x4a1b60 (forward with
  the factor's column, back with its row). The pixel colour goes in with
  channels 0 and 1 as exact doubles and channels 2 and 3 rounded through
  floats first (0x4a2520 stores them so).
- **Energy** 0x488710: the changed set and every cell are cleared; per
  contour the winding angle (atan2, wrapped to (-pi, pi]) flags the contour
  when it is not a full turn; 0x485e60 walks every edge through the grid
  (a DDA over the pixel lines, registering crossings with 0x485d50); per
  crossed pixel 0x4867d0 closes each contour's events into loops along the
  perimeter (0x4865b0 inserts the pixel corners, 0x4852d0 drops runs of
  fewer than three events) and 0x4868f0 turns the loops' signed areas
  (shoelace over the float coordinates) into region weights and the model
  colour, resetting inconsistent pixels to the source and flagging their
  contours; the energy is half the sum over all pixels of the squared float
  differences to the source, accumulated in doubles.
- **Gradient** 0x487fe0 / 0x4870b0: every pixel is re-evaluated, its
  residual weight is half the dot product of the difference with the
  contour colour (minus the parent's where an enclosing contour exists),
  and each event pair (an event and the next of its loop, the last pairing
  with the loop's first) adds the area derivative to the two nodes of the
  edge, split by the crossing parameter for boundary events.
- **The after-iteration hook** 0x4939a0 (vtable +0x20): every `every`-th
  iteration (smoother+0xb0, 10) runs 0x4849a0 over all contours, otherwise
  0x485be0 over the flagged ones: 0x484630 walks a contour against the
  neighbour lists, counts ray crossings (0x4844f0 with the direction
  0x47f0c0 and the crossing code 0x47e300) for the contour's parity, sums
  the signed crossings of every edge pair (0x47fff0, the pinch case
  0x4811a0) and marks inverted nodes (flags |= 0x16, aux = 3, the
  anti-inversion count 0x47edf0 runs down); a mark restarts the conjugate
  gradients (beta 0, the direction reset to minus the gradient) and
  re-evaluates the energy.
- **Line search type 5** 0x4ad3e0 (the anti-aliased optimizer blocks:
  `num_seeing` 2, `num_blind` 50, blend 0.5, one quadratic iteration): in a
  period of seeing plus blind iterations the first ones run the quadratic
  search 0x4ad010 and remember its step for the blind ones (optimizer+0x138):
  iteration 0 of each run sets the remembered step to that search's step,
  and every later seeing iteration blends the new step into it by
  `blind_blend_latest`. The rest step blindly by 0.8 of it, restoring the
  state and restarting the period when the energy got worse.

The former host header `aa_smoothing_fixtures.h` froze the anti-aliased
runs whole (`native-aa-smoothing.csv`), after every step
(`native-aa-smoothing-probe.csv`: neighbour lists, the state after
placement and perturbation, the phase-0 energy with every pixel's crossed
flag and model colour, the gradient, the state after every phase and after
puncturing, then the closing pass), every iteration of phase 0 with the
optimizer's counters, energies and states (`native-aa-smoothing-iterations.csv`),
the kd-tree in pre-order (`native-aa-kd-tree.csv`) and the colour model at
every pixel with two or more regions (`native-aa-colour-model.csv`) before the
host was removed on September 22, 2026; `aa_tests.rs` reproduces all five
to the last bit for the 42 cases (seven patterns, three presets, two
shifts; junction patterns exercise the constrained solve). The former bridge
handed the adapter the source pixels, the label image resolved through its
parent table, the region colours, the contours' region ids, colour bytes and
enclosing contours, the seed and the smoother's period and unit vectors, and
wrote back the node flags, the contours' rays and parities and the seed; the
entire smoothing stage now runs in Rust with no bridge or host.

## Performance (September 22, 2026)

The port is measured before it is changed: `python kit/tools/engine_timing.py`
tabulates the seconds of every stage that `verify_engine.py` records for the
28 references, and `cargo build --features profile` (`kit/rust/src/profile.rs`)
prints wall-clock spans inside the smoother after a conversion. Smoothing is
83% of engine time over the references; on logo-with-blending at high quality
the anti-aliased image energy 0x488710 was 56%, most of it a loop
that recomputed the squared model difference of all 160,000 pixels on each of
468 evaluations although only a few thousand cells carried events.

Every change below keeps the arithmetic and its order; only where values
are stored and how often they are recomputed differs. Each is exact by
construction and was proven so (the 42 native probes still match iteration by
iteration, the 28 references byte for byte), and each was measured by
`kit/tools/engine_ab.py` (the old and the new binary alternating on five
conversions, best of three, idle machine; a single profile run under load had
once suggested -44% for a change worth -7%, and whole-gate timings on a busy
machine swing by 30% either way, so only the alternating A/B counts).

1. The energy's per-pixel term. `Measurement.pixel_sum` holds every pixel's
   squared model difference and is kept current by every writer of `model`
   (the setup, and the energy for the pixels it processes); the energy loop
   still visits every pixel in row order and adds the same values in the same
   order, but computes a term only for the pixels that carried events. Cells
   are cleared through the `touched` list that `register` fills instead of
   sweeping the whole grid. -7% over the five (2.50 s -> 2.19 s on the
   blended logo at high).
2. The gradient reads the colour the energy computed. 0x4870b0 calls 0x4868f0
   again for every pixel before differentiating it; that computation depends
   only on the pixel's events and the contours' colours, neither of which
   changes between the energy and the gradient that follows it, so the energy
   keeps each pixel's enclosing region in `cell_f4` and the gradient reads it
   and `model` back. The gradient also walks a pixel's events by index instead
   of copying them, and 0x4852d0's run filter slides the kept runs down in
   place instead of building a new list per pixel. -17.6% over the five
   against the binary with change 1 (-19% on the blended logos).
3. The event pool. `register` used to push into one `Vec` per pixel (160,000
   of them, each a separate heap block, ~33,000 pushes per evaluation landing
   in ~9,000 of them); it now appends to one flat staged list, `group` sorts
   the list by pixel with a stable counting sort over `touched` (each pixel's
   events stay in registration order), and `process` closes each pixel's loops
   in a scratch list and appends the result to one processed pool that the
   colour, the gradient and `invalidate_pixel` address by (start, length).
   This is the original's own layout (the pool at measurement+0x1c). The
   inversion pass walks its neighbour lists by index instead of cloning
   them. -22.7% over the five against the binary with change 1 (-26% on the
   blended logo at high, 1.75 s -> 1.30 s in that run), so about -28% over
   the five against the port as it stood before the improvement pass.
4. No copies on the way through a pixel (overnight, September 22, 2026).
   `process` used to copy a pixel's events from the pool into a scratch list,
   close the loops there and copy the result into the processed pool; it now
   appends the pixel's events to the processed pool and closes the loops in
   place at that tail (`process_cell` takes the tail's offset; the corner
   insertions and the run filter already worked by absolute index). The
   raster loop also stops cloning each contour's node list per evaluation.
   With about 9,150 pixels and 33,300 events per evaluation on the blended
   logo, that is two copies of every event fewer per evaluation: -7.5% over
   the five (best of five, idle machine; -8% to -11% on the three
   anti-aliased cases, 1.44 s -> 1.32 s on the blended logo at high, the
   two non-anti-aliased cases unchanged within noise), byte-identical on the
   28 references and the 126 synthetic conversions.

With the `profile` build the blended logo at high now splits (relative
figures, loaded machine): image energy 0.79 s of which rasterization 0.25 s
and the per-pixel closing and colouring 0.46 s; image gradient 0.36 s;
inversion passes 0.13 s; the priors and the rest 0.1 s. What remains is
arithmetic the port must keep: the 468 evaluations the line search asks for,
one `atan2` per edge per evaluation for the turning angle and the
160,000-pixel walks that keep the summation order. The inversion pass's
segment tests have an exact early answer since September 22, 2026:
`segments_cross` returns 0 for two segments whose bounding boxes are more
than 1e-3 apart in x or y, where 0x47fff0 would first normalise both (two
roots, two divisions) and then answer 0 anyway. A non-zero answer needs an
endpoint within 1e-4 of the other segment or a proper crossing, and either
puts the boxes within 1e-4; a NaN never reads as apart, and coordinates
other than 0 or of a magnitude in 1e-100..1e6 take the full test, so no
product overflows and no segment's squared length underflows (a segment
under 1e-154 long would leave its direction unnormalised and the full test
can then call a far point "on" it). Equal to the full test on 240,000 random
and constructed pairs (`inversion_segment_test_early_answer_matches_the_full_test`),
bit-exact on the 42 probes, the 126 synthetic conversions and the 28
references; `engine_ab.py` against the build without it, loaded machine:
-7.5% over the five (best of seven; -11%, -7% and -6% on the three
anti-aliased cases) and -4.6% (best of nine; -2%, -10%, -3%), the two
non-anti-aliased cases unchanged. The pinch test 0x4811a0 keeps the full
test (its points are unit directions around the pinch node). Tried and
dropped for want of a measurable gain: walking only the
touched cells in the gradient (-0.2%); the per-pixel phase on threads. The
phase is exact in row bands (each pixel's closing and colouring depends only
on its own events, the contours and the source pixel; the flagged contours
are a set; the sum is walked in row order afterwards), and both ways of
running it were built and proven on the 42 probes: scoped threads spawned
per evaluation measured -0.1% (eight thread starts per evaluation cost what
the phase saves), and persistent workers holding band-owned arrays, with a
band handed over by channel per evaluation and no unsafe code, measured
-3.3% with eight bands and nothing with four. The phase is about 1 ms per
evaluation; the wake-ups and the per-evaluation migration of the bands'
arrays between cores (the main thread reads every band's results for the
sum and the gradient) eat the rest, so the core crate stays single-threaded
and dependency-free.
