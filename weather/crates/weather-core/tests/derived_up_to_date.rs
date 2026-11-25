//! The committed derived climate and golden samples are what the system data and the model give today.
//!
//! They are copied into the game (its ClimateProfile resources and its sampler test), so a change of
//! resourcesDynamic/data/system/tarsis.json or of the model that is not exported again leaves the game on an old
//! climate without a word: services #54 changed Sandbox's albedo and nothing followed for a month. This test is
//! what says so, in the pull request that makes the change.

mod common;

const HOW: &str = "export them again: in services/weather, `WEATHER_EXPORT=1 cargo test -p weather-core --test export_derived`, \
then follow docs/WEATHER.md of the game to carry them over (climate profiles, golden samples)";

fn check(file: &str, expected: serde_json::Value) {
    let path = common::root().join(file);
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"))).expect(file);
    if let Some(difference) = common::first_difference(&expected, &committed, "") {
        panic!("{file} is behind the system data or the model ({difference}); {HOW}");
    }
}

#[test]
fn the_committed_derived_climate_is_what_the_data_gives() {
    let Some((system, extras)) = common::inputs() else {
        eprintln!("tarsis.json not found beside this checkout: skipping");
        return;
    };
    check("data/derived/tarsis.json", common::derived(&system, extras.as_deref()));
}

#[test]
fn the_committed_golden_samples_are_what_the_data_gives() {
    let Some((system, extras)) = common::inputs() else {
        eprintln!("tarsis.json not found beside this checkout: skipping");
        return;
    };
    check("data/golden/tarsis_sandbox_samples.json", common::golden(&system, extras.as_deref()));
}

#[test]
fn a_moved_number_is_named_and_toolchain_digits_are_not() {
    let a = serde_json::json!({ "bodies": [{ "name": "Sandbox", "t_eq_k": 217.41992521656323 }] });
    let same = serde_json::json!({ "bodies": [{ "name": "Sandbox", "t_eq_k": 217.41992521656326 }] });
    let moved = serde_json::json!({ "bodies": [{ "name": "Sandbox", "t_eq_k": 258.5573 }] });
    assert_eq!(common::first_difference(&a, &same, ""), None, "the last digits of another toolchain");
    let found = common::first_difference(&a, &moved, "").expect("a moved value");
    assert!(found.contains(".bodies[0].t_eq_k"), "{found}");
}
