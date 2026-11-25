//! Test cases (Williamson et al. 1992) and the single-layer run on any body (step 2).

use crate::grid::Grid;
use crate::io::{self, Sampler, TableMeta};
use crate::model::{nu4_from_tau, Diag, Model, State};
use crate::vec3::*;
use std::f64::consts::PI;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const DAY: f64 = 86400.0;

#[derive(Clone, Copy, Debug)]
pub struct Planet {
    pub radius: f64,
    pub omega: f64,
    pub g: f64,
}

/// Earth constants exactly as defined by Williamson et al. (1992).
pub const EARTH_W92: Planet = Planet { radius: 6.37122e6, omega: 7.292e-5, g: 9.80616 };

impl Planet {
    /// Radius, rotation and gravity of a body of the system data.
    pub fn from_body(b: &crate::body::Body) -> Planet {
        Planet { radius: b.radius_m, omega: b.omega, g: b.gravity }
    }
}

/// Default damping time of the shortest grid mode by del^4 hyperdiffusion.
pub const DEFAULT_TAU_HOURS: f64 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct Norms {
    pub l1: f64,
    pub l2: f64,
    pub linf: f64,
}

/// Normalised error norms of Williamson et al. (1992), eqs. (82)-(84).
pub fn norms(grid: &Grid, h: &[f64], ht: &[f64]) -> Norms {
    let mut s1 = 0.0;
    let mut s1t = 0.0;
    let mut s2 = 0.0;
    let mut s2t = 0.0;
    let mut mx = 0.0f64;
    let mut mxt = 0.0f64;
    for i in 0..h.len() {
        let a = grid.area[i];
        let d = h[i] - ht[i];
        s1 += a * d.abs();
        s1t += a * ht[i].abs();
        s2 += a * d * d;
        s2t += a * ht[i] * ht[i];
        mx = mx.max(d.abs());
        mxt = mxt.max(ht[i].abs());
    }
    Norms { l1: s1 / s1t, l2: (s2 / s2t).sqrt(), linf: mx / mxt }
}

fn rotation_axis(alpha: f64) -> V3 {
    [-alpha.sin(), 0.0, alpha.cos()]
}

// ---------------------------------------------------------------- TC1

pub struct Tc1Result {
    pub alpha: f64,
    pub norms: Norms,
    pub min: f64,
    pub max: f64,
    pub mass_drift: f64,
    pub steps: usize,
}

/// Cosine bell advected once around the sphere (12 days) by a solid-body wind.
/// The face fluxes come from the stream function, so the discrete wind is exactly
/// non-divergent and the bell is carried as the tracer (layer depth fixed to 1).
pub fn tc1(level: u32, alpha: f64, courant: f64) -> Tc1Result {
    let p = EARTH_W92;
    let a = p.radius;
    let grid = Grid::new(level, a);
    let n = grid.n_cells();
    let u0 = 2.0 * PI * a / (12.0 * DAY);
    let k = rotation_axis(alpha);
    let psi = |x: V3| -a * u0 * dot(k, x);
    let pres: Vec<f64> = (0..grid.n_edges()).map(|e| -(psi(grid.e_b[e]) - psi(grid.e_a[e]))).collect();
    let (lc, tc) = (1.5 * PI, 0.0);
    let xc = from_lonlat(lc, tc);
    let h0 = 1000.0;
    let rb = a / 3.0;
    let bell: Vec<f64> = grid
        .x
        .iter()
        .map(|&x| {
            let r = a * angle(x, xc);
            if r < rb {
                0.5 * h0 * (1.0 + (PI * r / rb).cos())
            } else {
                0.0
            }
        })
        .collect();
    let mut st = State::zeros(n);
    for i in 0..n {
        st.h[i] = 1.0;
        st.v[i] = scale(cross(k, grid.x[i]), u0);
        st.m[i] = bell[i];
    }
    let model = Model {
        g: p.g,
        f: vec![0.0; n],
        b: vec![0.0; n],
        nu4: 0.0,
        dynamics: false,
        prescribed: Some(pres),
        grid,
        omega_for_diag: p.omega,
    };
    let mut w = model.new_work();
    let m0 = model.grid.integral(&st.m);
    let steps = model.run_until(&mut st, 12.0 * DAY, courant, &mut w).expect("TC1 failed");
    let m1 = model.grid.integral(&st.m);
    let nm = norms(&model.grid, &st.m, &bell);
    Tc1Result {
        alpha,
        norms: nm,
        min: st.m.iter().cloned().fold(f64::MAX, f64::min),
        max: st.m.iter().cloned().fold(f64::MIN, f64::max),
        mass_drift: (m1 - m0) / m0,
        steps,
    }
}

// ---------------------------------------------------------------- TC2

pub struct Tc2Result {
    pub alpha: f64,
    pub norms: Norms,
    pub vel_l2: f64,
    pub mass_drift: f64,
    pub steps: usize,
}

fn tc2_setup(level: u32, alpha: f64, tau_hours: f64) -> (Model, State, Vec<f64>) {
    let p = EARTH_W92;
    let a = p.radius;
    let grid = Grid::new(level, a);
    let n = grid.n_cells();
    let u0 = 2.0 * PI * a / (12.0 * DAY);
    let h0 = 2.94e4 / p.g;
    let k = rotation_axis(alpha);
    let mut st = State::zeros(n);
    let mut f = vec![0.0; n];
    for i in 0..n {
        let x = grid.x[i];
        let s = dot(k, x);
        st.h[i] = h0 - (a * p.omega * u0 + 0.5 * u0 * u0) * s * s / p.g;
        st.v[i] = scale(cross(k, x), u0);
        f[i] = 2.0 * p.omega * s;
    }
    let nu4 = nu4_from_tau(&grid, tau_hours * 3600.0);
    let h_init = st.h.clone();
    let model = Model { g: p.g, f, b: vec![0.0; n], nu4, dynamics: true, prescribed: None, grid, omega_for_diag: p.omega };
    (model, st, h_init)
}

/// Steady zonal geostrophic flow, Earth numbers of the paper, 5 days.
pub fn tc2(level: u32, alpha: f64, days: f64, courant: f64, tau_hours: f64) -> Tc2Result {
    let (model, mut st, h_init) = tc2_setup(level, alpha, tau_hours);
    let v_init = st.v.clone();
    let mut w = model.new_work();
    let m0 = model.grid.integral(&st.h);
    let steps = model.run_until(&mut st, days * DAY, courant, &mut w).expect("TC2 failed");
    let m1 = model.grid.integral(&st.h);
    let nm = norms(&model.grid, &st.h, &h_init);
    let mut e2 = 0.0;
    let mut t2 = 0.0;
    for i in 0..st.h.len() {
        let d = sub(st.v[i], v_init[i]);
        e2 += model.grid.area[i] * dot(d, d);
        t2 += model.grid.area[i] * dot(v_init[i], v_init[i]);
    }
    Tc2Result { alpha, norms: nm, vel_l2: (e2 / t2).sqrt(), mass_drift: (m1 - m0) / m0, steps }
}

// ---------------------------------------------------------------- TC5

pub struct Tc5Result {
    pub mass_drift: f64,
    pub energy_drift: f64,
    pub min_h: f64,
    pub max_speed: f64,
    pub steps: usize,
    pub daily: Vec<(f64, Diag)>,
    pub model: Model,
    pub state: State,
}

/// Zonal flow over an isolated mountain (Earth numbers), 15 days.
pub fn tc5(level: u32, days: f64, courant: f64, tau_hours: f64) -> Tc5Result {
    let p = EARTH_W92;
    let a = p.radius;
    let grid = Grid::new(level, a);
    let n = grid.n_cells();
    let u0 = 20.0;
    let h0 = 5960.0;
    let (hs0, r0) = (2000.0, PI / 9.0);
    let (lc, tc) = (1.5 * PI, PI / 6.0);
    let k = [0.0, 0.0, 1.0];
    let mut st = State::zeros(n);
    let mut f = vec![0.0; n];
    let mut b = vec![0.0; n];
    for i in 0..n {
        let x = grid.x[i];
        let (lon, lat) = (grid.lon[i], grid.lat[i]);
        let mut dl = lon - lc;
        dl = (dl + PI).rem_euclid(2.0 * PI) - PI;
        let r = (dl * dl + (lat - tc) * (lat - tc)).sqrt().min(r0);
        b[i] = hs0 * (1.0 - r / r0);
        let s = x[2];
        st.h[i] = h0 - (a * p.omega * u0 + 0.5 * u0 * u0) * s * s / p.g - b[i];
        st.v[i] = scale(cross(k, x), u0);
        f[i] = 2.0 * p.omega * s;
    }
    let nu4 = nu4_from_tau(&grid, tau_hours * 3600.0);
    let model = Model { g: p.g, f, b, nu4, dynamics: true, prescribed: None, grid, omega_for_diag: p.omega };
    let mut w = model.new_work();
    let d0 = model.diag(&st);
    let mut daily = vec![(0.0, d0)];
    let mut steps = 0;
    let mut min_h = d0.min_h;
    let mut max_speed = d0.max_speed;
    let nd = days.round() as usize;
    for day in 1..=nd {
        steps += model.run_until(&mut st, day as f64 * DAY, courant, &mut w).expect("TC5 failed");
        let d = model.diag(&st);
        min_h = min_h.min(d.min_h);
        max_speed = max_speed.max(d.max_speed);
        daily.push((day as f64, d));
    }
    let d1 = daily.last().unwrap().1;
    Tc5Result {
        mass_drift: (d1.mass - d0.mass) / d0.mass,
        energy_drift: (d1.energy - d0.energy) / d0.energy,
        min_h,
        max_speed,
        steps,
        daily,
        model,
        state: st,
    }
}

/// TC5 has no analytic solution: compare the free surface h + b at `days` on
/// coarser grids with a level-`ref_level` run, interpolated onto the fine cells.
pub fn tc5_convergence(levels: &[u32], ref_level: u32, days: f64, courant: f64, tau_hours: f64) -> Vec<(u32, Norms)> {
    let fine = tc5(ref_level, days, courant, tau_hours);
    let fsurf: Vec<f64> = (0..fine.model.grid.n_cells()).map(|i| fine.state.h[i] + fine.model.b[i]).collect();
    levels
        .iter()
        .map(|&lv| {
            let c = tc5(lv, days, courant, tau_hours);
            let cs: Vec<f64> = (0..c.model.grid.n_cells()).map(|i| c.state.h[i] + c.model.b[i]).collect();
            let mut hint = 0;
            let interp: Vec<f64> = fine
                .model
                .grid
                .x
                .iter()
                .map(|&p| {
                    let (t, w, nc) = c.model.grid.locate(p, hint);
                    hint = nc;
                    w[0] * cs[t[0]] + w[1] * cs[t[1]] + w[2] * cs[t[2]]
                })
                .collect();
            (lv, norms(&fine.model.grid, &interp, &fsurf))
        })
        .collect()
}

// ---------------------------------------------------------------- single layer on a body

#[derive(Clone, Debug)]
pub struct ShallowCfg {
    pub planet: Planet,
    /// Model day of the body (s): the time unit of the run lengths and of the outputs.
    pub day_s: f64,
    pub level: u32,
    pub h_mean: f64,
    pub u0: f64,
    /// Relief raster (512 x 256 float32); None = a smooth sphere.
    pub relief_path: Option<PathBuf>,
    /// Relief is scaled so that its highest cell reaches `relief_frac * h_mean`.
    pub relief_frac: f64,
    pub days: f64,
    pub every_hours: f64,
    pub out_dir: Option<PathBuf>,
    pub tau_hours: f64,
    pub courant: f64,
    pub img_w: usize,
    pub img_h: usize,
    /// Region of the dust release: lon min, lon max, lat min, lat max (degrees).
    pub release: [f64; 4],
}

impl ShallowCfg {
    /// The single-layer settings of a body (section `[shallow_water]`; mean depth by default the
    /// scale height).
    pub fn from_scenario(sc: &crate::setup::Scenario) -> ShallowCfg {
        let sw = &sc.s.shallow_water;
        ShallowCfg {
            planet: Planet::from_body(&sc.body),
            day_s: sc.body.day_s,
            level: sc.s.grid.level,
            h_mean: sw.mean_depth_m.unwrap_or(sc.body.climate.scale_height_m),
            u0: sw.u0_m_s,
            relief_path: sc.s.relief.file.clone(),
            relief_frac: sw.relief_frac,
            days: sw.days,
            every_hours: sw.every_days * sc.body.day_s / 3600.0,
            out_dir: None,
            tau_hours: sw.tau_hours,
            courant: sc.s.numerics.courant,
            img_w: sc.s.run.img_w,
            img_h: sc.s.run.img_h,
            release: sc.s.outputs.dust_release,
        }
    }
}

pub struct ShallowResult {
    pub relief_scale: f64,
    pub b_min: f64,
    pub b_max: f64,
    pub raw_min: f64,
    pub raw_max: f64,
    pub mass_drift: f64,
    pub tracer_drift: f64,
    pub energy_drift: f64,
    pub min_h: f64,
    pub min_tracer: f64,
    pub max_speed: f64,
    pub steps: usize,
    pub wall_s: f64,
    pub log: String,
}

pub fn shallow_setup(cfg: &ShallowCfg) -> std::io::Result<(Model, State, f64, f64, f64)> {
    let p = cfg.planet;
    let grid = Grid::new(cfg.level, p.radius);
    let n = grid.n_cells();
    let raw = match &cfg.relief_path {
        Some(path) => io::raster_to_cells(&grid, &io::read_f32_raster(path, 512, 256)?, 512, 256),
        None => vec![0.0; n],
    };
    let raw_max = raw.iter().cloned().fold(f64::MIN, f64::max);
    let raw_min = raw.iter().cloned().fold(f64::MAX, f64::min);
    let scale_r = if raw_max > 0.0 { (cfg.relief_frac * cfg.h_mean / raw_max).min(1.0) } else { 1.0 };
    let b: Vec<f64> = raw.iter().map(|r| r * scale_r).collect();
    let a = p.radius;
    let u0 = cfg.u0;
    let k = [0.0, 0.0, 1.0];
    let mut st = State::zeros(n);
    let mut f = vec![0.0; n];
    for i in 0..n {
        let x = grid.x[i];
        let s = x[2];
        st.h[i] = cfg.h_mean - (a * p.omega * u0 + 0.5 * u0 * u0) * s * s / p.g - b[i];
        st.v[i] = scale(cross(k, x), u0);
        f[i] = 2.0 * p.omega * s;
    }
    // Shift the free surface so that the mean depth is exactly h_mean.
    let mean = grid.integral(&st.h) / grid.area.iter().sum::<f64>();
    for h in st.h.iter_mut() {
        *h += cfg.h_mean - mean;
    }
    // Dust released over the release region of the settings, c = 1.
    let r = cfg.release;
    for i in 0..n {
        let (lo, la) = (grid.lon[i].to_degrees(), grid.lat[i].to_degrees());
        let c = if (r[0]..=r[1]).contains(&lo) && (r[2]..=r[3]).contains(&la) { 1.0 } else { 0.0 };
        st.m[i] = st.h[i] * c;
    }
    let nu4 = nu4_from_tau(&grid, cfg.tau_hours * 3600.0);
    let model = Model { g: p.g, f, b, nu4, dynamics: true, prescribed: None, grid, omega_for_diag: p.omega };
    Ok((model, st, scale_r, raw_min, raw_max))
}

fn write_snapshot(model: &Model, st: &State, smp: &Sampler, dir: &Path, day_s: f64) -> std::io::Result<()> {
    let gr = &model.grid;
    let n = gr.n_cells();
    let surf: Vec<f64> = (0..n).map(|i| st.h[i] + model.b[i]).collect();
    let mean_surf = gr.integral(&surf) / gr.area.iter().sum::<f64>();
    let eta: Vec<f64> = surf.iter().map(|s| s - mean_surf).collect();
    let c: Vec<f64> = (0..n).map(|i| if st.h[i] > 0.0 { st.m[i] / st.h[i] } else { 0.0 }).collect();
    let hours = st.time / 3600.0;
    let stem = format!("t{:05}h", hours.round() as i64);
    let meta = |field: &'static str, units: &'static str, desc: &'static str| TableMeta {
        field,
        units,
        description: desc,
        time_s: st.time,
        time_hours: hours,
        planet_day_s: day_s,
        grid_level: gr.level,
        grid_cells: n,
        radius_m: gr.radius,
        fixed_range: None,
    };
    let (w, h) = (smp.w, smp.h);
    io::write_table(
        &dir.join(format!("{stem}_hauteur.png")),
        w,
        h,
        &smp.scalar(&eta),
        &meta("hauteur", "m", "free-surface anomaly h + b minus its global mean (pressure proxy of the single layer)"),
    )?;
    let (ue, vn) = smp.east_north(&st.v);
    io::write_table(&dir.join(format!("{stem}_u.png")), w, h, &ue, &meta("u", "m/s", "eastward wind"))?;
    io::write_table(&dir.join(format!("{stem}_v.png")), w, h, &vn, &meta("v", "m/s", "northward wind"))?;
    io::write_table(
        &dir.join(format!("{stem}_poussiere.png")),
        w,
        h,
        &smp.scalar(&c),
        &meta("poussiere", "1", "passive dust mixing ratio (1 = released value)"),
    )?;
    Ok(())
}

pub fn run_shallow(cfg: &ShallowCfg) -> std::io::Result<ShallowResult> {
    let t_wall = Instant::now();
    let (model, mut st, scale_r, raw_min, raw_max) = shallow_setup(cfg)?;
    let smp = cfg.out_dir.as_ref().map(|_| Sampler::new(&model.grid, cfg.img_w, cfg.img_h));
    if let Some(dir) = &cfg.out_dir {
        std::fs::create_dir_all(dir)?;
        let meta = TableMeta {
            field: "relief_modele",
            units: "m",
            description: "bottom topography b used by the model (scaled real relief)",
            time_s: 0.0,
            time_hours: 0.0,
            planet_day_s: cfg.day_s,
            grid_level: model.grid.level,
            grid_cells: model.grid.n_cells(),
            radius_m: model.grid.radius,
            fixed_range: None,
        };
        let s = smp.as_ref().unwrap();
        io::write_table(&dir.join("relief_modele.png"), s.w, s.h, &s.scalar(&model.b), &meta)?;
        write_snapshot(&model, &st, s, dir, cfg.day_s)?;
    }
    let mut w = model.new_work();
    let d0 = model.diag(&st);
    let mut log = String::new();
    let _ = writeln!(log, "time_h,planet_days,mass_rel,tracer_rel,energy_rel,angmom_rel,min_h_m,min_m,max_speed_ms");
    let push = |log: &mut String, t: f64, d: &Diag| {
        let _ = writeln!(
            log,
            "{:.1},{:.3},{:.3e},{:.3e},{:.4e},{:.4e},{:.1},{:.3e},{:.2}",
            t / 3600.0,
            t / cfg.day_s,
            d.mass / d0.mass - 1.0,
            d.tracer / d0.tracer - 1.0,
            d.energy / d0.energy - 1.0,
            d.ang_mom / d0.ang_mom - 1.0,
            d.min_h,
            d.min_m,
            d.max_speed
        );
    };
    push(&mut log, 0.0, &d0);
    let t_end = cfg.days * cfg.day_s;
    let dt_out = cfg.every_hours * 3600.0;
    let mut steps = 0;
    let (mut min_h, mut min_m, mut max_speed) = (d0.min_h, d0.min_m, d0.max_speed);
    let mut t_next = dt_out.min(t_end);
    loop {
        steps += model
            .run_until(&mut st, t_next, cfg.courant, &mut w)
            .map_err(std::io::Error::other)?;
        let d = model.diag(&st);
        if !d.finite || d.min_h <= 0.0 {
            return Err(std::io::Error::other(format!("single-layer run failed at t = {} s: {:?}", st.time, d)));
        }
        min_h = min_h.min(d.min_h);
        min_m = min_m.min(d.min_m);
        max_speed = max_speed.max(d.max_speed);
        push(&mut log, st.time, &d);
        if let (Some(dir), Some(s)) = (&cfg.out_dir, &smp) {
            write_snapshot(&model, &st, s, dir, cfg.day_s)?;
        }
        if t_next >= t_end - 1e-6 {
            break;
        }
        t_next = (t_next + dt_out).min(t_end);
    }
    let d1 = model.diag(&st);
    if let Some(dir) = &cfg.out_dir {
        std::fs::write(dir.join("bilans.csv"), &log)?;
    }
    Ok(ShallowResult {
        relief_scale: scale_r,
        b_min: model.b.iter().cloned().fold(f64::MAX, f64::min),
        b_max: model.b.iter().cloned().fold(f64::MIN, f64::max),
        raw_min,
        raw_max,
        mass_drift: d1.mass / d0.mass - 1.0,
        tracer_drift: d1.tracer / d0.tracer - 1.0,
        energy_drift: d1.energy / d0.energy - 1.0,
        min_h,
        min_tracer: min_m,
        max_speed,
        steps,
        wall_s: t_wall.elapsed().as_secs_f64(),
        log,
    })
}

/// Simulated model days per wall-clock second (relief + tracer, no output).
pub fn bench(base: &ShallowCfg, level: u32, days: f64) -> std::io::Result<(f64, usize, f64)> {
    let mut cfg = base.clone();
    cfg.level = level;
    let (model, mut st, ..) = shallow_setup(&cfg)?;
    let mut w = model.new_work();
    // Warm-up (thread pool, caches).
    model.run_until(&mut st, 0.25 * cfg.day_s, cfg.courant, &mut w).map_err(std::io::Error::other)?;
    let t0 = Instant::now();
    let t_start = st.time;
    let steps = model
        .run_until(&mut st, t_start + days * cfg.day_s, cfg.courant, &mut w)
        .map_err(std::io::Error::other)?;
    let el = t0.elapsed().as_secs_f64();
    let dt_mean = days * cfg.day_s / steps as f64;
    Ok((days / el, steps, dt_mean))
}
