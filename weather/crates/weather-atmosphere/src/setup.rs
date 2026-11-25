//! Composition root: from a body (shared data) and its settings to a ready model and its initial
//! state. This is the only place that knows which components exist; the dynamical core (`pe`)
//! and each physics component only see their own parameters.
//!
//! Every parameter is either read from the settings file or, when the file leaves it out, derived
//! from the body's data by a written rule (`Scenario::*` methods below; the rules are listed in
//! NOTES.md, section 40).

use crate::body::{Body, DataFiles, Seasons};
use crate::grid::Grid;
use crate::io;
use crate::levels;
use crate::pe::*;
use crate::phys::convection::DryConvection;
use crate::phys::dust::{DustClasses, DustLifting, DustRadiation, DustSettling, MieRadiation};
use crate::phys::optics::{k_from_reflectance, OpticsTable};
use crate::phys::events::Events;
use crate::phys::ground::Ground;
use crate::phys::lid::Lid;
use crate::phys::sea::DustSea;
use crate::phys::{ColumnProcess, PhysicsSuite, Sun, SurfaceLayer};
use crate::relax::{HeldSuarez, Relaxation};
use crate::settings::{DustOptics, Settings};
use std::path::Path;

/// A body and the settings of a run on it.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub body: Body,
    pub s: Settings,
    /// The body's real orbit and spin, tied to Unix time (the service sets it): the sun and the
    /// seasons then follow the real sky. None = the idealised calendar (t = 0 = northern spring
    /// equinox, sun over `ground.sun_lon0_deg`), as in the offline runs and the regression tests.
    pub ephemeris: Option<std::sync::Arc<crate::ephemeris::Ephemeris>>,
}

/// Initial condition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Init {
    /// Equilibrium temperature of the relaxation at t = 0, at rest, plus a small noise.
    Equilibrium,
    /// Resting atmosphere with the given profile (no noise): balance tests.
    Rest(RefProfile),
}

/// The dust's Mie optics as built from the settings.
#[derive(Clone, Debug)]
pub struct MieSetup {
    /// Per-cell optics (its `kext` is filled by the suite).
    pub rad: MieRadiation,
    /// Extinction efficiencies of the two classes.
    pub qext: [f64; 2],
    /// Cells whose colour is darker than the slab model reaches (read as opaque grains).
    pub floored: usize,
}

pub struct PeSetup {
    pub model: PeModel,
    pub state: PeState,
    /// Model relief (m) per cell, after smoothing.
    pub relief: Vec<f64>,
    pub raw_min: f64,
    pub raw_max: f64,
}

impl Scenario {
    /// Loads a settings file (with `key=value` overrides) and its body. The body name comes from
    /// `body_name` if given, else from the file's `body` key.
    /// The data files come from the settings' `[data]` section.
    pub fn load(path: &Path, sets: &[(String, String)], body_name: Option<&str>) -> Result<Scenario, String> {
        Scenario::load_inner(path, sets, body_name, None)
    }

    /// Same as `load`, with the shared data files given by the caller (they win over `[data]`).
    pub fn load_with_data(path: &Path, sets: &[(String, String)], body_name: Option<&str>, files: &DataFiles) -> Result<Scenario, String> {
        Scenario::load_inner(path, sets, body_name, Some(files))
    }

    fn load_inner(path: &Path, sets: &[(String, String)], body_name: Option<&str>, files: Option<&DataFiles>) -> Result<Scenario, String> {
        let s = Settings::load(path, sets)?;
        let name = body_name.map(|x| x.to_string()).or_else(|| s.body.clone()).ok_or_else(|| format!("{}: no body named (key `body` or --body)", path.display()))?;
        let files = match (files, &s.data) {
            (Some(f), _) => f.clone(),
            (None, Some(d)) => d.files(),
            (None, None) => return Err(format!("{}: no [data] section and no data files given: cannot load the body", path.display())),
        };
        let body = Body::load(&files, &name, &s.body_overrides)?;
        Ok(Scenario { body, s, ephemeris: None })
    }

    /// Loads the settings of a body by name: `<dir>/<name>.toml`, or `<dir>/defaut.toml`.
    pub fn for_body(dir: &Path, name: &str, sets: &[(String, String)], files: &DataFiles) -> Result<Scenario, String> {
        Scenario::load_with_data(&Settings::file_for_body(dir, name), sets, Some(name), files)
    }

    /// A body already built by the caller (the service derives every climate at boot) and the
    /// settings file of that body in `dir` (`<name>.toml`, else `defaut.toml`), with overrides.
    /// The body's `[body_overrides]` are not applied here: they only serve offline regressions.
    pub fn from_body(body: Body, dir: &Path, sets: &[(String, String)]) -> Result<Scenario, String> {
        let s = Settings::load(&Settings::file_for_body(dir, &body.name), sets)?;
        Ok(Scenario { body, s, ephemeris: None })
    }

    /// Length of a chunk (s): the model day / chunks per day.
    pub fn chunk_s(&self) -> f64 {
        self.body.day_s / self.s.numerics.chunks_per_day as f64
    }

    pub fn chunks_per_day(&self) -> u64 {
        self.s.numerics.chunks_per_day
    }

    // ------------------------------------------------------------------ derived parameters

    /// Equator-to-pole contrast of the equilibrium temperature (K).
    pub fn dt_y(&self) -> f64 {
        let r = &self.s.relaxation;
        r.equator_pole_contrast_k.unwrap_or(r.contrast_factor * self.body.climate.delta_t_equator_pole_k)
    }

    /// Equilibrium temperature at the subsolar ground (K): the global mean of the recipe
    /// (t_max - contrast / 3) is the body's mean surface temperature.
    pub fn t_max(&self) -> f64 {
        self.s.relaxation.t_max_k.unwrap_or(self.body.t_surface_k + self.dt_y() / 3.0)
    }

    pub fn seasons(&self) -> Seasons {
        Seasons { tilt_deg: if self.s.relaxation.seasons { self.body.tilt_deg } else { 0.0 }, year_s: self.body.year_s }
    }

    /// The Held-Suarez relaxation of this body. `slow_lowest_layer`: a ground heats the lowest layer.
    pub fn held_suarez(&self, slow_lowest_layer: bool) -> HeldSuarez {
        let r = &self.s.relaxation;
        let b = &self.body;
        HeldSuarez {
            t_max: self.t_max(),
            dt_y: self.dt_y(),
            dtheta_z: r.static_stability_k,
            t_min: r.t_min_k.unwrap_or(b.climate.t_skin_k),
            k_a: 1.0 / r.slow_time_s.unwrap_or(b.radiative_time_s),
            k_s: 1.0 / r.fast_time_s,
            k_f: 1.0 / r.friction_time_s,
            sigma_b: r.sigma_b,
            seasons: self.seasons(),
            ephemeris: if self.s.relaxation.seasons { self.ephemeris.clone() } else { None },
            p0: b.p_surface,
            kappa: b.air.kappa,
            slow_lowest_layer,
        }
    }

    /// Reference profile of the pressure-gradient force and of the diffusion.
    pub fn tref(&self) -> RefProfile {
        let r = &self.s.reference;
        let c = &self.body.climate;
        RefProfile {
            t0: r.t0_k.unwrap_or(self.t_max() - self.dt_y() / 3.0),
            lapse: r.lapse_k_m.unwrap_or(c.lapse_env_k_m),
            t_strat: r.t_strat_k.unwrap_or(c.t_skin_k),
        }
    }

    /// Half levels: the settings' list, else the level rule.
    pub fn eta_half(&self) -> Result<Vec<f64>, String> {
        let g = &self.s.grid;
        let e = match &g.eta_half {
            Some(e) => e.clone(),
            None => levels::rule_eta_half(g.layers, self.body.climate.scale_height_m, self.body.climate.tropopause_m, g.rule_top_factor, g.rule_stretch),
        };
        levels::validate(&e)?;
        Ok(e)
    }

    /// Top of the terrain-following part of the hybrid coordinate.
    pub fn eta_t(&self, eta_half: &[f64]) -> f64 {
        let g = &self.s.grid;
        g.hybrid_eta_t.unwrap_or_else(|| {
            let c = &self.body.climate;
            levels::nearest_interior(eta_half, (-g.hybrid_top_factor * c.tropopause_m / c.scale_height_m).exp())
        })
    }

    pub fn ground_albedo(&self) -> f64 {
        self.s.ground.albedo.unwrap_or(self.body.bond_albedo)
    }

    /// Effective down-welling emissivity of the air from the grey infrared depth: 1 - exp(-tau_ir).
    pub fn eps_air(&self) -> f64 {
        self.s.ground.eps_air.unwrap_or(1.0 - (-self.body.climate.tau_ir).exp())
    }

    /// The dust classes, with the air's viscosity (Sutherland-like power law, as in weather-core)
    /// and the molecular mean free path at the mean surface state.
    pub fn dust_classes(&self) -> DustClasses {
        let d = &self.s.dust;
        let b = &self.body;
        let mu = d.viscosity_pa_s.unwrap_or(1.8e-5 * (b.t_surface_k / 293.0).powf(0.7));
        let mfp = d.mean_free_path_m.unwrap_or(mu / b.p_surface * (std::f64::consts::PI * b.air.rd * b.t_surface_k / 2.0).sqrt());
        DustClasses { d: d.diameters_m, rho_p: d.grain_density_kg_m3.unwrap_or(b.dust_grain_density), mu, mfp, qext: d.qext }
    }

    /// Lifting threshold on the surface stress (Pa): rho0 u*_t^2 with the data's threshold
    /// friction velocity (Shao & Lu with the body's gravity, grains and cohesion).
    pub fn lift_threshold(&self) -> f64 {
        let c = &self.body.climate;
        self.s.dust.threshold_pa.unwrap_or(c.rho0_kg_m3 * c.u_star_threshold_m_s * c.u_star_threshold_m_s)
    }

    pub fn lift_alpha(&self) -> f64 {
        self.s.dust.alpha_per_m.unwrap_or(self.s.dust.alpha_per_bias * self.body.climate.extras.dust_lift_bias)
    }

    /// Solar day used by the force-restore ground (s).
    pub fn forcing_day(&self) -> f64 {
        if self.body.solar_day_s.is_finite() {
            self.body.solar_day_s.abs()
        } else {
            self.body.day_s
        }
    }

    /// Readable list of the resolved parameters, with where each comes from.
    pub fn summary(&self) -> String {
        use std::fmt::Write as _;
        let s = &self.s;
        let tag = |o: bool| if o { "settings" } else { "data/rule" };
        let mut t = String::new();
        let _ = writeln!(t, "{}", self.body.describe());
        let eta = self.eta_half().unwrap_or_default();
        let _ = writeln!(t, "levels ({}): {:?}, hybrid eta_t {:.3}", tag(s.grid.eta_half.is_some()), eta, self.eta_t(&eta));
        let hs = self.held_suarez(s.ground.enabled);
        let _ = writeln!(
            t,
            "relaxation: contrast {:.2} K ({}), t_max {:.2} K ({}), t_min {:.1} K ({}), slow time {:.4e} s ({}); reference {:?}",
            hs.dt_y,
            tag(s.relaxation.equator_pole_contrast_k.is_some()),
            hs.t_max,
            tag(s.relaxation.t_max_k.is_some()),
            hs.t_min,
            tag(s.relaxation.t_min_k.is_some()),
            1.0 / hs.k_a,
            tag(s.relaxation.slow_time_s.is_some()),
            self.tref()
        );
        let dc = self.dust_classes();
        let _ = writeln!(
            t,
            "ground: albedo {:.3} ({}), eps_air {:.3} ({}), TIU {:.0}, z0 {:.1e} m, forcing day {:.0} s; dust: threshold {:.4} Pa ({}), alpha {:.3e} /m ({}), grains {:.0} kg/m3, mu {:.3e}, mfp {:.3e}",
            self.ground_albedo(),
            tag(s.ground.albedo.is_some()),
            self.eps_air(),
            tag(s.ground.eps_air.is_some()),
            s.ground.thermal_inertia_tiu.unwrap_or(self.body.thermal_inertia_tiu),
            s.ground.roughness_m.unwrap_or(self.body.roughness_length_m),
            self.forcing_day(),
            self.lift_threshold(),
            tag(s.dust.threshold_pa.is_some()),
            self.lift_alpha(),
            tag(s.dust.alpha_per_m.is_some()),
            dc.rho_p,
            dc.mu,
            dc.mfp
        );
        t
    }

    // ------------------------------------------------------------------ building

    /// Builds the model and its initial state.
    pub fn build(&self, init: Init) -> Result<PeSetup, String> {
        let s = &self.s;
        let b = &self.body;
        let c = PeConsts::from_body(b);
        let grid = Grid::new(s.grid.level, b.radius_m);
        let n = grid.n_cells();
        let raw = match &s.relief.file {
            Some(p) => {
                let raster = io::read_f32_raster(p, s.relief.width, s.relief.height).map_err(|e| format!("relief {}: {e}", p.display()))?;
                io::raster_to_cells(&grid, &raster, s.relief.width, s.relief.height)
            }
            None => vec![0.0; n],
        };
        let raw_min = raw.iter().cloned().fold(f64::MAX, f64::min);
        let raw_max = raw.iter().cloned().fold(f64::MIN, f64::max);
        let scaled: Vec<f64> = raw.iter().map(|r| r * s.relief.scale).collect();
        let relief = smooth_cells(&grid, &scaled, s.relief.smooth_passes, 0.5);
        let phis: Vec<f64> = relief.iter().map(|z| c.g * z).collect();
        let nm = &s.numerics;
        let o = nm.hyper_order;
        let nu_v = hyper_nu(&grid, nm.tau_wind_hours * 3600.0, o);
        let nu_t = hyper_nu(&grid, nm.tau_temperature_hours * 3600.0, o);
        let nu_ps = if nm.tau_ps_hours > 0.0 { hyper_nu(&grid, nm.tau_ps_hours * 3600.0, o) } else { 0.0 };
        let ground_on = s.ground.enabled;
        let hs = self.held_suarez(ground_on);
        let relax: Option<Box<dyn Relaxation>> = if s.relaxation.enabled { Some(Box::new(hs.clone())) } else { None };
        let eta = self.eta_half()?;
        let eta_t = self.eta_t(&eta);
        let tref = self.tref();
        let limits = GuardLimits { ps_min_frac: nm.guard_ps_min_frac, ps_max_frac: nm.guard_ps_max_frac, max_wind: nm.guard_max_wind_m_s };
        let mut model = PeModel::new(grid, c.clone(), eta, eta_t, phis, tref, o, nu_v, nu_t, nu_ps, relax, limits);
        let l = model.nl;
        let mut st = PeState::zeros(n, l);
        let ps_profile = if let Init::Rest(atm) = init { atm } else { tref };
        for i in 0..n {
            let ps = ps_profile.surface_pressure(relief[i], c.p0, &c);
            st.ps[i] = ps;
            let x = model.grid.x[i];
            let (sl, cl) = (x[2], (1.0 - x[2] * x[2]).max(0.0).sqrt());
            for k in 0..l {
                let pk = model.p_full(k, ps);
                let t = match init {
                    Init::Rest(atm) => atm.temp(pk, &c),
                    Init::Equilibrium => hs.initial_temperature(sl, cl, pk) + nm.noise_k * noise((i * l + k) as u64),
                };
                let theta = t / model.exner(k, ps);
                st.th[i * l + k] = model.dp(k, ps) * theta;
            }
        }
        let mean_ps = model.grid.integral(&st.ps) / model.grid.area.iter().sum::<f64>();
        model.set_ps_ref(mean_ps);
        if let Some(suite) = self.physics(&model, tref, &c)? {
            if s.dust.enabled {
                model.ntr = 2;
                st.dust_active = true;
            }
            model.phys = Some(suite);
            // Ground starts at the surface-air temperature (lowest-layer theta at the surface).
            for i in 0..n {
                let ps = st.ps[i];
                let th = st.th[i * l + l - 1] / model.dp(l - 1, ps);
                let ta = th * (ps / c.p0).powf(c.kappa);
                st.tg[i] = ta;
                st.t2[i] = ta;
            }
        }
        Ok(PeSetup { model, state: st, relief, raw_min, raw_max })
    }

    /// The column physics chosen by the settings (None when every component is off).
    fn physics(&self, model: &PeModel, tref: RefProfile, c: &PeConsts) -> Result<Option<PhysicsSuite>, String> {
        let s = &self.s;
        let b = &self.body;
        let dust_on = s.dust.enabled;
        let sea_on = s.sea.enabled && dust_on;
        let mut comps: Vec<Box<dyn ColumnProcess>> = Vec::new();
        let mut classes = self.dust_classes();
        let ground_cells = self.ground_albedo_cells(&model.grid)?;
        // Mie optics: the extinction efficiencies come from Mie theory instead of the settings.
        let mut mie = if dust_on && s.dust.optics == DustOptics::Mie { Some(self.mie_optics(&model.grid, &classes, ground_cells.as_deref())?) } else { None };
        if let Some(m) = &mie {
            classes.qext = m.qext;
        }
        let kext = if dust_on { [classes.kext(0), classes.kext(1)] } else { [0.0, 0.0] };
        if let Some(m) = &mut mie {
            m.rad.kext = kext;
        }
        // Fixed order (see phys::mod): radiation before the ground (it sets the sunlight reaching
        // the ground), events after the ground (heat) and after the lifting (dust), the lid before
        // the convection, the settling before the sea.
        if dust_on && s.dust.radiative {
            comps.push(Box::new(DustRadiation { absorb: self.absorption(&model.grid)?, mie: mie.map(|m| m.rad) }));
        }
        if s.ground.enabled {
            comps.push(Box::new(Ground {
                albedo: self.ground_albedo(),
                albedo_cells: ground_cells,
                eps_air: self.eps_air(),
                tiu: s.ground.thermal_inertia_tiu.unwrap_or(b.thermal_inertia_tiu),
                day_s: self.forcing_day(),
            }));
        }
        if dust_on && s.dust.lifting {
            comps.push(Box::new(DustLifting { tau_t: self.lift_threshold(), c_k: s.dust.saltation_constant, alpha: self.lift_alpha(), fine_frac: s.dust.fine_fraction }));
        }
        if s.events.enabled {
            comps.push(Box::new(Events));
        }
        if s.lid.enabled {
            comps.push(Box::new(Lid { z_bot: s.lid.z_m, depth: s.lid.depth_m, dt_k: s.lid.step_k, rate: 1.0 / (s.lid.fast_days * b.day_s) }));
        }
        if s.convection.enabled {
            comps.push(Box::new(DryConvection));
        }
        if dust_on {
            comps.push(Box::new(DustSettling { vs: [classes.settling_velocity(classes.d[0], c.g), classes.settling_velocity(classes.d[1], c.g)], dep_turb: s.dust.deposition_turbulent }));
        }
        if sea_on {
            // Target mixing ratio: the sea's optical depth over the air mass between its base and
            // top in the reference atmosphere (ground at the reference sphere).
            let band_mass = (tref.surface_pressure(s.sea.z_base_m, c.p0, c) - tref.surface_pressure(s.sea.z_top_m, c.p0, c)) / c.g;
            comps.push(Box::new(DustSea {
                z_base: s.sea.z_base_m,
                z_top: s.sea.z_top_m,
                q_target: s.sea.optical_depth / (kext[0] * band_mass),
                tau_fill: s.sea.fill_days * b.day_s,
                tau_excess: s.sea.excess_days * b.day_s,
                tau_clear: s.sea.clear_days * b.day_s,
            }));
        }
        if comps.is_empty() && !dust_on {
            return Ok(None);
        }
        let lid_probe_z = if s.lid.enabled {
            Some(s.lid.z_m)
        } else if sea_on {
            Some(s.sea.z_top_m)
        } else {
            None
        };
        Ok(Some(PhysicsSuite {
            sun: Sun { s0: b.irradiance, solar_day_s: b.solar_day_s, lon0_deg: s.ground.sun_lon0_deg, seasons: self.seasons(), ephemeris: self.ephemeris.clone(), uniform: s.ground.uniform_sun_w_m2 },
            surface: SurfaceLayer { z0: s.ground.roughness_m.unwrap_or(b.roughness_length_m), v_floor: s.ground.v_floor_m_s, stable_floor: s.ground.stable_floor },
            kext,
            lid_probe_z,
            components: comps,
        }))
    }

    /// Ground albedo per cell from the ground's colour map (None without a map).
    pub fn ground_albedo_cells(&self, grid: &Grid) -> Result<Option<Vec<f64>>, String> {
        let s = &self.s;
        match &s.ground.albedo_map {
            Some(p) => {
                let r = io::read_f32_raster(p, s.relief.width, s.relief.height).map_err(|e| format!("ground albedo map {}: {e}", p.display()))?;
                Ok(Some(io::raster_to_cells(grid, &r, s.relief.width, s.relief.height).iter().map(|a| a.clamp(0.0, 1.0)).collect()))
            }
            None => Ok(None),
        }
    }

    /// Albedo of the colour of the dust material per cell: the dust's colour map, else the
    /// ground's, else the uniform ground albedo.
    pub fn colour_albedo_cells(&self, grid: &Grid) -> Result<Vec<f64>, String> {
        let s = &self.s;
        if let Some(p) = &s.dust.absorption_albedo_map {
            let r = io::read_f32_raster(p, s.relief.width, s.relief.height).map_err(|e| format!("dust albedo map {}: {e}", p.display()))?;
            return Ok(io::raster_to_cells(grid, &r, s.relief.width, s.relief.height).iter().map(|a| a.clamp(0.0, 1.0)).collect());
        }
        Ok(self.ground_albedo_cells(grid)?.unwrap_or_else(|| vec![self.ground_albedo(); grid.n_cells()]))
    }

    /// Mie optics of the dust (option `dust.optics = "mie"`): the imaginary index of the material
    /// per cell from the colour (Hapke's equivalent slab with the ground grain size), the Mie
    /// optics of each class tabulated over that index, and the extinction efficiencies of the
    /// classes (area-weighted means over the cells, since the suite has one extinction per class).
    pub fn mie_optics(&self, grid: &Grid, classes: &DustClasses, ground_cells: Option<&[f64]>) -> Result<MieSetup, String> {
        let d = &self.s.dust;
        if !(d.refractive_index_real > 1.0 && d.mie_wavelength_m > 0.0 && d.ground_grain_m > 0.0 && d.mie_sigma_g >= 1.0) {
            return Err(format!(
                "dust Mie optics: need refractive_index_real > 1, mie_wavelength_m > 0, ground_grain_m > 0, mie_sigma_g >= 1 (got {}, {}, {}, {})",
                d.refractive_index_real, d.mie_wavelength_m, d.ground_grain_m, d.mie_sigma_g
            ));
        }
        let colour = self.colour_albedo_cells(grid)?;
        let n = grid.n_cells();
        let table = OpticsTable::new(classes.d, d.refractive_index_real, d.mie_sigma_g, d.mie_wavelength_m, 20);
        let mut k = Vec::with_capacity(n);
        let mut floored = 0;
        let (mut omega, mut g) = ([Vec::with_capacity(n), Vec::with_capacity(n)], [Vec::with_capacity(n), Vec::with_capacity(n)]);
        let mut qsum = [0.0; 2];
        let at: f64 = grid.area.iter().sum();
        for i in 0..n {
            let (ki, fl) = k_from_reflectance(colour[i], d.refractive_index_real, d.ground_grain_m, d.mie_wavelength_m);
            floored += fl as usize;
            k.push(ki);
            for c in 0..2 {
                let (q, w, a) = table.at(c, ki);
                qsum[c] += grid.area[i] * q;
                omega[c].push(w);
                g[c].push(a);
            }
        }
        let ground_albedo = ground_cells.map(|v| v.to_vec()).unwrap_or_else(|| vec![self.ground_albedo(); n]);
        let qext = [qsum[0] / at, qsum[1] / at];
        Ok(MieSetup { rad: MieRadiation { kext: [0.0; 2], qext, omega, g, k, ground_albedo }, qext, floored })
    }

    /// Absorption of the intercepted sunlight per cell.
    fn absorption(&self, grid: &Grid) -> Result<Vec<f64>, String> {
        let d = &self.s.dust;
        if let Some(p) = &d.absorption_albedo_map {
            let r = io::read_f32_raster(p, self.s.relief.width, self.s.relief.height).map_err(|e| format!("dust albedo map {}: {e}", p.display()))?;
            return Ok(io::raster_to_cells(grid, &r, self.s.relief.width, self.s.relief.height).iter().map(|a| (d.absorption_scale * (1.0 - a)).clamp(0.0, 1.0)).collect());
        }
        if let Some(p) = &d.absorption_table {
            return read_absorption_map(p, grid).map(|x| x.0).map_err(|e| format!("absorption table {}: {e}", p.display()));
        }
        let a = d.absorption_uniform.unwrap_or(1.0 - self.ground_albedo());
        Ok(vec![(d.absorption_scale * a).clamp(0.0, 1.0); grid.n_cells()])
    }
}

/// Neighbour-average smoothing: b_i <- (1 - w) b_i + w * area-weighted mean of the neighbours.
pub fn smooth_cells(grid: &Grid, b: &[f64], passes: usize, w: f64) -> Vec<f64> {
    let mut cur = b.to_vec();
    for _ in 0..passes {
        let next: Vec<f64> = (0..grid.n_cells())
            .map(|i| {
                let (mut s, mut a) = (0.0, 0.0);
                for q in grid.c_start[i]..grid.c_start[i + 1] {
                    let j = grid.c_nbr[q];
                    s += grid.area[j] * cur[j];
                    a += grid.area[j];
                }
                (1.0 - w) * cur[i] + w * s / a
            })
            .collect();
        cur = next;
    }
    cur
}

/// Largest height difference between neighbouring cells divided by their distance.
pub fn max_slope(grid: &Grid, b: &[f64]) -> f64 {
    (0..grid.n_edges()).map(|e| (b[grid.e_j[e]] - b[grid.e_i[e]]).abs() / grid.e_d[e]).fold(0.0, f64::max)
}

/// Deterministic noise in [-1, 1] from an integer (splitmix64).
fn noise(i: u64) -> f64 {
    let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// Reads the provisional dust-absorption map: lines `# ...` are comments; `biome <letter> <name>
/// <absorption>` define the table; `grid <W> <H>` is followed by H rows of W letters (north to
/// south, longitude -180 to 180). Returns the absorption per cell and the biome letter per cell.
pub fn read_absorption_map(path: &Path, grid: &Grid) -> std::io::Result<(Vec<f64>, Vec<char>)> {
    let txt = std::fs::read_to_string(path)?;
    let bad = |m: String| std::io::Error::new(std::io::ErrorKind::InvalidData, m);
    let mut table: Vec<(char, f64)> = Vec::new();
    let mut rows: Vec<Vec<char>> = Vec::new();
    let (mut w, mut h) = (0usize, 0usize);
    let mut in_grid = false;
    for line in txt.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if in_grid {
            rows.push(t.chars().collect());
            continue;
        }
        let f: Vec<&str> = t.split_whitespace().collect();
        match f.first().copied() {
            Some("biome") if f.len() >= 4 => {
                let ch = f[1].chars().next().unwrap();
                let a: f64 = f[3].parse().map_err(|_| bad(format!("bad absorption in '{t}'")))?;
                table.push((ch, a));
            }
            Some("grid") if f.len() >= 3 => {
                w = f[1].parse().map_err(|_| bad("bad grid width".into()))?;
                h = f[2].parse().map_err(|_| bad("bad grid height".into()))?;
                in_grid = true;
            }
            _ => return Err(bad(format!("unexpected line in the absorption map: '{t}'"))),
        }
    }
    if rows.len() != h || rows.iter().any(|r| r.len() != w) || w == 0 {
        return Err(bad(format!("absorption grid: expected {h} rows of {w} letters")));
    }
    let mut ab = Vec::with_capacity(grid.n_cells());
    let mut letters = Vec::with_capacity(grid.n_cells());
    for i in 0..grid.n_cells() {
        let (lo, la) = (grid.lon[i].to_degrees(), grid.lat[i].to_degrees());
        let col = (((lo + 180.0) / 360.0 * w as f64).floor() as usize).min(w - 1);
        let row = (((90.0 - la) / 180.0 * h as f64).floor() as usize).min(h - 1);
        let ch = rows[row][col];
        let a = table.iter().find(|(c, _)| *c == ch).map(|x| x.1).ok_or_else(|| bad(format!("biome letter '{ch}' not in the table")))?;
        ab.push(a);
        letters.push(ch);
    }
    Ok((ab, letters))
}
