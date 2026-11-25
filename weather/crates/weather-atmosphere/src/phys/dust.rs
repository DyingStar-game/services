//! Dust components of the column physics, two classes (fine `m`, coarse `m2`, mixing ratios in
//! kg/kg): its effect on the sunlight, its lifting by the wind, its settling and deposition.
//!
//!  * Lifting where the surface stress rho u*^2 exceeds a threshold in pascals: saltation flux of
//!    Kawamura / White, dust flux = alpha x saltation flux (Marticorena-Bergametti).
//!  * Stokes settling with the Cunningham slip correction and the body's gravity, dry deposition
//!    at the ground.
//!  * Radiative effect, two optics (setting `dust.optics`):
//!    - "colour": sunlight intercepted by the dust is absorbed in proportion to a per-cell
//!      absorption (1 - colour albedo), the rest is scattered (half still reaches the ground): dark
//!      dust heats the air, white dust mostly shades the ground;
//!    - "mie": per-cell single-scattering albedo and asymmetry of each class from Mie theory (see
//!      [`super::optics`]), and a delta-Eddington two-stream for the dust layer over the ground,
//!      multiple reflections included; it also gives the ground its net absorbed sunlight.

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
/// dust) or scattered (colour optics: half goes on down to the ground, half back to space; Mie
/// optics: by the two-stream).
#[derive(Clone, Debug)]
pub struct DustRadiation {
    /// Fraction of the intercepted sunlight that the dust of each cell absorbs (colour optics).
    pub absorb: Vec<f64>,
    /// Mie optics: when given, replaces the colour rule.
    pub mie: Option<MieRadiation>,
}

/// Per-cell Mie optics of the two classes and the ground albedo seen by the two-stream.
#[derive(Clone, Debug)]
pub struct MieRadiation {
    /// Mass extinction coefficients (m^2/kg) of the classes (the suite's, to split the depth).
    pub kext: [f64; 2],
    /// Extinction efficiencies of the classes (Mie, area-weighted over the cells).
    pub qext: [f64; 2],
    /// Single-scattering albedo and asymmetry parameter per class and cell.
    pub omega: [Vec<f64>; 2],
    pub g: [Vec<f64>; 2],
    /// Imaginary index of the dust material per cell (from the colour; for the outputs).
    pub k: Vec<f64>,
    /// Albedo of the ground under the layer, per cell.
    pub ground_albedo: Vec<f64>,
}

impl MieRadiation {
    /// Column optics: depth-weighted single-scattering albedo and scattering-weighted asymmetry of
    /// the mix of the two classes, from their columns (kg/m^2).
    pub fn mix(&self, i: usize, col_fine: f64, col_coarse: f64) -> (f64, f64) {
        let (t0, t1) = (self.kext[0] * col_fine, self.kext[1] * col_coarse);
        let (s0, s1) = (self.omega[0][i] * t0, self.omega[1][i] * t1);
        let t = t0 + t1;
        if t <= 0.0 || s0 + s1 <= 0.0 {
            return (self.omega[0][i], self.g[0][i]);
        }
        ((s0 + s1) / t, (self.g[0][i] * s0 + self.g[1][i] * s1) / (s0 + s1))
    }
}

impl DustRadiation {
    /// The sunlight absorbed by the dust heats each layer in proportion to its optical depth.
    fn heat_layers(&self, c: &mut ColumnCtx) {
        if c.sw_air > 0.0 {
            let (g, cp) = (c.c.g, c.c.cp);
            for k in 0..c.l {
                c.dtk[k] += g * c.sw_air * (c.tau_k[k] / c.tau_c) * c.dt / (cp * c.col.dp[k]);
            }
        }
    }
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
        if let Some(o) = &self.mie {
            let g = c.c.g;
            let (omega, asym) = o.mix(c.i, c.m.iter().sum::<f64>() / g, c.m2.iter().sum::<f64>() / g);
            let ts = super::optics::delta_eddington(c.tau_c, omega, asym, mu, o.ground_albedo[c.i]);
            c.sw_down = c.toa * ts.down;
            c.sw_ground = Some(c.toa * ts.ground);
            c.sw_air = (c.toa * ts.layer).max(0.0);
            self.heat_layers(c);
            return;
        }
        let tr = (-c.tau_c / mu).exp();
        let inter = c.toa * (1.0 - tr);
        let a = self.absorb[c.i];
        c.sw_down = c.toa * tr + 0.5 * inter * (1.0 - a);
        c.sw_air = inter * a;
        self.heat_layers(c);
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
