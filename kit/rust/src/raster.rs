//! The raster the app loads and hands to the engine: RGBA pixels in row order.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub [u8; 4]);

#[derive(Clone, Debug)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgba>,
}

/// The channels of a `#rrggbb` colour.
pub fn hex_rgb(color: &str) -> Option<[u8; 3]> {
    let h = color.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([v(0)?, v(2)?, v(4)?])
}

/// The pixels beside pixel `p` of a `w` by `h` picture (4-connected), left,
/// right, above, below.
pub fn neighbours4(p: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (p % w, p / w);
    [
        (x > 0).then(|| p - 1),
        (x + 1 < w).then(|| p + 1),
        (y > 0).then(|| p - w),
        (y + 1 < h).then(|| p + w),
    ]
    .into_iter()
    .flatten()
}

/// The pixels touching pixel `p` of a `w` by `h` picture at a corner only.
pub fn corner_neighbours(p: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (p % w, p / w);
    [
        (x > 0 && y > 0).then(|| p - w - 1),
        (x + 1 < w && y > 0).then(|| p - w + 1),
        (x > 0 && y + 1 < h).then(|| p + w - 1),
        (x + 1 < w && y + 1 < h).then(|| p + w + 1),
    ]
    .into_iter()
    .flatten()
}
