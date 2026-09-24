//! The object syntax of a PDF file: the lexer that splits bytes into tokens
//! and the objects (numbers, strings, names, arrays, dictionaries, streams
//! and references) the tokens make. Content streams are read with the same
//! lexer, whose operators are its bare words.
use std::rc::Rc;

/// How deeply arrays and dictionaries may nest before the rest is read flat,
/// so a file of ten thousand `[` cannot exhaust the stack.
const MAX_NESTING: usize = 64;

/// A PDF object. Children are shared, so a copy is cheap however large the
/// array or dictionary it holds.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(Rc<[u8]>),
    Name(Rc<[u8]>),
    Array(Rc<Vec<Obj>>),
    Dict(Rc<Dict>),
    Stream(Rc<Stream>),
    /// An indirect reference by object number (generations are not kept:
    /// the newest definition of a number is the one read).
    Ref(u32),
}

impl Obj {
    pub(super) fn number(&self) -> Option<f64> {
        match self {
            Obj::Int(v) => Some(*v as f64),
            Obj::Real(v) => Some(*v),
            _ => None,
        }
    }

    pub(super) fn int(&self) -> Option<i64> {
        match self {
            Obj::Int(v) => Some(*v),
            Obj::Real(v) if v.fract() == 0. && v.abs() < 9e15 => Some(*v as i64),
            _ => None,
        }
    }

    pub(super) fn name(&self) -> Option<&[u8]> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }

    /// A dictionary, or the dictionary of a stream.
    pub(super) fn dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) => Some(d),
            Obj::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub(super) fn array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// A dictionary's entries in the order written.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Dict(pub(super) Vec<(Rc<[u8]>, Obj)>);

impl Dict {
    /// The value of `key`, unresolved; the first of repeated keys.
    pub(super) fn get(&self, key: &[u8]) -> Option<&Obj> {
        self.0.iter().find(|(k, _)| &**k == key).map(|(_, v)| v)
    }

    /// The value of `key` as a name, when it is one written directly.
    pub(super) fn name(&self, key: &[u8]) -> Option<&[u8]> {
        self.get(key).and_then(Obj::name)
    }
}

/// A stream: its dictionary and its bytes as stored, before any filter.
#[derive(Debug, PartialEq)]
pub(super) struct Stream {
    pub(super) dict: Dict,
    pub(super) raw: Vec<u8>,
}

/// One token of PDF syntax. Bare words are keywords in the file's
/// structure (`obj`, `R`, `xref`) and operators in content streams.
#[derive(Debug, PartialEq)]
pub(super) enum Token<'a> {
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    ArrayOpen,
    ArrayClose,
    DictOpen,
    DictClose,
    Word(&'a [u8]),
}

pub(super) fn is_white(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

pub(super) fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(b: u8) -> bool {
    !is_white(b) && !is_delimiter(b)
}

/// Bytes read as tokens from `pos` on. Every token takes at least one byte,
/// so reading always ends.
pub(super) struct Lexer<'a> {
    pub(super) data: &'a [u8],
    pub(super) pos: usize,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Past white space and comments.
    pub(super) fn skip_space(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.peek_byte().is_some_and(|b| b != b'\n' && b != b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub(super) fn next_token(&mut self) -> Option<Token<'a>> {
        self.skip_space();
        let data = self.data;
        let start = self.pos;
        let b = *data.get(start)?;
        self.pos += 1;
        let token = match b {
            b'[' => Token::ArrayOpen,
            b']' => Token::ArrayClose,
            b'<' if self.peek_byte() == Some(b'<') => {
                self.pos += 1;
                Token::DictOpen
            }
            b'>' if self.peek_byte() == Some(b'>') => {
                self.pos += 1;
                Token::DictClose
            }
            b'<' => Token::Str(self.hex_string()),
            b'(' => Token::Str(self.literal_string()),
            b'/' => Token::Name(self.name()),
            b')' | b'>' | b'{' | b'}' => Token::Word(&data[start..self.pos]),
            _ => {
                while self.peek_byte().is_some_and(is_regular) {
                    self.pos += 1;
                }
                let word = &data[start..self.pos];
                number(word).unwrap_or(Token::Word(word))
            }
        };
        Some(token)
    }

    /// A name after its `/`, with `#xx` escapes decoded.
    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(b) = self.peek_byte().filter(|b| is_regular(*b)) {
            self.pos += 1;
            let escaped = self
                .data
                .get(self.pos..self.pos + 2)
                .filter(|_| b == b'#')
                .and_then(|h| Some(hex_value(h[0])? * 16 + hex_value(h[1])?));
            match escaped {
                Some(v) => {
                    out.push(v);
                    self.pos += 2;
                }
                None => out.push(b),
            }
        }
        out
    }

    /// A literal string after its `(`: balanced parentheses, escapes, and
    /// every end of line read as `\n`. An unterminated one runs to the end.
    fn literal_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(b) = self.peek_byte() {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.peek_byte() else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut value = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek_byte() {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(value as u8);
                        }
                        // A backslash before an end of line continues the string.
                        b'\r' => {
                            if self.peek_byte() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'\r' => {
                    if self.peek_byte() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
        out
    }

    /// A hex string after its `<`; an odd last digit is followed by 0.
    fn hex_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        while let Some(b) = self.peek_byte() {
            self.pos += 1;
            if b == b'>' {
                break;
            }
            let Some(v) = hex_value(b) else { continue };
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

    /// The object that starts with `token`, reading `N G R` as a reference
    /// when `refs` is set (content streams have none). `None` for a token
    /// that starts no object (a stray `]` or `>>`, or an operator).
    pub(super) fn object(&mut self, token: Token<'a>, refs: bool) -> Option<Obj> {
        self.object_at(token, refs, 0)
    }

    fn object_at(&mut self, token: Token<'a>, refs: bool, depth: usize) -> Option<Obj> {
        let obj = match token {
            Token::Int(n) => match refs.then(|| self.reference(n)).flatten() {
                Some(r) => r,
                None => Obj::Int(n),
            },
            Token::Real(v) => Obj::Real(v),
            Token::Name(n) => Obj::Name(n.into()),
            Token::Str(s) => Obj::Str(s.into()),
            Token::ArrayOpen if depth < MAX_NESTING => {
                let mut items = Vec::new();
                while let Some(token) = self.next_token() {
                    match token {
                        Token::ArrayClose => break,
                        token => items.extend(self.object_at(token, refs, depth + 1)),
                    }
                }
                Obj::Array(Rc::new(items))
            }
            Token::DictOpen if depth < MAX_NESTING => Obj::Dict(Rc::new(self.dict(refs, depth))),
            Token::Word(b"true") => Obj::Bool(true),
            Token::Word(b"false") => Obj::Bool(false),
            Token::Word(b"null") => Obj::Null,
            _ => return None,
        };
        Some(obj)
    }

    /// A dictionary's entries after its `<<`, up to `>>` or the end. A value
    /// missing before `>>` reads as null; a stray token where a key belongs
    /// is passed over.
    fn dict(&mut self, refs: bool, depth: usize) -> Dict {
        let mut entries = Vec::new();
        while let Some(token) = self.next_token() {
            match token {
                Token::DictClose => break,
                Token::Name(key) => {
                    let value = match self.next_token() {
                        Some(Token::DictClose) | None => {
                            entries.push((key.into(), Obj::Null));
                            break;
                        }
                        Some(token) => self.object_at(token, refs, depth + 1),
                    };
                    entries.push((key.into(), value.unwrap_or(Obj::Null)));
                }
                other => {
                    let _ = self.object_at(other, refs, depth + 1);
                }
            }
        }
        Dict(entries)
    }

    /// `G R` after the object number `n`, or nothing (and the position kept).
    fn reference(&mut self, n: i64) -> Option<Obj> {
        let save = self.pos;
        let found = match (self.next_token(), self.next_token()) {
            (Some(Token::Int(g)), Some(Token::Word(b"R"))) if g >= 0 => {
                u32::try_from(n).ok().map(Obj::Ref)
            }
            _ => None,
        };
        if found.is_none() {
            self.pos = save;
        }
        found
    }
}

pub(super) fn hex_value(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|v| v as u8)
}

/// A word as a number when it starts like one. PDF numbers have no
/// exponent; malformed ones (`--5`, `1.2.3`, a lone `-`) read as far as they
/// make sense, as viewers read them.
fn number(word: &[u8]) -> Option<Token<'static>> {
    let first = *word.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    let mut i = 0;
    let mut negative = false;
    while i < word.len() && matches!(word[i], b'+' | b'-') {
        negative |= word[i] == b'-';
        i += 1;
    }
    let body_start = i;
    let mut dot = false;
    while i < word.len() && (word[i].is_ascii_digit() || (word[i] == b'.' && !dot)) {
        dot |= word[i] == b'.';
        i += 1;
    }
    let body = std::str::from_utf8(&word[body_start..i]).ok()?;
    if !dot {
        if let Ok(v) = body.parse::<i64>() {
            return Some(Token::Int(if negative { -v } else { v }));
        }
    }
    let v = body
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .unwrap_or(0.);
    Some(Token::Real(if negative { -v } else { v }))
}
