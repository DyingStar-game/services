//! Acceptance tests of the primitive-equation model (steps 3 and 3b), on Sandbox with the exact
//! step-3b settings (`tests/data/sandbox_step3b.toml`). Heavy tests share one lock so that at most
//! one 4-thread pool works at a time (never more than 4 threads in total).

mod common;

use std::sync::Mutex;
use weather_atmosphere::pe::{ChunkOutcome, Guard, PeState, RefProfile};
use weather_atmosphere::run;
use weather_atmosphere::setup::{Init, Scenario};
use weather_atmosphere::with_threads;

static LOCK: Mutex<()> = Mutex::new(());

fn cfg() -> Scenario {
    common::test_scenario("sandbox_step3b.toml")
}

/// The same on the level-4 grid (480 km, ~16 times cheaper): conservation, restarts and the guard
/// do not depend on the grid, and the test suite stays short enough for the CI.
fn cfg_fast() -> Scenario {
    let mut c = cfg();
    c.s.grid.level = 4;
    c
}

fn setup(c: &Scenario) -> weather_atmosphere::setup::PeSetup {
    c.build(Init::Equilibrium).unwrap()
}

/// Held-Suarez Sandbox from rest, dust released at once, `chunks` chunks of 1/24 day.
fn hs_run(c: &Scenario, chunks: u64) -> (weather_atmosphere::pe::PeModel, PeState, f64, f64) {
    let s = setup(c);
    let model = s.model;
    let mut st = s.state;
    run::release_dust(&model, &mut st, c.s.outputs.dust_release);
    let d0 = model.diag(&st);
    let mut w = model.new_work();
    run::run_chunks(&model, &mut st, chunks, c.chunk_s(), c.s.numerics.courant, &mut w).unwrap();
    (model, st, d0.air_mass, d0.dust_mass)
}

/// Air mass (sum of ps * area / g) and dust mass are conserved to round-off; dust stays >= 0.
#[test]
fn pe_mass_and_dust_conservation() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg_fast();
    let (model, st, air0, dust0) = with_threads(4, || hs_run(&c, 2 * 24));
    let d = model.diag(&st);
    let (da, dd) = (d.air_mass / air0 - 1.0, d.dust_mass / dust0 - 1.0);
    println!(
        "PE Sandbox 2 days: air mass drift {da:.2e}, dust mass drift {dd:.2e}, min dust {:.2e}, max |v| {:.1} m/s, ps {:.0}..{:.0} hPa",
        d.min_dust,
        d.max_speed,
        d.min_ps / 100.0,
        d.max_ps / 100.0
    );
    assert!(da.abs() < 1e-13, "air mass drift {da}");
    assert!(dd.abs() < 1e-12, "dust mass drift {dd}");
    assert!(d.min_dust >= -1e-300, "negative dust {}", d.min_dust);
    assert!(d.finite);
}

/// Resting atmosphere over the real relief, no forcing, 1 day: the spurious winds measure the
/// pressure-gradient error. Isothermal and the run's own reference profile must stay at rest
/// to round-off. A profile that differs from the reference (5 K/km from 293 K, 215 K above) is
/// the honest test: with the hybrid coordinate (default: 8 layers, del^8, 4 h) the largest wind
/// after one day was measured at 1.47 m/s and the rms of the lowest layer at 0.37 m/s (pure
/// sigma, same settings: 17.5 and 1.02 m/s; step 3, 5 sigma layers: 16.4 and 3.5 m/s). Thresholds: 3 m/s and
/// 0.6 m/s, and at least 5 times better than pure sigma on the largest wind (NOTES.md, section 18).
#[test]
fn pe_resting_atmosphere_over_relief() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg();
    let iso = RefProfile { t0: 283.0, lapse: 0.0, t_strat: 283.0 };
    let refp = c.tref();
    let other = RefProfile { t0: 293.0, lapse: 0.005, t_strat: 215.0 };
    let (iso_max, iso_rms, slope) = with_threads(4, || run::rest_test(&c, iso, iso, true, 1.0)).unwrap();
    println!("rest, isothermal 283 K, max slope {slope:.4}: max |v| {iso_max:.2e} m/s, rms lowest layer {iso_rms:.2e} m/s");
    let (ref_max, ref_rms, _) = with_threads(4, || run::rest_test(&c, refp, refp, true, 1.0)).unwrap();
    println!("rest, run reference profile: max |v| {ref_max:.2e} m/s, rms lowest layer {ref_rms:.2e} m/s");
    let (o_max, o_rms, _) = with_threads(4, || run::rest_test(&c, refp, other, true, 1.0)).unwrap();
    println!("rest, other profile, hybrid: max |v| {o_max:.2} m/s, rms lowest layer {o_rms:.2} m/s");
    let mut cs = c.clone();
    cs.s.grid.hybrid_eta_t = Some(0.0);
    let (s_max, s_rms, _) = with_threads(4, || run::rest_test(&cs, refp, other, true, 1.0)).unwrap();
    println!("rest, other profile, pure sigma: max |v| {s_max:.2} m/s, rms lowest layer {s_rms:.2} m/s");
    assert!(iso_max < 1e-8, "isothermal rest state moved: {iso_max}");
    assert!(ref_max < 1e-8, "reference rest state moved: {ref_max}");
    assert!(o_max < 3.0, "pressure-gradient error too large: max {o_max} m/s");
    assert!(o_rms < 0.6, "pressure-gradient error too large: rms {o_rms} m/s");
    assert!(o_max * 5.0 < s_max, "hybrid ({o_max}) not clearly better than sigma ({s_max})");
}

/// A run of 2 days equals, bit for bit, 1 day + restart file + 1 day.
#[test]
fn pe_restart_bit_for_bit() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg_fast();
    let path = std::env::temp_dir().join("weather_atmosphere_pe_restart_test.bin");
    let (a, b) = with_threads(4, || {
        let (_, full, ..) = hs_run(&c, 48);
        let (model, half, ..) = hs_run(&c, 24);
        model.write_restart(&half, &path).unwrap();
        // A fresh model, as a new process would build it, then the restart file.
        let s = setup(&c);
        let model2 = s.model;
        let mut st = model2.read_restart(&path).unwrap();
        assert!(st.bit_equal(&half), "restart file does not round-trip");
        let mut w = model2.new_work();
        run::run_chunks(&model2, &mut st, 24, c.chunk_s(), c.s.numerics.courant, &mut w).unwrap();
        (full, st)
    });
    println!("restart: time {} s / {} s, chunk {} / {}", a.time, b.time, a.chunk, b.chunk);
    assert!(a.bit_equal(&b), "2 days != 1 day + restart + 1 day");
}

/// Same chunks with 1 and 4 threads give bit-identical states.
#[test]
fn pe_threads_bit_for_bit() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg_fast();
    let (_, a, ..) = with_threads(1, || hs_run(&c, 6));
    let (_, b, ..) = with_threads(4, || hs_run(&c, 6));
    assert!(a.bit_equal(&b), "1 thread and 4 threads differ");
}

/// The guard stops on an out-of-bounds surface pressure instead of running on.
#[test]
fn pe_guard_stops() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg();
    with_threads(4, || {
        let s = setup(&c);
        let model = s.model;
        let mut st = s.state;
        st.ps[100] = 1.0e3; // far below any real surface pressure
        assert!(model.check(&st).is_err());
        st.ps[100] = f64::NAN;
        assert!(model.check(&st).is_err());
    });
}

/// Guard with roll-back: one forced failure (a NaN surface pressure at the end of chunk 30) sends
/// the state back to the last model-day boundary (chunk 24); that day is run again with half the
/// time step, then the run goes on normally. A restart file written during the reduced day
/// continues bit for bit.
#[test]
fn pe_guard_rolls_back_and_recovers() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg_fast();
    let path = std::env::temp_dir().join("weather_atmosphere_pe_guard_restart.bin");
    with_threads(4, || {
        let s = setup(&c);
        let model = s.model;
        let mut st = s.state;
        run::release_dust(&model, &mut st, c.s.outputs.dust_release);
        let air0 = model.diag(&st).air_mass;
        let cpd = c.chunks_per_day();
        let chunk_s = c.chunk_s();
        let mut w = model.new_work();
        model.fault_chunk.store(30, std::sync::atomic::Ordering::Relaxed);
        model.fault_count.store(1, std::sync::atomic::Ordering::Relaxed);
        let mut g = Guard::new(&st);
        let mut steps = vec![0usize; 2 * cpd as usize + 1];
        let mut rollbacks = 0;
        let mut mid: Option<PeState> = None;
        while st.chunk < 2 * cpd {
            match model.advance_guarded(&mut st, &mut g, chunk_s, c.s.numerics.courant, cpd, &mut w).unwrap() {
                ChunkOutcome::Done(n) => steps[st.chunk as usize] = n,
                ChunkOutcome::RolledBack(msg) => {
                    println!("{msg}");
                    rollbacks += 1;
                    assert_eq!(st.chunk, cpd, "roll-back must return to the day boundary");
                    assert_eq!(st.dt_div, 2);
                    assert_eq!(g.last_good.chunk, cpd);
                }
            }
            if st.chunk == 36 && rollbacks == 1 && mid.is_none() {
                model.write_restart(&st, &path).unwrap();
                mid = Some(st.clone());
            }
        }
        let day0: usize = steps[1..=cpd as usize].iter().sum();
        let day1: usize = steps[cpd as usize + 1..].iter().sum();
        let d = model.diag(&st);
        println!("guard: {rollbacks} roll-back(s), steps day 0 {day0}, day 1 (halved step) {day1}, air drift {:.1e}", d.air_mass / air0 - 1.0);
        assert_eq!(rollbacks, 1);
        assert_eq!(g.events.len(), 1);
        assert!(day1 as f64 >= 1.8 * day0 as f64, "day 1 should use about twice as many steps");
        assert!(d.finite && st.dt_div == 1 && g.fails == 0);
        assert!((d.air_mass / air0 - 1.0).abs() < 1e-13);
        // Restart taken during the reduced day: a fresh model goes on bit for bit.
        let s2 = setup(&c);
        let model2 = s2.model;
        let mut st2 = model2.read_restart(&path).unwrap();
        assert!(st2.bit_equal(mid.as_ref().unwrap()));
        assert_eq!(st2.dt_div, 2);
        let mut g2 = Guard::new(&st2);
        let mut w2 = model2.new_work();
        while st2.chunk < 2 * cpd {
            model2.advance_guarded(&mut st2, &mut g2, chunk_s, c.s.numerics.courant, cpd, &mut w2).unwrap();
        }
        assert!(st2.bit_equal(&st), "restart during the reduced day is not bit for bit");
    });
}

/// Guard: a failure that comes back three times in a row (normal, half and quarter step) stops
/// the run for good, the last good state being the day boundary.
#[test]
fn pe_guard_stops_after_three_failures() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg_fast();
    with_threads(4, || {
        let s = setup(&c);
        let model = s.model;
        let mut st = s.state;
        let cpd = c.chunks_per_day();
        let mut w = model.new_work();
        model.fault_chunk.store(cpd + 2, std::sync::atomic::Ordering::Relaxed);
        model.fault_count.store(10, std::sync::atomic::Ordering::Relaxed);
        let mut g = Guard::new(&st);
        let mut res = Ok(ChunkOutcome::Done(0));
        while st.chunk < 3 * cpd {
            res = model.advance_guarded(&mut st, &mut g, c.chunk_s(), c.s.numerics.courant, cpd, &mut w);
            if res.is_err() {
                break;
            }
        }
        println!("{:?}", g.events);
        assert!(res.is_err(), "the guard should have stopped");
        assert_eq!(g.events.len(), 3);
        assert_eq!(g.last_good.chunk, cpd);
    });
}
