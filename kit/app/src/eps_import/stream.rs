//! The files a program reads: the program itself, strings run as programs,
//! and the decoding filters PostScript layers over them (ASCIIHexDecode,
//! ASCII85Decode, RunLengthDecode, LZWDecode, FlateDecode, SubFileDecode
//! and eexec). DCTDecode is only skipped: images are left out of the
//! import, so a JPEG's data needs its end found, not its pixels decoded.
//!
//! A filter never reads past the end of its own data (FlateDecode hands
//! back what it read ahead, eexec decrypts one byte at a time), so the
//! program goes on after the data where its writer meant it to.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

/// A file object: shared, since `currentfile` and every filter over it
/// read the same bytes.
pub(super) type File = Rc<RefCell<Stream>>;

/// Bytes a filter decodes at a time.
const CHUNK: usize = 4096;
/// Filters stacked on one another.
const MAX_CHAIN: usize = 16;

pub(super) struct Stream {
    source: Source,
    /// Bytes read and handed back, the next one last.
    back: Vec<u8>,
    closed: bool,
    /// The decoder reached its end of data.
    ended: bool,
    work: Rc<Work>,
    depth: usize,
}

enum Source {
    Bytes {
        data: Rc<[u8]>,
        at: usize,
    },
    Filter {
        from: File,
        decoder: Decoder,
        out: Vec<u8>,
        at: usize,
    },
}

/// The bytes every filter of one run has decoded, which count as work, and
/// how many it may: a small file can inflate to gigabytes.
pub(super) struct Work {
    done: Cell<u64>,
    cap: u64,
}

impl Work {
    pub(super) fn new(cap: u64) -> Rc<Self> {
        Rc::new(Self {
            done: Cell::new(0),
            cap,
        })
    }

    pub(super) fn done(&self) -> u64 {
        self.done.get()
    }
}

/// A decoding filter's state.
pub(super) enum Decoder {
    Hex,
    Base85,
    RunLength,
    Lzw(Box<Lzw>),
    Flate(Box<InflateState>),
    Jpeg,
    /// `count` more occurrences of `eod` pass (or, with no `eod`, `count`
    /// more bytes; below zero, all of them); `held` is a partial match.
    SubFile {
        count: i64,
        eod: Vec<u8>,
        held: Vec<u8>,
    },
    /// Type 1 font encryption; `hex` is decided by the first four bytes.
    Eexec {
        key: u16,
        hex: Option<bool>,
        skip: u8,
    },
}

impl Decoder {
    pub(super) fn lzw(early_change: bool) -> Self {
        Decoder::Lzw(Lzw::new(early_change))
    }

    pub(super) fn flate() -> Self {
        Decoder::Flate(InflateState::new_boxed(DataFormat::Zlib))
    }

    /// SubFileDecode's parameters: an empty `eod` with a count of zero
    /// passes everything.
    pub(super) fn sub_file(count: i64, eod: Vec<u8>) -> Self {
        let count = if eod.is_empty() && count == 0 {
            -1
        } else {
            count
        };
        Decoder::SubFile {
            count,
            eod,
            held: Vec::new(),
        }
    }

    pub(super) fn eexec() -> Self {
        Decoder::Eexec {
            key: 55665,
            hex: None,
            skip: 4,
        }
    }

    /// Decodes the next piece into `out`; true at the end of the data.
    fn decode(&mut self, from: &File, out: &mut Vec<u8>) -> Result<bool, &'static str> {
        match self {
            Decoder::Hex => hex(from, out),
            Decoder::Base85 => base85(from, out),
            Decoder::RunLength => run_length(from, out),
            Decoder::Lzw(lzw) => lzw.decode(from, out),
            Decoder::Flate(state) => flate(state, from, out),
            Decoder::Jpeg => skip_jpeg(from).map(|()| true),
            Decoder::SubFile { count, eod, held } => sub_file(count, eod, held, from, out),
            Decoder::Eexec { key, hex, skip } => eexec(key, hex, skip, from, out),
        }
    }
}

impl Stream {
    pub(super) fn bytes(data: Rc<[u8]>, work: &Rc<Work>) -> File {
        Rc::new(RefCell::new(Stream {
            source: Source::Bytes { data, at: 0 },
            back: Vec::new(),
            closed: false,
            ended: false,
            work: work.clone(),
            depth: 0,
        }))
    }

    pub(super) fn filter(
        from: File,
        decoder: Decoder,
        work: &Rc<Work>,
    ) -> Result<File, &'static str> {
        let depth = from.borrow().depth + 1;
        if depth > MAX_CHAIN {
            return Err("limitcheck");
        }
        Ok(Rc::new(RefCell::new(Stream {
            source: Source::Filter {
                from,
                decoder,
                out: Vec::new(),
                at: 0,
            },
            back: Vec::new(),
            closed: false,
            ended: false,
            work: work.clone(),
            depth,
        })))
    }

    fn refill(&mut self) -> Result<(), &'static str> {
        let Source::Filter {
            from,
            decoder,
            out,
            at,
        } = &mut self.source
        else {
            return Ok(());
        };
        out.clear();
        *at = 0;
        let result = decoder.decode(from, out);
        let done = self.work.done() + out.len() as u64 + 1;
        self.work.done.set(done);
        if done > self.work.cap {
            self.ended = true;
            return Err("limitcheck");
        }
        match result {
            Ok(ended) => {
                self.ended = ended || out.is_empty();
                Ok(())
            }
            Err(error) => {
                self.ended = true;
                Err(error)
            }
        }
    }
}

/// The next byte of `file`, or `None` at its end.
pub(super) fn read(file: &File) -> Result<Option<u8>, &'static str> {
    let mut stream = file.borrow_mut();
    if let Some(byte) = stream.back.pop() {
        return Ok(Some(byte));
    }
    if stream.closed {
        return Ok(None);
    }
    loop {
        match &mut stream.source {
            Source::Bytes { data, at } => {
                let byte = data.get(*at).copied();
                if byte.is_some() {
                    *at += 1;
                }
                return Ok(byte);
            }
            Source::Filter { out, at, .. } => {
                if let Some(byte) = out.get(*at).copied() {
                    *at += 1;
                    return Ok(Some(byte));
                }
            }
        }
        if stream.ended {
            return Ok(None);
        }
        stream.refill()?;
    }
}

/// Hands `byte` back to be read next.
pub(super) fn unread(file: &File, byte: u8) {
    file.borrow_mut().back.push(byte);
}

pub(super) fn close(file: &File) {
    let mut stream = file.borrow_mut();
    stream.closed = true;
    stream.back.clear();
}

pub(super) fn is_closed(file: &File) -> bool {
    file.borrow().closed
}

/// Reads and discards up to `count` bytes; how many there were.
pub(super) fn skip(file: &File, count: u64) -> Result<u64, &'static str> {
    let mut done = 0;
    while done < count {
        {
            let mut stream = file.borrow_mut();
            if stream.back.is_empty() && !stream.closed {
                let wanted = usize::try_from(count - done).unwrap_or(usize::MAX);
                match &mut stream.source {
                    Source::Bytes { data, at } => {
                        let taken = (data.len() - *at).min(wanted);
                        *at += taken;
                        done += taken as u64;
                        if taken == 0 {
                            return Ok(done);
                        }
                        continue;
                    }
                    Source::Filter { out, at, .. } if *at < out.len() => {
                        let taken = (out.len() - *at).min(wanted);
                        *at += taken;
                        done += taken as u64;
                        continue;
                    }
                    Source::Filter { .. } => {}
                }
            }
        }
        match read(file)? {
            Some(_) => done += 1,
            None => break,
        }
    }
    Ok(done)
}

/// Where the next byte of a plain file is, for `fileposition`.
pub(super) fn position(file: &File) -> Option<usize> {
    let stream = file.borrow();
    match &stream.source {
        Source::Bytes { at, .. } => Some(at.saturating_sub(stream.back.len())),
        Source::Filter { .. } => None,
    }
}

pub(super) fn set_position(file: &File, position: usize) -> bool {
    let mut stream = file.borrow_mut();
    let Source::Bytes { data, at } = &mut stream.source else {
        return false;
    };
    if position > data.len() {
        return false;
    }
    *at = position;
    stream.back.clear();
    true
}

/// The file a filter reads from.
fn source_of(file: &File) -> Option<File> {
    match &file.borrow().source {
        Source::Filter { from, .. } => Some(from.clone()),
        Source::Bytes { .. } => None,
    }
}

/// Whether closing `file` should read it to the end of its data: a
/// decoding filter other than eexec (whose end is where the program
/// closes it).
pub(super) fn drains_on_close(file: &File) -> bool {
    match &file.borrow().source {
        Source::Filter { decoder, .. } => !matches!(decoder, Decoder::Eexec { .. }),
        Source::Bytes { .. } => false,
    }
}

/// After an image has read what it needs from a filter, reads the filter's
/// end-of-data marker if it is next (`>`, `~>`, a run length of 128), and
/// likewise down the chain, so the program goes on after the data.
pub(super) fn settle(file: &File) -> Result<(), &'static str> {
    let mut current = file.clone();
    while let Some(from) = source_of(&current) {
        match read(&current)? {
            Some(byte) => {
                unread(&current, byte);
                return Ok(());
            }
            None => current = from,
        }
    }
    Ok(())
}

pub(super) fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0)
}

pub(super) fn hex_digit(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Five base-85 digits (each already less 33) as four bytes.
pub(super) fn base85_word(group: &[u8; 5]) -> Option<[u8; 4]> {
    let value = group.iter().fold(0u64, |acc, d| acc * 85 + u64::from(*d));
    u32::try_from(value).ok().map(u32::to_be_bytes)
}

fn hex(from: &File, out: &mut Vec<u8>) -> Result<bool, &'static str> {
    let mut high: Option<u8> = None;
    loop {
        let c = match read(from)? {
            None | Some(b'>') => {
                if let Some(high) = high {
                    out.push(high << 4);
                }
                return Ok(true);
            }
            Some(c) => c,
        };
        if is_space(c) {
            continue;
        }
        let digit = hex_digit(c).ok_or("ioerror")?;
        match high.take() {
            Some(high) => {
                out.push((high << 4) | digit);
                if out.len() >= CHUNK {
                    return Ok(false);
                }
            }
            None => high = Some(digit),
        }
    }
}

fn base85(from: &File, out: &mut Vec<u8>) -> Result<bool, &'static str> {
    let mut group = [0u8; 5];
    let mut n = 0;
    loop {
        match read(from)? {
            c @ (None | Some(b'~')) => {
                if c.is_some() && !matches!(read(from)?, Some(b'>') | None) {
                    return Err("ioerror");
                }
                if n == 1 {
                    return Err("ioerror");
                }
                if n > 0 {
                    group[n..].fill(84);
                    let word = base85_word(&group).ok_or("ioerror")?;
                    out.extend_from_slice(&word[..n - 1]);
                }
                return Ok(true);
            }
            Some(b'z') if n == 0 => out.extend_from_slice(&[0; 4]),
            Some(c) if is_space(c) => continue,
            Some(c @ b'!'..=b'u') => {
                group[n] = c - b'!';
                n += 1;
                if n == 5 {
                    out.extend_from_slice(&base85_word(&group).ok_or("ioerror")?);
                    n = 0;
                }
            }
            Some(_) => return Err("ioerror"),
        }
        if n == 0 && out.len() >= CHUNK {
            return Ok(false);
        }
    }
}

fn run_length(from: &File, out: &mut Vec<u8>) -> Result<bool, &'static str> {
    while out.len() < CHUNK {
        let Some(length) = read(from)? else {
            return Ok(true);
        };
        match length {
            128 => return Ok(true),
            0..=127 => {
                for _ in 0..=length {
                    let Some(byte) = read(from)? else {
                        return Ok(true);
                    };
                    out.push(byte);
                }
            }
            _ => {
                let Some(byte) = read(from)? else {
                    return Ok(true);
                };
                out.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
            }
        }
    }
    Ok(false)
}

/// LZW as PostScript and PDF write it: codes of 9 to 12 bits, most
/// significant bit first, 256 clearing the table and 257 ending the data.
pub(super) struct Lzw {
    prefix: Vec<u16>,
    last: Vec<u8>,
    first: Vec<u8>,
    length: Vec<u16>,
    next: usize,
    width: u32,
    bits: u32,
    count: u32,
    previous: Option<usize>,
    early: usize,
}

impl Lzw {
    fn new(early_change: bool) -> Box<Self> {
        let mut lzw = Box::new(Lzw {
            prefix: vec![0; 4096],
            last: vec![0; 4096],
            first: vec![0; 4096],
            length: vec![0; 4096],
            next: 258,
            width: 9,
            bits: 0,
            count: 0,
            previous: None,
            early: usize::from(early_change),
        });
        for code in 0..256 {
            lzw.last[code] = code as u8;
            lzw.first[code] = code as u8;
            lzw.length[code] = 1;
        }
        lzw
    }

    /// Appends the string of `code` to `out`.
    fn emit(&self, code: usize, out: &mut Vec<u8>) {
        let length = usize::from(self.length[code]);
        let start = out.len();
        out.resize(start + length, 0);
        let mut code = code;
        for slot in out[start..].iter_mut().rev() {
            *slot = self.last[code];
            code = usize::from(self.prefix[code]);
        }
    }

    fn decode(&mut self, from: &File, out: &mut Vec<u8>) -> Result<bool, &'static str> {
        while out.len() < CHUNK {
            while self.count < self.width {
                let Some(byte) = read(from)? else {
                    return Ok(true);
                };
                self.bits = (self.bits << 8) | u32::from(byte);
                self.count += 8;
            }
            self.count -= self.width;
            let code = ((self.bits >> self.count) & ((1 << self.width) - 1)) as usize;
            self.bits &= (1 << self.count) - 1;
            match code {
                256 => {
                    self.next = 258;
                    self.width = 9;
                    self.previous = None;
                }
                257 => return Ok(true),
                _ => {
                    let first = if code < self.next {
                        self.emit(code, out);
                        self.first[code]
                    } else if code == self.next {
                        // The string being defined: the previous one and
                        // its own first byte.
                        let previous = self.previous.ok_or("ioerror")?;
                        self.emit(previous, out);
                        out.push(self.first[previous]);
                        self.first[previous]
                    } else {
                        return Err("ioerror");
                    };
                    if let Some(previous) = self.previous.filter(|_| self.next < 4096) {
                        let next = self.next;
                        self.prefix[next] = previous as u16;
                        self.last[next] = first;
                        self.first[next] = self.first[previous];
                        self.length[next] = self.length[previous] + 1;
                        self.next += 1;
                    }
                    self.previous = Some(code);
                    if self.next + self.early >= (1 << self.width) && self.width < 12 {
                        self.width += 1;
                    }
                }
            }
        }
        Ok(false)
    }
}

/// Inflates zlib data, handing back whatever it read past the end.
fn flate(state: &mut InflateState, from: &File, out: &mut Vec<u8>) -> Result<bool, &'static str> {
    let mut input = Vec::with_capacity(512);
    let mut buffer = vec![0u8; CHUNK];
    loop {
        input.clear();
        let mut at_end = false;
        while input.len() < 512 {
            match read(from)? {
                Some(byte) => input.push(byte),
                None => {
                    at_end = true;
                    break;
                }
            }
        }
        let flush = if at_end {
            MZFlush::Finish
        } else {
            MZFlush::None
        };
        let result = inflate(state, &input, &mut buffer, flush);
        out.extend_from_slice(&buffer[..result.bytes_written]);
        for byte in input[result.bytes_consumed..].iter().rev() {
            unread(from, *byte);
        }
        match result.status {
            Ok(MZStatus::StreamEnd) => return Ok(true),
            Ok(_) | Err(MZError::Buf) => {
                if result.bytes_written > 0 {
                    return Ok(false);
                }
                if at_end || result.bytes_consumed == 0 {
                    return Ok(true);
                }
            }
            Err(_) => return Err("ioerror"),
        }
    }
}

/// Reads past one JPEG image: its marker segments, and the entropy-coded
/// data after each start of scan, to the end-of-image marker.
fn skip_jpeg(from: &File) -> Result<(), &'static str> {
    let mut pending: Option<u8> = None;
    loop {
        let marker = match pending.take() {
            Some(marker) => marker,
            None => {
                match read(from)? {
                    Some(0xFF) => {}
                    Some(_) => return Err("ioerror"),
                    None => return Ok(()),
                }
                let mut code = 0xFF;
                while code == 0xFF {
                    let Some(next) = read(from)? else {
                        return Ok(());
                    };
                    code = next;
                }
                code
            }
        };
        match marker {
            0xD9 => return Ok(()),
            0xD8 | 0x01 | 0xD0..=0xD7 => {}
            _ => {
                let (Some(high), Some(low)) = (read(from)?, read(from)?) else {
                    return Ok(());
                };
                let length = ((u64::from(high) << 8) | u64::from(low)).saturating_sub(2);
                if skip(from, length)? < length {
                    return Ok(());
                }
                if marker == 0xDA {
                    match scan_end(from)? {
                        Some(code) => pending = Some(code),
                        None => return Ok(()),
                    }
                }
            }
        }
    }
}

/// Reads entropy-coded data up to the next marker that is not a restart,
/// and answers that marker (`None` at the end of the file).
fn scan_end(from: &File) -> Result<Option<u8>, &'static str> {
    loop {
        let Some(byte) = read(from)? else {
            return Ok(None);
        };
        if byte != 0xFF {
            continue;
        }
        let mut code = 0xFF;
        while code == 0xFF {
            let Some(next) = read(from)? else {
                return Ok(None);
            };
            code = next;
        }
        if code != 0 && !(0xD0..=0xD7).contains(&code) {
            return Ok(Some(code));
        }
    }
}

fn sub_file(
    count: &mut i64,
    eod: &[u8],
    held: &mut Vec<u8>,
    from: &File,
    out: &mut Vec<u8>,
) -> Result<bool, &'static str> {
    if eod.is_empty() {
        while out.len() < CHUNK {
            if *count == 0 {
                return Ok(true);
            }
            let Some(byte) = read(from)? else {
                return Ok(true);
            };
            out.push(byte);
            if *count > 0 {
                *count -= 1;
            }
        }
        return Ok(false);
    }
    while out.len() < CHUNK {
        let Some(byte) = read(from)? else {
            out.append(held);
            return Ok(true);
        };
        held.push(byte);
        while !eod.starts_with(held) {
            out.push(held.remove(0));
        }
        if held.len() == eod.len() {
            if *count == 0 {
                held.clear();
                return Ok(true);
            }
            *count -= 1;
            out.append(held);
        }
    }
    Ok(false)
}

/// Decrypts one byte (the first four, the cipher's random lead-in, are
/// dropped): eexec's end is where the program closes the file, so it must
/// not read ahead.
fn eexec(
    key: &mut u16,
    hex: &mut Option<bool>,
    skip: &mut u8,
    from: &File,
    out: &mut Vec<u8>,
) -> Result<bool, &'static str> {
    if hex.is_none() {
        let mut first = read(from)?;
        while first.is_some_and(is_space) {
            first = read(from)?;
        }
        let Some(first) = first else {
            return Ok(true);
        };
        let mut head = vec![first];
        while head.len() < 4 {
            match read(from)? {
                Some(byte) => head.push(byte),
                None => break,
            }
        }
        *hex = Some(head.len() == 4 && head.iter().all(u8::is_ascii_hexdigit));
        for byte in head.iter().rev() {
            unread(from, *byte);
        }
    }
    loop {
        let cipher = if *hex == Some(true) {
            let mut digits = [0u8; 2];
            for digit in &mut digits {
                loop {
                    let Some(c) = read(from)? else {
                        return Ok(true);
                    };
                    if is_space(c) {
                        continue;
                    }
                    match hex_digit(c) {
                        Some(value) => {
                            *digit = value;
                            break;
                        }
                        None => {
                            unread(from, c);
                            return Ok(true);
                        }
                    }
                }
            }
            (digits[0] << 4) | digits[1]
        } else {
            let Some(byte) = read(from)? else {
                return Ok(true);
            };
            byte
        };
        let plain = cipher ^ (*key >> 8) as u8;
        *key = u16::from(cipher)
            .wrapping_add(*key)
            .wrapping_mul(52845)
            .wrapping_add(22719);
        if *skip > 0 {
            *skip -= 1;
            continue;
        }
        out.push(plain);
        return Ok(false);
    }
}
