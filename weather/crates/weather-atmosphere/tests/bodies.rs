//! Bodies built from the shared system data, the airless refusal, the absence of Sandbox constants
//! in the code, the level rule, the settings files (service files and test files), and the
//! regression of the code against fingerprints of the prototype's step-4 code. Heavy tests share
//! one lock.

mod common;

use common::{files, root, settings_dir, test_scenario};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use weather_atmosphere::body::{Body, BodyOverrides};
use weather_atmosphere::setup::{Init, Scenario};
use weather_atmosphere::with_threads;

static LOCK: Mutex<()> = Mutex::new(());

fn body(name: &str) -> Result<Body, String> {
    Body::load(&files(), name, &BodyOverrides::default())
}

/// Sandbox from the data: the step-4 hand values come out of the system file through
/// weather-core (gravity from mass and radius, air from the gas mixture).
#[test]
fn body_built_from_the_data() {
    let b = body("Sandbox").unwrap();
    println!("{}", b.describe());
    assert!((b.gravity - 7.7747).abs() < 1e-5, "gravity {}", b.gravity);
    assert!((b.air.molar_mass * 1e3 - 28.98).abs() < 0.01, "molar mass {}", b.air.molar_mass);
    assert!((b.air.cp - 1005.68).abs() < 0.01, "cp {}", b.air.cp);
    assert_eq!(b.radius_m, 6.356e6);
    assert_eq!(b.rotation_period_s, 90000.0);
    assert_eq!(b.p_surface, 1.02e5);
    assert_eq!(b.t_surface_k, 283.0);
    assert_eq!(b.thermal_inertia_tiu, 120.0);
    assert!((b.year_s - 14_777_126.0).abs() < 1.0, "year {}", b.year_s);
    // Solar day: 1 / (1/90000 - 1/year) = 90 551.6 s, longer than the sidereal day.
    assert!((b.solar_day_s - 90_551.55).abs() < 0.1, "solar day {}", b.solar_day_s);
    assert_eq!(b.day_s, b.solar_day_s);
    // A moon's year is its planet's (the seasons follow the star).
    let m = body("P5_M2").unwrap();
    let p = body("Tarsis 5").unwrap();
    assert_eq!(m.year_s, p.year_s);
    assert!(m.solar_day_s > m.rotation_period_s);
    // A planet locked to its star has no solar day: the model counts in sidereal rotations.
    let t1 = body("Tarsis 1").unwrap();
    assert!(t1.solar_day_s.is_infinite());
    assert_eq!(t1.day_s, t1.rotation_period_s);
    // Name matching ignores case (the data writes "Sandbox", the celestial service "SandBox").
    assert_eq!(body("sandbox").unwrap().name, "Sandbox");
}

/// A body without an atmosphere is refused with a message that says why; an unknown name too.
#[test]
fn airless_body_is_refused() {
    let e = body("P3_M1").unwrap_err();
    println!("{e}");
    assert!(e.contains("no atmosphere"), "{e}");
    let e = body("Nowhere").unwrap_err();
    assert!(e.contains("no body called"), "{e}");
    let e = Scenario::for_body(&settings_dir(), "P5_M5", &[], &files()).unwrap_err();
    assert!(e.contains("no atmosphere"), "{e}");
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// No Sandbox constant left in the model's code: they live in the data and in the settings files.
#[test]
fn no_sandbox_literal_in_the_code() {
    let needles = ["7.7747", "6.356e6", "6_356", "6356000", "90000", "90_000", "25.0 * 3600", "1005.68", "28.978", "14_777_126", "14777126", "2_566_519"];
    let mut files = Vec::new();
    rust_files(&root().join("src"), &mut files);
    rust_files(&root().join("examples"), &mut files);
    let mut hits = Vec::new();
    for f in &files {
        let txt = std::fs::read_to_string(f).unwrap();
        for (n, line) in txt.lines().enumerate() {
            for nd in needles {
                if line.contains(nd) {
                    hits.push(format!("{}:{}: `{nd}` in: {}", f.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(files.len() >= 15, "source files not found");
    assert!(hits.is_empty(), "Sandbox literals in the code:\n{}", hits.join("\n"));
}

/// The level rule lands near the 8 hand levels chosen for Sandbox at step 3b (within 0.03 in
/// eta), and the hybrid rule picks the same top of the terrain-following part (0.31) for both
/// hand-chosen level sets.
#[test]
fn level_rule_near_the_sandbox_levels() {
    let b = body("Sandbox").unwrap();
    let c = &b.climate;
    let hand8 = [0.0, 0.06, 0.13, 0.21, 0.31, 0.46, 0.66, 0.87, 1.0];
    let rule8 = weather_atmosphere::levels::rule_eta_half(8, c.scale_height_m, c.tropopause_m, 1.5, 1.5);
    println!("rule, 8 layers: {rule8:.3?}");
    for (a, h) in rule8.iter().zip(hand8) {
        assert!((a - h).abs() < 0.03, "rule {a} vs hand {h}");
    }
    let target = (-0.6 * c.tropopause_m / c.scale_height_m).exp();
    assert_eq!(weather_atmosphere::levels::nearest_interior(&hand8, target), 0.31);
    let hand11 = [0.0, 0.06, 0.13, 0.21, 0.31, 0.46, 0.60, 0.705, 0.80, 0.90, 0.953, 1.0];
    assert_eq!(weather_atmosphere::levels::nearest_interior(&hand11, target), 0.31);
}

/// Settings: inheritance across folders (the test file inherits the service's `defaut.toml`),
/// overrides, unknown keys refused, paths resolved against the file's folder.
#[test]
fn settings_files() {
    let sc = test_scenario("sandbox_step4.toml");
    assert_eq!(sc.s.grid.layers, 11);
    assert_eq!(sc.body.gravity, 7.7747);
    assert_eq!(sc.chunk_s(), 3750.0);
    assert!(sc.s.relief.file.as_ref().unwrap().exists(), "relief path not resolved: {:?}", sc.s.relief.file);
    let p = root().join("tests/data/sandbox_step4.toml");
    let sc2 = Scenario::load_with_data(&p, &[("lid.enabled".into(), "false".into()), ("grid.level".into(), "3".into())], None, &files()).unwrap();
    assert!(!sc2.s.lid.enabled && sc2.s.grid.level == 3);
    let e = Scenario::load_with_data(&settings_dir().join("sandbox.toml"), &[("ground.albdo".into(), "0.5".into())], None, &files()).unwrap_err();
    assert!(e.contains("albdo"), "{e}");
    // Without data files the offline loader says so (the service passes its own body instead).
    let e = Scenario::load(&settings_dir().join("sandbox.toml"), &[], None).unwrap_err();
    assert!(e.contains("[data]"), "{e}");
}

/// The service's Sandbox settings carry the worldbuilding decision of 2026-11-25: the ground and
/// dust albedo map, the air emissivity by the data's rule, the strict dust-sea top; and every
/// body with an air builds (level 2) from the service's defaults alone and passes the guard.
#[test]
fn service_settings_build_every_body() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = settings_dir();
    let sc = Scenario::for_body(&dir, "Sandbox", &[], &files()).unwrap();
    assert_eq!(sc.s.grid.layers, 11);
    assert!(sc.s.ground.albedo_map.as_ref().unwrap().exists());
    assert!(sc.s.dust.absorption_albedo_map.as_ref().unwrap().exists());
    assert!(sc.s.ground.eps_air.is_none() && sc.s.ground.albedo.is_none());
    assert!((sc.eps_air() - (1.0 - (-sc.body.climate.tau_ir).exp())).abs() < 1e-15);
    assert!(sc.s.sea.enabled && sc.s.sea.z_base_m == 500.0 && sc.s.sea.z_top_m == 3500.0 && sc.s.sea.clear_days == 0.25);
    println!("{}", sc.summary());
    for (name, air) in weather_atmosphere::body::list_bodies(&files()).unwrap() {
        if !air {
            continue;
        }
        let mut sc = Scenario::for_body(&dir, &name, &[], &files()).unwrap();
        sc.s.grid.level = 2;
        let s = with_threads(2, || sc.build(Init::Equilibrium)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let names = s.model.phys.as_ref().map(|p| p.names()).unwrap_or_default();
        println!("{name}: {} layers, eta_t {:.3}, chunk {:.0} s, components {names:?}", s.model.nl, s.model.eta_t, sc.chunk_s());
        assert!(s.model.check(&s.state).is_ok(), "{name}: initial state refused by the guard");
    }
}

/// Regression: Sandbox with the prototype's exact step-4 / step-3b settings gives, after one day
/// on the level-4 grid, a restart file whose FNV-1a fingerprint equals the one the prototype's
/// step-4 code wrote (before its refactor into components, then its move into this crate): bit
/// for bit, for the step-4 model (11 layers + column physics) and the step-3b model (8 layers,
/// Held-Suarez only). A change here means the model's numbers changed: on purpose (then update
/// the fingerprints and say why in the commit) or not (a bug).
///
/// The fingerprints are those of the x86_64-pc-windows-msvc toolchain, where the prototype ran:
/// sin, exp, ln, powf... come from the platform's math library, whose last bits differ between
/// platforms (measured: the windows-gnu toolchain already gives other fingerprints). Elsewhere
/// (Linux CI, GNU toolchain) the test is ignored; run it with the MSVC toolchain:
/// `cargo +stable-x86_64-pc-windows-msvc test -p weather-atmosphere --test bodies regression`.
#[test]
#[cfg_attr(not(all(windows, target_env = "msvc")), ignore = "fingerprints of the x86_64-pc-windows-msvc math library")]
fn regression_against_the_step4_code() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let expect = [("sandbox_step4.toml", 0xc48d_6028_b9ff_884f_u64), ("sandbox_step3b.toml", 0xcd35_f845_027e_80ee_u64)];
    for (file, want) in expect {
        let mut sc = test_scenario(file);
        sc.s.grid.level = 4;
        let got = with_threads(4, || {
            let s = sc.build(Init::Equilibrium).unwrap();
            let (model, mut st) = (s.model, s.state);
            let mut w = model.new_work();
            weather_atmosphere::run::run_chunks(&model, &mut st, sc.chunks_per_day(), sc.chunk_s(), sc.s.numerics.courant, &mut w).unwrap();
            let p = std::env::temp_dir().join(format!("weather_atmosphere_regression_{file}.bin"));
            model.write_restart(&st, &p).unwrap();
            weather_atmosphere::fnv1a(&std::fs::read(&p).unwrap())
        });
        println!("{file}: {got:#018x} (step 4: {want:#018x})");
        assert_eq!(got, want, "{file}: not bit-identical to the step-4 code");
    }
}
