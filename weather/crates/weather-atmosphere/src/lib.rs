//! Atmosphere model of the DyingStar weather service, for any body of the system data that has
//! an air (it grew out of the offline prototype of the weather R&D, steps 2 to 5):
//!
//! - a single-layer rotating shallow-water model (`model`, `cases`), kept for the standard
//!   Williamson test cases of the grid and the transport;
//! - dry primitive equations in a hybrid sigma-pressure coordinate (`pe`, the dynamical core) with a
//!   swappable temperature relaxation (`relax`, Held-Suarez with seasons);
//! - column physics as swappable components (`phys`): ground and day-night, dust, dust sea, lid,
//!   dry convection, game events;
//! - bodies from the shared system data through `weather-core` (`body`), per-body settings files
//!   (`settings`), vertical levels by rule (`levels`), the composition root (`setup`), offline runs
//!   (`run`), outputs as 16-bit PNG tables (`io`, `outputs`);
//! - the pieces a long-running service needs (`live`): a model kept in memory, advanced chunk by
//!   chunk under the roll-back guard, rolling checkpoints, tables rendered in memory.
//!
//! Unlike the rest of the weather service, this model is NOT a pure function of (seed, time): the
//! atmosphere is chaotic, so its state lives in memory and in checkpoint files.

pub mod body;
pub mod cases;
pub mod grid;
pub mod io;
pub mod levels;
pub mod live;
pub mod model;
pub mod outputs;
pub mod pe;
pub mod phys;
pub mod relax;
pub mod run;
pub mod settings;
pub mod setup;
pub mod vec3;

/// Runs `f` inside a dedicated rayon pool of `threads` threads (never more than 4).
pub fn with_threads<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    let n = threads.clamp(1, 4);
    rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build()
        .expect("thread pool")
        .install(f)
}

/// FNV-1a 64 of a byte string (fingerprints of restart files in the regression tests).
pub fn fnv1a(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
