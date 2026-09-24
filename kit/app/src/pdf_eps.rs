//! PDF and EPS written directly from the app's own SVG documents (September
//! 23, 2026). A save used to start Python twice and run CairoSVG: 1.3 to 7 s
//! against a conversion under 0.2 s, and no PDF or EPS at all without Python
//! (the defect sweep of September 23, 2026). The documents are flat filled
//! and stroked paths, so both formats are a direct transcription: the same
//! path operators, the SVG's `viewBox` mapped onto a page of its declared
//! size (points as written; px and unitless numbers at 96 per inch, as
//! CairoSVG sized them), `preserveAspectRatio`'s default placement.
//!
//! Only what the app writes is read: `svg`, `g`, `path` and `rect`, the
//! presentation attributes `fill`, `stroke`, `stroke-width`,
//! `stroke-linejoin`, `stroke-linecap`, `fill-rule`, `opacity`,
//! `fill-opacity` and `stroke-opacity`, and a `translate` transform.
//! Anything else is refused, as is a reference to another file. EPS cannot
//! hold partial opacity and refuses it, as the Python exporter did; it is
//! drawn on a white page like CairoSVG's.
use std::collections::HashMap;
use std::fmt::Write as _;

/// The PDF of `svg`, one page at its declared size.
pub fn to_pdf(svg: &str) -> Result<Vec<u8>, String> {
    let drawing = Drawing::parse(svg)?;
    let mut pdf = Pdf {
        view: drawing.view,
        ..Pdf::default()
    };
    let mut content = String::new();
    let _ = writeln!(content, "{} cm", matrix(&drawing.page_matrix()));
    pdf.items(&drawing.items, &mut content);
    Ok(pdf.finish(&drawing, &content))
}

/// The EPS of `svg` at its declared size, on white.
pub fn to_eps(svg: &str) -> Result<Vec<u8>, String> {
    let drawing = Drawing::parse(svg)?;
    if drawing.translucent() {
        return Err(
            "This image has partial transparency that EPS cannot preserve as vectors. \
                    Save as SVG or PDF instead."
                .into(),
        );
    }
    let (width, height) = (drawing.width, drawing.height);
    let mut out = String::new();
    let _ = write!(
        out,
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 {} {}\n%%HiResBoundingBox: 0 0 {} {}\n\
         %%Creator: VectorMagik\n%%LanguageLevel: 2\n%%Pages: 1\n%%EndComments\n%%BeginProlog\n\
         {EPS_PROLOG}%%EndProlog\n%%Page: 1 1\nq\n1 1 1 rg 0 0 {} {} re f\n{} cm\n",
        width.ceil(),
        height.ceil(),
        num(width),
        num(height),
        num(width),
        num(height),
        matrix(&drawing.page_matrix()),
    );
    eps_items(&drawing.items, &mut out);
    out.push_str("Q\nshowpage\n%%Trailer\n%%EOF\n");
    Ok(out.into_bytes())
}

/// The PDF operators as PostScript procedures, so both formats share one
/// transcription (`B`, fill then stroke, is spelled out in `eps_items`).
const EPS_PROLOG: &str = "/m {moveto} bind def /l {lineto} bind def /c {curveto} bind def\n\
/h {closepath} bind def /f {fill} bind def /f* {eofill} bind def /S {stroke} bind def\n\
/rg {setrgbcolor} bind def /w {setlinewidth} bind def /j {setlinejoin} bind def\n\
/J {setlinecap} bind def /M {setmiterlimit} bind def /q {gsave} bind def\n\
/Q {grestore} bind def\n\
/cm {6 array astore concat} bind def\n\
/re {4 2 roll moveto 1 index 0 rlineto 0 exch rlineto neg 0 rlineto closepath} bind def\n";

/// Presentation state, as inherited down the tree.
#[derive(Clone, Copy)]
struct Style {
    fill: Option<[u8; 3]>,
    stroke: Option<[u8; 3]>,
    stroke_width: f64,
    join: u8,
    cap: u8,
    evenodd: bool,
    fill_opacity: f64,
    stroke_opacity: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Some([0, 0, 0]),
            stroke: None,
            stroke_width: 1.,
            join: 0,
            cap: 0,
            evenodd: false,
            fill_opacity: 1.,
            stroke_opacity: 1.,
        }
    }
}

/// An element still open: its style, opacity, offset and children.
type Open = (Style, f64, (f64, f64), Vec<Item>);

/// A point in user space.
type Xy = (f64, f64);

enum Item {
    Shape {
        path: String,
        style: Style,
        opacity: f64,
    },
    Group {
        opacity: f64,
        offset: (f64, f64),
        items: Vec<Item>,
    },
}

struct Drawing {
    /// The page in points.
    width: f64,
    height: f64,
    view: [f64; 4],
    items: Vec<Item>,
}

impl Drawing {
    fn parse(svg: &str) -> Result<Self, String> {
        let mut stack: Vec<Open> = Vec::new();
        let mut root: Option<(f64, f64, [f64; 4])> = None;
        let mut items = None;
        let mut rest = svg;
        while let Some(at) = rest.find('<') {
            rest = &rest[at..];
            if let Some(skip) = ["<?", "<!--", "<!"].iter().find(|p| rest.starts_with(**p)) {
                let close = if *skip == "<!--" { "-->" } else { ">" };
                let end = rest.find(close).ok_or("Unterminated markup")?;
                rest = &rest[end + close.len()..];
                continue;
            }
            let end = rest.find('>').ok_or("Unterminated tag")?;
            let tag = &rest[1..end];
            rest = &rest[end + 1..];
            if let Some(name) = tag.strip_prefix('/') {
                match name.trim() {
                    "g" | "svg" => {
                        let (_, opacity, offset, children) =
                            stack.pop().ok_or("An unmatched closing tag")?;
                        match stack.last_mut() {
                            Some(parent) => parent.3.push(Item::Group {
                                opacity,
                                offset,
                                items: children,
                            }),
                            None => items = Some(children),
                        }
                    }
                    _ => {}
                }
                continue;
            }
            let closed = tag.ends_with('/');
            let tag = tag.trim_end_matches('/');
            let name = tag.split_whitespace().next().unwrap_or("");
            let attributes = attributes(&tag[name.len()..])?;
            let get = |key: &str| attr(&attributes, key);
            let inherited = stack.last().map(|s| s.0).unwrap_or_default();
            let style = styled(inherited, &attributes)?;
            let opacity = fraction(get("opacity"))?;
            match name {
                "svg" if root.is_none() => {
                    root = Some(page_of(&attributes)?);
                    stack.push((style, opacity, (0., 0.), Vec::new()));
                }
                "g" => {
                    let offset = translation(get("transform"))?;
                    let group = (style, opacity, offset, Vec::new());
                    if closed {
                        continue;
                    }
                    stack.push(group);
                }
                "path" | "rect" => {
                    if get("transform").is_some() {
                        return Err("Only generated vector shapes can be exported".into());
                    }
                    let path = if name == "path" {
                        path_ops(get("d").unwrap_or(""))?
                    } else {
                        let number = |key: &str| number(get(key).unwrap_or("0"));
                        format!(
                            "{} {} {} {} re\n",
                            num(number("x")?),
                            num(number("y")?),
                            num(number("width")?),
                            num(number("height")?)
                        )
                    };
                    let shape = Item::Shape {
                        path,
                        style,
                        opacity,
                    };
                    stack
                        .last_mut()
                        .ok_or("A shape outside the <svg> element")?
                        .3
                        .push(shape);
                    if !closed {
                        // `<path ...></path>`: nothing inside is drawn.
                        let close = format!("</{name}>");
                        let end = rest.find(&close).ok_or("Unterminated shape")?;
                        rest = &rest[end + close.len()..];
                    }
                }
                "title" | "desc" | "metadata" => {
                    if !closed {
                        let close = format!("</{name}>");
                        let end = rest.find(&close).ok_or("Unterminated element")?;
                        rest = &rest[end + close.len()..];
                    }
                }
                _ => return Err("Only generated vector shapes can be exported".into()),
            }
        }
        let (width, height, view) = root.ok_or("Not an SVG document")?;
        let items = items.ok_or("The <svg> element is not closed")?;
        Ok(Self {
            width,
            height,
            view,
            items,
        })
    }

    /// Whether anything is drawn with an opacity below 1.
    fn translucent(&self) -> bool {
        fn any(items: &[Item]) -> bool {
            items.iter().any(|item| match item {
                Item::Shape { style, opacity, .. } => {
                    *opacity < 1. || style.fill_opacity < 1. || style.stroke_opacity < 1.
                }
                Item::Group { opacity, items, .. } => *opacity < 1. || any(items),
            })
        }
        any(&self.items)
    }

    /// User space onto the page, y up, the view box placed as
    /// `preserveAspectRatio="xMidYMid meet"` places it.
    fn page_matrix(&self) -> [f64; 6] {
        let [vx, vy, vw, vh] = self.view;
        let scale = (self.width / vw).min(self.height / vh);
        let tx = (self.width - vw * scale) / 2. - vx * scale;
        let ty = (self.height - vh * scale) / 2. - vy * scale;
        [scale, 0., 0., -scale, tx, self.height - ty]
    }
}

/// The page size in points and the view box of the root element.
/// The attributes of one tag, by name.
type Attributes = HashMap<String, String>;

fn attr<'a>(attributes: &'a Attributes, key: &str) -> Option<&'a str> {
    attributes.get(key).map(String::as_str)
}

fn page_of(attributes: &Attributes) -> Result<(f64, f64, [f64; 4]), String> {
    let get = |key: &str| attr(attributes, key);
    let view = match get("viewBox") {
        Some(value) => {
            let numbers: Vec<f64> = value
                .split(|c: char| c == ',' || c.is_ascii_whitespace())
                .filter(|t| !t.is_empty())
                .map(number)
                .collect::<Result<_, _>>()?;
            match numbers[..] {
                [x, y, w, h] if w > 0. && h > 0. => Some([x, y, w, h]),
                _ => return Err("The viewBox must hold four numbers with a positive size".into()),
            }
        }
        None => None,
    };
    let length = |key: &str, fallback: Option<f64>| -> Result<f64, String> {
        match get(key) {
            Some(value) if !value.trim().ends_with('%') => points(value),
            _ => fallback
                .map(|px| px * 0.75)
                .ok_or_else(|| format!("The <svg> element has no {key}")),
        }
    };
    let width = length("width", view.map(|v| v[2]))?;
    let height = length("height", view.map(|v| v[3]))?;
    if !(width > 0. && height > 0.) {
        return Err("The drawing must have a positive size".into());
    }
    let view = view.unwrap_or([0., 0., width / 0.75, height / 0.75]);
    Ok((width, height, view))
}

/// An SVG length in points: pt as written, px or no unit at 96 per inch.
fn points(value: &str) -> Result<f64, String> {
    let value = value.trim();
    let split = value
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(value.len());
    let (magnitude, unit) = value.split_at(split);
    let factor = match unit {
        "" | "px" => 0.75,
        "pt" => 1.,
        "pc" => 12.,
        "in" => 72.,
        "cm" => 72. / 2.54,
        "mm" => 72. / 25.4,
        _ => return Err(format!("Unsupported length {value:?}")),
    };
    Ok(number(magnitude)? * factor)
}

fn number(text: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("Bad number {text:?}"))
}

fn fraction(value: Option<&str>) -> Result<f64, String> {
    value.map_or(Ok(1.), |v| Ok(number(v)?.clamp(0., 1.)))
}

fn colour(value: &str) -> Result<Option<[u8; 3]>, String> {
    let value = value.trim();
    if value == "none" {
        return Ok(None);
    }
    let hex = value
        .strip_prefix('#')
        .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("Unsupported colour {value:?}"))?;
    let channel = |i: usize, n: usize| {
        let v = u8::from_str_radix(&hex[i * n..i * n + n], 16).unwrap_or(0);
        if n == 1 {
            v * 17
        } else {
            v
        }
    };
    match hex.len() {
        6 => Ok(Some([channel(0, 2), channel(1, 2), channel(2, 2)])),
        3 => Ok(Some([channel(0, 1), channel(1, 1), channel(2, 1)])),
        _ => Err(format!("Unsupported colour {value:?}")),
    }
}

/// `inherited` with the element's own presentation attributes applied.
fn styled(inherited: Style, attributes: &Attributes) -> Result<Style, String> {
    let get = |key: &str| attr(attributes, key);
    let mut style = inherited;
    if let Some(v) = get("fill") {
        style.fill = colour(v)?;
    }
    if let Some(v) = get("stroke") {
        style.stroke = colour(v)?;
    }
    if let Some(v) = get("stroke-width") {
        style.stroke_width = number(v.trim_end_matches("px"))?;
    }
    if let Some(v) = get("stroke-linejoin") {
        style.join = match v.trim() {
            "miter" => 0,
            "round" => 1,
            "bevel" => 2,
            other => return Err(format!("Unsupported stroke-linejoin {other:?}")),
        };
    }
    if let Some(v) = get("stroke-linecap") {
        style.cap = match v.trim() {
            "butt" => 0,
            "round" => 1,
            "square" => 2,
            other => return Err(format!("Unsupported stroke-linecap {other:?}")),
        };
    }
    if let Some(v) = get("fill-rule") {
        style.evenodd = v.trim() == "evenodd";
    }
    if let Some(v) = get("fill-opacity") {
        style.fill_opacity = fraction(Some(v))?;
    }
    if let Some(v) = get("stroke-opacity") {
        style.stroke_opacity = fraction(Some(v))?;
    }
    Ok(style)
}

/// `translate(x y)` (or none); any other transform is refused.
fn translation(value: Option<&str>) -> Result<(f64, f64), String> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok((0., 0.));
    };
    let inner = value
        .strip_prefix("translate(")
        .and_then(|v| v.strip_suffix(')'))
        .ok_or("Only generated vector shapes can be exported")?;
    let numbers: Vec<f64> = inner
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|t| !t.is_empty())
        .map(number)
        .collect::<Result<_, _>>()?;
    match numbers[..] {
        [x] => Ok((x, 0.)),
        [x, y] => Ok((x, y)),
        _ => Err(format!("Bad transform {value:?}")),
    }
}

/// The attributes of a tag, refusing references to other files.
fn attributes(text: &str) -> Result<Attributes, String> {
    let mut out = HashMap::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let eq = rest.find('=').ok_or("A malformed attribute")?;
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|q| *q == '"' || *q == '\'');
        let quote = quote.ok_or("A malformed attribute")?;
        let close = after[1..].find(quote).ok_or("An unterminated attribute")?;
        let value = &after[1..1 + close];
        if name.rsplit(':').next() == Some("href") || value.to_ascii_lowercase().contains("url(") {
            return Err("External resources are not allowed".into());
        }
        out.insert(name.to_owned(), value.to_owned());
        rest = after[close + 2..].trim_start();
    }
    Ok(out)
}

/// SVG path data as PDF path operators in user space: every command made
/// absolute, `H` and `V` as lines, quadratics raised to cubics.
fn path_ops(d: &str) -> Result<String, String> {
    let mut tokens = PathTokens { rest: d };
    let mut out = String::new();
    let (mut current, mut start) = ((0., 0.), (0., 0.));
    // The previous cubic's second control point and quadratic's control
    // point, for `S` and `T`.
    let (mut last_cubic, mut last_quad): (Option<Xy>, Option<Xy>) = (None, None);
    let mut command: Option<char> = None;
    while let Some(next) = tokens.peek() {
        let c = if next.is_ascii_alphabetic() {
            tokens.rest = &tokens.rest[1..];
            next
        } else {
            // Numbers after a command's arguments repeat it; after M, as L.
            match command {
                Some(c) if c != 'Z' && c != 'z' => c,
                _ => return Err("Bad path data".into()),
            }
        };
        let relative = c.is_ascii_lowercase();
        let base = if relative { current } else { (0., 0.) };
        let point = |tokens: &mut PathTokens| -> Result<(f64, f64), String> {
            Ok((tokens.number()? + base.0, tokens.number()? + base.1))
        };
        let (mut cubic, mut quad) = (None, None);
        command = Some(c);
        match c.to_ascii_uppercase() {
            'Z' => {
                out.push_str("h\n");
                current = start;
            }
            'M' => {
                current = point(&mut tokens)?;
                start = current;
                let _ = writeln!(out, "{} {} m", num(current.0), num(current.1));
                command = Some(if relative { 'l' } else { 'L' });
            }
            'L' => {
                current = point(&mut tokens)?;
                let _ = writeln!(out, "{} {} l", num(current.0), num(current.1));
            }
            'H' => {
                current.0 = tokens.number()? + base.0;
                let _ = writeln!(out, "{} {} l", num(current.0), num(current.1));
            }
            'V' => {
                current.1 = tokens.number()? + base.1;
                let _ = writeln!(out, "{} {} l", num(current.0), num(current.1));
            }
            'C' | 'S' => {
                let first = if c.eq_ignore_ascii_case(&'C') {
                    point(&mut tokens)?
                } else {
                    last_cubic.map_or(current, |p| (2. * current.0 - p.0, 2. * current.1 - p.1))
                };
                let second = point(&mut tokens)?;
                let end = point(&mut tokens)?;
                write_cubic(&mut out, first, second, end);
                cubic = Some(second);
                current = end;
            }
            'Q' | 'T' => {
                let control = if c.eq_ignore_ascii_case(&'Q') {
                    point(&mut tokens)?
                } else {
                    last_quad.map_or(current, |p| (2. * current.0 - p.0, 2. * current.1 - p.1))
                };
                let end = point(&mut tokens)?;
                let toward = |a: (f64, f64)| {
                    (
                        a.0 + 2. / 3. * (control.0 - a.0),
                        a.1 + 2. / 3. * (control.1 - a.1),
                    )
                };
                write_cubic(&mut out, toward(current), toward(end), end);
                quad = Some(control);
                current = end;
            }
            _ => return Err("Only generated vector shapes can be exported".into()),
        }
        (last_cubic, last_quad) = (cubic, quad);
    }
    Ok(out)
}

fn write_cubic(out: &mut String, a: (f64, f64), b: (f64, f64), end: (f64, f64)) {
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(a.0),
        num(a.1),
        num(b.0),
        num(b.1),
        num(end.0),
        num(end.1)
    );
}

struct PathTokens<'a> {
    rest: &'a str,
}

impl PathTokens<'_> {
    /// The next character after separators, without taking it.
    fn peek(&mut self) -> Option<char> {
        self.rest = self
            .rest
            .trim_start_matches(|c: char| c == ',' || c.is_ascii_whitespace());
        self.rest.chars().next()
    }

    fn number(&mut self) -> Result<f64, String> {
        self.peek();
        let bytes = self.rest.as_bytes();
        let mut end = 0;
        if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
            end += 1;
        }
        let mut dot = false;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || (bytes[end] == b'.' && !dot)) {
            dot |= bytes[end] == b'.';
            end += 1;
        }
        if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
            let mut exp = end + 1;
            if exp < bytes.len() && (bytes[exp] == b'-' || bytes[exp] == b'+') {
                exp += 1;
            }
            if exp < bytes.len() && bytes[exp].is_ascii_digit() {
                end = exp;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
            }
        }
        let value = number(&self.rest[..end]).map_err(|_| "Bad path data".to_owned())?;
        self.rest = &self.rest[end..];
        Ok(value)
    }
}

/// A number as PDF and PostScript write it: at most four decimals, no
/// trailing zeros.
fn num(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".to_owned(),
        _ => text.to_owned(),
    }
}

fn matrix(m: &[f64; 6]) -> String {
    m.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

/// A colour as three components from 0 to 1, each `c / 255` rounded up at
/// six decimals: MuPDF truncates a component times 255, so 104 written as
/// 0.4078 (or 0.407843) showed as 103, one level dark on most fills, which
/// took the photographs' PDFs 0.1 further from their SVG than CairoSVG's
/// (September 23, 2026); rounded up it is 104 truncated or rounded.
fn rgb(c: [u8; 3]) -> String {
    let channel = |v: u8| {
        let text = format!("{:.6}", (v as f64 / 255. * 1e6).ceil() / 1e6);
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    format!("{} {} {}", channel(c[0]), channel(c[1]), channel(c[2]))
}

/// The paint operator for a shape, or `None` when nothing is painted.
fn paint(style: &Style) -> Option<&'static str> {
    let stroked = style.stroke.is_some() && style.stroke_width > 0.;
    match (style.fill.is_some(), stroked, style.evenodd) {
        (true, true, false) => Some("B"),
        (true, true, true) => Some("B*"),
        (true, false, false) => Some("f"),
        (true, false, true) => Some("f*"),
        (false, true, _) => Some("S"),
        (false, false, _) => None,
    }
}

/// Width, join and cap, and SVG's miter limit of 4 (PDF's and PostScript's
/// own is 10, which draws a sharp corner's miter out further).
fn stroke_state(style: &Style) -> String {
    format!(
        "{} w {} j {} J 4 M\n",
        num(style.stroke_width),
        style.join,
        style.cap
    )
}

#[derive(Default)]
struct Pdf {
    /// `(ca, CA)` of each graphics state `/G<i>`.
    states: Vec<(f64, f64)>,
    /// The content of each transparency group `/X<i>`, object `6 + i`.
    forms: Vec<String>,
    view: [f64; 4],
}

impl Pdf {
    fn state(&mut self, fill: f64, stroke: f64) -> String {
        let index = match self.states.iter().position(|s| *s == (fill, stroke)) {
            Some(i) => i,
            None => {
                self.states.push((fill, stroke));
                self.states.len() - 1
            }
        };
        format!("/G{index} gs\n")
    }

    /// `content` painted as one transparency group at `opacity`.
    fn group(&mut self, content: String, opacity: f64, out: &mut String) {
        self.forms.push(content);
        let form = self.forms.len() - 1;
        let state = self.state(opacity, opacity);
        let _ = write!(out, "q\n{state}/X{form} Do\nQ\n");
    }

    fn items(&mut self, items: &[Item], out: &mut String) {
        for item in items {
            match item {
                Item::Shape {
                    path,
                    style,
                    opacity,
                } => {
                    let Some(op) = paint(style) else {
                        continue;
                    };
                    if op.starts_with('B') && *opacity < 1. {
                        // Fill and stroke overlap: blended once, as a group.
                        let mut inner = String::new();
                        self.shape(path, style, 1., op, &mut inner);
                        self.group(inner, *opacity, out);
                    } else {
                        self.shape(path, style, *opacity, op, out);
                    }
                }
                Item::Group {
                    opacity,
                    offset,
                    items,
                } => {
                    let mut inner = String::new();
                    if *offset != (0., 0.) {
                        let _ = writeln!(inner, "1 0 0 1 {} {} cm", num(offset.0), num(offset.1));
                    }
                    self.items(items, &mut inner);
                    if *opacity < 1. {
                        self.group(inner, *opacity, out);
                    } else {
                        let _ = write!(out, "q\n{inner}Q\n");
                    }
                }
            }
        }
    }

    fn shape(&mut self, path: &str, style: &Style, opacity: f64, op: &str, out: &mut String) {
        let (fill_alpha, stroke_alpha) =
            (opacity * style.fill_opacity, opacity * style.stroke_opacity);
        let translucent = fill_alpha < 1. || stroke_alpha < 1.;
        if translucent {
            out.push_str("q\n");
            let state = self.state(fill_alpha, stroke_alpha);
            out.push_str(&state);
        }
        if let (Some(fill), true) = (style.fill, op != "S") {
            let _ = writeln!(out, "{} rg", rgb(fill));
        }
        if let (Some(stroke), true) = (style.stroke, op.starts_with('B') || op == "S") {
            let _ = writeln!(out, "{} RG", rgb(stroke));
            out.push_str(&stroke_state(style));
        }
        out.push_str(path);
        out.push_str(op);
        out.push('\n');
        if translucent {
            out.push_str("Q\n");
        }
    }

    fn finish(self, drawing: &Drawing, content: &str) -> Vec<u8> {
        let (width, height) = (num(drawing.width), num(drawing.height));
        let [vx, vy, vw, vh] = self.view;
        let bbox = format!(
            "[{} {} {} {}]",
            num(vx - vw),
            num(vy - vh),
            num(vx + 2. * vw),
            num(vy + 2. * vh)
        );
        let mut resources = String::from("<<");
        if !self.states.is_empty() {
            resources.push_str(" /ExtGState <<");
            for (i, (fill, stroke)) in self.states.iter().enumerate() {
                let _ = write!(
                    resources,
                    " /G{i} << /ca {} /CA {} >>",
                    num(*fill),
                    num(*stroke)
                );
            }
            resources.push_str(" >>");
        }
        if !self.forms.is_empty() {
            resources.push_str(" /XObject <<");
            for i in 0..self.forms.len() {
                let _ = write!(resources, " /X{i} {} 0 R", 6 + i);
            }
            resources.push_str(" >>");
        }
        resources.push_str(" >>");
        let group = if self.states.is_empty() {
            ""
        } else {
            " /Group << /S /Transparency /CS /DeviceRGB >>"
        };
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
                 /Resources 4 0 R /Contents 5 0 R{group} >>"
            )
            .into_bytes(),
            resources.into_bytes(),
            stream("", content),
        ];
        for form in &self.forms {
            objects.push(stream(
                &format!(
                    " /Type /XObject /Subtype /Form /BBox {bbox} \
                     /Group << /S /Transparency /CS /DeviceRGB >> /Resources 4 0 R"
                ),
                form,
            ));
        }
        let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
        for offset in offsets {
            let _ = writeln!(table, "{offset:010} 00000 n ");
        }
        let _ = write!(
            table,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        );
        out.extend_from_slice(table.as_bytes());
        out
    }
}

/// A stream object's body: `extra` dictionary entries and `data`, deflated.
fn stream(extra: &str, data: &str) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(data.as_bytes(), 6);
    let mut out = format!(
        "<<{extra} /Length {} /Filter /FlateDecode >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    out.extend_from_slice(&packed);
    out.extend_from_slice(b"\nendstream");
    out
}

/// The shapes as PostScript. PostScript has one current colour, so a shape
/// both filled and stroked is filled inside `gsave`, which keeps its path
/// for the stroke.
fn eps_items(items: &[Item], out: &mut String) {
    for item in items {
        match item {
            Item::Shape { path, style, .. } => {
                let Some(op) = paint(style) else {
                    continue;
                };
                let fill_op = if style.evenodd { "f*" } else { "f" };
                match (style.fill, style.stroke, op) {
                    (Some(fill), Some(stroke), "B" | "B*") => {
                        let _ = write!(
                            out,
                            "{path}q {} rg {fill_op} Q\n{} rg {}S\n",
                            rgb(fill),
                            rgb(stroke),
                            stroke_state(style)
                        );
                    }
                    (_, Some(stroke), "S") => {
                        let _ = writeln!(out, "{} rg {}{path}S", rgb(stroke), stroke_state(style));
                    }
                    (Some(fill), _, _) => {
                        let _ = write!(out, "{} rg\n{path}{fill_op}\n", rgb(fill));
                    }
                    _ => {}
                }
            }
            Item::Group { offset, items, .. } => {
                out.push_str("q\n");
                if *offset != (0., 0.) {
                    let _ = writeln!(out, "1 0 0 1 {} {} cm", num(offset.0), num(offset.1));
                }
                eps_items(items, out);
                out.push_str("Q\n");
            }
        }
    }
}

#[cfg(test)]
mod tests;
