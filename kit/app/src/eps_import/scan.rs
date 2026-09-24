//! The scanner: PostScript's tokens read from a file as the interpreter
//! meets them (numbers with radix and exponent, names, literal and
//! immediately evaluated names, strings with their escapes, hexadecimal
//! and base-85 strings, procedures, and the array and dictionary brackets).
use super::object::{Arr, Name, Obj};
use super::stream::{self, base85_word, hex_digit, is_space, File};
use super::{Fault, Machine, Res, MAX_STRING};

/// Procedures nested in one another in the source.
const MAX_NESTING: usize = 256;
/// Bytes in one name or number.
const MAX_NAME: usize = 65_535;

enum Lexeme {
    End,
    Open,
    Close,
    Object(Obj),
}

fn is_delimiter(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn syntax<T>() -> Res<T> {
    Err(Fault::Error("syntaxerror"))
}

impl Machine {
    /// The next object in `file` (a whole procedure when one opens), or
    /// `None` at the end of the file.
    pub(super) fn token(&mut self, file: &File) -> Res<Option<Obj>> {
        let mut open: Vec<Vec<Obj>> = Vec::new();
        loop {
            let obj = match self.lexeme(file)? {
                Lexeme::End if open.is_empty() => return Ok(None),
                Lexeme::End => return syntax(),
                Lexeme::Open => {
                    if open.len() >= MAX_NESTING {
                        return Err(Fault::Error("limitcheck"));
                    }
                    open.push(Vec::new());
                    continue;
                }
                Lexeme::Close => match open.pop() {
                    Some(items) => Obj::Array(Arr::new(items, true)),
                    None => return syntax(),
                },
                Lexeme::Object(obj) => obj,
            };
            match open.last_mut() {
                None => return Ok(Some(obj)),
                Some(items) => {
                    if items.len() >= super::MAX_ARRAY {
                        return Err(Fault::Error("limitcheck"));
                    }
                    items.push(obj);
                }
            }
        }
    }

    pub(super) fn byte(&self, file: &File) -> Res<Option<u8>> {
        stream::read(file).map_err(Fault::Error)
    }

    fn lexeme(&mut self, file: &File) -> Res<Lexeme> {
        loop {
            let Some(c) = self.byte(file)? else {
                return Ok(Lexeme::End);
            };
            let lexeme = match c {
                c if is_space(c) => continue,
                b'%' => {
                    while let Some(c) = self.byte(file)? {
                        if c == b'\r' || c == b'\n' {
                            break;
                        }
                    }
                    continue;
                }
                b'(' => Lexeme::Object(Obj::Str(super::object::Str::new(
                    self.literal_string(file)?,
                ))),
                b'<' => match self.byte(file)? {
                    Some(b'<') => Lexeme::Object(Obj::command("<<")),
                    Some(b'~') => Lexeme::Object(Obj::string(&self.base85_string(file)?)),
                    other => {
                        if let Some(other) = other {
                            stream::unread(file, other);
                        }
                        Lexeme::Object(Obj::string(&self.hex_string(file)?))
                    }
                },
                b'>' => match self.byte(file)? {
                    Some(b'>') => Lexeme::Object(Obj::command(">>")),
                    _ => return syntax(),
                },
                b'[' => Lexeme::Object(Obj::command("[")),
                b']' => Lexeme::Object(Obj::command("]")),
                b'{' => Lexeme::Open,
                b'}' => Lexeme::Close,
                b')' => return syntax(),
                b'/' => {
                    let immediate = match self.byte(file)? {
                        Some(b'/') => true,
                        Some(other) => {
                            stream::unread(file, other);
                            false
                        }
                        None => false,
                    };
                    let name: Name = self.regular(file, None)?.into();
                    if immediate {
                        let value = self.lookup(&name).ok_or(Fault::Error("undefined"))?;
                        Lexeme::Object(value)
                    } else {
                        Lexeme::Object(Obj::Name(name, false))
                    }
                }
                first => {
                    let text = self.regular(file, Some(first))?;
                    Lexeme::Object(number(&text).unwrap_or_else(|| Obj::Name(text.into(), true)))
                }
            };
            return Ok(lexeme);
        }
    }

    /// A name or number's characters, up to a delimiter (left to be read)
    /// or a white-space character (consumed, a CR LF pair as one).
    fn regular(&self, file: &File, first: Option<u8>) -> Res<Vec<u8>> {
        let mut text: Vec<u8> = first.into_iter().collect();
        while let Some(c) = self.byte(file)? {
            if is_space(c) {
                if c == b'\r' {
                    match self.byte(file)? {
                        Some(b'\n') | None => {}
                        Some(other) => stream::unread(file, other),
                    }
                }
                break;
            }
            if is_delimiter(c) {
                stream::unread(file, c);
                break;
            }
            if text.len() >= MAX_NAME {
                return Err(Fault::Error("limitcheck"));
            }
            text.push(c);
        }
        Ok(text)
    }

    /// A `( )` string after its opening parenthesis.
    fn literal_string(&self, file: &File) -> Res<Vec<u8>> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        loop {
            let Some(c) = self.byte(file)? else {
                return syntax();
            };
            let byte = match c {
                b'(' => {
                    depth += 1;
                    c
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    c
                }
                b'\r' => {
                    match self.byte(file)? {
                        Some(b'\n') | None => {}
                        Some(other) => stream::unread(file, other),
                    }
                    b'\n'
                }
                b'\\' => {
                    let Some(escape) = self.byte(file)? else {
                        return syntax();
                    };
                    match escape {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        b'f' => 12,
                        b'0'..=b'7' => {
                            let mut value = u32::from(escape - b'0');
                            for _ in 0..2 {
                                match self.byte(file)? {
                                    Some(digit @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(digit - b'0');
                                    }
                                    Some(other) => {
                                        stream::unread(file, other);
                                        break;
                                    }
                                    None => break,
                                }
                            }
                            value as u8
                        }
                        b'\r' => {
                            match self.byte(file)? {
                                Some(b'\n') | None => {}
                                Some(other) => stream::unread(file, other),
                            }
                            continue;
                        }
                        b'\n' => continue,
                        other => other,
                    }
                }
                other => other,
            };
            if out.len() >= MAX_STRING {
                return Err(Fault::Error("limitcheck"));
            }
            out.push(byte);
        }
    }

    /// A `< >` string after its opening bracket.
    fn hex_string(&self, file: &File) -> Res<Vec<u8>> {
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(c) = self.byte(file)? else {
                return syntax();
            };
            if c == b'>' {
                if let Some(high) = high {
                    out.push(high << 4);
                }
                return Ok(out);
            }
            if is_space(c) {
                continue;
            }
            let Some(digit) = hex_digit(c) else {
                return syntax();
            };
            match high.take() {
                Some(high) => {
                    if out.len() >= MAX_STRING {
                        return Err(Fault::Error("limitcheck"));
                    }
                    out.push((high << 4) | digit);
                }
                None => high = Some(digit),
            }
        }
    }

    /// A `<~ ~>` string after its opening bracket.
    fn base85_string(&self, file: &File) -> Res<Vec<u8>> {
        let mut out = Vec::new();
        let mut group = [0u8; 5];
        let mut n = 0;
        loop {
            let Some(c) = self.byte(file)? else {
                return syntax();
            };
            match c {
                b'~' => {
                    if self.byte(file)? != Some(b'>') || n == 1 {
                        return syntax();
                    }
                    if n > 0 {
                        group[n..].fill(84);
                        let Some(word) = base85_word(&group) else {
                            return syntax();
                        };
                        out.extend_from_slice(&word[..n - 1]);
                    }
                    return Ok(out);
                }
                b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
                c if is_space(c) => {}
                b'!'..=b'u' => {
                    group[n] = c - b'!';
                    n += 1;
                    if n == 5 {
                        let Some(word) = base85_word(&group) else {
                            return syntax();
                        };
                        out.extend_from_slice(&word);
                        n = 0;
                    }
                }
                _ => return syntax(),
            }
            if out.len() > MAX_STRING {
                return Err(Fault::Error("limitcheck"));
            }
        }
    }
}

/// The number a token spells, or `None` when it is a name: integers
/// (becoming reals past 32 bits), reals with an optional exponent, and
/// `base#digits`.
pub(super) fn number(text: &[u8]) -> Option<Obj> {
    if let Some(hash) = text.iter().position(|c| *c == b'#') {
        let (base, digits) = (&text[..hash], &text[hash + 1..]);
        if base.is_empty() || base.len() > 2 || !base.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let base: u32 = std::str::from_utf8(base).ok()?.parse().ok()?;
        if !(2..=36).contains(&base) || digits.is_empty() {
            return None;
        }
        let mut value: u64 = 0;
        for c in digits {
            value = value * u64::from(base) + u64::from((*c as char).to_digit(base)?);
            if value > u64::from(u32::MAX) {
                return None;
            }
        }
        return Some(Obj::Int(value as u32 as i32));
    }
    let body = text
        .strip_prefix(b"+")
        .or_else(|| text.strip_prefix(b"-"))
        .unwrap_or(text);
    if body.is_empty() {
        return None;
    }
    let spelled = std::str::from_utf8(text).ok()?;
    if body.iter().all(u8::is_ascii_digit) {
        return Some(match spelled.parse::<i32>() {
            Ok(value) => Obj::Int(value),
            Err(_) => Obj::Real(spelled.parse().ok()?),
        });
    }
    let mut at = 0;
    let mut digits = 0;
    while at < body.len() && body[at].is_ascii_digit() {
        at += 1;
        digits += 1;
    }
    if at < body.len() && body[at] == b'.' {
        at += 1;
        while at < body.len() && body[at].is_ascii_digit() {
            at += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if at < body.len() && (body[at] == b'e' || body[at] == b'E') {
        at += 1;
        if at < body.len() && (body[at] == b'+' || body[at] == b'-') {
            at += 1;
        }
        let start = at;
        while at < body.len() && body[at].is_ascii_digit() {
            at += 1;
        }
        if at == start {
            return None;
        }
    }
    if at != body.len() {
        return None;
    }
    let value: f64 = spelled
        .replace(".e", ".0e")
        .replace(".E", ".0E")
        .parse()
        .ok()?;
    value.is_finite().then_some(Obj::Real(value))
}

/// A real as `cvs` writes it: six significant digits, always with a
/// decimal point or an exponent.
pub(super) fn format_real(value: f64) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == value.trunc() && value.abs() < 1e7 {
        return format!("{value:.1}");
    }
    let magnitude = value.abs().log10().floor() as i32;
    let trim = |text: &str| {
        let text = text.trim_end_matches('0');
        if text.ends_with('.') {
            format!("{text}0")
        } else {
            text.to_owned()
        }
    };
    if (-4..7).contains(&magnitude) {
        let decimals = (5 - magnitude).max(0) as usize;
        trim(&format!("{value:.decimals$}"))
    } else {
        let text = format!("{value:.5e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let exponent: i32 = exponent.parse().unwrap_or(0);
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exponent.abs())
    }
}
