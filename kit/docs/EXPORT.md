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

## Owned: dithered areas as pattern fills (September 24, 2026)

A picture with an exact palette comes back pixel for pixel (the exact
recovery), so a dithered area is one square per pixel: the 96 px checker of
work/dither-sample.png saved as 4,608 squares, 207 KB of SVG. `dither.rs`
finds, in each unstroked one-colour path, the rectangles over which its
squares repeat a tile of up to 8 by 8 cells (at least two tiles each way and
16 squares; the largest rectangle each round, over every tile size, grown to
where the repetition stops). The saved SVG draws each such rectangle once,
filled with a `pattern` of the tile (`patternUnits="userSpaceOnUse"`, anchored
at the rectangle's corner), right after what is left of its path; the PDF
(and so the AI) fills it with a tiling pattern (type 1, coloured, its matrix
onto the page), the EPS with `makepattern`. The document in the app keeps its
squares: the preview and every edit see them as before. Inside a translucent
group the PDF keeps the squares (a pattern there would be placed in the
group's space), and a path with boundary strokes is left whole. The app's
three vector readers draw such a pattern back as its squares ("Convert it" of
a saved file), so a save and a reopen draw the same pixels; other programs'
patterns are still drawn in one colour.

DXF and EMF (September 25, 2026). A DXF holds outlines, not fills, so
`dxf.rs` makes the tile a block of its inked squares and places it as
AutoCAD's rectangular array (an `INSERT` with column and row counts, which R12
has too); the columns and rows where the rectangle ends inside a tile are
arrays of the part of the tile they hold, and blocks drawing the same squares
are made once. A hatch was the first idea and is not used: its pattern is
lines clipped to a boundary, so the squares would come back as loose edges,
and the edges on the boundary as each reader decides. EMF has no pattern that
scales with the drawing (a GDI pattern brush tiles in the output device's
pixels and paints every cell of its tile, the cells between the squares too),
so `emf.rs` writes the area's squares as one `EMR_POLYPOLYGON16` after the rest
of the path, four corners a square where the path spent a move, a line record
and a close; a bitmap stretched over the area would be smaller still but turn
the squares into a picture. On the dither sample (desktop chain, stacked;
testing/pattern-fills-dxf-emf-2026-09-25): DXF 1,029,317 -> 44,637 bytes
(splines), 1,723,175 -> 65,263 (fine lines), 1,720,052 -> 62,140 (coarse
lines), EMF 326,676 -> 104,460. ezdxf reads every file strictly with no audit
error and, the arrays exploded, draws exactly the old files' outlines (4,795
and 4,789); Illustrator's AutoCAD import, the strict reader, opens all six and
draws every outline where the SVG has it (12 of 12 checks); GDI and GDI+ draw the new EMF and the old one alike to the pixel at
1x and 4x. The 34 frozen references saved as DXF in the three modes and as EMF
come out byte for byte as before (136 files).

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
