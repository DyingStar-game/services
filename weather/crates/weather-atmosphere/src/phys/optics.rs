//! From the colour of the ground to the optics of the airborne dust, and the sunlight through the
//! dust layer over the ground.
//!
//!  * **Colour -> absorption of the material** (Hapke 2012, "Theory of Reflectance and Emittance
//!    Spectroscopy", 2nd ed.): the albedo of the ground's colour map is read as the diffusive
//!    (bihemispherical) reflectance r0 of a thick layer of grains, r0 = (1 - gamma) / (1 + gamma),
//!    gamma = sqrt(1 - w), which gives the single-scattering albedo w of one ground grain. Hapke's
//!    equivalent-slab model of a grain,
//!        w = Se + (1 - Se)(1 - Si) Theta / (1 - Si Theta),   Theta = exp(-alpha <D>),
//!        Se = ((n-1)^2 + k^2) / ((n+1)^2 + k^2) + 0.05,   Si = 1 - 4 / (n (n+1)^2),
//!        <D> = 2/3 (n^2 - (n^2 - 1)^(3/2) / n) D,
//!    is inverted for the internal transmission Theta, hence the absorption coefficient alpha of the
//!    material and its imaginary index k = alpha lambda / (4 pi). The grain size D of the ground is
//!    a setting: the same colour means a weakly absorbing material if the grains are large, a more
//!    absorbing one if they are small.
//!  * **Optics per class**: Mie (see [`super::mie`]) at each class's effective diameter with
//!    m = n + i k, averaged over a lognormal width: Qext, single-scattering albedo omega,
//!    asymmetry g. Tabulated over log k once, then read per cell.
//!  * **Two-stream**: a delta-Eddington layer (Joseph, Wiscombe & Weinman 1976; the generalised
//!    two-stream equations and Eddington coefficients of Meador & Weaver 1980) over a Lambertian
//!    ground, solved exactly with the ground as the lower boundary condition, so the multiple
//!    reflections between the layer and the ground are included. It returns the fractions of the
//!    incoming sunlight reflected to space, absorbed by the layer and absorbed by the ground; they
//!    sum to 1 by construction, and the layer absorbs nothing when omega = 1.
//!
//! Formulas from the cited textbooks and papers; no code copied or translated from any model.

use super::mie::{lognormal_optics, Cx};

/// Hapke's diffusive reflectance r0 of a thick layer of grains of single-scattering albedo w.
pub fn reflectance_from_w(w: f64) -> f64 {
    let gamma = (1.0 - w.clamp(0.0, 1.0)).sqrt();
    (1.0 - gamma) / (1.0 + gamma)
}

/// Inverse of [`reflectance_from_w`]: single-scattering albedo of the grains of a ground of
/// diffusive reflectance r0.
pub fn w_from_reflectance(r0: f64) -> f64 {
    let r = r0.clamp(0.0, 1.0);
    let gamma = (1.0 - r) / (1.0 + r);
    1.0 - gamma * gamma
}

/// Mean path length factor of the equivalent slab: <D> / D.
fn slab_path_factor(n: f64) -> f64 {
    2.0 / 3.0 * (n * n - (n * n - 1.0).powf(1.5) / n)
}

/// External and internal surface reflection coefficients of the equivalent slab (k << n).
fn slab_reflections(n: f64, k: f64) -> (f64, f64) {
    let se = ((n - 1.0).powi(2) + k * k) / ((n + 1.0).powi(2) + k * k) + 0.05;
    let si = 1.0 - 4.0 / (n * (n + 1.0).powi(2));
    (se, si)
}

/// Single-scattering albedo of one grain of diameter `d` (m) and index n + i k at `wavelength` (m),
/// by the equivalent-slab model.
pub fn slab_w(n: f64, k: f64, d: f64, wavelength: f64) -> f64 {
    let alpha = 4.0 * std::f64::consts::PI * k / wavelength;
    let theta = (-alpha * slab_path_factor(n) * d).exp();
    let (se, si) = slab_reflections(n, k);
    se + (1.0 - se) * (1.0 - si) * theta / (1.0 - si * theta)
}

/// Smallest internal transmission allowed by the inversion: a colour darker than the bare
/// reflection of the grain surfaces (w < Se) cannot be reached, it is read as an opaque grain.
pub const THETA_MIN: f64 = 1e-9;

/// Imaginary index k of the material of a ground of diffusive reflectance r0 made of grains of
/// diameter `d` (m), real index n, at `wavelength` (m). Returns k and whether the colour was darker
/// than the model can reach (the transmission then floors at [`THETA_MIN`]).
pub fn k_from_reflectance(r0: f64, n: f64, d: f64, wavelength: f64) -> (f64, bool) {
    let w = w_from_reflectance(r0);
    // Se depends on k only through k^2 << (n+1)^2: computed at k = 0 (exact enough for k < 0.1).
    let (se, si) = slab_reflections(n, 0.0);
    let num = w - se;
    let theta_raw = num / ((1.0 - se) * (1.0 - si) + si * num);
    let floored = !(theta_raw > THETA_MIN);
    let theta = if floored { THETA_MIN } else { theta_raw.min(1.0) };
    let alpha = -theta.ln() / (slab_path_factor(n) * d);
    (alpha * wavelength / (4.0 * std::f64::consts::PI), floored)
}

/// Fate of the sunlight reaching the top of the dust layer, as fractions of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoStream {
    /// Sent back to space (diffuse reflection of the layer-ground system).
    pub reflect: f64,
    /// Absorbed in the dust layer.
    pub layer: f64,
    /// Absorbed by the ground (net: what reaches it minus what it reflects).
    pub ground: f64,
    /// Total (direct + diffuse) sunlight reaching the ground, multiple reflections included.
    pub down: f64,
}

/// Highest single-scattering albedo used by the two-stream: at omega = 1 its two solutions merge
/// (k = 0); 1 - 1e-6 changes the fluxes by less than 1e-5.
pub const OMEGA_MAX: f64 = 1.0 - 1e-6;

/// Delta-Eddington two-stream for a homogeneous layer of optical depth `tau`, single-scattering
/// albedo `omega` and asymmetry `g`, lit by a beam at cosine zenith angle `mu0`, over a Lambertian
/// ground of albedo `ag`. Incoming flux on the horizontal = 1.
pub fn delta_eddington(tau: f64, omega: f64, g: f64, mu0: f64, ag: f64) -> TwoStream {
    // Delta scaling of the forward peak (f = g^2).
    let w0 = omega.clamp(0.0, OMEGA_MAX);
    let f = g * g;
    let tau = (1.0 - w0 * f) * tau.max(0.0);
    let w = (1.0 - f) * w0 / (1.0 - w0 * f);
    let g = g / (1.0 + g);
    // Eddington coefficients (Meador & Weaver 1980, table 1).
    let mut mu0 = mu0;
    let g1 = (7.0 - w * (4.0 + 3.0 * g)) / 4.0;
    let g2 = -(1.0 - w * (4.0 - 3.0 * g)) / 4.0;
    let k = (2.0 * (1.0 - w) * 1.5 * (1.0 - w * g)).sqrt();
    // The particular solution is singular at k = 1/mu0: nudge mu0 off it.
    if (k * mu0 - 1.0).abs() < 1e-6 {
        mu0 *= 1.0 + 2e-6;
    }
    let g3 = (2.0 - 3.0 * g * mu0) / 4.0;
    let g4 = 1.0 - g3;
    // dF+/dt = g1 F+ - g2 F- - w g3 S e^{-t/mu0},  dF-/dt = g2 F+ - g1 F- + w g4 S e^{-t/mu0},
    // with S = 1/mu0 (beam irradiance normal to the beam for a unit flux on the horizontal).
    let a = 1.0 / mu0;
    let s = a;
    let det = k * k - a * a;
    let z1 = (w * g3 * s * (g1 - a) + g2 * w * g4 * s) / det;
    let z2 = ((g1 + a) * w * g4 * s + g2 * w * g3 * s) / det;
    // Homogeneous modes: C1 (k+g1, g2) e^{k(t - tau)} and C2 (g2, k+g1) e^{-k t}.
    let ek = (-k * tau).exp();
    let e0 = (-a * tau).exp();
    let kg = k + g1;
    // F-(0) = 0;  F+(tau) = ag (F-(tau) + e0).
    let (a11, a12, b1) = (g2 * ek, kg, -z2);
    let (a21, a22, b2) = (kg - ag * g2, ek * (g2 - ag * kg), ag * (z2 + 1.0) * e0 - z1 * e0);
    let dd = a11 * a22 - a12 * a21;
    let c1 = (b1 * a22 - a12 * b2) / dd;
    let c2 = (a11 * b2 - b1 * a21) / dd;
    let up0 = c1 * kg * ek + c2 * g2 + z1;
    let dn_b = c1 * g2 + c2 * kg * ek + z2 * e0;
    let down = dn_b + e0;
    let ground = (1.0 - ag) * down;
    TwoStream { reflect: up0, layer: 1.0 - up0 - ground, ground, down }
}

/// Mie optics of the two dust classes, tabulated over the imaginary index k (log spaced), and the
/// per-cell values read from it.
#[derive(Clone, Debug)]
pub struct OpticsTable {
    pub log_k: Vec<f64>,
    /// Per class: Qext, omega, g at each tabulated k.
    pub qext: [Vec<f64>; 2],
    pub omega: [Vec<f64>; 2],
    pub g: [Vec<f64>; 2],
}

impl OpticsTable {
    /// Table from k = 1e-8 to 1 (`n` points per decade) for the class diameters `d` (m).
    pub fn new(d: [f64; 2], n_real: f64, sigma_g: f64, wavelength: f64, per_decade: usize) -> OpticsTable {
        let (lo, hi) = (-8.0f64, 0.0f64);
        let n = ((hi - lo) * per_decade as f64).round() as usize + 1;
        let log_k: Vec<f64> = (0..n).map(|j| lo + (hi - lo) * j as f64 / (n - 1) as f64).collect();
        let mut t = OpticsTable { log_k: log_k.clone(), qext: [vec![], vec![]], omega: [vec![], vec![]], g: [vec![], vec![]] };
        for c in 0..2 {
            for &lk in &log_k {
                let o = lognormal_optics(d[c], sigma_g, wavelength, Cx::new(n_real, 10f64.powf(lk)), 48);
                t.qext[c].push(o.qext);
                t.omega[c].push(o.omega);
                t.g[c].push(o.g);
            }
        }
        t
    }

    /// Linear interpolation in log k (clamped to the table).
    pub fn at(&self, c: usize, k: f64) -> (f64, f64, f64) {
        let lk = k.max(1e-30).log10();
        let n = self.log_k.len();
        let (lo, hi) = (self.log_k[0], self.log_k[n - 1]);
        let p = ((lk.clamp(lo, hi) - lo) / (hi - lo) * (n - 1) as f64).min((n - 1) as f64);
        let j = (p.floor() as usize).min(n - 2);
        let f = p - j as f64;
        let li = |v: &Vec<f64>| v[j] + f * (v[j + 1] - v[j]);
        (li(&self.qext[c]), li(&self.omega[c]), li(&self.g[c]))
    }
}
