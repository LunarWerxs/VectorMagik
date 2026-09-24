//! Part of `pdf_import`: the picture a page holds, for a PDF with no shapes
//! to keep. A scanned or exported logo often reaches the app as a PDF (or an
//! AI) that is one embedded image: converting it keeps nothing, but its
//! picture can be traced like any other. This finds the page's largest image
//! (found as the page is drawn, forms included) and decodes it: JPEG as
//! stored (DCTDecode), or plain samples after the stream's filters, in
//! gray, RGB, CMYK (converted naively), ICC-based (by its component count)
//! or indexed colour, 1 or 8 bits a component, with its soft mask as
//! transparency. JPEG 2000, CCITT and JBIG2 images are not read.

use super::document::Document;
use super::syntax::{Obj, Stream};
use vector_rebuild::raster::{Raster, Rgba};

/// The most pixels a picture taken from a PDF may have.
const MAX_PIXELS: u64 = 50_000_000;

/// An image drawn on the page, and its size in pixels.
pub(super) struct Found {
    pub stream: std::rc::Rc<Stream>,
    pub area: u64,
}

/// The picture `found` holds, or why not.
pub(super) fn decode(doc: &Document, found: &Found) -> Result<Raster, String> {
    let dict = &found.stream.dict;
    let int = |key: &[u8]| {
        dict.get(key)
            .map(|v| doc.resolve(v))
            .and_then(|v| v.int())
            .unwrap_or(0)
    };
    let (width, height) = (int(b"Width"), int(b"Height"));
    if width <= 0 || height <= 0 || (width as u64) * (height as u64) > MAX_PIXELS {
        return Err("its picture has no size the app reads".into());
    }
    let (width, height) = (width as usize, height as usize);
    let filters = match dict.get(b"Filter").map(|f| doc.resolve(f)) {
        Some(Obj::Array(items)) => items
            .iter()
            .filter_map(|i| doc.resolve(i).name().map(<[u8]>::to_vec))
            .collect(),
        Some(Obj::Name(name)) => vec![name.to_vec()],
        _ => Vec::new(),
    };
    let mut pixels = if filters
        .last()
        .is_some_and(|f| f == b"DCTDecode" || f == b"DCT")
    {
        if filters.len() > 1 {
            return Err("its picture is a JPEG wrapped in another filter".into());
        }
        let image =
            image::load_from_memory_with_format(&found.stream.raw, image::ImageFormat::Jpeg)
                .map_err(|e| format!("its JPEG picture could not be read: {e}"))?
                .to_rgba8();
        if (image.width() as usize, image.height() as usize) != (width, height) {
            return Err("its JPEG picture is not the size the page says".into());
        }
        image.pixels().map(|p| Rgba(p.0)).collect::<Vec<_>>()
    } else {
        let data = doc
            .decode(&found.stream)
            .map_err(|e| format!("its picture could not be read, because {e}"))?;
        samples(doc, dict, &data, width, height)?
    };
    // The soft mask, a gray image of the same size, is the transparency.
    if let Some(Obj::Stream(mask)) = dict.get(b"SMask").map(|m| doc.resolve(m)) {
        if let Ok(alpha) = doc.decode(&mask) {
            if alpha.len() >= width * height {
                for (pixel, a) in pixels.iter_mut().zip(alpha) {
                    pixel.0[3] = a;
                }
            }
        }
    }
    Ok(Raster {
        width,
        height,
        pixels,
    })
}

/// A colour space's shape: how many components a sample has and how one
/// becomes RGB.
enum Space {
    Gray,
    Rgb,
    Cmyk,
    /// A palette of RGB colours, indexed by one component.
    Indexed(Vec<[u8; 3]>),
}

fn space(doc: &Document, value: Option<&Obj>) -> Result<Space, String> {
    let value = value.map(|v| doc.resolve(v)).unwrap_or(Obj::Null);
    let by_name = |name: &[u8]| match name {
        b"DeviceGray" | b"G" | b"CalGray" => Some(Space::Gray),
        b"DeviceRGB" | b"RGB" | b"CalRGB" => Some(Space::Rgb),
        b"DeviceCMYK" | b"CMYK" => Some(Space::Cmyk),
        _ => None,
    };
    if let Some(name) = value.name() {
        return by_name(name).ok_or_else(|| "its picture's colour space is not read".into());
    }
    let items = value.array().ok_or("its picture has no colour space")?;
    let head = items.first().and_then(Obj::name).unwrap_or_default();
    match head {
        b"ICCBased" => {
            let n = match items.get(1).map(|v| doc.resolve(v)) {
                Some(Obj::Stream(s)) => s.dict.get(b"N").and_then(Obj::int).unwrap_or(3),
                _ => 3,
            };
            Ok(match n {
                1 => Space::Gray,
                4 => Space::Cmyk,
                _ => Space::Rgb,
            })
        }
        b"Indexed" | b"I" => {
            let base = space(doc, items.get(1))?;
            let high = items
                .get(2)
                .map(|v| doc.resolve(v))
                .and_then(|v| v.int())
                .unwrap_or(255)
                .clamp(0, 255) as usize;
            let lookup = match items.get(3).map(|v| doc.resolve(v)) {
                Some(Obj::Str(bytes)) => bytes.to_vec(),
                Some(Obj::Stream(s)) => doc.decode(&s).unwrap_or_default(),
                _ => Vec::new(),
            };
            let per = match base {
                Space::Gray => 1,
                Space::Rgb => 3,
                Space::Cmyk => 4,
                Space::Indexed(_) => return Err("its picture's palette is itself indexed".into()),
            };
            let palette = (0..=high)
                .map(|i| {
                    let entry = lookup
                        .get(i * per..i * per + per)
                        .unwrap_or(&[0, 0, 0, 0][..per]);
                    to_rgb(&base, entry)
                })
                .collect();
            Ok(Space::Indexed(palette))
        }
        other => by_name(other).ok_or_else(|| "its picture's colour space is not read".into()),
    }
}

fn to_rgb(space: &Space, c: &[u8]) -> [u8; 3] {
    match space {
        Space::Gray => [c[0], c[0], c[0]],
        Space::Rgb => [c[0], c[1], c[2]],
        Space::Cmyk => {
            let k = 255 - u16::from(c[3]);
            let channel = |v: u8| ((255 - u16::from(v)) * k / 255) as u8;
            [channel(c[0]), channel(c[1]), channel(c[2])]
        }
        Space::Indexed(palette) => palette.get(usize::from(c[0])).copied().unwrap_or([0, 0, 0]),
    }
}

/// Plain samples, row by row, as RGBA.
fn samples(
    doc: &Document,
    dict: &super::syntax::Dict,
    data: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<Rgba>, String> {
    let mask = dict
        .get(b"ImageMask")
        .map(|v| doc.resolve(v))
        .is_some_and(|v| matches!(v, Obj::Bool(true)));
    let bits = dict
        .get(b"BitsPerComponent")
        .map(|v| doc.resolve(v))
        .and_then(|v| v.int())
        .unwrap_or(if mask { 1 } else { 8 });
    let space = if mask {
        Space::Gray
    } else {
        space(doc, dict.get(b"ColorSpace"))?
    };
    let components = match space {
        Space::Gray | Space::Indexed(_) => 1,
        Space::Rgb => 3,
        Space::Cmyk => 4,
    };
    if bits != 8 && bits != 1 {
        return Err(format!(
            "its picture has {bits} bits a component, which the app does not read"
        ));
    }
    if bits == 1 && components != 1 {
        return Err("its picture has 1-bit colour, which the app does not read".into());
    }
    let row = (width * components * bits as usize).div_ceil(8);
    if data.len() < row * height {
        return Err("its picture's data is shorter than its size".into());
    }
    let mut pixels = Vec::with_capacity(width * height);
    let mut sample = [0u8; 4];
    for y in 0..height {
        let line = &data[y * row..y * row + row];
        for x in 0..width {
            if bits == 1 {
                let on = line[x / 8] >> (7 - x % 8) & 1 == 1;
                // A 1-bit gray image is white where the bit is 1; an image
                // mask paints its colour where the bit is 0, drawn black here.
                sample[0] = if on { 255 } else { 0 };
            } else {
                sample[..components]
                    .copy_from_slice(&line[x * components..x * components + components]);
            }
            let [r, g, b] = to_rgb(&space, &sample[..components.max(1)]);
            pixels.push(Rgba([r, g, b, 255]));
        }
    }
    Ok(pixels)
}
