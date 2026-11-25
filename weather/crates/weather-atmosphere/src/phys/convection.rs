//! Dry convective adjustment (component of the column physics): an unstable column (potential
//! temperature decreasing upward) is mixed to neutral, potential temperature and dust together,
//! conserving their masses. This is the afternoon mixing that carries the ground's heat and dust
//! upward; a hydrostatic model cannot overturn the air without it.

use super::{ColumnCtx, ColumnProcess};
use std::any::Any;

#[derive(Clone, Debug, Default)]
pub struct DryConvection;

impl ColumnProcess for DryConvection {
    fn name(&self) -> &'static str {
        "dry-convection"
    }

    fn adjust(&self, c: &mut ColumnCtx) {
        convective_adjust(&mut c.theta, c.m, c.m2, &c.col.dp, c.l, c.dust_on);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Pool-adjacent-violators, from the ground up: a layer warmer (in potential temperature) than the
/// one above is merged with it into a slab of uniform potential temperature (mass-weighted) and
/// uniform dust mixing ratios. Conserves sum dp theta and the dust.
pub fn convective_adjust(theta: &mut [f64; 16], m: &mut [f64], m2: &mut [f64], dp: &[f64; 16], l: usize, dust: bool) {
    // Slabs from the ground up: (lowest layer index, highest layer index, mass, theta mass, M, M2).
    let mut slabs: Vec<(usize, usize, f64, f64, f64, f64)> = Vec::with_capacity(l);
    let mut merged = false;
    for k in (0..l).rev() {
        slabs.push((k, k, dp[k], dp[k] * theta[k], m[k], m2[k]));
        while slabs.len() >= 2 {
            let up = slabs[slabs.len() - 1];
            let lo = slabs[slabs.len() - 2];
            if lo.3 / lo.2 > up.3 / up.2 {
                slabs.pop();
                let last = slabs.len() - 1;
                slabs[last] = (lo.0, up.1, lo.2 + up.2, lo.3 + up.3, lo.4 + up.4, lo.5 + up.5);
                merged = true;
            } else {
                break;
            }
        }
    }
    if !merged {
        return;
    }
    for s in &slabs {
        if s.0 == s.1 {
            continue;
        }
        let th = s.3 / s.2;
        for k in s.1..=s.0 {
            theta[k] = th;
            if dust {
                m[k] = dp[k] * s.4 / s.2;
                m2[k] = dp[k] * s.5 / s.2;
            }
        }
    }
}
