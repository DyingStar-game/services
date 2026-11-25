//! Acceptance tests of the step-4 column physics (ground, day-night cycle, dust, dust sea, lid,
//! events). Heavy tests share one lock (never more than one 4-thread pool at a time). Most run on
//! the level-4 grid (480 km) to stay short; the physics is per column, so the grid does not matter.

mod common;

use std::sync::Mutex;
use weather_atmosphere::pe::{ChunkOutcome, Guard, PeModel, PeState, PeWork};
use weather_atmosphere::phys::ground::Ground;
use weather_atmosphere::phys::sea::DustSea;
use weather_atmosphere::phys::{di, Event, SIGMA_SB};
use weather_atmosphere::run;
use weather_atmosphere::setup::{Init, Scenario};
use weather_atmosphere::with_threads;

static LOCK: Mutex<()> = Mutex::new(());

/// Sandbox with the exact step-4 settings (`tests/data/sandbox_step4.toml`) on a level-`level` grid.
fn cfg4(level: u32) -> Scenario {
    let mut c = common::test_scenario("sandbox_step4.toml");
    c.s.grid.level = level;
    c
}

fn setup(c: &Scenario) -> (PeModel, PeState, PeWork) {
    let s = c.build(Init::Equilibrium).unwrap();
    let w = s.model.new_work();
    (s.model, s.state, w)
}

fn courant(c: &Scenario) -> f64 {
    c.s.numerics.courant
}

/// Total dust (both classes) in kg.
fn dust_total(m: &PeModel, st: &PeState) -> f64 {
    let d = m.diag(st);
    d.dust_mass + d.dust2_mass
}

/// A resting atmosphere on a flat, uniform ground under a steady sun overhead everywhere (no day
/// and night, no dust, no lid; Held-Suarez with no horizontal contrast, relaxed in 2 days so that
/// the test is short) reaches an equilibrium: the ground and the air stop changing, the ground's
/// energy budget closes (sunlight + down-welling infrared = emitted infrared + sensible heat) and
/// the air stays at rest.
/// Slow (20 model days, ~25 s optimised on 4 threads): ignored by default, run with `--ignored`.
#[test]
#[ignore = "slow: 20 model days; run with --ignored"]
fn phys_energy_balance_equilibrium() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = cfg4(4);
    c.s.relief.scale = 0.0;
    c.s.relaxation.seasons = false;
    c.s.relaxation.equator_pole_contrast_k = Some(0.0);
    c.s.relaxation.static_stability_k = 0.0;
    c.s.numerics.noise_k = 0.0;
    c.s.ground.uniform_sun_w_m2 = Some(400.0);
    c.s.dust.lifting = false;
    c.s.dust.radiative = false;
    c.s.sea.enabled = false;
    c.s.lid.enabled = false;
    // Held-Suarez relaxed in 2 days so that the test is short.
    c.s.relaxation.slow_time_s = Some(2.0 * 86400.0);
    c.s.relaxation.fast_time_s = 2.0 * 86400.0;
    with_threads(4, || {
        let (model, mut st, mut w) = setup(&c);
        let l = model.nl;
        let cpd = c.chunks_per_day();
        run::run_chunks(&model, &mut st, 19 * cpd, c.chunk_s(), courant(&c), &mut w).unwrap();
        let tg0 = st.tg.clone();
        let tl0: Vec<f64> = (0..model.grid.n_cells()).map(|i| model.temperature(&st, i, l - 1)).collect();
        run::run_chunks(&model, &mut st, cpd, c.chunk_s(), courant(&c), &mut w).unwrap();
        let ph = model.phys.as_ref().unwrap();
        let eps = ph.find::<Ground>().unwrap().eps_air;
        let n = model.grid.n_cells();
        let mut dtg: f64 = 0.0;
        let mut dtl: f64 = 0.0;
        let mut resid: f64 = 0.0;
        for i in 0..n {
            dtg = dtg.max((st.tg[i] - tg0[i]).abs());
            dtl = dtl.max((model.temperature(&st, i, l - 1) - tl0[i]).abs());
            let r = &w.ph.diag[i * di::N..(i + 1) * di::N];
            let g = r[di::SWG] + eps * SIGMA_SB * r[di::TA].powi(4) - SIGMA_SB * r[di::TG].powi(4) - r[di::H];
            resid = resid.max(g.abs());
        }
        let d = model.diag(&st);
        let r0 = &w.ph.diag[0..di::N];
        println!(
            "equilibrium: Tg {:.3} K, Ta {:.3} K, H {:.2} W/m2, sunlight absorbed {:.1} W/m2; last day: max |dTg| {dtg:.2e} K, max |dT lowest layer| {dtl:.2e} K; max ground budget residual {resid:.2e} W/m2; max |v| {:.2e} m/s",
            r0[di::TG],
            r0[di::TA],
            r0[di::H],
            r0[di::SWG],
            d.max_speed
        );
        assert!(dtg < 0.02, "ground still changing: {dtg} K per day");
        assert!(dtl < 0.02, "air still changing: {dtl} K per day");
        assert!(resid < 0.5, "ground energy budget does not close: {resid} W/m2");
        assert!(d.max_speed < 0.1, "the uniform atmosphere moved: {} m/s", d.max_speed);
        assert!((r0[di::SWG] - 0.39 * 400.0).abs() < 1e-9);
    });
}

/// Dust mass budget: with lifting (threshold lowered so that it happens everywhere), settling,
/// deposition, transport and mixing, the change of the total dust equals lifted - deposited, to
/// round-off; with the dust sea on, plus what the sea relaxation added. Dust stays >= 0.
#[test]
fn phys_dust_mass_budget() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for sea in [false, true] {
        let mut c = cfg4(4);
        c.s.dust.threshold_pa = Some(0.005);
        c.s.dust.alpha_per_m = Some(1.0e-6);
        c.s.sea.enabled = sea;
        with_threads(4, || {
            let (model, mut st, mut w) = setup(&c);
            let m0 = dust_total(&model, &st);
            run::run_chunks(&model, &mut st, 2 * c.chunks_per_day(), c.chunk_s(), courant(&c), &mut w).unwrap();
            let m1 = dust_total(&model, &st);
            let [lift, dep, seaadd, ev] = st.budget;
            let expect = lift - dep + seaadd + ev;
            let err = ((m1 - m0) - expect).abs() / lift.abs().max(seaadd.abs());
            let d = model.diag(&st);
            println!(
                "dust budget (sea {sea}): lifted {lift:.4e} kg, deposited {dep:.4e} kg, sea {seaadd:.4e} kg, change {:.4e} kg, relative error {err:.2e}, min dust {:.2e} / {:.2e}",
                m1 - m0,
                d.min_dust,
                d.min_dust2
            );
            assert!(lift > 0.0 && dep > 0.0, "lifting and deposition must both happen");
            assert!(err < 1e-9, "dust budget error {err}");
            assert!(d.min_dust >= -1e-300 && d.min_dust2 >= -1e-300, "negative dust");
            if sea {
                assert!(seaadd > 0.0);
            }
        });
    }
}

/// The dust-sea relaxation builds and keeps the layer: from no dust and without lifting, after
/// 15 days the fine dust inside 500-3 500 m is near the target (measured 0.86 of it with a 1-day
/// refill) and the air above 6 km stays much clearer (measured 0.16 of the target: daytime
/// convection over the plateaus lifts some; bound 0.2). Area-weighted means over the layers that
/// lie fully in each range.
/// Slow (15 model days, ~20 s optimised on 4 threads): ignored by default, run with `--ignored`.
#[test]
#[ignore = "slow: 15 model days; run with --ignored"]
fn phys_sea_relaxation_keeps_the_layer() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = cfg4(4);
    c.s.dust.lifting = false;
    with_threads(4, || {
        let (model, mut st, mut w) = setup(&c);
        run::run_chunks(&model, &mut st, 15 * c.chunks_per_day(), c.chunk_s(), courant(&c), &mut w).unwrap();
        let qt = model.phys.as_ref().unwrap().find::<DustSea>().unwrap().q_target;
        let l = model.nl;
        let (mut sin, mut win, mut sab, mut wab, mut slo, mut wlo) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for i in 0..model.grid.n_cells() {
            let (zh, _) = model.column_heights(&st, i);
            let a = model.grid.area[i];
            for k in 1..l {
                let q = st.m[i * l + k] / model.dp(k, st.ps[i]);
                if zh[k + 1] >= 600.0 && zh[k] <= 3400.0 {
                    sin += q * a;
                    win += a;
                }
                if zh[k + 1] >= 6000.0 {
                    sab += q * a;
                    wab += a;
                }
                if zh[k] <= 400.0 {
                    slo += q * a;
                    wlo += a;
                }
            }
        }
        let (qin, qab, qlo) = (sin / win / qt, sab / wab / qt, slo / wlo.max(1e-30) / qt);
        println!("dust sea after 15 days: fine dust / target inside 500-3500 m {qin:.3}, above 6 km {qab:.4}, below 500 m {qlo:.3}");
        assert!(qin > 0.75 && qin < 1.3, "the sea is not held near its target: {qin}");
        assert!(qab < 0.2, "the air above the sea is not clear: {qab}");
    });
}

/// A large game event (heat and dust over a 500 km patch, 12 h) is injected little by little:
/// half of the dust is in after half the duration, all of it at the end, exactly; the guard never
/// has to roll back, and the dust budget still closes.
#[test]
fn phys_large_event_is_stable() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg4(4);
    with_threads(4, || {
        let (model, mut st, mut w) = setup(&c);
        let cpd = c.chunks_per_day();
        let chunk_s = c.chunk_s();
        run::run_chunks(&model, &mut st, cpd, chunk_s, courant(&c), &mut w).unwrap();
        let dust_kg = 2.0e10;
        // 1e15 W over ~4e11 m^2: a few thousand W/m^2 at the centre, for 12 hours.
        let ev = Event {
            lon_deg: -39.0,
            lat_deg: 22.0,
            radius_m: 500e3,
            start_s: st.time + 2.0 * chunk_s,
            duration_s: 12.0 * chunk_s,
            heat_w: 1.0e15,
            dust_kg,
            fine_frac: 0.3,
        };
        model.inject_event(&mut st, ev).unwrap();
        let mut g = Guard::new(&st);
        let m0 = dust_total(&model, &st);
        let b0 = st.budget;
        let mut half = 0.0;
        let mut max_t: f64 = 0.0;
        for c_ in 0..(2 * cpd) {
            match model.advance_guarded(&mut st, &mut g, chunk_s, courant(&c), cpd, &mut w).unwrap() {
                ChunkOutcome::Done(_) => {}
                ChunkOutcome::RolledBack(m) => panic!("the guard rolled back: {m}"),
            }
            if c_ + 1 == 8 {
                half = st.budget[3] - b0[3];
            }
            max_t = max_t.max(model.diag(&st).max_t);
        }
        let injected = st.budget[3] - b0[3];
        let m1 = dust_total(&model, &st);
        let [lift, dep, sea, evb] = st.budget;
        let err = ((m1 - m0) - ((lift - b0[0]) - (dep - b0[1]) + (sea - b0[2]) + (evb - b0[3]))).abs() / injected;
        println!(
            "event: dust injected after half the duration {:.4} of the total, at the end {:.12}; budget error {err:.2e}; max T {max_t:.1} K; guard events {}",
            half / dust_kg,
            injected / dust_kg,
            g.events.len()
        );
        assert!((half / dust_kg - 0.5).abs() < 1e-9, "the injection is not spread over time: {}", half / dust_kg);
        assert!((injected / dust_kg - 1.0).abs() < 1e-9, "not all the dust was injected");
        assert!(err < 1e-9);
        assert!(g.events.is_empty());
        assert!(model.diag(&st).finite);
    });
}

/// Restart with the step-4 state (ground temperatures, two dust classes, budget, event journal):
/// 2 days in one go equal 1 day + restart file + 1 day, bit for bit, with an event that is
/// running across the restart. Also: 1 thread and 4 threads give the same state, bit for bit.
#[test]
fn phys_restart_and_threads_bit_for_bit() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = cfg4(4);
    let cpd = c.chunks_per_day();
    let path = std::env::temp_dir().join("weather_atmosphere_phys_restart_test.bin");
    let ev = |t0: f64| Event { lon_deg: 10.0, lat_deg: -5.0, radius_m: 400e3, start_s: t0, duration_s: 20.0 * c.chunk_s(), heat_w: 2.0e14, dust_kg: 5.0e9, fine_frac: 0.5 };
    let run = |chunks: u64| {
        let (model, mut st, mut w) = setup(&c);
        model.inject_event(&mut st, ev((cpd - 6) as f64 * c.chunk_s())).unwrap();
        run::run_chunks(&model, &mut st, chunks, c.chunk_s(), courant(&c), &mut w).unwrap();
        (model, st)
    };
    let (a, b) = with_threads(4, || {
        let (_, full) = run(2 * cpd);
        let (model, half) = run(cpd);
        model.write_restart(&half, &path).unwrap();
        let (model2, _, mut w2) = setup(&c);
        let mut st = model2.read_restart(&path).unwrap();
        assert!(st.bit_equal(&half), "restart file does not round-trip");
        run::run_chunks(&model2, &mut st, cpd, c.chunk_s(), courant(&c), &mut w2).unwrap();
        (full, st)
    });
    println!("restart: events {} / {}, budget {:?}", a.events.len(), b.events.len(), b.budget);
    assert!(a.bit_equal(&b), "2 days != 1 day + restart + 1 day (step 4)");
    let (_, s1) = with_threads(1, || run(6));
    let (_, s4) = with_threads(4, || run(6));
    assert!(s1.bit_equal(&s4), "1 thread and 4 threads differ (step 4)");
}
