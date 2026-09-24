# Contour construction: 0x00499EB0 recovered

The contour builder at engine+0x17e8 turns the label image segmentation
leaves (one region id per pixel, Array2D pointer at builder+0xc) into the
shared node set and one closed contour per region that smoothing and fitting
consume. `kit/rust/src/recovered_topology.rs` is the owned port;
the host captured 48 label images through the unmodified original before it
was removed on September 22, 2026, which left the seven sample SVGs
byte-identical.

## Records

Node records (stride 0x28, the node set's own array at set+0x8, also reached
through builder+0x14): `+0`/`+8` position as doubles, `+0x10`/`+0x14` the
same as floats (0x480230 writes both), `+0x18` curve index (-1 from the
record constructor), `+0x1c` state byte (3 at a corner, else 0), `+0x1d` flag
byte (0x80 on the canvas border; during thinning 0x40 marks a node for
removal and 0x80 a node the run walk passed), `+0x1e` zero, `+0x1f` and the
array at `+0x20` untouched here. The node set (builder+0x18) keeps the
canvas size at `+0x34`/`+0x38` and, at `+0x20..+0x2c`, how many nodes sit on
the left, right, top and bottom edges (0x47e450 sets h+1, h+1, w-1, w-1;
thinning subtracts what it removed).

Contour records (stride 0x60, builder+0x10): `+0x18`/`+0x1c` node ids and
count, `+0x20`/`+0x24` twelve-byte entries (word 1 = the label across the
edge leaving the node, -1 outside the canvas; word 2 = the number of unit
steps of the edge arriving at the node; word 0 is uninitialised memory here
and the smoother's later corner flag), `+0x28`/`+0x2c` the sorted distinct
labels across the contour's edges without -1, `+0x30` the first of those
that does not list this contour back (-1 otherwise). Words 0..5 and
0x34..0x5c stay zero.

## Algorithm

1. **Node grid, 0x498a50.** A (w+1) by (h+1) grid counts, for every pixel
   with `x < w-1` and `y < h-1`, the two vertices of the edge below it when
   the pixel below differs and of the edge to its right when the pixel to the
   right differs; the vertex (w-1, h-1) is counted separately from its own
   pixel's two neighbours. Numbering: the left column of vertices top to
   bottom, the right column, the top row for `x` in 1..w-1, the bottom row,
   then interior vertices with a count in scan order; every other interior
   vertex is -1. Because a boundary never ends at an interior vertex, the
   loop bounds lose nothing: an interior vertex is a node exactly when a
   label change touches it.
2. **Node records, 0x480230.** Position, the float copies, and state 3 at
   the four canvas corners.
3. **Tracing, 0x4999d0**, from the first node among the corners (0,0),
   (1,0), (1,1), (0,1) of the first pixel in scan order of each region whose
   contour is still empty. Directions are left, down, right, up; step `k`
   lies between the pixels `PIXEL[k]` and `PIXEL[k+1]` of the four around the
   vertex (up-left, down-left, down-right, up-right), and is taken only when
   the far vertex is a node, the two pixels differ and the first is the
   region (the region stays on the right hand). A vertex with two such ways
   on (a checkerboard corner) chooses the one whose region-side pixel is the
   pixel the previous step ran along; the first vertex takes the first way.
   The node id is pushed on arrival, the label across the chosen edge after
   each step; the walk ends on reaching a vertex already marked with the
   region, and only vertices with at most one way on are marked. The labels
   across the edges are sorted, made unique and stripped of -1 for `+0x28`.
4. **Corners.** Of the six pairs among the four labels around a node (-1
   outside the canvas), fewer than two equal pairs makes the node a corner.
5. **Thinning, 0x4993e0.** Border nodes that are not corners get 0x40 (the
   first and last of the left and right columns, the canvas corners, are
   skipped); every border node gets 0x80. In a conversion the builder's extra
   array pointer is the shared block at engine+0x208, whose second word is
   Shared::is_anti_aliased, so anti-aliased presets skip the straight-run
   thinning and the others run it. Unless the builder's array at
   +0x1c is non-empty, each contour of at least 20 nodes is walked over
   `n + 31` consecutive node pairs: the unit step between the previous and
   current node is rounded from the positions; a removed previous node, a
   passed current node or a corner at either end resets the run; from the
   31st pair on the previous node is marked passed; a change of sign in x or
   y breaks the run; two bit strings record which steps moved in x and in y;
   once the run has 5 steps (capped at 31) the pattern table is searched for
   the first entry no longer than the run whose mask equals either string's
   low bits, and its keep bits then mark every node of the last `length + 1`
   positions whose bit is clear for removal, after which the run restarts.
   The pattern table is the constructor's: (5, 0, 33), (9, 170, 585),
   (9, 146, 585), (13, 4369, 8481), (10, 132, 1057), (10, 330, 1057), i.e.
   straight runs of five keep their two ends and the three regular
   stair-steps keep one node per period.
6. **Compaction.** Surviving nodes are renumbered in order; each contour
   keeps only surviving ids, copying the entry of each kept node with word 2
   set to the distance from the previous kept position, and the first kept
   entry's word 2 to the wrap-around distance.
7. **Enclosing, 0x49a213.** The first neighbour whose own neighbour list
   lacks this contour. The lists are sorted and distinct from step 3 on, so
   the port asks with a binary search where the original scans (September
   22, 2026: a background around K specks made K scans of K entries; the
   answer is the same). The builder also reads the caller's label image in
   place rather than copying it, and gathers the four labels around a
   vertex in a fixed array.

## Evidence

Spans verified byte for byte by `recover_fitting.py`: 0x47e450, 0x480230,
0x498930, 0x498a50, 0x499180, 0x4993e0, 0x4999d0 (normal return; the
out-of-line blocks 0x499e35..0x499eaa follow it) and 0x499eb0. The direction
tables and the pattern table were read from the constructed engine by the
capture and are checked against the Rust constants by a core test.

`native-topology.csv`: one `tables` line, then 48 label images (3 to 26
pixels a side: splits, nested frames, checkerboard blocks, diagonals, blocky
noise, one region, discs, stripes; the extra-array flag set on every fifth)
with every node and contour record the original leaves. The core test
reproduces all of them exactly. Before the host was removed, the Rust
result was written back through the original resize helpers (0x484e60, 0x47e450,
0x480230, 0x474b90, 0x480da0) and the seven sample SVGs remained byte-identical.
