//! Temperature relaxation of the free atmosphere (a "physics" component evaluated inside the
//! dynamical core's tendencies, since it acts at every Runge-Kutta stage): a trait, so that the
//! Held-Suarez recipe can be replaced (e.g. by a grey radiation scheme) without touching the core.

use crate::body::Seasons;

/// What the core asks a relaxation scheme, per cell and layer.
pub trait Relaxation: Send + Sync + std::fmt::Debug {
    /// Equilibrium temperature (K), temperature relaxation rate (1/s) and Rayleigh friction rate
    /// (1/s) at latitude (sin, cos), sine of the subsolar latitude `sin0`, full-level pressure `p`
    /// (Pa), sigma = p / ps. `lowest` = the layer touching the ground.
    fn terms(&self, sin_lat: f64, cos_lat: f64, sin0: f64, p: f64, sigma: f64, lowest: bool) -> (f64, f64, f64);
    /// Subsolar latitude (radians) at time t (s) used by `terms`.
    fn subsolar_lat(&self, t: f64) -> f64;
    /// Equilibrium temperature without the floor's season (initial state): at latitude sine `sl`,
    /// cosine `cl`, pressure `p`, with the sun over the equator.
    fn initial_temperature(&self, sl: f64, cl: f64, p: f64) -> f64;
}

/// Held & Suarez (1994) with a seasonal subsolar latitude (the forcing of steps 3 and 4).
#[derive(Clone, Copy, Debug)]
pub struct HeldSuarez {
    /// Equilibrium temperature at the ground under the sun (K).
    pub t_max: f64,
    /// Equator-to-pole contrast of the equilibrium temperature (K).
    pub dt_y: f64,
    /// Static-stability parameter (K).
    pub dtheta_z: f64,
    /// Lower bound of the equilibrium temperature (K).
    pub t_min: f64,
    /// Slow (free-atmosphere) relaxation rate (1/s).
    pub k_a: f64,
    /// Fast (boundary-layer, equator) relaxation rate (1/s).
    pub k_s: f64,
    /// Rayleigh friction rate at the ground (1/s).
    pub k_f: f64,
    /// Top of the frictional boundary layer (sigma = p / ps).
    pub sigma_b: f64,
    pub seasons: Seasons,
    /// Reference pressure (Pa) and kappa = R / cp of the air.
    pub p0: f64,
    pub kappa: f64,
    /// True when a ground component heats and cools the lowest layer: only the slow relaxation is
    /// kept there.
    pub slow_lowest_layer: bool,
}

impl Relaxation for HeldSuarez {
    #[inline]
    fn terms(&self, sin_lat: f64, cos_lat: f64, sin0: f64, p: f64, sigma: f64, lowest: bool) -> (f64, f64, f64) {
        let teq = ((self.t_max - self.dt_y * (sin_lat - sin0).powi(2) - self.dtheta_z * (p / self.p0).ln() * cos_lat * cos_lat)
            * (p / self.p0).powf(self.kappa))
        .max(self.t_min);
        let bl = ((sigma - self.sigma_b) / (1.0 - self.sigma_b)).max(0.0);
        let mut kt = self.k_a + (self.k_s - self.k_a) * bl * cos_lat.powi(4);
        if lowest && self.slow_lowest_layer {
            kt = self.k_a;
        }
        let kv = self.k_f * bl;
        (teq, kt, kv)
    }

    fn subsolar_lat(&self, t: f64) -> f64 {
        self.seasons.subsolar_lat(t)
    }

    fn initial_temperature(&self, sl: f64, cl: f64, p: f64) -> f64 {
        ((self.t_max - self.dt_y * sl * sl - self.dtheta_z * (p / self.p0).ln() * cl * cl) * (p / self.p0).powf(self.kappa)).max(self.t_min)
    }
}
