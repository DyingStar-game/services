//! Physical constants. The values the celestial service imports with (`import-system.ts`) are kept
//! identical so an orbit computed here is the one the game and Horizon see.

/// Gravitational constant, m³ kg⁻¹ s⁻².
pub const G: f64 = 6.6743e-11;
/// Stefan–Boltzmann constant, W m⁻² K⁻⁴.
pub const SIGMA: f64 = 5.670_374_419e-8;
/// Molar gas constant, J mol⁻¹ K⁻¹.
pub const R_GAS: f64 = 8.314_462_618;
/// Astronomical unit, metres (same as the celestial service and the game's KeplerOrbit).
pub const AU_M: f64 = 1.495_978_707e11;
/// Solar mass the celestial service multiplies `mass_Sun` by (import-system.ts MASS_SUN).
pub const SOLAR_MASS_KG: f64 = 1.989e30;
/// Earth mass the celestial service multiplies `mass_Me` by (import-system.ts MASS_EARTH).
pub const EARTH_MASS_KG: f64 = 5.972e24;
/// Solar luminosity, W.
pub const SOLAR_LUMINOSITY_W: f64 = 3.828e26;
/// Von Kármán constant of the logarithmic wind profile.
pub const KARMAN: f64 = 0.4;
/// Seconds in a day, for readouts.
pub const SECONDS_PER_DAY: f64 = 86_400.0;
