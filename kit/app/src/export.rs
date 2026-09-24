//! File-format selection around the generated SVG. No fitting geometry changes.
use std::path::Path;

/// The vector formats a document can be saved as, by output extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Svg,
    /// PDF and EPS are written from the SVG in-process (`crate::pdf_eps`).
    Pdf,
    Eps,
}
/// The format `output`'s extension asks for, case-insensitively; anything
/// but SVG, PDF or EPS is refused.
pub fn output_kind(output: &Path) -> Result<OutputKind, String> {
    let extension = output
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "svg" => Ok(OutputKind::Svg),
        "pdf" => Ok(OutputKind::Pdf),
        "eps" => Ok(OutputKind::Eps),
        _ => Err("Choose an SVG, PDF or EPS output filename".into()),
    }
}

pub fn write_vector(source: &Path, output: &Path, svg: &str) -> Result<(), String> {
    if crate::same_file(source, output) {
        return Err("Input and output must be different files".into());
    }
    let bytes = match output_kind(output)? {
        OutputKind::Svg => return crate::write_svg(source, output, svg),
        OutputKind::Pdf => crate::pdf_eps::to_pdf(svg)?,
        OutputKind::Eps => crate::pdf_eps::to_eps(svg)?,
    };
    // Finish conversion before touching the selected output file, and never
    // truncate it in place.
    crate::write_replacing(output, &bytes)
}

/// `svg` as it is saved under the improved defaults, drawing the same in
/// fewer bytes: every number of its path data written as short as it reads
/// the same (the engine writes two decimals everywhere, `100.00`, `72.50`,
/// `-0.00`) and no `opacity="1.00"`, the default, on every path (5% of a
/// photograph's file). Spaces and command letters stay, so every reader of
/// the engine's layout reads it.
pub fn compact_svg(svg: &str) -> String {
    let svg = svg.replace(" opacity=\"1.00\"", "");
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg.as_str();
    while let Some(at) = rest.find(" d=\"") {
        let start = at + 4;
        let Some(length) = rest[start..].find('"') else {
            break;
        };
        out.push_str(&rest[..start]);
        for (i, token) in rest[start..start + length].split(' ').enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(short(token));
        }
        rest = &rest[start + length..];
    }
    out.push_str(rest);
    out
}

/// A decimal number without its trailing zeros (and point), `-0` as `0`;
/// anything else as it is.
fn short(token: &str) -> &str {
    if !token.contains('.') || token.parse::<f64>().is_err() {
        return token;
    }
    match token.trim_end_matches('0').trim_end_matches('.') {
        "-0" | "" | "-" => "0",
        short => short,
    }
}

#[cfg(test)]
mod tests {
    use super::compact_svg;

    #[test]
    fn path_numbers_lose_their_trailing_zeros_and_paths_their_default_opacity() {
        let svg = "<svg width=\"40pt\" viewBox=\"0 0 40 40\"><path fill=\"#ffffff\" opacity=\"1.00\" d=\" M 0.00 40.00 L 12.50 -0.00 C 1.25 3.10 100.00 -2.50 7.05 9.00 Z\" /><path fill=\"none\" stroke-width=\"1.00\" d=\" M 10.00 0.50 Z\" /></svg>";
        assert_eq!(
            compact_svg(svg),
            "<svg width=\"40pt\" viewBox=\"0 0 40 40\"><path fill=\"#ffffff\" d=\" M 0 40 L 12.5 0 C 1.25 3.1 100 -2.5 7.05 9 Z\" /><path fill=\"none\" stroke-width=\"1.00\" d=\" M 10 0.5 Z\" /></svg>"
        );
    }
}
