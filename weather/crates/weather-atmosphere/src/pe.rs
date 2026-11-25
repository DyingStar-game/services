//! Dry hydrostatic primitive equations in a hybrid sigma-pressure vertical coordinate on the
//! icosahedral hexagonal grid (steps 3 and 3b of the weather R&D).
//!
//! Vertical layout (Lorenz grid): N layers between half levels k-1/2 and k+1/2; k = 0 is the
//! top layer. Half-level pressures are p_{k+1/2} = A_{k+1/2} + B_{k+1/2} ps, with p = 0 at the
//! top (A = B = 0) and p = ps at the ground (A = 0, B = 1). Near the ground the layers follow the
//! relief (B > 0); above `eta_t` they are pure pressure surfaces (B = 0), so the upper layers,
//! where the tropopause lies, no longer slope over the mountains. eta_t = 0 gives pure sigma.
//! Horizontal velocity and potential temperature live at full levels, the vertical mass flux
//! W = d(eta)/dt dp/d(eta) at half levels (zero at the top and at the ground).
//!
//! Prognostic variables, per cell:
//!   ps               surface pressure (Pa), flux form => total air mass conserved to round-off;
//!   v_k              horizontal wind (m/s), 3-D Cartesian vector tangent to the sphere;
//!   Theta_k          = dp_k * theta_k, mass-weighted potential temperature (dp_k = dA_k + dB_k ps);
//!   M_k              = dp_k * q_k, mass-weighted passive dust tracer.
//!
//! Equations (written from the textbook primitive equations, e.g. Simmons & Burridge 1981
//! for the vertical discretisation in hybrid coordinates, Held & Suarez 1994 for the forcing):
//!   d ps / dt        = - sum_k div(dp_k v_k)
//!   W_{k+1/2}        = - sum_{j<=k} div(dp_j v_j) - B_{k+1/2} d ps/dt
//!   d v_k / dt       = -(zeta_k + f) r x v_k - grad(Phi_k + |v_k|^2/2) - R T_k (grad p / p)_k
//!                      - vertical advection (Simmons-Burridge form) - k_v(sigma) v_k + hyperdiffusion
//!   d Theta_k / dt   = - div(dp_k v_k theta_k) - [W theta]_{k-1/2}^{k+1/2}
//!                      + dp_k / Pi_k * ( -k_T (T_k - T_eq) + hyperdiffusion of T'_k )
//!   d M_k / dt       = - div(dp_k v_k q_k) - [W q]_{k-1/2}^{k+1/2}   (positive scheme)
//!   Phi              hydrostatic, Simmons-Burridge: Phi_k = Phi_{k+1/2} + alpha_k R T_k
//!   (grad p / p)_k   = [ln(p_{k+1/2}/p_{k-1/2}) B_{k-1/2} + alpha_k dB_k] grad(ps) / dp_k
//!   T_k              = theta_k * Pi_k,  Pi_k = (p_k / p0)^kappa,  ln p_k = ln p_{k+1/2} - alpha_k
//!
//! Hyperdiffusion of order 2n (n = 2, 3, 4: del^4, del^6, del^8), with the sign that damps,
//! acts on the wind, on T' = T - T_ref(p) (a fixed reference profile, so that diffusing along
//! sloping layers does not mix a resting stratified atmosphere), and on ps' = ps - ps_ref
//! (ps_ref = hydrostatic surface pressure over the relief for T_ref) as a mass flux, so that air
//! mass, theta and dust stay consistent and conserved. Its coefficient is set by the e-folding
//! time tau of the shortest grid mode: nu = (d^2 / 6)^n / tau.
//! Time stepping: SSP-RK3, advanced in fixed "chunks" (e.g. 1/24 day) split into equal steps
//! whose length follows the CFL rule at the start of the chunk: the same chunk boundaries
//! give the same steps, which makes restarts bit-for-bit.

use crate::grid::Grid;
use crate::vec3::*;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Fixed reference temperature profile T_ref(p) (used by the T hyperdiffusion and by ps_ref):
/// T_ref = max(t_strat, t0 (p/p0)^(R lapse / g)), a constant-lapse-rate troposphere capped by an
/// isothermal stratosphere. `lapse = 0` gives an isothermal reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefProfile {
    pub t0: f64,
    pub lapse: f64,
    pub t_strat: f64,
}

#[derive(Clone, Debug)]
pub struct PeConsts {
    pub g: f64,
    pub rd: f64,
    pub cp: f64,
    pub kappa: f64,
    /// Reference pressure of the Exner function, of Held-Suarez and of the hybrid levels (Pa).
    pub p0: f64,
    pub omega: f64,
}

impl PeConsts {
    /// The air and the rotation of a body (all from the shared data, see `body`).
    pub fn from_body(b: &crate::body::Body) -> PeConsts {
        PeConsts { g: b.gravity, rd: b.air.rd, cp: b.air.cp, kappa: b.air.kappa, p0: b.p_surface, omega: b.omega }
    }
}

impl RefProfile {
    pub fn temp(&self, p: f64, c: &PeConsts) -> f64 {
        if self.lapse == 0.0 {
            return self.t0;
        }
        (self.t0 * (p / c.p0).powf(c.rd * self.lapse / c.g)).max(self.t_strat)
    }
    /// Geopotential (m^2/s^2) of the pressure p in a resting atmosphere with this profile
    /// (zero at p0): the exact integral of d Phi / d ln p = -R T_ref(p).
    pub fn geopotential(&self, p: f64, c: &PeConsts) -> f64 {
        if self.lapse == 0.0 {
            return c.rd * self.t0 * (c.p0 / p).ln();
        }
        let a = c.rd * self.lapse / c.g;
        // Tropopause pressure: where the troposphere reaches t_strat.
        let pt = c.p0 * (self.t_strat / self.t0).powf(1.0 / a);
        let tropo = |q: f64| c.g * self.t0 / self.lapse * (1.0 - (q / c.p0).powf(a));
        if p >= pt {
            tropo(p)
        } else {
            tropo(pt) + c.rd * self.t_strat * (pt / p).ln()
        }
    }
    /// Surface pressure of a resting atmosphere with this profile over the height z (m),
    /// given the pressure ps_sea at z = 0 (valid while the ground stays in the troposphere).
    pub fn surface_pressure(&self, z: f64, ps_sea: f64, c: &PeConsts) -> f64 {
        if self.lapse == 0.0 {
            ps_sea * (-c.g * z / (c.rd * self.t0)).exp()
        } else {
            ps_sea * (1.0 - self.lapse * z / self.t0).powf(c.g / (c.rd * self.lapse))
        }
    }
}

/// Hybrid coefficients: B = max(0, (eta - eta_t) / (1 - eta_t)) (terrain-following below eta_t,
/// linear in eta down to the ground), A = p0 (eta - B), so that p = A + B ps equals p0 eta for
/// ps = p0. eta_t = 0 gives pure sigma (A = 0, B = eta), exactly.
pub fn hybrid_coefficients(eta_half: &[f64], eta_t: f64, p0: f64) -> (Vec<f64>, Vec<f64>) {
    assert!((0.0..1.0).contains(&eta_t), "eta_t in [0, 1)");
    let b: Vec<f64> = eta_half.iter().map(|&e| ((e - eta_t) / (1.0 - eta_t)).max(0.0)).collect();
    let a: Vec<f64> = eta_half.iter().zip(&b).map(|(&e, &bb)| p0 * (e - bb)).collect();
    (a, b)
}

/// Order of the hyperdiffusion (2 = del^4, 3 = del^6, 4 = del^8) and its coefficient from the
/// e-folding time `tau_s` of the shortest mode of the hexagonal grid (eigenvalue about -6 / d^2).
pub fn hyper_nu(grid: &Grid, tau_s: f64, order: u32) -> f64 {
    let d = grid.mean_spacing();
    (d * d / 6.0).powi(order as i32) / tau_s
}

#[derive(Clone)]
pub struct PeState {
    pub ps: Vec<f64>,
    /// v[i * L + k]
    pub v: Vec<V3>,
    /// Theta[i * L + k] = dp theta
    pub th: Vec<f64>,
    /// M[i * L + k] = dp q
    pub m: Vec<f64>,
    pub time: f64,
    /// Number of completed chunks (the time is chunk * chunk_s at chunk boundaries).
    pub chunk: u64,
    pub dust_active: bool,
    /// Divisor of the CFL time step while `chunk < reduced_until` (set by the guard after a
    /// failure; stored in restart files so that restarts stay bit-for-bit).
    pub dt_div: u32,
    pub reduced_until: u64,
    /// Second tracer M2[i * L + k] = dp q2 (step 4: coarse dust, kg/kg); unused when the model
    /// has a single tracer (steps 3 and 3b).
    pub m2: Vec<f64>,
    /// Ground skin temperature and deep (restore) temperature of the force-restore scheme (K),
    /// per cell (step 4; zero when the ground is off).
    pub tg: Vec<f64>,
    pub t2: Vec<f64>,
    /// Cumulative dust budget (kg, whole planet, both classes): lifted, deposited (settling and
    /// dry deposition at the ground), added by the dust-sea relaxation, added by events.
    pub budget: [f64; 4],
    /// Journal of the game events (heat and dust injected spread over time). Kept across a
    /// roll-back of the guard and written to the restart files.
    pub events: Vec<crate::phys::Event>,
}

impl PeState {
    pub fn zeros(n: usize, l: usize) -> PeState {
        PeState {
            ps: vec![0.0; n],
            v: vec![[0.0; 3]; n * l],
            th: vec![0.0; n * l],
            m: vec![0.0; n * l],
            time: 0.0,
            chunk: 0,
            dust_active: false,
            dt_div: 1,
            reduced_until: 0,
            m2: vec![0.0; n * l],
            tg: vec![0.0; n],
            t2: vec![0.0; n],
            budget: [0.0; 4],
            events: Vec::new(),
        }
    }
    pub fn bit_equal(&self, o: &PeState) -> bool {
        let eq = |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits());
        eq(&self.ps, &o.ps)
            && eq(&self.th, &o.th)
            && eq(&self.m, &o.m)
            && eq(&self.m2, &o.m2)
            && eq(&self.tg, &o.tg)
            && eq(&self.t2, &o.t2)
            && eq(&self.budget, &o.budget)
            && self.events == o.events
            && self.v.len() == o.v.len()
            && self.v.iter().zip(&o.v).all(|(a, b)| eq(a, b))
            && self.time.to_bits() == o.time.to_bits()
            && self.chunk == o.chunk
            && self.dust_active == o.dust_active
            && self.dt_div == o.dt_div
            && self.reduced_until == o.reduced_until
    }
}

pub struct PeModel {
    pub grid: Grid,
    pub c: PeConsts,
    pub nl: usize,
    /// eta = p / p0 at half levels for a column with ps = p0.
    pub eta_half: Vec<f64>,
    /// Top of the terrain-following part (eta), 0 = pure sigma.
    pub eta_t: f64,
    pub a_half: Vec<f64>,
    pub b_half: Vec<f64>,
    pub da: Vec<f64>,
    pub db: Vec<f64>,
    /// Full-level p / p0 of a column with ps = p0 (labels of the layers in the outputs).
    pub sig_full: Vec<f64>,
    /// Coriolis parameter per cell.
    pub f: Vec<f64>,
    /// Surface geopotential per cell (m^2/s^2).
    pub phis: Vec<f64>,
    /// Reference surface pressure used by the ps hyperdiffusion.
    pub ps_ref: Vec<f64>,
    pub tref: RefProfile,
    /// Hyperdiffusion order n (del^2n) and coefficients for wind, T' and ps'.
    pub hyper_order: u32,
    pub nu_v: f64,
    pub nu_t: f64,
    pub nu_ps: f64,
    /// Temperature relaxation and friction of the free atmosphere (Held-Suarez by default),
    /// evaluated at every Runge-Kutta stage. None = no forcing.
    pub relax: Option<Box<dyn crate::relax::Relaxation>>,
    /// Lower / upper bounds of a cell's surface pressure accepted by the guard (Pa).
    pub ps_bounds: (f64, f64),
    /// Largest wind speed accepted by the guard (m/s).
    pub max_wind: f64,
    /// Pressure-gradient force computed on the deviation from the reference atmosphere
    /// (default). False = the plain form -grad(Phi) - R T grad(p)/p, kept for the tests.
    pub pgf_ref: bool,
    /// Test hook: the end of chunk number `fault_chunk` is corrupted (a NaN surface pressure)
    /// the next `fault_count` times it is reached. Zero = never.
    pub fault_chunk: AtomicU64,
    pub fault_count: AtomicU32,
    /// Number of transported tracers: 1 (steps 3, 3b: `m`) or 2 (step 4: fine `m`, coarse `m2`).
    pub ntr: usize,
    /// Column physics (a suite of swappable components: ground and day-night, dust, dust sea,
    /// lid, convection, events), applied after every dynamics step. None = dynamics only.
    pub phys: Option<crate::phys::PhysicsSuite>,
}

/// Bounds of the guard: surface pressure as fractions of the mean surface pressure, largest wind.
#[derive(Clone, Copy, Debug)]
pub struct GuardLimits {
    pub ps_min_frac: f64,
    pub ps_max_frac: f64,
    pub max_wind: f64,
}

/// Scratch buffers, allocated once.
pub struct PeWork {
    /// Per-cell diagnostics of the last physics step (the published "ingredients").
    pub ph: crate::phys::PhysWork,
    // Pass A, per cell: PA_L values per layer, then ps' and ln ps.
    pa: Vec<f64>,
    eb: Vec<f64>,
    cc: Vec<f64>,
    // Iterated Laplacians, per cell: (vx, vy, vz, T') per layer, then ps'.
    lap_a: Vec<f64>,
    lap_b: Vec<f64>,
    ed: Vec<f64>,
    // Pass E outputs
    ratio: Vec<f64>,
    vflux: Vec<f64>,
    tlim: Vec<f64>,
    // tendencies
    dps: Vec<f64>,
    dv: Vec<V3>,
    dth: Vec<f64>,
    dm: Vec<f64>,
    dm2: Vec<f64>,
    s1: PeState,
    s2: PeState,
}

// Strides of the per-edge / per-cell scratch records.
const PA_L: usize = 9; // theta, T, Bernoulli, q, T', coefficient of grad ps in the PGF, dp, p, q2
const EB_L: usize = 8; // circ, bern, lap vx vy vz, lap T', grad q face, grad q2 face
const EB_X: usize = 2; // ln ps face, lap ps'
const CC_L: usize = 14; // zeta, gradB(3), gradq(3), cmin, cmax, gradq2(3), cmin2, cmax2
const CC_X: usize = 3; // grad ln ps (3)
const ED_L: usize = 8; // mass flux, theta flux, del^2n v(3), del^2n T', raw tracer flux, raw tracer-2 flux
// Offsets of the two tracers inside those records.
const PA_Q: [usize; 2] = [3, 8];
const CC_G: [usize; 2] = [4, 9];
const CC_MIN: [usize; 2] = [7, 12];
const ED_Q: [usize; 2] = [6, 7];

#[derive(Clone, Copy, Debug)]
pub struct PeDiag {
    /// Total air mass (kg).
    pub air_mass: f64,
    /// Total dust (kg of air * mixing ratio).
    pub dust_mass: f64,
    pub min_ps: f64,
    pub max_ps: f64,
    pub max_speed: f64,
    pub min_t: f64,
    pub max_t: f64,
    pub min_dust: f64,
    pub finite: bool,
    /// Total of the second tracer (kg of air * mixing ratio), step 4: coarse dust (kg).
    pub dust2_mass: f64,
    pub min_dust2: f64,
}

/// What the guarded chunk advance did.
#[derive(Clone, Debug, PartialEq)]
pub enum ChunkOutcome {
    /// The chunk ran, with this many steps.
    Done(usize),
    /// The chunk failed; the state went back to the last good one (message for the log).
    RolledBack(String),
}

/// Guard with roll-back: the last good state (taken at each model-day boundary) and the number
/// of failures in a row.
pub struct Guard {
    pub last_good: PeState,
    pub fails: u32,
    /// Failures in a row that stop the run for good.
    pub max_fails: u32,
    pub events: Vec<String>,
}

impl Guard {
    pub fn new(st: &PeState) -> Guard {
        Guard { last_good: st.clone(), fails: 0, max_fails: 3, events: Vec::new() }
    }
}

/// Column pressure geometry (Simmons-Burridge) for one surface pressure: layer thickness dp
/// (Pa), alpha and ln(p_{k+1/2} / p_{k-1/2}) of the hydrostatic integration, full-level pressure.
pub struct Column {
    pub dp: [f64; 16],
    pub alpha: [f64; 16],
    pub lnr: [f64; 16],
    pub pf: [f64; 16],
    cpg: [f64; 16],
}

impl PeModel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        grid: Grid,
        c: PeConsts,
        eta_half: Vec<f64>,
        eta_t: f64,
        phis: Vec<f64>,
        tref: RefProfile,
        hyper_order: u32,
        nu_v: f64,
        nu_t: f64,
        nu_ps: f64,
        relax: Option<Box<dyn crate::relax::Relaxation>>,
        limits: GuardLimits,
    ) -> PeModel {
        let nl = eta_half.len() - 1;
        assert!((2..=16).contains(&nl), "2 to 16 layers");
        assert!((2..=5).contains(&hyper_order), "hyperdiffusion order 2 to 5");
        let (a_half, b_half) = hybrid_coefficients(&eta_half, eta_t, c.p0);
        let da: Vec<f64> = (0..nl).map(|k| a_half[k + 1] - a_half[k]).collect();
        let db: Vec<f64> = (0..nl).map(|k| b_half[k + 1] - b_half[k]).collect();
        let f: Vec<f64> = grid.x.iter().map(|x| 2.0 * c.omega * x[2]).collect();
        let n = grid.n_cells();
        // Lowest surface pressure for which every layer keeps a positive thickness, with margin.
        let mut ps_lo: f64 = limits.ps_min_frac * c.p0;
        for k in 0..nl {
            if db[k] > 0.0 {
                ps_lo = ps_lo.max(1.25 * (-da[k] / db[k]));
            }
        }
        let ps_hi = limits.ps_max_frac * c.p0;
        let mut m = PeModel {
            grid,
            c,
            nl,
            eta_half,
            eta_t,
            a_half,
            b_half,
            da,
            db,
            sig_full: vec![0.0; nl],
            f,
            phis,
            ps_ref: vec![0.0; n],
            tref,
            hyper_order,
            nu_v,
            nu_t,
            nu_ps,
            relax,
            ps_bounds: (ps_lo, ps_hi),
            max_wind: limits.max_wind,
            pgf_ref: true,
            fault_chunk: AtomicU64::new(0),
            fault_count: AtomicU32::new(0),
            ntr: 1,
            phys: None,
        };
        let p0 = m.c.p0;
        m.sig_full = (0..nl).map(|k| m.p_full(k, p0) / p0).collect();
        m.set_ps_ref(p0);
        m
    }

    /// ps_ref = hydrostatic surface pressure of the reference profile over the relief, scaled
    /// so that its global mean is `ps_mean`.
    pub fn set_ps_ref(&mut self, ps_mean: f64) {
        let c = self.c.clone();
        let raw: Vec<f64> = self.phis.iter().map(|&p| self.tref.surface_pressure(p / c.g, 1.0, &c)).collect();
        let mean = self.grid.integral(&raw) / self.grid.area.iter().sum::<f64>();
        self.ps_ref = raw.iter().map(|r| r * ps_mean / mean).collect();
    }

    pub fn new_work(&self) -> PeWork {
        let (n, ne, l) = (self.grid.n_cells(), self.grid.n_edges(), self.nl);
        PeWork {
            ph: crate::phys::PhysWork::new(if self.phys.is_some() { n } else { 0 }),
            pa: vec![0.0; n * (PA_L * l + 2)],
            eb: vec![0.0; ne * (EB_L * l + EB_X)],
            cc: vec![0.0; n * (CC_L * l + CC_X)],
            lap_a: vec![0.0; n * (4 * l + 1)],
            lap_b: vec![0.0; n * (4 * l + 1)],
            ed: vec![0.0; ne * ED_L * l],
            ratio: vec![1.0; n * l * 2],
            vflux: vec![0.0; n * (l + 1) * 2],
            tlim: vec![0.0; ne * l * 2],
            dps: vec![0.0; n],
            dv: vec![[0.0; 3]; n * l],
            dth: vec![0.0; n * l],
            dm: vec![0.0; n * l],
            dm2: vec![0.0; n * l],
            s1: PeState::zeros(n, l),
            s2: PeState::zeros(n, l),
        }
    }

    /// Mass per unit area times g of layer k (Pa) for surface pressure ps.
    #[inline]
    pub fn dp(&self, k: usize, ps: f64) -> f64 {
        self.da[k] + self.db[k] * ps
    }

    #[inline]
    pub fn column(&self, ps: f64) -> Column {
        let l = self.nl;
        let mut col = Column { dp: [0.0; 16], alpha: [0.0; 16], lnr: [0.0; 16], pf: [0.0; 16], cpg: [0.0; 16] };
        for k in 0..l {
            let ph_up = self.a_half[k] + self.b_half[k] * ps;
            let ph_dn = self.a_half[k + 1] + self.b_half[k + 1] * ps;
            let d = ph_dn - ph_up;
            let (lnr, alpha) = if ph_up <= 0.0 {
                (0.0, std::f64::consts::LN_2)
            } else {
                let r = (ph_dn / ph_up).ln();
                (r, 1.0 - ph_up / d * r)
            };
            col.dp[k] = d;
            col.alpha[k] = alpha;
            col.lnr[k] = lnr;
            col.pf[k] = ph_dn * (-alpha).exp();
            // Top layer (p = 0 above): grad(p_{3/2}) / p_{3/2}, which keeps an isothermal resting
            // atmosphere balanced in pure sigma (alpha = ln 2 would not) and vanishes when the
            // layer is a pure pressure layer.
            col.cpg[k] = if ph_up <= 0.0 { self.b_half[k + 1] / ph_dn } else { (lnr * self.b_half[k] + alpha * self.db[k]) / d };
        }
        col
    }

    /// Full-level pressure (Pa) of layer k for surface pressure ps (Simmons-Burridge).
    pub fn p_full(&self, k: usize, ps: f64) -> f64 {
        self.column(ps).pf[k]
    }

    /// Exner function at full level k for surface pressure ps.
    #[inline]
    pub fn exner(&self, k: usize, ps: f64) -> f64 {
        (self.p_full(k, ps) / self.c.p0).powf(self.c.kappa)
    }

    /// Temperature (K) of cell i, layer k.
    pub fn temperature(&self, st: &PeState, i: usize, k: usize) -> f64 {
        let l = self.nl;
        let ps = st.ps[i];
        st.th[i * l + k] / self.dp(k, ps) * self.exner(k, ps)
    }

    /// dst = Laplacian of src for every cell (records of `stride` values), flux form.
    fn laplacian_pass(gr: &Grid, src: &[f64], dst: &mut [f64], stride: usize) {
        dst.par_chunks_mut(stride).enumerate().for_each(|(i, o)| {
            o.iter_mut().for_each(|x| *x = 0.0);
            let si = &src[i * stride..(i + 1) * stride];
            for s in gr.c_start[i]..gr.c_start[i + 1] {
                let e = gr.c_edge[s];
                let ld = gr.e_l[e] / gr.e_d[e];
                let nb = gr.c_nbr[s];
                let sn = &src[nb * stride..(nb + 1) * stride];
                for m in 0..stride {
                    o[m] += (sn[m] - si[m]) * ld;
                }
            }
            let inv = 1.0 / gr.area[i];
            o.iter_mut().for_each(|x| *x *= inv);
        });
    }

    fn rhs(&self, st: &PeState, t: f64, dt: f64, w: &mut PeWork) {
        let gr = &self.grid;
        let l = self.nl;
        let c = &self.c;
        let (rd, p0, kappa) = (c.rd, c.p0, c.kappa);
        let dust = st.dust_active;
        let ntr = if dust { self.ntr } else { 0 };
        let ms: [&[f64]; 2] = [&st.m, &st.m2];
        let spa = PA_L * l + 2;
        let shl = 4 * l + 1;

        // ---- Pass A (cells): column geometry, theta, T, hydrostatic Phi, Bernoulli, q, T', ps'.
        {
            let ps = &st.ps;
            let tref = self.tref;
            let phis = &self.phis;
            let ps_ref = &self.ps_ref;
            let pgf_ref = self.pgf_ref;
            w.pa.par_chunks_mut(spa).enumerate().for_each(|(i, o)| {
                let p = ps[i];
                let col = self.column(p);
                o[PA_L * l] = p - ps_ref[i];
                o[PA_L * l + 1] = p.ln();
                let mut tdev = [0.0f64; 16];
                for k in 0..l {
                    let r = &mut o[PA_L * k..PA_L * (k + 1)];
                    let mass = col.dp[k];
                    let th = st.th[i * l + k] / mass;
                    let pk = col.pf[k];
                    let tt = th * (pk / p0).powf(kappa);
                    let tp = tt - tref.temp(pk, c);
                    r[0] = th;
                    r[1] = tt;
                    r[3] = if dust { (st.m[i * l + k] / mass).max(0.0) } else { 0.0 };
                    r[8] = if ntr > 1 { (st.m2[i * l + k] / mass).max(0.0) } else { 0.0 };
                    r[4] = tp;
                    r[5] = col.cpg[k];
                    r[6] = mass;
                    r[7] = pk;
                    tdev[k] = if pgf_ref { tp } else { tt };
                }
                // Hydrostatic integration from the ground up (Simmons-Burridge), of the
                // deviation from the reference atmosphere only: Phi = Phi_ref(p) + Phi'.
                // The reference part has no pressure-gradient force (it depends on p alone),
                // so it is dropped exactly; this removes the pressure-gradient error of a
                // resting reference atmosphere over steep relief.
                let mut phi_half = if pgf_ref { phis[i] - tref.geopotential(p, c) } else { phis[i] };
                for k in (0..l).rev() {
                    let phik = phi_half + col.alpha[k] * rd * tdev[k];
                    let vk = st.v[i * l + k];
                    o[PA_L * k + 2] = phik + 0.5 * dot(vk, vk);
                    if k > 0 {
                        phi_half += rd * tdev[k] * col.lnr[k];
                    }
                }
            });
        }

        // ---- Pass B (edges): circulation, Bernoulli face terms, first-Laplacian differences.
        {
            let pa = &w.pa;
            let v = &st.v;
            let sb = EB_L * l + EB_X;
            w.eb.par_chunks_mut(sb).enumerate().for_each(|(e, o)| {
                let (i, j) = (gr.e_i[e], gr.e_j[e]);
                let le = gr.e_l[e];
                let ld = le / gr.e_d[e];
                let tt = gr.e_t[e];
                let (pi, pj) = (&pa[i * spa..(i + 1) * spa], &pa[j * spa..(j + 1) * spa]);
                for k in 0..l {
                    let (a, b) = (i * l + k, j * l + k);
                    let r = &mut o[EB_L * k..EB_L * (k + 1)];
                    let ve = scale(add(v[a], v[b]), 0.5);
                    let (oi, oj) = (&pi[PA_L * k..], &pj[PA_L * k..]);
                    r[0] = dot(ve, tt) * le;
                    r[1] = 0.5 * (oj[2] - oi[2]) * le;
                    r[2] = (v[b][0] - v[a][0]) * ld;
                    r[3] = (v[b][1] - v[a][1]) * ld;
                    r[4] = (v[b][2] - v[a][2]) * ld;
                    r[5] = (oj[4] - oi[4]) * ld;
                    r[6] = 0.5 * (oj[3] - oi[3]) * le;
                    r[7] = 0.5 * (oj[8] - oi[8]) * le;
                }
                o[EB_L * l] = 0.5 * (pj[PA_L * l + 1] - pi[PA_L * l + 1]) * le;
                o[EB_L * l + 1] = (pj[PA_L * l] - pi[PA_L * l]) * ld;
            });
        }

        // ---- Pass C (cells): vorticity, gradients, first Laplacians, tracer neighbourhood range.
        {
            let eb = &w.eb;
            let pa = &w.pa;
            let sb = EB_L * l + EB_X;
            let sc = CC_L * l + CC_X;
            w.cc.par_chunks_mut(sc).zip(w.lap_a.par_chunks_mut(shl)).enumerate().for_each(|(i, (o, lp))| {
                o.iter_mut().for_each(|x| *x = 0.0);
                lp.iter_mut().for_each(|x| *x = 0.0);
                let inv = 1.0 / gr.area[i];
                let x = gr.x[i];
                for k in 0..l {
                    let q = pa[i * spa + PA_L * k + 3];
                    o[CC_L * k + 7] = q;
                    o[CC_L * k + 8] = q;
                    let q2 = pa[i * spa + PA_L * k + 8];
                    o[CC_L * k + 12] = q2;
                    o[CC_L * k + 13] = q2;
                }
                let mut gps = [0.0; 3];
                let mut lps = 0.0;
                for s in gr.c_start[i]..gr.c_start[i + 1] {
                    let e = gr.c_edge[s];
                    let sg = gr.c_sign[s];
                    let n = gr.e_n[e];
                    let nb = gr.c_nbr[s];
                    let r = &eb[e * sb..(e + 1) * sb];
                    for k in 0..l {
                        let a = &r[EB_L * k..EB_L * (k + 1)];
                        let oc = &mut o[CC_L * k..CC_L * (k + 1)];
                        oc[0] += sg * a[0];
                        oc[1] += n[0] * a[1];
                        oc[2] += n[1] * a[1];
                        oc[3] += n[2] * a[1];
                        oc[4] += n[0] * a[6];
                        oc[5] += n[1] * a[6];
                        oc[6] += n[2] * a[6];
                        let qn = pa[nb * spa + PA_L * k + 3];
                        oc[7] = oc[7].min(qn);
                        oc[8] = oc[8].max(qn);
                        if ntr > 1 {
                            oc[9] += n[0] * a[7];
                            oc[10] += n[1] * a[7];
                            oc[11] += n[2] * a[7];
                            let qn2 = pa[nb * spa + PA_L * k + 8];
                            oc[12] = oc[12].min(qn2);
                            oc[13] = oc[13].max(qn2);
                        }
                        let lk = &mut lp[4 * k..4 * k + 4];
                        lk[0] += sg * a[2];
                        lk[1] += sg * a[3];
                        lk[2] += sg * a[4];
                        lk[3] += sg * a[5];
                    }
                    gps = add(gps, scale(n, r[EB_L * l]));
                    lps += sg * r[EB_L * l + 1];
                }
                for k in 0..l {
                    let oc = &mut o[CC_L * k..CC_L * (k + 1)];
                    oc[0] *= inv;
                    let gb = tangent(scale([oc[1], oc[2], oc[3]], inv), x);
                    oc[1] = gb[0];
                    oc[2] = gb[1];
                    oc[3] = gb[2];
                    let gq = tangent(scale([oc[4], oc[5], oc[6]], inv), x);
                    oc[4] = gq[0];
                    oc[5] = gq[1];
                    oc[6] = gq[2];
                    if ntr > 1 {
                        let gq2 = tangent(scale([oc[9], oc[10], oc[11]], inv), x);
                        oc[9] = gq2[0];
                        oc[10] = gq2[1];
                        oc[11] = gq2[2];
                    }
                }
                for m in 0..4 * l {
                    lp[m] *= inv;
                }
                let gps = tangent(scale(gps, inv), x);
                o[CC_L * l] = gps[0];
                o[CC_L * l + 1] = gps[1];
                o[CC_L * l + 2] = gps[2];
                lp[4 * l] = lps * inv;
            });
        }

        // ---- Further Laplacians for del^6 / del^8: lap_a holds L^(n-1) afterwards.
        for _ in 2..self.hyper_order {
            Self::laplacian_pass(gr, &w.lap_a, &mut w.lap_b, shl);
            std::mem::swap(&mut w.lap_a, &mut w.lap_b);
        }
        // Damping sign of del^2n: (-1)^(n+1).
        let sgn = if self.hyper_order % 2 == 0 { -1.0 } else { 1.0 };

        // ---- Pass D (edges): mass fluxes (advective + ps hyperdiffusion), theta fluxes,
        //      hyperdiffusion fluxes, upwind-biased tracer fluxes.
        {
            let cc = &w.cc;
            let lap = &w.lap_a;
            let pa = &w.pa;
            let (v, ps) = (&st.v, &st.ps);
            let sc = CC_L * l + CC_X;
            let (nu_ps, da, db) = (self.nu_ps, &self.da, &self.db);
            w.ed.par_chunks_mut(ED_L * l).enumerate().for_each(|(e, o)| {
                let (i, j) = (gr.e_i[e], gr.e_j[e]);
                let le = gr.e_l[e];
                let ld = le / gr.e_d[e];
                let n = gr.e_n[e];
                let (li, lj) = (&lap[i * shl..(i + 1) * shl], &lap[j * shl..(j + 1) * shl]);
                let pse = 0.5 * (ps[i] + ps[j]);
                // ps hyperdiffusion as a mass flux from i to j (per unit B).
                let fdiff = -sgn * nu_ps * (lj[4 * l] - li[4 * l]) * ld;
                for k in 0..l {
                    let (a, b) = (i * l + k, j * l + k);
                    let r = &mut o[ED_L * k..ED_L * (k + 1)];
                    let ve = scale(add(v[a], v[b]), 0.5);
                    let fl = (da[k] + db[k] * pse) * dot(ve, n) * le + db[k] * fdiff;
                    let (thi, thj) = (pa[i * spa + PA_L * k], pa[j * spa + PA_L * k]);
                    r[0] = fl;
                    r[1] = fl * 0.5 * (thi + thj);
                    r[2] = (lj[4 * k] - li[4 * k]) * ld;
                    r[3] = (lj[4 * k + 1] - li[4 * k + 1]) * ld;
                    r[4] = (lj[4 * k + 2] - li[4 * k + 2]) * ld;
                    r[5] = (lj[4 * k + 3] - li[4 * k + 3]) * ld;
                    r[6] = 0.0;
                    r[7] = 0.0;
                    for t in 0..ntr {
                        let (u, dn, slot) = if fl >= 0.0 { (i, j, gr.e_slot_i[e]) } else { (j, i, gr.e_slot_j[e]) };
                        let cu = &cc[u * sc + CC_L * k..u * sc + CC_L * (k + 1)];
                        let qu = pa[u * spa + PA_L * k + PA_Q[t]];
                        let qd = pa[dn * spa + PA_L * k + PA_Q[t]];
                        let g0 = CC_G[t];
                        let gq = [cu[g0], cu[g0 + 1], cu[g0 + 2]];
                        // "kappa = 1/3" face value clipped to the upwind neighbourhood range.
                        let raw = qu + (2.0 / 3.0) * dot(gq, gr.c_r[slot]) + (qd - qu) / 6.0;
                        r[ED_Q[t]] = fl * raw.clamp(cu[CC_MIN[t]], cu[CC_MIN[t] + 1]).max(0.0);
                    }
                }
            });
        }

        // ---- Pass E (cells): column continuity, vertical advection, forcing, hyperdiffusion,
        //      tendencies, tracer vertical fluxes and outflow limiter.
        {
            let ed = &w.ed;
            let cc = &w.cc;
            let pa = &w.pa;
            let v = &st.v;
            let ps = &st.ps;
            let f = &self.f;
            let sc = CC_L * l + CC_X;
            let (nu_v, nu_t) = (self.nu_v, self.nu_t);
            let relax = self.relax.as_deref();
            let sin0 = relax.map(|h| h.subsolar_lat(t).sin()).unwrap_or(0.0);
            let b_half = &self.b_half;
            let pgf_ref = self.pgf_ref;
            w.dps
                .par_iter_mut()
                .zip(w.dv.par_chunks_mut(l))
                .zip(w.dth.par_chunks_mut(l))
                .zip(w.ratio.par_chunks_mut(2 * l))
                .zip(w.vflux.par_chunks_mut(2 * (l + 1)))
                .enumerate()
                .for_each(|(i, ((((dps, dv), dth), ratio), vfl))| {
                    let inv = 1.0 / gr.area[i];
                    let x = gr.x[i];
                    let ci = &cc[i * sc..(i + 1) * sc];
                    let pc = &pa[i * spa..(i + 1) * spa];
                    // Edge sums per layer: divergence of mass flux, of theta flux, del^2n.
                    let mut div = [0.0f64; 16];
                    let mut dth_h = [0.0f64; 16];
                    let mut d4 = [[0.0f64; 4]; 16];
                    let mut out = [[0.0f64; 16]; 2];
                    for s in gr.c_start[i]..gr.c_start[i + 1] {
                        let e = gr.c_edge[s];
                        let sg = gr.c_sign[s];
                        let r = &ed[e * ED_L * l..(e + 1) * ED_L * l];
                        for k in 0..l {
                            let a = &r[ED_L * k..ED_L * (k + 1)];
                            div[k] += sg * a[0];
                            dth_h[k] += sg * a[1];
                            for m in 0..4 {
                                d4[k][m] += sg * a[2 + m];
                            }
                            out[0][k] += (sg * a[6]).max(0.0);
                            out[1][k] += (sg * a[7]).max(0.0);
                        }
                    }
                    let mut sum_div = 0.0;
                    for k in 0..l {
                        div[k] *= inv;
                        dth_h[k] *= inv;
                        sum_div += div[k];
                    }
                    let dpsdt = -sum_div;
                    *dps = dpsdt;
                    // Vertical mass flux W at half levels (Pa/s, positive = towards the ground).
                    let mut wh = [0.0f64; 17];
                    let mut acc = 0.0;
                    for k in 0..l {
                        acc += div[k];
                        wh[k + 1] = -acc - b_half[k + 1] * dpsdt;
                    }
                    wh[l] = 0.0; // exactly zero at the ground (round-off otherwise)
                    let p = ps[i];
                    let gps = [ci[CC_L * l], ci[CC_L * l + 1], ci[CC_L * l + 2]];
                    let (sin_lat, cos_lat) = (x[2], (1.0 - x[2] * x[2]).max(0.0).sqrt());
                    for k in 0..l {
                        let ik = i * l + k;
                        let oc = &ci[CC_L * k..CC_L * (k + 1)];
                        let r = &pc[PA_L * k..PA_L * (k + 1)];
                        let (th_k, tt_k, tp_k, cpg, mass, pk) = (r[0], r[1], r[4], r[5], r[6], r[7]);
                        let zeta = oc[0];
                        let gb = [oc[1], oc[2], oc[3]];
                        let vk = v[ik];
                        // Vertical advection of momentum (Simmons-Burridge form).
                        let mut va = [0.0; 3];
                        if k + 1 < l {
                            va = add(va, scale(sub(v[ik + 1], vk), wh[k + 1]));
                        }
                        if k > 0 {
                            va = add(va, scale(sub(vk, v[ik - 1]), wh[k]));
                        }
                        va = scale(va, 0.5 / mass);
                        let mut acc_v = sub(scale(cross(x, vk), -(zeta + f[i])), gb);
                        // -R T' (grad p / p) = -R T' (cpg ps) grad(ln ps): the reference part is
                        // balanced (see pass A). Written on grad(ln ps) so that pure sigma layers
                        // (cpg ps = 1) get the linear operator on ln ps, which an isothermal
                        // atmosphere over the relief balances exactly.
                        acc_v = sub(acc_v, scale(gps, rd * cpg * p * if pgf_ref { tp_k } else { tt_k }));
                        acc_v = sub(acc_v, va);
                        acc_v = add(acc_v, scale([d4[k][0], d4[k][1], d4[k][2]], sgn * nu_v * inv));
                        // Temperature tendency (K/s) from forcing and diffusion.
                        let mut dtemp = sgn * nu_t * d4[k][3] * inv;
                        if let Some(h) = relax {
                            let (teq, kt, kv) = h.terms(sin_lat, cos_lat, sin0, pk, pk / p, k + 1 == l);
                            dtemp -= kt * (tt_k - teq);
                            acc_v = sub(acc_v, scale(vk, kv));
                        }
                        dv[k] = tangent(acc_v, x);
                        // Theta: horizontal flux divergence, centred vertical fluxes, heating.
                        let th_up = if k > 0 { 0.5 * (pc[PA_L * (k - 1)] + th_k) } else { 0.0 };
                        let th_dn = if k + 1 < l { 0.5 * (th_k + pc[PA_L * (k + 1)]) } else { 0.0 };
                        let pi_k = tt_k / th_k;
                        dth[k] = -dth_h[k] - (wh[k + 1] * th_dn - wh[k] * th_up) + mass * dtemp / pi_k;
                    }
                    // Dust: vertical upwind fluxes, then the outflow limiter (a cell may not export
                    // more than it holds during one forward-Euler stage).
                    for t in 0..ntr {
                        let vfl = &mut vfl[t * (l + 1)..(t + 1) * (l + 1)];
                        let ratio = &mut ratio[t * l..(t + 1) * l];
                        let qo = PA_Q[t];
                        vfl[0] = 0.0;
                        vfl[l] = 0.0;
                        for k in 1..l {
                            // half level between layer k-1 (above) and k (below)
                            let wv = wh[k];
                            vfl[k] = if wv >= 0.0 { wv * pc[PA_L * (k - 1) + qo] } else { wv * pc[PA_L * k + qo] };
                        }
                        for k in 0..l {
                            let mut o = out[t][k] * inv;
                            if vfl[k + 1] > 0.0 {
                                o += vfl[k + 1];
                            }
                            if vfl[k] < 0.0 {
                                o -= vfl[k];
                            }
                            let loss = o * dt;
                            let avail = ms[t][i * l + k].max(0.0) * (1.0 - 1e-10);
                            ratio[k] = if loss > avail { avail / loss } else { 1.0 };
                        }
                    }
                });
        }

        // ---- Pass F (edges): scale outgoing horizontal tracer fluxes by the upwind ratio.
        // ---- Pass G (cells): dust tendency.
        // Layouts: ratio[(i * 2 + t) * l + k], vflux[(i * 2 + t) * (l + 1) + k], tlim[(e * 2 + t) * l + k].
        if dust {
            let (ed, ratio) = (&w.ed, &w.ratio);
            w.tlim.par_chunks_mut(2 * l).enumerate().for_each(|(e, tl)| {
                for t in 0..ntr {
                    for k in 0..l {
                        let tr = ed[e * ED_L * l + ED_L * k + ED_Q[t]];
                        let u = if tr >= 0.0 { gr.e_i[e] } else { gr.e_j[e] };
                        tl[t * l + k] = tr * ratio[(u * 2 + t) * l + k];
                    }
                }
            });
            let (tlim, vflux) = (&w.tlim, &w.vflux);
            w.dm.par_chunks_mut(l).zip(w.dm2.par_chunks_mut(l)).enumerate().for_each(|(i, (dm, dm2))| {
                let inv = 1.0 / gr.area[i];
                for t in 0..ntr {
                    let dmt: &mut [f64] = if t == 0 { &mut *dm } else { &mut *dm2 };
                    let vf = &vflux[(i * 2 + t) * (l + 1)..(i * 2 + t + 1) * (l + 1)];
                    let rt = &ratio[(i * 2 + t) * l..(i * 2 + t + 1) * l];
                    for k in 0..l {
                        let mut s_ = 0.0;
                        for s in gr.c_start[i]..gr.c_start[i + 1] {
                            s_ += gr.c_sign[s] * tlim[(gr.c_edge[s] * 2 + t) * l + k];
                        }
                        // Limited vertical fluxes: owned by the upwind layer of the same column.
                        let top = if k > 0 {
                            let r = if vf[k] >= 0.0 { rt[k - 1] } else { rt[k] };
                            vf[k] * r
                        } else {
                            0.0
                        };
                        let bot = if k + 1 < l {
                            let r = if vf[k + 1] >= 0.0 { rt[k] } else { rt[k + 1] };
                            vf[k + 1] * r
                        } else {
                            0.0
                        };
                        dmt[k] = -s_ * inv - (bot - top);
                    }
                }
            });
        } else {
            w.dm.iter_mut().for_each(|x| *x = 0.0);
            w.dm2.iter_mut().for_each(|x| *x = 0.0);
        }
    }

    /// out = a * x + b * (y + dt * k)
    #[allow(clippy::too_many_arguments)]
    fn combine(&self, out: &mut PeState, a: f64, x: &PeState, b: f64, y: &PeState, dt: f64, w: &PeWork) {
        out.ps.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * x.ps[i] + b * (y.ps[i] + dt * w.dps[i]));
        out.th.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * x.th[i] + b * (y.th[i] + dt * w.dth[i]));
        if x.dust_active {
            out.m.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * x.m[i] + b * (y.m[i] + dt * w.dm[i]));
            if self.ntr > 1 {
                out.m2.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * x.m2[i] + b * (y.m2[i] + dt * w.dm2[i]));
            }
        }
        out.v
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, o)| *o = add(scale(x.v[i], a), scale(add(y.v[i], scale(w.dv[i], dt)), b)));
        out.dust_active = x.dust_active;
    }

    /// One SSP-RK3 step of length dt.
    pub fn step(&self, st: &mut PeState, dt: f64, w: &mut PeWork) {
        let t = st.time;
        self.rhs(st, t, dt, w);
        let mut s1 = std::mem::replace(&mut w.s1, PeState::zeros(0, 0));
        let mut s2 = std::mem::replace(&mut w.s2, PeState::zeros(0, 0));
        self.combine(&mut s1, 0.0, st, 1.0, st, dt, w);
        self.rhs(&s1, t + dt, dt, w);
        self.combine(&mut s2, 0.75, st, 0.25, &s1, dt, w);
        self.rhs(&s2, t + 0.5 * dt, dt, w);
        // In place: st = 1/3 st + 2/3 (s2 + dt k)
        let (a, b) = (1.0 / 3.0, 2.0 / 3.0);
        st.ps.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * *o + b * (s2.ps[i] + dt * w.dps[i]));
        st.th.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * *o + b * (s2.th[i] + dt * w.dth[i]));
        if st.dust_active {
            st.m.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * *o + b * (s2.m[i] + dt * w.dm[i]));
            if self.ntr > 1 {
                st.m2.par_iter_mut().enumerate().for_each(|(i, o)| *o = a * *o + b * (s2.m2[i] + dt * w.dm2[i]));
            }
        }
        st.v
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, o)| *o = add(scale(*o, a), scale(add(s2.v[i], scale(w.dv[i], dt)), b)));
        st.time += dt;
        w.s1 = s1;
        w.s2 = s2;
    }

    /// Fastest signal per cell: Lamb-wave speed sqrt(cp/cv R T) of the warmest layer + wind.
    pub fn cfl_dt(&self, st: &PeState, courant: f64) -> f64 {
        let gr = &self.grid;
        let l = self.nl;
        let gamma = self.c.cp / (self.c.cp - self.c.rd);
        let speed: Vec<f64> = (0..gr.n_cells())
            .into_par_iter()
            .map(|i| {
                let mut s: f64 = 0.0;
                let ps = st.ps[i];
                let col = self.column(ps);
                for k in 0..l {
                    let t = (st.th[i * l + k] / col.dp[k] * (col.pf[k] / self.c.p0).powf(self.c.kappa)).max(1.0);
                    let vk = st.v[i * l + k];
                    s = s.max((gamma * self.c.rd * t).sqrt() + dot(vk, vk).sqrt());
                }
                if s.is_nan() {
                    f64::INFINITY
                } else {
                    s
                }
            })
            .collect();
        let r = (0..gr.n_edges())
            .into_par_iter()
            .map(|e| gr.e_d[e] / speed[gr.e_i[e]].max(speed[gr.e_j[e]]).max(1e-12))
            .reduce(|| f64::MAX, f64::min);
        courant * r
    }

    /// Advances one chunk of length `chunk_s` in equal CFL-limited steps.
    /// Then checks the state (finite values, surface pressure within bounds).
    pub fn advance_chunk(&self, st: &mut PeState, chunk_s: f64, courant: f64, w: &mut PeWork) -> Result<usize, String> {
        let dtc = self.cfl_dt(st, courant);
        if !(dtc.is_finite() && dtc > 0.0) {
            return Err(format!("invalid CFL time step at t = {:.0} s", st.time));
        }
        let n = (chunk_s / dtc).ceil().max(1.0) as usize;
        if n > 100_000 {
            return Err(format!("CFL time step collapsed ({dtc:.3e} s) at t = {:.0} s", st.time));
        }
        let dt = chunk_s / n as f64;
        for _ in 0..n {
            self.step(st, dt, w);
            if let Some(ph) = &self.phys {
                ph.step(self, st, dt, w);
            }
        }
        st.chunk += 1;
        st.time = st.chunk as f64 * chunk_s;
        // Test hook (see `fault_chunk`).
        if self.fault_count.load(Ordering::Relaxed) > 0 && st.chunk == self.fault_chunk.load(Ordering::Relaxed) {
            self.fault_count.fetch_sub(1, Ordering::Relaxed);
            st.ps[0] = f64::NAN;
        }
        self.check(st)?;
        Ok(n)
    }

    /// Advances one chunk under the roll-back guard. The time step is divided by `st.dt_div`
    /// while `st.chunk < st.reduced_until`. On a failure the state goes back to the last good
    /// one (the last model-day boundary), the step is halved (then quartered) for the next
    /// model day, and the run goes on; the failure number `max_fails` in a row is an error.
    pub fn advance_guarded(
        &self,
        st: &mut PeState,
        g: &mut Guard,
        chunk_s: f64,
        courant: f64,
        chunks_per_day: u64,
        w: &mut PeWork,
    ) -> Result<ChunkOutcome, String> {
        let reduced = st.chunk < st.reduced_until;
        let c_eff = if reduced { courant / st.dt_div.max(1) as f64 } else { courant };
        match self.advance_chunk(st, chunk_s, c_eff, w) {
            Ok(n) => {
                if st.chunk >= st.reduced_until {
                    st.dt_div = 1;
                }
                if st.chunk % chunks_per_day == 0 {
                    g.last_good.clone_from(st);
                    if st.chunk >= st.reduced_until {
                        g.fails = 0;
                    }
                }
                Ok(ChunkOutcome::Done(n))
            }
            Err(e) => {
                g.fails += 1;
                let failed_day = st.time / (chunk_s * chunks_per_day as f64);
                if g.fails >= g.max_fails {
                    let msg = format!("guard: failure {} in a row at day {failed_day:.3} ({e}): stopping for good", g.fails);
                    g.events.push(msg.clone());
                    return Err(msg);
                }
                // The event journal survives the roll-back: an event injected after the last
                // good state is applied again (from its own start time) when the day is re-run.
                let journal = std::mem::take(&mut st.events);
                st.clone_from(&g.last_good);
                st.events = journal;
                st.dt_div = 1u32 << g.fails;
                st.reduced_until = st.chunk + chunks_per_day;
                let msg = format!(
                    "guard: failure {} in a row at day {failed_day:.3} ({e}); back to day {:.3}, time step / {} until day {:.3}",
                    g.fails,
                    st.time / (chunk_s * chunks_per_day as f64),
                    st.dt_div,
                    st.reduced_until as f64 / chunks_per_day as f64
                );
                g.events.push(msg.clone());
                Ok(ChunkOutcome::RolledBack(msg))
            }
        }
    }

    /// Guard: every value finite, every surface pressure within `ps_bounds`, winds below `max_wind`.
    pub fn check(&self, st: &PeState) -> Result<(), String> {
        let (lo, hi) = self.ps_bounds;
        let bad_ps = st.ps.par_iter().position_first(|&p| !(p.is_finite() && p > lo && p < hi));
        if let Some(i) = bad_ps {
            return Err(format!(
                "surface pressure out of bounds at t = {:.0} s: cell {} (lon {:.1}, lat {:.1}) ps = {:.1} Pa",
                st.time,
                i,
                self.grid.lon[i].to_degrees(),
                self.grid.lat[i].to_degrees(),
                st.ps[i]
            ));
        }
        let vmax2 = self.max_wind * self.max_wind;
        let ok = st.th.par_iter().all(|x| x.is_finite() && *x > 0.0)
            && st.m.par_iter().all(|x| x.is_finite())
            && st.m2.par_iter().all(|x| x.is_finite())
            && st.tg.par_iter().all(|x| x.is_finite() && *x >= 0.0)
            && st.v.par_iter().all(|x| x[0].is_finite() && x[1].is_finite() && x[2].is_finite() && dot(*x, *x) < vmax2);
        if !ok {
            return Err(format!("non-finite, negative or runaway values at t = {:.0} s", st.time));
        }
        Ok(())
    }

    pub fn diag(&self, st: &PeState) -> PeDiag {
        let gr = &self.grid;
        let l = self.nl;
        let mut d = PeDiag {
            air_mass: 0.0,
            dust_mass: 0.0,
            min_ps: f64::MAX,
            max_ps: f64::MIN,
            max_speed: 0.0,
            min_t: f64::MAX,
            max_t: f64::MIN,
            min_dust: f64::MAX,
            finite: true,
            dust2_mass: 0.0,
            min_dust2: f64::MAX,
        };
        for i in 0..gr.n_cells() {
            let a = gr.area[i];
            d.air_mass += a * st.ps[i] / self.c.g;
            d.min_ps = d.min_ps.min(st.ps[i]);
            d.max_ps = d.max_ps.max(st.ps[i]);
            for k in 0..l {
                let ik = i * l + k;
                d.dust_mass += a * st.m[ik] / self.c.g;
                d.min_dust = d.min_dust.min(st.m[ik]);
                d.dust2_mass += a * st.m2[ik] / self.c.g;
                d.min_dust2 = d.min_dust2.min(st.m2[ik]);
                d.max_speed = d.max_speed.max(dot(st.v[ik], st.v[ik]).sqrt());
                let t = self.temperature(st, i, k);
                d.min_t = d.min_t.min(t);
                d.max_t = d.max_t.max(t);
                if !(t.is_finite() && st.v[ik][0].is_finite() && st.m[ik].is_finite()) {
                    d.finite = false;
                }
            }
            if !st.ps[i].is_finite() {
                d.finite = false;
            }
        }
        d
    }

    // ------------------------------------------------------------ restart files

    /// Writes the full state (bit-exact) to `path`, with the vertical coefficients so that a
    /// file made for other levels is refused.
    pub fn write_restart(&self, st: &PeState, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let b = self.restart_bytes(st);
        // Write then rename, so that a crash never leaves a half-written restart file.
        let tmp = path.with_extension("tmp");
        std::fs::File::create(&tmp)?.write_all(&b)?;
        std::fs::rename(&tmp, path)
    }

    /// The full state (bit-exact) as bytes (format `DSPE0003`), for any storage.
    pub fn restart_bytes(&self, st: &PeState) -> Vec<u8> {
        let mut b: Vec<u8> = Vec::with_capacity(8 * (st.ps.len() + 5 * st.th.len()) + 256);
        let u = |b: &mut Vec<u8>, x: u64| b.extend_from_slice(&x.to_le_bytes());
        b.extend_from_slice(b"DSPE0003");
        u(&mut b, self.grid.level as u64);
        u(&mut b, self.nl as u64);
        u(&mut b, st.ps.len() as u64);
        u(&mut b, st.chunk);
        u(&mut b, st.time.to_bits());
        u(&mut b, st.dust_active as u64);
        u(&mut b, st.dt_div as u64);
        u(&mut b, st.reduced_until);
        for x in &st.budget {
            u(&mut b, x.to_bits());
        }
        u(&mut b, st.events.len() as u64);
        for x in self.a_half.iter().chain(&self.b_half) {
            u(&mut b, x.to_bits());
        }
        for x in &st.ps {
            u(&mut b, x.to_bits());
        }
        for v in &st.v {
            for x in v {
                u(&mut b, x.to_bits());
            }
        }
        for x in st.th.iter().chain(&st.m).chain(&st.m2).chain(&st.tg).chain(&st.t2) {
            u(&mut b, x.to_bits());
        }
        for ev in &st.events {
            for x in ev.to_array() {
                u(&mut b, x.to_bits());
            }
        }
        b
    }

    pub fn read_restart(&self, path: &std::path::Path) -> std::io::Result<PeState> {
        self.state_from_bytes(&std::fs::read(path)?)
    }

    /// The state from bytes written by `restart_bytes`; refused if made for another grid, layer
    /// count or vertical levels, or of the wrong size.
    pub fn state_from_bytes(&self, b: &[u8]) -> std::io::Result<PeState> {
        let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
        if b.len() < 112 || &b[0..8] != b"DSPE0003" {
            return Err(bad("not a DSPE0003 restart file (files of steps 3 and 3b, DSPE0001 / DSPE0002, lack the step-4 state)"));
        }
        let u = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let (level, nl, n) = (u(8), u(16) as usize, u(24) as usize);
        if level != self.grid.level as u64 || nl != self.nl || n != self.grid.n_cells() {
            return Err(bad("restart file made for another grid or layer count"));
        }
        let nev = u(104) as usize;
        let ne = crate::phys::Event::N_FIELDS;
        let head = 112 + 16 * (nl + 1);
        let need = head + 8 * (n + 3 * n * nl + 3 * n * nl + 2 * n + ne * nev);
        if b.len() != need {
            return Err(bad("restart file has the wrong size"));
        }
        for (q, x) in self.a_half.iter().chain(&self.b_half).enumerate() {
            if u(112 + 8 * q) != x.to_bits() {
                return Err(bad("restart file made for other vertical levels"));
            }
        }
        let mut st = PeState::zeros(n, nl);
        st.chunk = u(32);
        st.time = f64::from_bits(u(40));
        st.dust_active = u(48) != 0;
        st.dt_div = u(56) as u32;
        st.reduced_until = u(64);
        for q in 0..4 {
            st.budget[q] = f64::from_bits(u(72 + 8 * q));
        }
        let mut o = head;
        let mut next = || {
            let x = f64::from_bits(u64::from_le_bytes(b[o..o + 8].try_into().unwrap()));
            o += 8;
            x
        };
        for x in st.ps.iter_mut() {
            *x = next();
        }
        for v in st.v.iter_mut() {
            for x in v.iter_mut() {
                *x = next();
            }
        }
        for x in st.th.iter_mut() {
            *x = next();
        }
        for x in st.m.iter_mut().chain(st.m2.iter_mut()).chain(st.tg.iter_mut()).chain(st.t2.iter_mut()) {
            *x = next();
        }
        for _ in 0..nev {
            let mut a = [0.0; crate::phys::Event::N_FIELDS];
            for x in a.iter_mut() {
                *x = next();
            }
            st.events.push(crate::phys::Event::from_array(&a));
        }
        Ok(st)
    }
}
