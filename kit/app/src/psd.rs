//! Photoshop documents (PSD) and large documents (PSB), read as one picture
//! to trace (the owner, September 24, 2026: "allow Photoshop imports, like
//! PSD"; the original Vector Magic opened them too). The `image` crate reads
//! neither, so this follows Adobe's file format specification.
//!
//! The picture is the document's merged composite, the image data at the end
//! of the file, which Photoshop writes whenever "Maximize compatibility" is
//! on. Without it that composite is a blank white page (the version info
//! resource says so), and the visible layers are put together here instead.
//! Every layer is drawn with the normal blend mode whatever its own (a
//! multiply or screen layer comes out as a normal one), at its opacity and
//! fill opacity, through its layer mask, and when clipped, through the
//! coverage of its base layer (exact for an opaque base, close for a
//! translucent one). A hidden group hides its layers and a group's opacity
//! multiplies theirs (overlapping layers in a translucent group then show
//! through each other, which Photoshop's flattened group would not). Shape
//! and fill layers are drawn from their vector art (`psd/vector.rs`, which
//! `shapes` also offers as vector artwork); adjustment layers, layer effects
//! and the vector masks of raster layers are left out. A document that a
//! program other than Photoshop wrote (it has no version info) may leave its
//! shape layers out of its composite, which then shows one colour (ag-psd
//! does, as VectorMojo's sample shows); with visible shape layers its layers
//! are put together instead.
//!
//! Colour: bitmap (a set bit is black), grayscale and duotone (as
//! grayscale), indexed (with its transparent index), RGB, CMYK converted
//! naively from its ink amounts without a colour profile (Photoshop stores
//! CMYK inverted, 255 meaning no ink), Lab through the CIE formulas under
//! D50, Photoshop's white, to sRGB, and multichannel as RGB from its first
//! three channels or grey from its first. 16-bit samples are scaled to 8
//! bits; 32-bit samples are linear light, clamped to 0..1 and encoded as sRGB
//! (alpha is coverage and is scaled). A malformed file is refused in plain
//! words, and the pixel limit is checked before anything is allocated.
use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZFlush, MZStatus};
use vector::{Page, Shape};
use vector_rebuild::raster::{Raster, Rgba};

mod descriptor;
mod vector;

/// What every read past the end of the file, or of one of its sections, says.
const SHORT: &str = "This Photoshop document is cut short or damaged";

/// The widest and tallest a large document (PSB) may be; no layer is decoded
/// past it either.
const PSB_SIDE: u32 = 300_000;

/// Additional layer information keys whose length takes eight bytes in a
/// large document.
const WIDE_KEYS: [&[u8; 4]; 13] = [
    b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2",
    b"FEid", b"FXid", b"PxSD",
];

/// The keys of adjustment and fill layers, whose pixels are not what they
/// show.
const ADJUSTMENTS: [&[u8; 4]; 20] = [
    b"SoCo", b"GdFl", b"PtFl", b"brit", b"levl", b"curv", b"expA", b"vibA", b"hue ", b"hue2",
    b"blnc", b"blwh", b"phfl", b"mixr", b"clrL", b"nvrt", b"post", b"thrs", b"grdm", b"selc",
];

/// The picture a Photoshop document (PSD or PSB) shows, refused when it has
/// more than `max_pixels` pixels.
pub fn decode(bytes: &[u8], max_pixels: u64) -> Result<Raster, String> {
    let mut file = Reader::new(bytes);
    let header = Header::read(&mut file)?;
    if header.width as u64 * header.height as u64 > max_pixels {
        return Err(format!(
            "Image must contain 1 to {} million pixels",
            max_pixels / 1_000_000
        ));
    }
    let colour_data = file.section(false)?;
    let palette = if header.mode == Mode::Indexed {
        // Red for the 256 entries, then green, then blue.
        colour_data
            .get(..768)
            .ok_or("This indexed-colour Photoshop document has no colour table")?
    } else {
        &[]
    };
    let resources = Resources::read(file.section(false)?);
    let layers = LayerSection::read(file.section(header.psb)?, header.psb);
    let compositor = || Compositor::new(&header, palette, max_pixels, resources.ppi);
    if !resources.merged {
        if let Some(info) = layers.info {
            let records = read_layers(info, header.psb)?;
            if !records.is_empty() {
                return compositor().run(&records);
            }
        }
    }
    let alpha = merged_alpha(&header, &resources, &layers);
    let picture = merged(
        file.rest(),
        &header,
        palette,
        alpha,
        resources.transparent_index,
    )?;
    // A composite of one colour from a program other than Photoshop may
    // leave the shape layers out.
    if !resources.versioned && picture.pixels.windows(2).all(|pair| pair[0] == pair[1]) {
        if let Some(records) = layers
            .info
            .and_then(|info| read_layers(info, header.psb).ok())
        {
            let page = Page {
                width: header.width,
                height: header.height,
                ppi: resources.ppi,
            };
            let shapes = records
                .iter()
                .zip(group_opacities(&records))
                .any(|(layer, shown)| {
                    shown.is_some()
                        && layer.section == 0
                        && Shape::read(&layer.vector, page).is_ok_and(|shape| shape.is_some())
                });
            if shapes {
                return compositor().run(&records);
            }
        }
    }
    Ok(picture)
}

/// The shape layers of a Photoshop document (PSD or PSB) as vector artwork:
/// every visible shape and fill layer, bottom first, as the flat paths
/// `import::Imported` holds, on a page of the document's size at its
/// resolution (0.75 points a pixel without one); `None` when it has no
/// visible shape layer. What else it holds (raster and text layers,
/// effects) is named in `skipped`, and a shape layer whose settings are
/// damaged refuses the whole document.
pub fn shapes(bytes: &[u8]) -> Result<Option<crate::import::Imported>, String> {
    let mut file = Reader::new(bytes);
    let header = Header::read(&mut file)?;
    file.section(false)?; // The colour mode data.
    let resources = Resources::read(file.section(false)?);
    let layers = LayerSection::read(file.section(header.psb)?, header.psb);
    let Some(info) = layers.info else {
        return Ok(None);
    };
    let page = Page {
        width: header.width,
        height: header.height,
        ppi: resources.ppi,
    };
    vector::artwork(page, &read_layers(info, header.psb)?)
}

/// A cursor over big-endian bytes that refuses to read past their end.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, at: 0 }
    }
    fn rest(&self) -> &'a [u8] {
        &self.bytes[self.at..]
    }
    fn take(&mut self, n: u64) -> Result<&'a [u8], String> {
        let rest = self.rest();
        let n = usize::try_from(n)
            .ok()
            .filter(|&n| n <= rest.len())
            .ok_or(SHORT)?;
        self.at += n;
        Ok(&rest[..n])
    }
    fn skip(&mut self, n: u64) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N as u64)?);
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, String> {
        self.array().map(|[byte]| byte)
    }
    fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_be_bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_be_bytes)
    }
    fn i32(&mut self) -> Result<i32, String> {
        self.array().map(i32::from_be_bytes)
    }
    /// A length field: four bytes, or eight in a large document where the
    /// specification says so (`wide`).
    fn length(&mut self, wide: bool) -> Result<u64, String> {
        if wide {
            self.array().map(u64::from_be_bytes)
        } else {
            self.u32().map(u64::from)
        }
    }
    /// A section: its length, then that many bytes.
    fn section(&mut self, wide: bool) -> Result<&'a [u8], String> {
        let n = self.length(wide)?;
        self.take(n)
    }
}

/// Photoshop's colour modes, as the header numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Bitmap,
    Grayscale,
    Indexed,
    Rgb,
    Cmyk,
    Multichannel,
    Duotone,
    Lab,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Bitmap => "bitmap",
            Mode::Grayscale => "grayscale",
            Mode::Indexed => "indexed-colour",
            Mode::Rgb => "RGB",
            Mode::Cmyk => "CMYK",
            Mode::Multichannel => "multichannel",
            Mode::Duotone => "duotone",
            Mode::Lab => "Lab",
        }
    }
}

/// The file header: what the rest of the file is laid out by.
struct Header {
    /// A large document (PSB, version 2): some lengths take eight bytes and
    /// PackBits row lengths four.
    psb: bool,
    channels: usize,
    width: usize,
    height: usize,
    depth: u16,
    mode: Mode,
}

impl Header {
    fn read(file: &mut Reader) -> Result<Header, String> {
        if file.take(4).ok() != Some(&b"8BPS"[..]) {
            return Err("This is not a Photoshop document".into());
        }
        let psb = match file.u16()? {
            1 => false,
            2 => true,
            version => {
                return Err(format!(
                    "Photoshop document version {version} is not one this app reads"
                ))
            }
        };
        file.skip(6)?;
        let channels = file.u16()?;
        let (height, width) = (file.u32()?, file.u32()?);
        let (depth, mode) = (file.u16()?, file.u16()?);
        if !(1..=56).contains(&channels) {
            return Err(format!(
                "A Photoshop document has 1 to 56 channels, not {channels}"
            ));
        }
        let side = if psb { PSB_SIDE } else { 30_000 };
        if width == 0 || height == 0 || width > side || height > side {
            return Err(format!(
                "A Photoshop document is 1 to {side} pixels wide and high, not {width} by {height}"
            ));
        }
        let mode = match mode {
            0 => Mode::Bitmap,
            1 => Mode::Grayscale,
            2 => Mode::Indexed,
            3 => Mode::Rgb,
            4 => Mode::Cmyk,
            7 => Mode::Multichannel,
            8 => Mode::Duotone,
            9 => Mode::Lab,
            _ => {
                return Err(format!(
                    "Photoshop colour mode {mode} is not one this app reads"
                ))
            }
        };
        let depths: &[u16] = match mode {
            Mode::Bitmap => &[1],
            Mode::Indexed => &[8],
            _ => &[8, 16, 32],
        };
        if !depths.contains(&depth) {
            return Err(format!(
                "A {} Photoshop document cannot have {depth} bits a channel",
                mode.name()
            ));
        }
        let header = Header {
            psb,
            channels: channels.into(),
            width: width as usize,
            height: height as usize,
            depth,
            mode,
        };
        if header.channels < header.colours() {
            return Err(format!(
                "This {} Photoshop document has too few channels ({channels})",
                mode.name()
            ));
        }
        Ok(header)
    }

    /// How many channels make up the colour, before any alpha.
    fn colours(&self) -> usize {
        match self.mode {
            Mode::Rgb | Mode::Lab => 3,
            Mode::Cmyk => 4,
            Mode::Multichannel if self.channels >= 3 => 3,
            _ => 1,
        }
    }

    /// The bytes a row of `width` samples takes.
    fn row_bytes(&self, width: usize) -> usize {
        if self.depth == 1 {
            width.div_ceil(8)
        } else {
            width * usize::from(self.depth / 8)
        }
    }

    /// Whether 32-bit samples of a channel are linear light to encode as
    /// sRGB: colour channels, except Lab's and CMYK's, which are not light.
    fn gamma(&self, colour: bool) -> bool {
        self.depth == 32 && colour && !matches!(self.mode, Mode::Lab | Mode::Cmyk)
    }
}

/// What the image resources say about the composite.
struct Resources {
    /// Whether the composite at the end of the file is real (resource 1057,
    /// the version info); it is not when "Maximize compatibility" was off.
    merged: bool,
    /// Whether the version info is there at all: Photoshop always writes it.
    versioned: bool,
    /// The palette index an indexed-colour document shows as transparent
    /// (resource 1047).
    transparent_index: Option<u8>,
    /// The identifiers of the alpha channels (resource 1053); a zero marks
    /// the composite's transparency.
    alpha_ids: Vec<u32>,
    /// The resolution in pixels an inch (resource 1005, a 16.16 fixed-point
    /// number whatever unit it is shown in): the size of the page shape
    /// layers are drawn on.
    ppi: Option<f64>,
}

impl Resources {
    /// The resources in `section`. A malformed block ends the list: they only
    /// refine the picture.
    fn read(section: &[u8]) -> Resources {
        let mut found = Resources {
            merged: true,
            versioned: false,
            transparent_index: None,
            alpha_ids: Vec::new(),
            ppi: None,
        };
        let mut r = Reader::new(section);
        while let Ok((id, data)) = resource(&mut r) {
            found.versioned |= id == 1057;
            match (id, data) {
                (1005, [a, b, c, d, ..]) => {
                    let ppi = f64::from(u32::from_be_bytes([*a, *b, *c, *d])) / 65_536.;
                    found.ppi = Some(ppi).filter(|ppi| *ppi >= 1.);
                }
                (1057, [_, _, _, _, merged, ..]) => found.merged = *merged != 0,
                (1047, [high, low, ..]) => {
                    found.transparent_index = u8::try_from(u16::from_be_bytes([*high, *low])).ok()
                }
                (1053, _) => {
                    found.alpha_ids = data
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|id| u32::from_be_bytes(*id))
                        .collect()
                }
                _ => {}
            }
        }
        found
    }
}

/// One image resource block: its id and its data.
fn resource<'a>(r: &mut Reader<'a>) -> Result<(u16, &'a [u8]), String> {
    r.skip(4)?; // The signature, 8BIM.
    let id = r.u16()?;
    // The name is a Pascal string padded to an even length, its length byte
    // included.
    let name = u64::from(r.u8()?);
    r.skip(name + (name + 1) % 2)?;
    let data = r.section(false)?;
    let _ = r.skip(data.len() as u64 % 2);
    Ok((id, data))
}

/// The layer and mask information section, as far as the composite needs it.
#[derive(Default)]
struct LayerSection<'a> {
    /// The layer info from its layer count on: the section's own, or the one
    /// a 16- or 32-bit document keeps in its `Lr16` or `Lr32` block instead.
    info: Option<&'a [u8]>,
    /// The layer count as the section gives it: negative when the first alpha
    /// channel of the composite is its transparency.
    count: i16,
    /// Whether a `Mtrn`, `Mt16` or `Mt32` block says the composite has
    /// transparency.
    merged_transparency: bool,
}

impl<'a> LayerSection<'a> {
    /// The section's parts; a malformed one is left out, as the composite
    /// does not depend on them.
    fn read(section: &'a [u8], psb: bool) -> LayerSection<'a> {
        let mut found = LayerSection::default();
        let mut r = Reader::new(section);
        let Ok(info) = r.section(psb) else {
            return found;
        };
        if let [high, low, ..] = info {
            found.info = Some(info);
            found.count = i16::from_be_bytes([*high, *low]);
        }
        // The global layer mask info, then the additional information.
        if r.section(false).is_ok() {
            for (key, data) in tagged_blocks(r.rest(), psb) {
                match &key {
                    b"Lr16" | b"Lr32" if data.len() >= 2 => {
                        if found.info.is_none() {
                            found.count = i16::from_be_bytes([data[0], data[1]]);
                        }
                        found.info = Some(data);
                    }
                    b"Mtrn" | b"Mt16" | b"Mt32" => found.merged_transparency = true,
                    _ => {}
                }
            }
        }
        found
    }
}

/// The additional layer information blocks in `bytes`, key and data. Writers
/// pad them to two or four bytes, so up to three bytes of padding are
/// stepped over; anything else ends the list.
fn tagged_blocks(bytes: &[u8], psb: bool) -> Vec<([u8; 4], &[u8])> {
    let mut blocks = Vec::new();
    let mut r = Reader::new(bytes);
    while let Some(pad) = (0..4).find(|&pad| {
        r.rest()
            .get(pad..pad + 4)
            .is_some_and(|signature| signature == b"8BIM" || signature == b"8B64")
    }) {
        let _ = r.skip(pad as u64 + 4);
        let Ok(key) = r.array::<4>() else { break };
        let wide = psb && WIDE_KEYS.contains(&&key);
        let Ok(data) = r.section(wide) else { break };
        blocks.push((key, data));
    }
    blocks
}

/// Which channel of the composite is its transparency, if any, by
/// Photoshop's rules as psd-tools reads them: a `Mtrn` block or a negative
/// layer count make it the first channel after the colour ones; otherwise
/// the extra channels are saved selections when there are layers or the
/// alpha identifiers say so, and a zero identifier marks the transparency.
fn merged_alpha(header: &Header, resources: &Resources, layers: &LayerSection) -> Option<usize> {
    let colours = header.colours();
    let modes = [
        Mode::Grayscale,
        Mode::Duotone,
        Mode::Rgb,
        Mode::Cmyk,
        Mode::Lab,
    ];
    if header.channels <= colours || !modes.contains(&header.mode) {
        return None;
    }
    if layers.merged_transparency || layers.count < 0 {
        return Some(colours);
    }
    if layers.count > 0 {
        return None;
    }
    let ids = &resources.alpha_ids;
    match ids.iter().position(|&id| id == 0) {
        Some(offset) => header
            .channels
            .checked_sub(ids.len())
            .map(|first| first + offset)
            .filter(|index| (colours..header.channels).contains(index)),
        None if ids.is_empty() => Some(colours),
        None => None,
    }
}

/// The composite image data at the end of the file, with its transparency
/// from channel `alpha` or, in an indexed-colour document, from
/// `transparent_index`.
fn merged(
    data: &[u8],
    header: &Header,
    palette: &[u8],
    alpha: Option<usize>,
    transparent_index: Option<u8>,
) -> Result<Raster, String> {
    let (width, height) = (header.width, header.height);
    let mut r = Reader::new(data);
    let method = r.u16()?;
    let rows = header.channels as u64 * height as u64;
    let mut rows = Rows::new(method, r.rest(), rows, header.psb)?;
    let whole = Rect {
        top: 0,
        left: 0,
        bottom: height as i64,
        right: width as i64,
    };
    let window = Window::of(whole, width, height);
    let curve = (header.depth == 32).then(srgb_curve);
    let colours = header.colours();
    let mut planes = Vec::new();
    let mut skipped = vec![0; header.row_bytes(width)];
    for index in 0..=alpha.unwrap_or(colours - 1) {
        if index < colours || Some(index) == alpha {
            let gamma = curve.as_deref().filter(|_| header.gamma(index < colours));
            planes.push(plane(&mut rows, whole, window, header, gamma)?);
        } else {
            for _ in 0..height {
                rows.row(&mut skipped, header.depth, false)?;
            }
        }
    }
    let (colour_planes, alpha_plane) = planes.split_at(colours);
    let mut samples = [0; 4];
    let pixels = (0..width * height)
        .map(|i| {
            for (sample, plane) in samples.iter_mut().zip(colour_planes) {
                *sample = plane[i];
            }
            let [r, g, b] = colour(header.mode, &samples[..colours], palette);
            let a = match alpha_plane.first() {
                Some(plane) => plane[i],
                None if header.mode == Mode::Indexed && transparent_index == Some(samples[0]) => 0,
                None => 255,
            };
            Rgba([r, g, b, a])
        })
        .collect();
    Ok(Raster {
        width,
        height,
        pixels,
    })
}

/// Channel rows as the file stores them: raw, PackBits, or zlib with or
/// without prediction.
enum Rows<'a> {
    Raw(Reader<'a>),
    PackBits {
        /// Each row's compressed length.
        lengths: Reader<'a>,
        data: Reader<'a>,
        /// Whether a length takes four bytes (a large document) or two.
        wide: bool,
    },
    Zip {
        input: &'a [u8],
        state: Box<InflateState>,
        /// Whether each row is delta coded (compression method 3).
        predict: bool,
        scratch: Vec<u8>,
    },
}

impl<'a> Rows<'a> {
    /// `rows` rows stored in `data` with compression `method`.
    fn new(method: u16, data: &'a [u8], rows: u64, wide: bool) -> Result<Rows<'a>, String> {
        Ok(match method {
            0 => Rows::Raw(Reader::new(data)),
            1 => {
                let mut r = Reader::new(data);
                let lengths = Reader::new(r.take(rows.saturating_mul(if wide { 4 } else { 2 }))?);
                Rows::PackBits {
                    lengths,
                    data: r,
                    wide,
                }
            }
            2 | 3 => Rows::Zip {
                input: data,
                state: InflateState::new_boxed(DataFormat::Zlib),
                predict: method == 3,
                scratch: Vec::new(),
            },
            _ => {
                return Err(format!(
                    "Photoshop compression method {method} is not one this app reads"
                ))
            }
        })
    }

    /// The next row into `out` (as long as a row), samples of `depth` bits;
    /// a row that is not `wanted` is passed over, decoded only when the rows
    /// after it depend on it.
    fn row(&mut self, out: &mut [u8], depth: u16, wanted: bool) -> Result<(), String> {
        match self {
            Rows::Raw(data) => {
                let row = data.take(out.len() as u64)?;
                if wanted {
                    out.copy_from_slice(row);
                }
            }
            Rows::PackBits {
                lengths,
                data,
                wide,
            } => {
                let n = if *wide {
                    u64::from(lengths.u32()?)
                } else {
                    u64::from(lengths.u16()?)
                };
                let packed = data.take(n)?;
                if wanted {
                    unpack(packed, out)?;
                }
            }
            Rows::Zip {
                input,
                state,
                predict,
                scratch,
            } => {
                inflate_into(state, input, out)?;
                if *predict {
                    unpredict(out, depth, scratch);
                }
            }
        }
        Ok(())
    }
}

/// Expands one PackBits row into `out`; a run past the row's end is cut
/// there.
fn unpack(mut data: &[u8], out: &mut [u8]) -> Result<(), String> {
    let mut at = 0;
    while at < out.len() {
        let (&header, rest) = data.split_first().ok_or(SHORT)?;
        data = rest;
        match header {
            128 => {}
            0..=127 => {
                let n = usize::from(header) + 1;
                let literal = data.get(..n).ok_or(SHORT)?;
                data = &data[n..];
                let end = (at + n).min(out.len());
                out[at..end].copy_from_slice(&literal[..end - at]);
                at = end;
            }
            _ => {
                let (&value, rest) = data.split_first().ok_or(SHORT)?;
                data = rest;
                let end = (at + 257 - usize::from(header)).min(out.len());
                out[at..end].fill(value);
                at = end;
            }
        }
    }
    Ok(())
}

/// Fills `out` from the zlib stream `input`, moving `input` past what was
/// read.
fn inflate_into(state: &mut InflateState, input: &mut &[u8], out: &mut [u8]) -> Result<(), String> {
    let mut filled = 0;
    while filled < out.len() {
        let step = inflate(state, input, &mut out[filled..], MZFlush::None);
        *input = &input[step.bytes_consumed.min(input.len())..];
        filled += step.bytes_written;
        match step.status {
            Ok(MZStatus::Ok) if step.bytes_consumed + step.bytes_written > 0 => {}
            Ok(MZStatus::StreamEnd) if filled >= out.len() => {}
            _ => {
                return Err(
                    "A compressed channel in this Photoshop document is damaged or cut short"
                        .into(),
                )
            }
        }
    }
    Ok(())
}

/// Undoes the prediction of one zlib-compressed row: each sample was stored
/// as its difference from the one before, 16-bit samples as 16-bit numbers,
/// and a 32-bit row with its bytes regrouped first (every sample's first
/// byte, then every second byte, and so on) and delta coded bytewise.
fn unpredict(row: &mut [u8], depth: u16, scratch: &mut Vec<u8>) {
    if depth == 16 {
        let mut previous = 0u16;
        for pair in row.as_chunks_mut::<2>().0 {
            previous = previous.wrapping_add(u16::from_be_bytes(*pair));
            *pair = previous.to_be_bytes();
        }
        return;
    }
    let mut previous = 0u8;
    for byte in row.iter_mut() {
        previous = previous.wrapping_add(*byte);
        *byte = previous;
    }
    if depth == 32 {
        let width = row.len() / 4;
        scratch.clear();
        scratch.extend_from_slice(row);
        for (x, sample) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            for (b, byte) in sample.iter_mut().enumerate() {
                *byte = scratch[b * width + x];
            }
        }
    }
}

/// A rectangle in document pixels, as layer records give it.
#[derive(Clone, Copy, Debug, Default)]
struct Rect {
    top: i64,
    left: i64,
    bottom: i64,
    right: i64,
}

impl Rect {
    fn read(r: &mut Reader) -> Result<Rect, String> {
        let (top, left, bottom, right) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
        Ok(Rect {
            top: top.into(),
            left: left.into(),
            bottom: bottom.into(),
            right: right.into(),
        })
    }
    fn width(&self) -> i64 {
        (self.right - self.left).max(0)
    }
    fn height(&self) -> i64 {
        (self.bottom - self.top).max(0)
    }
}

/// The part of the document a rectangle covers, in document pixels.
#[derive(Clone, Copy, Debug, Default)]
struct Window {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Window {
    fn of(rect: Rect, width: usize, height: usize) -> Window {
        let clamp = |v: i64, end: usize| v.clamp(0, end as i64) as usize;
        let (x0, y0) = (clamp(rect.left, width), clamp(rect.top, height));
        Window {
            x0,
            y0,
            x1: clamp(rect.right, width).max(x0),
            y1: clamp(rect.bottom, height).max(y0),
        }
    }
    fn width(&self) -> usize {
        self.x1 - self.x0
    }
    fn area(&self) -> usize {
        self.width() * (self.y1 - self.y0)
    }
    fn is_empty(&self) -> bool {
        self.area() == 0
    }
    /// Where document pixel (`x`, `y`) is in a plane over this window.
    fn index(&self, x: usize, y: usize) -> Option<usize> {
        ((self.x0..self.x1).contains(&x) && (self.y0..self.y1).contains(&y))
            .then(|| (y - self.y0) * self.width() + x - self.x0)
    }
}

/// The eight-bit samples of one channel, stored in `rows` over `rect`, for
/// the part of it inside `window`, row by row; `gamma` is the sRGB curve for
/// 32-bit colour.
fn plane(
    rows: &mut Rows,
    rect: Rect,
    window: Window,
    header: &Header,
    gamma: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if window.is_empty() {
        return Ok(Vec::new());
    }
    let mut row = vec![0; header.row_bytes(rect.width() as usize)];
    let mut plane = Vec::with_capacity(window.area());
    let from = (window.x0 as i64 - rect.left) as usize;
    for y in rect.top..window.y1 as i64 {
        let wanted = y >= window.y0 as i64;
        rows.row(&mut row, header.depth, wanted)?;
        if wanted {
            plane.extend(
                (from..from + window.width()).map(|x| sample(&row, x, header.depth, gamma)),
            );
        }
    }
    Ok(plane)
}

/// Sample `x` of a row as eight bits: a set bit is black, 16 bits are
/// scaled, 32-bit floats clamped to 0..1 and looked up in `gamma` when they
/// are linear light.
fn sample(row: &[u8], x: usize, depth: u16, gamma: Option<&[u8]>) -> u8 {
    match depth {
        1 => {
            if (row[x / 8] >> (7 - x % 8)) & 1 == 1 {
                0
            } else {
                255
            }
        }
        16 => {
            let v = u32::from(u16::from_be_bytes([row[2 * x], row[2 * x + 1]]));
            ((v * 255 + 32_767) / 65_535) as u8
        }
        32 => {
            let v =
                f32::from_be_bytes([row[4 * x], row[4 * x + 1], row[4 * x + 2], row[4 * x + 3]]);
            // Not a number, too, is nothing.
            let v = if v > 0. { v.min(1.) } else { 0. };
            match gamma {
                Some(curve) => curve[(v * 65_535. + 0.5) as usize],
                None => (v * 255. + 0.5) as u8,
            }
        }
        _ => row[x],
    }
}

/// A linear-light value (0 to 1) as an eight-bit sRGB sample.
fn srgb(v: f64) -> u8 {
    let v = v.clamp(0., 1.);
    let encoded = if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    };
    (encoded * 255. + 0.5) as u8
}

/// `srgb` at every 16-bit step from 0 to 1: what 32-bit colour samples are
/// looked up in.
fn srgb_curve() -> Vec<u8> {
    (0..=65_535u32)
        .map(|i| srgb(f64::from(i) / 65_535.))
        .collect()
}

/// The sRGB colour of one pixel from its colour channels' samples.
fn colour(mode: Mode, samples: &[u8], palette: &[u8]) -> [u8; 3] {
    let entry = |i: usize| palette.get(i).copied().unwrap_or(0);
    match (mode, samples) {
        (Mode::Indexed, [i, ..]) => {
            let i = usize::from(*i);
            [entry(i), entry(256 + i), entry(512 + i)]
        }
        // Stored inverted, so each is the share of light its ink lets
        // through, and so is black's.
        (Mode::Cmyk, [c, m, y, k, ..]) => [ink(*c, *k), ink(*m, *k), ink(*y, *k)],
        (Mode::Lab, [l, a, b, ..]) => lab(*l, *a, *b),
        (_, [r, g, b, ..]) => [*r, *g, *b],
        (_, [v, ..]) => [*v; 3],
        _ => [0; 3],
    }
}

/// The light left by one CMYK ink and black, both stored inverted.
fn ink(colour: u8, black: u8) -> u8 {
    ((u32::from(colour) * u32::from(black) + 127) / 255) as u8
}

/// Photoshop's Lab (L from 0 to 255 for 0 to 100, a and b offset by 128) as
/// sRGB: CIE XYZ under D50, Photoshop's Lab white, then the sRGB primaries
/// adapted to D50 (Bradford).
fn lab(l: u8, a: u8, b: u8) -> [u8; 3] {
    let fy = (f64::from(l) * 100. / 255. + 16.) / 116.;
    let fx = fy + (f64::from(a) - 128.) / 500.;
    let fz = fy - (f64::from(b) - 128.) / 200.;
    let inverse = |t: f64| {
        if t > 6. / 29. {
            t * t * t
        } else {
            3. * (6. / 29.) * (6. / 29.) * (t - 4. / 29.)
        }
    };
    let (x, y, z) = (0.9642 * inverse(fx), inverse(fy), 0.8249 * inverse(fz));
    [
        srgb(3.133_856_1 * x - 1.616_866_7 * y - 0.490_614_6 * z),
        srgb(-0.978_768_4 * x + 1.916_141_5 * y + 0.033_454 * z),
        srgb(0.071_945_3 * x - 0.228_991_4 * y + 1.405_242_7 * z),
    ]
}

/// A layer mask: where it lies and what it is outside that.
#[derive(Clone, Copy)]
struct Mask {
    rect: Rect,
    outside: u8,
    /// Whether it was drawn from other data (flag 8), a vector mask's.
    from_vector: bool,
}

/// A layer's mask from its layer mask data, unless it has none or it is
/// disabled.
fn read_mask(data: &[u8]) -> Option<Mask> {
    let mut r = Reader::new(data);
    let rect = Rect::read(&mut r).ok()?;
    let (outside, flags) = (r.u8().ok()?, r.u8().ok()?);
    (flags & 2 == 0).then_some(Mask {
        rect,
        outside,
        from_vector: flags & 8 != 0,
    })
}

/// One layer record with its channels' image data.
struct Layer<'a> {
    rect: Rect,
    /// Each channel's id (0, 1, 2... colour, -1 transparency, -2 the layer
    /// mask) and its compression method and image data.
    channels: Vec<(i16, &'a [u8])>,
    opacity: u8,
    /// The fill opacity (`iOpa`), which scales a layer without effects as
    /// its opacity does.
    fill: u8,
    /// Whether the layer is clipped to the layer below it.
    clipped: bool,
    hidden: bool,
    /// Its role in a group (`lsct`): 1 and 2 are a group's own record, above
    /// its layers, 3 the divider below them, and 0 a layer.
    section: u32,
    mask: Option<Mask>,
    /// An adjustment or fill layer, whose pixels are not what it shows.
    adjustment: bool,
    /// Its blend mode's key (`norm`, `scrn`, `pass`...); every layer is
    /// drawn as a normal one, and the flat paths of `shapes` say so.
    blend: [u8; 4],
    /// The blocks that make it vector art: a shape or fill layer's.
    vector: vector::Blocks<'a>,
}

/// The layer records of a layer info (from its layer count on), bottom layer
/// first, each with its channels' image data.
fn read_layers(info: &[u8], psb: bool) -> Result<Vec<Layer<'_>>, String> {
    let mut r = Reader::new(info);
    let count = i16::from_be_bytes(r.array()?).unsigned_abs();
    let mut layers = Vec::new();
    let mut lengths = Vec::new();
    for _ in 0..count {
        let rect = Rect::read(&mut r)?;
        let mut channels = Vec::new();
        for _ in 0..r.u16()? {
            channels.push((i16::from_be_bytes(r.array()?), r.length(psb)?));
        }
        if r.take(4)? != b"8BIM" {
            return Err(SHORT.into());
        }
        let blend = r.array()?;
        let (opacity, clipping, flags) = (r.u8()?, r.u8()?, r.u8()?);
        r.skip(1)?;
        let mut extra = Reader::new(r.section(false)?);
        let mask = read_mask(extra.section(false)?);
        extra.section(false)?; // The blending ranges.
                               // The name, a Pascal string padded to four bytes with its length.
        let name = u64::from(extra.u8()?);
        let tail = match extra.skip((name + 1).next_multiple_of(4) - 1) {
            Ok(()) => extra.rest(),
            Err(_) => &[],
        };
        let mut layer = Layer {
            rect,
            channels: Vec::new(),
            opacity,
            fill: 255,
            clipped: clipping != 0,
            hidden: flags & 2 != 0,
            section: 0,
            mask,
            adjustment: false,
            blend,
            vector: vector::Blocks::default(),
        };
        for (key, data) in tagged_blocks(tail, psb) {
            let vector = &mut layer.vector;
            match (&key, data) {
                (b"lsct" | b"lsdk", [a, b, c, d, ..]) => {
                    layer.section = u32::from_be_bytes([*a, *b, *c, *d])
                }
                (b"iOpa", [fill, ..]) => layer.fill = *fill,
                (b"vmsk" | b"vsms", _) => {
                    vector.mask.get_or_insert(data);
                }
                (b"vscg", _) => vector.content = Some(data),
                (b"vstk", _) => vector.stroke = Some(data),
                (b"lfx2" | b"lmfx", _) => vector.effects = Some(data),
                (b"TySh", _) => vector.text = true,
                (b"SoCo" | b"GdFl" | b"PtFl", _) => {
                    vector.fill = Some(data);
                    layer.adjustment = true;
                }
                (key, _) if ADJUSTMENTS.contains(&key) => layer.adjustment = true,
                _ => {}
            }
        }
        layers.push(layer);
        lengths.push(channels);
    }
    for (layer, channels) in layers.iter_mut().zip(lengths) {
        for (id, length) in channels {
            layer.channels.push((id, r.take(length)?));
        }
    }
    Ok(layers)
}

/// Each layer's share of its groups, bottom layer first: the product of the
/// opacities of the groups holding it, or `None` when it or one of them is
/// hidden. A group's divider comes below its layers and its own record above
/// them, so the records are read top down.
fn group_opacities(layers: &[Layer]) -> Vec<Option<f32>> {
    let mut shown = vec![None; layers.len()];
    let mut groups: Vec<Option<f32>> = Vec::new();
    for (i, layer) in layers.iter().enumerate().rev() {
        let own = groups
            .last()
            .copied()
            .unwrap_or(Some(1.))
            .filter(|_| !layer.hidden);
        match layer.section {
            1 | 2 => groups.push(own.map(|o| o * f32::from(layer.opacity) / 255.)),
            3 => {
                groups.pop();
            }
            _ => shown[i] = own,
        }
    }
    shown
}

/// How much of each pixel a layer covered (0 to 255) over its window: what
/// the layers clipped to it are drawn within.
#[derive(Default)]
struct Coverage {
    window: Window,
    values: Vec<u8>,
}

impl Coverage {
    fn at(&self, x: usize, y: usize) -> u8 {
        self.window
            .index(x, y)
            .and_then(|i| self.values.get(i))
            .copied()
            .unwrap_or(0)
    }
}

/// Puts the visible layers together when the file has no composite of its
/// own.
struct Compositor<'h> {
    header: &'h Header,
    palette: &'h [u8],
    curve: Option<Vec<u8>>,
    canvas: Vec<Rgba>,
    /// The layer pixels still allowed to be decoded: sixteen times the pixel
    /// limit, so a file of many huge layers cannot keep the app busy.
    budget: u64,
    max_pixels: u64,
    /// The document's resolution, which a stroke's width in points needs.
    ppi: Option<f64>,
}

impl<'h> Compositor<'h> {
    fn new(header: &'h Header, palette: &'h [u8], max_pixels: u64, ppi: Option<f64>) -> Self {
        Compositor {
            header,
            palette,
            curve: (header.depth == 32).then(srgb_curve),
            canvas: vec![Rgba([0; 4]); header.width * header.height],
            budget: max_pixels.saturating_mul(16),
            max_pixels,
            ppi,
        }
    }

    /// The document as shape layers are drawn on it.
    fn page(&self) -> Page {
        Page {
            width: self.header.width,
            height: self.header.height,
            ppi: self.ppi,
        }
    }

    /// The layers, bottom first, drawn one over another on a transparent
    /// canvas.
    fn run(mut self, layers: &[Layer]) -> Result<Raster, String> {
        // What clipped layers are drawn within: the last unclipped layer's
        // coverage, empty when it showed nothing.
        let mut base: Option<Coverage> = None;
        for (layer, shown) in layers.iter().zip(group_opacities(layers)) {
            if layer.section != 0 {
                // A group's own records are no picture, and clipping stays
                // within a group.
                base = None;
                continue;
            }
            let clip = if layer.clipped { base.as_ref() } else { None };
            let drawn = match shown {
                Some(group) if !clip.is_some_and(|c| c.values.is_empty()) => {
                    // A shape or fill layer is drawn from its vector art; one
                    // whose settings are damaged is drawn as before (its
                    // pixels, or nothing for a fill layer).
                    let shape = Shape::read(&layer.vector, self.page()).ok().flatten();
                    if shape.is_some() || !layer.adjustment {
                        self.draw(layer, shape.as_ref(), group, clip)?
                    } else {
                        Coverage::default()
                    }
                }
                _ => Coverage::default(),
            };
            if clip.is_none() {
                base = Some(drawn);
            }
        }
        Ok(Raster {
            width: self.header.width,
            height: self.header.height,
            pixels: self.canvas,
        })
    }

    /// Counts a channel over `rect` against the limits before it is decoded.
    fn claim(&mut self, rect: Rect) -> Result<(), String> {
        let side = i64::from(PSB_SIDE);
        let pixels = rect.width() as u64 * rect.height() as u64;
        if rect.width() > side || rect.height() > side || pixels > self.max_pixels {
            return Err(format!(
                "A layer in this Photoshop document is larger than {} million pixels",
                self.max_pixels / 1_000_000
            ));
        }
        self.budget = self
            .budget
            .checked_sub(pixels)
            .ok_or("The layers of this Photoshop document hold too many pixels to put together")?;
        Ok(())
    }

    /// Draws `layer` (from its vector art when it is a `shape`) at `group`
    /// times its own opacity, within `clip` when it is clipped, and returns
    /// how much of each pixel it covered.
    fn draw(
        &mut self,
        layer: &Layer,
        shape: Option<&Shape>,
        group: f32,
        clip: Option<&Coverage>,
    ) -> Result<Coverage, String> {
        let header = self.header;
        let (width, height) = (header.width, header.height);
        let page = self.page();
        let shape = match shape {
            Some(shape) => match shape.draw(page, &mut |rect| self.claim(rect))? {
                Some(drawn) => Some(drawn),
                None => return Ok(Coverage::default()),
            },
            None => None,
        };
        let window = match &shape {
            Some(drawn) => drawn.window,
            None => Window::of(layer.rect, width, height),
        };
        if window.is_empty() {
            return Ok(Coverage::default());
        }
        if shape.is_none() {
            self.claim(layer.rect)?;
        }
        let channel = |id: i16| {
            layer
                .channels
                .iter()
                .find(|(c, _)| *c == id)
                .map(|&(_, data)| data)
        };
        let mask = layer
            .mask
            .zip(channel(-2))
            .map(|(mask, data)| (mask, data, Window::of(mask.rect, width, height)));
        if let Some((mask, _, mask_window)) = &mask {
            if !mask_window.is_empty() {
                self.claim(mask.rect)?;
            }
        }
        let curve = self.curve.as_deref();
        let colours = header.colours();
        let mut planes = Vec::with_capacity(colours);
        let mut transparency = None;
        if shape.is_none() {
            for c in 0..colours {
                let gamma = curve.filter(|_| header.gamma(true));
                let data = channel(c as i16);
                let plane = data.map(|data| layer_plane(data, layer.rect, window, header, gamma));
                planes.push(plane.transpose()?.flatten());
            }
            transparency = channel(-1)
                .map(|data| layer_plane(data, layer.rect, window, header, None))
                .transpose()?
                .flatten();
        }
        let mask = mask
            .map(|(mask, data, mask_window)| {
                layer_plane(data, mask.rect, mask_window, header, None)
                    .map(|plane| (mask.outside, mask_window, plane.unwrap_or_default()))
            })
            .transpose()?;
        let opacity = group * f32::from(layer.opacity) / 255. * f32::from(layer.fill) / 255.;
        let at = |plane: &Option<Vec<u8>>, i: usize| plane.as_ref().and_then(|p| p.get(i)).copied();
        let mut coverage = Coverage {
            window,
            values: Vec::with_capacity(window.area()),
        };
        let mut samples = [0; 4];
        for y in window.y0..window.y1 {
            for x in window.x0..window.x1 {
                let i = coverage.values.len();
                let (colour, mut alpha) = match &shape {
                    Some(drawn) => {
                        let (colour, alpha) = drawn.at(i);
                        (colour, alpha * opacity)
                    }
                    None => {
                        for (sample, plane) in samples.iter_mut().zip(&planes) {
                            *sample = at(plane, i).unwrap_or(0);
                        }
                        let alpha =
                            at(&transparency, i).map_or(opacity, |a| f32::from(a) / 255. * opacity);
                        (
                            colour(header.mode, &samples[..colours], self.palette),
                            alpha,
                        )
                    }
                };
                if let Some((outside, mask_window, values)) = &mask {
                    let shown = mask_window
                        .index(x, y)
                        .and_then(|j| values.get(j))
                        .copied()
                        .unwrap_or(*outside);
                    alpha *= f32::from(shown) / 255.;
                }
                coverage.values.push((alpha * 255. + 0.5) as u8);
                if let Some(clip) = clip {
                    alpha *= f32::from(clip.at(x, y)) / 255.;
                }
                over(&mut self.canvas[y * width + x], colour, alpha);
            }
        }
        Ok(coverage)
    }
}

/// One layer channel's samples inside `window`, or `None` when the channel
/// holds no data.
fn layer_plane(
    data: &[u8],
    rect: Rect,
    window: Window,
    header: &Header,
    gamma: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, String> {
    if data.len() < 2 || window.is_empty() {
        return Ok(None);
    }
    let mut r = Reader::new(data);
    let method = r.u16()?;
    let mut rows = Rows::new(method, r.rest(), rect.height() as u64, header.psb)?;
    plane(&mut rows, rect, window, header, gamma).map(Some)
}

/// `colour` at coverage `alpha` (0 to 1) drawn over `below`: the normal blend
/// mode.
fn over(below: &mut Rgba, colour: [u8; 3], alpha: f32) {
    if alpha <= 0. {
        return;
    }
    let alpha = alpha.min(1.);
    let under = f32::from(below.0[3]) / 255. * (1. - alpha);
    let total = alpha + under;
    for (c, value) in colour.into_iter().enumerate() {
        below.0[c] =
            ((f32::from(value) * alpha + f32::from(below.0[c]) * under) / total + 0.5) as u8;
    }
    below.0[3] = (total * 255. + 0.5) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Photoshop document built byte by byte; `image` is its composite
    /// image data (compression method and data).
    struct Psd {
        version: u16,
        channels: u16,
        width: u32,
        height: u32,
        depth: u16,
        mode: u16,
        colour_data: Vec<u8>,
        resources: Vec<u8>,
        layers: Vec<u8>,
        image: Vec<u8>,
    }

    impl Psd {
        fn new(mode: u16, depth: u16, channels: u16, (width, height): (u32, u32)) -> Psd {
            Psd {
                version: 1,
                channels,
                width,
                height,
                depth,
                mode,
                colour_data: Vec::new(),
                resources: Vec::new(),
                layers: Vec::new(),
                image: Vec::new(),
            }
        }

        fn bytes(&self) -> Vec<u8> {
            let mut out = b"8BPS".to_vec();
            out.extend(self.version.to_be_bytes());
            out.extend([0; 6]);
            out.extend(self.channels.to_be_bytes());
            out.extend(self.height.to_be_bytes());
            out.extend(self.width.to_be_bytes());
            out.extend(self.depth.to_be_bytes());
            out.extend(self.mode.to_be_bytes());
            for part in [&self.colour_data, &self.resources] {
                length(&mut out, part.len(), false);
                out.extend(part);
            }
            length(&mut out, self.layers.len(), self.version == 2);
            out.extend(&self.layers);
            out.extend(&self.image);
            out
        }
    }

    fn length(out: &mut Vec<u8>, n: usize, wide: bool) {
        if wide {
            out.extend((n as u64).to_be_bytes());
        } else {
            out.extend((n as u32).to_be_bytes());
        }
    }

    /// Uncompressed image data: the planes one after another.
    fn raw(planes: &[&[u8]]) -> Vec<u8> {
        let mut out = vec![0, 0];
        for plane in planes {
            out.extend(*plane);
        }
        out
    }

    /// One row as PackBits: runs of three or more repeated, the rest literal.
    fn pack(row: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < row.len() {
            let run = row[i..]
                .iter()
                .take(128)
                .take_while(|&&b| b == row[i])
                .count();
            if run >= 3 {
                out.extend([(257 - run) as u8, row[i]]);
                i += run;
                continue;
            }
            let start = i;
            while i < row.len()
                && i - start < 128
                && !(i + 2 < row.len() && row[i] == row[i + 1] && row[i] == row[i + 2])
            {
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend(&row[start..i]);
        }
        out
    }

    /// PackBits image data for `rows`, every channel's in order: the method,
    /// the row lengths, then the rows.
    fn packbits(rows: &[&[u8]], wide: bool) -> Vec<u8> {
        let packed: Vec<Vec<u8>> = rows.iter().map(|row| pack(row)).collect();
        let mut out = vec![0, 1];
        for row in &packed {
            if wide {
                out.extend((row.len() as u32).to_be_bytes());
            } else {
                out.extend((row.len() as u16).to_be_bytes());
            }
        }
        for row in &packed {
            out.extend(row);
        }
        out
    }

    /// `row` delta coded as Photoshop's zlib with prediction stores it.
    fn predicted(row: &[u8], depth: u16) -> Vec<u8> {
        let mut row: Vec<u8> = if depth == 32 {
            let width = row.len() / 4;
            (0..4)
                .flat_map(|b| (0..width).map(move |x| 4 * x + b))
                .map(|i| row[i])
                .collect()
        } else {
            row.to_vec()
        };
        if depth == 16 {
            for i in (1..row.len() / 2).rev() {
                let before = u16::from_be_bytes([row[2 * i - 2], row[2 * i - 1]]);
                let v = u16::from_be_bytes([row[2 * i], row[2 * i + 1]]).wrapping_sub(before);
                row[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes());
            }
        } else {
            for i in (1..row.len()).rev() {
                row[i] = row[i].wrapping_sub(row[i - 1]);
            }
        }
        row
    }

    /// Zlib image data for `rows`, predicted when `depth` is given.
    fn zip(rows: &[&[u8]], depth: Option<u16>) -> Vec<u8> {
        let mut data = Vec::new();
        for row in rows {
            data.extend(depth.map_or(row.to_vec(), |depth| predicted(row, depth)));
        }
        let mut out = vec![0, if depth.is_some() { 3 } else { 2 }];
        out.extend(miniz_oxide::deflate::compress_to_vec_zlib(&data, 6));
        out
    }

    fn resource(id: u16, data: &[u8]) -> Vec<u8> {
        let mut out = b"8BIM".to_vec();
        out.extend(id.to_be_bytes());
        out.extend([0, 0]);
        length(&mut out, data.len(), false);
        out.extend(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// An additional layer information block.
    fn block(key: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = b"8BIM".to_vec();
        out.extend(key);
        length(&mut out, data.len(), false);
        out.extend(data);
        out
    }

    /// A layer and mask section around a layer info (from its count on).
    fn layer_section(info: &[u8], wide: bool) -> Vec<u8> {
        let mut out = Vec::new();
        length(&mut out, info.len(), wide);
        out.extend(info);
        out.extend([0; 4]); // No global layer mask.
        out
    }

    struct TestLayer {
        /// Top, left, bottom, right.
        rect: [i32; 4],
        channels: Vec<(i16, Vec<u8>)>,
        opacity: u8,
        clipping: u8,
        flags: u8,
        mask: Vec<u8>,
        blocks: Vec<u8>,
    }

    fn layer(rect: [i32; 4], channels: Vec<(i16, Vec<u8>)>) -> TestLayer {
        TestLayer {
            rect,
            channels,
            opacity: 255,
            clipping: 0,
            flags: 0,
            mask: Vec::new(),
            blocks: Vec::new(),
        }
    }

    /// A layer of one colour over all of `rect`, uncompressed.
    fn solid(rect: [i32; 4], rgb: [u8; 3]) -> TestLayer {
        let area = ((rect[2] - rect[0]) * (rect[3] - rect[1])) as usize;
        let channels = (0..3)
            .map(|c| (c as i16, raw(&[&vec![rgb[c]; area]])))
            .collect();
        layer(rect, channels)
    }

    /// A group's divider (below its layers) or its own record (above them).
    fn group(section: u32, flags: u8) -> TestLayer {
        let mut record = layer([0; 4], Vec::new());
        record.flags = flags;
        record.blocks = block(b"lsct", &section.to_be_bytes());
        record
    }

    /// A rectangle and the rest of a 20-byte layer mask record.
    fn mask(rect: [i32; 4]) -> Vec<u8> {
        rect.iter()
            .flat_map(|v| v.to_be_bytes())
            .chain([0; 4])
            .collect()
    }

    fn layer_info(layers: &[TestLayer]) -> Vec<u8> {
        let mut out = (layers.len() as i16).to_be_bytes().to_vec();
        for layer in layers {
            for v in layer.rect {
                out.extend(v.to_be_bytes());
            }
            out.extend((layer.channels.len() as u16).to_be_bytes());
            for (id, data) in &layer.channels {
                out.extend(id.to_be_bytes());
                length(&mut out, data.len(), false);
            }
            out.extend(b"8BIMnorm");
            out.extend([layer.opacity, layer.clipping, layer.flags, 0]);
            let mut extra = Vec::new();
            length(&mut extra, layer.mask.len(), false);
            extra.extend(&layer.mask);
            extra.extend([0; 4]); // No blending ranges.
            extra.extend([0; 4]); // An empty name, padded to four bytes.
            extra.extend(&layer.blocks);
            length(&mut out, extra.len(), false);
            out.extend(extra);
        }
        for layer in layers {
            for (_, data) in &layer.channels {
                out.extend(data);
            }
        }
        out
    }

    /// An RGB document of `layers` whose composite is Photoshop's blank white
    /// one ("Maximize compatibility" off).
    fn without_composite((width, height): (u32, u32), layers: &[TestLayer]) -> Vec<u8> {
        let mut psd = Psd::new(3, 8, 3, (width, height));
        psd.resources = resource(1057, &[0, 0, 0, 1, 0]);
        psd.layers = layer_section(&layer_info(layers), false);
        let white = vec![255; (width * height) as usize];
        psd.image = raw(&[&white, &white, &white]);
        psd.bytes()
    }

    fn pixels(raster: &Raster) -> Vec<[u8; 4]> {
        raster.pixels.iter().map(|p| p.0).collect()
    }

    /// Whether every channel is within one of the one wanted.
    fn assert_close(found: &[[u8; 4]], wanted: &[[u8; 4]]) {
        assert_eq!(found.len(), wanted.len());
        for (found, wanted) in found.iter().zip(wanted) {
            let near = found.iter().zip(wanted).all(|(a, b)| a.abs_diff(*b) <= 1);
            assert!(near, "{found:?} is not {wanted:?}");
        }
    }

    #[test]
    fn rgb_reads_alike_raw_packbits_and_zip() {
        let planes: [&[u8]; 3] = [
            &[255, 255, 255, 1, 2, 0, 0, 0, 0, 9],
            &[0, 0, 0, 3, 4, 7, 7, 7, 8, 9],
            &[10, 10, 10, 10, 10, 1, 2, 3, 4, 5],
        ];
        let mut psd = Psd::new(3, 8, 3, (5, 2));
        psd.image = raw(&planes);
        let raster = decode(&psd.bytes(), 100).unwrap();
        assert_eq!((raster.width, raster.height), (5, 2));
        assert_eq!(
            pixels(&raster),
            [
                [255, 0, 10, 255],
                [255, 0, 10, 255],
                [255, 0, 10, 255],
                [1, 3, 10, 255],
                [2, 4, 10, 255],
                [0, 7, 1, 255],
                [0, 7, 2, 255],
                [0, 7, 3, 255],
                [0, 8, 4, 255],
                [9, 9, 5, 255],
            ]
        );
        let rows: Vec<&[u8]> = planes.iter().flat_map(|plane| plane.chunks(5)).collect();
        for image in [
            packbits(&rows, false),
            zip(&rows, None),
            zip(&rows, Some(8)),
        ] {
            psd.image = image;
            assert_eq!(decode(&psd.bytes(), 100).unwrap().pixels, raster.pixels);
        }
        // The hook every picture is opened through.
        let bytes = psd.bytes();
        assert_eq!(
            crate::decode_raster_up_to(&bytes, 100).unwrap().pixels,
            raster.pixels
        );
        assert!(crate::decode_raster_up_to(&bytes, 9)
            .unwrap_err()
            .contains("million pixels"));
    }

    #[test]
    fn composite_transparency_follows_photoshops_rules() {
        let planes: [&[u8]; 4] = [&[1, 2], &[3, 4], &[5, 6], &[0, 255]];
        let alpha = [[1, 3, 5, 0], [2, 4, 6, 255]];
        let opaque = [[1, 3, 5, 255], [2, 4, 6, 255]];
        let count = |count: i16| layer_section(&count.to_be_bytes(), false);
        let mut mtrn = layer_section(&[], false);
        mtrn.extend(block(b"Mtrn", &[]));
        let cases = [
            // A negative layer count: the first extra channel is the
            // transparency.
            (count(-1), Vec::new(), &alpha),
            // Layers without it, or alpha identifiers: saved selections.
            (count(1), Vec::new(), &opaque),
            (Vec::new(), resource(1053, &[0, 0, 0, 7]), &opaque),
            (Vec::new(), Vec::new(), &alpha),
            (mtrn, Vec::new(), &alpha),
        ];
        for (layers, resources, wanted) in cases {
            let mut psd = Psd::new(3, 8, 4, (2, 1));
            psd.layers = layers;
            psd.resources = resources;
            psd.image = raw(&planes);
            assert_eq!(pixels(&decode(&psd.bytes(), 100).unwrap()), wanted);
        }
        // A saved selection first, then the transparency, which the zero
        // alpha identifier marks.
        let mut psd = Psd::new(3, 8, 5, (2, 1));
        psd.resources = resource(1053, &[0, 0, 0, 5, 0, 0, 0, 0]);
        psd.image = raw(&[&[1, 2], &[3, 4], &[5, 6], &[9, 9], &[0, 255]]);
        assert_eq!(pixels(&decode(&psd.bytes(), 100).unwrap()), alpha);
    }

    #[test]
    fn grayscale_16_bit_is_scaled_and_unpredicted() {
        let row: Vec<u8> = [0u16, 65_535, 32_768, 257]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        let mut psd = Psd::new(1, 16, 1, (4, 1));
        let wanted = [0, 255, 128, 1].map(|v| [v, v, v, 255]);
        for image in [
            raw(&[&row]),
            zip(&[&row], Some(16)),
            packbits(&[&row], false),
        ] {
            psd.image = image;
            assert_eq!(pixels(&decode(&psd.bytes(), 100).unwrap()), wanted);
        }
    }

    #[test]
    fn float_32_bit_is_encoded_as_srgb_and_its_alpha_is_not() {
        let floats = [0f32, 1., 0.214, 2., -1., f32::NAN];
        let row: Vec<u8> = floats.iter().flat_map(|v| v.to_be_bytes()).collect();
        let half: Vec<u8> = [0.5f32; 6].iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut psd = Psd::new(1, 32, 2, (6, 1));
        psd.layers = layer_section(&(-1i16).to_be_bytes(), false);
        let wanted = [0, 255, 128, 255, 0, 0].map(|v| [v, v, v, 128]);
        for image in [raw(&[&row, &half]), zip(&[&row, &half], Some(32))] {
            psd.image = image;
            assert_close(&pixels(&decode(&psd.bytes(), 100).unwrap()), &wanted);
        }
    }

    #[test]
    fn cmyk_is_inverted_ink() {
        let mut psd = Psd::new(4, 8, 4, (3, 1));
        psd.image = raw(&[
            &[255, 255, 0],
            &[255, 255, 255],
            &[255, 255, 255],
            &[255, 0, 255],
        ]);
        assert_eq!(
            pixels(&decode(&psd.bytes(), 100).unwrap()),
            [[255, 255, 255, 255], [0, 0, 0, 255], [0, 255, 255, 255]]
        );
    }

    #[test]
    fn indexed_colour_uses_its_table_and_transparent_index() {
        let mut psd = Psd::new(2, 8, 1, (3, 1));
        psd.colour_data = vec![0; 768];
        for (i, [r, g, b]) in [[10, 20, 30], [200, 100, 50]].into_iter().enumerate() {
            psd.colour_data[i] = r;
            psd.colour_data[256 + i] = g;
            psd.colour_data[512 + i] = b;
        }
        psd.resources = resource(1047, &[0, 2]);
        psd.image = raw(&[&[0, 1, 2]]);
        assert_eq!(
            pixels(&decode(&psd.bytes(), 100).unwrap()),
            [[10, 20, 30, 255], [200, 100, 50, 255], [0, 0, 0, 0]]
        );
        psd.colour_data.truncate(767);
        assert!(decode(&psd.bytes(), 100)
            .unwrap_err()
            .contains("colour table"));
    }

    #[test]
    fn bitmap_set_bits_are_black() {
        let mut psd = Psd::new(0, 1, 1, (10, 2));
        psd.image = raw(&[&[0b1010_0000, 0b0100_0000, 0, 0]]);
        let raster = decode(&psd.bytes(), 100).unwrap();
        let found: Vec<u8> = raster.pixels.iter().map(|p| p.0[0]).collect();
        let mut wanted = [255; 20];
        for x in [0, 2, 9] {
            wanted[x] = 0;
        }
        assert_eq!(found, wanted);
    }

    #[test]
    fn lab_is_converted_to_srgb() {
        let mut psd = Psd::new(9, 8, 3, (4, 1));
        // White, black, mid grey and sRGB's red (Lab 54.29 80.80 69.89 under
        // D50).
        psd.image = raw(&[
            &[255, 0, 128, 138],
            &[128, 128, 128, 209],
            &[128, 128, 128, 198],
        ]);
        let found = pixels(&decode(&psd.bytes(), 100).unwrap());
        assert_close(&found[..2], &[[255, 255, 255, 255], [0, 0, 0, 255]]);
        let [r, g, b, _] = found[2];
        assert!((118..=121).contains(&r) && r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1);
        let [r, g, b, _] = found[3];
        assert!(r >= 240 && g <= 40 && b <= 40, "{:?}", found[3]);
    }

    #[test]
    fn large_documents_use_wide_lengths() {
        let mut psd = Psd::new(3, 8, 4, (2, 2));
        psd.version = 2;
        psd.layers = layer_section(&(-1i16).to_be_bytes(), true);
        let rows: [&[u8]; 8] = [
            &[1, 1],
            &[2, 2],
            &[3, 3],
            &[4, 4],
            &[5, 5],
            &[6, 6],
            &[255, 0],
            &[0, 255],
        ];
        psd.image = packbits(&rows, true);
        assert_eq!(
            pixels(&decode(&psd.bytes(), 100).unwrap()),
            [[1, 3, 5, 255], [1, 3, 5, 0], [2, 4, 6, 0], [2, 4, 6, 255]]
        );
        // Wider than a PSD may be.
        let mut psd = Psd::new(1, 8, 1, (30_001, 1));
        psd.image = raw(&[&vec![7; 30_001]]);
        assert!(decode(&psd.bytes(), 100_000).is_err());
        psd.version = 2;
        assert_eq!(decode(&psd.bytes(), 100_000).unwrap().width, 30_001);
    }

    #[test]
    fn layers_are_put_together_without_a_composite() {
        let full = [0, 0, 3, 4];
        let mut hidden = solid(full, [0, 255, 0]);
        hidden.flags = 2;
        // Blue over the lower right at half its fill, past the right edge.
        let mut blue = solid([1, 1, 3, 5], [0, 0, 255]);
        blue.channels.push((-1, raw(&[&[255; 8]])));
        blue.blocks = block(b"iOpa", &[128, 0, 0, 0]);
        // White clipped to it.
        let mut clipped = solid(full, [255, 255, 255]);
        clipped.clipping = 1;
        // Green through a mask showing only the top left pixel.
        let mut masked = solid(full, [0, 255, 0]);
        masked.mask = mask([0, 0, 1, 2]);
        masked.channels.push((-2, raw(&[&[255, 0]])));
        let layers = [
            solid(full, [255, 0, 0]),
            hidden,
            // A hidden group holding black.
            group(3, 0),
            solid(full, [0, 0, 0]),
            group(1, 2),
            blue,
            clipped,
            masked,
        ];
        let mut bytes = without_composite((4, 3), &layers);
        let (red, green, mixed) = ([255, 0, 0, 255], [0, 255, 0, 255], [191, 128, 192, 255]);
        #[rustfmt::skip]
        let wanted = [
            green, red, red, red,
            red, mixed, mixed, mixed,
            red, mixed, mixed, mixed,
        ];
        assert_close(&pixels(&decode(&bytes, 100).unwrap()), &wanted);
        // With a real composite, the composite is the picture: the version
        // info's data starts at byte 46 and says so in its fifth byte.
        assert_eq!(bytes[46..51], [0, 0, 0, 1, 0]);
        bytes[50] = 1;
        let composite = pixels(&decode(&bytes, 100).unwrap());
        assert!(composite.iter().all(|p| *p == [255; 4]));
    }

    #[test]
    fn layer_channels_decode_every_compression() {
        // Two rows of three, the first column left of the document.
        let shape = layer(
            [1, -1, 3, 2],
            vec![
                (-1, zip(&[&[255, 255, 0], &[255, 0, 255]], Some(8))),
                (0, raw(&[&[10, 20, 30, 40, 50, 60]])),
                (1, packbits(&[&[1, 1, 1], &[2, 3, 4]], false)),
                (2, zip(&[&[100, 110, 120], &[130, 140, 150]], None)),
            ],
        );
        let raster = decode(&without_composite((2, 3), &[shape]), 100).unwrap();
        let clear = [0; 4];
        assert_eq!(
            pixels(&raster),
            [
                clear,
                clear,
                [20, 1, 110, 255],
                clear,
                clear,
                [60, 4, 150, 255]
            ]
        );
    }

    #[test]
    fn hostile_headers_are_refused() {
        let refused = |psd: Psd, words: &str| {
            let error = decode(&psd.bytes(), 1_000_000).unwrap_err();
            assert!(error.contains(words), "{error}");
        };
        let mut psd = Psd::new(3, 8, 3, (1, 1));
        psd.version = 3;
        refused(psd, "version 3");
        refused(Psd::new(3, 8, 0, (1, 1)), "56 channels");
        refused(Psd::new(3, 8, 57, (1, 1)), "56 channels");
        refused(Psd::new(3, 8, 3, (0, 1)), "wide and high");
        refused(Psd::new(3, 8, 3, (1, 30_001)), "wide and high");
        refused(Psd::new(3, 7, 3, (1, 1)), "bits");
        refused(Psd::new(0, 8, 1, (1, 1)), "bits");
        refused(Psd::new(2, 16, 1, (1, 1)), "bits");
        refused(Psd::new(5, 8, 3, (1, 1)), "colour mode 5");
        refused(Psd::new(3, 8, 2, (1, 1)), "too few channels");
        refused(Psd::new(3, 8, 3, (30_000, 30_000)), "million pixels");
        assert!(decode(b"GIF89a", 100)
            .unwrap_err()
            .contains("not a Photoshop"));
        assert!(decode(b"8BP", 100).is_err());
    }

    #[test]
    fn hostile_lengths_are_refused() {
        let mut psd = Psd::new(3, 8, 3, (2, 1));
        psd.image = raw(&[&[1, 2], &[3, 4], &[5, 6]]);
        let good = psd.bytes();
        // Resources, then the layer section, claiming more than there is.
        for at in [30, 34] {
            let mut bytes = good.clone();
            bytes[at..at + 4].copy_from_slice(&[255; 4]);
            assert!(decode(&bytes, 100).is_err());
        }
        psd.version = 2;
        let mut bytes = psd.bytes();
        bytes[34..42].copy_from_slice(&[255; 8]);
        assert!(decode(&bytes, 100).is_err());
        psd.version = 1;
        // Row lengths past the data, and an unknown compression method.
        psd.image = vec![0, 1, 0, 200, 0, 200, 0, 200, 0, 1, 2];
        assert!(decode(&psd.bytes(), 100).is_err());
        psd.image = vec![0, 9, 1, 2, 3, 4, 5, 6];
        assert!(decode(&psd.bytes(), 100).unwrap_err().contains("method 9"));
        // A layer far larger than the pixel limit, and layers holding more
        // than sixteen times it together.
        let huge = layer([0, 0, 20_000, 20_000], vec![(0, raw(&[&[1]]))]);
        let error = decode(&without_composite((2, 2), &[huge]), 1_000_000).unwrap_err();
        assert!(error.contains("larger than"), "{error}");
        let row = pack(&[9; 100]);
        let mut channel = vec![0, 1];
        for _ in 0..100 {
            channel.extend((row.len() as u16).to_be_bytes());
        }
        for _ in 0..100 {
            channel.extend(&row);
        }
        let many: Vec<TestLayer> = (0..17)
            .map(|_| {
                layer(
                    [0, 0, 100, 100],
                    (0..3).map(|c| (c, channel.clone())).collect(),
                )
            })
            .collect();
        let bytes = without_composite((100, 100), &many);
        let error = decode(&bytes, 10_000).unwrap_err();
        assert!(error.contains("too many pixels"), "{error}");
        assert_eq!(
            decode(&bytes, 20_000).unwrap().pixels[0],
            Rgba([9, 9, 9, 255])
        );
        // A layer count with no records behind it.
        let mut psd = Psd::new(3, 8, 3, (1, 1));
        psd.resources = resource(1057, &[0, 0, 0, 1, 0]);
        psd.layers = layer_section(&32_767i16.to_be_bytes(), false);
        psd.image = raw(&[&[1], &[2], &[3]]);
        assert!(decode(&psd.bytes(), 100).is_err());
    }

    #[test]
    fn cut_short_files_are_refused() {
        let mut psd = Psd::new(3, 8, 4, (3, 2));
        psd.resources = resource(1005, &[1, 2, 3]);
        let rows: Vec<&[u8]> = vec![&[1, 1, 1]; 8];
        for version in [1, 2] {
            psd.version = version;
            psd.layers = layer_section(&(-1i16).to_be_bytes(), version == 2);
            psd.image = packbits(&rows, version == 2);
            let bytes = psd.bytes();
            assert!(decode(&bytes, 100).is_ok());
            for end in 0..bytes.len() {
                assert!(decode(&bytes[..end], 100).is_err(), "{end} bytes");
            }
        }
    }

    #[test]
    fn damaged_bytes_never_panic() {
        let mut psd = Psd::new(3, 8, 4, (3, 2));
        psd.layers = layer_section(&(-1i16).to_be_bytes(), false);
        psd.image = packbits(&[&[1, 1, 1][..]; 8], false);
        let mut shape = solid([0, 0, 2, 3], [1, 2, 3]);
        shape
            .channels
            .push((-1, zip(&[&[255, 0, 255], &[0, 255, 0]], Some(8))));
        shape.mask = mask([0, 0, 1, 1]);
        shape.channels.push((-2, raw(&[&[255]])));
        let layered = without_composite((3, 2), &[shape, group(3, 0), group(1, 0)]);
        for bytes in [psd.bytes(), layered] {
            for at in 0..bytes.len() {
                for value in [0, 1, 0x7F, 0x80, 0xFE, 0xFF] {
                    let mut damaged = bytes.clone();
                    damaged[at] = value;
                    let _ = decode(&damaged, 1 << 16);
                }
            }
        }
    }

    /// VectorMojo's demo document (ag-psd wrote it): eight vector shape
    /// layers whose pixels are transparent, over a composite of one navy
    /// that leaves them out, and no version info.
    #[test]
    fn the_vector_mojo_sample_opens() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/samples/vector-mojo-sample.psd");
        let raster = crate::load_raster_up_to(&path, 16_000_000).unwrap();
        assert_eq!((raster.width, raster.height), (1200, 800));
        assert!(raster.pixels.iter().any(|p| p.0 != [15, 23, 42, 255]));
        // Its layers parse, and put together from their vector art they are
        // the picture.
        let bytes = std::fs::read(&path).unwrap();
        let mut file = Reader::new(&bytes);
        let header = Header::read(&mut file).unwrap();
        file.section(false).unwrap();
        let resources = Resources::read(file.section(false).unwrap());
        assert!(resources.merged && !resources.versioned);
        let section = LayerSection::read(file.section(header.psb).unwrap(), header.psb);
        let layers = read_layers(section.info.unwrap(), header.psb).unwrap();
        assert_eq!(layers.len(), 8);
        let first = layers[0].rect;
        assert_eq!(
            (first.top, first.left, first.bottom, first.right),
            (40, 40, 760, 1160)
        );
        let drawn = Compositor::new(&header, &[], 16_000_000, None)
            .run(&layers)
            .unwrap();
        assert_eq!(drawn.pixels, raster.pixels);
    }

    mod shapes;
}
