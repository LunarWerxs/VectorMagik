# Export: preparation, size and the SVG writer

Ported September 22, 2026 into `rust/src/recovered_export.rs`, the last stage
of the engine. The oracle was `--export-fixtures` on the headless host (removed
the same day): 126 cases, the fourteen synthetic blended images under all nine
presets, run through the original import, preprocessing, segmentation, contour
construction, smoothing and fitting, with layering k % 3 and stroking
(k / 3) % 2, one process per case (the generator at 0xa21d2c outlives a case
inside one process). Each case records what the export reads and the bytes the
original wrote: `rust/fixtures/native-export.csv`. The port reproduces every
one, and through `recovered_pipeline::vectorize` the same 126 conversions and
the 28 frozen sample references in `fixtures/reference` come out byte for byte.

## What the export reads

The export object at engine+0x1ff8 points at the fitter's arrays: the contour
records (fitter+0x20, 0x60 bytes each: colour bytes at +0x14 in blue, green,
red, alpha order, the enclosing contour at +0x30, node ids at +0x18/+0x1c, the
fitting's pieces at +0x40/+0x44, 0x1c bytes each: index, kind, edge, start
node, end node, start position, end position), the node positions
(fitter+0x24, x and y doubles at the head of each 0x28-byte record) and the
fitted cubics (fitter+0x28, four points each). export+0x0 holds 72, the
resolution the coordinates are in; export+0x2c is the layering (0 paths only,
1 hole loops, 2 hole loops in colour groups; the application uses 2),
export+0x34 the stroking (0 strokes every path with its own fill at width
0.09375; the application uses 1); the bitset at export+0x3c/+0x44 hides
shapes (0x46c330, never set by the application).

## 0x47d4c0: hole loops

For every shape, its children (shapes whose +0x30 names it, in index order)
are walked: runs of consecutive pieces whose `edge` word equals the shape's
index are collected in the child's own piece order, a run never wrapping
around the end of the piece list. Runs are chained: run k continues into the
last run l whose first piece's start node equals run k's last piece's end node
(the original stores every match, so the last wins). Every run with a
successor starts a loop: the loop appends runs following the chain until the
next run has no successor left, clearing each visited link. The loops are
stored per shape; export+0x8 remembers the shape count they were built for.

## 0x4745d0: size

The largest x and y over all nodes, each rounded half up to an integer, are
the viewBox extent; the width and height in points are those integers scaled
from export+0x0 to the requested resolution in single precision
(`(dpi as f32 * max as f32) * (1.0f32 / base as f32)`, truncated).

## 0x47c2f0: colour groups

With layering 2 the shapes are grouped by their four colour bytes in order of
first appearance, each tagged 0 (first of a group), 3 (inside), 1 (last) or 2
(alone); 0 and 2 open a `<g>`, 1 and 2 close it.

## 0x47c660 and 0x4741b0: the walk

In group order (or index order without groups) every shape whose alpha
exceeds 10 (0x475b40) is emitted: the group opens if tagged so, the path
begins with the shape's colour, the shape's own pieces follow in order (the
first with `first = 1`), then with layering 1 or 2 each hole loop is walked
backwards from its last entry with every piece reversed (kinds 1 and 2
swapped, line endpoints swapped; the first emitted piece gets `first = 2`),
the path ends, and the group closes if tagged so. A line piece joins node
`ids[index]` to `ids[(index + 1) % len]`; a cubic piece is `curves[index]`,
reversed for kind 2.

## The writer (vtable 0x8dcfa0)

`_wfopen(path, L"w")` opens the file in text mode, so every `endl` is CR LF.
The stream has `std::fixed` and precision 2, which MSVCR71 formats as "%.2f":
the exact binary value's digits rounded half away from zero (`fixed2`); Rust's
own `{:.2}` rounds half to even and prints "0.12" where the original prints
"0.13" for 0.125. The text, in order: the XML declaration, the DOCTYPE, the
`<svg width="Wpt" height="Hpt" viewBox="0 0 X Y" version="1.1" xmlns=...>`
line without a trailing line end; then per group `\r\n<g id="#rrggbbaa">`
(bytes red, green, blue, alpha); per path `\r\n<path fill="#rrggbb"` followed
by ` stroke="#rrggbb" stroke-width="0.09375"` when stroking is 0, then
`" opacity="1.00"` above alpha 0xfa or `"%0.2f"` of alpha times
0.00392156862745098, then ` d="`; pieces write ` M x y` for `first` 1 or 2,
then ` L x y` or ` C x1 y1 x2 y2 x3 y3`; the path ends with ` Z" />`; a group
closes with `\r\n</g>`; the document ends with `\r\n</svg>\r\n`.

## Owned: straight fills (off in the original)

The region colours the writer receives are means of premultiplied pixels
(preprocessing stores `c * alpha / 255`), and the original writes those bytes
as the fill together with the opacity, so a translucent region is scaled by
its alpha twice: (255, 0, 0) at alpha 128 becomes `#800000` at 0.50 and
renders (191, 128, 128) over white and (64, 0, 0) over black instead of the
source's (255, 128, 128) and (128, 0, 0). `ExportSettings::straight_fills`
(owned, September 22, 2026) writes `min(255, round(c * 255 / alpha))` in the
fill, the stroke and the group id of every shape whose opacity is written
below 1.00 (alpha 11 to 250); the opacity, the order and every coordinate
stay as they are, and alpha above 250 keeps its bytes (the writer declares
those opaque, and either colour is off by at most 255 - alpha levels summed
over white and black). `ExportSettings::new` leaves it off, which the 126
native cases pin; the app turns it on under the improved defaults
(`recovered_pipeline::Options::straight_fills`). None of the seven samples
has a region with alpha 11 to 250 (the frozen references hold only alpha
0xff and 0xfe), so it changes no sample's output; the app test
`translucent_regions_keep_their_colour_under_the_improved_defaults` renders a
synthetic image with (255, 0, 0) at 128 and (0, 96, 255) at 200 within 3
levels of the source over white and black, where the original's fill is 44
to 64 levels dark. Over every translucent pixel of that image (render_svg at
the source size, both categories): the original's fill is off by 20.8 levels
on average and 63.7 at most over white (21.0 and 63.7 over black), the
straight fill by 0.27 and 1.0 (0.23 and 1.0).

## Lessons

- The fixture's first attempt ran all 126 cases in one process; the
  anti-aliased cases then disagreed because the original's smoothing draws
  from a generator whose state the previous case had advanced. One process
  per case, and seed 1 in the port, resolved it.
- A "-0.00" in the port's text where the original printed "0.00" exposed
  last-bit differences in the fitting's cubic solve. Only the original's own
  arithmetic (dgemm sums, dgesv, 0x479e10's evaluation order) removed them;
  the 96 interval fixtures had passed at 1e-7 before and compare to the bit
  now.
- The builder's "extra array" at builder+0x1c, which decides whether the
  straight-run thinning runs, is the shared block at engine+0x208; its second
  word is `Shared::is_anti_aliased`. The smoother's diagonal unit constant is
  one unit below Rust's `FRAC_1_SQRT_2`; the placement decisions feel it.
