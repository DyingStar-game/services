//! Planetary climate and weather model of DyingStar.
//!
//! Everything here is a PURE FUNCTION of its inputs: a body's physical parameters, a seed and an
//! absolute time (unix seconds). Nothing keeps state between calls, so two instances of the service,
//! or the service and the game's own twin (`DyingStar-game/scenes/planet/climate_model.gd`), land on
//! the same numbers — the design already used for rotation and orbits (game ADR 0009).
//!
//! Conventions shared with the game: the reference plane of an orbit is XZ with +Y as its north,
//! orbits run counter-clockwise seen from +Y (the sense bodies spin in), a body-fixed direction has
//! longitude `atan2(z, x)` and latitude `asin(y)` (HEALPix.vec2lonlat), and an axial tilt tips the
//! spin axis about +Z (`Planet.spin_basis_at`).

pub mod bodies;
pub mod climate;
pub mod constants;
pub mod gases;
pub mod hash;
pub mod math;
pub mod orbit;
pub mod sampler;
pub mod spin;
pub mod synoptic;
