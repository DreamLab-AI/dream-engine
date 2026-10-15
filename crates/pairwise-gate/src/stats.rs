//! Dependency-free statistics: exact binomial tail and Wilson lower bound.

use core::f64::consts::LN_2;

/// Exact one-sided binomial tail `P(X >= wins)` for `X ~ Binomial(n, 1/2)`.
///
/// This is the p-value of the one-sided sign test with null hypothesis
/// `p = 0.5` over `n` non-tied pairs of which `wins` favour the candidate.
///
/// The tail is summed in log space (Neumaier-compensated running log-pmf and an
/// online log-sum-exp), so it is stable for `n` in the tens of thousands and
/// never produces `NaN`. A tail smaller than the smallest positive `f64`
/// (for example all 10 000 of 10 000) is returned as exactly `0.0`.
///
/// Edge cases: `wins == 0` gives `1.0` (including `n == 0`); `wins > n` gives
/// `0.0` (an empty tail).
///
/// # Examples
///
/// ```
/// // 9 wins in 10 non-tied pairs: (C(10,9) + C(10,10)) / 2^10 = 11/1024.
/// let p = pairwise_gate::sign_test_p_value(9, 10);
/// assert!((p - 11.0 / 1024.0).abs() < 1e-15);
/// ```
#[must_use]
pub fn sign_test_p_value(wins: usize, n: usize) -> f64 {
    if wins == 0 {
        return 1.0;
    }
    if wins > n {
        return 0.0;
    }
    let nf = n as f64;
    // Compensated running value of ln pmf(k) = ln C(n,k) - n ln 2.
    let mut sum = -nf * LN_2;
    let mut comp = 0.0_f64;
    // Online log-sum-exp of ln pmf(k) for k >= wins.
    let mut max = f64::NEG_INFINITY;
    let mut acc = 0.0_f64;
    for k in 0..=n {
        if k >= wins {
            let x = sum + comp;
            if x > max {
                acc = acc * (max - x).exp() + 1.0;
                max = x;
            } else {
                acc += (x - max).exp();
            }
        }
        if k < n {
            let step = ((n - k) as f64).ln() - ((k + 1) as f64).ln();
            let t = sum + step;
            if sum.abs() >= step.abs() {
                comp += (sum - t) + step;
            } else {
                comp += (step - t) + sum;
            }
            sum = t;
        }
    }
    let p = (max + acc.ln()).exp();
    if p.is_nan() {
        0.0
    } else {
        p.min(1.0)
    }
}

/// Wilson score lower bound for a binomial proportion `wins / n` at normal
/// quantile `z` (for example `1.96` for a two-sided 95% interval, `1.645` for a
/// one-sided 95% bound).
///
/// Returns `0.0` when `n == 0`. The result is clamped to `[0, 1]`.
///
/// # Examples
///
/// ```
/// // 8 of 10 at z = 1.96: the textbook value 0.4902.
/// let lo = pairwise_gate::wilson_lower(8, 10, 1.96);
/// assert!((lo - 0.4902).abs() < 1e-4);
/// ```
#[must_use]
pub fn wilson_lower(wins: usize, n: usize, z: f64) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let nf = n as f64;
    let p = (wins.min(n)) as f64 / nf;
    let z2 = z * z;
    let centre = p + z2 / (2.0 * nf);
    let spread = z * (p * (1.0 - p) / nf + z2 / (4.0 * nf * nf)).sqrt();
    let lo = (centre - spread) / (1.0 + z2 / nf);
    if lo.is_nan() {
        0.0
    } else {
        lo.clamp(0.0, 1.0)
    }
}
