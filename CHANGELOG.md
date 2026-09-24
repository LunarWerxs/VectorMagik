# Changelog

## 0.3.0 (September 24, 2026)

- The browser version is the Windows app itself, compiled to WebAssembly and drawn on a
  canvas, with the same tracing, node editing, shapes, stickers, undo and saving.
- Simplify, set by hand, smooths the small kinks it keeps (a bend under 20 degrees, within
  the tolerance); Auto's tolerance draws exactly as before.
- Delete a node from its right-click menu, plainly (its two curves joined, keeping their
  outer handles) or keeping the shape (one curve refitted to the outline they drew).
- Node markers are hollow, so the edge a node sits on shows through them.
- The zoomed vector is drawn in the display's own pixels, sharp on screens scaled to 150%
  or 200%; in the browser, a zoom or pan renders it once the view rests.

## 0.2.0 (September 23, 2026): first public release

- The tracing engine, reconstructed in Rust with zero original machine code, reproducing
  the original's output byte for byte under `--defaults original`.
- Improved defaults, each measured: better tracing settings for artwork and photographs,
  true lines and circles, shapes drawn as circles, ellipses and rounded rectangles,
  straightening, Auto simplification, fills taken from the pixels, pixel art traced on its
  pixel edges.
- The desktop editor: Auto settings, overlay and side-by-side views with Hold to compare,
  curve nodes you can round, move and put back, shape selection (delete, merge), color
  limits and backgrounds, die-cut stickers, undo and redo, convert automatically, and
  saving as SVG, PDF or EPS at any size or by dragging the file out.
- A command line that runs the desktop's chain without a window.
