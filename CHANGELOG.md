# Changelog

## Unreleased

## 0.5.1 (September 26, 2026)

- Before a picture: one compact welcome card (drop, click, Browse for an image or Try a
  sample) instead of two full-height cards; the toolbar says Pick an image and hides what
  has nothing to act on.
- Controls answer: the picture box lights up and dips when clicked; card headings and More
  options (now a disclosure row) highlight under the pointer; switches ease their colour;
  the empty Vector card offers Convert and shows a conversion's progress.
- Rows slide in and out instead of popping: Auto settings off, the photo and background
  options, the Shapes card's tools, DXF curves, the licence key and the phone's settings
  sheet.
- Convert is lit while there is something to convert, then Save; a Save or Download button
  in the Save panel; Undo and Redo buttons.
- Plainer words: Artwork, Pixel art, Photo; Amount; "Click to hide"; one-line tooltips; the
  version in the settings' footer and in Appearance.

## 0.5.0 (September 26, 2026)

- For AI assistants: `vector-magic-rebuild --mcp` is an MCP server, so Claude, Cursor and
  other assistants can trace pictures on your computer: vectorize (the app's Auto settings
  unless asked otherwise, with a picture of the result), inspect (what a file is and what
  Auto would choose) and view (any picture or vector file as an image).
- The engine alone: a download per system with just the command-line program, for an
  assistant or a script that does not need the window.

## 0.4.6 (September 26, 2026)

- Try a sample: the empty window opens and converts a sample logo in one click.
- Works on a phone: in a narrow window the settings open as a sheet from a gear in the
  toolbar, and Convert shrinks to its icon.
- Curve nodes stay hidden until you turn them on, and the app remembers. The license
  question's free choice is the prominent one. The Save panel shows format and size; the
  layering options sit under More options. The Advanced card says Off until used.
- Hint text is brighter in both themes; Keyboard shortcuts opens with a click.
- In the browser: Shift+Tab leaves the app, a browser without WebGL2 says so instead of
  showing a blank page, and the page's light or dark follows the app's choice and the
  device's.
- Rounding many corners on a large picture no longer holds hundreds of megabytes.

## 0.4.5 (September 25, 2026)

- One look: Glass, with the cards a little lighter than a flat near-black in the dark
  (no light or gradient behind them) and frosted white in the light. Appearance chooses
  light, dark or the system's, in the browser too, where the page follows the app's
  choice. On the website's app page the menu dropped the License badge and the theme
  switch, and Download sits at its far right.

## 0.4.4 (September 25, 2026)

- A calmer window. The toolbar shows the picture's name as its title beside Open, Convert,
  Save and Close (the typed path field and the window-snapshot button are gone; the
  snapshot keeps its shortcut). The sidebar opens with Conversion and Curves only, the
  other cards fold to one line, and the switches most pictures never need sit under More
  options; no switch carries an explanation line any more (hover for it). The footer
  dropped the segments chip, the zoom slider and 1:1 (click the zoom figure for 1:1).
- Studio is the default look and Classic is gone (a saved Classic opens as Studio).
  Glass is plain grey glass, without the purple and blue light behind it, and the cards'
  shadows are no longer cut off at the sidebar's edges and under the header.
- In the browser, the site's menu folds up out of the app's way: point at the window's
  top edge (or tap the handle there) to bring it down.

## 0.4.3 (September 25, 2026)

- Every rounded corner gets two nodes. On a large rounded square one corner could come
  out with three, because the tracer stopped the straight edge a little before the
  corner's curve began and joined them with a short extra piece; that piece now goes into
  the edge and the curve beside it, so the corner is one curve. Every other sample of the
  quality check draws exactly as before.
- Delete node keeps the shape: the two curves meeting at the node become one curve fitted
  to the outline they drew. The plain delete that kept the old handles, which bent a
  rounded corner, is gone, and with it the second menu entry its tooltip used to hide.

## 0.4.2 (September 25, 2026)

- VectorMagik runs on Mac and Linux. A universal Mac app for Apple Silicon and Intel
  (macOS 11 or newer, not notarized yet: right-click it and choose Open the first time)
  and a Linux build for X11 and Wayland (Ubuntu 22.04, Debian 12 or newer), each with the
  command line beside it. Their Open and Save dialogs are the system's (AppleScript's on
  a Mac, zenity's or kdialog's on Linux); dragging a result out of the window stays
  Windows-only for now.
- Icons that drew as empty boxes now draw: the Advanced settings card's icon everywhere,
  and the cards' fold arrows and the arrow in texts in the browser.
- Small print keeps its ink along the line: a line of pale text on a dark background
  no longer comes out with some letters white and others lilac, and letters the tracer
  had run together are drawn apart from their pixels, in the line's colour.
- Pixel-edged artwork is traced from its pixels: a screenshot's text, an aliased logo or
  an icon, drawn without anti-aliasing in exact colours, keeps its letters and lines
  instead of being smoothed like a photograph. Small pixel-edged text is much closer to
  the true letters (at a 10 px cap the & keeps its loop and "tt" no longer reads "(t"),
  1 px diagonals stay whole and straight, and a straight side is one line. Such a picture
  skips true lines, Auto straighten and Auto's stronger simplify, which bent what the
  tracer drew.
- Dithered pixel art saves small: an area whose pixels repeat a small pattern (a checker,
  an ordered dither) is written as one shape filled with that pattern in SVG, PDF, AI and
  EPS, instead of a square per pixel (a 96 px checker: 207 KB of SVG before). The picture
  is the same pixel for pixel, and the app opens its own saved files back as the squares.
- DXF and EMF save dithered areas small too. A DXF draws the repeating pattern once and
  places it as an AutoCAD array (the dither sample: 1 MB of DXF down to 45 KB, the same
  outlines once exploded); an EMF writes the area's squares as one polygon record, about a
  third of the size, the same pixels in Windows.
- Small lettering (a cap height around 10 px) is drawn from its pixels: letters come out
  as letters instead of wedges, an i keeps its dot, and the curves are as smooth as the
  engine's own, at about 17% more bytes for such pictures.
- Pale small print on dark backgrounds keeps its colour: lilac and mint text no longer come
  out white and grey, and yellow and pink are no longer dulled; slightly bolder small print
  (13 and 14 px) is drawn from its pixels too.
- Pixel art keeps its single pixels on the picture's edge. A dot touching the border (a
  dither's outer row, a sprite's last pixel) was dropped; it now comes back like any other.

## 0.4.1 (September 24, 2026)

- DXF files open in Adobe Illustrator and other AutoCAD-based readers: the "lines" modes no
  longer carry a color code their DXF version lacks, and the "spline curves" mode gives
  every layer the plot style AutoCAD expects. The "lines" modes pick colors from AutoCAD's
  own palette.
- Shapes grouped by color stay grouped when a PDF or AI file is opened in Illustrator.
- Not sure yet? The first-run question offers commercial use free for 24 hours, no key and
  no payment; afterwards the app asks once more.

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
