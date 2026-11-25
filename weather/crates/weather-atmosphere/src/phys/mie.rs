//! Lorenz-Mie scattering by a homogeneous sphere, and its average over a lognormal size
//! distribution: the optics of the airborne dust grains.
//!
//! Written from the textbook equations (Bohren & Huffman 1983, "Absorption and Scattering of Light
//! by Small Particles", chapter 4 and the algorithm of their appendix A): the logarithmic
//! derivative D_n(mx) by downward recurrence, the Riccati-Bessel functions psi_n(x) and chi_n(x) by
//! upward recurrence, the coefficients a_n and b_n, then
//!     Qext = 2/x^2 sum (2n+1) Re(a_n + b_n)
//!     Qsca = 2/x^2 sum (2n+1) (|a_n|^2 + |b_n|^2)
//!     g Qsca = 4/x^2 [ sum n(n+2)/(n+1) Re(a_n a*_{n+1} + b_n b*_{n+1})
//!                      + sum (2n+1)/(n(n+1)) Re(a_n b*_n) ]
//!     Qback = 1/x^2 |sum (2n+1) (-1)^n (a_n - b_n)|^2
//! The refractive index is m = n + i k with k >= 0 for an absorbing grain (Bohren & Huffman's
//! sign convention). No code copied or translated from any Mie program.

use std::ops::{Add, Div, Mul, Sub};

/// Minimal complex number (only what the Mie series needs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cx {
    pub re: f64,
    pub im: f64,
}

impl Cx {
    pub const fn new(re: f64, im: f64) -> Cx {
        Cx { re, im }
    }
    pub fn conj(self) -> Cx {
        Cx::new(self.re, -self.im)
    }
    pub fn norm2(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn scale(self, s: f64) -> Cx {
        Cx::new(self.re * s, self.im * s)
    }
}

impl Add for Cx {
    type Output = Cx;
    fn add(self, o: Cx) -> Cx {
        Cx::new(self.re + o.re, self.im + o.im)
    }
}

impl Sub for Cx {
    type Output = Cx;
    fn sub(self, o: Cx) -> Cx {
        Cx::new(self.re - o.re, self.im - o.im)
    }
}

impl Mul for Cx {
    type Output = Cx;
    fn mul(self, o: Cx) -> Cx {
        Cx::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}

impl Div for Cx {
    type Output = Cx;
    fn div(self, o: Cx) -> Cx {
        // Smith's algorithm: no overflow for large |o|.
        if o.re.abs() >= o.im.abs() {
            let r = o.im / o.re;
            let d = o.re + o.im * r;
            Cx::new((self.re + self.im * r) / d, (self.im - self.re * r) / d)
        } else {
            let r = o.re / o.im;
            let d = o.re * r + o.im;
            Cx::new((self.re * r + self.im) / d, (self.im * r - self.re) / d)
        }
    }
}

/// Efficiencies of one sphere: extinction, scattering, backscattering, and the asymmetry
/// parameter g (mean cosine of the scattering angle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MieResult {
    pub qext: f64,
    pub qsca: f64,
    pub qback: f64,
    pub g: f64,
}

impl MieResult {
    /// Single-scattering albedo Qsca / Qext.
    pub fn omega(&self) -> f64 {
        if self.qext > 0.0 {
            (self.qsca / self.qext).min(1.0)
        } else {
            0.0
        }
    }
}

/// Mie efficiencies of a homogeneous sphere of size parameter x = 2 pi r / lambda and relative
/// refractive index m = n + i k (k >= 0 absorbs).
pub fn mie(x: f64, m: Cx) -> MieResult {
    assert!(x > 0.0 && x.is_finite(), "Mie: size parameter must be positive, got {x}");
    // Number of terms (Wiscombe's criterion, as in Bohren & Huffman's appendix).
    let nstop = (x + 4.0 * x.cbrt() + 2.0).ceil() as usize;
    let mx = m * Cx::new(x, 0.0);
    // Start of the downward recurrence of D_n: the usual max(nstop, |mx|) + 15 is not enough for
    // large, weakly absorbing spheres (measured on Wiscombe's x = 100 and x = 10 000 cases with
    // m = 1.33 + 1e-5 i: off by 3e-6 and 2e-3); the start error dies out within ~3 |mx|^(1/3)
    // terms, so start 8 |mx|^(1/3) higher.
    let nmx = ((nstop as f64).max(mx.abs()) + 8.0 * mx.abs().cbrt()).ceil() as usize + 16;
    // Logarithmic derivative D_n(mx), downward: D_{n-1} = n/(mx) - 1/(D_n + n/(mx)).
    let mut d = vec![Cx::new(0.0, 0.0); nmx + 1];
    for n in (1..=nmx).rev() {
        let r = Cx::new(n as f64, 0.0) / mx;
        d[n - 1] = r - Cx::new(1.0, 0.0) / (d[n] + r);
    }
    // Riccati-Bessel functions, upward: psi_{-1} = cos x, psi_0 = sin x; chi_{-1} = -sin x,
    // chi_0 = cos x; xi_n = psi_n - i chi_n.
    let (mut psi0, mut psi1) = (x.cos(), x.sin());
    let (mut chi0, mut chi1) = (-x.sin(), x.cos());
    let mut xi1 = Cx::new(psi1, -chi1);
    let (mut qext, mut qsca, mut gq) = (0.0, 0.0, 0.0);
    let mut back = Cx::new(0.0, 0.0);
    let (mut an1, mut bn1) = (Cx::new(0.0, 0.0), Cx::new(0.0, 0.0));
    for n in 1..=nstop {
        let fnn = n as f64;
        let psi = (2.0 * fnn - 1.0) * psi1 / x - psi0;
        let chi = (2.0 * fnn - 1.0) * chi1 / x - chi0;
        let xi = Cx::new(psi, -chi);
        let nx = Cx::new(fnn / x, 0.0);
        let da = d[n] / m + nx;
        let db = m * d[n] + nx;
        let an = (da.scale(psi) - Cx::new(psi1, 0.0)) / (da * xi - xi1);
        let bn = (db.scale(psi) - Cx::new(psi1, 0.0)) / (db * xi - xi1);
        let w = 2.0 * fnn + 1.0;
        qext += w * (an.re + bn.re);
        qsca += w * (an.norm2() + bn.norm2());
        let sign = if n % 2 == 1 { -1.0 } else { 1.0 };
        back = back + (an - bn).scale(w * sign);
        if n > 1 {
            let f = fnn - 1.0;
            gq += f * (f + 2.0) / (f + 1.0) * (an1 * an.conj() + bn1 * bn.conj()).re;
        }
        gq += w / (fnn * (fnn + 1.0)) * (an * bn.conj()).re;
        an1 = an;
        bn1 = bn;
        psi0 = psi1;
        psi1 = psi;
        chi0 = chi1;
        chi1 = chi;
        xi1 = xi;
    }
    let x2 = x * x;
    let qsca = 2.0 / x2 * qsca;
    MieResult { qext: 2.0 / x2 * qext, qsca, qback: back.norm2() / x2, g: if qsca > 0.0 { 4.0 / x2 * gq / qsca } else { 0.0 } }
}

/// Bulk optics of a population of grains (one dust class).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulkOptics {
    /// Extinction efficiency averaged over the cross-sections: <Qext pi r^2> / <pi r^2>; with the
    /// effective diameter it gives the mass extinction coefficient 1.5 Qext / (rho d_eff).
    pub qext: f64,
    /// Single-scattering albedo <Qsca r^2> / <Qext r^2>.
    pub omega: f64,
    /// Asymmetry parameter <g Qsca r^2> / <Qsca r^2>.
    pub g: f64,
}

/// Mie optics averaged over a lognormal number distribution of geometric standard deviation
/// `sigma_g` whose effective diameter (2 <r^3> / <r^2>) is `d_eff`: median radius
/// r_g = r_eff exp(-2.5 ln^2 sigma_g). `sigma_g` = 1 gives the single sphere. The distribution is
/// integrated in ln r over +-3.5 ln sigma_g (`nodes` points, trapezoid), which smooths the
/// resonance ripples a single size shows.
pub fn lognormal_optics(d_eff: f64, sigma_g: f64, wavelength: f64, m: Cx, nodes: usize) -> BulkOptics {
    let r_eff = 0.5 * d_eff;
    let ls = sigma_g.max(1.0).ln();
    let two_pi = 2.0 * std::f64::consts::PI;
    if ls < 1e-6 || nodes < 2 {
        let r = mie(two_pi * r_eff / wavelength, m);
        return BulkOptics { qext: r.qext, omega: r.omega(), g: r.g };
    }
    let rg = r_eff * (-2.5 * ls * ls).exp();
    let (mut a, mut e, mut s, mut gs) = (0.0, 0.0, 0.0, 0.0);
    for j in 0..nodes {
        let t = -3.5 + 7.0 * j as f64 / (nodes - 1) as f64;
        let wt = if j == 0 || j == nodes - 1 { 0.5 } else { 1.0 };
        let r = rg * (t * ls).exp();
        // Number density per ln r (Gaussian in t) times the cross-section weight r^2.
        let w = wt * (-0.5 * t * t).exp() * r * r;
        let q = mie(two_pi * r / wavelength, m);
        a += w;
        e += w * q.qext;
        s += w * q.qsca;
        gs += w * q.qsca * q.g;
    }
    BulkOptics { qext: e / a, omega: (s / e).min(1.0), g: gs / s }
}
