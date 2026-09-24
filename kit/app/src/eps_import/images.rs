//! Images and text, which the import leaves out: an image's data is read
//! past from whatever source feeds it (so the program goes on after it) and
//! the image counted; text moves the current point on and is noted. Fonts
//! behave enough for the prologs that define, re-encode and scale them.
use super::graphics::{apply_delta, matrix_of, multiply, pop_drop, Pt};
use super::object::{Dict, Obj};
use super::ops::{fail, Entry};
use super::stream;
use super::{Fault, Machine, Res};

impl Machine {
    /// A font by name: the one defined, or a stand-in (text is not drawn,
    /// so only its metrics' shape matters).
    pub(super) fn font_named(&mut self, key: &Obj) -> Res<Dict> {
        if let Some(Obj::Dict(font)) = self.fonts.get(key) {
            return Ok(font);
        }
        let font = Dict::new(12);
        let name = key
            .text()
            .map_or(Obj::name("Courier"), |t| Obj::Name(t.into(), false));
        font.set("FontName", name);
        font.set("FontType", Obj::Int(1));
        font.set("FontMatrix", Obj::numbers(&[0.001, 0., 0., 0.001, 0., 0.]));
        font.set("FontBBox", Obj::numbers(&[-200., -250., 1000., 1000.]));
        font.set("PaintType", Obj::Int(0));
        font.set(
            "Encoding",
            self.systemdict
                .find(b"StandardEncoding")
                .unwrap_or(Obj::Null),
        );
        font.set("CharStrings", Obj::Dict(Dict::new(1)));
        font.set("FID", Obj::FontId);
        self.fonts
            .put(key.clone(), Obj::Dict(font.clone()))
            .map_err(Fault::Error)?;
        Ok(font)
    }

    /// Text is not drawn: it is noted, and the current point moves on as
    /// if each character were 0.6 em wide.
    fn shown(&mut self, characters: usize) -> Res {
        self.art.text = true;
        if self.gs.point.is_some() {
            let (dx, dy) = apply_delta(self.gs.ctm, self.text_width(characters));
            let (x, y) = self.gs.point.unwrap_or_default();
            self.move_to((x + dx, y + dy))?;
        }
        Ok(())
    }

    fn text_width(&self, characters: usize) -> Pt {
        let matrix = match &self.gs.font {
            Obj::Dict(font) => font.find(b"FontMatrix").and_then(|m| matrix_of(&m)),
            _ => None,
        }
        .unwrap_or([0.001, 0., 0., 0.001, 0., 0.]);
        let width = 600. * characters as f64;
        (matrix[0] * width, matrix[1] * width)
    }

    /// Reads past an image's data from each source (a file, a procedure
    /// called until it has given enough, or a string, which is reused) and
    /// counts the image.
    fn consume(&mut self, sources: Vec<Obj>, needed: u64) -> Res {
        self.art.images += 1;
        let mut got = vec![0u64; sources.len()];
        for (index, source) in sources.iter().enumerate() {
            match source {
                Obj::File(file, _) => {
                    stream::skip(file, needed).map_err(Fault::Error)?;
                    stream::settle(file).map_err(Fault::Error)?;
                    got[index] = needed;
                }
                Obj::Str(_) => got[index] = needed,
                Obj::Array(_) | Obj::Name(..) | Obj::Op(_) => {}
                _ => return fail("typecheck"),
            }
        }
        loop {
            let mut called = false;
            for (index, source) in sources.iter().enumerate() {
                if got[index] >= needed {
                    continue;
                }
                self.step()?;
                self.exec(source.clone())?;
                called = true;
                match self.pop()? {
                    Obj::Str(data) if data.len > 0 => got[index] += data.len as u64,
                    Obj::Str(_) => got[index] = needed,
                    _ => return fail("typecheck"),
                }
            }
            if !called {
                return Ok(());
            }
        }
    }

    fn image_data(
        &mut self,
        width: i32,
        height: i32,
        bits: i32,
        comps: usize,
        sources: Vec<Obj>,
    ) -> Res {
        self.image_data_at(width, height, bits, comps, sources, None)
    }

    /// `image_data`, with the image's matrix when the operator gave one: an
    /// 8-bit gray, RGB or CMYK image from one source is kept as the picture
    /// when asked for (`Art::keep_picture`) and larger than any before it.
    fn image_data_at(
        &mut self,
        width: i32,
        height: i32,
        bits: i32,
        comps: usize,
        sources: Vec<Obj>,
        matrix: Option<[f64; 6]>,
    ) -> Res {
        if width < 0 || height < 0 || !matches!(bits, 1 | 2 | 4 | 8 | 12 | 16) || sources.is_empty()
        {
            return fail("rangecheck");
        }
        let per_source = if sources.len() > 1 { 1 } else { comps as u64 };
        let row = (width as u64 * bits as u64 * per_source).div_ceil(8);
        let needed = row.saturating_mul(height as u64);
        let area = width as usize * height as usize;
        let separate = sources.len() > 1;
        let keep = self.art.keep_picture
            && bits == 8
            && (sources.len() == 1 || sources.len() == comps)
            && matches!(comps, 1 | 3 | 4)
            && area > 0
            && area <= 50_000_000
            && self
                .art
                .picture
                .as_ref()
                .is_none_or(|p| p.width * p.height < area);
        if !keep {
            return self.consume(sources, needed);
        }
        let data = if separate {
            // One source a component (Photoshop's red, green and blue):
            // each source's rows in turn, then interleaved.
            let per = area;
            let channels = self.collect_channels(sources, per)?;
            if channels.iter().any(|c| c.len() < per) {
                return Ok(());
            }
            let mut data = Vec::with_capacity(per * comps);
            for i in 0..per {
                data.extend(channels.iter().map(|c| c[i]));
            }
            data
        } else {
            self.collect(sources.into_iter().next().unwrap_or(Obj::Null), needed)?
        };
        let needed = (area * comps) as u64;
        if data.len() as u64 >= needed {
            self.art.picture = Some(super::Captured {
                width: width as usize,
                height: height as usize,
                comps,
                // A matrix that maps the rows upward (d > 0) means the
                // samples start at the image's bottom row.
                upward: matrix.is_some_and(|m| m[3] > 0.),
                data,
            });
        }
        Ok(())
    }

    /// Read `per` bytes from each of several procedure sources, calling them
    /// in turn as `image` does (a file or string source among them is read
    /// past, and its channel left short).
    fn collect_channels(&mut self, sources: Vec<Obj>, per: usize) -> Res<Vec<Vec<u8>>> {
        self.art.images += 1;
        let mut channels = vec![Vec::with_capacity(per.min(64 << 20)); sources.len()];
        loop {
            let mut called = false;
            for (index, source) in sources.iter().enumerate() {
                if channels[index].len() >= per
                    || !matches!(source, Obj::Array(_) | Obj::Name(..) | Obj::Op(_))
                {
                    continue;
                }
                self.step()?;
                self.exec(source.clone())?;
                called = true;
                match self.pop()? {
                    Obj::Str(chunk) if chunk.len > 0 => {
                        channels[index].extend_from_slice(&chunk.to_vec())
                    }
                    Obj::Str(_) => channels[index].resize(per, 0),
                    _ => return fail("typecheck"),
                }
            }
            if !called {
                break;
            }
        }
        for channel in &mut channels {
            channel.truncate(per);
        }
        Ok(channels)
    }

    /// Read `needed` bytes of an image's data from one source and keep them:
    /// a file, a procedure called until it has given enough, or a string,
    /// which is reused.
    fn collect(&mut self, source: Obj, needed: u64) -> Res<Vec<u8>> {
        self.art.images += 1;
        let needed = needed as usize;
        let mut data = Vec::with_capacity(needed.min(64 << 20));
        match &source {
            Obj::File(file, _) => {
                while data.len() < needed {
                    match stream::read(file).map_err(Fault::Error)? {
                        Some(byte) => data.push(byte),
                        None => break,
                    }
                }
                stream::settle(file).map_err(Fault::Error)?;
            }
            Obj::Str(text) => {
                let bytes = text.to_vec();
                if !bytes.is_empty() {
                    while data.len() < needed {
                        let take = (needed - data.len()).min(bytes.len());
                        data.extend_from_slice(&bytes[..take]);
                    }
                }
            }
            Obj::Array(_) | Obj::Name(..) | Obj::Op(_) => {
                while data.len() < needed {
                    self.step()?;
                    self.exec(source.clone())?;
                    match self.pop()? {
                        Obj::Str(chunk) if chunk.len > 0 => data.extend_from_slice(&chunk.to_vec()),
                        Obj::Str(_) => break,
                        _ => return fail("typecheck"),
                    }
                }
            }
            _ => return fail("typecheck"),
        }
        data.truncate(needed);
        Ok(data)
    }

    /// The image dictionary form of `image` and `imagemask`.
    fn image_dict(&mut self, dict: &Dict, mask: bool) -> Res {
        let int_of = |d: &Dict, key: &[u8]| match d.find(key) {
            Some(Obj::Int(v)) => Ok(v),
            Some(Obj::Real(v)) => Ok(v as i32),
            _ => fail("rangecheck"),
        };
        let sources_of = |d: &Dict| -> Res<Vec<Obj>> {
            let source = d.find(b"DataSource").ok_or(Fault::Error("undefined"))?;
            match (d.find(b"MultipleDataSources"), source) {
                (Some(Obj::Bool(true)), Obj::Array(list)) => Ok(list.to_vec()),
                (_, source) => Ok(vec![source]),
            }
        };
        let comps = if mask { 1 } else { self.gs.space.count() };
        match int_of(dict, b"ImageType").unwrap_or(1) {
            3 => {
                let (Some(Obj::Dict(data)), Some(Obj::Dict(masks))) =
                    (dict.find(b"DataDict"), dict.find(b"MaskDict"))
                else {
                    return fail("typecheck");
                };
                let (width, height) = (int_of(&data, b"Width")?, int_of(&data, b"Height")?);
                let bits = int_of(&data, b"BitsPerComponent")?;
                match int_of(dict, b"InterleaveType").unwrap_or(1) {
                    3 => {
                        let (mw, mh) = (int_of(&masks, b"Width")?, int_of(&masks, b"Height")?);
                        self.image_data(mw, mh, 1, 1, sources_of(&masks)?)?;
                        self.art.images -= 1;
                        self.image_data(width, height, bits, comps, sources_of(&data)?)
                    }
                    2 => {
                        let (mw, mh) = (int_of(&masks, b"Width")?, int_of(&masks, b"Height")?);
                        let mask_bytes = (mw.max(0) as u64).div_ceil(8) * mh.max(0) as u64;
                        let row =
                            (width.max(0) as u64 * bits.max(0) as u64 * comps as u64).div_ceil(8);
                        self.consume(sources_of(&data)?, mask_bytes + row * height.max(0) as u64)
                    }
                    _ => self.image_data(width, height, bits, comps + 1, sources_of(&data)?),
                }
            }
            _ => {
                let (width, height) = (int_of(dict, b"Width")?, int_of(dict, b"Height")?);
                let bits = if mask {
                    1
                } else {
                    int_of(dict, b"BitsPerComponent")?
                };
                if mask {
                    return self.image_data(width, height, bits, comps, sources_of(dict)?);
                }
                let matrix = dict.find(b"ImageMatrix").and_then(|m| matrix_of(&m));
                self.image_data_at(width, height, bits, comps, sources_of(dict)?, matrix)
            }
        }
    }

    /// The operator of an image: its dictionary form, or the operands of
    /// `image` (grey) and `imagemask` (one bit).
    fn image_operator(&mut self, mask: bool) -> Res {
        if let Some(Obj::Dict(_)) = self.stack.last() {
            let dict = self.pop_dict()?;
            return self.image_dict(&dict, mask);
        }
        let source = self.pop()?;
        let matrix = matrix_of(&self.pop()?);
        let third = self.pop()?;
        let height = self.pop_int()?;
        let width = self.pop_int()?;
        let bits = match (mask, third) {
            (true, _) => 1,
            (false, Obj::Int(bits)) => bits,
            _ => return fail("typecheck"),
        };
        if mask {
            return self.image_data(width, height, bits, 1, vec![source]);
        }
        self.image_data_at(width, height, bits, 1, vec![source], matrix)
    }
}

fn show_string(m: &mut Machine, below: usize) -> Res {
    let text = m.pop_str()?;
    m.drop_n(below)?;
    m.shown(text.len)
}

fn show_under(m: &mut Machine, above: usize) -> Res {
    m.drop_n(above)?;
    let text = m.pop_str()?;
    m.shown(text.len)
}

fn transformed_font(font: &Dict, matrix: [f64; 6]) -> Dict {
    let copy = Dict::new(font.size() + 1);
    for (key, value) in font.entries() {
        let _ = copy.put(key, value);
    }
    let own = font
        .find(b"FontMatrix")
        .and_then(|m| matrix_of(&m))
        .unwrap_or([0.001, 0., 0., 0.001, 0., 0.]);
    copy.set("FontMatrix", Obj::numbers(&multiply(own, matrix)));
    copy
}

fn select_font(m: &mut Machine) -> Res {
    let size = m.pop()?;
    let key = m.pop()?;
    let font = m.font_named(&key)?;
    let matrix = match size.num() {
        Some(s) => [s, 0., 0., s, 0., 0.],
        None => matrix_of(&size).ok_or(Fault::Error("typecheck"))?,
    };
    m.gs.font = Obj::Dict(transformed_font(&font, matrix));
    Ok(())
}

fn define_font(m: &mut Machine) -> Res {
    let font = m.pop_dict()?;
    let key = m.pop()?;
    font.put(Obj::name("FID"), Obj::FontId)
        .map_err(Fault::Error)?;
    m.fonts
        .put(key, Obj::Dict(font.clone()))
        .map_err(Fault::Error)?;
    m.answer(Obj::Dict(font))
}

fn current_font(m: &mut Machine) -> Res {
    let font = match &m.gs.font {
        Obj::Dict(font) => font.clone(),
        _ => m.font_named(&Obj::name("Courier"))?,
    };
    m.answer(Obj::Dict(font))
}

fn colour_image(m: &mut Machine) -> Res {
    let comps = m.pop_int()?;
    if !matches!(comps, 1 | 3 | 4) {
        return fail("rangecheck");
    }
    let multiple = m.pop_bool()?;
    let count = if multiple { comps as usize } else { 1 };
    if m.stack.len() < count {
        return fail("stackunderflow");
    }
    let sources = m.stack.split_off(m.stack.len() - count);
    let matrix = matrix_of(&m.pop()?);
    let bits = m.pop_int()?;
    let height = m.pop_int()?;
    let width = m.pop_int()?;
    m.image_data_at(width, height, bits, comps as usize, sources, matrix)
}

/// The image, font and text operators.
pub(super) static OPS: &[Entry] = &[
    // Images, read past and counted.
    ("image", |m| m.image_operator(false), 5),
    ("imagemask", |m| m.image_operator(true), 5),
    ("colorimage", colour_image, 7),
    // Fonts and text, which is not drawn.
    (
        "findfont",
        |m| {
            let key = m.pop()?;
            let font = m.font_named(&key)?;
            m.answer(Obj::Dict(font))
        },
        1,
    ),
    (
        "scalefont",
        |m| {
            let s = m.pop_num()?;
            let font = m.pop_dict()?;
            m.answer(Obj::Dict(transformed_font(&font, [s, 0., 0., s, 0., 0.])))
        },
        2,
    ),
    (
        "makefont",
        |m| {
            let (_, matrix) = m.pop_matrix()?;
            let font = m.pop_dict()?;
            m.answer(Obj::Dict(transformed_font(&font, matrix)))
        },
        2,
    ),
    (
        "setfont",
        |m| {
            let font = m.pop_dict()?;
            m.gs.font = Obj::Dict(font);
            Ok(())
        },
        1,
    ),
    ("currentfont", current_font, 0),
    ("rootfont", current_font, 0),
    ("selectfont", select_font, 2),
    ("definefont", define_font, 2),
    (
        "undefinefont",
        |m| {
            let key = m.pop()?;
            m.fonts.remove(&key).map_err(Fault::Error)
        },
        1,
    ),
    ("show", |m| show_string(m, 0), 1),
    ("ashow", |m| show_string(m, 2), 3),
    ("widthshow", |m| show_string(m, 3), 4),
    ("awidthshow", |m| show_string(m, 5), 6),
    ("kshow", |m| show_string(m, 1), 2),
    ("cshow", |m| show_string(m, 1), 2),
    ("xshow", |m| show_under(m, 1), 2),
    ("yshow", |m| show_under(m, 1), 2),
    ("xyshow", |m| show_under(m, 1), 2),
    (
        "glyphshow",
        |m| {
            m.pop()?;
            m.shown(1)
        },
        1,
    ),
    (
        "charpath",
        |m| {
            m.pop_bool()?;
            let text = m.pop_str()?;
            m.shown(text.len)
        },
        2,
    ),
    (
        "stringwidth",
        |m| {
            let text = m.pop_str()?;
            let width = m.text_width(text.len);
            m.answer_point(width)
        },
        1,
    ),
    ("setcachedevice", |m| pop_drop(m, 6), 6),
    ("setcachedevice2", |m| pop_drop(m, 10), 10),
    ("setcharwidth", |m| pop_drop(m, 2), 2),
];
