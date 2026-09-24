//! Files, filters, resources, save and restore, and the parts of
//! systemdict prologs probe (the page device, the user and system
//! parameters, halftones and the like), which answer as a Level 2 printer
//! would. The program never reaches the file system: only `%stdin` and
//! `%stdout` open, and they are empty.
use super::object::{Dict, Obj};
use super::ops::{clear_to_mark, fail, int, iterate, Entry};
use super::stream::{self, hex_digit, Decoder, File, Stream};
use super::{Fault, Machine, Res, MAX_STRING};

fn read(m: &mut Machine) -> Res {
    let file = m.pop_file()?;
    match m.byte(&file)? {
        Some(byte) => {
            m.push(Obj::Int(i32::from(byte)));
            m.answer(Obj::Bool(true))
        }
        None => m.answer(Obj::Bool(false)),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Reading {
    Bytes,
    Hex,
    Line,
}

/// `readstring`, `readhexstring` and `readline`: fills the string from the
/// file and answers the part filled, and whether the read was whole.
fn read_into(m: &mut Machine, how: Reading) -> Res {
    let target = m.pop_str()?;
    let file = m.pop_file()?;
    let mut filled = 0;
    let mut whole = true;
    let mut high: Option<u8> = None;
    loop {
        if how != Reading::Line && filled >= target.len {
            break;
        }
        let Some(byte) = m.byte(&file)? else {
            whole = false;
            break;
        };
        match how {
            Reading::Bytes => {
                target.set(filled, byte);
                filled += 1;
            }
            Reading::Hex => {
                if let Some(digit) = hex_digit(byte) {
                    match high.take() {
                        Some(h) => {
                            target.set(filled, (h << 4) | digit);
                            filled += 1;
                        }
                        None => high = Some(digit),
                    }
                }
            }
            Reading::Line => match byte {
                b'\n' => break,
                b'\r' => {
                    match m.byte(&file)? {
                        Some(b'\n') | None => {}
                        Some(other) => stream::unread(&file, other),
                    }
                    break;
                }
                _ => {
                    if filled >= target.len {
                        return fail("rangecheck");
                    }
                    target.set(filled, byte);
                    filled += 1;
                }
            },
        }
    }
    m.push(Obj::Str(target.interval(0, filled)));
    m.answer(Obj::Bool(whole))
}

fn closefile(m: &mut Machine) -> Res {
    let file = m.pop_file()?;
    if stream::drains_on_close(&file) {
        stream::skip(&file, u64::MAX).map_err(Fault::Error)?;
    }
    stream::close(&file);
    Ok(())
}

fn pop_source(m: &mut Machine) -> Res<File> {
    match m.pop()? {
        Obj::File(file, _) => Ok(file),
        Obj::Str(text) => Ok(Stream::bytes(text.to_vec().into(), &m.work)),
        _ => fail("typecheck"),
    }
}

/// `filter`: the decoding filters over a file or string (their data is
/// read as it is needed), and encoding filters as places output vanishes.
fn filter(m: &mut Machine) -> Res {
    let name = m.pop_name()?;
    let params = match m.stack.last() {
        Some(Obj::Dict(_)) => Some(m.pop_dict()?),
        _ => None,
    };
    let param = |key: &[u8]| params.as_ref().and_then(|d| d.find(key));
    let decoder = match name.as_slice() {
        b"ASCIIHexDecode" => Decoder::Hex,
        b"ASCII85Decode" => Decoder::Base85,
        b"RunLengthDecode" => Decoder::RunLength,
        b"LZWDecode" => {
            Decoder::lzw(param(b"EarlyChange").is_none_or(|v| !matches!(v, Obj::Int(0))))
        }
        b"FlateDecode" => Decoder::flate(),
        b"DCTDecode" => Decoder::Jpeg,
        b"SubFileDecode" => {
            let (count, eod) = if params.is_some() {
                let count = match param(b"EODCount") {
                    Some(Obj::Int(count)) => count,
                    _ => 0,
                };
                (
                    count,
                    param(b"EODString")
                        .and_then(|s| s.text())
                        .unwrap_or_default(),
                )
            } else {
                let eod = m.pop_str()?.to_vec();
                (m.pop_int()?, eod)
            };
            Decoder::sub_file(i64::from(count), eod)
        }
        b"ReusableStreamDecode" => {
            let source = pop_source(m)?;
            let mut data = Vec::new();
            while let Some(byte) = m.byte(&source)? {
                if data.len() >= MAX_STRING {
                    return fail("limitcheck");
                }
                data.push(byte);
            }
            let file = Stream::bytes(data.into(), &m.work);
            return m.answer(Obj::File(file, false));
        }
        b"NullEncode" | b"ASCIIHexEncode" | b"ASCII85Encode" | b"RunLengthEncode"
        | b"LZWEncode" | b"FlateEncode" | b"DCTEncode" | b"CCITTFaxEncode" => {
            if name == b"RunLengthEncode" && params.is_none() {
                m.pop_int()?;
            }
            m.pop()?;
            let sink = Stream::bytes(Vec::new().into(), &m.work);
            return m.answer(Obj::File(sink, false));
        }
        _ => return fail("undefined"),
    };
    let source = pop_source(m)?;
    let file = Stream::filter(source, decoder, &m.work).map_err(Fault::Error)?;
    m.answer(Obj::File(file, false))
}

/// `eexec`: runs the decrypted program with systemdict pushed, as the
/// encrypted part of an embedded Type 1 font expects.
fn eexec(m: &mut Machine) -> Res {
    let source = pop_source(m)?;
    let decrypted = Stream::filter(source, Decoder::eexec(), &m.work).map_err(Fault::Error)?;
    let dicts = m.dicts.len();
    let systemdict = m.systemdict.clone();
    m.dicts.push(systemdict);
    let result = m.run_stream(decrypted, true);
    if m.dicts.len() > dicts {
        m.dicts.truncate(dicts);
    }
    result
}

fn file(m: &mut Machine) -> Res {
    m.pop_str()?;
    let name = m.pop_str()?.to_vec();
    match name.as_slice() {
        b"%stdin" | b"%lineedit" | b"%statementedit" | b"%stdout" | b"%stderr" => {
            let file = Stream::bytes(Vec::new().into(), &m.work);
            m.answer(Obj::File(file, false))
        }
        _ => fail("undefinedfilename"),
    }
}

impl Machine {
    /// The dictionary of a resource category, made when `create` asks.
    fn category(&self, category: &Obj, create: bool) -> Option<Dict> {
        match self.resources.get(category) {
            Some(Obj::Dict(dict)) => Some(dict),
            _ if create => {
                let dict = Dict::new(16);
                self.resources
                    .put(category.clone(), Obj::Dict(dict.clone()))
                    .ok()?;
                Some(dict)
            }
            _ => None,
        }
    }
}

fn is_category(category: &Obj, name: &[u8]) -> bool {
    category.text().as_deref() == Some(name)
}

fn findresource(m: &mut Machine) -> Res {
    let category = m.pop()?;
    let key = m.pop()?;
    if let Some(value) = m.category(&category, false).and_then(|d| d.get(&key)) {
        return m.answer(value);
    }
    if is_category(&category, b"Font") {
        let font = m.font_named(&key)?;
        return m.answer(Obj::Dict(font));
    }
    if is_category(&category, b"Encoding") {
        let name = key.text().unwrap_or_default();
        if name == b"StandardEncoding" || name == b"ISOLatin1Encoding" {
            if let Some(encoding) = m.systemdict.find(&name) {
                return m.answer(encoding);
            }
        }
    }
    if is_category(&category, b"Category") {
        if let Some(dict) = m.category_named(&key) {
            return m.answer(Obj::Dict(dict));
        }
    }
    fail("undefinedresource")
}

/// The resource categories of Level 2.
const CATEGORIES: &[&[u8]] = &[
    b"Category",
    b"Generic",
    b"Font",
    b"Encoding",
    b"Form",
    b"Pattern",
    b"ProcSet",
    b"ColorSpace",
    b"Halftone",
    b"ColorRendering",
    b"Filter",
    b"ColorSpaceFamily",
    b"Emulator",
    b"IODevice",
    b"ColorRenderingType",
    b"FMapType",
    b"FontType",
    b"FormType",
    b"HalftoneType",
    b"ImageType",
    b"PatternType",
];

impl Machine {
    /// A category by name, when it is one of Level 2's or the program
    /// defined it.
    fn category_named(&self, key: &Obj) -> Option<Dict> {
        let standard = key
            .text()
            .is_some_and(|name| CATEGORIES.contains(&name.as_slice()));
        self.category(key, standard)
    }
}

fn defineresource(m: &mut Machine) -> Res {
    let category = m.pop()?;
    let instance = m.pop()?;
    let key = m.pop()?;
    if is_category(&category, b"Font") {
        if let Obj::Dict(font) = &instance {
            font.put(Obj::name("FID"), Obj::FontId)
                .map_err(Fault::Error)?;
            m.fonts
                .put(key.clone(), instance.clone())
                .map_err(Fault::Error)?;
        }
    }
    let dict = m
        .category(&category, true)
        .ok_or(Fault::Error("typecheck"))?;
    dict.put(key, instance.clone()).map_err(Fault::Error)?;
    m.answer(instance)
}

fn resourcestatus(m: &mut Machine) -> Res {
    let category = m.pop()?;
    let key = m.pop()?;
    let known = m
        .category(&category, false)
        .is_some_and(|d| d.get(&key).is_some())
        || (is_category(&category, b"Font") && m.fonts.get(&key).is_some())
        || (is_category(&category, b"Category") && m.category_named(&key).is_some());
    if known {
        m.push(Obj::Int(1));
        m.push(Obj::Int(0));
    }
    m.answer(Obj::Bool(known))
}

fn resourceforall(m: &mut Machine) -> Res {
    let category = m.pop()?;
    let scratch = m.pop_str()?;
    let procedure = m.pop_proc()?;
    let template = m.pop_str()?.to_vec();
    let Some(dict) = m.category(&category, false) else {
        return Ok(());
    };
    for (key, _) in dict.entries() {
        let name = key.text().unwrap_or_default();
        let matches = match template.strip_suffix(b"*") {
            Some(prefix) => name.starts_with(prefix),
            None => name == template,
        };
        if !matches || name.len() > scratch.len {
            continue;
        }
        scratch.write(0, &name);
        m.push(Obj::Str(scratch.interval(0, name.len())));
        if !iterate(m, &procedure)? {
            break;
        }
    }
    Ok(())
}

fn save(m: &mut Machine) -> Res {
    m.next_save += 1;
    let id = m.next_save;
    m.push_state(Some(id))?;
    m.saves.push(id);
    m.answer(Obj::Save(id))
}

/// `restore` brings back the graphics state of its `save`; the memory the
/// program changed since is not rolled back.
fn restore(m: &mut Machine) -> Res {
    let Obj::Save(id) = m.pop()? else {
        return fail("typecheck");
    };
    let Some(at) = m.saves.iter().position(|s| *s == id) else {
        return fail("invalidrestore");
    };
    m.saves.truncate(at);
    while let Some((state, save)) = m.pop_state() {
        if save == Some(id) {
            m.gs = state;
            break;
        }
    }
    Ok(())
}

fn currentpagedevice(m: &mut Machine) -> Res {
    let [left, bottom, right, top] = m.page;
    let device = Dict::new(12);
    device.set("PageSize", Obj::numbers(&[right - left, top - bottom]));
    device.set("HWResolution", Obj::numbers(&[72., 72.]));
    device.set("ImagingBBox", Obj::Null);
    device.set("Orientation", Obj::Int(0));
    device.set("NumCopies", Obj::Null);
    device.set("Duplex", Obj::Bool(false));
    device.set("Policies", Obj::Dict(Dict::new(1)));
    device.set("InputAttributes", Obj::Dict(Dict::new(1)));
    device.set("OutputAttributes", Obj::Dict(Dict::new(1)));
    m.answer(Obj::Dict(device))
}

fn answer_dict(m: &mut Machine) -> Res {
    m.answer(Obj::Dict(Dict::new(4)))
}

/// The user and system parameters prologs read, with a printer's values.
fn parameters(m: &mut Machine, system: bool) -> Res {
    const USER: &[(&str, i32)] = &[
        ("MaxFontItem", 12_000),
        ("MinFontCompress", 100),
        ("MaxUPathItem", 0),
        ("MaxFormItem", 100_000),
        ("MaxPatternItem", 20_000),
        ("MaxScreenItem", 48_000),
        ("MaxOpStack", 100_000),
        ("MaxDictStack", 1_000),
        ("MaxExecStack", 200),
        ("MaxLocalVM", 64_000_000),
        ("VMReclaim", 0),
        ("VMThreshold", 1_000_000),
    ];
    const SYSTEM: &[(&str, i32)] = &[
        ("MaxFontCache", 2_000_000),
        ("CurFontCache", 0),
        ("MaxOutlineCache", 1_000_000),
        ("CurOutlineCache", 0),
        ("MaxUPathCache", 300_000),
        ("CurUPathCache", 0),
        ("MaxFormCache", 1_000_000),
        ("CurFormCache", 0),
        ("MaxPatternCache", 1_000_000),
        ("CurPatternCache", 0),
        ("MaxScreenStorage", 84_000),
        ("CurScreenStorage", 0),
        ("MaxDisplayList", 1_000_000),
        ("CurDisplayList", 0),
        ("Revision", 1),
        ("ByteOrder", 0),
    ];
    let values = if system { SYSTEM } else { USER };
    let dict = Dict::new(values.len());
    for (key, value) in values {
        dict.set(key, Obj::Int(*value));
    }
    m.answer(Obj::Dict(dict))
}

/// The file, resource, memory and device operators.
pub(super) static OPS: &[Entry] = &[
    // Files and filters.
    (
        "currentfile",
        |m| {
            let file = m.current_file();
            m.answer(Obj::File(file, false))
        },
        0,
    ),
    ("read", read, 1),
    ("readstring", |m| read_into(m, Reading::Bytes), 2),
    ("readhexstring", |m| read_into(m, Reading::Hex), 2),
    ("readline", |m| read_into(m, Reading::Line), 2),
    (
        "flushfile",
        |m| {
            let file = m.pop_file()?;
            stream::skip(&file, u64::MAX)
                .map_err(Fault::Error)
                .map(drop)
        },
        1,
    ),
    ("closefile", closefile, 1),
    ("filter", filter, 4),
    ("eexec", eexec, 1),
    ("file", file, 2),
    (
        "run",
        |m| {
            m.pop()?;
            fail("undefinedfilename")
        },
        1,
    ),
    (
        "deletefile",
        |m| {
            m.pop()?;
            fail("undefinedfilename")
        },
        1,
    ),
    (
        "renamefile",
        |m| {
            m.drop_n(2)?;
            fail("undefinedfilename")
        },
        2,
    ),
    ("filenameforall", |m| m.drop_n(3), 3),
    (
        "status",
        |m| match m.pop()? {
            Obj::File(file, _) => m.answer(Obj::Bool(!stream::is_closed(&file))),
            Obj::Str(_) => m.answer(Obj::Bool(false)),
            _ => fail("typecheck"),
        },
        1,
    ),
    (
        "bytesavailable",
        |m| {
            m.pop_file()?;
            m.answer(Obj::Int(-1))
        },
        1,
    ),
    (
        "fileposition",
        |m| {
            let file = m.pop_file()?;
            let position = stream::position(&file).ok_or(Fault::Error("ioerror"))?;
            m.answer(int(position))
        },
        1,
    ),
    (
        "setfileposition",
        |m| {
            let position = m.pop_count(usize::MAX)?;
            let file = m.pop_file()?;
            if stream::set_position(&file, position) {
                Ok(())
            } else {
                fail("ioerror")
            }
        },
        2,
    ),
    ("resetfile", |m| m.drop_n(1), 1),
    ("write", |m| m.drop_n(2), 2),
    ("writestring", |m| m.drop_n(2), 2),
    ("writehexstring", |m| m.drop_n(2), 2),
    ("print", |m| m.drop_n(1), 1),
    ("=", |m| m.drop_n(1), 1),
    ("==", |m| m.drop_n(1), 1),
    ("pstack", |_| Ok(()), 0),
    ("stack", |_| Ok(()), 0),
    ("flush", |_| Ok(()), 0),
    ("echo", |m| m.drop_n(1), 1),
    ("prompt", |_| Ok(()), 0),
    ("printobject", |m| m.drop_n(2), 2),
    ("writeobject", |m| m.drop_n(3), 3),
    ("setobjectformat", |m| m.drop_n(1), 1),
    ("currentobjectformat", |m| m.answer(Obj::Int(0)), 0),
    // Resources.
    ("findresource", findresource, 2),
    ("defineresource", defineresource, 3),
    (
        "undefineresource",
        |m| {
            let category = m.pop()?;
            let key = m.pop()?;
            if let Some(dict) = m.category(&category, false) {
                dict.remove(&key).map_err(Fault::Error)?;
            }
            Ok(())
        },
        2,
    ),
    ("resourcestatus", resourcestatus, 2),
    ("resourceforall", resourceforall, 4),
    (
        "findencoding",
        |m| {
            m.push(Obj::name("Encoding"));
            findresource(m)
        },
        1,
    ),
    // Memory, jobs and the device.
    ("save", save, 0),
    ("restore", restore, 1),
    (
        "vmstatus",
        |m| {
            m.push(Obj::Int(0));
            m.push(Obj::Int(1_000_000));
            m.answer(Obj::Int(64_000_000))
        },
        0,
    ),
    ("vmreclaim", |m| m.drop_n(1), 1),
    ("setvmthreshold", |m| m.drop_n(1), 1),
    ("setglobal", |m| m.pop_bool().map(drop), 1),
    ("currentglobal", |m| m.answer(Obj::Bool(false)), 0),
    (
        "gcheck",
        |m| {
            m.pop()?;
            m.answer(Obj::Bool(false))
        },
        1,
    ),
    ("version", |m| m.answer(Obj::string(b"2017.110")), 0),
    ("product", |m| m.answer(Obj::string(b"VectorMagik")), 0),
    ("revision", |m| m.answer(Obj::Int(1)), 0),
    ("serialnumber", |m| m.answer(Obj::Int(0)), 0),
    ("realtime", |m| m.answer(int((m.steps / 1000) as usize)), 0),
    ("usertime", |m| m.answer(int((m.steps / 1000) as usize)), 0),
    ("setuserparams", |m| m.drop_n(1), 1),
    ("currentuserparams", |m| parameters(m, false), 0),
    ("setsystemparams", |m| m.drop_n(1), 1),
    ("currentsystemparams", |m| parameters(m, true), 0),
    ("setdevparams", |m| m.drop_n(2), 2),
    (
        "currentdevparams",
        |m| {
            m.pop()?;
            answer_dict(m)
        },
        1,
    ),
    (
        "startjob",
        |m| {
            m.drop_n(2)?;
            m.answer(Obj::Bool(false))
        },
        2,
    ),
    ("setjobtimeout", |m| m.drop_n(1), 1),
    ("setpagedevice", |m| m.pop_dict().map(drop), 1),
    ("currentpagedevice", currentpagedevice, 0),
    (
        "internaldict",
        |m| {
            m.pop_int()?;
            let internal = m.internal.clone();
            m.answer(Obj::Dict(internal))
        },
        1,
    ),
    ("defineuserobject", |m| m.drop_n(2), 2),
    ("execuserobject", |m| m.drop_n(1), 1),
    ("undefineuserobject", |m| m.drop_n(1), 1),
    (
        "cachestatus",
        |m| {
            for _ in 0..7 {
                m.push(Obj::Int(0));
            }
            Ok(())
        },
        0,
    ),
    ("setcachelimit", |m| m.drop_n(1), 1),
    ("setcacheparams", clear_to_mark, 0),
    (
        "currentcacheparams",
        |m| {
            m.push(Obj::Mark);
            m.push(Obj::Int(0));
            m.answer(Obj::Int(0))
        },
        0,
    ),
    (
        "ucachestatus",
        |m| {
            m.push(Obj::Mark);
            for _ in 0..5 {
                m.push(Obj::Int(0));
            }
            Ok(())
        },
        0,
    ),
    ("setucacheparams", clear_to_mark, 0),
    ("letter", |_| Ok(()), 0),
    ("legal", |_| Ok(()), 0),
    ("a4", |_| Ok(()), 0),
    ("a3", |_| Ok(()), 0),
    ("b5", |_| Ok(()), 0),
    ("note", |_| Ok(()), 0),
    ("lettersmall", |_| Ok(()), 0),
    ("a4small", |_| Ok(()), 0),
    // Halftones, transfer functions and the like, which do not change the
    // artwork's shapes or colours.
    ("setscreen", |m| m.drop_n(3), 3),
    (
        "currentscreen",
        |m| {
            m.push(Obj::Real(60.));
            m.push(Obj::Real(45.));
            m.answer(Obj::nothing())
        },
        0,
    ),
    ("setcolorscreen", |m| m.drop_n(12), 12),
    (
        "currentcolorscreen",
        |m| {
            for _ in 0..4 {
                m.push(Obj::Real(60.));
                m.push(Obj::Real(45.));
                m.push(Obj::nothing());
            }
            Ok(())
        },
        0,
    ),
    ("settransfer", |m| m.drop_n(1), 1),
    ("currenttransfer", |m| m.answer(Obj::nothing()), 0),
    ("setcolortransfer", |m| m.drop_n(4), 4),
    (
        "currentcolortransfer",
        |m| {
            for _ in 0..4 {
                m.push(Obj::nothing());
            }
            Ok(())
        },
        0,
    ),
    ("setblackgeneration", |m| m.drop_n(1), 1),
    ("currentblackgeneration", |m| m.answer(Obj::nothing()), 0),
    ("setundercolorremoval", |m| m.drop_n(1), 1),
    ("currentundercolorremoval", |m| m.answer(Obj::nothing()), 0),
    ("sethalftone", |m| m.drop_n(1), 1),
    (
        "currenthalftone",
        |m| {
            let halftone = Dict::new(4);
            halftone.set("HalftoneType", Obj::Int(1));
            halftone.set("Frequency", Obj::Real(60.));
            halftone.set("Angle", Obj::Real(45.));
            halftone.set("SpotFunction", Obj::nothing());
            m.answer(Obj::Dict(halftone))
        },
        0,
    ),
    ("setcolorrendering", |m| m.drop_n(1), 1),
    ("currentcolorrendering", answer_dict, 0),
    ("setoverprint", |m| m.drop_n(1), 1),
    ("currentoverprint", |m| m.answer(Obj::Bool(false)), 0),
    ("setsmoothness", |m| m.drop_n(1), 1),
    ("currentsmoothness", |m| m.answer(Obj::Real(0.02)), 0),
];
