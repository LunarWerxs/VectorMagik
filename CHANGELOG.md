# Changelog

## 0.5.19 (October 1, 2026)

- Small, blurry pictures are cleaned up before they are traced. A soft JPEG logo or
  cartoon (enlarged, blurred, saved with its color at half resolution) is first restored
  by a small learned network, VectorMagik's own, that takes out the blur, the color
  bleeding at edges and the JPEG's 8 x 8 blocks, at the picture's own size; the tracing
  then follows the crisp picture. On a bird logo that came out with notches in its wing, a
  grey patch at its neck and a dark rim in its tail, all three are gone, and soft logos
  and cartoons trace closer to the clean pictures they were made from. It runs on artwork
  in color whose edges are soft; crisp pictures, lettering in one ink and photographs are
  left as they are. It is on by default (Conversion, More options, Clean up blurry
  pictures) and adds a fraction of a second in the desktop app, two to three seconds in a
  browser. The cleaned-up picture is a third picture to look at beside the original and
  the vector: Cleaned up in the picture's heading, or the C key (B and V show the other
  two).
- Enlarge: the cleanup can also make the picture two or four times larger, sharp, and
  trace that (Conversion, More options, Enlarge, under Clean up blurry pictures; off
  unless chosen). Tiny details such as small lettering come out crisper, but it takes
  about three (2x) or seven (4x) times as long, curves wave a little more and files grow;
  for most pictures the plain cleanup gives as good a drawing. Cleaned up shows the
  enlarged picture, and Save picture keeps it as a PNG for anyone who only wants the
  bigger, sharper picture.
- Reduce nodes: the Curves card has a slider that takes nodes away, with the count beside it.
  Up to its middle the look stays the same; further right, corners and circles round off
  too, and at the far end every shape is a blob. Where three colors meet stays put, so
  colors never part; a many-colored picture keeps its shape longest, lettering goes first.
- The edges of a JPEG that was sharpened for the web trace as smooth lines. Such sharpening
  leaves a thin dark rim along one side of every edge and a light halo on the other, and
  they came out as small bumps, spikes, slivers and steps along a logo's edges; the
  cleanup now takes them out first.
- Gradient logos come out as gradients. Bands of color that trace one smooth ramp (a
  logo's orange-to-pink feather, say) are drawn as one shape filled with a real gradient
  taken from the picture, where that matches the picture at least as well as the bands.
  They showed as patchy bands with wandering borders before. It is on by default (Curves,
  Gradient fills). SVG, PDF, Illustrator and EPS keep the gradients; DXF and EMF draw each
  in its middle color. Each gradient runs to the ends of its shape, so where two meet the
  seam is softer. And the bands are joined before the shape's outline is traced, so it is
  one clean outline: no more notches in the edge where two bands met it, and files a fifth
  to a half smaller.
- Where two colors meet an outline, they now meet it exactly: zoomed in, the background no
  longer shows through as a light hairline along the last pixel of the border between them,
  and no sliver of color pokes past the outline as a sharp little tick (also where a shape
  narrows to a thin point).
- A cartoon or line drawing with photographs in it is traced as artwork: Auto called it a
  photograph and its ink lines came out as strings of grey blots. Such a picture takes
  longer to convert (about 20 seconds in the desktop app for a 1334 x 616 frame, about a
  minute in a browser).
- Artwork converts faster in the desktop app, which now shares the work between all of the
  computer's cores: typically two to three times, up to four (that frame took about 80
  seconds before). The result is exactly the same, to the byte.
- In the browser, the tracing shares its heaviest work between the computer's cores too
  (Chrome, Edge and Firefox; Safari keeps to one): a quarter to a third faster on a logo,
  about twice as fast on a detailed cartoon. The result is exactly the same, to the byte.
- A long conversion says so as it starts: the picture reads, for example, "A detailed
  picture: this can take about a minute." In a browser it adds that the desktop app is
  faster (several times, where the browser keeps to one core).
- Black-and-white photographs, and a portrait or other photograph on a plain background,
  are traced as photographs: they were traced as artwork, much slower and posterized.
- The color list opens at once and scrolls normally with thousands of colors, and it takes
  a number: type how many colors to keep next to "Reduce to" and the picture converts
  again with that many.
- In the browser on a phone or tablet, pinch with two fingers to zoom the result, and move
  two fingers to pan. Pinching did nothing before.

## 0.5.18 (September 28, 2026)

- The drop box says what it takes: images, and PDF, Illustrator, EPS, SVG and Photoshop
  files, one or several at once ("Browse for files").
- A converted design file shows its name at the top of the window; the box there still
  read "Pick an image".
- The command-line program in each download is named vectormagik-cli now. An AI assistant
  set up to run it over MCP needs the new name in its setting (for example
  `claude mcp add vectormagik -- /path/to/vectormagik-cli --mcp`).
- The command line's statistics line (--stats) leaves out two fields that never changed,
  "backend" and "original_code".

## 0.5.17 (September 28, 2026)

- Faster, with every drawing exactly as before. In the browser, squaring the corners of an
  enlarged or blurry picture takes a third to under half the time it did (56% less on
  enlarged lettering, 70% less on an enlarged logo), a logo converts nearly twice as fast and
  a photograph 20 to 27% sooner. Editing is quicker too: moving, deleting or rounding a node
  no longer works out the straight lines and circles again, so a node settles 4 to 7 times
  sooner on a logo. In the app, a conversion takes 12 to 25% less time.
- Pick the page of a PDF or Illustrator file with several pages: the question about a
  vector file has a page picker. Only the first page could be opened before.
- Save an SVG at 0 to 3 decimals, or as the smallest file that draws the same, and copy
  the SVG code straight to the clipboard from the Save popup. A PNG saves on white or
  clear.
- Convert a whole batch: drop or open several files at once and they convert together with
  the current settings, pictures traced and vector files kept as they are. The app saves
  them into a folder you pick; the browser downloads one zip.
- Vector files keep how they look when converted as they are: a PDF, Illustrator or EPS
  file keeps its text (as outlines, from the fonts it carries), pictures, gradients,
  clipping and blend modes, and an SVG its text, gradients, clip paths, masks and
  pictures. They used to come through as flat shapes only, with text and pictures left
  out and gradients in one colour. Saved as PDF, AI or EPS they keep all of it too.
- A Photoshop file converted as it is keeps every visible layer: shapes stay vectors in
  their colours, gradients, patterns and strokes; photos, text and smart objects stay as
  their pixels, through their masks and clipping. Only its shape layers came through
  before.
- The command line: --page picks a PDF's page, --svg-decimals, --svg-smallest and
  --png-background save as the Save popup does.

## 0.5.16 (September 28, 2026)

- In the browser, a drawing no longer changes a moment after it appears: a sharp picture's
  corners were squared a fraction of a second after its drawing showed. It shows once now,
  finished; only an enlarged or blurry picture, whose corners take seconds, shows first and
  says it is squaring corners, with its nodes waiting until it is done.
- Convert shows that it is working: the vector dims under a turning spinner and
  "Converting...", the Convert button reads "Converting..." until the result is there, and
  the pointer shows progress. In the browser the page sat still with no sign of work until
  the result appeared. In the app the time and a Cancel button sit under the spinner.
- The settings panel shows only what you need: each feature shows its switch, and what an
  automatic setting picked, the amounts, sliders, colors and node counts sit under More
  options until you open it. How to edit nodes is the switch's tooltip, and the Sticker card
  offers Cut out background only when the picture's edge needs it.
- A settings card lights up edge to edge under the pointer, in its own shape: a folded card
  all over, an open card across its top with room below it, and all of that lit area folds
  it. It was a small pill floating inside the card, with a gap all around and a "Click to
  show" tip, which is gone.
- Deleting or restoring a shape is several times quicker, most of all in the browser, the
  first time after Convert too: it no longer redoes the corner step for the whole drawing (a
  logo's delete 0.4 s to 0.07 s on a desktop, seconds in a busy browser tab), and every other
  outline stays as it was.
- The Convert button has room around "Converting..." and its spinner, which sat close to its
  edges on the website.
- More room at the window's sides: the toolbar, the settings, the pictures and the status bar
  sit 20 pixels from the edges instead of 12 (12 still on a phone). In the narrowest window
  the logo steps aside so the toolbar's buttons never overlap.

## 0.5.15 (September 28, 2026)

- Slight dips along a straight-looking edge stay: the step that makes edges truly straight
  could flatten a small dip in a small icon's edge (the top of a lips icon at 48 pixels),
  drawing it almost a pixel off. It checks the picture's pixels first now and keeps the dip.
- Small round ends stay round: in a sharp picture, a short stroke with rounded ends (an
  accent over a letter at 48 pixels) could be redrawn as a rectangle with square corners.
  It keeps the round ends now.

## 0.5.14 (September 27, 2026)

- Tiny dots stay: at medium quality, which the app picks for pictures 48 to 160 pixels
  across, a dot or a hole a pixel or two wide could vanish, drawn as an outline with no
  area. It stays now, and photographs come out a little smoother and smaller.
- A corner the picture cuts or rounds stays that way: the step that redraws an outline as
  a true shape could turn a rectangle with one corner cut off or rounded into a plain
  rectangle, squaring that corner. It keeps such a corner now, and draws a rectangle whose
  corners differ with each corner as the picture shows it.
- Busy enlarged pictures lose their grey blur: an enlarged icon with thin strokes close
  together could be mistaken for a sharp picture, so the blur between its shapes was drawn
  as grey bands. It is recognized now and sharpened like any other enlarged picture.
- Points and slots stay sharp at medium quality, which the app picks for pictures 48 to
  160 pixels across: its smoothing rounded off narrow points and V-shaped slots and merged
  small details. Sharp pictures there now keep them.
- Thin gaps and lines in enlarged pictures come back crisp: in a black-and-white (or any
  two-colour) picture that was enlarged or blurred, a gap or a line narrower than the blur
  used to come out as a grey band wider than itself, and nearby shapes merged into grey
  patches. The app now works out the sharp picture whose blur it is, so such gaps come back
  white and such lines come back as thin solid strokes.
- Tiny holes and slivers stay in small pictures: in a sharp two-colour picture, a hole two
  pixels wide or a thin sliver between two shapes could be merged away, and small details
  could come out as a grey patch. They are drawn back from the pixels now.
- A long curved edge between two squared corners stays in place: when the square-corner
  step squared both ends of one long curved edge (the lower lip of an enlarged lips icon),
  each end was fitted as if the other had not moved, and together they bowed the edge
  about a pixel past the picture's. It keeps to the picture now.

## 0.5.13 (September 27, 2026)

- Corners come back inside small lettering that was enlarged: the tracing draws the
  inside of such a letter as a long chain of short pieces, and the square-corner step
  used to skip chains that long, so the cut end of an enlarged "a" stayed rounded. It now
  finds the corners inside them, and a round shape (a scalloped edge, a round shoulder)
  stays round.
- Corners that are alike come out alike: on a blurred icon with two mirror-image inner
  corners, one could come out square and the other round. Both come out square now.

## 0.5.12 (September 27, 2026)

- In a browser tab the drawing shows right away: the tab used to stay blank while it
  squared up corners (about 11 seconds for an enlarged line of lettering); it now shows
  the drawing first and squares the corners just after, saying so meanwhile. The
  finished drawing is the same.
- Moving a node, straightening, the Shapes card and the sticker no longer redo the corner
  step, in the app and in the browser.

## 0.5.11 (September 27, 2026)

- Sharp points stay sharp: where a crisp picture has a thin sharp point, the tracing
  could round it off into a small cap and the corner step then pushed the cap into a
  bulge past the point (on a GitHub logo, one of the two alike points at the neck came
  out as a hook). Both come out as corners where the picture has them: the corner step
  no longer moves the outline where it cannot see the picture, and the square-corner
  step now works on crisp pictures as well as enlarged ones.

## 0.5.10 (September 27, 2026)

- Square corners stay square in a soft or enlarged picture: where the blur had rounded a
  corner off, the app finds the corners the pixels show under their blur and draws them
  sharp, so an enlarged M's stem has a flat top and square corners instead of a dome, and
  the narrow V between two strokes is one clean line to its tip instead of a small white
  step. A shape the picture has round stays round, and a crisp picture traces as before.
- The version in the browser draws a soft picture exactly as the app does (0.5.9 drew two
  of its five soft test pictures slightly differently).

## 0.5.9 (September 27, 2026)

- No grey rim around shapes in a soft or enlarged picture: the app measures how blurred
  the picture's edges are and, past an ordinary screenshot's, sharpens them back to one
  pixel before tracing, so a dark logo on white comes out dark on white, not with a grey
  band round every shape and grey filling its narrow gaps. A crisp picture traces exactly
  as before.

## 0.5.8 (September 27, 2026)

- No thin spikes where a soft picture has a rounded corner or a gentle curve: the corner
  step no longer draws a curve that runs back over itself or curls into a spike beside the
  corner (0.5.7 drew such a thorn on 20 of 900 test icons).

## 0.5.7 (September 27, 2026)

- Corners the tracing rounded off are put back where the picture has them: where a
  GitHub logo's ear meets its head, the outline no longer dips before it climbs the ear.

## 0.5.6 (September 26, 2026)

- A hover tip over the picture no longer draws two thin streaks off its lower corners: tips
  get a smaller shadow, one they are tall enough to carry.
- A node's menu says what its delete does: Delete, keep the shape.

## 0.5.5 (September 26, 2026)

- Try a sample opens a small, visibly blocky picture (the SVG logo at 96 pixels), so the
  original and the vector look different at a glance.
- The Save popup says what PDF and EPS are for.
- `vectormagik-preview --scale 2` draws the same window at twice the pixels.

## 0.5.4 (September 26, 2026)

- A small picture comes up a whole number of times bigger, to about half the space (two or
  three times on a big screen), instead of at its own size in the middle.
- 1x, 2x and 3x beside Fit for a small picture, and the 2 and 3 keys.
- The logo, window icon and Mac icon in the app's blue and greys.

## 0.5.3 (September 26, 2026)

- A picture is shown at most at its own size: a small one is no longer blown up to fill
  the window.
- The picture cards are sized to the picture, grow out of the welcome card when it opens
  and fold back into it when it closes.
- In the browser, the drop zone lights up while a file is dragged over it (it lit for one
  frame).
- The browser version's loading screen and menu in the app's own colours, not the site's
  teal.
- No scroll bars around a picture that fits its card (a hair of one showed under the
  pointer).
- Hold: the original stays for as long as its button is held (it let go after a moment),
  and the Original and Vector buttons no longer shift when pressed.
- In the browser, a file dropped outside the drop zone does nothing instead of replacing
  the app, and the Save panel's file card shows a pointing hand, not a grab.

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

- For AI assistants: `vectormagik-cli --mcp` is an MCP server, so Claude, Cursor and
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
- Export to SVG, PDF, EPS, AI, DXF, EMF and PNG; shapes
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

- The tracing engine, written in Rust.
- Improved defaults, each measured: better tracing settings for artwork and photographs,
  true lines and circles, shapes drawn as circles, ellipses and rounded rectangles,
  straightening, Auto simplification, fills taken from the pixels, pixel art traced on its
  pixel edges.
- The desktop editor: Auto settings, overlay and side-by-side views with Hold to compare,
  curve nodes you can round, move and put back, shape selection (delete, merge), color
  limits and backgrounds, die-cut stickers, undo and redo, convert automatically, and
  saving as SVG, PDF or EPS at any size or by dragging the file out.
- A command line that runs the desktop's chain without a window.
