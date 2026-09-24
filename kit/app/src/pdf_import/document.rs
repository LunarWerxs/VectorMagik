//! A PDF file's structure: where each object is (the classic cross-reference
//! table, cross-reference streams, object streams, and the incremental
//! updates chained by `/Prev`, newest first), its objects read on demand,
//! and its streams decoded. A file whose cross-reference is broken is read
//! by scanning it for `N G obj` and its last trailer, as viewers do.
use super::filters::{self, Params};
use super::syntax::{is_white, Dict, Lexer, Obj, Stream, Token};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// All streams of one file together decode to at most this many bytes
/// (16 MB under test, so the test of a file that expands past it stays
/// light).
const MAX_DECODED: usize = if cfg!(test) { 16 << 20 } else { 256 << 20 };

/// How many objects may be loading at once, each waiting on the next (a
/// stream's `/Length` in an object stream whose `/Length` is indirect ...).
const MAX_LOADING: usize = 32;

pub(super) const PROTECTED: &str = "This PDF is protected by a password, so its artwork cannot be \
     read. Remove the protection in the program that made it, save it again and open that copy.";

/// Where an object is defined.
#[derive(Clone, Copy, Debug)]
enum Entry {
    /// At this byte offset in the file.
    At(usize),
    /// In the object stream with this number, at this index.
    InStream(u32, usize),
}

/// A decoded object stream: its data, where its first object starts, and
/// each object's number and offset from there.
struct ObjectStream {
    data: Vec<u8>,
    first: usize,
    index: Vec<(u32, usize)>,
}

/// What a scan of the whole file found, for files whose cross-reference is
/// broken.
#[derive(Default)]
struct Scan {
    /// The offset of every `N G obj`, the last of a number winning.
    objects: HashMap<u32, usize>,
    /// Objects that look like object streams, cross-reference streams and
    /// catalogs, in file order.
    object_streams: Vec<u32>,
    xref_streams: Vec<u32>,
    catalogs: Vec<u32>,
    /// Where each `trailer` keyword ends.
    trailers: Vec<usize>,
}

pub(super) struct Document<'a> {
    data: &'a [u8],
    /// Where `%PDF-` starts; some writers count offsets from there.
    header: usize,
    xref: HashMap<u32, Entry>,
    pub(super) trailer: Rc<Dict>,
    cache: RefCell<HashMap<u32, Obj>>,
    loading: RefCell<Vec<u32>>,
    object_streams: RefCell<HashMap<u32, Option<Rc<ObjectStream>>>>,
    scan: OnceCell<Scan>,
    /// The start of every `endstream`, for streams whose `/Length` is wrong.
    endstreams: OnceCell<Vec<usize>>,
    /// Decoded bytes still allowed.
    budget: Cell<usize>,
}

impl<'a> Document<'a> {
    pub(super) fn open(data: &'a [u8]) -> Result<Self, String> {
        let head = &data[..data.len().min(1024)];
        let header = find(head, b"%PDF-", 0)
            .ok_or("This file is not a PDF: it does not start with %PDF-.")?;
        let mut doc = Document {
            data,
            header,
            xref: HashMap::new(),
            trailer: Rc::default(),
            cache: RefCell::default(),
            loading: RefCell::default(),
            object_streams: RefCell::default(),
            scan: OnceCell::new(),
            endstreams: OnceCell::new(),
            budget: Cell::new(MAX_DECODED),
        };
        let (entries, trailer) = doc.read_chain();
        doc.xref = entries;
        doc.trailer = Rc::new(trailer);
        doc.forget();
        if doc.trailer.get(b"Encrypt").is_some() {
            return Err(PROTECTED.into());
        }
        if doc.catalog().is_none() {
            doc.recover()?;
        }
        Ok(doc)
    }

    /// The document catalog, the root of everything.
    pub(super) fn catalog(&self) -> Option<Obj> {
        let root = self.resolve(self.trailer.get(b"Root")?);
        root.dict()?.get(b"Pages")?;
        Some(root)
    }

    /// Objects read so far, dropped when the cross-reference changes.
    fn forget(&self) {
        self.cache.borrow_mut().clear();
        self.object_streams.borrow_mut().clear();
    }

    /// Every cross-reference section from the last `startxref` back through
    /// `/Prev` (and a hybrid file's `/XRefStm`), the newest definition of
    /// each object winning, and the trailers merged the same way.
    fn read_chain(&self) -> (HashMap<u32, Entry>, Dict) {
        let mut entries = HashMap::new();
        let mut trailer = Dict::default();
        let mut next = rfind(self.data, b"startxref").and_then(|at| {
            let mut lexer = Lexer::new(self.data, at + 9);
            match lexer.next_token() {
                Some(Token::Int(offset)) => usize::try_from(offset).ok(),
                _ => None,
            }
        });
        let mut seen = HashSet::new();
        while let Some(at) = next.take() {
            if !seen.insert(at) || seen.len() > 1024 {
                break;
            }
            let shifted = || (self.header > 0).then(|| self.read_section(at + self.header));
            let Some((section, dict)) = self.read_section(at).or_else(|| shifted().flatten())
            else {
                break;
            };
            let hidden = dict
                .get(b"XRefStm")
                .and_then(Obj::int)
                .and_then(|x| self.read_section(usize::try_from(x).ok()?));
            for (number, entry) in section
                .into_iter()
                .chain(hidden.into_iter().flat_map(|(s, _)| s))
            {
                entries.entry(number).or_insert(entry);
            }
            next = dict
                .get(b"Prev")
                .and_then(Obj::int)
                .and_then(|p| usize::try_from(p).ok());
            for (key, value) in dict.0 {
                if trailer.get(&key).is_none() && &*key != b"Prev" {
                    trailer.0.push((key, value));
                }
            }
        }
        (entries, trailer)
    }

    /// One cross-reference section at `at`: a classic table and its
    /// trailer, or a cross-reference stream and its dictionary.
    fn read_section(&self, at: usize) -> Option<(Vec<(u32, Entry)>, Dict)> {
        let mut lexer = Lexer::new(self.data, at);
        match lexer.next_token()? {
            Token::Word(b"xref") => read_table(&mut lexer),
            Token::Int(_) => {
                let (_, obj) = self.read_object(at, None)?;
                let Obj::Stream(stream) = obj else {
                    return None;
                };
                let entries = self.read_xref_stream(&stream)?;
                Some((entries, stream.dict.clone()))
            }
            _ => None,
        }
    }

    /// A cross-reference stream's entries: `/W` field widths, `/Index`
    /// ranges (all of `/Size` by default), type 1 at an offset, type 2 in
    /// an object stream.
    fn read_xref_stream(&self, stream: &Stream) -> Option<Vec<(u32, Entry)>> {
        let widths: Vec<usize> = self
            .resolve(stream.dict.get(b"W")?)
            .array()?
            .iter()
            .map(|w| self.resolve(w).int().and_then(|w| usize::try_from(w).ok()))
            .collect::<Option<_>>()?;
        if widths.len() != 3 || widths.iter().any(|w| *w > 8) {
            return None;
        }
        let row = widths.iter().sum::<usize>();
        let data = self.decode(stream).ok()?;
        if row == 0 {
            return None;
        }
        let ranges: Vec<i64> = match stream.dict.get(b"Index").map(|i| self.resolve(i)) {
            Some(Obj::Array(items)) => items.iter().filter_map(Obj::int).collect(),
            _ => vec![0, stream.dict.get(b"Size").and_then(Obj::int)?],
        };
        let field = |bytes: &[u8]| bytes.iter().fold(0u64, |v, b| v << 8 | u64::from(*b));
        let mut rows = data.chunks_exact(row);
        let mut entries = Vec::new();
        for [start, count] in ranges.as_chunks::<2>().0 {
            let (Ok(start), Ok(count)) = (u32::try_from(*start), u64::try_from(*count)) else {
                break;
            };
            for i in 0..count {
                let Some(bytes) = rows.next() else {
                    return Some(entries);
                };
                let Some(number) = u32::try_from(i).ok().and_then(|i| start.checked_add(i)) else {
                    break;
                };
                let (kind, rest) = bytes.split_at(widths[0]);
                let (second, third) = rest.split_at(widths[1]);
                let kind = if widths[0] == 0 { 1 } else { field(kind) };
                let (second, third) = (field(second), field(third));
                let entry = match kind {
                    1 => usize::try_from(second).ok().map(Entry::At),
                    2 => match (u32::try_from(second), usize::try_from(third)) {
                        (Ok(s), Ok(i)) => Some(Entry::InStream(s, i)),
                        _ => None,
                    },
                    _ => None,
                };
                entries.extend(entry.map(|e| (number, e)));
            }
        }
        Some(entries)
    }

    /// The indirect object at `offset`, when `N G obj` stands there (and
    /// `N` is `expect`, when given): its number and value.
    fn read_object(&self, offset: usize, expect: Option<u32>) -> Option<(u32, Obj)> {
        let mut lexer = Lexer::new(self.data, offset);
        let (Some(Token::Int(number)), Some(Token::Int(_)), Some(Token::Word(b"obj"))) =
            (lexer.next_token(), lexer.next_token(), lexer.next_token())
        else {
            return None;
        };
        let number = u32::try_from(number).ok()?;
        if expect.is_some_and(|e| e != number) {
            return None;
        }
        let obj = match lexer.next_token() {
            Some(Token::Word(b"endobj")) | None => Obj::Null,
            Some(token) => lexer.object(token, true).unwrap_or(Obj::Null),
        };
        let Obj::Dict(dict) = obj else {
            return Some((number, obj));
        };
        if lexer.next_token() != Some(Token::Word(b"stream")) {
            return Some((number, Obj::Dict(dict)));
        }
        let mut start = lexer.pos;
        match self.data.get(start..start + 2) {
            Some(b"\r\n") => start += 2,
            _ if matches!(self.data.get(start), Some(b'\n' | b'\r')) => start += 1,
            _ => {}
        }
        let raw = self.stream_data(&dict, start).to_vec();
        let dict = Rc::try_unwrap(dict).unwrap_or_else(|d| (*d).clone());
        Some((number, Obj::Stream(Rc::new(Stream { dict, raw }))))
    }

    /// A stream's bytes from `start`: `/Length` of them when `endstream`
    /// follows, else up to the next `endstream`, else to the end.
    fn stream_data(&self, dict: &Dict, start: usize) -> &'a [u8] {
        let data = self.data;
        let length = dict
            .get(b"Length")
            .and_then(|l| self.resolve(l).int())
            .and_then(|l| usize::try_from(l).ok());
        if let Some(end) = length.and_then(|l| start.checked_add(l)) {
            if end <= data.len() {
                let mut lexer = Lexer::new(data, end);
                lexer.skip_space();
                if data[lexer.pos..].starts_with(b"endstream") {
                    return &data[start..end];
                }
            }
        }
        let ends = self.endstreams.get_or_init(|| find_all(data, b"endstream"));
        let at = ends.partition_point(|e| *e < start);
        let Some(&end) = ends.get(at) else {
            return &data[start.min(data.len())..];
        };
        let mut end = end;
        if data[..end].ends_with(b"\r\n") {
            end -= 2;
        } else if data[..end].ends_with(b"\n") || data[..end].ends_with(b"\r") {
            end -= 1;
        }
        &data[start..end.max(start)]
    }

    /// `obj`, or the object it refers to (null when there is none).
    pub(super) fn resolve(&self, obj: &Obj) -> Obj {
        match obj {
            Obj::Ref(number) => self.get(*number),
            other => other.clone(),
        }
    }

    /// Object `number`, read once and kept. An object that refers back to
    /// itself while it loads reads as null.
    pub(super) fn get(&self, number: u32) -> Obj {
        if let Some(obj) = self.cache.borrow().get(&number) {
            return obj.clone();
        }
        {
            let loading = self.loading.borrow();
            if loading.contains(&number) || loading.len() >= MAX_LOADING {
                return Obj::Null;
            }
        }
        self.loading.borrow_mut().push(number);
        let obj = self.load(number).unwrap_or(Obj::Null);
        self.loading.borrow_mut().pop();
        self.cache.borrow_mut().insert(number, obj.clone());
        obj
    }

    fn load(&self, number: u32) -> Option<Obj> {
        let listed = match self.xref.get(&number) {
            Some(Entry::At(offset)) => self.read_object(*offset, Some(number)).or_else(|| {
                let shifted = offset
                    .checked_add(self.header)
                    .filter(|_| self.header > 0)?;
                self.read_object(shifted, Some(number))
            }),
            Some(Entry::InStream(stream, index)) => self.stream_member(*stream, *index, number),
            None => None,
        };
        match listed {
            Some((_, obj)) => Some(obj),
            None => {
                let offset = *self.scan().objects.get(&number)?;
                self.read_object(offset, Some(number)).map(|(_, obj)| obj)
            }
        }
    }

    /// Object `number` from the object stream `stream`, where the
    /// cross-reference puts it at `index` (or wherever it is listed).
    fn stream_member(&self, stream: u32, index: usize, number: u32) -> Option<(u32, Obj)> {
        let objects = self.object_stream(stream)?;
        let listed = objects.index.get(index).filter(|(n, _)| *n == number);
        let (_, offset) = listed.or_else(|| objects.index.iter().find(|(n, _)| *n == number))?;
        let mut lexer = Lexer::new(&objects.data, objects.first.checked_add(*offset)?);
        let token = lexer.next_token()?;
        Some((number, lexer.object(token, true)?))
    }

    fn object_stream(&self, number: u32) -> Option<Rc<ObjectStream>> {
        if let Some(found) = self.object_streams.borrow().get(&number) {
            return found.clone();
        }
        let read = || -> Option<ObjectStream> {
            let Obj::Stream(stream) = self.get(number) else {
                return None;
            };
            let count = stream.dict.get(b"N").and_then(|n| self.resolve(n).int())?;
            let first = stream
                .dict
                .get(b"First")
                .and_then(|f| self.resolve(f).int())?;
            let data = self.decode(&stream).ok()?;
            let mut lexer = Lexer::new(&data, 0);
            let mut index = Vec::new();
            for _ in 0..count.max(0) {
                match (lexer.next_token(), lexer.next_token()) {
                    (Some(Token::Int(n)), Some(Token::Int(offset))) => {
                        match (u32::try_from(n), usize::try_from(offset)) {
                            (Ok(n), Ok(offset)) => index.push((n, offset)),
                            _ => break,
                        }
                    }
                    _ => break,
                }
            }
            Some(ObjectStream {
                first: usize::try_from(first).ok()?,
                data,
                index,
            })
        };
        let found = read().map(Rc::new);
        self.object_streams
            .borrow_mut()
            .insert(number, found.clone());
        found
    }

    /// A stream's bytes with its filters undone, within what is left of the
    /// file's budget. An error says why in words that follow "because".
    pub(super) fn decode(&self, stream: &Stream) -> Result<Vec<u8>, String> {
        let dict = &stream.dict;
        let one_or_many = |key: &[u8]| -> Vec<Obj> {
            match dict.get(key).map(|v| self.resolve(v)) {
                Some(Obj::Array(items)) => items.iter().map(|i| self.resolve(i)).collect(),
                Some(Obj::Null) | None => Vec::new(),
                Some(other) => vec![other],
            }
        };
        let (names, parms) = (one_or_many(b"Filter"), one_or_many(b"DecodeParms"));
        let mut data = stream.raw.clone();
        for (i, name) in names.iter().enumerate() {
            let name = name.name().ok_or("its filter is not a name")?;
            let params = parms
                .get(i)
                .and_then(Obj::dict)
                .map(|d| self.params(d))
                .unwrap_or_default();
            data = filters::apply(name, &data, &params, self.budget.get())?;
            self.budget.set(self.budget.get() - data.len());
        }
        Ok(data)
    }

    fn params(&self, dict: &Dict) -> Params {
        let int = |key: &[u8], default: i64| {
            dict.get(key)
                .and_then(|v| self.resolve(v).int())
                .unwrap_or(default)
        };
        let defaults = Params::default();
        Params {
            predictor: int(b"Predictor", defaults.predictor),
            colors: int(b"Colors", defaults.colors),
            bits: int(b"BitsPerComponent", defaults.bits),
            columns: int(b"Columns", defaults.columns),
            early_change: int(b"EarlyChange", defaults.early_change),
        }
    }

    fn scan(&self) -> &Scan {
        self.scan.get_or_init(|| scan(self.data))
    }

    /// The cross-reference rebuilt from a scan of the file, and the trailer
    /// from its last `trailer` (or cross-reference stream) that names a
    /// catalog, or failing both, from an object that is one.
    fn recover(&mut self) -> Result<(), String> {
        let damaged = || "This PDF is damaged: its list of pages could not be found.".to_owned();
        let scan = self.scan();
        let mut xref: HashMap<u32, Entry> = scan
            .objects
            .iter()
            .map(|(n, at)| (*n, Entry::At(*at)))
            .collect();
        let object_streams = scan.object_streams.clone();
        let trailers = scan.trailers.clone();
        let xref_streams = scan.xref_streams.clone();
        let catalogs = scan.catalogs.clone();
        self.xref.clone_from(&xref);
        self.forget();
        for number in object_streams {
            if let Some(objects) = self.object_stream(number) {
                for (i, (member, _)) in objects.index.iter().enumerate() {
                    xref.entry(*member).or_insert(Entry::InStream(number, i));
                }
            }
        }
        self.xref = xref;
        self.forget();
        let mut candidates: Vec<Rc<Dict>> = Vec::new();
        for at in trailers.iter().rev() {
            let mut lexer = Lexer::new(self.data, *at);
            if let Some(Obj::Dict(dict)) = lexer.next_token().and_then(|t| lexer.object(t, true)) {
                candidates.push(dict);
            }
        }
        for number in xref_streams.iter().rev() {
            if let Some(dict) = self.get(*number).dict() {
                candidates.push(Rc::new(dict.clone()));
            }
        }
        if candidates.iter().any(|t| t.get(b"Encrypt").is_some()) {
            return Err(PROTECTED.into());
        }
        for trailer in candidates {
            self.trailer = trailer;
            if self.catalog().is_some() {
                return Ok(());
            }
        }
        for number in catalogs.iter().rev() {
            self.trailer = Rc::new(Dict(vec![(b"Root".as_slice().into(), Obj::Ref(*number))]));
            if self.catalog().is_some() {
                return Ok(());
            }
        }
        Err(damaged())
    }
}

/// A classic cross-reference table after its `xref`: subsections of
/// `first count` and `offset generation n|f` lines, then the trailer.
fn read_table(lexer: &mut Lexer) -> Option<(Vec<(u32, Entry)>, Dict)> {
    let mut entries = Vec::new();
    loop {
        match lexer.next_token()? {
            Token::Int(first) => {
                let Some(Token::Int(count)) = lexer.next_token() else {
                    return None;
                };
                for i in 0..count.max(0) {
                    let (Some(Token::Int(offset)), Some(Token::Int(_)), Some(Token::Word(kind))) =
                        (lexer.next_token(), lexer.next_token(), lexer.next_token())
                    else {
                        return None;
                    };
                    let number = first.checked_add(i).and_then(|n| u32::try_from(n).ok());
                    let offset = usize::try_from(offset).ok().filter(|o| *o > 0);
                    if let (Some(number), Some(offset), b"n") = (number, offset, kind) {
                        if number > 0 {
                            entries.push((number, Entry::At(offset)));
                        }
                    }
                }
            }
            Token::Word(b"trailer") => {
                let token = lexer.next_token()?;
                return match lexer.object(token, true)? {
                    Obj::Dict(dict) => Some((entries, (*dict).clone())),
                    _ => None,
                };
            }
            _ => return None,
        }
    }
}

/// Every `N G obj` in `data`, and the objects whose first bytes say they
/// are object streams, cross-reference streams or catalogs.
fn scan(data: &[u8]) -> Scan {
    let mut scan = Scan::default();
    let starts = find_all(data, b"obj");
    for (k, &at) in starts.iter().enumerate() {
        let after = data.get(at + 3).copied();
        if at == 0 || !is_white(data[at - 1]) || after.is_some_and(|b| b.is_ascii_alphanumeric()) {
            continue;
        }
        // Back over the generation, white space and the number.
        let mut i = at;
        while i > 0 && is_white(data[i - 1]) {
            i -= 1;
        }
        let generation_end = i;
        while i > 0 && data[i - 1].is_ascii_digit() {
            i -= 1;
        }
        if i == generation_end || i == 0 || !is_white(data[i - 1]) {
            continue;
        }
        while i > 0 && is_white(data[i - 1]) {
            i -= 1;
        }
        let number_end = i;
        while i > 0 && data[i - 1].is_ascii_digit() {
            i -= 1;
        }
        if i == number_end || (i > 0 && data[i - 1].is_ascii_alphanumeric()) {
            continue;
        }
        let Some(number) = std::str::from_utf8(&data[i..number_end])
            .ok()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        scan.objects.insert(number, i);
        let window_end = starts.get(k + 1).copied().unwrap_or(data.len());
        let window = &data[at..window_end.min(at + 512)];
        if find(window, b"/ObjStm", 0).is_some() {
            scan.object_streams.push(number);
        }
        if find(window, b"/XRef", 0).is_some() {
            scan.xref_streams.push(number);
        }
        if find(window, b"/Catalog", 0).is_some() {
            scan.catalogs.push(number);
        }
    }
    scan.trailers = find_all(data, b"trailer")
        .into_iter()
        .map(|at| at + 7)
        .collect();
    scan
}

pub(super) fn find(data: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    data.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn rfind(data: &[u8], needle: &[u8]) -> Option<usize> {
    data.windows(needle.len()).rposition(|w| w == needle)
}

fn find_all(data: &[u8], needle: &[u8]) -> Vec<usize> {
    data.windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}
