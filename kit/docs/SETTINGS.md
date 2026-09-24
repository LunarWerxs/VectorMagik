# Recovered settings contract

Primary evidence: `analysis/disassembly/0046b9a0-apply_engine_settings.asm`, `0046cd90-reset_settings.asm`, `00464520-settings_qt_dispatch.asm`, `parameter-bindings.json`, and `numeric-constants.json`.

## Two settings representations

The Qt `VectorizationSettings` object contains a plain settings subobject at **+0x08**, established by its constructors at `0x00444510` and `0x004445D0`. Engine routines receive the plain settings representation. Confusing the two shifts every field by eight bytes.

| Field | Qt object offset | Plain settings offset | Storage | Reset value |
|---|---:|---:|---|---:|
| Image category | 0x08 | 0x00 | i32 enum | -1 |
| Image quality | 0x0C | 0x04 | i32 enum | 1 |
| Number of colors | 0x10 | 0x08 | i32 | 2 |
| Color sensitivity | 0x14 | 0x0C | i32 enum | 1 |
| Palette anti-aliasing | 0x18 | 0x10 | byte boolean, padded | 1 |
| Segmentation complexity | 0x1C | 0x14 | i32 | 5 |
| Minimum pixels | 0x20 | 0x18 | i32 | 0 |
| Anti-alias rejection | 0x24 | 0x1C | i32 enum | 0 |
| Cluster colors | 0x28 | 0x20 | byte boolean, padded | 1 |
| Contour smoothness | 0x2C | 0x24 | i32 | 6 |
| Detect sharp corners | 0x30 | 0x28 | i32 used as boolean | 1 |
| Curve complexity | 0x34 | 0x2C | i32 | 6 |
| Contour anti-aliasing | 0x38 | 0x30 | byte boolean, padded | 1 |
| Use advanced mode | 0x3C | 0x34 | byte boolean | 0 |

The advanced block occupies **44 bytes**: eleven four-byte slots, with several one-byte booleans. `0x00440190` copies eleven DWORDs. `AdvancedSettings` keeps its fields in the block's order, and its doc comment records the layout (field k in the slot at offset 4k; palette anti-aliasing, colour clustering and contour anti-aliasing as one-byte booleans at the start of their slots, corner detection as a whole DWORD). Nothing in the port reads or writes the block itself: its decoder and encoder (`from_legacy_block`, `to_legacy_block`) had no caller and were deleted on September 22, 2026.

Reset values are observed at `0x0046CD90`. Later image classification, GUI operations, and saved state may override them. They are not a claim about every visible startup setting.

Other recovered plain-settings fields include palette mode at +0x64, batch color count at +0x68, flatten mode at +0x6C, four float color components at +0x70..+0x7C, and recommended flatten mode at +0x80. Palette containers occupy earlier offsets; their full ownership/layout is not reconstructed.

## Enum labels and baseline presets

The category switch at `0x00440200` and its embedded strings identify category **0 = Photograph**, **1 = Artwork with blending**, **2 = Artwork without blending**. Quality is **0 = High**, **1 = Medium**, **2 = Low**. Both also expose -1 for no selection and 3 for auto-detect. Rust models the three resolved categories and qualities. The original classifier remains unrecovered; the desktop now supplies a new local Auto estimate from color/edge statistics and source resolution, with manual overrides. It selects these resolved presets without claiming to emulate the native auto-detect enum.

The table at `0x00A21D08`, read by `0x0046BD78`, selects these preset image-type codes:

| Category | High | Medium | Low |
|---|---:|---:|---:|
| Photograph | 8 | 7 | 6 |
| Artwork with blending | 5 | 4 | 3 |
| Artwork without blending | 2 | 1 | 0 |

Palette finding first loads preset **9** through pointer `0x00A21C3C`. Advanced segmentation/smoothing/fitting load preset **4** through `0x00A21C28`. Advanced controls then replace selected fields. The named `image_type_code` is a preset identity, not the UI category enum.

## Recovered advanced parameter formulas

Since September 22, 2026 the engine runs these formulas: `advanced_preset`
in `kit/rust/src/lib.rs` merges the segmentation, smoothing and fitting stages
of `advanced_parameters` into the one parameter map the port's stages read
(the palette stage's keys have no consumer in the port; the port reads one
`Shared::is_anti_aliased`, so anti-alias rejection and contour anti-aliasing
must agree), and the CLI's `--advanced` (RUNNING.md) selects it. The CLI's
`--advanced` and the preview snapshot's share one grammar (`engine::parse_advanced`
and `engine::parse_sliders`: three sliders, then `key=value` settings, every part
trimmed); the preview takes only `corners=`, because the desktop card derives
anti-aliasing and the minimum region size from the image type, and refuses
`aa=` and `minpix=` rather than dropping them. There is no
native oracle for advanced mode (the host was removed before it was wired),
so its outputs are the recovered formulas' outputs, not byte-verified ones.
Two defaults rest on them (September 22, 2026, measured, RUNNING.md and
docs/TODO.md): high-quality photographs trace at the detail ceiling
(segmentation 12, smoothness 6, curves 6, anti-aliasing off as in preset 8)
and high-quality blended artwork at segmentation detail 11 and smoothness
3 (11,3,6, anti-aliasing on; 5,4,6 until September 23, 2026), unless `--defaults original` asks for presets 8 and 5;
`kit/fixtures/reference/*-high-detail.svg` freeze what was accepted.

These formulas come from the x87 instructions in `0x0046B9A0`, not from guessing their intended behavior. Rust restricts the three continuous controls to the analyzed domain 1..12. Whether all original entry paths impose the same bounds remains unverified.

### Palette stage

Palette anti-aliasing sets `Shared::is_anti_aliased`. Sensitivity codes select `(use_16_hist_bins, use_26_connected_peaks)` as follows:

| Sensitivity | Histogram switch | Connected-peaks switch |
|---:|---:|---:|
| 0 | 1 | 0 |
| 1 | 1 | 1 |
| 2 | 0 | 1 |

Number of colors is carried in settings and consumed outside this mapping routine. The Rust parameter function does not yet implement palette selection or apply a color-count constraint to an image.

### Segmentation stage

For complexity `c`, let `r = 12 - c`:

```text
lambda_pre     = f32((r + 1) × 0.0001)
lambda_initial = lambda_pre
lambda_final   = f32(0.05 × 400^(r / 11))
min_num_pixels = user minimum pixels
```

The initial/pre values are stored as f32 at engine +0x938/+0x93C; final is f32 at +0x940. Higher complexity reduces these penalties.

Rejection codes 0, 1, 2 set engine fields `(0x214, 0x20C)` to `(1,1)`, `(0,1)`, `(0,0)`. The second field is named `Shared::is_anti_aliased` by the registration code. The first field's complete semantics are not established. Color clustering sets the integer at engine +0x218 from a byte boolean. No stage of the port reads +0x214 or +0x218 (`advanced_preset`'s doc comment); the helper that reproduced this triple (`segmentation_flags`) restated its own formula and was deleted on September 22, 2026.

### Smoothing and fitting stages

For smoothness `s`, let `t = (s - 1) / 11`. Each of the three phases interpolates geometrically:

```text
value = minimum × (maximum / minimum)^t
```

| Parameter | Anti-aliasing | Minimum values for phases 0/1/2 | Maximum values |
|---|---|---|---|
| Prior strengths | on | 0.05, 0.01, 0.45 | 10, 10, 10 |
| Prior strengths | off | 0.05, 0.01, 0.45 | 20, 20, 20 |
| Length penalties | on | 0.25, 0.05, 0.005 | 10, 1, 1 |
| Length penalties | off | 0.5, 0.5, 0.5 | 40, 40, 40 |

The mappings set measurement types to the anti-alias flag, prior types to `[3,3,3]`, corner puncturing to `[0, detect_corners, 0]`, and enabled phases to `[1, detect_corners, 1]`. Thus turning off sharp-corner detection disables the middle smoothing phase in this advanced mapping.

For curve complexity `c`, let `r = 12 - c`:

```text
BezierFitter::stat_thresh_initial = (r + 1) × 0.0001
BezierFitter::stat_thresh_final   = 0.1 × 200^(r / 11)
```

These are stored as f64 at engine +0x1F08 and +0x1F10. Higher curve complexity tightens the thresholds. Ordinary scheduling is now recovered: it compares changes in summed fit residual error, accepts merges strictly below the active threshold and applies ordered backward/forward shifts. See [RUST_PIPELINE.md](RUST_PIPELINE.md). The optional pass at +0x1F24 is never registered and is constructed off; its objective is recovered in [FITTING.md](FITTING.md).

## Verification

`tools/verify_settings_arithmetic.py` (driving the `Machine` of `tools/settings_machine.py`) interprets only the relevant integer and x87 instruction blocks, using constants read from the binary. It executes no native target code and makes no external calls. Its 144 cases cover all twelve levels of each continuous control, both contour anti-alias settings, both corner settings, and all three rejection codes. This is coverage across levels and discrete settings, not the full Cartesian product of three independent sliders.

Rust tests compare mapped numeric fields against those fixtures. Python and Rust use f64 intermediate arithmetic, whereas the original x87 may retain 80-bit intermediates. Tolerance-based agreement is established; bit-exact historical floating-point behavior is not.

The crate has no cache invalidation helper (the proposed `earliest_invalidated` had no caller and was deleted on September 22, 2026). For a future cache: the original builds nested string signatures at `0x0046A510` and `0x0046A650`; the fitting signature incorporates smoothing state and curve complexity. Image contents, palette edits, segmentation edits, basic-mode controls, and background state must also enter a complete cache policy.
