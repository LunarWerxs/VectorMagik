use std::collections::HashMap;

use super::*;

/// One `path` element of an import: its attributes, and its path data as
/// commands with their numbers.
struct Element {
    attrs: HashMap<String, String>,
    commands: Vec<(char, Vec<f64>)>,
}

impl Element {
    fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map_or("", String::as_str)
    }

    fn count(&self, command: char) -> usize {
        self.commands.iter().filter(|c| c.0 == command).count()
    }
}

fn path_data(d: &str) -> Vec<(char, Vec<f64>)> {
    let mut out: Vec<(char, Vec<f64>)> = Vec::new();
    for token in d.split(|c: char| c.is_ascii_whitespace() || c == ',') {
        if token.is_empty() {
            continue;
        }
        match token.chars().next() {
            Some(c) if c.is_ascii_alphabetic() => out.push((c, Vec::new())),
            _ => out.last_mut().unwrap().1.push(token.parse().unwrap()),
        }
    }
    out
}

fn elements(svg: &str) -> Vec<Element> {
    svg.split("<path ")
        .skip(1)
        .map(|chunk| {
            let mut rest = &chunk[..chunk.find("/>").unwrap()];
            let mut attrs = HashMap::new();
            while let Some(eq) = rest.find("=\"") {
                let name = rest[..eq].trim().to_owned();
                let after = &rest[eq + 2..];
                let end = after.find('"').unwrap();
                attrs.insert(name, after[..end].to_owned());
                rest = &after[end + 1..];
            }
            let commands = path_data(&attrs["d"]);
            Element { attrs, commands }
        })
        .collect()
}

fn eps(body: &str) -> Vec<u8> {
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{body}\nshowpage\n%%EOF\n"
    )
    .into_bytes()
}

fn import(body: &str) -> Imported {
    let imported = to_svg(&eps(body)).unwrap();
    // Every import is in the subset the app's own writers read.
    crate::pdf_eps::to_pdf(&imported.svg).unwrap();
    imported
}

fn shapes(body: &str) -> Vec<Element> {
    elements(&import(body).svg)
}

const SQUARE: &str = "0 0 moveto 10 0 lineto 10 10 lineto closepath fill";

#[test]
fn a_plain_path_is_one_element_in_points_with_y_down() {
    let out = import(
        "newpath 10 10 moveto 90 10 lineto 50 90 10 90 10 50 curveto closepath \
         1 0 0 setrgbcolor fill",
    );
    assert!(out.svg.starts_with(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100pt\" height=\"100pt\" \
         viewBox=\"0 0 100 100\">"
    ));
    let found = elements(&out.svg);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].attr("d"), "M 10 90 L 90 90 C 50 10 10 10 10 50 Z");
    assert_eq!(found[0].attr("fill"), "#ff0000");
    assert_eq!((out.pages, out.skipped.len()), (1, 0));
}

#[test]
fn abbreviations_a_prolog_binds_and_defines_are_run() {
    let found = shapes(
        "/bd {bind def} bind def /m {moveto} bd /l {lineto} bd /h {closepath} bd \
         /tri {m l l h} bd /rgb {setrgbcolor} bd \
         /m load 0 get type /operatortype eq {0 0 1 rgb} {1 0 0 rgb} ifelse \
         90 10 50 90 10 10 tri fill",
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].attr("d"), "M 10 90 L 50 10 L 90 90 Z");
    assert_eq!(found[0].attr("fill"), "#0000ff");
}

#[test]
fn translate_rotate_and_scale_place_the_path_and_grestore_undoes_them() {
    let found = shapes(
        "gsave 50 50 translate 90 rotate 2 2 scale \
         0 0 moveto 10 0 lineto 10 5 lineto closepath 0 setgray fill grestore \
         0 0 moveto 5 0 lineto 5 5 lineto closepath fill",
    );
    assert_eq!(found[0].attr("d"), "M 50 50 L 50 30 L 40 30 Z");
    assert_eq!(found[1].attr("d"), "M 0 100 L 5 100 L 5 95 Z");
}

#[test]
fn arcs_become_cubic_curves_on_the_circle() {
    let found = shapes(
        "newpath 50 50 20 0 90 arc stroke newpath 50 50 20 90 0 arcn stroke \
         newpath 50 50 10 0 360 arc closepath fill",
    );
    let on_circle = |e: &Element, radius: f64| {
        let start = &e.commands[0].1;
        let (mut x0, mut y0) = (start[0], start[1]);
        for (command, v) in &e.commands[1..] {
            if *command != 'C' {
                continue;
            }
            // The curve's midpoint lies on the circle too.
            let x = 0.125 * x0 + 0.375 * v[0] + 0.375 * v[2] + 0.125 * v[4];
            let y = 0.125 * y0 + 0.375 * v[1] + 0.375 * v[3] + 0.125 * v[5];
            assert!(((x - 50.).hypot(y - 50.) - radius).abs() < 0.05, "{x} {y}");
            assert!(((v[4] - 50.).hypot(v[5] - 50.) - radius).abs() < 0.01);
            (x0, y0) = (v[4], v[5]);
        }
    };
    assert_eq!(found[0].commands[0], ('M', vec![70., 50.]));
    assert_eq!(found[0].commands[1].1[4..], [50., 30.]);
    assert_eq!(found[0].attr("fill"), "none");
    assert_eq!(found[0].attr("stroke"), "#000000");
    on_circle(&found[0], 20.);
    assert_eq!(found[1].commands[0], ('M', vec![50., 30.]));
    assert_eq!(found[1].commands[1].1[4..], [70., 50.]);
    on_circle(&found[1], 20.);
    assert_eq!((found[2].count('C'), found[2].count('Z')), (4, 1));
    on_circle(&found[2], 10.);
}

#[test]
fn cmyk_separation_and_colour_spaces_become_srgb() {
    let found = shapes(
        "0 1 1 0 setcmykcolor 0 0 10 10 rectfill \
         0 0 0 0.5 setcmykcolor 0 0 10 10 rectfill \
         /DeviceCMYK setcolorspace 0 0 1 0 setcolor 0 0 10 10 rectfill \
         [/Separation (Spot) /DeviceCMYK {dup 0 mul exch dup 0.5 mul exch dup 1 mul exch 0 mul}] \
         setcolorspace 1 setcolor 0 0 10 10 rectfill \
         [/Indexed /DeviceRGB 1 <00000000ff00>] setcolorspace 1 setcolor 0 0 10 10 rectfill \
         0.5 1 1 sethsbcolor 0 0 10 10 rectfill",
    );
    let colours: Vec<&str> = found.iter().map(|e| e.attr("fill")).collect();
    assert_eq!(
        colours,
        ["#ff0000", "#808080", "#ffff00", "#ff8000", "#00ff00", "#00ffff"]
    );
}

#[test]
fn eofill_keeps_the_even_odd_rule() {
    let found = shapes(
        "0 0 moveto 100 0 lineto 100 100 lineto 0 100 lineto closepath \
         25 25 moveto 75 25 lineto 75 75 lineto 25 75 lineto closepath eofill",
    );
    assert_eq!(found[0].attr("fill-rule"), "evenodd");
    assert_eq!((found[0].count('M'), found[0].count('Z')), (2, 2));
}

#[test]
fn a_stroke_is_as_wide_as_the_scale_makes_it_and_joins_the_fill_it_follows() {
    let found = shapes(
        "gsave 2 2 scale 3 setlinewidth 1 setlinejoin 2 setlinecap \
         0 0 moveto 10 10 lineto stroke grestore \
         0 setlinewidth 0 0 moveto 5 5 lineto stroke \
         [2 1] 0 setdash 0 0 moveto 10 0 lineto 10 10 lineto closepath \
         gsave 1 0 0 setrgbcolor fill grestore 0 0 1 setrgbcolor stroke",
    );
    assert_eq!(found[0].attr("d"), "M 0 100 L 20 80");
    assert_eq!(found[0].attr("stroke-width"), "6");
    assert_eq!(found[0].attr("stroke-linejoin"), "round");
    assert_eq!(found[0].attr("stroke-linecap"), "square");
    assert_eq!(found[1].attr("stroke-width"), "0.25");
    assert_eq!(found.len(), 3);
    assert_eq!(found[2].attr("fill"), "#ff0000");
    assert_eq!(found[2].attr("stroke"), "#0000ff");
    let out = import("[2 1] 0 setdash 0 0 moveto 10 0 lineto stroke");
    assert_eq!(out.skipped, ["dashed lines, drawn solid"]);
}

#[test]
fn image_data_is_read_past_through_filters_procedures_and_dictionaries() {
    let out = import(
        "gsave 2 2 8 [2 0 0 2 0 0] currentfile /ASCIIHexDecode filter image\n\
         00ff ff00>\n\
         grestore /picstr 2 string def\n\
         2 2 8 [2 0 0 2 0 0] {currentfile picstr readhexstring pop} image\n\
         00ff\nff00\n\
         << /ImageType 1 /Width 3 /Height 1 /BitsPerComponent 8 /Decode [0 1] \
         /ImageMatrix [3 0 0 1 0 0] /DataSource currentfile /ASCII85Decode filter >> image\n\
         !!!!~>\n\
         /DeviceRGB setcolorspace << /ImageType 1 /Width 1 /Height 1 /BitsPerComponent 8 \
         /Decode [0 1 0 1 0 1] /ImageMatrix [1 0 0 1 0 0] \
         /DataSource currentfile /ASCIIHexDecode filter /DCTDecode filter >> image\n\
         FFD8FFE00004AAAAFFDA0002 12FF0034FFD056 FFD9>\n\
         1 1 true [1 0 0 1 0 0] {<80>} imagemask\n\
         0 setgray 0 0 moveto 10 0 lineto 10 10 lineto fill",
    );
    assert_eq!(out.skipped, ["5 images"]);
    assert_eq!(elements(&out.svg).len(), 1);
}

#[test]
fn text_is_left_out_and_fonts_behave_for_the_prolog() {
    let out = import(
        "/Helvetica findfont 12 scalefont setfont 10 10 moveto (Hello) show \
         /Times-Roman findfont dup length dict begin \
         {1 index /FID ne {def} {pop pop} ifelse} forall \
         /Encoding ISOLatin1Encoding def currentdict end /Times-ISO exch definefont pop \
         /Times-ISO 10 selectfont 10 30 moveto (World) show \
         (abc) stringwidth pop 0 gt {0 0 moveto 10 0 lineto 10 10 lineto fill} if",
    );
    assert_eq!(out.skipped, ["text"]);
    assert_eq!(elements(&out.svg).len(), 1);
}

#[test]
fn stopped_errordict_and_the_probes_prologs_make_work() {
    let found = shapes(&format!(
        "/result 0 def /bump {{/result result 1 add def}} def \
         {{nosuchoperator}} stopped {{bump}} if \
         $error /errorname get /undefined eq {{bump}} if \
         {{1 0 div}} stopped {{pop pop bump}} if \
         errordict /rangecheck {{pop bump}} put -1 array pop \
         /languagelevel where {{pop languagelevel 2 ge {{bump}} if}} if \
         {{/NoSuchProcSet /ProcSet findresource}} stopped {{pop pop bump}} if \
         0 [1 2 3] {{add}} forall 6 eq {{bump}} if \
         0 1 1 100 {{dup 5 gt {{pop exit}} if add}} for 15 eq {{bump}} if \
         mark 1 2 3 counttomark 3 eq {{cleartomark bump}} if \
         (a b) ( ) search {{3 1 roll pop pop (a) eq {{bump}} if}} if \
         16#FF 255 eq 1.5e1 15 eq and {{bump}} if \
         count 0 eq result 11 eq and {{0 1 0 setrgbcolor}} {{1 0 0 setrgbcolor}} ifelse {SQUARE}"
    ));
    assert_eq!(found[0].attr("fill"), "#00ff00");
}

/// An Illustrator 3 file as Illustrator wrote it, less its procedure set.
const ILLUSTRATOR_3: &str = "%!PS-Adobe-2.0 EPSF-1.2\n%%Creator: Adobe Illustrator(TM) 3.2\n\
    %%BoundingBox: 0 0 100 100\n%%DocumentProcSets: Adobe_Illustrator_1.2d1 0 0\n\
    %%EndComments\n%%EndProlog\n%%BeginSetup\nAdobe_Illustrator_1.2d1 /initialize get exec\n\
    %%EndSetup\n0 0 0 1 k\n10 10 m\n90 10 L\n90 90 L\n10 90 l\nf\n\
    1 0 0 0 K\n2 w\n10 10 m\n50 90 50 90 90 10 c\nS\n0 0 1 0 k\n*u\n0 0 m\n100 0 L\n\
    100 100 L\n0 100 L\nf\n25 25 m\n25 75 L\n75 75 L\n75 25 L\nf\n*U\n\
    0.5 g 20 20 m 30 30 40 20 v F\n(Hello) Tx\n0 A\n%%PageTrailer\n%%Trailer\n\
    Adobe_Illustrator_1.2d1 /terminate get exec\n%%EOF\n";

#[test]
fn an_illustrator_3_file_without_its_procedure_set_is_read_with_illustrator_meanings() {
    let out = to_svg(ILLUSTRATOR_3.as_bytes()).unwrap();
    let found = elements(&out.svg);
    assert_eq!(found.len(), 4, "{}", out.svg);
    assert_eq!(found[0].attr("d"), "M 10 90 L 90 90 L 90 10 L 10 10 Z");
    assert_eq!(found[0].attr("fill"), "#000000");
    assert_eq!(found[1].attr("d"), "M 10 90 C 50 10 50 10 90 90");
    assert_eq!(
        (
            found[1].attr("fill"),
            found[1].attr("stroke"),
            found[1].attr("stroke-width")
        ),
        ("none", "#00ffff", "2")
    );
    assert_eq!(found[2].attr("fill"), "#ffff00");
    assert_eq!((found[2].count('M'), found[2].count('Z')), (2, 2));
    assert_eq!(found[3].attr("d"), "M 20 80 C 20 80 30 70 40 80");
    assert_eq!(found[3].attr("fill"), "#808080");
    assert_eq!(out.skipped, ["text"]);
}

#[test]
fn a_dos_binary_header_points_at_the_postscript_between_its_previews() {
    let ps = eps(SQUARE);
    let tiff = b"II*\0not really a preview";
    let start = 30u32;
    let mut file = vec![0xC5, 0xD0, 0xD3, 0xC6];
    for word in [
        start,
        ps.len() as u32,
        0,
        0,
        start + ps.len() as u32,
        tiff.len() as u32,
    ] {
        file.extend(word.to_le_bytes());
    }
    file.extend([0xFF, 0xFF]);
    file.extend(&ps);
    file.extend(tiff);
    assert_eq!(to_svg(&file).unwrap(), to_svg(&ps).unwrap());
    assert!(to_svg(&file[..10]).is_err());
    file[8] = 0xFF;
    assert!(to_svg(&file).unwrap_err().contains("past the end"));
}

#[test]
fn runaway_and_hostile_programs_are_stopped_in_plain_words() {
    let error = convert(&eps("{} loop"), 100_000).unwrap_err();
    assert!(error.contains("runs too long"), "{error}");
    let error = convert(&eps("{{} loop} stopped"), 100_000).unwrap_err();
    assert!(error.contains("runs too long"), "{error}");
    let error = to_svg(&eps("/r {true {r} if} def r")).unwrap_err();
    assert!(error.contains("too deeply"), "{error}");
    let error = to_svg(&eps("2000000 array")).unwrap_err();
    assert!(error.contains("memory"), "{error}");
    let error = to_svg(&eps("100000000 string")).unwrap_err();
    assert!(error.contains("memory"), "{error}");
    let nested = format!("{}{}", "{".repeat(1000), "}".repeat(1000));
    assert!(to_svg(&eps(&nested)).is_err());
    // A chain of arrays a hundred thousand deep is dropped without
    // recursion, as is a cycle.
    let out = import(&format!(
        "/a [] def 100000 {{/a [a] def}} repeat /a null def \
         /b 1 array def b 0 b put {SQUARE}"
    ));
    assert_eq!(elements(&out.svg).len(), 1);
}

#[test]
fn garbage_is_refused() {
    for bad in [
        &b""[..],
        b"\x00\x01\x02garbage",
        b"<svg/>",
        b"%!PS\n\xff\xfe\x80 ))) >",
        b"%!PS\n(unterminated",
        b"%!PS\nnosuchoperator",
        b"%!PS\n0 0 lineto",
        b"%!PS\n{ { {",
    ] {
        assert!(to_svg(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
    assert!(to_svg(b"%!PS\nnosuchoperator")
        .unwrap_err()
        .contains("\"nosuchoperator\""));
}

#[test]
fn decoding_filters_decode() {
    // The LZW example of the PDF reference: -----A---B.
    let found = shapes(&format!(
        "<800B6050220C0C8501> /LZWDecode filter 10 string readstring pop \
         (-----A---B) eq {{0 1 0}} {{1 0 0}} ifelse setrgbcolor {SQUARE}"
    ));
    assert_eq!(found[0].attr("fill"), "#00ff00");
    // A program deflated and hex encoded, run as a file.
    let program = b"0 1 0 setrgbcolor 0 0 moveto 10 0 lineto 10 10 lineto fill";
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(program, 6);
    let hex: String = packed.iter().map(|b| format!("{b:02x}")).collect();
    let found = shapes(&format!(
        "currentfile /ASCIIHexDecode filter /FlateDecode filter cvx exec\n{hex}>\n\
         0 0 1 setrgbcolor 20 20 moveto 30 20 lineto 30 30 lineto fill"
    ));
    let colours: Vec<&str> = found.iter().map(|e| e.attr("fill")).collect();
    assert_eq!(colours, ["#00ff00", "#0000ff"]);
    // Run-length data, and a subfile read past to its end marker.
    let found = shapes(&format!(
        "<02616263fe78> /RunLengthDecode filter 6 string readstring pop \
         (abcxxx) eq {{0 1 0}} {{1 0 0}} ifelse setrgbcolor \
         currentfile 0 (%%EndData) /SubFileDecode filter flushfile\n\
         not PostScript ) }} > at all\n%%EndData\n{SQUARE}"
    ));
    assert_eq!(found[0].attr("fill"), "#00ff00");
}

/// Real files mangled at random (bytes changed, cut, repeated, and
/// operators dropped in) are read or refused, never a panic.
#[test]
fn mangled_files_are_read_or_refused_without_panicking() {
    let mut seeds: Vec<Vec<u8>> = vec![
        ILLUSTRATOR_3.as_bytes().to_vec(),
        eps(&format!(
            "/bd {{bind def}} bind def /s 2 string def {SQUARE} \
             2 2 8 [2 0 0 2 0 0] {{currentfile s readhexstring pop}} image\n00ff\nff00\n\
             50 50 20 0 270 arc gsave 0.5 setgray fill grestore stroke \
             [/Indexed /DeviceRGB 1 <00000000ff00>] setcolorspace 1 setcolor 0 0 5 5 rectfill"
        )),
        crate::pdf_eps::to_eps(
            "<svg width=\"20pt\" height=\"20pt\" viewBox=\"0 0 20 20\"><path fill=\"#ff0000\" \
             stroke=\"#000000\" d=\"M 1 1 L 19 1 L 10 19 Z\"/></svg>",
        )
        .unwrap(),
    ];
    seeds.push(eps("/a [1 2 3] def a {pop} forall currentfile /ASCII85Decode filter 8 string readstring\n!!!!!!!!!!~>\n"));
    const SNIPPETS: &[&str] = &[
        "{",
        "}",
        "[",
        "]",
        "<<",
        ">>",
        "(",
        ")",
        "<",
        ">",
        "<~",
        "~>",
        "exec",
        "loop",
        "repeat",
        "restore",
        "grestore",
        "0 0 moveto",
        "currentfile closefile",
        "100000 array",
        "dup",
        "copy",
        "roll",
        "pop",
        "exit",
        "stop",
        "{} loop",
        "3 -1 roll",
        "cvx exec",
        "bind",
        "save",
        "-1 index",
        "clear",
        "cleartomark",
        "setcolorspace",
        "image",
        "eexec",
        "filter",
        "/ASCII85Decode",
        "/LZWDecode",
        "/FlateDecode",
        "/DCTDecode",
        "/RunLengthDecode",
        "currentfile",
        "readstring",
        "readhexstring",
        "token",
        "def",
        "end",
        "begin",
        "userdict",
        "systemdict",
        "put",
        "get",
        "forall",
        "stopped",
        "quit",
        "showpage",
        "undef",
        "load",
        "store",
        "16#FFFFFFFF",
        "1e308 dup mul",
        "0 div",
        "arc",
        "arcto",
        "pathbbox",
        "reversepath",
        "flattenpath",
        "strokepath",
        "clippath",
        "rectfill",
        "setdash",
        "makefont",
        "definefont",
        "findfont",
        "show",
        "kshow",
        "search",
        "putinterval",
        "getinterval",
        "cvs",
        "cvrs",
        "cvn",
        "setpattern",
        "execform",
        "colorimage",
        "imagemask",
        "%%EOF",
        "\n",
        " ",
        "*u",
        "*U",
        "f",
        "S",
        "m",
        "l",
        "c",
        "v",
        "y",
        "k",
        "x",
        "Xa",
    ];
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n.max(1) as u64) as usize
    };
    for round in 0..300 {
        let mut data = seeds[round % seeds.len()].clone();
        for _ in 0..1 + next(4) {
            let at = next(data.len() + 1);
            match next(6) {
                0 => {
                    for _ in 0..1 + next(8) {
                        let i = next(data.len().max(1));
                        if i < data.len() {
                            data[i] = next(256) as u8;
                        }
                    }
                }
                1 => {
                    let end = (at + next(200)).min(data.len());
                    data.drain(at..end);
                }
                2 => {
                    let end = (at + next(300)).min(data.len());
                    let piece = data[at..end].to_vec();
                    let to = next(data.len() + 1);
                    data.splice(to..to, piece);
                }
                3 => data.truncate(at),
                _ => {
                    let snippet = format!(" {} ", SNIPPETS[next(SNIPPETS.len())]);
                    data.splice(at..at, snippet.bytes());
                }
            }
        }
        let input = data.clone();
        let result = std::thread::Builder::new()
            .stack_size(STACK)
            .spawn(move || {
                if let Ok(out) = convert(&input, 1_000_000) {
                    crate::pdf_eps::to_pdf(&out.svg).unwrap();
                }
            })
            .unwrap()
            .join();
        assert!(
            result.is_ok(),
            "round {round} panicked on {:?}",
            String::from_utf8_lossy(&data)
        );
    }
}

#[test]
fn a_filter_that_inflates_without_end_is_stopped() {
    let zeros = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; 8 << 20], 9);
    let hex: String = zeros.iter().map(|b| format!("{b:02x}")).collect();
    let bytes = eps(&format!(
        "4096 4096 8 [1 0 0 1 0 0] currentfile /ASCIIHexDecode filter /FlateDecode filter image\n\
         {hex}>\n{SQUARE}"
    ));
    let error = convert(&bytes, 100_000).unwrap_err();
    assert!(error.contains("runs too long"), "{error}");
    assert_eq!(elements(&convert(&bytes, STEP_LIMIT).unwrap().svg).len(), 1);
}

/// Encrypts `plain` as eexec does, in hexadecimal.
fn eexec_hex(plain: &[u8]) -> String {
    let mut key: u16 = 55665;
    let mut out = String::new();
    for p in [0u8; 4].iter().chain(plain) {
        let c = p ^ (key >> 8) as u8;
        key = u16::from(c)
            .wrapping_add(key)
            .wrapping_mul(52845)
            .wrapping_add(22719);
        let _ = write!(out, "{c:02x}");
        if out.len() % 64 == 62 {
            out.push('\n');
        }
    }
    out
}

#[test]
fn an_eexec_section_runs_and_the_file_goes_on_after_it() {
    let secret = eexec_hex(
        b"1 0 0 setrgbcolor 0 0 moveto 10 0 lineto 10 10 lineto fill currentfile closefile\n",
    );
    let found = shapes(&format!(
        "currentfile eexec\n{secret}\n0 0 1 setrgbcolor 20 20 moveto 30 20 lineto 30 30 lineto fill"
    ));
    let colours: Vec<&str> = found.iter().map(|e| e.attr("fill")).collect();
    assert_eq!(colours, ["#ff0000", "#0000ff"]);
}

#[test]
fn vectormojos_sample_eps_reads() {
    // VectorMojo's tools/fixtures/simple.eps (MIT, ours).
    let file = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 120 80\n%%LanguageLevel: 2\n\
        %%Pages: 1\n%%EndComments\nnewpath\n10 10 moveto\n110 10 lineto\n110 70 lineto\n\
        10 70 lineto\nclosepath\n0.14 0.34 0.90 setrgbcolor\nfill\nshowpage\n%%EOF\n";
    let out = to_svg(file.as_bytes()).unwrap();
    assert!(out
        .svg
        .contains("width=\"120pt\" height=\"80pt\" viewBox=\"0 0 120 80\""));
    let found = elements(&out.svg);
    assert_eq!(found[0].attr("d"), "M 10 70 L 110 70 L 110 10 L 10 10 Z");
    assert_eq!(found[0].attr("fill"), "#2457e6");
}

/// The app's own documents, written as EPS by `pdf_eps` and read back: the
/// same paths within 0.01 point and the same colours, after the white page
/// `to_eps` paints first.
#[test]
fn eps_the_app_writes_reads_back_as_the_same_paths_and_colours() {
    let documents: [(&str, f64, (f64, f64)); 3] = [
        (
            "<svg width=\"120pt\" height=\"80pt\" viewBox=\"0 0 120 80\">\
             <path fill=\"#1f77b4\" d=\"M 10 10 L 110 10 L 60 70 Z\"/>\
             <path fill=\"#ff7f0e\" fill-rule=\"evenodd\" d=\"M 0 0 L 50 0 L 50 50 L 0 50 Z \
             M 10 10 L 40 10 L 40 40 L 10 40 Z\"/>\
             <path fill=\"none\" stroke=\"#2ca02c\" stroke-width=\"2.5\" stroke-linejoin=\"round\" \
             stroke-linecap=\"round\" d=\"M 5 75 C 30 50 90 50 115 75\"/>\
             <path fill=\"#d62728\" stroke=\"#000000\" stroke-width=\"1\" \
             d=\"M 70 20 L 100 20 L 100 40 Z\"/></svg>",
            1.,
            (0., 0.),
        ),
        (
            "<svg width=\"200\" height=\"100\" viewBox=\"0 0 400 200\">\
             <g fill=\"#123456\" transform=\"translate(10 20)\">\
             <path d=\"M 0 0 L 100 0 L 100 50 Z\"/></g></svg>",
            0.375,
            (3.75, 7.5),
        ),
        (
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG \
             1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n<svg width=\"40pt\" \
             height=\"30pt\" viewBox=\"0 0 80 60\" version=\"1.1\" \
             xmlns=\"http://www.w3.org/2000/svg\">\n<g id=\"#ff0000ff\">\n<path fill=\"#ff0000\" \
             opacity=\"1.00\" d=\" M 0.00 60.00 L 80.00 60.00 L 40.00 0.00 Z\" />\n</g>\n\
             <g id=\"#00ff80ff\">\n<path fill=\"#00ff80\" opacity=\"1.00\" \
             d=\" M 10.00 10.00 C 20.00 0.00 60.00 0.00 70.00 10.00 L 40.00 50.00 Z\" />\n</g>\n</svg>",
            0.5,
            (0., 0.),
        ),
    ];
    for (document, scale, offset) in documents {
        let written = crate::pdf_eps::to_eps(document).unwrap();
        let out = to_svg(&written).unwrap();
        let read = elements(&out.svg);
        let wanted = elements(document);
        assert_eq!(read.len(), wanted.len() + 1, "{}", out.svg);
        assert_eq!(read[0].attr("fill"), "#ffffff");
        assert_eq!(read[0].count('L'), 3);
        for (got, want) in read[1..].iter().zip(&wanted) {
            assert_eq!(got.commands.len(), want.commands.len(), "{}", got.attr("d"));
            for ((c1, v1), (c2, v2)) in got.commands.iter().zip(&want.commands) {
                assert_eq!(c1, c2);
                for (i, (a, b)) in v1.iter().zip(v2).enumerate() {
                    let b = b * scale + if i % 2 == 0 { offset.0 } else { offset.1 };
                    assert!((a - b).abs() < 0.01, "{a} {b} in {}", got.attr("d"));
                }
            }
            let inherited = if want.attrs.contains_key("fill") {
                want.attr("fill")
            } else {
                "#123456"
            };
            assert_eq!(got.attr("fill"), inherited);
            assert_eq!(got.attr("stroke"), want.attr("stroke"));
            assert_eq!(got.attr("fill-rule"), want.attr("fill-rule"));
            if let Some(width) = want.attrs.get("stroke-width") {
                let width: f64 = width.parse().unwrap();
                let got_width: f64 = got.attr("stroke-width").parse().unwrap();
                assert!((got_width - width * scale).abs() < 0.01);
                assert_eq!(
                    got.attr("stroke-linejoin"),
                    want.attrs
                        .get("stroke-linejoin")
                        .map_or("miter", String::as_str)
                );
            }
        }
    }
}

#[test]
fn an_eps_with_no_shapes_offers_the_picture_it_draws_or_its_preview() {
    // One interleaved source, rows from the top.
    let one = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 2 2\n\
        2 2 scale 2 2 8 [2 0 0 -2 0 2] {<ff000000ff000000ffffffff>} false 3 colorimage\nshowpage\n";
    let drawn = picture(one).unwrap().expect("the image is the picture");
    let px: Vec<[u8; 4]> = drawn.pixels.iter().map(|p| p.0).collect();
    assert_eq!(
        px,
        vec![
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255]
        ]
    );
    // Photoshop's form: one procedure a component, and a colour device.
    let separate = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 2 2\n\
        statusdict /processcolors get exec 1 gt { } { quit } ifelse\n\
        2 2 8 [2 0 0 2 0 0] {<ff0000ff>} {<00ff00ff>} {<0000ffff>} true 3 colorimage\nshowpage\n";
    let drawn = picture(separate)
        .unwrap()
        .expect("the channels are the picture");
    let px: Vec<[u8; 4]> = drawn.pixels.iter().map(|p| p.0).collect();
    // The matrix maps rows upward, so the last row comes first.
    assert_eq!(
        px,
        vec![
            [0, 0, 255, 255],
            [255, 255, 255, 255],
            [255, 0, 0, 255],
            [0, 255, 0, 255]
        ]
    );
    // No image drawn: the EPSI preview, 0 white and the maximum black,
    // rows from the bottom.
    let epsi = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 2 2\n%%BeginPreview: 2 2 8 2\n\
        % 00FF\n% 8080\n%%EndPreview\nshowpage\n";
    let drawn = picture(epsi).unwrap().expect("the preview is the picture");
    let grey: Vec<u8> = drawn.pixels.iter().map(|p| p.0[0]).collect();
    assert_eq!(grey, vec![127, 127, 255, 0]);
    // Nothing at all.
    assert!(picture_is_none(
        b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 2 2\nshowpage\n"
    ));
}

fn picture_is_none(bytes: &[u8]) -> bool {
    picture(bytes).unwrap().is_none()
}
