//! Runs of the primitive-equation model on any body: the long run (spin-up then a recorded
//! period, with outputs, restarts and the guard), the resting-atmosphere test, the speed bench.

use crate::io::{self, Sampler, TableMeta};
use crate::outputs::{self, PhysRecorder};
use crate::pe::*;
use crate::setup::{max_slope, Init, Scenario};
use crate::vec3::*;
use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Instant;

/// Passive dust released (q = 1 in every layer) over the region of the settings
/// (`outputs.dust_release` = lon min, lon max, lat min, lat max, degrees).
pub fn release_dust(model: &PeModel, st: &mut PeState, region: [f64; 4]) {
    let l = model.nl;
    let gr = &model.grid;
    for i in 0..gr.n_cells() {
        let (lo, la) = (gr.lon[i].to_degrees(), gr.lat[i].to_degrees());
        let q = if (region[0]..=region[1]).contains(&lo) && (region[2]..=region[3]).contains(&la) { 1.0 } else { 0.0 };
        for k in 0..l {
            st.m[i * l + k] = model.dp(k, st.ps[i]) * q;
        }
    }
    st.dust_active = true;
}

/// Advances `n` chunks; stops at the first guard failure.
pub fn run_chunks(model: &PeModel, st: &mut PeState, n: u64, chunk_s: f64, courant: f64, w: &mut PeWork) -> Result<usize, String> {
    let mut steps = 0;
    for _ in 0..n {
        steps += model.advance_chunk(st, chunk_s, courant, w)?;
    }
    Ok(steps)
}

pub struct PeRunResult {
    pub steps: usize,
    pub wall_s: f64,
    pub air_drift: f64,
    pub dust_drift: f64,
    pub min_dust: f64,
    pub max_speed: f64,
    pub min_ps: f64,
    pub max_ps: f64,
    pub log: String,
    /// Roll-backs (and the final stop, if any) of the guard.
    pub guard_events: Vec<String>,
    pub failed: Option<String>,
}

/// The long run: spin-up, then a recorded period with outputs, restarts and the guard.
pub fn run_pe(sc: &Scenario) -> Result<PeRunResult, String> {
    let ioe = |e: std::io::Error| e.to_string();
    let t_wall = Instant::now();
    let s = &sc.s;
    let day_s = sc.body.day_s;
    let setup = sc.build(Init::Equilibrium)?;
    let model = setup.model;
    let relief = setup.relief;
    let mut st = setup.state;
    let restarted = if let Some(p) = &s.run.restart_from {
        st = model.read_restart(p).map_err(ioe)?;
        true
    } else {
        false
    };
    let cpd = sc.chunks_per_day();
    let chunk_s = sc.chunk_s();
    let courant = s.numerics.courant;
    let total = sc.s.chunks(s.run.spinup_days + s.run.record_days);
    let rec_start = sc.s.chunks(s.run.spinup_days);
    let every = |days: f64| if days > 0.0 { sc.s.chunks(days).max(1) } else { u64::MAX };
    let diag_every = sc.s.chunks(s.run.diag_every_days).max(1);
    let table_every = every(s.run.tables_every_days);
    let restart_every = every(s.run.restart_every_days);

    let out_dir = s.run.out_dir.clone();
    let smp_t = out_dir.as_ref().map(|_| Sampler::new(&model.grid, s.run.img_w, s.run.img_h));
    let smp_c = out_dir.as_ref().map(|_| Sampler::new(&model.grid, 192, 96));
    let mut climat: Option<std::fs::File> = None;
    if let Some(dir) = &out_dir {
        std::fs::create_dir_all(dir).map_err(ioe)?;
        std::fs::write(dir.join("corps.txt"), sc.summary()).map_err(ioe)?;
        let smp = smp_t.as_ref().unwrap();
        let meta = TableMeta {
            field: "relief_modele",
            units: "m",
            description: "relief used by the model (cell averages of the game's relief, smoothed)",
            time_s: 0.0,
            time_hours: 0.0,
            planet_day_s: day_s,
            grid_level: model.grid.level,
            grid_cells: model.grid.n_cells(),
            radius_m: model.grid.radius,
            fixed_range: None,
        };
        io::write_table(&dir.join("relief_modele.png"), smp.w, smp.h, &smp.scalar(&relief), &meta).map_err(ioe)?;
        if let Some(rad) = model.phys.as_ref().and_then(|p| p.find::<crate::phys::dust::DustRadiation>()) {
            let meta_a = TableMeta { field: "absorption_poussiere", units: "1", description: "fraction of the intercepted sunlight absorbed by the dust, per model cell", ..meta };
            io::write_table(&dir.join("absorption_poussiere.png"), smp.w, smp.h, &smp.scalar(&rad.absorb), &meta_a).map_err(ioe)?;
        }
        outputs::write_climate_header(&model, sc, dir, &relief, smp_c.as_ref().unwrap()).map_err(ioe)?;
        let f = std::fs::OpenOptions::new().create(true).append(restarted).write(true).truncate(!restarted).open(dir.join("climat.bin")).map_err(ioe)?;
        climat = Some(f);
    }
    let mut w = model.new_work();
    let mut rec = match (&out_dir, &model.phys) {
        (Some(dir), Some(_)) => Some(PhysRecorder::new(&model, &relief, dir, restarted, sc).map_err(ioe)?),
        _ => None,
    };
    let d0 = model.diag(&st);
    let mut log = String::new();
    if !restarted {
        let _ = writeln!(log, "planet_days,air_mass_rel,dust_mass_rel,min_ps_hPa,max_ps_hPa,min_T_K,max_T_K,max_speed_ms,min_dust,mean_dt_s,wall_s");
    }
    let air0 = d0.air_mass;
    let mut dust0 = if st.dust_active { d0.dust_mass } else { 0.0 };
    let mut guard = Guard::new(&st);
    // Log lines and climate records of the current model day: kept back until the day is
    // complete, so that a roll-back of the guard never leaves records of a failed attempt.
    let mut day_log = String::new();
    let mut day_clim: Vec<u8> = Vec::new();
    let mut guard_log = String::new();
    let mut steps = 0usize;
    let mut steps_since = 0usize;
    let mut chunks_since = 0u64;
    let (mut max_speed, mut min_dust, mut min_ps, mut max_ps) = (d0.max_speed, f64::MAX, d0.min_ps, d0.max_ps);
    let mut failed = None;
    while st.chunk < total {
        if st.chunk >= rec_start && !st.dust_active {
            release_dust(&model, &mut st, s.outputs.dust_release);
            dust0 = model.diag(&st).dust_mass;
            if let (Some(dir), Some(smp)) = (&out_dir, &smp_t) {
                outputs::write_tables(&model, &st, &relief, smp, dir, Some(&w), sc).map_err(ioe)?;
            }
        }
        match model.advance_guarded(&mut st, &mut guard, chunk_s, courant, cpd, &mut w) {
            Ok(ChunkOutcome::Done(n)) => {
                steps += n;
                steps_since += n;
                chunks_since += 1;
            }
            Ok(ChunkOutcome::RolledBack(msg)) => {
                eprintln!("{msg}");
                let _ = writeln!(guard_log, "{msg}");
                day_log.clear();
                day_clim.clear();
                steps_since = 0;
                chunks_since = 0;
                continue;
            }
            Err(e) => {
                let msg = format!("GUARD STOP: {e}; last good state at day {:.3}", guard.last_good.time / day_s);
                eprintln!("{msg}");
                let _ = writeln!(guard_log, "{msg}");
                if let Some(dir) = &out_dir {
                    model.write_restart(&guard.last_good, &dir.join("restart_derniere_bonne.bin")).map_err(ioe)?;
                }
                failed = Some(msg);
                break;
            }
        }
        let ch = st.chunk;
        if let Some(r) = rec.as_mut() {
            if ch > rec_start {
                r.hourly(&model, &st, &w, &relief).map_err(ioe)?;
                if ch % cpd == 0 {
                    r.day_end();
                }
            }
            if ch >= rec_start && ch % diag_every == 0 {
                r.diag(&model, &st, &w, &relief).map_err(ioe)?;
            }
        }
        if ch % diag_every == 0 {
            let d = model.diag(&st);
            max_speed = max_speed.max(d.max_speed);
            min_ps = min_ps.min(d.min_ps);
            max_ps = max_ps.max(d.max_ps);
            if st.dust_active {
                min_dust = min_dust.min(d.min_dust);
            }
            let mean_dt = chunks_since as f64 * chunk_s / steps_since.max(1) as f64;
            let _ = writeln!(
                day_log,
                "{:.4},{:.3e},{:.3e},{:.2},{:.2},{:.2},{:.2},{:.2},{:.3e},{:.1},{:.1}",
                st.time / day_s,
                d.air_mass / air0 - 1.0,
                if st.dust_active && dust0 > 0.0 { d.dust_mass / dust0 - 1.0 } else { 0.0 },
                d.min_ps / 100.0,
                d.max_ps / 100.0,
                d.min_t,
                d.max_t,
                d.max_speed,
                if st.dust_active { d.min_dust } else { 0.0 },
                mean_dt,
                t_wall.elapsed().as_secs_f64()
            );
            steps_since = 0;
            chunks_since = 0;
            if ch >= rec_start {
                if let (true, Some(smp)) = (climat.is_some(), &smp_c) {
                    let r = outputs::climate_record(&model, &st, &relief, smp, day_s);
                    day_clim.extend(r.iter().flat_map(|x| x.to_le_bytes()));
                }
            }
            if !s.run.quiet && ch % (diag_every * 20) == 0 {
                println!(
                    "day {:7.2}  max |v| {:5.1} m/s  ps {:6.1}..{:6.1} hPa  T {:5.1}..{:5.1} K  air drift {:+.1e}  wall {:.0} s",
                    st.time / day_s,
                    d.max_speed,
                    d.min_ps / 100.0,
                    d.max_ps / 100.0,
                    d.min_t,
                    d.max_t,
                    d.air_mass / air0 - 1.0,
                    t_wall.elapsed().as_secs_f64()
                );
            }
        }
        if ch % cpd == 0 || ch == total {
            log.push_str(&day_log);
            day_log.clear();
            if let Some(f) = climat.as_mut() {
                f.write_all(&day_clim).map_err(ioe)?;
            }
            day_clim.clear();
        }
        if ch >= rec_start && (ch - rec_start) % table_every == 0 {
            if let (Some(dir), Some(smp)) = (&out_dir, &smp_t) {
                outputs::write_tables(&model, &st, &relief, smp, dir, Some(&w), sc).map_err(ioe)?;
            }
        }
        if ch % restart_every == 0 || ch == total {
            if let Some(dir) = &out_dir {
                model.write_restart(&st, &dir.join("restart_dernier.bin")).map_err(ioe)?;
                if ch == rec_start {
                    model.write_restart(&st, &dir.join("restart_fin_mise_en_regime.bin")).map_err(ioe)?;
                }
            }
        }
    }
    if failed.is_none() {
        log.push_str(&day_log);
        if let Some(f) = climat.as_mut() {
            f.write_all(&day_clim).map_err(ioe)?;
        }
    }
    if let Some(dir) = &out_dir {
        if !guard_log.is_empty() {
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("garde.log")).map_err(ioe)?;
            f.write_all(guard_log.as_bytes()).map_err(ioe)?;
        }
    }
    if let (Some(r), Some(dir), Some(smp)) = (&rec, &out_dir, &smp_t) {
        r.finish(&model, smp, dir, &relief).map_err(ioe)?;
    }
    let d1 = model.diag(&st);
    if let Some(dir) = &out_dir {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("bilans.csv")).map_err(ioe)?;
        f.write_all(log.as_bytes()).map_err(ioe)?;
    }
    Ok(PeRunResult {
        steps,
        wall_s: t_wall.elapsed().as_secs_f64(),
        air_drift: d1.air_mass / air0 - 1.0,
        dust_drift: if st.dust_active && dust0 > 0.0 { d1.dust_mass / dust0 - 1.0 } else { 0.0 },
        min_dust: if min_dust == f64::MAX { 0.0 } else { min_dust },
        max_speed,
        min_ps,
        max_ps,
        log,
        guard_events: guard.events,
        failed,
    })
}

/// Resting atmosphere `atm` over the relief, no forcing, model reference profile `tref`:
/// largest wind, rms wind of the lowest layer after `days`, and the largest relief slope.
/// `pgf_ref = false` uses the plain pressure-gradient form (no reference subtraction).
pub fn rest_test(sc: &Scenario, tref: RefProfile, atm: RefProfile, pgf_ref: bool, days: f64) -> Result<(f64, f64, f64), String> {
    let mut c = sc.clone();
    c.s.relaxation.enabled = false;
    c.s.reference.t0_k = Some(tref.t0);
    c.s.reference.lapse_k_m = Some(tref.lapse);
    c.s.reference.t_strat_k = Some(tref.t_strat);
    let setup = c.build(Init::Rest(atm))?;
    let mut model = setup.model;
    model.pgf_ref = pgf_ref;
    let mut st = setup.state;
    let mut w = model.new_work();
    run_chunks(&model, &mut st, c.s.chunks(days), c.chunk_s(), c.s.numerics.courant, &mut w)?;
    let d = model.diag(&st);
    // rms wind of the lowest layer
    let l = model.nl;
    let gr = &model.grid;
    let (mut s2, mut a) = (0.0, 0.0);
    for i in 0..gr.n_cells() {
        let v = st.v[i * l + l - 1];
        s2 += gr.area[i] * dot(v, v);
        a += gr.area[i];
    }
    Ok((d.max_speed, (s2 / a).sqrt(), max_slope(gr, &setup.relief)))
}

/// Simulated model days per wall-clock second (forcing, physics or passive dust, no output).
pub fn pe_bench(sc: &Scenario, days: f64) -> Result<(f64, usize, f64), String> {
    let setup = sc.build(Init::Equilibrium)?;
    let model = setup.model;
    let mut st = setup.state;
    if model.phys.is_none() {
        release_dust(&model, &mut st, sc.s.outputs.dust_release);
    }
    let mut w = model.new_work();
    let chunk_s = sc.chunk_s();
    let courant = sc.s.numerics.courant;
    run_chunks(&model, &mut st, 2, chunk_s, courant, &mut w)?;
    let t0 = Instant::now();
    let steps = run_chunks(&model, &mut st, sc.s.chunks(days), chunk_s, courant, &mut w)?;
    let el = t0.elapsed().as_secs_f64();
    Ok((days / el, steps, days * sc.body.day_s / steps as f64))
}
