//! The model's sky follows the body's real orbit and spin (`ephemeris`): declination, sub-stellar
//! longitude and distance equal what `weather_core` (the service, the game's twins) gives at the
//! same Unix time; the sunniest cell of the model sits under the sub-stellar point given by
//! `weather_core` (same longitude convention as `spin::dir_of`); a resume keeps it all bit for bit.

mod common;

use std::sync::Arc;
use weather_atmosphere::ephemeris::Ephemeris;
use weather_atmosphere::phys::di;
use weather_atmosphere::run::run_chunks;
use weather_atmosphere::setup::{Init, Scenario};
use weather_atmosphere::vec3::{angle, from_lonlat};
use weather_atmosphere::with_threads;
use weather_core::climate::BodyInputs;
use weather_core::spin::{declination_deg, substellar_point};

/// A cold start at this Unix time (2026-11-25), with a spin-up of `SPINUP_S` before it.
const NOW: f64 = 1_795_600_000.0;
const SPINUP_S: f64 = 10.0 * 90_551.5;

fn sandbox_inputs() -> BodyInputs {
    let f = common::files();
    let system = std::fs::read_to_string(&f.system).unwrap();
    let extras = std::fs::read_to_string(f.extras.unwrap()).unwrap();
    weather_core::bodies::load(&system, Some(&extras)).unwrap().into_iter().find(|b| b.name.eq_ignore_ascii_case("sandbox")).unwrap()
}

fn ephemeris(b: &BodyInputs, offset: f64) -> Arc<Ephemeris> {
    // Sandbox is a planet: its star orbit is its own (a moon would take its planet's).
    Arc::new(Ephemeris { unix_offset_s: offset, tilt_rad: b.tilt_deg.to_radians(), rotation_period_s: b.rotation_period_s, star_orbit: b.orbit.solver(b.mass_kg) })
}

/// What the weather service computes for the same body at Unix time `u` (`state.to_star`,
/// `declination_deg`, `substellar_point`), in degrees.
fn service_sky(b: &BodyInputs, u: f64) -> (f64, f64) {
    let to_star = b.orbit.solver(b.mass_kg).position_at(u).scale(-1.0);
    let decl = declination_deg(b.tilt_deg.to_radians(), to_star);
    let (lon, _) = substellar_point(b.tilt_deg.to_radians(), b.rotation_period_s, u, to_star);
    (decl, lon)
}

fn scenario(b: &BodyInputs, level: u32) -> Scenario {
    let mut sc = Scenario::for_body(&common::settings_dir(), "Sandbox", &[], &common::files()).unwrap();
    sc.s.grid.level = level;
    sc.ephemeris = Some(ephemeris(b, NOW - SPINUP_S));
    sc
}

fn wrap_deg(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

#[test]
fn model_sky_equals_weather_core() {
    let b = sandbox_inputs();
    let sc = scenario(&b, 3);
    let s = with_threads(2, || sc.build(Init::Equilibrium)).unwrap();
    let sun = &s.model.phys.as_ref().unwrap().sun;
    let relax = s.model.relax.as_ref().unwrap();
    // At the end of the spin-up the model is at the real instant NOW; check it and a year around.
    let year = b.orbit.solver(b.mass_kg).period_s();
    for k in 0..40 {
        let t = SPINUP_S + (k as f64 - 20.0) * year / 40.0 + 1234.5 * k as f64;
        let (decl, lon, flux) = sun.position(t);
        let (decl_c, lon_c) = service_sky(&b, t + NOW - SPINUP_S);
        assert!((decl.to_degrees() - decl_c).abs() < 1e-9, "declination {} vs {decl_c}", decl.to_degrees());
        assert!(wrap_deg(lon.to_degrees() - lon_c).abs() < 1e-9, "sub-stellar longitude {} vs {lon_c}", lon.to_degrees());
        assert!((relax.subsolar_lat(t) - decl).abs() < 1e-15, "the relaxation's season differs from the sun's");
        assert!(flux > 0.0);
    }
    // The tilt shows: over a year the declination swings by about +-15 degrees.
    let decls: Vec<f64> = (0..200).map(|k| sun.position(k as f64 * year / 200.0).0.to_degrees()).collect();
    let (lo, hi) = decls.iter().fold((f64::MAX, f64::MIN), |(a, c), &x| (a.min(x), c.max(x)));
    assert!(hi > 14.0 && lo < -14.0, "declination range {lo} .. {hi}");
    // Distance: the flux factor averages to 1 over an orbit (the model's flux stays the data's
    // annual mean), and swings with the eccentricity.
    let n = 20_000;
    let e = ephemeris(&b, 0.0);
    let mean: f64 = (0..n).map(|k| e.at(k as f64 * year / n as f64).flux_factor).sum::<f64>() / n as f64;
    let ecc = b.orbit.solver(b.mass_kg).eccentricity();
    println!("NOW: decl {:.3} deg, sub-stellar lon {:.3} deg; flux factor mean {mean:.6} (eccentricity {ecc:.4})", service_sky(&b, NOW).0, service_sky(&b, NOW).1);
    assert!((mean - 1.0).abs() < 1e-4, "flux factor mean {mean}");
}

/// The sunniest model cell (largest cosine of the zenith angle, from the column physics) sits
/// under the sub-stellar point that weather_core gives, at the game's longitudes and latitudes.
#[test]
fn sunniest_cell_under_the_substellar_point() {
    let _g = common::heavy();
    let b = sandbox_inputs();
    let sc = scenario(&b, 4);
    with_threads(4, || {
        let s = sc.build(Init::Equilibrium).unwrap();
        let (model, mut st) = (s.model, s.state);
        let mut w = model.new_work();
        run_chunks(&model, &mut st, 3, sc.chunk_s(), sc.s.numerics.courant, &mut w).unwrap();
        let n = model.grid.n_cells();
        let best = (0..n).max_by(|&i, &j| w.ph.diag[i * di::N + di::COSZ].partial_cmp(&w.ph.diag[j * di::N + di::COSZ]).unwrap()).unwrap();
        let (decl_c, lon_c) = service_sky(&b, st.time + NOW - SPINUP_S);
        let cell = model.grid.x[best];
        let sub = from_lonlat(lon_c.to_radians(), decl_c.to_radians());
        let off = angle(cell, sub).to_degrees();
        println!(
            "sunniest cell lon {:.1} lat {:.1}; weather_core sub-stellar point lon {lon_c:.1} lat {decl_c:.1}; {off:.2} deg apart",
            model.grid.lon[best].to_degrees(),
            model.grid.lat[best].to_degrees()
        );
        // Level 4 cells are ~4 degrees apart; the last step's middle is up to a few minutes back.
        assert!(off < 6.0, "the sunniest cell is {off} degrees from the sub-stellar point");
    });
}

/// A resume keeps the sky: 4 chunks in one go = 2 chunks + restart file + a model rebuilt with the
/// same offset + 2 chunks, bit for bit (and the sky at the resumed time is the real one).
#[test]
fn resume_keeps_the_sky() {
    let _g = common::heavy();
    let b = sandbox_inputs();
    let sc = scenario(&b, 3);
    let path = std::env::temp_dir().join("weather_atmosphere_sky_restart.bin");
    with_threads(2, || {
        let run = |chunks: u64| {
            let s = sc.build(Init::Equilibrium).unwrap();
            let (model, mut st) = (s.model, s.state);
            let mut w = model.new_work();
            run_chunks(&model, &mut st, chunks, sc.chunk_s(), sc.s.numerics.courant, &mut w).unwrap();
            (model, st)
        };
        let (_, full) = run(4);
        let (model, half) = run(2);
        model.write_restart(&half, &path).unwrap();
        // A new process: the offset comes back from the checkpoint's metadata.
        let mut sc2 = sc.clone();
        sc2.ephemeris = Some(ephemeris(&b, sc.ephemeris.as_ref().unwrap().unix_offset_s));
        let s2 = sc2.build(Init::Equilibrium).unwrap();
        let mut st = s2.model.read_restart(&path).unwrap();
        let (decl, lon, _) = s2.model.phys.as_ref().unwrap().sun.position(st.time);
        let (decl_c, lon_c) = service_sky(&b, st.time + NOW - SPINUP_S);
        assert!((decl.to_degrees() - decl_c).abs() < 1e-9 && wrap_deg(lon.to_degrees() - lon_c).abs() < 1e-9);
        let mut w = s2.model.new_work();
        run_chunks(&s2.model, &mut st, 2, sc.chunk_s(), sc.s.numerics.courant, &mut w).unwrap();
        assert!(st.bit_equal(&full), "resumed run diverged");
    });
}
