//! The colour model 0x4a2520 (the object at engine+0xa58, constructed by
//! 0x4a2350): the mixture of up to nine region colours that best explains
//! one pixel. Two regions project the pixel colour onto the segment
//! between the region colours (single precision, with a pull towards the
//! middle that fades with the colour distance); more regions solve a
//! non-negative least squares with a sum-to-one constraint over the first
//! three channels (0x4a1f20 builds the Gram matrix with a ridge and its
//! Cholesky factor, 0x4a2040 runs the active-set loop).
//!
//! The neighbourhood (which regions take part) is the caller's: the contour
//! smoother maps labels to contour regions, the sub-pixel segmenter uses
//! the labels themselves with a substitution. `ids` and `n` are set before
//! [`ColourModel::evaluate`] runs; `weights` and `out` are its results.

/// 1/255 as the original's single-precision constant (0x8dc494).
const INV255: f32 = 0.003921568859368563;

pub struct ColourModel {
    pub n: usize,
    /// +0x3c: the regions of the neighbourhood, the pixel's own first.
    pub ids: [i32; 9],
    /// +0x60: the weight of each region.
    pub weights: [f64; 9],
    /// +0xd8: three channels per region.
    colors: [f64; 27],
    /// +0x1b0: the Gram matrix, then its Cholesky factor (stride 9).
    gram: [f64; 81],
    /// +0x480: the constraint rows (the first all ones).
    constraints: [f64; 81],
    /// +0x708, +0x990: the solved rows and the Schur complement.
    y: [f64; 81],
    schur: [f64; 81],
    /// +0x438, +0xc18, +0xcf0.
    b: [f64; 9],
    lambda: [f64; 9],
    r: [f64; 9],
    /// +0xd38.
    active: [u8; 9],
    /// +0xd48: the pixel colour, then the model colour (three channels).
    pub out: [f64; 4],
}

impl Default for ColourModel {
    fn default() -> Self {
        Self::new()
    }
}

impl ColourModel {
    /// 0x4a2350.
    pub fn new() -> ColourModel {
        let mut constraints = [0.0; 81];
        for c in constraints.iter_mut().take(9) {
            *c = 1.0;
        }
        ColourModel {
            n: 0,
            ids: [0; 9],
            weights: [0.0; 9],
            colors: [0.0; 27],
            gram: [0.0; 81],
            constraints,
            y: [0.0; 81],
            schur: [0.0; 81],
            b: [0.0; 9],
            lambda: [0.0; 9],
            r: [0.0; 9],
            active: [0; 9],
            out: [0.0; 4],
        }
    }

    /// 0x4a1f20: the Gram matrix of the region colours with a ridge, and
    /// its Cholesky factor.
    fn prepare(&mut self, colour_of: &dyn Fn(i32) -> [f32; 4]) {
        let n = self.n;
        for i in 0..n {
            let c = colour_of(self.ids[i]);
            self.colors[3 * i] = c[0] as f64;
            self.colors[3 * i + 1] = c[1] as f64;
            self.colors[3 * i + 2] = c[2] as f64;
        }
        for i in 0..n {
            for j in 0..n {
                let mut sum = 0.0f64;
                for l in 0..3 {
                    sum += self.colors[3 * i + l] * self.colors[3 * j + l];
                }
                self.gram[i + 9 * j] = 1.0 * sum;
            }
        }
        for i in 0..n {
            self.gram[i + 9 * i] += 0.01;
        }
        for k in 0..n {
            if !cholesky_column(&mut self.gram, k) {
                break;
            }
        }
    }

    /// 0x4a2040: the weights of the regions, non-negative and summing to
    /// one, closest to the pixel colour in the Gram metric (an active-set
    /// loop over the sum constraint and the regions driven to zero).
    fn solve(&mut self) {
        let n = self.n;
        for j in 0..n {
            let mut sum = 0.0f64;
            for l in 0..3 {
                sum += self.colors[3 * j + l] * self.out[l];
            }
            self.b[j] = 1.0 * sum;
        }
        self.active = [0; 9];
        let mut m = 1usize;
        loop {
            for r in 0..m {
                let row: [f64; 9] = self.constraints[9 * r..9 * r + 9].try_into().unwrap();
                let mut out = [0.0f64; 9];
                cholesky_solve(&self.gram, &mut out, &row, n);
                self.y[9 * r..9 * r + 9].copy_from_slice(&out);
            }
            for i in 0..m {
                for j in 0..m {
                    let mut sum = 0.0f64;
                    for l in 0..n {
                        sum += self.constraints[9 * i + l] * self.y[9 * j + l];
                    }
                    self.schur[i + 9 * j] = 1.0 * sum;
                }
            }
            for j in 0..m {
                let mut sum = 0.0f64;
                for l in 0..n {
                    sum += self.y[9 * j + l] * self.b[l];
                }
                self.r[j] = 1.0 * sum;
            }
            self.r[0] -= 1.0;
            for k in 0..m {
                if !cholesky_column(&mut self.schur, k) {
                    break;
                }
            }
            let r = self.r;
            let mut lambda = [0.0f64; 9];
            cholesky_solve(&self.schur, &mut lambda, &r, m);
            self.lambda = lambda;
            let mut w = [0.0f64; 9];
            w[..n].copy_from_slice(&self.b[..n]);
            for j in 0..m {
                let scaled = -self.lambda[j];
                if scaled != 0.0 {
                    for (wl, &cv) in w[..n]
                        .iter_mut()
                        .zip(self.constraints[9 * j..9 * j + n].iter())
                    {
                        *wl += scaled * cv;
                    }
                }
            }
            let mut solved = [0.0f64; 9];
            cholesky_solve(&self.gram, &mut solved, &w, n);
            self.weights = solved;
            let mut min = 0.0f64;
            let mut at: i32 = -1;
            for i in 0..n {
                let v = self.weights[i];
                if !(min <= v) && self.active[i] == 0 {
                    at = i as i32;
                    min = v;
                }
            }
            if at < 0 {
                break;
            }
            let i = at as usize;
            if -0.0001 <= self.weights[i] {
                break;
            }
            for l in 0..n {
                self.constraints[9 * m + l] = 0.0;
            }
            self.constraints[9 * m + i] = 1.0;
            self.active[i] = 1;
            m += 1;
            if m > 8 {
                break;
            }
        }
        let mut out = [0.0f64; 3];
        for i in 0..n {
            let scaled = 1.0 * self.weights[i];
            if scaled != 0.0 {
                for (o, &cv) in out.iter_mut().zip(self.colors[3 * i..3 * i + 3].iter()) {
                    *o += scaled * cv;
                }
            }
        }
        self.out[..3].copy_from_slice(&out);
    }

    /// 0x4a2520 once `ids` and `n` hold the neighbourhood: the model colour
    /// at the pixel (four single-precision channels) and the weight of the
    /// pixel's own region (1 with fewer than two regions).
    pub fn evaluate(
        &mut self,
        px: [u8; 4],
        colour_of: &dyn Fn(i32) -> [f32; 4],
    ) -> ([f32; 4], f64) {
        let n = self.n;
        if n <= 1 {
            let out = if self.ids[0] >= 0 {
                colour_of(self.ids[0])
            } else {
                [0.0; 4]
            };
            return (out, 1.0);
        }
        if n == 2 {
            let c0 = colour_of(self.ids[0]);
            let c1 = colour_of(self.ids[1]);
            let mut s = [0.0f32; 4];
            for ch in 0..4 {
                s[ch] = (px[ch] as f32) * INV255;
            }
            let mut d = [0.0f32; 4];
            for ch in 0..4 {
                d[ch] = c1[ch] - c0[ch];
            }
            let dd = ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]) + d[3] * d[3];
            if dd as f64 > 0.0 {
                let mut e = [0.0f32; 4];
                for ch in 0..4 {
                    e[ch] = s[ch] - c0[ch];
                }
                let de = ((d[0] * e[0] + d[1] * e[1]) + d[2] * e[2]) + d[3] * e[3];
                let t = de as f64 / dd as f64;
                let q = 0.01 / (dd as f64 + 0.02);
                let t = (t + ((1.0 - (t + t)) * q)).clamp(0.0, 1.0);
                self.weights[0] = 1.0 - t;
                self.weights[1] = t;
                let tf = t as f32;
                let out = [
                    tf * d[0] + c0[0],
                    c0[1] + tf * d[1],
                    c0[2] + tf * d[2],
                    c0[3] + tf * d[3],
                ];
                return (out, 1.0 - t);
            }
            self.weights[0] = 0.5;
            self.weights[1] = 0.5;
            return (c0, 0.5);
        }
        self.prepare(colour_of);
        // The original keeps the first two channels' products as doubles
        // and stores the other two through floats first.
        self.out[0] = (px[0] as f64) * (INV255 as f64);
        self.out[1] = (px[1] as f64) * (INV255 as f64);
        self.out[2] = ((px[2] as f32) * INV255) as f64;
        self.out[3] = ((px[3] as f32) * INV255) as f64;
        self.solve();
        let out = [
            self.out[0] as f32,
            self.out[1] as f32,
            self.out[2] as f32,
            self.out[3] as f32,
        ];
        (out, self.weights[0])
    }
}

/// 0x4a1980: one column of the Cholesky factorisation in place (stride 9,
/// the factor in the lower triangle of the column-major view).
fn cholesky_column(a: &mut [f64; 81], k: usize) -> bool {
    for i in 0..k {
        let mut v = a[k + 9 * i];
        for j in 0..i {
            v -= a[i + 9 * j] * a[k + 9 * j];
        }
        a[k + 9 * i] = v / a[i + 9 * i];
    }
    let mut d = a[k + 9 * k];
    for j in 0..k {
        let v = a[k + 9 * j];
        d -= v * v;
    }
    if !(0.0 < d) {
        a[k + 9 * k] = d;
        return false;
    }
    a[k + 9 * k] = d.sqrt();
    true
}

/// 0x4a1b60: solve with the factor of `cholesky_column` (forward with the
/// factor's column, then back with its row, both dividing by the
/// diagonal), `n` unknowns. The lower triangle still holds the matrix
/// itself and is never read.
fn cholesky_solve(lu: &[f64; 81], x: &mut [f64], b: &[f64], n: usize) {
    for i in 0..n {
        let mut sum = b[i];
        for k in (0..i).rev() {
            sum -= lu[k * 9 + i] * x[k];
        }
        x[i] = sum / lu[i * 9 + i];
    }
    for i in (0..n).rev() {
        let mut sum = x[i];
        for j in i + 1..n {
            sum -= lu[i * 9 + j] * x[j];
        }
        x[i] = sum / lu[i * 9 + i];
    }
}
