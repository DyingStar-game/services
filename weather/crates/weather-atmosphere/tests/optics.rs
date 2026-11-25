//! Tests of the dust optics of the Mie option: the Mie solver against published reference values,
//! the colour -> imaginary index inversion (Hapke's equivalent slab), the delta-Eddington
//! two-stream (limits, energy, a closed form), and the Sandbox settings with the Mie optics.

mod common;

use weather_atmosphere::phys::dust::DustRadiation;
use weather_atmosphere::phys::mie::{lognormal_optics, mie, Cx};
use weather_atmosphere::phys::optics::{delta_eddington, k_from_reflectance, reflectance_from_w, slab_w, w_from_reflectance, OpticsTable};
use weather_atmosphere::phys::{di, ground::Ground};
use weather_atmosphere::run;
use weather_atmosphere::settings::DustOptics;
use weather_atmosphere::setup::{Init, Scenario};
use weather_atmosphere::with_threads;

fn close(name: &str, got: f64, want: f64, tol: f64) {
    assert!((got - want).abs() <= tol, "{name}: got {got:.9}, want {want:.9} (tolerance {tol:e})");
}

/// Reference values of Wiscombe's MIEV0 test cases (NCAR technical note TN-140, 1979) and of
/// Bohren & Huffman's worked example (m = 1.55, r = 0.525 um, lambda = 0.6328 um). Indices are
/// written n + i k with k >= 0 absorbing. Columns: x, n, k, Qext (NaN = not given), Qsca, g
/// (NaN = not given), tolerance.
fn reference() -> [(f64, f64, f64, f64, f64, f64, f64); 14] {
    let xbh = 2.0 * std::f64::consts::PI * 0.525 / 0.6328;
    [
    (xbh, 1.55, 0.0, 3.10543, 3.10543, 0.63314, 1e-5),
    (xbh, 1.55, 0.1, 2.86165188243, 1.66424911991, 0.80128972639, 1e-7),
    (10.0, 1.5, 0.0, f64::NAN, 2.8820, f64::NAN, 1e-4),
    (1.0, 1.33, 1e-5, f64::NAN, 0.093923, 0.184517, 1e-6),
    (100.0, 1.33, 1e-5, f64::NAN, 2.096594, 0.868959, 1e-6),
    (10000.0, 1.33, 1e-5, f64::NAN, 1.723857, 0.907840, 1e-6),
    (0.055, 1.5, 1.0, 0.101491, 0.000011, 0.000491, 1e-6),
    (0.056, 1.5, 1.0, 0.1033467, 0.000012, 0.000509, 1e-6),
    (1.0, 1.5, 1.0, 2.336321, 0.6634538, 0.192136, 1e-6),
    (100.0, 1.5, 1.0, 2.097502, 1.283697, 0.850252, 1e-3),
    (10000.0, 1.5, 1.0, 2.004368, 1.236575, 0.846309, 1e-6),
    (1.0, 10.0, 10.0, f64::NAN, 2.049405, -0.110664, 1e-6),
    (100.0, 10.0, 10.0, f64::NAN, 1.836785, 0.556215, 1e-6),
    (0.099, 0.75, 0.0, f64::NAN, 0.000007, 0.001448, 1e-6),
    ]
}

#[test]
fn mie_matches_published_reference_values() {
    for (x, n, k, qext, qsca, g, tol) in reference() {
        let r = mie(x, Cx::new(n, k));
        println!("x {x}, m {n}+{k}i: Qext {:.7}, Qsca {:.7}, g {:.7}, Qback {:.5}", r.qext, r.qsca, r.g, r.qback);
        if qext.is_finite() {
            close(&format!("Qext x={x} m={n}+{k}i"), r.qext, qext, tol);
        }
        close(&format!("Qsca x={x} m={n}+{k}i"), r.qsca, qsca, tol);
        if g.is_finite() {
            close(&format!("g x={x} m={n}+{k}i"), r.g, g, tol);
        }
        if k == 0.0 {
            close("no absorption: Qext = Qsca", r.qext, r.qsca, 1e-9 * r.qext.max(1e-6));
        }
    }
    // Bohren & Huffman's example also gives the backscattering efficiency.
    close("Qback B&H", mie(2.0 * std::f64::consts::PI * 0.525 / 0.6328, Cx::new(1.55, 0.0)).qback, 2.92534, 1e-5);
}

/// Small spheres: Rayleigh's limits Qsca = 8/3 x^4 |L|^2, Qabs = 4 x Im(L), L = (m^2-1)/(m^2+2).
#[test]
fn mie_rayleigh_limit() {
    let m = Cx::new(1.77, 0.01);
    let x = 1e-3;
    let m2 = m * m;
    let l = (m2 - Cx::new(1.0, 0.0)) / (m2 + Cx::new(2.0, 0.0));
    let r = mie(x, m);
    close("Rayleigh Qsca (relative)", r.qsca / (8.0 / 3.0 * x.powi(4) * l.norm2()), 1.0, 1e-5);
    close("Rayleigh Qabs (relative)", (r.qext - r.qsca) / (4.0 * x * l.im), 1.0, 1e-5);
    close("Rayleigh g", r.g, 0.0, 1e-5);
}

/// Averaging over a narrow lognormal tends to the single sphere; a wider one smooths the ripples
/// but keeps Qext near 2 for large grains and omega within (0, 1].
#[test]
fn lognormal_average() {
    let m = Cx::new(1.77, 1e-4);
    let one = mie(std::f64::consts::PI * 1.5e-6 / 0.55e-6, m);
    let narrow = lognormal_optics(1.5e-6, 1.0001, 0.55e-6, m, 48);
    close("narrow lognormal Qext", narrow.qext, one.qext, 2e-3);
    close("narrow lognormal g", narrow.g, one.g, 2e-3);
    let wide = lognormal_optics(10e-6, 1.5, 0.55e-6, m, 48);
    assert!(wide.qext > 1.9 && wide.qext < 2.4, "coarse Qext {}", wide.qext);
    assert!(wide.omega > 0.0 && wide.omega <= 1.0 && wide.g > 0.5 && wide.g < 1.0, "coarse optics {wide:?}");
}

/// Hapke: r0 <-> w round trip, and colour -> k -> grain albedo -> colour round trip.
#[test]
fn hapke_inversion_round_trip() {
    for w in [0.0, 0.1, 0.5, 0.9, 0.99, 0.9999] {
        close("w -> r0 -> w", w_from_reflectance(reflectance_from_w(w)), w, 1e-12);
    }
    let (n, lambda) = (1.77, 0.55e-6);
    for d in [50e-6, 100e-6, 300e-6] {
        for r0 in [0.15, 0.27, 0.4, 0.6, 0.84] {
            let (k, floored) = k_from_reflectance(r0, n, d, lambda);
            assert!(!floored && k > 0.0, "r0 {r0} D {d}: k {k}, floored {floored}");
            let back = reflectance_from_w(slab_w(n, k, d, lambda));
            println!("D {:.0} um, r0 {r0}: k {k:.3e}, back {back:.6}", d * 1e6);
            // Se is evaluated at k = 0 in the inversion: exact to O(k^2).
            close("colour -> k -> colour", back, r0, 1e-6);
        }
        // Larger grains for the same colour: a less absorbing material.
        let (k_small, _) = k_from_reflectance(0.27, n, d, lambda);
        let (k_big, _) = k_from_reflectance(0.27, n, 3.0 * d, lambda);
        close("k scales as 1/D", k_small / k_big, 3.0, 1e-9);
    }
    // A colour darker than the bare surface reflection of the grains floors (opaque grain).
    assert!(k_from_reflectance(0.01, n, 100e-6, lambda).1);
}

/// Two-stream limits and energy.
#[test]
fn two_stream_limits_and_energy() {
    // No dust: the ground alone.
    for ag in [0.0, 0.3, 0.8] {
        let t = delta_eddington(0.0, 0.9, 0.7, 0.6, ag);
        close("tau 0: reflection = ground albedo", t.reflect, ag, 1e-12);
        close("tau 0: ground absorbs 1 - albedo", t.ground, 1.0 - ag, 1e-12);
        close("tau 0: layer absorbs nothing", t.layer, 0.0, 1e-12);
    }
    // White, isotropic, thick: bright, (almost) nothing absorbed by the layer. omega is capped at
    // 1 - 1e-6 and a photon scatters ~tau^2 times in a thick conservative layer: at tau = 200 the
    // layer still absorbs a few 1e-4.
    let t = delta_eddington(200.0, 1.0, 0.0, 0.5, 0.0);
    assert!(t.reflect > 0.98, "thick white layer reflects {}", t.reflect);
    close("omega 1: layer absorbs nothing", t.layer, 0.0, 1e-3);
    // Energy: reflected + layer + ground = incoming, every term in [0, 1], over a wide range.
    for tau in [0.01, 0.1, 0.3, 1.0, 3.0, 30.0] {
        for omega in [0.0, 0.3, 0.8, 0.95, 0.999, 1.0] {
            for g in [0.0, 0.5, 0.85] {
                for mu0 in [0.05, 0.3, 0.7, 1.0] {
                    for ag in [0.0, 0.27, 0.84] {
                        let t = delta_eddington(tau, omega, g, mu0, ag);
                        let sum = t.reflect + t.layer + t.ground;
                        let ctx = format!("tau {tau} omega {omega} g {g} mu0 {mu0} ag {ag}: {t:?}");
                        assert!((sum - 1.0).abs() < 1e-9, "energy {sum}: {ctx}");
                        assert!(t.reflect >= -1e-9 && t.reflect <= 1.0 + 1e-9 && t.layer >= -1e-4 && t.ground >= -1e-9, "{ctx}");
                        if omega == 1.0 {
                            // omega capped at 1 - 1e-6: a few 1e-4 at most in thick layers over bright ground.
                            assert!(t.layer.abs() < 1e-3, "conservative layer absorbs: {ctx}");
                        }
                        if omega == 0.0 {
                            // Pure absorber: only the direct beam crosses it, both ways.
                            let tr = (-tau / mu0).exp();
                            assert!(t.reflect <= ag * tr + 1e-9, "{ctx}");
                        }
                    }
                }
            }
        }
    }
    // Conservative Eddington layer over a black ground, closed form (Meador & Weaver 1980):
    // R = (g1 tau + (g3 - g1 mu0)(1 - e^{-tau/mu0})) / (1 + g1 tau), here with g = 0 (no delta
    // scaling): g1 = 3/4, g3 = 1/2.
    for tau in [0.1f64, 1.0, 5.0] {
        for mu0 in [0.3f64, 0.8] {
            let r = (0.75 * tau + (0.5 - 0.75 * mu0) * (1.0 - (-tau / mu0).exp())) / (1.0 + 0.75 * tau);
            close(&format!("conservative closed form tau {tau} mu0 {mu0}"), delta_eddington(tau, 1.0, 0.0, mu0, 0.0).reflect, r, 1e-4);
        }
    }
    // Thin layer, black ground: single scattering, a fraction g3 of the scattered light goes up.
    let (tau, mu0) = (1e-5, 0.5);
    close("single scattering", delta_eddington(tau, 1.0, 0.0, mu0, 0.0).reflect / (tau / mu0), 0.5, 1e-4);
    // White dust over a dark ground brightens the planet; dark dust over a bright ground darkens it.
    assert!(delta_eddington(0.3, 0.98, 0.7, 0.6, 0.2).reflect > 0.2);
    assert!(delta_eddington(0.3, 0.5, 0.7, 0.6, 0.8).reflect < 0.8);
}

#[test]
fn optics_table_interpolates_the_mie_values() {
    let d = [1.5e-6, 10e-6];
    let t = OpticsTable::new(d, 1.77, 1.5, 0.55e-6, 20);
    for k in [3e-6, 7e-5, 2e-3] {
        for c in 0..2 {
            let (q, w, g) = t.at(c, k);
            let e = lognormal_optics(d[c], 1.5, 0.55e-6, Cx::new(1.77, k), 48);
            close("table Qext", q, e.qext, 2e-3);
            close("table omega", w, e.omega, 2e-3 * (1.0 - e.omega).max(0.01));
            close("table g", g, e.g, 2e-3);
        }
    }
}

/// The Sandbox settings use the Mie optics; the default settings keep the colour optics (the
/// step-4 regression and every other test run the unchanged colour path). With the Mie optics, a
/// physics step gives every lit column sunlight shares that add up: absorbed by the ground and by
/// the dust, the rest reflected, none negative.
#[test]
fn sandbox_uses_mie_and_fluxes_add_up() {
    let _g = common::heavy();
    let step4 = common::test_scenario("sandbox_step4.toml");
    assert_eq!(step4.s.dust.optics, DustOptics::Colour);
    let mut sc = Scenario::for_body(&common::settings_dir(), "Sandbox", &[], &common::files()).unwrap();
    assert_eq!(sc.s.dust.optics, DustOptics::Mie);
    sc.s.grid.level = 4;
    with_threads(4, || {
        let s = sc.build(Init::Equilibrium).unwrap();
        let (model, mut st) = (s.model, s.state);
        let mut w = model.new_work();
        let ph = model.phys.as_ref().unwrap();
        let rad = ph.find::<DustRadiation>().unwrap();
        let o = rad.mie.as_ref().expect("Mie optics in the Sandbox settings");
        assert!(ph.find::<Ground>().is_some());
        for c in 0..2 {
            assert!(o.qext[c] > 1.5 && o.qext[c] < 3.5, "Qext {:?}", o.qext);
            assert!(o.omega[c].iter().all(|&x| x > 0.0 && x <= 1.0) && o.g[c].iter().all(|&x| x > 0.0 && x < 1.0));
        }
        run::run_chunks(&model, &mut st, 6, sc.chunk_s(), sc.s.numerics.courant, &mut w).unwrap();
        let mut lit = 0;
        for i in 0..model.grid.n_cells() {
            let r = &w.ph.diag[i * di::N..(i + 1) * di::N];
            if r[di::COSZ] > 0.0 {
                lit += 1;
                let toa = ph.sun.s0 * r[di::COSZ];
                let refl = toa - r[di::SWG] - r[di::SW_AIR];
                assert!(r[di::SWG] >= 0.0 && r[di::SW_AIR] >= 0.0 && refl >= -1e-9 * toa, "cell {i}: toa {toa}, ground {}, dust {}", r[di::SWG], r[di::SW_AIR]);
            }
        }
        assert!(lit > 0);
    });
}
