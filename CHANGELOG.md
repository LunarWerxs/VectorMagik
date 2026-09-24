# Changelog

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
