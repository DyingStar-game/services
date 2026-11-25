//! Column physics of the primitive-equation model, applied after every dynamics step (operator
//! splitting, each column on its own: results do not depend on the number of threads).
//!
//! The physics is a **suite of components**, each behind the [`ColumnProcess`] trait and each
//! switchable from the settings file without touching the dynamical core (`pe`) or the other
//! components. A column goes through four phases, and in each phase through the components in
//! the suite's order:
//!
//!  * `heat`    : sources of heat. The suite has computed the column (temperatures, heights), the
//!                surface layer (exchange coefficient, friction velocity) and the sunlight at the
//!                top; components add K to `dtk[k]` or W/m^2 to `q_low` (the lowest layer), then
//!                the suite applies them to the potential temperature.
//!  * `sources` : sources of dust (lifting by the wind, events).
//!  * `adjust`  : rearrangements of the column (lid relaxation, dry convective adjustment).
//!  * `sinks`   : losses and relaxations of the dust (settling and deposition, dust sea).
//!
//! The suite then records the per-cell diagnostics (the published "ingredients") and the
//! planet-wide dust budget. Components (see their files): [`ground::Ground`] (energy balance of
//! the ground and the day-night cycle), [`dust::DustRadiation`], [`dust::DustLifting`],
//! [`dust::DustSettling`], [`sea::DustSea`], [`lid::Lid`], [`convection::DryConvection`],
//! [`events::Events`]. The dust optics of the Mie option live in [`mie`] and [`optics`].
//!
//! Formulas from the textbooks and papers cited in the research notes (Stull, Practical
//! Meteorology; Kok et al. 2012; Noilhan & Planton 1989 for force-restore; Davies 1945 for the
//! slip correction). No code copied or translated from any model.

pub mod convection;
pub mod dust;
pub mod events;
pub mod ground;
pub mod lid;
pub mod mie;
pub mod optics;
pub mod sea;

pub use events::Event;

use crate::body::Seasons;
use crate::pe::{Column, PeConsts, PeModel, PeState, PeWork};
use crate::relax::Relaxation;
use crate::vec3::*;
use rayon::prelude::*;
use std::any::Any;

/// Stefan-Boltzmann constant (W m^-2 K^-4).
pub const SIGMA_SB: f64 = 5.670374e-8;
/// Von Karman constant.
pub const KARMAN: f64 = 0.4;

/// Everything a component may read or change in one column during one physics step.
pub struct ColumnCtx<'a> {
    pub i: usize,
    pub l: usize,
    /// Index of the lowest layer (l - 1).
    pub kb: usize,
    pub dt: f64,
    pub c: &'a PeConsts,
    pub col: &'a Column,
    /// Unit position, sine and cosine of the latitude, surface pressure (Pa).
    pub x: V3,
    pub sin_lat: f64,
    pub cos_lat: f64,
    pub ps: f64,
    /// Potential temperature (K) of each layer: what the components change.
    pub theta: [f64; 16],
    /// Exner function and temperature (K) at the start of the step.
    pub pik: [f64; 16],
    pub tk: [f64; 16],
    /// Absolute altitudes (m) of the half levels (0 = top, infinite; l = ground) and full levels.
    pub zh: [f64; 17],
    pub zf: [f64; 16],
    /// Ground altitude, Exner function at the ground, surface-air temperature, density.
    pub zg: f64,
    pub pis: f64,
    pub ta: f64,
    pub rho_s: f64,
    /// Height of the lowest full level above the ground and ln(z1 / z0).
    pub z1: f64,
    pub ln1: f64,
    /// Neutral exchange coefficient and wind speed of the lowest layer.
    pub cdn: f64,
    pub v1: f64,
    /// Dust mass per layer (dp q) of the fine and coarse classes.
    pub m: &'a mut [f64],
    pub m2: &'a mut [f64],
    /// Ground skin and deep temperatures (K).
    pub tg: &'a mut f64,
    pub t2: &'a mut f64,
    /// Dust is transported and acted on (two tracers active).
    pub dust_on: bool,
    /// Surface layer: ground temperature at the start of the step, convective velocity, gust
    /// speed, bulk Richardson number, stability factor, heat-exchange coefficient (W m^-2 K^-1),
    /// friction velocity and surface stress.
    pub tg0: f64,
    pub wstar: f64,
    pub veff: f64,
    pub rib: f64,
    pub fst: f64,
    pub hcoef: f64,
    pub ustar: f64,
    pub tau_s: f64,
    /// Sunlight: cosine of the zenith angle, flux at the top, flux reaching the ground, flux
    /// absorbed by the dust in the air (W/m^2), dust optical depth per layer and of the column.
    pub cosz: f64,
    pub toa: f64,
    pub sw_down: f64,
    pub sw_air: f64,
    /// Net sunlight absorbed by the ground when the dust radiation computed it (Mie optics, with
    /// the reflections between the ground and the dust layer); None: the ground takes
    /// (1 - albedo) x sw_down.
    pub sw_ground: Option<f64>,
    pub tau_k: [f64; 16],
    pub tau_c: f64,
    /// Heating accumulators: K per layer, W/m^2 into the lowest layer.
    pub dtk: [f64; 16],
    pub q_low: f64,
    /// Results of the ground: new skin temperature, its change, sensible heat flux, sunlight
    /// absorbed by the ground; and the dust lifting rate (kg m^-2 s^-1).
    pub tg1: f64,
    pub dtg: f64,
    pub hflux: f64,
    pub swg: f64,
    pub lift: f64,
    /// Dust budget of this step (kg/m^2): lifted, deposited, added by the sea, added by events.
    pub bud: [f64; 4],
    /// Event sources of this cell: heat (W/m^2), fine and coarse dust (kg m^-2 s^-1).
    pub ev: [f64; 3],
    /// Temperature relaxation of the dynamics (the lid shifts its target) and the sine of the
    /// subsolar latitude.
    pub relax: Option<&'a dyn Relaxation>,
    pub sin0: f64,
}

/// One swappable piece of the column physics. Every hook has an empty default: a component
/// implements only the phases it acts in.
pub trait ColumnProcess: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &'static str;
    /// Once per step before the columns (e.g. spreading the events over the cells).
    fn prepare(&self, _m: &PeModel, _st: &PeState, _t_end: f64, _dt: f64, _ev: &mut [f64]) {}
    fn heat(&self, _c: &mut ColumnCtx) {}
    fn sources(&self, _c: &mut ColumnCtx) {}
    fn adjust(&self, _c: &mut ColumnCtx) {}
    fn sinks(&self, _c: &mut ColumnCtx) {}
    /// Typed access for the outputs and the tests.
    fn as_any(&self) -> &dyn Any;
}

/// The sun as seen from the body: overhead flux, subsolar longitude drifting westward once per
/// solar day, subsolar latitude from the seasons.
#[derive(Clone, Debug)]
pub struct Sun {
    /// Sunlight at the top of the atmosphere, sun overhead (W/m^2).
    pub s0: f64,
    /// Solar day (s), signed; infinite = the sun stands still.
    pub solar_day_s: f64,
    /// Subsolar longitude at t = 0 (degrees).
    pub lon0_deg: f64,
    pub seasons: Seasons,
    /// The real orbit and spin: when given, declination, sub-stellar longitude and distance come
    /// from it (and `solar_day_s`, `lon0_deg`, `seasons` are not used).
    pub ephemeris: Option<std::sync::Arc<crate::ephemeris::Ephemeris>>,
    /// Test hook: a steady sun of this flux (W/m^2, top of the atmosphere) overhead everywhere.
    pub uniform: Option<f64>,
}

impl Sun {
    /// Declination (radians), sub-stellar longitude (radians) and overhead flux (W/m^2) at model
    /// time `t`.
    pub fn position(&self, t: f64) -> (f64, f64, f64) {
        match &self.ephemeris {
            Some(e) => {
                let p = e.at(t);
                (p.decl_rad, p.sub_lon_rad, self.s0 * p.flux_factor)
            }
            None => (self.seasons.subsolar_lat(t), self.lon0_deg.to_radians() - 2.0 * std::f64::consts::PI * t / self.solar_day_s, self.s0),
        }
    }
}

/// Surface-layer parameters (shared by the ground and the dust).
#[derive(Clone, Debug)]
pub struct SurfaceLayer {
    /// Roughness length (m).
    pub z0: f64,
    /// Smallest wind used by the exchange formulas (m/s): sub-grid gusts.
    pub v_floor: f64,
    /// Lower bound of the stable-stratification factor of the exchange coefficient.
    pub stable_floor: f64,
}

/// The column physics: the shared context builders and the ordered components.
#[derive(Debug)]
pub struct PhysicsSuite {
    pub sun: Sun,
    pub surface: SurfaceLayer,
    /// Mass extinction coefficients (m^2/kg) of the two dust classes (zero without dust).
    pub kext: [f64; 2],
    /// Altitude (m) where the lid-strength diagnostic is read (the lid, else the sea top).
    pub lid_probe_z: Option<f64>,
    pub components: Vec<Box<dyn ColumnProcess>>,
}

/// Indices of the per-cell diagnostics of the last physics step.
pub mod di {
    /// Sensible heat flux from the ground to the air (W/m^2).
    pub const H: usize = 0;
    /// Ground skin minus surface-air temperature (K).
    pub const DT_GA: usize = 1;
    /// Boundary-layer depth above the ground (m).
    pub const ZI: usize = 2;
    /// Tendency of the ground temperature (K/h).
    pub const DTG: usize = 3;
    /// Surface wind stress (Pa).
    pub const TAU: usize = 4;
    /// Dust lifting rate (kg m^-2 s^-1).
    pub const LIFT: usize = 5;
    /// Lid strength: potential temperature 500 m above minus 500 m below the lid bottom (K).
    pub const LID: usize = 6;
    pub const USTAR: usize = 7;
    pub const WSTAR: usize = 8;
    /// Dust optical depth of the column (visible).
    pub const TAU_DUST: usize = 9;
    pub const TG: usize = 10;
    pub const TA: usize = 11;
    /// Sunlight absorbed by the ground (W/m^2).
    pub const SWG: usize = 12;
    pub const COSZ: usize = 13;
    /// Fine and coarse dust columns (kg/m^2).
    pub const COL_FINE: usize = 14;
    pub const COL_COARSE: usize = 15;
    /// Sunlight absorbed by the dust in the air (W/m^2).
    pub const SW_AIR: usize = 16;
    pub const N: usize = 17;
}

/// Scratch and diagnostics of the physics.
pub struct PhysWork {
    /// diag[i * di::N + q]
    pub diag: Vec<f64>,
    /// Dust budget of the last step per cell (kg/m^2): lifted, deposited, sea, events.
    bud: Vec<f64>,
    /// Event sources of the current step per cell: heat (W/m^2), fine and coarse dust.
    ev: Vec<f64>,
}

impl PhysWork {
    pub fn new(n: usize) -> PhysWork {
        PhysWork { diag: vec![0.0; n * di::N], bud: vec![0.0; n * 4], ev: vec![0.0; n * 3] }
    }
    pub fn get(&self, q: usize) -> Vec<f64> {
        let n = self.diag.len() / di::N;
        (0..n).map(|i| self.diag[i * di::N + q]).collect()
    }
}

/// Fraction of the interval [a, b] that lies in [lo, hi].
#[inline]
pub(crate) fn overlap(a: f64, b: f64, lo: f64, hi: f64) -> f64 {
    if b <= a {
        return 0.0;
    }
    ((b.min(hi) - a.max(lo)).max(0.0)) / (b - a)
}

impl PhysicsSuite {
    /// The first component of type T, if the suite has one.
    pub fn find<T: 'static>(&self) -> Option<&T> {
        self.components.iter().find_map(|c| c.as_any().downcast_ref::<T>())
    }

    /// Names of the active components, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.components.iter().map(|c| c.name()).collect()
    }

    /// Column physics of one step of length dt ending at st.time (see the module header).
    pub fn step(&self, model: &PeModel, st: &mut PeState, dt: f64, w: &mut PeWork) {
        let gr = &model.grid;
        let l = model.nl;
        let kb = l - 1;
        let c = &model.c;
        let (g, rd, cp, kappa, p0) = (c.g, c.rd, c.cp, c.kappa, c.p0);
        let t_end = st.time;
        let t_mid = t_end - 0.5 * dt;
        let n = gr.n_cells();

        w.ph.ev.iter_mut().for_each(|x| *x = 0.0);
        for comp in &self.components {
            comp.prepare(model, st, t_end, dt, &mut w.ph.ev);
        }

        // Sun: declination from the seasons, subsolar longitude from the solar day.
        let sp = &self.sun;
        let (decl, lon_s, s0) = sp.position(t_mid);
        let sin0 = decl.sin();
        let sun = [decl.cos() * lon_s.cos(), decl.cos() * lon_s.sin(), decl.sin()];
        let sl = &self.surface;
        let lnz = |z1: f64| (z1.max(2.0 * sl.z0) / sl.z0).ln();
        let relax = model.relax.as_deref();

        let ps = &st.ps;
        let v = &st.v;
        let phis = &model.phis;
        let ev = &w.ph.ev;
        let dust_on = st.dust_active && model.ntr > 1;
        st.th
            .par_chunks_mut(l)
            .zip(st.m.par_chunks_mut(l))
            .zip(st.m2.par_chunks_mut(l))
            .zip(st.tg.par_iter_mut())
            .zip(st.t2.par_iter_mut())
            .zip(w.ph.diag.par_chunks_mut(di::N))
            .zip(w.ph.bud.par_chunks_mut(4))
            .enumerate()
            .for_each(|(i, ((((((thc, mc), m2c), tg), t2), dg), bud))| {
                let p = ps[i];
                let col = model.column(p);
                let x = gr.x[i];
                let (sin_lat, cos_lat) = (x[2], (1.0 - x[2] * x[2]).max(0.0).sqrt());
                let mut theta = [0.0f64; 16];
                let mut pik = [0.0f64; 16];
                let mut tk = [0.0f64; 16];
                for k in 0..l {
                    theta[k] = thc[k] / col.dp[k];
                    pik[k] = (col.pf[k] / p0).powf(kappa);
                    tk[k] = theta[k] * pik[k];
                }
                // Absolute altitudes of the half levels (zh) and full levels (zf), hydrostatic.
                let mut zh = [0.0f64; 17];
                let mut zf = [0.0f64; 16];
                zh[l] = phis[i] / g;
                for k in (0..l).rev() {
                    zf[k] = zh[k + 1] + col.alpha[k] * rd * tk[k] / g;
                    zh[k] = if k > 0 { zh[k + 1] + rd * tk[k] / g * col.lnr[k] } else { f64::INFINITY };
                }
                let zg = zh[l];
                let pis = (p / p0).powf(kappa);
                let ta = theta[kb] * pis;
                let rho_s = p / (rd * ta);
                let z1 = zf[kb] - zg;
                let ln1 = lnz(z1);
                let cdn = (KARMAN / ln1).powi(2);
                let v1 = norm(v[i * l + kb]);
                if *tg <= 0.0 {
                    *tg = ta;
                    *t2 = ta;
                }
                let tg0 = *tg;

                // Surface layer: gust velocity (wind, floor, convective velocity of the current
                // state), stability, heat-exchange coefficient, friction velocity. The friction
                // velocity carries the same stability factor as the heat exchange (capped at 2
                // when unstable): stable nights are calm at the ground, daytime mixing brings the
                // wind down.
                let h0 = rho_s * cp * cdn * (v1 * v1 + sl.v_floor * sl.v_floor).sqrt() * (tg0 - ta);
                let zi0 = (zh[kb] - zg).max(z1);
                let wstar = if h0 > 0.0 { (g / ta * zi0.max(parcel_top(&theta, &zf, kb, zg)) * h0 / (rho_s * cp)).cbrt() } else { 0.0 };
                let veff = (v1 * v1 + wstar * wstar + sl.v_floor * sl.v_floor).sqrt();
                let thg = tg0 / pis;
                let rib = g * z1 * (theta[kb] - thg) / (0.5 * (theta[kb] + thg) * veff * veff);
                let fst = if rib >= 0.0 { (1.0 / (1.0 + 10.0 * rib).powi(2)).max(sl.stable_floor) } else { (1.0 - 16.0 * rib).sqrt().min(5.0) };
                let hcoef = rho_s * cp * cdn * veff * fst;
                let fm = if rib >= 0.0 { fst } else { fst.min(2.0) };
                let ustar = KARMAN * veff / ln1 * fm.sqrt();
                let tau_s = rho_s * ustar * ustar;

                // Sunlight at the top and the dust optical depth.
                let cosz = if sp.uniform.is_some() { 1.0 } else { dot(x, sun) };
                let toa = sp.uniform.unwrap_or(s0 * cosz.max(0.0));
                let mut tau_k = [0.0f64; 16];
                let mut tau_c = 0.0;
                for k in 0..l {
                    tau_k[k] = (self.kext[0] * mc[k] + self.kext[1] * m2c[k]) / g;
                    tau_c += tau_k[k];
                }

                let mut cx = ColumnCtx {
                    i,
                    l,
                    kb,
                    dt,
                    c,
                    col: &col,
                    x,
                    sin_lat,
                    cos_lat,
                    ps: p,
                    theta,
                    pik,
                    tk,
                    zh,
                    zf,
                    zg,
                    pis,
                    ta,
                    rho_s,
                    z1,
                    ln1,
                    cdn,
                    v1,
                    m: mc,
                    m2: m2c,
                    tg,
                    t2,
                    dust_on,
                    tg0,
                    wstar,
                    veff,
                    rib,
                    fst,
                    hcoef,
                    ustar,
                    tau_s,
                    cosz,
                    toa,
                    sw_down: toa,
                    sw_air: 0.0,
                    sw_ground: None,
                    tau_k,
                    tau_c,
                    dtk: [0.0; 16],
                    q_low: 0.0,
                    tg1: tg0,
                    dtg: 0.0,
                    hflux: 0.0,
                    swg: 0.0,
                    lift: 0.0,
                    bud: [0.0; 4],
                    ev: [ev[i * 3], ev[i * 3 + 1], ev[i * 3 + 2]],
                    relax,
                    sin0,
                };

                // Phase 1: heat, then applied to the potential temperature.
                for comp in &self.components {
                    comp.heat(&mut cx);
                }
                cx.dtk[kb] += g * cx.q_low * dt / (cp * col.dp[kb]);
                for k in 0..l {
                    cx.theta[k] += cx.dtk[k] / cx.pik[k];
                }
                // Phases 2 and 3: dust sources, rearrangements of the column.
                for comp in &self.components {
                    comp.sources(&mut cx);
                }
                for comp in &self.components {
                    comp.adjust(&mut cx);
                }
                for k in 0..l {
                    thc[k] = cx.theta[k] * col.dp[k];
                }
                // Phase 4: dust sinks and relaxations.
                for comp in &self.components {
                    comp.sinks(&mut cx);
                }
                bud.copy_from_slice(&cx.bud);

                // Diagnostics (the published ingredients).
                let theta = &cx.theta;
                let ta1 = theta[kb] * pis;
                let hflux = cx.hflux;
                let zi = if hflux > 0.0 {
                    parcel_top(theta, &zf, kb, zg).max(z1)
                } else {
                    // Stable: Zilitinkevich depth 0.4 sqrt(u* L / |f|), bounded by the lowest layer.
                    let fk = (2.0 * c.omega * x[2]).abs().max(1e-5);
                    let kin = (-hflux / (rho_s * cp)).max(1e-9);
                    let lob = ustar.powi(3) * ta / (KARMAN * g * kin);
                    (0.4 * (ustar * lob / fk).sqrt()).clamp(10.0, (zh[kb] - zg).max(z1))
                };
                let lid_k = match self.lid_probe_z {
                    Some(zl) if zl - 500.0 > zg + 0.5 * z1 => interp_z(theta, &zf, l, zl + 500.0) - interp_z(theta, &zf, l, zl - 500.0),
                    _ => 0.0,
                };
                dg[di::H] = hflux;
                dg[di::DT_GA] = cx.tg1 - ta1;
                dg[di::ZI] = zi;
                dg[di::DTG] = cx.dtg / dt * 3600.0;
                dg[di::TAU] = tau_s;
                dg[di::LIFT] = cx.lift;
                dg[di::LID] = lid_k;
                dg[di::USTAR] = ustar;
                dg[di::WSTAR] = wstar;
                dg[di::TAU_DUST] = tau_c;
                dg[di::TG] = cx.tg1;
                dg[di::TA] = ta1;
                dg[di::SWG] = cx.swg;
                dg[di::COSZ] = cosz;
                dg[di::COL_FINE] = cx.m.iter().sum::<f64>() / g;
                dg[di::COL_COARSE] = cx.m2.iter().sum::<f64>() / g;
                dg[di::SW_AIR] = cx.sw_air;
            });

        // Planet-wide budget (sequential sum: deterministic).
        for q in 0..4 {
            let mut s = 0.0;
            for i in 0..n {
                s += gr.area[i] * w.ph.bud[i * 4 + q];
            }
            st.budget[q] += s;
        }
    }
}

impl PeModel {
    /// Absolute altitudes (m) of the half levels (index 0 = top, infinite; l = ground) and of the
    /// full levels of column i (hydrostatic, Simmons-Burridge).
    pub fn column_heights(&self, st: &PeState, i: usize) -> ([f64; 17], [f64; 16]) {
        let l = self.nl;
        let c = &self.c;
        let ps = st.ps[i];
        let col = self.column(ps);
        let mut zh = [0.0f64; 17];
        let mut zf = [0.0f64; 16];
        zh[l] = self.phis[i] / c.g;
        for k in (0..l).rev() {
            let t = st.th[i * l + k] / col.dp[k] * (col.pf[k] / c.p0).powf(c.kappa);
            zf[k] = zh[k + 1] + col.alpha[k] * c.rd * t / c.g;
            zh[k] = if k > 0 { zh[k + 1] + c.rd * t / c.g * col.lnr[k] } else { f64::INFINITY };
        }
        (zh, zf)
    }
}

/// Height above the ground (m) of the top of the convective layer by the parcel method: the level
/// where the potential temperature first exceeds that of the lowest layer by 0.5 K (linear
/// interpolation between full levels).
pub(crate) fn parcel_top(theta: &[f64; 16], zf: &[f64; 16], kb: usize, zg: f64) -> f64 {
    let thp = theta[kb] + 0.5;
    for k in (0..kb).rev() {
        if theta[k] > thp {
            let f = (thp - theta[k + 1]) / (theta[k] - theta[k + 1]);
            return zf[k + 1] + f.clamp(0.0, 1.0) * (zf[k] - zf[k + 1]) - zg;
        }
    }
    zf[0] - zg
}

/// Linear interpolation of a per-layer value at absolute altitude z (clamped at the ends).
pub fn interp_z(val: &[f64], zf: &[f64], l: usize, z: f64) -> f64 {
    let kb = l - 1;
    if z <= zf[kb] {
        return val[kb];
    }
    for k in (0..kb).rev() {
        if z <= zf[k] {
            let f = (z - zf[k + 1]) / (zf[k] - zf[k + 1]);
            return val[k + 1] + f * (val[k] - val[k + 1]);
        }
    }
    val[0]
}
