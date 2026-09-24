# Changelog

## 0.4.0 (September 24, 2026)

- Photoshop PSD and PSB open and are traced like any picture, as TIFF and TGA do. SVG, PDF,
  Illustrator AI and EPS files are read as vector artwork: trace them from pixels or convert
  them as they are, with what was left out (text, images) named.
- The original Vector Magic's export options: SVG, PDF, EPS, AI, DXF, EMF and PNG; shapes
  stacked, cut out, or cut out and grouped by color; stroke shape boundaries; DXF as spline
  curves or as lines.
- Two previews of a new design, chosen from the Appearance button: Glass (floating
  translucent panes over soft color, after Apple's Liquid Glass) and Studio (a gray
  sidebar and toolbar in system blue). Classic stays the default.
- Light or dark in every look, or Windows' own setting; the browser version follows the
  site's light and dark switch.
- Portable: the C runtime is linked in, and the window keeps its settings beside itself
  when its folder takes files. Both programs are signed by LUNARWERX LLC.
- Commercial licenses, US$9 a month per seat: the app asks once whether it is for
  personal or commercial use, and a license key is checked once online, then offline.

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
