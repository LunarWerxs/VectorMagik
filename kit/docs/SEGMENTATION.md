# Preprocessing 0x00473850 and segmentation 0x00488D20 recovered

`kit/rust/src/recovered_segmentation.rs` (with `super_pixels.rs`,
`sub_pixels.rs` and the shared `recovered_colour_model.rs`) holds the
owned port of the preprocessing stage and the whole segmentation stage for
every preset. The oracles are `--segmentation-fixtures` (126 cases: the
fourteen synthetic blended images of the anti-aliased smoothing fixtures
under all nine presets, `native-segmentation.csv`), `--segmentation-aa-stages`
(the 42 anti-aliased cases after each of the nine stages of their path, with
the edge images, the boundary mask and every region's 42 features,
`native-segmentation-aa-stages.csv`) and `--segmentation-aa-sweeps` (the
same cases after every single sweep, rebuild and pass of the sub-pixel
segmenter, with the cached costs, `native-segmentation-aa-sweeps.csv`). All
three reproduce bit for bit; the host captured the fixtures through
`kit/native/segmentation_fixtures.h` and `segmentation_aa_fixtures.h` before
its removal on September 22, 2026, and the fixture CSVs remain. Earlier
transitional verification redirected 0x488d20 through
`kit/native/segmentation_bridge.h` before the port was completed.

## Preprocessing (0x473850)

The `Preprocessor` at engine+0x300 wraps a Magick++ image (the engine links
ImageMagick; 0x4ae*/0x4af* are Magick++, 0x4d6430 the library filter).

- The import 0x473c20 builds the library image from the BGRA bytes and
  premultiplies every colour channel by the pixel's alpha (`c * a / 255`
  truncated): a transparent pixel becomes (0, 0, 0, 0). Opaque images pass
  through unchanged.
- 0x472a00 applies the filter `Preprocessor::filter_type` selects:
  1 (every preset) calls the library's `EnhanceImage` (Magick++ 0x4ae620,
  the filter 0x4d6430) `Preprocessor::reduce_noise_order` times (presets 0,
  1, 2, 5, 9: 0; 4, 8: 1; 3, 7: 2; 6: 3). Types 2 and 3 are selected by no
  preset and are not ported.
- `EnhanceImage`: for every pixel, over the 5x5 window (edge pixels
  replicated outside the image), every neighbour whose distance to the
  centre pixel is below 255^2/25 = 2601 joins a weighted mean with the
  kernel 5 8 10 8 5 / 8 20 40 20 8 / 10 40 80 40 10 / 8 20 40 20 8 /
  5 8 10 8 5. The distance is the library's red-mean form over the
  library's (red, green, blue, opacity) pixel with opacity = 255 - alpha.
  The four channel sums are single precision, the total weight double, and
  the result `trunc((f32(0.5 * total) + sum - 1) * (1 / total))`.
- 0x472cb0 fills the global neighbour tables (`dx` 1 0 -1 0 -1 1 -1 1 at
  0xa68fe8, `dy` 0 1 0 -1 1 -1 -1 1 at 0xa69054, the 24-neighbourhood at
  0xa69078 / 0xa68f78, the row offsets); the port keeps them as constants.
- 0x473450 writes the library image back into the engine image at
  engine+0x8.

## Segmentation (0x488d20)

`Segmenter` at engine+0x930 (parameters 0x930..0x954), the super-pixel
segmenter at engine+0x934 (its label image engine+0x98, its segmentation
object engine+0x104 with the colour table engine+0x70), the sub-pixel
segmenter at engine+0x988 (parameters 0x98c..0x9a4, its segmentation object
engine+0x130 with the derived region records at +0x44, 0xb8 bytes each, and
the colour model engine+0xa58) and the shared block engine+0x208. 0x488d20
takes one of three paths on the preset (`super_pixels.rs`, `sub_pixels.rs`):

- presets 0, 1, 2 (`use_over_seg_diag_extr`): 0x4ab730(super, 1),
  0x4ab820, 0x48ae30;
- presets 6, 7, 8, 9: 0x4ab730(super, 0), 0x489de0;
- presets 3, 4, 5 (anti-aliased): 0x4ab730(super, 0), then the sub-pixel
  stages 0x4a9ac0, 0x4a83a0, 0x48b160, 0x4a9890, 0x4a9d10, 0x4a83a0,
  0x48b160, 0x4a7eb0;

then for all 0x4ab710, 0x488c70 (the contour records) and 0x4891f0.

The port's `segment` takes images of at least 2 pixels a side, the bound
`recovered_pipeline::vectorize` already set (September 22, 2026: a 1-pixel
side made the sub-pixel edge image read pixel -1 and panic; the 2x2, 2x9
and 9x2 solid, transparent and checkerboard images run under all ten
presets in `smallest_images_segment_under_every_preset`). The second
decision tree reads the shared block's field at engine+0x298, which no
preset sets (0 in every oracle case); `Shared::image_type_code`, which each
preset registers as its own code, is the different field engine+0x21c and
no stage of the port reads it.

### The super-pixel segmenter

- 0x4aab30 seeds the label image with the identity, copies `lambda_initial`
  into the running lambda and derives the rise `(lambda_final /
  lambda_initial) ^ (1 / (max_iterations * rise_fraction))`.
- 0x4aaac0 labels every 16x16 block through a quadtree (0x481c60): a block
  whose total variance (0x481130, the four channels as `byte / 255`
  doubles) is at most `lambda_pre` is one super-pixel, otherwise its four
  quadrants are tried in turn down to single pixels.
- 0x48ae30 renumbers the labels (0x48ac00: the union-find is flattened,
  every 4-connected component of one label gets the next number in scan
  order through the flood fill 0x484100, colour indices survive) and
  recomputes every region's pixel count and single-precision colour sums.
- 0x4ab3c0 walks every pixel and its right and lower neighbours (mode 0;
  mode 1 all eight, mode 2 merges any region under `min_num_pixels`);
  0x4aafd0 merges two regions when the cost 0x4aac30 (`|Sp|^2 / np +
  |Sq|^2 / nq - |Sp + Sq|^2 / (np + nq)`, single precision) is below the
  running lambda, at the final lambda also when either is under
  `min_num_pixels`, and otherwise moves the boundary pixel that sits closer
  to the other region's mean colour (0x4aaea0).
- 0x4ab730 runs `max_iterations` such passes, renumbering every
  `num_iter_btwn_renum`, raising lambda by the rise and capping it at
  `lambda_final`. Presets 6 to 9 then register every region's mean colour
  rounded to bytes in the colour table (0x489de0, 0x484c90 giving distinct
  colours indices in order of first use, 0x482e90 the float table).
- Presets 0 to 2 run the loop with `min_num_pixels` zeroed, then one
  8-neighbour pass and the diagonal extraction 0x489a50 (where a 2x2 block
  joins two regions only diagonally, one of the other two pixels takes the
  diagonal's label: the one that is not itself a corner, or the one closer
  in colour; both diagonals matching, the direction with the smaller second
  differences around the block wins), then 0x4ab820 truncates every
  region's mean colour to bytes, makes the distinct byte colours the colour
  table and replaces the labels by colour indices (a region under ten
  pixels would adopt a large region's colour when `count * |mean
  difference|^2` fell below the starting best score, the constant 10 that
  0x4aaa20 stores at engine+0x958, negated by 0x4ab820 to -10, which no
  score does, so the port leaves that search out), and 0x48ae30 re-derives
  the regions as components.
- 0x4ab710 merges regions under `min_num_pixels` once more; 0x488c70 fills
  the contour records (pixel count, colour index, colour bytes through
  0x469480: `trunc(f * 255 + 0.5)`).

### The sub-pixel segmenter (presets 3, 4, 5)

- 0x48b690 rebuilds the segmentation from the label image: renumbering
  (0x48ac00), per region the pixel count and colour sums, the set of its
  boundary pixels and their outside neighbours (8 or 24), and for its
  interior pixels the count, sums and sums of squares; the region colour is
  the per-channel median of at least three interior pixels, otherwise the
  mean of all its pixels; 0x48a370 caches every boundary pixel's
  colour-model cost (0x48a240: the squared distance of the pixel to the
  model colour of `recovered_colour_model` over the pixel's own region and
  its neighbours' regions, plus `self_weight_penalty_low * (1 - w)` and
  `self_weight_penalty_high * (self_weight_eps - w)` when positive, `w`
  the weight of the pixel's own region).
- 0x4a8640 sweeps the regions: each region of at most the given size joins
  the neighbour (of at most the other given size) with the lowest merge
  cost under `SubPixelSegmenter::lambda_f` (0x4a6bb0: the interior cost
  0x489f40 of describing the region's interior with the neighbour's colour,
  minus its own interior and cached boundary cost, plus its boundary pixels
  re-evaluated as the neighbour's unless the first part is already above
  lambda); 0x48bc60 merges (the union, the folded statistics, the boundary
  set carried over, pixels that became interior moved into the interior
  statistics). The sweep files every costed (region, neighbour) pair in the
  set at engine+0x9e4 under the 32-bit key `(region << 16) | neighbour`
  (`shl esi,0x10` at 0x4a86db, `or esi,ebx` at 0x4a86e3), and skips a pair
  whose key is already there. Past label 65,535 the key aliases: region
  65,536 shares region 0's keys and a neighbour's high bits land in the
  region's half, so a pair can be skipped that was never costed. The port
  keeps the packing (`visited_key`, pinned by
  `sweep_pair_keys_alias_past_16_bits_as_the_original`); only an image
  whose sweep sees more than 65,536 labels can feel it.
  The port's sweep is faster than the original's but computes the same
  bits (September 23, 2026): the original re-costs a region's whole
  boundary set for every neighbour, and a region that grows by merging
  into the next label is re-costed again on every candidate, which is
  quadratic on a pixel checker or a dither (a 96 px one took 108 s). The
  port keeps each pixel's neighbourhood list up to date through the merges
  (`relabel_hoods`), caches every term under a key versioned by the
  regions it reads (`sweep_term`), shares one term vector per colour class
  and reuses its running sums up to the first pixel a candidate changes
  (`step_class`, `step_term`), memoises the three-colour solves, stops a
  boundary sum once it passes the best cost so far (every term is
  non-negative, as 0x48b090 `set_cost_limited` stops), and replays the
  original's union-find root lookups (`touch`), since the native step test
  compares raw labels and parents. The sums are added in the original's
  order, so the result is the same to the bit (the sweep oracle, the 126
  synthetic conversions, the 34 references; 3.1 s for that dither). The
  cache relies on every sweep phase starting after `sub_init` or
  `sub_rebuild`, which build the neighbourhood lists.
- 0x4a9ac0 runs the sweeps with growing size limits in three phases: both
  limits 1, then 2 up to 21; any neighbour with the region's own limit 1,
  then 2 up to 21; then any region. Each phase runs one sweep and, while
  sweeps still merge, up to twenty more (at most twenty-one). After them
  come the beach pass
  0x4a7f20 (with the colour model's count field left at -1 by its
  constructor, the argmax picks index 0 and every pixel is re-assigned to
  its own region; only the cached costs are refreshed), the diagonal
  extraction 0x489a50, a last sweep over regions of at most four pixels,
  rebuilding between the phases.
- 0x4a83a0 gives every region with fewer than three interior pixels the
  colour of the large region (at least three interior pixels, the hundred
  largest) that explains its pixels best (interior cost plus the model
  cost of its boundary pixels evaluated as that region, 0x48b090 stopping
  early once above the margin) when that beats its own cost less
  `color_cluster_margin`.
- 0x48b160 registers the region colours in the colour table, replaces the
  labels by colour indices and re-derives the super-pixel segmentation's
  regions (0x48ae30 on the base object).
- 0x4a9890 computes 42 features per region (0x4a8800) and marks with flag
  1 the regions the decision tree 0x4ae090 calls anti-aliasing. The
  features: the sums over the region's pixels of the two edge images
  0x483150 (the summed squared colour differences of the prepared image to
  its four neighbours, and the squared difference of a Gaussian blur of
  the prepared image (Magick++ `gaussianBlur(1.5, 0.75)`: a 5x5 kernel,
  ImageMagick's GaussianBlurImage 0x4bab40 over the colour channels with
  the edge pixels replicated) to the mean of its four neighbours), their
  square roots and per-pixel averages, the pixel and interior counts, the
  count of pixels off the label-boundary mask 0x481f60, the neighbouring
  regions' count and size classes, the count of brighter and darker
  neighbours (two luminance formulas with different summation orders), the
  region's luminance, colour variance, interior-to-boundary ratio and
  bounding-box aspect, the closest large region's colour distance and the
  reconstruction error of a one-component PCA (ten power iterations
  0x4a72a0 in single precision, the previous region's eigenvalue estimate
  seeding the early stop). The closest large region (feature 39: the
  smallest squared colour distance, single-precision differences squared
  and summed in double, to any region of more than ten interior pixels,
  1e100 when there is none) is a scan over every region for every region
  in the original; the port finds the same minimum to the bit with a k-d
  tree over the large regions' colours (`NearestColour`; September 22,
  2026). The minimum of non-negative distances does not depend on the
  visiting order, a subtree is skipped only when its splitting plane alone
  is at least as far as the best so far, and non-finite colours, which
  never win the scan, are left out
  (`nearest_large_colour_matches_the_full_scan_to_the_bit`).
- 0x4a9d10 refines the flagged regions: each is eroded pixel by pixel into
  the neighbouring unflagged region whose adjacent pixel is closest in
  colour (the pixel with the smallest distance first); the model cost of
  the region's own and outside pixels before and after become features 33
  to 38, and the second tree 0x4ae1f0 decides per region whether the
  erosion stands or its pixels return; then the segmentation is rebuilt
  and every region's flags recomputed from both trees.
- 0x4a7eb0 releases the sub-pixel state; the super-pixel tail follows.

Floating point follows the original's precision at every step: the region
sums are single, the merge costs single, the interior costs and the colour
model double, the features double from single-precision inputs.

## Lessons

- The first divergence in the sub-pixel sweeps was the merge direction:
  0x4a6bb0 costs the sweeping region described by its neighbour, and
  0x48bc60 makes the sweeping region join the chosen neighbour. Reading the
  argument slots after the prologue (which register holds which argument)
  settled it in one pass once the per-sweep oracle pointed at the step.
- Two luminance formulas in 0x4a8800 use the same weights in different
  summation orders; the x87 listing, not the pseudo-code, is the authority
  for that.
- 0x483150 filters the library image again before blurring it; the oracle
  shows the extra filter has no effect on the blurred copy (the prepared
  image is what gets blurred), while calling 0x483150 twice in one engine
  does perturb later stages, so the fixture computes it on a second engine.
