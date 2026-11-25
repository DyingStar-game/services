//! The "lid" (component of the column physics): the equilibrium temperature of the relaxation
//! scheme gets a step at a fixed absolute altitude, (dt_k / 2) tanh((z - z_lid) / (depth / 2)),
//! warmer above and cooler below, over the whole column except the lowest layer, so that nothing
//! above the lid becomes unstable. It acts at the relaxation rate of each level (the relaxation
//! toward the plain equilibrium runs in the dynamics; adding k_T x step shifts its target), and
//! inside the band |z - z_lid| < depth also at the fast rate `rate` (triangular weight). Without a
//! relaxation scheme the lid does nothing.

use super::{ColumnCtx, ColumnProcess};
use std::any::Any;

#[derive(Clone, Debug)]
pub struct Lid {
    /// Altitude of the step (m, absolute) and its thickness (m).
    pub z_bot: f64,
    pub depth: f64,
    /// Temperature step of the shifted equilibrium, warmer above minus cooler below (K).
    pub dt_k: f64,
    /// Fast relaxation rate near the step (1/s).
    pub rate: f64,
}

impl ColumnProcess for Lid {
    fn name(&self) -> &'static str {
        "lid"
    }

    fn adjust(&self, c: &mut ColumnCtx) {
        let Some(h) = c.relax else { return };
        let (dt, kb, zl) = (c.dt, c.kb, self.z_bot);
        let half = 0.5 * self.depth;
        for k in 0..kb {
            let step = 0.5 * self.dt_k * ((c.zf[k] - zl) / half).tanh();
            let pk = c.col.pf[k];
            let (teq, kt, _) = h.terms(c.sin_lat, c.cos_lat, c.sin0, pk, pk / c.ps, false);
            let mut tcur = c.theta[k] * c.pik[k] + dt * kt * step;
            let band = (1.0 - (c.zf[k] - zl).abs() / self.depth).max(0.0);
            if band > 0.0 {
                let a = dt * self.rate * band;
                tcur = (tcur + a * (teq + step)) / (1.0 + a);
            }
            c.theta[k] = tcur / c.pik[k];
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
