//! Shared helpers of the integration tests: where the shared data and the settings files live in
//! the `services` repository, and one lock so that at most one 4-thread pool works at a time.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use weather_atmosphere::body::DataFiles;
use weather_atmosphere::setup::Scenario;

static LOCK: Mutex<()> = Mutex::new(());

/// Heavy tests take this lock (never more than one 4-thread pool at a time).
pub fn heavy() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The crate's folder.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The weather service's folder (`services/weather`).
pub fn weather_dir() -> PathBuf {
    root().join("../..")
}

/// The service's settings folder of the atmosphere model.
pub fn settings_dir() -> PathBuf {
    weather_dir().join("data/atmosphere")
}

/// The shared system file (resourcesDynamic) and the weather service's climate extras.
pub fn files() -> DataFiles {
    DataFiles { system: weather_dir().join("../resourcesDynamic/data/system/tarsis.json"), extras: Some(weather_dir().join("data/climate/tarsis.json")) }
}

/// A settings file of `tests/data/` (the prototype's exact step-4 / step-3b Sandbox).
pub fn test_scenario(file: &str) -> Scenario {
    Scenario::load_with_data(&root().join("tests/data").join(file), &[], None, &files()).unwrap()
}
