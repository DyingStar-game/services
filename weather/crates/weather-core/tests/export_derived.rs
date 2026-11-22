//! Exports what the game needs from the model, as data (no formula twin on the game side for the
//! climate derivation): `data/derived/<system>.json` with every body's derived climate, and
//! `data/golden/<system>_samples.json`, point samples the game's `WeatherSampler` must reproduce.
//!
//! Run with `WEATHER_EXPORT=1 cargo test -p weather-core --test export_derived`; the files are
//! committed, so a change in the model shows up as a diff.

use serde_json::json;
use std::path::Path;
use weather_core::bodies::load;
use weather_core::climate::derive;
use weather_core::sampler::sample;
use weather_core::spin::{declination_deg, local_solar_hours, substellar_point};
use weather_core::synoptic::systems_at;

#[test]
fn export_derived_climate_and_golden_samples() {
    if std::env::var("WEATHER_EXPORT").is_err() {
        eprintln!("WEATHER_EXPORT not set: skipping the export");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let system = std::fs::read_to_string(root.join("../resourcesDynamic/data/system/tarsis.json")).expect("tarsis.json");
    let extras = std::fs::read_to_string(root.join("data/climate/tarsis.json")).ok();
    let bodies = load(&system, extras.as_deref()).expect("load");

    // 1. Derived climate of every body.
    let derived: Vec<_> = bodies.iter().map(derive).collect();
    std::fs::create_dir_all(root.join("data/derived")).unwrap();
    std::fs::write(root.join("data/derived/tarsis.json"), serde_json::to_string_pretty(&json!({ "system": "tarsis", "generated_by": "weather-core tests/export_derived.rs", "bodies": derived })).unwrap()).unwrap();

    // 2. Golden samples on SandBox at a fixed instant: the game's sampler must return the same.
    let sandbox = bodies.iter().find(|b| b.name.eq_ignore_ascii_case("sandbox")).unwrap();
    let c = derive(sandbox);
    let t = 1_791_313_357.0;
    let to_star = sandbox.orbit.solver(sandbox.mass_kg).position_at(t).scale(-1.0);
    let decl = declination_deg(sandbox.tilt_deg.to_radians(), to_star);
    let (sub_lon, _) = substellar_point(sandbox.tilt_deg.to_radians(), sandbox.rotation_period_s, t, to_star);
    let systems = systems_at(&c, t, decl);
    let points = [(-39.5, 24.7, 380.0), (0.0, 0.0, 0.0), (120.0, -45.0, 2000.0), (-170.0, 70.0, 0.0), (45.0, 10.0, 5000.0)];
    let samples: Vec<_> = points
        .iter()
        .map(|&(lon, lat, alt)| {
            let s = sample(&c, &systems, lon, lat, alt, local_solar_hours(lon, sub_lon), decl);
            json!({ "lon": lon, "lat": lat, "alt": alt, "sample": s })
        })
        .collect();
    std::fs::create_dir_all(root.join("data/golden")).unwrap();
    std::fs::write(
        root.join("data/golden/tarsis_sandbox_samples.json"),
        serde_json::to_string_pretty(&json!({ "body": "SandBox", "t": t, "declination_deg": decl, "substellar_lon_deg": sub_lon, "systems": systems, "samples": samples })).unwrap(),
    )
    .unwrap();
    eprintln!("exported {} bodies and {} samples with {} systems", derived.len(), samples.len(), systems.len());
}
