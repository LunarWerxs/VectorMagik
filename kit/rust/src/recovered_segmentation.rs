//! Preprocessing (0x473850, the `Preprocessor` at engine+0x300) recovered
//! from the disassembly, and the segmentation that follows it (0x488d20, in
//! progress).
//!
//! Preprocessing takes the imported BGRA image and leaves the engine's
//! source image at engine+0x8:
//!
//! * the import 0x473c20 premultiplies every pixel's colour channels by its
//!   alpha (`c * a / 255`, truncated), so a fully transparent pixel is all
//!   zero and an opaque one is unchanged;
//! * with `Preprocessor::filter_type` 1 (every preset) the ImageMagick
//!   library's `EnhanceImage` (0x4ae620 in Magick++, the filter at 0x4d6430)
//!   runs `Preprocessor::reduce_noise_order` times (0 for presets 0, 1, 2, 5
//!   and 9; 1 for 4 and 8; 2 for 3 and 7; 3 for 6): every pixel becomes the
//!   weighted mean of the 5x5 window pixels (edge pixels replicated outside
//!   the image) whose colour distance to it is below 255^2 / 25, with the
//!   library's red-mean distance over red, green, blue and opacity (opacity
//!   being 255 minus alpha) and the kernel 5 8 10 8 5 / 8 20 40 20 8 /
//!   10 40 80 40 10 / 8 20 40 20 8 / 5 8 10 8 5; the channel sums are
//!   single precision and the result is `(sum + weight / 2 - 1) / weight`
//!   truncated. Filter types 2 and 3 are selected by no preset and are not
//!   ported;
//! * 0x472cb0 fills the global neighbour tables (`dx`/`dy` of the eight
//!   neighbours and the row offsets) the segmentation and the colour model
//!   walk; the port keeps them as constants.

/// The eight neighbour offsets 0x472cb0 sets at 0xa68fe8 / 0xa69054.
pub const NEIGHBOUR_DX: [i32; 8] = [1, 0, -1, 0, -1, 1, -1, 1];
pub const NEIGHBOUR_DY: [i32; 8] = [0, 1, 0, -1, 1, -1, -1, 1];

/// The library's 5x5 kernel, row by row.
const ENHANCE_WEIGHTS: [f64; 25] = [
    5.0, 8.0, 10.0, 8.0, 5.0, 8.0, 20.0, 40.0, 20.0, 8.0, 10.0, 40.0, 80.0, 40.0, 10.0, 8.0, 20.0,
    40.0, 20.0, 8.0, 5.0, 8.0, 10.0, 8.0, 5.0,
];
const INV255: f64 = 0.00392156862745098;

/// One pass of `EnhanceImage` over a BGRA image. The library's pixel is
/// (red, green, blue, opacity); the distance and the sums follow the
/// listing's operation order.
pub(crate) fn enhance(pixels: &[u8], w: usize, h: usize) -> Vec<u8> {
    let at = |x: i32, y: i32| -> [i32; 4] {
        let sx = x.clamp(0, w as i32 - 1) as usize;
        let sy = y.clamp(0, h as i32 - 1) as usize;
        let p = &pixels[(sy * w + sx) * 4..][..4];
        [p[2] as i32, p[1] as i32, p[0] as i32, 255 - p[3] as i32]
    };
    let mut out = pixels.to_vec();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let c = at(x, y);
            let mut sums = [0.0f32; 4];
            let mut total = 0.0f64;
            let mut k = 0;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let n = at(x + dx, y + dy);
                    let mean = ((n[0] + c[0]) as f64) * 0.5;
                    let d = (n[0] - c[0]) as f64;
                    let mut dist = (512.0 + mean) * d * d * INV255;
                    let d = (n[1] - c[1]) as f64;
                    dist += 4.0 * d * d;
                    let mean = ((n[2] + c[2]) as f64) * 0.5;
                    let d = (n[2] - c[2]) as f64;
                    dist += (767.0 - mean) * d * d * INV255;
                    let mean = ((n[3] + c[3]) as f64) * 0.5;
                    let d = (n[3] - c[3]) as f64;
                    dist += (767.0 - mean) * d * d * INV255;
                    if dist < 2601.0 {
                        let weight = ENHANCE_WEIGHTS[k];
                        for ch in 0..4 {
                            sums[ch] = (sums[ch] as f64 + weight * n[ch] as f64) as f32;
                        }
                        total += weight;
                    }
                    k += 1;
                }
            }
            let half = (0.5 * total) as f32;
            let inverse = 1.0 / total;
            let mut result = [0i32; 4];
            for ch in 0..4 {
                result[ch] = (((half as f64 + sums[ch] as f64) - 1.0) * inverse) as i32;
            }
            let o = &mut out[(y as usize * w + x as usize) * 4..][..4];
            o[0] = result[2] as u8;
            o[1] = result[1] as u8;
            o[2] = result[0] as u8;
            o[3] = (255 - result[3]) as u8;
        }
    }
    out
}

/// 0x473c20 then 0x473850: the engine's source image from the imported BGRA
/// pixels, for a preset's `reduce_noise_order`.
pub fn preprocess(pixels: &[u8], w: usize, h: usize, order: i32) -> Vec<u8> {
    let _span = crate::profile::span("preprocess");
    let mut image = pixels.to_vec();
    for p in image.chunks_mut(4) {
        let a = p[3] as u32;
        for c in p.iter_mut().take(3) {
            *c = ((*c as u32) * a / 255) as u8;
        }
    }
    for _ in 0..order.max(0) {
        image = enhance(&image, w, h);
    }
    image
}

mod sub_pixels;
mod super_pixels;
pub use super_pixels::{segment, Record, Region, Segmentation, SegmenterParams};

#[cfg(test)]
mod tests;
