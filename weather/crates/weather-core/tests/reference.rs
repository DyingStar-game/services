//! Reference checks of the climate derivation against published values (NASA fact sheets, the
//! textbooks cited in the R&D) for Earth and Mars, and against the game's own atmosphere profile
//! for Tarsis III. Tolerances say how far the simplified model is allowed to be.

use weather_core::climate::{band_wind, derive, surface_temperature_k, Band, BodyInputs, BodyKind, ClimateExtras, OrbitElements, StarInputs, Temperatures};
use weather_core::constants::{AU_M, EARTH_MASS_KG};

const SUN_W: f64 = 3.828e26;
const SUN_KG: f64 = 1.989e30;

fn earth() -> BodyInputs {
    BodyInputs {
        name: "Earth".into(),
        kind: BodyKind::Planet,
        radius_m: 6.371e6,
        mass_kg: EARTH_MASS_KG,
        rotation_period_s: 86_164.1,
        tidal_locked: false,
        tilt_deg: 23.44,
        orbit: OrbitElements { periapsis_au: 0.9833, apoapsis_au: 1.0167, inclination_deg: 0.0, ascending_node_deg: 0.0, arg_peri_deg: 0.0, mean_anomaly_deg: 0.0, primary_mass_kg: SUN_KG },
        star_distance_m: AU_M,
        star: StarInputs { luminosity_w: SUN_W, mass_kg: SUN_KG, temperature_k: 5772.0 },
        pressure_pa: 101_325.0,
        gases_pct: vec![("N2".into(), 78.08), ("O2".into(), 20.95), ("Ar".into(), 0.93), ("CO2".into(), 0.04), ("H2O".into(), 1.0)],
        bond_albedo: 0.294,
        temperatures: Some(Temperatures { day_k: 295.0, night_k: 281.0, surface_k: 288.0, eq_k: 255.0 }),
        extras: ClimateExtras { thermal_inertia_tiu: 2000.0, humidity_inventory: 0.7, ..Default::default() },
    }
}

fn mars() -> BodyInputs {
    BodyInputs {
        name: "Mars".into(),
        kind: BodyKind::Planet,
        radius_m: 3.3895e6,
        mass_kg: 6.4171e23,
        rotation_period_s: 88_642.7,
        tidal_locked: false,
        tilt_deg: 25.19,
        orbit: OrbitElements { periapsis_au: 1.3814, apoapsis_au: 1.6660, inclination_deg: 0.0, ascending_node_deg: 0.0, arg_peri_deg: 0.0, mean_anomaly_deg: 0.0, primary_mass_kg: SUN_KG },
        star_distance_m: 1.5237 * AU_M,
        star: StarInputs { luminosity_w: SUN_W, mass_kg: SUN_KG, temperature_k: 5772.0 },
        pressure_pa: 636.0,
        gases_pct: vec![("CO2".into(), 95.1), ("N2".into(), 2.6), ("Ar".into(), 1.9), ("O2".into(), 0.16)],
        bond_albedo: 0.25,
        temperatures: Some(Temperatures { day_k: 242.0, night_k: 184.0, surface_k: 210.0, eq_k: 210.0 }),
        extras: ClimateExtras { dust_grain_density_kg_m3: 3000.0, thermal_inertia_tiu: 250.0, dust_lift_bias: 0.9, ..Default::default() },
    }
}

/// Tarsis III (SandBox) exactly as tarsis.json and data/climate/tarsis.json describe it.
fn sandbox() -> BodyInputs {
    BodyInputs {
        name: "SandBox".into(),
        kind: BodyKind::Planet,
        radius_m: 6.356e6,
        mass_kg: 0.788 * EARTH_MASS_KG,
        rotation_period_s: 25.0 * 3600.0,
        tidal_locked: false,
        tilt_deg: 15.0,
        orbit: OrbitElements { periapsis_au: 0.5335, apoapsis_au: 0.5665, inclination_deg: 1.5, ascending_node_deg: 110.0, arg_peri_deg: 45.0, mean_anomaly_deg: 20.0, primary_mass_kg: 0.758581416228569 * SUN_KG },
        star_distance_m: 0.55 * AU_M,
        star: StarInputs { luminosity_w: 1.2675956075398006e26, mass_kg: 0.758581416228569 * SUN_KG, temperature_k: 4831.3 },
        pressure_pa: 102_000.0,
        gases_pct: vec![("Ar".into(), 1.1), ("CH4".into(), 0.2), ("CO2".into(), 2.0), ("N2".into(), 78.1), ("O2".into(), 18.6)],
        bond_albedo: 0.32,
        temperatures: Some(Temperatures { day_k: 290.0, night_k: 275.0, surface_k: 283.0, eq_k: 258.5 }),
        extras: ClimateExtras { weather_seed: Some(20261006), dust_grain_density_kg_m3: 4000.0, dust_lift_bias: 0.9, thermal_inertia_tiu: 120.0, wind_scale_max_ms: 45.0, dust_tau_max: 8.0, ..Default::default() },
    }
}

fn within(value: f64, target: f64, tol: f64, what: &str) {
    assert!((value - target).abs() <= tol, "{what}: {value} vs {target} ± {tol}");
}

#[test]
fn earth_reference() {
    let c = derive(&earth());
    within(c.surface_gravity, 9.82, 0.03, "g");
    within(c.t_eq_k, 255.0, 1.5, "T_eq");
    within(c.tau_ir, 0.84, 0.05, "tau");
    within(c.scale_height_m, 8_430.0, 150.0, "H");
    within(c.rho0_kg_m3, 1.22, 0.03, "rho0");
    within(c.lapse_dry_k_m * 1000.0, 9.8, 0.25, "Gamma_d");
    within(c.tropopause_m, 11_000.0, 2_500.0, "tropopause");
    within(c.sound_speed_m_s, 340.0, 6.0, "c");
    within(c.hadley_edge_deg, 31.0, 4.0, "Hadley edge");
    assert_eq!(c.cells, 3);
    assert!(c.u_jet_m_s > 20.0 && c.u_jet_m_s < 45.0, "jet {}", c.u_jet_m_s);
    assert!(c.delta_t_equator_pole_k > 25.0 && c.delta_t_equator_pole_k < 50.0, "dT {}", c.delta_t_equator_pole_k);
    within(c.v10_threshold_m_s, 6.5, 1.5, "sand threshold at 10 m");
    within(c.radiative_time_s / 86_400.0, 30.0, 12.0, "radiative time (days)");
    let trades = band_wind(&c, 15.0);
    assert_eq!(trades.band, Band::Trades);
    assert!(trades.east_m_s < 0.0 && trades.north_m_s < 0.0, "NE trades blow toward the south-west: {trades:?}");
    assert!(trades.east_m_s.hypot(trades.north_m_s) > 2.0 && trades.east_m_s.hypot(trades.north_m_s) < 8.0, "{trades:?}");
    let westerlies = band_wind(&c, 45.0);
    assert_eq!(westerlies.band, Band::Westerlies);
    assert!(westerlies.east_m_s > 0.0 && westerlies.north_m_s > 0.0, "{westerlies:?}");
    let south = band_wind(&c, -15.0);
    assert!(south.east_m_s < 0.0 && south.north_m_s > 0.0, "SE trades blow toward the north-west: {south:?}");
    assert_eq!(band_wind(&c, 75.0).band, Band::PolarEasterlies);
}

#[test]
fn mars_reference() {
    let c = derive(&mars());
    within(c.surface_gravity, 3.73, 0.03, "g");
    within(c.t_eq_k, 210.0, 2.0, "T_eq");
    within(c.scale_height_m, 11_100.0, 600.0, "H");
    within(c.rho0_kg_m3, 0.016, 0.002, "rho0");
    within(c.lapse_dry_k_m * 1000.0, 4.5, 0.2, "Gamma_d");
    within(c.sound_speed_m_s, 235.0, 15.0, "c");
    assert!(c.hadley_edge_deg >= 42.0, "Hadley edge {}", c.hadley_edge_deg);
    assert!(c.cells <= 2);
    assert!(c.v10_threshold_m_s > 14.0 && c.v10_threshold_m_s < 40.0, "sand threshold {}", c.v10_threshold_m_s);
    assert!(c.radiative_time_s < 3.0 * 86_400.0, "thin air cools fast: {}", c.radiative_time_s / 86_400.0);
    within(c.diurnal_amplitude_k, 29.0, 1.0, "half swing from data");
}

#[test]
fn tarsis_iii_reference() {
    let c = derive(&sandbox());
    within(c.surface_gravity, 7.775, 0.01, "g");
    within(c.molar_mass_kg_mol * 1e3, 29.18, 0.03, "M");
    within(c.scale_height_m, 10_371.0, 12.0, "H as the game's atmosphere profile");
    within(c.rho0_kg_m3, 1.265, 0.003, "rho0 as the game's air_density(0)");
    within(c.t_eq_k, 258.5, 0.6, "T_eq as tarsis.json");
    within(c.tau_ir, 0.58, 0.03, "tau");
    within(c.coriolis_45_s, 9.9e-5, 0.1e-5, "f(45)");
    within(c.hadley_edge_deg, 31.0, 3.5, "Hadley edge");
    assert_eq!(c.cells, 3, "three cells like Earth");
    within(c.u_star_threshold_m_s, 0.207, 0.006, "u*_t corundum (0.215 with Earth's cohesion)");
    within(c.v10_threshold_m_s, 6.0, 0.4, "sand threshold at 10 m");
    within(c.settling_2um_m_day, 33.0, 4.0, "2 um settles ~33 m/day");
    within(c.sound_speed_m_s, 336.0, 4.0, "c");
    within(c.diurnal_amplitude_k, 7.5, 0.1, "half swing from data");
    assert_eq!(c.seed, 20261006);
    // The wind cap from the data holds.
    assert!(c.v10_scale_m_s <= 45.0);
}

#[test]
fn temperature_follows_latitude_season_and_hour() {
    let c = derive(&sandbox());
    // Equator warmer than the pole, mid-afternoon warmer than before dawn, summer pole warmer than
    // winter pole.
    let eq_noon = surface_temperature_k(&c, 0.0, 14.0, 0.0);
    let pole = surface_temperature_k(&c, 85.0, 14.0, 0.0);
    assert!(eq_noon > pole + 15.0, "{eq_noon} vs {pole}");
    let eq_dawn = surface_temperature_k(&c, 0.0, 4.0, 0.0);
    assert!(eq_noon - eq_dawn > 10.0 && eq_noon - eq_dawn < 20.0, "{eq_noon} - {eq_dawn}");
    let summer = surface_temperature_k(&c, 60.0, 12.0, 15.0);
    let winter = surface_temperature_k(&c, 60.0, 12.0, -15.0);
    assert!(summer > winter + 5.0, "{summer} vs {winter}");
}

#[test]
fn an_airless_body_has_no_wind_and_no_veil() {
    let mut korax = sandbox();
    korax.name = "P3_M1".into();
    korax.kind = BodyKind::Moon;
    korax.pressure_pa = 0.0;
    korax.gases_pct.clear();
    korax.temperatures = Some(Temperatures { day_k: 287.5, night_k: 257.5, surface_k: 272.5, eq_k: 272.5 });
    let c = derive(&korax);
    assert!(!c.has_atmosphere);
    assert_eq!(c.rho0_kg_m3, 0.0);
    assert_eq!(band_wind(&c, 30.0).band, Band::Calm);
    assert_eq!(c.v10_threshold_m_s, 0.0);
}
