//! Independent source-size SVG rasterization for local end-to-end verification.
//! Without WIDTH HEIGHT the picture renders at its viewBox size, which for an
//! engine document is the source image's pixel size (its declared width and
//! height are points, 4/3 larger); a document without a viewBox renders at
//! its declared size.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 && args.len() != 5 {
        return Err("Usage: render_svg INPUT.svg OUTPUT.png [WIDTH HEIGHT]".into());
    }
    let source = std::fs::read_to_string(&args[1])?;
    let tree = resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default())?;
    let size = tree.size();
    let (w, h) = if args.len() == 5 {
        (args[3].parse::<u32>()?, args[4].parse::<u32>()?)
    } else {
        let (width, height) = vector_magic_rebuild::view_box_size(&source)
            .unwrap_or((f64::from(size.width()), f64::from(size.height())));
        (width.ceil() as u32, height.ceil() as u32)
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("Invalid raster dimensions")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height()),
        &mut pixmap.as_mut(),
    );
    image::RgbaImage::from_fn(w, h, |x, y| {
        let c = pixmap.pixel(x, y).unwrap().demultiply();
        image::Rgba([c.red(), c.green(), c.blue(), c.alpha()])
    })
    .save(&args[2])?;
    Ok(())
}
