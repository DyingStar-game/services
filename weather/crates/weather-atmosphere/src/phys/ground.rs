//! Ground energy balance and the day-night cycle (component of the column physics).
//!
//! Force-restore (Bhumralkar 1975, Deardorff 1978): skin temperature Tg and deep temperature T2,
//!     dTg/dt = C_T G - omega (Tg - T2),   dT2/dt = (Tg - T2) / day,
//!     C_T = sqrt(2 omega) / I  (I = thermal inertia, J m^-2 K^-1 s^-1/2, omega = 2 pi / day),
//!     G = absorbed sunlight + eps_air sigma Ta^4 - sigma Tg^4 - H,
//! solved implicitly (linearised), so it is stable for any step. Ta is the surface air temperature;
//! the down-welling infrared of the air uses an effective emissivity `eps_air` (a grey closure, not
//! a radiation scheme). Sensible heat H = hcoef (Tg - Ta) from the suite's surface layer. The air
//! receives H and the infrared exchange eps sigma (Tg^4 - Ta^4) in its lowest layer.

use super::{ColumnCtx, ColumnProcess, SIGMA_SB};
use std::any::Any;

#[derive(Clone, Debug)]
pub struct Ground {
    /// Albedo of the ground (used where `albedo_cells` is None).
    pub albedo: f64,
    /// Optional albedo per cell (the ground colour map); replaces `albedo` when given.
    pub albedo_cells: Option<Vec<f64>>,
    /// Effective emissivity of the air for the down-welling infrared.
    pub eps_air: f64,
    /// Thermal inertia of the ground (J m^-2 K^-1 s^-1/2, "TIU").
    pub tiu: f64,
    /// Period of the forcing (s): the solar day (the sidereal rotation for a body locked to its star).
    pub day_s: f64,
}

impl ColumnProcess for Ground {
    fn name(&self) -> &'static str {
        "ground"
    }

    fn heat(&self, c: &mut ColumnCtx) {
        let dt = c.dt;
        let omega_d = 2.0 * std::f64::consts::PI / self.day_s;
        let c_t = (2.0 * omega_d).sqrt() / self.tiu;
        let (tg0, ta, hcoef) = (c.tg0, c.ta, c.hcoef);
        let albedo = self.albedo_cells.as_ref().map_or(self.albedo, |a| a[c.i]);
        // Net sunlight absorbed: from the dust radiation when it solved the layer over the ground.
        let swg = match c.sw_ground {
            Some(net) => net,
            None => (1.0 - albedo) * c.sw_down,
        };
        let lw_dn = self.eps_air * SIGMA_SB * ta.powi(4);
        let gflux = swg + lw_dn - SIGMA_SB * tg0.powi(4) - hcoef * (tg0 - ta);
        let dgdt = 4.0 * SIGMA_SB * tg0.powi(3) + hcoef;
        let dtg = dt * (c_t * gflux - omega_d * (tg0 - *c.t2)) / (1.0 + dt * (c_t * dgdt + omega_d));
        let tg1 = tg0 + dtg;
        *c.tg = tg1;
        *c.t2 += dt * (tg1 - *c.t2) / self.day_s;
        let hflux = hcoef * (tg1 - ta);
        // Infrared exchange between the ground and the air, consistent with the ground's closure:
        // the air absorbs eps sigma Tg^4 and sends eps sigma Ta^4 down. Given to the lowest layer
        // (it cools it at night, warms it by day).
        let lw_ex = self.eps_air * SIGMA_SB * (tg1.powi(4) - ta.powi(4));
        c.q_low += hflux;
        c.q_low += lw_ex;
        c.swg = swg;
        c.tg1 = tg1;
        c.dtg = dtg;
        c.hflux = hflux;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
