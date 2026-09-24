//! The stream filters that carry drawing: Flate (with the PNG and TIFF
//! predictors), LZW, ASCIIHex, ASCII85 and RunLength. Each decoder stops at
//! `limit` output bytes, so a small file cannot expand without end. The
//! image filters (DCT, JPX, CCITT, JBIG2) are refused: only images use them,
//! and images are left out.
use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
use miniz_oxide::inflate::TINFLStatus;

/// What a filter's `/DecodeParms` say, with the defaults the standard gives.
pub(super) struct Params {
    pub(super) predictor: i64,
    pub(super) colors: i64,
    pub(super) bits: i64,
    pub(super) columns: i64,
    pub(super) early_change: i64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            predictor: 1,
            colors: 1,
            bits: 8,
            columns: 1,
            early_change: 1,
        }
    }
}

pub(super) const TOO_LARGE: &str = "it expands to more data than the app reads";

/// `data` through the filter named `filter`, at most `limit` bytes out.
pub(super) fn apply(
    filter: &[u8],
    data: &[u8],
    params: &Params,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let out = match filter {
        b"FlateDecode" | b"Fl" => predict(inflate(data, limit)?, params)?,
        b"LZWDecode" | b"LZW" => predict(lzw(data, params.early_change != 0, limit)?, params)?,
        b"ASCIIHexDecode" | b"AHx" => ascii_hex(data),
        b"ASCII85Decode" | b"A85" => ascii85(data, limit)?,
        b"RunLengthDecode" | b"RL" => run_length(data, limit)?,
        b"DCTDecode" | b"DCT" | b"JPXDecode" | b"CCITTFaxDecode" | b"CCF" | b"JBIG2Decode" => {
            return Err("it is an image".into())
        }
        other => {
            return Err(format!(
                "it uses the {} filter",
                String::from_utf8_lossy(other)
            ))
        }
    };
    if out.len() > limit {
        return Err(TOO_LARGE.into());
    }
    Ok(out)
}

/// Deflated data, zlib-wrapped or raw. A stream cut short or damaged near
/// its end gives what came out before the damage, as viewers show it.
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    match inflate_with(data, inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER, limit)? {
        (out, _) if !out.is_empty() => Ok(out),
        (_, true) => Ok(Vec::new()),
        _ => match inflate_with(data, 0, limit)? {
            (out, true) => Ok(out),
            (out, false) if !out.is_empty() => Ok(out),
            _ => Err("its compressed data is damaged".into()),
        },
    }
}

/// The output and whether the data ended cleanly.
fn inflate_with(data: &[u8], flags: u32, limit: usize) -> Result<(Vec<u8>, bool), String> {
    let flags = flags
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | inflate_flags::TINFL_FLAG_IGNORE_ADLER32;
    let mut out = vec![0; data.len().saturating_mul(4).clamp(64, limit.max(64))];
    let mut state = Box::<DecompressorOxide>::default();
    let (mut input, mut at) = (data, 0);
    loop {
        let (status, used, written) = decompress(&mut state, input, &mut out, at, flags);
        at += written;
        input = &input[used.min(input.len())..];
        match status {
            TINFLStatus::Done => {
                out.truncate(at);
                return Ok((out, true));
            }
            TINFLStatus::HasMoreOutput => {
                if out.len() >= limit {
                    return Err(TOO_LARGE.into());
                }
                let grown = out.len().saturating_mul(2).min(limit);
                out.resize(grown, 0);
            }
            _ => {
                out.truncate(at);
                return Ok((out, false));
            }
        }
    }
}

/// The PNG (10 to 15) or TIFF (2) predictor undone, row by row.
fn predict(mut data: Vec<u8>, params: &Params) -> Result<Vec<u8>, String> {
    if params.predictor < 2 {
        return Ok(data);
    }
    let bad = || "its predictor settings are out of range".to_owned();
    if !(1..=32).contains(&params.colors)
        || ![1, 2, 4, 8, 16].contains(&params.bits)
        || !(1..=1 << 20).contains(&params.columns)
    {
        return Err(bad());
    }
    let (colors, bits, columns) = (
        params.colors as usize,
        params.bits as usize,
        params.columns as usize,
    );
    let row = (colors * bits * columns).div_ceil(8);
    let pixel = (colors * bits).div_ceil(8);
    if params.predictor == 2 {
        for line in data.chunks_mut(row) {
            tiff_row(line, colors, bits);
        }
        return Ok(data);
    }
    if params.predictor < 10 {
        return Err(bad());
    }
    // No row is longer than the data, whatever the parameters claim.
    let width = row.min(data.len());
    let mut out = Vec::with_capacity(data.len());
    let mut previous = vec![0u8; width];
    for line in data.chunks(row + 1) {
        let (kind, line) = (line[0], &line[1..]);
        let mut current = vec![0u8; width];
        for i in 0..line.len() {
            let left = if i >= pixel { current[i - pixel] } else { 0 };
            let up = previous[i];
            let corner = if i >= pixel { previous[i - pixel] } else { 0 };
            let guess = match kind {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, corner),
                _ => return Err("its PNG predictor is damaged".into()),
            };
            current[i] = line[i].wrapping_add(guess);
        }
        out.extend_from_slice(&current[..line.len()]);
        previous = current;
    }
    Ok(out)
}

fn paeth(left: u8, up: u8, corner: u8) -> u8 {
    let p = i16::from(left) + i16::from(up) - i16::from(corner);
    let (pa, pb, pc) = (
        (p - i16::from(left)).abs(),
        (p - i16::from(up)).abs(),
        (p - i16::from(corner)).abs(),
    );
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        up
    } else {
        corner
    }
}

/// One row of TIFF predictor 2: every sample added to the one a pixel to
/// its left.
fn tiff_row(line: &mut [u8], colors: usize, bits: usize) {
    match bits {
        8 => {
            for i in colors..line.len() {
                line[i] = line[i].wrapping_add(line[i - colors]);
            }
        }
        16 => {
            for i in (2 * colors..line.len().saturating_sub(1)).step_by(2) {
                let left = u16::from_be_bytes([line[i - 2 * colors], line[i - 2 * colors + 1]]);
                let own = u16::from_be_bytes([line[i], line[i + 1]]);
                line[i..i + 2].copy_from_slice(&own.wrapping_add(left).to_be_bytes());
            }
        }
        _ => {
            let samples = line.len() * 8 / bits;
            let mask = (1u16 << bits) - 1;
            let read = |line: &[u8], s: usize| {
                let bit = s * bits;
                (u16::from(line[bit / 8]) >> (8 - bits - bit % 8)) & mask
            };
            for s in colors..samples {
                let value = (read(line, s) + read(line, s - colors)) & mask;
                let bit = s * bits;
                let shift = 8 - bits - bit % 8;
                let byte = &mut line[bit / 8];
                *byte = (*byte & !((mask as u8) << shift)) | ((value as u8) << shift);
            }
        }
    }
}

/// LZW as PDF writes it: 9 to 12 bit codes, most significant bit first,
/// 256 clears the table and 257 ends; with `early` the code widens one code
/// sooner, the default.
fn lzw(data: &[u8], early: bool, limit: usize) -> Result<Vec<u8>, String> {
    // Each entry: the code it extends, its last byte, its first byte.
    let mut table: Vec<(u16, u8, u8)> = Vec::with_capacity(4096);
    let reset = |table: &mut Vec<(u16, u8, u8)>| {
        table.clear();
        table.extend((0..=255u8).map(|b| (u16::MAX, b, b)));
        table.extend([(u16::MAX, 0, 0), (u16::MAX, 0, 0)]);
    };
    reset(&mut table);
    let mut out = Vec::new();
    let mut scratch = Vec::new();
    let (mut width, mut bit) = (9usize, 0usize);
    let mut previous: Option<u16> = None;
    while bit + width <= data.len() * 8 {
        let mut code = 0usize;
        for i in bit..bit + width {
            code = code << 1 | usize::from(data[i / 8] >> (7 - i % 8) & 1);
        }
        bit += width;
        match code {
            256 => {
                reset(&mut table);
                width = 9;
                previous = None;
                continue;
            }
            257 => break,
            _ => {}
        }
        let first = match (code < table.len(), previous) {
            (true, _) => table[code].2,
            (false, Some(p)) if code == table.len() => table[usize::from(p)].2,
            _ => return Err("its LZW data is damaged".into()),
        };
        if let Some(p) = previous.filter(|_| table.len() < 4096) {
            let head = table[usize::from(p)].2;
            table.push((p, first, head));
        }
        // The code's bytes, collected from its end back to its root.
        scratch.clear();
        let mut at = code;
        while at != usize::from(u16::MAX) && scratch.len() <= 4096 {
            let (prefix, byte, _) = table[at];
            scratch.push(byte);
            at = usize::from(prefix);
        }
        out.extend(scratch.iter().rev());
        if out.len() > limit {
            return Err(TOO_LARGE.into());
        }
        previous = Some(code as u16);
        if table.len() + usize::from(early) >= 1 << width && width < 12 {
            width += 1;
        }
    }
    Ok(out)
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        let Some(v) = super::syntax::hex_value(b) else {
            continue;
        };
        match high.take() {
            Some(h) => out.push(h * 16 + v),
            None => high = Some(v),
        }
    }
    if let Some(h) = high {
        out.push(h * 16);
    }
    out
}

fn ascii85(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let data = data.strip_prefix(b"<~").unwrap_or(data);
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u8; 5];
    let mut n = 0;
    let flush = |group: &[u8; 5], n: usize, out: &mut Vec<u8>| -> Result<(), String> {
        let mut value: u64 = 0;
        for (i, &c) in group.iter().enumerate() {
            value = value * 85 + u64::from(if i < n { c } else { 84 });
        }
        let value = u32::try_from(value).map_err(|_| "its ASCII85 data is damaged")?;
        out.extend_from_slice(&value.to_be_bytes()[..n - 1]);
        Ok(())
    };
    for &b in data {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = b - b'!';
                n += 1;
                if n == 5 {
                    flush(&group, 5, &mut out)?;
                    n = 0;
                }
            }
            b if super::syntax::is_white(b) => {}
            _ => return Err("its ASCII85 data is damaged".into()),
        }
        if out.len() > limit {
            return Err(TOO_LARGE.into());
        }
    }
    if n > 1 {
        flush(&group, n, &mut out)?;
    }
    Ok(out)
}

fn run_length(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&length) = data.get(i) {
        match length {
            128 => break,
            0..=127 => {
                let run = &data[i + 1..(i + 2 + usize::from(length)).min(data.len())];
                out.extend_from_slice(run);
                i += 2 + usize::from(length);
            }
            _ => {
                let Some(&b) = data.get(i + 1) else { break };
                out.extend(std::iter::repeat_n(b, 257 - usize::from(length)));
                i += 2;
            }
        }
        if out.len() > limit {
            return Err(TOO_LARGE.into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standards_lzw_example_decodes() {
        let data = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&data, true, 1 << 20).unwrap(), b"-----A---B");
    }

    #[test]
    fn the_text_filters_decode() {
        assert_eq!(ascii_hex(b"48 65 6C6c6F>"), b"Hello");
        assert_eq!(ascii_hex(b"7>"), [0x70]);
        assert_eq!(
            ascii85(b"<~87cURD]i,\"Ebo80~>", 1 << 20).unwrap(),
            b"Hello World!"
        );
        // A last group of three characters is two bytes.
        assert_eq!(ascii85(b"88/~>", 1 << 20).unwrap(), b"Hi");
        assert_eq!(ascii85(b"z~>", 1 << 20).unwrap(), [0; 4]);
        let rle = [2, b'a', b'b', b'c', 254, b'x', 128];
        assert_eq!(run_length(&rle, 1 << 20).unwrap(), b"abcxxx");
        assert!(run_length(&[129, 0, 129, 0], 100).is_err());
    }

    #[test]
    fn the_tiff_predictor_adds_the_pixel_to_the_left() {
        let params = Params {
            predictor: 2,
            colors: 1,
            bits: 8,
            columns: 4,
            ..Params::default()
        };
        assert_eq!(predict(vec![1, 1, 1, 1], &params).unwrap(), [1, 2, 3, 4]);
        let params = Params { bits: 4, ..params };
        assert_eq!(predict(vec![0x11, 0x11], &params).unwrap(), [0x12, 0x34]);
    }

    #[test]
    fn a_deflate_bomb_stops_at_the_limit() {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0; 1 << 20], 6);
        assert_eq!(inflate(&packed, 1 << 21).unwrap().len(), 1 << 20);
        assert_eq!(inflate(&packed, 1000).unwrap_err(), TOO_LARGE);
        // Cut short, it gives what came before the cut.
        let text = miniz_oxide::deflate::compress_to_vec_zlib(&[b'a'; 5000], 0);
        let cut = inflate(&text[..3000], 1 << 20).unwrap();
        assert!(cut.len() > 2000 && cut.iter().all(|b| *b == b'a'));
    }
}
