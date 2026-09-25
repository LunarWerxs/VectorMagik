use crate::export::{vector_bytes, ExportOptions, OutputKind};

/// A document shaped like the engine's for dithered pixel art: a white
/// field, a 2 px checker of squares over 32 x 32 px in one colour's path,
/// and three lone squares of the same colour that repeat nothing.
fn dithered() -> String {
    let mut squares = String::new();
    for y in 4..36 {
        for x in 4..36 {
            if (x + y) % 2 == 0 {
                squares.push_str(&format!(
                    " M {} {} L {x} {} L {x} {y} L {} {y} L {} {} Z",
                    x + 1,
                    y + 1,
                    y + 1,
                    x + 1,
                    x + 1,
                    y + 1
                ));
            }
        }
    }
    for (x, y) in [(0, 0), (38, 1), (2, 39)] {
        squares.push_str(&format!(
            " M {} {} L {x} {} L {x} {y} L {} {y} L {} {} Z",
            x + 1,
            y + 1,
            y + 1,
            x + 1,
            x + 1,
            y + 1
        ));
    }
    format!(
        "<svg width=\"40\" height=\"40\" viewBox=\"0 0 40 40\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n\
         <g id=\"#ffffffff\">\n<path fill=\"#ffffff\" d=\" M 0 0 L 40 0 L 40 40 L 0 40 Z\" />\n</g>\n\
         <g id=\"#1428a0ff\">\n<path fill=\"#1428a0\" d=\"{squares}\" />\n</g>\n</svg>\n"
    )
}

#[cfg(feature = "render")]
#[test]
fn a_dithered_area_saves_as_one_patterned_rectangle_that_draws_and_reopens_the_same() {
    let document = dithered();
    let saved = String::from_utf8(
        vector_bytes(OutputKind::Svg, &document, &ExportOptions::default()).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.matches("<pattern ").count(), 1, "{saved}");
    assert!(
        saved.len() * 10 < document.len(),
        "{} of {} bytes",
        saved.len(),
        document.len()
    );
    let before = crate::preview_pixels(&document).unwrap();
    let after = crate::preview_pixels(&saved).unwrap();
    assert_eq!(
        crate::changed_fraction(&before, &after),
        0.,
        "the saved SVG draws other pixels"
    );
    // Opened again ("Convert it"), the pattern is its squares once more.
    let reopened = crate::svg_import::to_svg(saved.as_bytes()).unwrap();
    assert!(reopened.skipped.is_empty(), "{:?}", reopened.skipped);
    let again = crate::preview_pixels(&reopened.svg).unwrap();
    assert_eq!(
        crate::changed_fraction(&before, &again),
        0.,
        "the reopened SVG draws other pixels"
    );
}

#[test]
fn a_dithered_area_saves_to_pdf_and_eps_as_a_tiling_pattern() {
    let document = dithered();
    let pdf = vector_bytes(OutputKind::Pdf, &document, &ExportOptions::default()).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert_eq!(text.matches("/PatternType 1").count(), 1);
    assert!(text.contains("/Pattern << /P0 6 0 R >>"), "{text}");
    let eps = vector_bytes(OutputKind::Eps, &document, &ExportOptions::default()).unwrap();
    let eps_text = String::from_utf8_lossy(&eps);
    assert_eq!(eps_text.matches("makepattern").count(), 1);
    // The field and the three lone squares are still drawn as paths.
    assert_eq!(eps_text.matches("\nh\n").count(), 4, "{eps_text}");
    // Opened again ("Convert it"), each draws its squares, not one colour.
    #[cfg(feature = "render")]
    {
        let before = crate::preview_pixels(&document).unwrap();
        for (kind, reopened) in [
            ("PDF", crate::pdf_import::to_svg(&pdf, 0).unwrap()),
            ("EPS", crate::eps_import::to_svg(&eps).unwrap()),
        ] {
            assert!(
                reopened.skipped.is_empty(),
                "{kind}: {:?}",
                reopened.skipped
            );
            let again = crate::preview_pixels(&reopened.svg).unwrap();
            assert_eq!(
                crate::changed_fraction(&before, &again),
                0.,
                "the reopened {kind} draws other pixels"
            );
        }
    }
}

#[test]
fn squares_that_repeat_nothing_are_left_as_they_are() {
    // A diagonal line of squares repeats no tile across a rectangle.
    let mut squares = String::new();
    for i in 0..40 {
        squares.push_str(&format!(
            " M {i} {i} L {} {i} L {} {} L {i} {} Z",
            i + 1,
            i + 1,
            i + 1,
            i + 1
        ));
    }
    let document = format!(
        "<svg width=\"40\" height=\"40\" viewBox=\"0 0 40 40\" xmlns=\"http://www.w3.org/2000/svg\">\n\
         <path fill=\"#000000\" d=\"{squares}\" />\n</svg>\n"
    );
    assert_eq!(super::patterned_svg(&document), document);
}
