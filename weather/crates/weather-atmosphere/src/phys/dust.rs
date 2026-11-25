//! Dust components of the column physics, two classes (fine `m`, coarse `m2`, mixing ratios in
//! kg/kg): its effect on the sunlight, its lifting by the wind, its settling and deposition.
//!
//!  * Lifting where the surface stress rho u*^2 exceeds a threshold in pascals: saltation flux of
//!    Kawamura / White, dust flux = alpha x saltation flux (Marticorena-Bergametti).
//!  * Stokes settling with the Cunningham slip correction and the body's gravity, dry deposition
//!    at the ground.
//!  * Radiative effect: sunlight intercepted by the dust is absorbed in proportion to a per-cell
//!    absorption, the rest is scattered (half still reaches the ground): dark dust heats the air,
//!    white dust mostly shades the ground.

use super::{ColumnCtx, ColumnProcess};
use std::any::Any;

/// The two dust classes: grain sizes, density and the air properties their fall depends on.
#[derive(Clone, Debug)]
pub struct DustClasses {
    /// Grain diameters (m) and density (kg/m^3).
    pub d: [f64; 2],
    pub rho_p: f64,
    /// Dynamic viscosity of the air (Pa s) and mean free path of the molecules (m).
    pub mu: f64,
    pub mfp: f64,
    /// Extinction efficiencies (visible).
    pub qext: [f64; 2],
}

impl DustClasses {
    /// Stokes settling velocity (m/s) with the Cunningham slip correction (Davies coefficients).
    pub fn settling_velocity(&self, d: f64, g: f64) -> f64 {
        let cc = 1.0 + 2.0 * self.mfp / d * (1.257 + 0.400 * (-0.55 * d / self.mfp).exp());
        self.rho_p * g * d * d * cc / (18.0 * self.mu)
    }
    /// Mass extinction coefficient (m^2/kg) of class c: tau = (3/2) Qext M / (rho_p d).
    pub fn kext(&self, c: usize) -> f64 {
        1.5 * self.qext[c] / (self.rho_p * self.d[c])
    }
}

/// Sunlight intercepted by the dust: absorbed (heats the air, layer by layer in proportion to the
/// dust) or scattered (half goes on down to the ground, half back to space).
#[derive(Clone, Debug)]
pub struct DustRadiation {
    /// Fraction of the intercepted sunlight that the dust of each cell absorbs.
    pub absorb: Vec<f64>,
}

impl ColumnProcess for DustRadiation {
    fn name(&self) -> &'static str {
        "dust-radiation"
    }

    fn heat(&self, c: &mut ColumnCtx) {
        if !(c.dust_on && c.toa > 0.0 && c.tau_c > 0.0) {
            return;
        }
        let mu = c.cosz.max(0.05);
        let tr = (-c.tau_c / mu).exp();
        let inter = c.toa * (1.0 - tr);
        let a = self.absorb[c.i];
        c.sw_down = c.toa * tr + 0.5 * inter * (1.0 - a);
        c.sw_air = inter * a;
        if c.sw_air > 0.0 {
            let (g, cp) = (c.c.g, c.c.cp);
            for k in 0..c.l {
                c.dtk[k] += g * c.sw_air * (c.tau_k[k] / c.tau_c) * c.dt / (cp * c.col.dp[k]);
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Lifting by the wind: above a threshold surface stress, a saltation flux Q and a dust flux
/// alpha Q into the lowest layer, split between the classes.
#[derive(Clone, Debug)]
pub struct DustLifting {
    /// Threshold on the surface stress (Pa).
    pub tau_t: f64,
    /// Saltation-flux constant (Kawamura / White).
    pub c_k: f64,
    /// Sandblasting efficiency: dust flux / saltation flux (1/m). Measured values span 1e-5 to
    /// 1e-2; with dust everywhere they lift far too much (the haze runs away), so the value is
    /// chosen for the haze wanted.
    pub alpha: f64,
    /// Fraction of the lifted dust in the fine class.
    pub fine_frac: f64,
}

impl ColumnProcess for DustLifting {
    fn name(&self) -> &'static str {
        "dust-lifting"
    }

    fn sources(&self, c: &mut ColumnCtx) {
        let mut lift = 0.0;
        if c.dust_on && c.tau_s > self.tau_t {
            let (g, kb, rho_s, ustar, dt) = (c.c.g, c.kb, c.rho_s, c.ustar, c.dt);
            let ut = (self.tau_t / rho_s).sqrt();
            let r = ut / ustar;
            let q = self.c_k * rho_s / g * ustar.powi(3) * (1.0 - r * r) * (1.0 + r);
            lift = self.alpha * q;
            c.m[kb] += g * lift * dt * self.fine_frac;
            c.m2[kb] += g * lift * dt * (1.0 - self.fine_frac);
        }
        c.lift = lift;
        c.bud[0] = lift * c.dt;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Settling of both classes (top down, one layer per step, exact exponential, positive) and dry
/// deposition at the ground (settling plus a turbulent part, a multiple of u*).
#[derive(Clone, Debug)]
pub struct DustSettling {
    /// Settling velocities (m/s) of the two classes.
    pub vs: [f64; 2],
    /// Turbulent part of the dry-deposition velocity at the ground, as a multiple of u*.
    pub dep_turb: f64,
}

impl ColumnProcess for DustSettling {
    fn name(&self) -> &'static str {
        "dust-settling"
    }

    fn sinks(&self, c: &mut ColumnCtx) {
        let mut dep = 0.0;
        if c.dust_on {
            let (g, rd, l, kb, dt) = (c.c.g, c.c.rd, c.l, c.kb, c.dt);
            for cl in 0..2 {
                let mm: &mut [f64] = if cl == 0 { &mut *c.m } else { &mut *c.m2 };
                let mut outs = [0.0f64; 16];
                for k in 0..l {
                    let rho = c.col.pf[k] / (rd * c.tk[k]);
                    let vel = if k == kb { self.vs[cl] + self.dep_turb * c.ustar } else { self.vs[cl] };
                    outs[k] = mm[k].max(0.0) * (1.0 - (-vel * rho * g * dt / c.col.dp[k]).exp());
                }
                for k in 0..l {
                    mm[k] -= outs[k];
                    if k < kb {
                        mm[k + 1] += outs[k];
                    }
                }
                dep += outs[kb] / g;
            }
        }
        c.bud[1] = dep;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
