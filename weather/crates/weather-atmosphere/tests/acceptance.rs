//! Acceptance tests. Heavy tests share one lock so that at most one 4-thread
//! pool works at a time (never more than 4 threads in total).

mod common;

use std::f64::consts::PI;
use std::sync::Mutex;
use weather_atmosphere::cases::{self, ShallowCfg};
use weather_atmosphere::grid::Grid;
use weather_atmosphere::model::State;
use weather_atmosphere::vec3::*;
use weather_atmosphere::with_threads;

static LOCK: Mutex<()> = Mutex::new(());

/// The single layer on Sandbox with the step-4 numbers (the step-2 run used the same).
fn sandbox_cfg() -> ShallowCfg {
    ShallowCfg::from_scenario(&common::test_scenario("sandbox_step4.toml"))
}

#[test]
fn grid_geometry() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for level in [2u32, 5] {
        let r = 6.356e6;
        let g = Grid::new(level, r);
        assert_eq!(g.n_cells(), 10 * 4usize.pow(level) + 2);
        let pent = (0..g.n_cells()).filter(|&i| g.c_start[i + 1] - g.c_start[i] == 5).count();
        assert_eq!(pent, 12);
        let total: f64 = g.area.iter().sum();
        let exact = 4.0 * PI * r * r;
        assert!(((total - exact) / exact).abs() < 1e-12, "area sum {total} vs {exact}");
        assert_eq!(g.n_edges(), 3 * (g.n_cells() - 2));
        assert!(g.e_l.iter().all(|&l| l > 0.0));
    }
    let g = Grid::new(5, 6.356e6);
    let (amin, amax) = g.area.iter().fold((f64::MAX, 0.0f64), |(a, b), &x| (a.min(x), b.max(x)));
    println!("level 5: mean spacing {:.1} km, area ratio max/min {:.3}", g.mean_spacing() / 1e3, amax / amin);
}

/// Discrete gradient and Laplacian of z = sin(lat) against the exact values.
#[test]
fn operators_accuracy() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let r = 6.356e6;
    let g = Grid::new(5, r);
    let z: Vec<f64> = g.x.iter().map(|x| x[2]).collect();
    let (mut eg, mut ng, mut el, mut nl) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..g.n_cells() {
        let mut gr = [0.0; 3];
        let mut lap = 0.0;
        for s in g.c_start[i]..g.c_start[i + 1] {
            let e = g.c_edge[s];
            let j = g.c_nbr[s];
            gr = add(gr, scale(g.e_n[e], g.c_sign[s] * 0.5 * (z[j] - z[i]) * g.e_l[e]));
            lap += (z[j] - z[i]) * g.e_l[e] / g.e_d[e];
        }
        let gr = tangent(scale(gr, 1.0 / g.area[i]), g.x[i]);
        let lap = lap / g.area[i];
        let ex = scale(tangent([0.0, 0.0, 1.0], g.x[i]), 1.0 / r);
        let exl = -2.0 * z[i] / (r * r);
        let d = sub(gr, ex);
        eg += g.area[i] * dot(d, d);
        ng += g.area[i] * dot(ex, ex);
        el += g.area[i] * (lap - exl).powi(2);
        nl += g.area[i] * exl * exl;
    }
    let (eg, el) = ((eg / ng).sqrt(), (el / nl).sqrt());
    println!("relative l2 error: gradient {eg:.2e}, Laplacian {el:.2e}");
    assert!(eg < 1e-2 && el < 2e-2);
}

/// Williamson TC1: cosine bell advected once around the sphere.
#[test]
fn williamson_tc1() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut l2_5 = 0.0;
    for alpha in [0.0, PI / 2.0 - 0.05] {
        let r = with_threads(4, || cases::tc1(5, alpha, 0.5));
        println!(
            "TC1 L5 alpha={alpha:.3}: l1={:.4} l2={:.4} linf={:.4} min={:.2e} max={:.1} mass drift={:.1e}",
            r.norms.l1, r.norms.l2, r.norms.linf, r.min, r.max, r.mass_drift
        );
        assert!(r.norms.l1 < 0.2 && r.norms.l2 < 0.15 && r.norms.linf < 0.2);
        assert!(r.min >= -1e-300, "positivity violated: {}", r.min);
        assert!(r.mass_drift.abs() < 1e-12);
        l2_5 = r.norms.l2;
    }
    let r6 = with_threads(4, || cases::tc1(6, PI / 2.0 - 0.05, 0.5));
    println!("TC1 L6: l1={:.4} l2={:.4} linf={:.4}", r6.norms.l1, r6.norms.l2, r6.norms.linf);
    assert!(r6.norms.l2 < l2_5 / 3.0, "no convergence with resolution");
}

/// Williamson TC2: steady geostrophic zonal flow, Earth numbers, 5 days.
#[test]
fn williamson_tc2() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for alpha in [0.0, PI / 4.0] {
        let r = with_threads(4, || cases::tc2(5, alpha, 5.0, 0.8, cases::DEFAULT_TAU_HOURS));
        println!(
            "TC2 L5 alpha={alpha:.3}: l1={:.3e} l2={:.3e} linf={:.3e} |v| l2={:.3e} mass drift={:.1e}",
            r.norms.l1, r.norms.l2, r.norms.linf, r.vel_l2, r.mass_drift
        );
        assert!(r.norms.l2 < 1e-3 && r.norms.linf < 5e-3);
        assert!(r.mass_drift.abs() < 1e-12);
    }
}

/// Williamson TC5: zonal flow over an isolated mountain, 15 days.
#[test]
fn williamson_tc5() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let r = with_threads(4, || cases::tc5(5, 15.0, 0.8, cases::DEFAULT_TAU_HOURS));
    println!(
        "TC5 L5 15 days: mass drift={:.2e} energy drift={:.3e} min h={:.1} max |v|={:.2}",
        r.mass_drift, r.energy_drift, r.min_h, r.max_speed
    );
    assert!(r.daily.iter().all(|(_, d)| d.finite));
    assert!(r.mass_drift.abs() < 1e-12);
    assert!(r.energy_drift.abs() < 1e-4);
    assert!(r.min_h > 3000.0 && r.max_speed < 80.0);
}

/// Short single-layer Sandbox run on the real relief.
#[test]
fn sandbox_short_conservation() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = sandbox_cfg();
    cfg.days = 5.0;
    cfg.every_hours = 25.0;
    let r = with_threads(4, || cases::run_shallow(&cfg)).expect("sandbox run");
    println!(
        "Sandbox 5 days: mass drift={:.2e} dust drift={:.2e} min h={:.0} min dust={:.2e}",
        r.mass_drift, r.tracer_drift, r.min_h, r.min_tracer
    );
    assert!(r.mass_drift.abs() < 1e-12 && r.tracer_drift.abs() < 1e-12);
    assert!(r.min_h > 0.2 * cfg.h_mean);
    assert!(r.min_tracer >= -1e-300);
}

/// Same steps with 1 and 4 threads must give bit-identical states (fixed summation order).
#[test]
fn thread_count_does_not_change_results() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = sandbox_cfg();
    let run = |th: usize| -> State {
        with_threads(th, || {
            let (model, mut st, ..) = cases::shallow_setup(&cfg).unwrap();
            let mut w = model.new_work();
            for _ in 0..50 {
                model.step(&mut st, 400.0, &mut w);
            }
            st
        })
    };
    let (a, b) = (run(1), run(4));
    assert!(a.h.iter().zip(&b.h).all(|(x, y)| x.to_bits() == y.to_bits()));
    assert!(a.m.iter().zip(&b.m).all(|(x, y)| x.to_bits() == y.to_bits()));
    assert!(a.v.iter().zip(&b.v).all(|(x, y)| x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits())));
}

/// A table written to PNG decodes back within one quantisation step.
#[test]
fn png_table_roundtrip() {
    let dir = std::env::temp_dir().join("weather_atmosphere_png_test");
    std::fs::create_dir_all(&dir).unwrap();
    let (w, h) = (64usize, 32usize);
    let data: Vec<f32> = (0..w * h).map(|k| ((k as f32) * 0.37).sin() * 25.0).collect();
    let meta = weather_atmosphere::io::TableMeta {
        field: "u",
        units: "m/s",
        description: "test",
        time_s: 0.0,
        time_hours: 0.0,
        planet_day_s: 90000.0,
        grid_level: 0,
        grid_cells: 12,
        radius_m: 1.0,
        fixed_range: None,
    };
    let p = dir.join("t_u.png");
    let (lo, hi) = weather_atmosphere::io::write_table(&p, w, h, &data, &meta).unwrap();
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&p).unwrap()));
    let mut rd = dec.read_info().unwrap();
    let mut buf = vec![0; rd.output_buffer_size()];
    let info = rd.next_frame(&mut buf).unwrap();
    assert_eq!(info.bit_depth, png::BitDepth::Sixteen);
    let step = (hi - lo) / 65535.0;
    for k in 0..w * h {
        let q = u16::from_be_bytes([buf[2 * k], buf[2 * k + 1]]) as f64;
        let v = lo + q * step;
        assert!((v - data[k] as f64).abs() <= 0.51 * step + 1e-6);
    }
    let json = std::fs::read_to_string(p.with_extension("json")).unwrap();
    assert!(json.contains("\"min\"") && json.contains("\"max\""));
}
