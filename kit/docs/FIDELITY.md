> Historical record of the independent Rust research backend (`--backend rust`), archived on September 22, 2026 in history/archives/research-vectorizer-2026-09-22.zip. Its metrics did not establish usable enlarged logo geometry. Nothing on this page describes the current program: the CLI and the desktop run the owned port of the original engine (`kit/app/src/engine.rs`, no original code, binary or external process), and its output is measured by `kit/tools/engine_quality.py` and `kit/tools/quality_round.py`. [NATIVE_ENGINE.md](NATIVE_ENGINE.md) records the headless host, removed the same day.

# Image fidelity and topology refinement

This September 20 update is independently designed Rust behavior. It does not
claim to recover the original optional optimizer. The established correction
at `0x0049D8A0` remains: that routine builds a gradient and damped Hessian, with
no solve or tangent constraints. Native evidence and compatibility tests remain
separate from this image-quality work.

## What changed

`curves.rs` evaluates all five available reparameterization candidates and keeps
the accepted candidate with the smallest source-point squared error. Corner
selection also checks whether a turn is concentrated locally, retaining sharp
moderate-angle corners without making every sampled arc a corner.

`raster_fit.rs` refines sparse lines and cubic handles against the input pixels.
For closed polygons, each edge contributes signed area to each pixel using
Green's theorem and an analytic integral of a clamped linear function. Binary
regions sum contributions from every boundary, including holes and opposing
edges crossing the same pixel. This fixes the former independent-hole objective,
which could consume the same pixel twice. Cubics are flattened at a maximum
control-polygon sampling step of 0.5 pixels (up to 512 samples per segment), so
this remains an approximation to exact cubic area.

Open multicolor chains use nearest-edge tangent coverage; pixels incompatible
with that palette pair are excluded. A spatial tree supplies signed distance
and tangent direction. This local halfplane model is approximate at corners and
where several regions meet. Source-driven steps are accepted only if their
objective improves. Smooth handle directions are coupled, while endpoints and
short junction edges stay fixed. Proper self-crossings are rejected. Almost
straight cubics with folded handles become lines; the optimizer cannot recreate
those folds. Global inter-chain intersection freedom is not proved.

Binary refinement includes a 0.67/0.33 coverage margin to preserve pixel
classification across rasterizers. A local test found the cached resvg renderer
quantizes horizontal-edge coverage in approximately quarter-pixel vertical
steps, while CairoSVG gives finer area coverage. The margin is an optimization
goal, not a relaxed test threshold: both renderer suites still threshold alpha
at 128 and require the original component/hole counts. The margin can slightly
increase antialias intensity error while stabilizing contacts.

When source or predicted 3-by-3 neighborhoods show an unresolved connectivity
risk, the optimizer can split an existing segment using de Casteljau and permit
a local corner. It adds at most eight segments per binary document, with at most
four splitting rounds. It does not emit a rectangle for every source pixel.
Tests continue to enforce the original small segment ceilings for smooth shapes.

Work is bounded: multicolor refinement skips chains above 300 segments and
12,000 nearby pixels; binary refinement skips documents above 600 initial
segments or 12,000 nearby pixels. Each optimization call permits ten million
sample evaluations, counting both trial directions. Multicolor chains receive
two calls; binary chains may receive up to five calls with intervening local
repairs. These caps preserve usability but can leave very complex contours less
refined. Pixel refinement is a quality cost; the separate contour-fit benchmark
must not be presented as total conversion speed.

## Palette and SVG behavior

Palette learning uses a full 3-by-3 neighborhood to distinguish uniform artwork
interiors. Strongly supported exact colors retain their opacity; intentional
partial alpha is tested separately. Images with more distinct colors than the
requested palette and at most half their pixels in uniform neighborhoods use
continuous-tone handling: palette fitting uses the full raster, region labels
use nearest colors, and small-region merges prefer similar neighboring colors.
This is an explicit heuristic, not a semantic photograph classifier. It avoids
turning photographic shades into antialias strips or letting uniform-background
seeds consume detailed areas. The CLI and desktop accept up to 64 colors.

Opaque SVG regions paint enclosing components first. An opaque canvas backing
prevents transparent cracks; transparent documents use one nonzero-winding
coverage clip to preserve holes. A common silhouette color can underpaint the
clip once. The transparent and opaque cases are covered by nested-hole/island
and junction rendering tests. Continuous-tone output uses a 0.5-pixel stroke in
each face's own color to cover antialias seams, with common stroke settings in a
group. This deliberately overlaps adjacent colors by 0.25 pixels; it is not
identical to the unstroked shared-curve geometry.

SVG contains editable paths, not embedded bitmaps. Compact and verbose exports
use identical four-decimal coordinates and have matching renders. Shared segment
counts, serialized commands, and file size measure different things; photos
need substantially more paths than simple logos.

## Validation

See `../TEST_RESULTS.md` and the JSON reports for current measurements. The core
has independent polygon-clipping/shoelace references, 3,731 halfplane comparisons,
5,832 nearest-frame queries, curve subdivision/tangency checks, rejected-step
rollback, pinned edges, folded-handle rejection, palette and color-merge tests.
The original recovered-math references are retained unchanged.

Both CairoSVG and the desktop's resvg renderer run all 52 authored/stress cases
at their exact input dimensions. The three public-domain/CC0 photo fixtures
are locally installed scikit-image samples, explicitly resized once to at most
256 pixels; attribution, original hashes and transforms are in
`../fixtures/photos/provenance.json`. Nine photo settings cases run the actual
CLI and both renderers. Their comparisons are against the saved pre-photo-change
reconstruction, not the original program. No network downloads were involved.

Photographs remain flat-color vectors: continuous gradients, extremely fine
texture and lossless photographic identity are outside this representation.
The saved previews and per-image metrics expose that tradeoff instead of using
low whole-image averages as proof of identical output.
