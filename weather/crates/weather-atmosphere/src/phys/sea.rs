//! The permanent "dust sea" (component of the column physics): the fine dust is relaxed toward a
//! target mixing ratio between two fixed absolute altitudes (over the reference sphere), filled
//! quickly when it lacks, emptied slowly when in excess, and the air above the top is cleared
//! slowly. Each layer receives the relaxation in proportion to the part of its thickness that lies
//! in the band.

use super::{overlap, ColumnCtx, ColumnProcess};
use std::any::Any;

#[derive(Clone, Debug)]
pub struct DustSea {
    /// Base and top of the permanent dust layer (m, absolute altitude over the reference sphere).
    pub z_base: f64,
    pub z_top: f64,
    /// Target mixing ratio of the fine dust inside the layer (kg/kg).
    pub q_target: f64,
    /// Relaxation times (s): filling up to the target, removing an excess, clearing above the top.
    pub tau_fill: f64,
    pub tau_excess: f64,
    pub tau_clear: f64,
    /// Under the base: the fine dust above `below_share` x `q_target` is removed in `tau_below`
    /// (s); 0 leaves the air under the base alone. Only the fine class: the sand the wind lifts
    /// (walls, devils) is the coarse one.
    pub below_share: f64,
    pub tau_below: f64,
}

impl ColumnProcess for DustSea {
    fn name(&self) -> &'static str {
        "dust-sea"
    }

    fn sinks(&self, c: &mut ColumnCtx) {
        let mut sea_add = 0.0;
        if c.dust_on {
            let (g, l, dt) = (c.c.g, c.l, c.dt);
            let (zh, zf) = (&c.zh, &c.zf);
            for k in 0..l {
                let (lo, hi) = (zh[k + 1], if k > 0 { zh[k] } else { zh[1] + 2.0 * (zf[0] - zh[1]) });
                let fin = overlap(lo, hi, self.z_base, self.z_top);
                let above = overlap(lo, hi, self.z_top, f64::INFINITY);
                let q = c.m[k] / c.col.dp[k];
                let mut dq = 0.0;
                if fin > 0.0 {
                    dq = if q < self.q_target {
                        (self.q_target - q) * (1.0 - (-dt * fin / self.tau_fill).exp())
                    } else {
                        -(q - self.q_target) * (1.0 - (-dt * fin / self.tau_excess).exp())
                    };
                }
                if above > 0.0 {
                    dq -= (q + dq) * (1.0 - (-dt * above / self.tau_clear).exp());
                }
                let below = overlap(lo, hi, f64::NEG_INFINITY, self.z_base);
                if below > 0.0 && self.tau_below > 0.0 {
                    dq -= below_excess(q + dq, self.below_share * self.q_target, dt * below / self.tau_below);
                }
                let dm = dq * c.col.dp[k];
                c.m[k] += dm;
                sea_add += dm / g;
            }
        }
        c.bud[2] = sea_add;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The mixing ratio to take from `q` under the sea's base: its excess over `cap`, relaxed by
/// `steps` relaxation times (dt over tau, already weighted by the layer's share under the base).
fn below_excess(q: f64, cap: f64, steps: f64) -> f64 {
    if q <= cap {
        return 0.0;
    }
    (q - cap) * (1.0 - (-steps).exp())
}

#[cfg(test)]
mod tests {
    use super::below_excess;

    #[test]
    fn under_the_base_only_the_excess_goes() {
        assert_eq!(below_excess(0.05, 0.1, 1.0), 0.0, "under the cap: kept");
        let taken = below_excess(1.0, 0.1, 1.0);
        assert!((taken - 0.9 * (1.0 - (-1.0f64).exp())).abs() < 1e-12);
        assert!(below_excess(1.0, 0.1, 50.0) <= 0.9 + 1e-12, "never below the cap");
    }
}
