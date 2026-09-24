# Recovered fitting primitives and caller decisions

September 20, 2026. Static disassembly plus direct native execution, preferred image base `0x00400000`. The direct execution ran in a headless host, which was removed on September 22, 2026; nothing in the current tree needs it.
Evidence: [fitting-evidence.json](../analysis/fitting-evidence.json), the explicit
spans it names in `analysis/disassembly/`, and the original indexed instructions.
`tools/recover_fitting.py` checks every instruction of the recorded spans (204
spans and 30,207 instructions on September 22, 2026, when the fitting's 47 spans
had grown with the rest of the port; kit/docs/REPRODUCE.md keeps the current
count) against both the extracted PE bytes and objdump decoding anchored at each
entry, plus 36 double constants and the preparation dispatch table. This verifies bytes and boundaries, not analyst interpretations. Running the engine, the app and the tests needs nothing from the original program.

## Historical direct native execution

Before it was removed on September 22, 2026, the headless host called 0x479e10,
0x49d8a0 and 0x49d360 in the unchanged original image. Those runs produced 1,056
evaluation checks and 32 actual native cases, which the Rust port compares
against gradients, Hessians, fitted controls and SSE. Parameter arrays were
copied using 0x49ca70 because the callees consume their owned storage.
The complete original segmentation-to-SVG pipeline also ran without the EXE
entry point or UI. See [NATIVE_ENGINE.md](NATIVE_ENGINE.md) for that history.
The static-only uncertainties below remain for routines not individually compared.

## Correction to the earlier constrained-fit attribution

`0x0049D8A0` ends with `ret 0x18` at **0x0049DCED**. A distinct function begins
immediately at **0x0049DCF0**, without intervening INT3 padding. The previous
padding heuristic included that next function, through `0x0049E372`, and thereby
incorrectly attributed its solve call at `0x0049E019` to `0x0049D8A0`.

The corrected label is **cubic_fit_derivatives**. This routine calls matrix
multiplication at `0x005BBD40` twice and copies a vector and a matrix to output
buffers. It does not call `0x0049D270`, solve for control points, update the
curve, normalize tangent directions, or impose tangent constraints. The exact
range replaces the old `0049d8a0-constrained_fit_candidate.asm`; generation tools
now use explicit ends so regeneration does not recreate the mistaken attribution.

## Cubic derivative outputs: 0x0049D8A0

Observed inputs are the cubic in ECX; an eight-byte by-value parameter-array
descriptor; a pointer to a point-array descriptor; the sample count; and two
output pointers. After the fixed prologue, these appear at stack offsets
`+0xB0/+0xB4`, `+0xB8`, `+0xBC`, `+0xC0`, and `+0xC4`. The cubic contains
four `(f64,f64)` points at offsets `0`, `0x10`, `0x20`, `0x30`.

| Instructions | Direct observation | Mathematical interpretation |
|---|---|---|
| 0x0049D8CA to 0x0049D954 | Allocate 2N×1, 2N×4, 4×4, and 4×1 buffers | Residual, design matrix, damped Hessian, gradient |
| 0x0049D9E0 to 0x0049DAB8 | Form cubic Bernstein values and fill alternating coordinate columns | Unknown order `[P1.x, P1.y, P2.x, P2.y]` |
| 0x0049DAD0 to 0x0049DAFF | Evaluate cubic, then subtract the sample | Residual is `C(t)-sample`, not its negative |
| 0x0049DB20 to 0x0049DB40 | Store double at 0x008DE7E0 on the 4×4 diagonal | Damping `lambda=1e-5` |
| 0x0049DB6D to 0x0049DBA0 | Multiply with alpha=2, beta=1 | `H = 2 A^T A + lambda I` |
| 0x0049DBE5 to 0x0049DC24 | Multiply with alpha=2, beta=0 | `g = 2 A^T r` |
| 0x0049DC2C to 0x0049DC4C | Copy 8 DWORDs, then 32 DWORDs | Four gradient doubles, sixteen Hessian doubles |

For `u=1-t`, `a=3u²t`, `b=3ut²`, the two design rows per sample are:

```text
[a, 0, b, 0]
[0, a, 0, b]
```

The mathematical objective underlying the gradient is `E = sum ||C(t)-q||²`.
The added diagonal belongs only to the Hessian output; **there is no `lambda*p`
term in the gradient**. Consequently these are a data-error gradient and a
damped data-error Hessian, not the derivatives of the absolute-control-point
ridge objective used by the direct fitter at `0x0049D360`. Their factor of two
also differs from that fitter's normal matrix. Do not conflate the two systems.
The native matrix is column-major. The Rust API exposes row-major entries;
symmetry makes the numerical entries equal.

The seven checked binary doubles are zero (`0x006FBE28`), two (`0x006FBEE8`),
one (`0x006FBEF0`), three (`0x006FDDD0`), `1e-5` (`0x008DE7E0`), a quarter
(`0x008DE4D8`), and three quarters (`0x008DCF28`).

The descriptor copy made by caller `0x0049ED80` via `0x0049CA70` owns a separate
parameter buffer; `0x0049D8A0` frees that passed copy at `0x0049DCCD`. It also
frees its local matrix buffers. The point-array descriptor is borrowed and the
output pointers belong to the caller. This is local ownership evidence, not a
recovered general container ABI. Rust borrows its inputs and returns owned values.

## Exact numerics

`0x0049D360` builds the direct cubic fit from a 2N by 4 design matrix, forms the
normal matrix with the reference `dgemm` (alpha 2, beta 1), adds the ridge `1e-5`
after the sum, and solves with the reference LAPACK `dgesv` ported in
`kit/rust/src/recovered_lapack.rs` (`dgetf2` with the pivot's reciprocal
multiplied in, `dlaswp`, two `dtrsm` sweeps). `recovered_fit::fit_cubic` and
`fit_quadratic` build the normal equations exactly as the original's `dgemm`
does, summing over the samples in order with the ridge added after the sum.
`Cubic::evaluate` follows the order at `0x479e10`: weights `(1-t)^2*(1-t)`,
`((1-t)^2*t)*3`, `((1-t)*t^2)*3`, `t^2*t`, accumulated as `((P0+P1)+P2)+P3`.
The quadratic fit of the three-sample branch is `0x49dcf0`. `0x49aee0`
multiplies by the reciprocal binomial. The 96 interval fixtures now compare to
the bit.

## Sample collection and parameterization: 0x0049C7B0

The collector receives a curve, contour index, start/end positions, two output
descriptors, and a flag controlling mutation. It reads a node-ID array and count
from contour record offsets `+0x18/+0x1C` (stride `0x60`); points and state bytes
come from node records of stride `0x28`, at `+0/+8` and `+0x1C` respectively.

It raises the end by the contour length until `end >= start`, then computes
`N = end-start-1`. It returns zero without collecting when `N < 1`. Only positions
strictly between the endpoints become samples. The first cumulative distance is
from the curve's P0 to the first sample; subsequent distances are between samples.
The final distance to the curve's P3 is included in the total denominator.
Distances use `sqrt(dx²+dy²)`, not squared distance or uniform index spacing.

```text
d[i] = distance(P0,q[0]) + sum(j=1..i) distance(q[j-1],q[j])
L = d[N-1] + distance(q[N-1],P3)
if L == 0: L = 1
t[i] = d[i] * (1/L)
```

The zero-total branch is `0x0049C9A5 to 0x0049C9B9`; the formula above describes
finite inputs. Repeated points remain, including parameters exactly zero or one.
All coincident points produce all-zero parameters, rather than uniform fallback.
When its final argument is nonzero, the collector writes state byte 2 to each
interior node (`0x0049C946 to 0x0049C955`). The Rust `chord_length_samples` function
ports the numerical portion only: callers supply endpoints and interior points.
It neither traverses native records nor mutates flags.

## Boundary selection: 0x0049A7A0

The four stack arguments are contour index, starting position, signed direction,
and a byte state to skip. All four recovered direct call sites pass direction
`-1` or `+1`. The routine saves the original **node ID**, steps first, and then
continues while the next node's state equals the requested state and its ID is
different from the original ID. It returns a modulo-contour-length **position**.

This makes two details observable: the starting node's own state is not checked
before the first step, and a repeated reference to the starting node stops the
walk even at another array position. It is not merely a search for a different
flag or a loop back to the original array index.

Initial traversal `0x004A0F00` searches around state-0 nodes, skipping state 0.
Refinement `0x0049EF50` searches around state-1 nodes, skipping state 1. Both
adjust the forward result by whole contour lengths until it is strictly greater
than the backward result; equal results therefore denote a full circuit for
these callers. The Rust helper returns just the modulo stop position and leaves
this caller-level unwrapping separate.

## Fit dispatch and derivative record

The sample count is the number of **interior samples**, not the count including
endpoints. Caller `0x0049E960` obtains it from the collector and branches as follows:

| Interior count | Observed behavior |
|---:|---|
| 0 | Return zero error; no numerical solve |
| 1 or 2 | Evaluate a two-point helper at 0.25 and 0.75 to set inner cubic points; error path evaluates only the first collected sample |
| 3 | Fit a quadratic using 0x0049DCF0, then convert through 0x0049A970 / 0x0049AEE0 |
| >3 | Fit a cubic with 0x0049D360 |

The low-count branch is now ported exactly for one/two samples in
`fit_low_count`. At `0x0049EC84` and `0x0049ECA9`, constants 0.25 and 0.75 feed
`0x0049AAB0`, a two-point linear interpolator, to set P1 and P2. At `0x0049ECD0`
the first parameter alone is loaded; `0x0049ECD8` calls the same LINE evaluator.
The squared difference from the first point is returned at `0x0049ED21`. There
is no loop over the second point and no evaluation of the emitted cubic. Thus
the returned error and emitted curve use different parameterizations. Tests
explicitly distinguish this behavior from cubic residuals and summing both points.

The three-sample branch converts its quadratic through these confirmed helpers:

| Helper | Confirmed behavior |
|---|---|
| `0x0049A970..0x0049AAA3` | Quadratic Bernstein points to ascending powers: `A0=Q0`, `A1=2(Q1-Q0)`, `A2=Q0-2Q1+Q2` |
| `0x0049A2F0..0x0049A3DC` | Copy a 3-by-2 coefficient block into a 4-by-2 destination at requested offsets; caller uses zero offsets and has cleared the destination |
| `0x0049AEE0..0x0049B04A` | Invert the degree-three binomial forward-difference relation, producing cubic Bernstein points |

The recurrence at `0x0049AEE0` is
`P_i=A_i/binom(3,i)-sum(j<i) (-1)^(i+j)*binom(i,j)*P_j`.
Because the caller leaves A3 zero, the composition is quadratic degree elevation:
`P=[Q0, Q0+2(Q1-Q0)/3, Q2+2(Q1-Q2)/3, Q2]` in exact real arithmetic.
Rust preserves the conversion through power coefficients; its f64 rounding is
not an exact x87 simulation. Sixty-four curves are compared against independent
quadratic De Casteljau evaluation, and general cubic coefficients against Horner.
All table ends are exclusive and verified from final returns.

The only indexed direct call to `0x0049D8A0` is at `0x0049EE38` inside
`0x0049ED80`. That caller collects the same samples, copies contour/start/end
metadata, and writes a derivative record:

| Record offset | Observed use |
|---:|---|
| +0x00 | Up to four gradient doubles |
| +0x20 | Up to a 4×4 matrix |
| +0xA0 | Active coordinate count: 4 for >3 interior samples, 2 for 3, 0 for 1 to 2 |
| +0xA4 | Cumulative coordinate offset assigned by optional pass 0x0049FC80 |
| +0xA8/+0xAC/+0xB0 | Contour/start/end indices copied by 0x0049ED80 |

For exactly three interior samples it calls quadratic derivatives at `0x0049E380`
and embeds the 2-vector / 2×2 block via `0x0049A3E0` / `0x0049A470`.
For count zero it returns without initializing the record; do not assume that
branch produces a fresh zero record. Refinement uses a **0xB8-byte record stride**
and a **0x40-byte cubic stride**, accumulates active coordinate counts at fitter
offset `+0x34`, and only builds derivative records when fitter `+0x1C` is nonzero.
The optional pass walks those records and assigns global offsets; its objective
and every consumer are recovered below.

## Optional pass 0x0049FC80: objective, assembly, update, faulting solver

September 21, 2026. `0x004A1030` calls `0x0049FC80` only when fitter `+0x1c` is
nonzero. The fitter constructor `0x0049B240` sets `+0x1c` to zero, `+0x38` to
10.0 (`0x006FBEF8`), `+0x10` to 0.5 (`0x006FD6B0`, the ramp fraction that had
no named default), `+0x18` to 10 and the two thresholds to widened floats
`1e-4`/`0.01` (`0x008DE8C0`/`0x008DE8B8`); the parameter registration at
`0x0046F900` binds only `+0`, `+8` and `+0x18`. No preset can turn the pass on,
so the shipped product never executed it. The removed host mode
`--optimizer-fixtures` set `+0x1c` directly.

**Objective, confirmed from two independent consumers.** `0x0049CCB0`
evaluates, over every record with nonzero `+0xa0`, the sum of squared
distances between the stored cubic (`0x00479E10`) and the interior samples of
the record's contour interval (`0x0049C7B0` without mutation), then over every
junction of two curve parts the weighted squared difference of unit tangents:

```text
E = sum_records sum_i |C(t_i) - q_i|^2 + w * sum_junctions |A/|A| - B/|B||^2
```

`w` is fitter `+0x38`. The pass itself never calls `0x0049CCB0`; it assembles
the gradient and Hessian of exactly this `E`: the records supply the data
blocks, and each junction adds `(-2/|A|)(uB - (uA.uB)uA)` scaled by `w` to the
end-side control (subtracted, because `A = P3 - P2`) and the mirror term to the
start-side control (added, `B = P1 - P0`); the Hessian blocks are
`(2/|A|^2)(dot I + uA uB^T + uB uA^T - 3 dot uA uA^T)` times `w` for each side
and `-w (-2/(|A||B|)) (I - uB uB^T)(I - uA uA^T)` for the mixed block, indexed
[b-coordinate][a-coordinate]. These are the derivatives of `w|uA - uB|^2 =
2w(1 - uA.uB)`; the constant `-2` is `0x006FD8F8`. The original's own
finite-difference check `0x0049F970` compares that gradient with `0x0049CCB0`
and the Rust test repeats the comparison.

**Junction walk** (shared by `0x0049CCB0` and `0x0049FC80`): for each contour
and each part `i` with cyclic successor, both parts must pass `0x0047E2C0`
(kind nonzero and record `+0xa0` nonzero); the node between them is the part's
`start_position` for kind 2, else its `end_position`; bit 0 of the first word
of that position's twelve-byte metadata triple at contour `+0x20` skips the
junction. Tangents come from `0x0047FF00` (end) and `0x0047FE00` (start): for a
four-coordinate record `P3 - P2` and `P1 - P0` read through `0x0047E1F0`, which
maps control `k` to `3 - k` for kind 2; for a two-coordinate record the cubic is
converted to its quadratic (`0x0047F650` cubic Bernstein to power, `0x0047F900`
keeps three coefficients, `0x0047F790` power to quadratic Bernstein) and the
tangents are `Q2 - Q1`/`Q1 - Q0`, swapped for kind 2. `0x0047E250` maps a
coordinate to its global index: record offset plus `c`, plus 2 for the end-side
control of a forward curve or the start-side control of a reversed one, and
always plus `c` alone for two-coordinate records.

**Assembly order.** Offsets `+0xa4` accumulate `+0xa0` over the records. The
gradient vector receives each record's gradient and the rows (a
`std::vector<std::map<int,double>>` holding the upper triangle, `+8` of the
container counting insertions) receive `H[i][j]` for `j >= i`. Then per
junction, per coordinate `i`: gradient A then B, the upper-triangular A and B
blocks for `k >= i`, then the mixed entries for `k` in 0..2 at
`(min(ib, ia), max(ib, ia))`. Insertion stores the value; an existing entry is
increased. The Rust port keeps this order.

**Update.** For every record and active coordinate, `0x0049B2B0` reads the
value (`P1.x, P1.y, P2.x, P2.y` at doubles 2..5 of the curve for four
coordinates; the quadratic middle from the conversion above for two),
subtracts the solution entry, and `0x0049B350` writes it back, regenerating a
two-coordinate cubic through `0x0049A970`, `0x0049A2F0` and `0x0049AEE0`.

**The solver cannot run.** `0x0049BD00` checks sizes, resizes the solution and
calls `0x0049BB40`, whose CSR conversion loads its three array pointers from
absolute addresses `0xc`, `0x10` and `0x14` (`a1 0c 00 00 00` at `0x0049BB90`).
The removed host mode `--solver-probe` called `0x0049BD00` on a one-term system
under a vectored handler and recorded `c0000005` at `0x0049BB90` reading
`0000000c`. The shipped optional pass therefore faulted before its update loop.
The Rust `solve` is owned Gaussian elimination with partial pivoting on the same
sparse rows; before the host was removed on September 22, 2026, its
`VM_OPTIONAL_OPTIMIZER=1` mode ran the original pass with that solver
substituted, which is how the assembly and update were proven inside
complete image jobs. `0x0049F970` (finite differences, `printf`, CSV files) is
silenced there and not ported; it changes no geometry.

The 48 rows of `native-optimizer.csv` capture the complete original pass with
only those two callees replaced: the assembled system, a supplied solution and
the resulting curves, plus `0x0049CCB0` before and after. See
[RUST_PIPELINE.md](RUST_PIPELINE.md).

## Initial fit decisions and orchestration

`0x004A0DC0` is an interval-fit-and-mark wrapper, not itself a recursive split
routine. It calls `0x0049F8B0`, traverses the resulting interval list, writes
state 5 to interval starts, state 1 plus a curve index at node `+0x18` to interior
nodes, and finally overwrites the two outer endpoint states with 4. These are
observed numeric states, not yet named corner/junction semantics.
This wrapper's schedule and ordered writes are now ported in `recovered_fit.rs`.
The terminal record also consumes one curve index. Byte-only state changes
preserve the previous index. Ninety-six frozen native calls confirm every state
and index; the wrapper now runs as Rust in `recovered_fit.rs`.

`0x0049A560` initializes consecutive one-step interval records of stride `0x20`.
`0x0049F8B0` performs a configured number of passes, invoking `0x0049F310` on
adjacent intervals with sufficient successors. Its threshold starts from fitter
`+0`, moves geometrically toward `+8` using an additional factor at `+0x10`, and
uses the iteration count at `+0x18` (registered engine `+0x1F20`). The
unregistered `+0x10` field is constructed 0.5 at `0x006FD6B0` (`RAMP_FRACTION`
in `recovered_pipeline.rs`, see the constructor above), the fraction of the pass
count used for the geometric threshold ramp; see
[RUST_PIPELINE.md](RUST_PIPELINE.md).

`0x0049F250` returns `E(merged)-E(left)-E(right)` using cached interval errors
and calls to `0x0049E960`. At `0x0049F325 to 0x0049F344`, a finite error increase
**strictly less than** the current threshold causes a merge; equality takes the
remaining path. The remaining code compares changes from one-node boundary
shifts, marks cached errors dirty, and adjusts interval positions and lengths.
The full numerical interval dispatch and merge/shift schedule are now ported
in `recovered_fit.rs` with owned storage. Ninety-six native interval cases and
96 schedules confirm all count branches, degeneracies, cyclic traversal,
strict comparisons, both shift directions and the post-pass threshold. The
temporary native record adapter was never part of the Rust core and is gone.
Where the original marks a changed interval's cached error dirty and refits it
when the walk comes back, the port stores the error it has just computed for
the new range (the merged interval, or both shifted ones): an interval's error
depends on its range alone, so the values are the refit's to the bit and every
merge saves one interval fit and every shift two (September 22, 2026).

The high-level order at `0x004A1030` remains preparation `0x0049B560`, initial
traversal `0x004A0F00`, refinement `0x0049EF50`, finalization `0x0049B620`, and
conditional optional pass `0x0049FC80` when fitter `+0x1C` is nonzero (ported
above; off in every preset).
The initial traversal is now ported too. Forty-eight
native fixtures cover shared, reversed and repeated node identities; final
flags, indices and curve allocation counts match exactly. Original numeric
state decisions are preserved across contours, rather than resetting each ring.

## Preparation, refinement and finalization now ported

`recovered_state.rs` implements 0x49b560, 0x49ef50, 0x49ed80 and the normal
path of 0x49b620. The preparation jump table at 0x49b604 maps states 1/2/5 to
zero, 4 to 3, and retains all other states. Its scratch capacity is max contour
length plus two, or one when there are no contours.

Refinement preserves contour order and shared node identities, fits state-one
runs, marks their interiors two, and accumulates active derivative coordinates.
The quadratic middle control must survive the preceding three-sample fit;
recovering it from the emitted cubic would introduce different rounding. Only
written curve slots are returned; untouched slots remain explicit
`None` in the owned fitting result.

Finalization emits seven DWORDs per part in this order:
`[index, kind, edge, start_node, end_node, start_position, end_position]`.
Kind 0 is a line (index is the contour position); kinds 1 and 2 reference forward
and reversed curves. A run uses the first interior position's edge metadata.
Direction compares the curve start with the preceding node independently in x/y,
using inclusive tolerance 9.999999974752427e-7, the widened float at 0x8de900.
Wrapping and repeated IDs retain the original order. The verified normal-return
span ends at 0x49ba79; the following assertion/exception tail is excluded.

The owned `fit_contours` API preserves the ordinary 0x4a1030 stage sequence,
without native progress callbacks or native allocations. Its inputs are smoothed
contours; it is not yet an image-to-vector API. Forty-eight complete native stage
sequences confirm the resulting flags, indices, curve slots and final parts.
`fit_contours_optimized` continues with the optional pass. See RUST_PIPELINE.md
for the frozen native fitting cases and their capture modes; the 48
contour-construction images are in [TOPOLOGY.md](TOPOLOGY.md).

## Rust scope and verification

`rust/src/fitting.rs` implements the cubic derivative outputs, numerical chord
parameterization, low-count fitting, power/Bernstein conversions, and the boundary
flag walk for canonical indices and +/-1 steps. Safe slices, enums, finite/range validation, and overflow errors are new
API design choices. They do not reproduce C++ allocation or exception behavior.
The derivative helper accepts empty, endpoint-only, and repeated samples, even
though the recovered dispatcher normally calls its cubic branch only above three.

Nine new tests cover a hand-computed derivative case, 48 independent NumPy dense
matrix cases, finite differences of a separately evaluated De Casteljau error,
absence of gradient ridge, damping magnitude, endpoint/repeated samples,
nonuniform and collapsed chord lengths, repeated node IDs, 2,568 exhaustive small
ring walks, and invalid/overflow inputs. These supplement the original ten tests.
NumPy fixtures validate the reconstructed algebra, not original executable output.
Rust/Python f64 does not emulate x87 extended intermediates or exceptional values.

Preparation, refinement, derivative construction and finalization now live in
`recovered_state.rs`, with the ordinary orchestration exposed as owned
`fit_contours`. Frozen native fixtures verify partial record writes, shared
traversal, curve orientation and complete fitting results.
`recovered_optimizer.rs` holds
the optional pass; `0x0049C220` turned out to be the statistics printer of the
gradient-check diagnostic, not part of the update. The scheduler and interval marking now execute as Rust replacements;
seven source images produce byte-identical SVGs against the frozen references,
28 conversions under `--defaults original` and six more under the improved
defaults, 34 in all, checked by `tools/verify_engine.py`.
See [RUST_PIPELINE.md](RUST_PIPELINE.md). The independently
designed research pipeline (`curves.rs`, `topology.rs` and `pipeline.rs`, not a
port of the optimizer) was archived on September 22, 2026 in
history/archives/research-vectorizer-2026-09-22.zip with its page; none of its
files remain in `kit/`. Three further compatibility tests cover the conversion/low-count
rules above; image pipeline tests are reported separately in TEST_RESULTS.md.
