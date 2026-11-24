//! Exports what the game needs from the model, as data (no formula twin on the game side for the
//! climate derivation): `data/derived/<system>.json` with every body's derived climate, and
//! `data/golden/<system>_samples.json`, point samples the game's `WeatherSampler` must reproduce.
//!
//! Run with `WEATHER_EXPORT=1 cargo test -p weather-core --test export_derived`; the files are
//! committed, so a change in the model shows up as a diff. derived_up_to_date.rs fails while they lag
//! behind the system data or the model.

mod common;

#[test]
fn export_derived_climate_and_golden_samples() {
    if std::env::var("WEATHER_EXPORT").is_err() {
        eprintln!("WEATHER_EXPORT not set: skipping the export");
        return;
    }
    let root = common::root();
    let (system, extras) = common::inputs().expect("tarsis.json");

    // 1. Derived climate of every body.
    let derived = common::derived(&system, extras.as_deref());
    std::fs::create_dir_all(root.join("data/derived")).unwrap();
    std::fs::write(root.join("data/derived/tarsis.json"), serde_json::to_string_pretty(&derived).unwrap()).unwrap();

    // 2. Golden samples on SandBox at a fixed instant: the game's sampler must return the same.
    let golden = common::golden(&system, extras.as_deref());
    std::fs::create_dir_all(root.join("data/golden")).unwrap();
    std::fs::write(root.join("data/golden/tarsis_sandbox_samples.json"), serde_json::to_string_pretty(&golden).unwrap()).unwrap();
    eprintln!(
        "exported {} bodies and {} samples with {} systems",
        derived["bodies"].as_array().map_or(0, |b| b.len()),
        golden["samples"].as_array().map_or(0, |s| s.len()),
        golden["systems"].as_array().map_or(0, |s| s.len())
    );
}
