//! The reference LAPACK `dgesv` the original links (0x5bbd30, reached from
//! the fitting's solver wrapper 0x49d270) on the small column-major systems
//! the curve fits build: `dgetf2` (partial pivoting, the pivot's reciprocal
//! multiplied into the column, the rank-one update), `dlaswp` and the two
//! `dtrsm` sweeps of `dgetrs`. Every operation is performed in the reference
//! order so the results agree with the original to the last bit.

/// Solves `a * x = b` in place for one right-hand side. `a` is `n` by `n`
/// column-major (`a[i + j * n]` is row `i`, column `j`); the solution
/// replaces `b`. Returns the LAPACK `info` value: 0, or the first pivot
/// column that was exactly zero.
pub fn dgesv(n: usize, a: &mut [f64], b: &mut [f64]) -> usize {
    debug_assert!(a.len() == n * n && b.len() == n);
    let mut ipiv = vec![0usize; n];
    let mut info = 0;
    // dgetf2.
    for j in 0..n {
        // idamax over column j from row j: the first largest magnitude.
        let mut jp = j;
        let mut dmax = a[j + j * n].abs();
        for i in j + 1..n {
            let v = a[i + j * n].abs();
            if v > dmax {
                dmax = v;
                jp = i;
            }
        }
        ipiv[j] = jp;
        if a[jp + j * n] != 0.0 {
            if jp != j {
                // dswap of the whole rows.
                for k in 0..n {
                    a.swap(j + k * n, jp + k * n);
                }
            }
            if j + 1 < n {
                // dscal by the reciprocal of the pivot.
                let reciprocal = 1.0 / a[j + j * n];
                for i in j + 1..n {
                    a[i + j * n] *= reciprocal;
                }
            }
        } else if info == 0 {
            info = j + 1;
        }
        if j + 1 < n {
            // dger with alpha -1: column by column, temp = -y(j).
            for k in j + 1..n {
                let y = a[j + k * n];
                if y != 0.0 {
                    let temp = -y;
                    for i in j + 1..n {
                        a[i + k * n] += a[i + j * n] * temp;
                    }
                }
            }
        }
    }
    // dgetrs: dlaswp on b, then L (unit) and U (non-unit) triangular solves.
    for (i, &ip) in ipiv.iter().enumerate().take(n) {
        if ip != i {
            b.swap(i, ip);
        }
    }
    for k in 0..n {
        if b[k] != 0.0 {
            for i in k + 1..n {
                b[i] -= b[k] * a[i + k * n];
            }
        }
    }
    for k in (0..n).rev() {
        if b[k] != 0.0 {
            b[k] /= a[k + k * n];
            for i in 0..k {
                b[i] -= b[k] * a[i + k * n];
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_a_pivoting_system_and_reports_singular_columns() {
        // Column-major [[0, 2], [1, 0]]: the first column needs a swap.
        let mut a = vec![0.0, 1.0, 2.0, 0.0];
        let mut b = vec![4.0, 3.0];
        assert_eq!(dgesv(2, &mut a, &mut b), 0);
        assert_eq!(b, vec![3.0, 2.0]);
        let mut a = vec![0.0, 0.0, 1.0, 1.0];
        let mut b = vec![1.0, 1.0];
        assert_eq!(dgesv(2, &mut a, &mut b), 1);
    }

    #[test]
    fn four_by_four_matches_direct_elimination() {
        let mut a = vec![
            4.0, 1.0, 0.0, 0.5, 1.0, 3.0, 0.2, 0.0, 0.0, 0.2, 5.0, 1.0, 0.5, 0.0, 1.0, 6.0,
        ];
        let mut b = vec![1.0, 2.0, 3.0, 4.0];
        let copy = a.clone();
        assert_eq!(dgesv(4, &mut a, &mut b), 0);
        for i in 0..4 {
            let row: f64 = (0..4).map(|j| copy[i + j * 4] * b[j]).sum();
            assert!((row - (i + 1) as f64).abs() < 1e-12);
        }
    }
}
