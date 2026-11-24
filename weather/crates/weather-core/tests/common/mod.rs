//! What `data/derived/<system>.json` and `data/golden/<system>_sandbox_samples.json` must hold, built from the
//! system data of resourcesDynamic and the climate extras: one builder for the export (export_derived.rs) and
//! for the check that the committed files are still what the data gives (derived_up_to_date.rs).

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use weather_core::bodies::load;
use weather_core::climate::derive;
use weather_core::sampler::sample;
use weather_core::spin::{declination_deg, local_solar_hours, substellar_point};
use weather_core::synoptic::systems_at;

/// The golden instant and places on SandBox: the game's sampler must return the same.
pub const GOLDEN_T: f64 = 1_791_313_357.0;
pub const GOLDEN_POINTS: [(f64, f64, f64); 5] =
    [(-39.5, 24.7, 380.0), (0.0, 0.0, 0.0), (120.0, -45.0, 2000.0), (-170.0, 70.0, 0.0), (45.0, 10.0, 5000.0)];

/// The weather crate's root (`services/weather`).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The system file and the climate extras, `None` when the system file is not beside this checkout.
pub fn inputs() -> Option<(String, Option<String>)> {
    let root = root();
    let system = std::fs::read_to_string(root.join("../resourcesDynamic/data/system/tarsis.json")).ok()?;
    let extras = std::fs::read_to_string(root.join("data/climate/tarsis.json")).ok();
    Some((system, extras))
}

/// `data/derived/tarsis.json` as the data gives it.
pub fn derived(system: &str, extras: Option<&str>) -> Value {
    let bodies = load(system, extras).expect("load");
    let derived: Vec<_> = bodies.iter().map(derive).collect();
    json!({ "system": "tarsis", "generated_by": "weather-core tests/export_derived.rs", "bodies": derived })
}

/// `data/golden/tarsis_sandbox_samples.json` as the data gives it.
pub fn golden(system: &str, extras: Option<&str>) -> Value {
    let bodies = load(system, extras).expect("load");
    let sandbox = bodies.iter().find(|b| b.name.eq_ignore_ascii_case("sandbox")).expect("SandBox");
    let c = derive(sandbox);
    let to_star = sandbox.orbit.solver(sandbox.mass_kg).position_at(GOLDEN_T).scale(-1.0);
    let decl = declination_deg(sandbox.tilt_deg.to_radians(), to_star);
    let (sub_lon, _) = substellar_point(sandbox.tilt_deg.to_radians(), sandbox.rotation_period_s, GOLDEN_T, to_star);
    let systems = systems_at(&c, GOLDEN_T, decl);
    let samples: Vec<_> = GOLDEN_POINTS
        .iter()
        .map(|&(lon, lat, alt)| {
            let s = sample(&c, &systems, lon, lat, alt, local_solar_hours(lon, sub_lon), decl);
            json!({ "lon": lon, "lat": lat, "alt": alt, "sample": s })
        })
        .collect();
    json!({ "body": "SandBox", "t": GOLDEN_T, "declination_deg": decl, "substellar_lon_deg": sub_lon, "systems": systems, "samples": samples })
}

/// The first place where [expected] and [found] differ beyond a relative tolerance on numbers (the last digits
/// move from one toolchain to another), as a JSON path and the two values; `None` when they agree.
pub fn first_difference(expected: &Value, found: &Value, path: &str) -> Option<String> {
    match (expected, found) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap_or(f64::NAN), b.as_f64().unwrap_or(f64::NAN));
            let scale = a.abs().max(b.abs()).max(1e-12);
            ((a - b).abs() / scale > 1e-9).then(|| format!("{path}: {b} committed, {a} from the data"))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: {} entries committed, {} from the data", b.len(), a.len()));
            }
            a.iter().zip(b).enumerate().find_map(|(i, (x, y))| first_difference(x, y, &format!("{path}[{i}]")))
        }
        (Value::Object(a), Value::Object(b)) => {
            if let Some(key) = a.keys().find(|k| !b.contains_key(*k)).or_else(|| b.keys().find(|k| !a.contains_key(*k))) {
                return Some(format!("{path}.{key}: present on one side only"));
            }
            a.iter().find_map(|(k, x)| first_difference(x, &b[k], &format!("{path}.{k}")))
        }
        _ => (expected != found).then(|| format!("{path}: {found} committed, {expected} from the data")),
    }
}
