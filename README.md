# VectorMagik

**Turn pictures into clean vectors, offline, on Windows.** Drop in a PNG, JPEG, GIF,
BMP or PNM and get SVG, PDF or EPS back: logos, artwork, pixel art and photographs.
Nothing leaves your computer.

[![License](https://img.shields.io/badge/license-PolyForm%20Noncommercial-orange)](#licence)
![Platform](https://img.shields.io/badge/platform-Windows%20x64-blue)
![Rust](https://img.shields.io/badge/built%20with-Rust-b7410e)

![VectorMagik converting a logo, with its curve nodes shown](docs/overlay.png)

## Download

**[Download VectorMagik for Windows (x64)](https://github.com/LunarWerxs/VectorMagik/releases/latest/download/VectorMagik-windows-x64.zip)**,
unzip it anywhere and run `VectorMagik.exe`. No installer, no account, no internet.
Windows may warn about an unrecognised app the first time; choose *More info* then *Run anyway*.

## What it does

- **Auto settings** work out the kind of picture (artwork with or without anti-aliasing,
  pixel art, photograph) and its quality, and pick the tracing settings for it.
- **Clean curves**: true straight lines and circles where the picture has them, circles,
  ellipses and rounded rectangles drawn as those shapes, near-straight edges squared up,
  and a Simplify slider that removes nodes while the picture stays the same.
- **Edit by hand**: show the curve nodes, click a corner to round it (Tiny, Tight,
  Medium or Wide), drag a node to move it, right-click for more. Ctrl+Z undoes anything.
- **Shapes**: select shapes to delete them (a background, say) or merge two shades into one.
- **Colors and background**: limit the picture to 1 to 64 colors, or flatten
  transparency onto white, black or any color.
- **Stickers**: a die-cut outline (border, white rim, optional shadow) around the shapes.
- **Save** as SVG, PDF or EPS at 1x, 2x, 4x, 8x or any width, through the save dialog
  or by dragging the file straight out of the window.

![A photograph and its vector side by side](docs/side-by-side.png)

## Command line

`vector-magic-rebuild.exe` in the same folder does the same work without a window:

```text
vector-magic-rebuild picture.png -o picture.svg --category auto --quality auto --simplify auto
```

The desktop's own chain is `--category auto --quality auto --simplify auto --regularize 0.8
--straighten auto --primitives on --stack on`. Run it with `--help` for every option
(`--colors`, `--background`, `--sticker`, `--advanced`, PDF and EPS output and more).

## Build from source

With a current stable Rust toolchain on Windows:

```text
cargo build --release --manifest-path kit/app/Cargo.toml --features desktop
cargo test --release --manifest-path kit/rust/Cargo.toml
cargo test --release --manifest-path kit/app/Cargo.toml --features desktop
```

The programs land in `kit/app/target/release/`: `vector-magic-desktop.exe` (the app),
`vector-magic-rebuild.exe` (the command line) and `vector-magic-preview.exe` (renders the
app's window to a PNG without opening it). The engine library is `kit/rust`
(`vector-rebuild`); `kit/docs` describes its stages.

## How it works

VectorMagik is an independent reimplementation, in Rust, of the tracing engine of
Vector Magic's desktop program, reconstructed stage by stage from the original program
(segmentation, contour construction, smoothing, curve fitting and export) and checked
against it: under `--defaults original` it reproduces the original's output byte for byte
on the reference pictures in `kit/fixtures/reference`. On top of that engine it adds its
own improvements, each measured before it became a default: better tracing settings,
true lines, circles and shapes, straightening, simplification that keeps circles round,
fills taken from the pixels, and the desktop editor.

VectorMagik is not affiliated with, endorsed by or connected to Vector Magic, Inc.
"Vector Magic" is their trademark. It contains none of the original program's machine code or files.

## Licence

Free for personal and other noncommercial use under the
[PolyForm Noncommercial License 1.0.0](LICENSE). Commercial use needs a licence from
LunarWerx: [open an issue](https://github.com/LunarWerxs/VectorMagik/issues/new) to ask.

Required Notice: Copyright (c) 2026 LunarWerx.

The test pictures are free to use: the sample logos are public domain or freely licensed
(`kit/fixtures/samples/LICENSE.txt`), and the photographs come from scikit-image's sample
data (astronaut: NASA, public domain; coffee: Rachel Michetti, CC0; chelsea: Stefan van der
Walt, CC0).
