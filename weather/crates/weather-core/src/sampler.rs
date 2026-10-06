//! What the weather is at ONE point: the climate background plus every system in reach. This is
//! the function the game re-implements (`WeatherSampler`), so the server, the clients and this
//! service agree; the service uses it for its map API.

use crate::climate::{air_density, band_wind, surface_temperature_k, Band, Climate};
use crate::synoptic::{contribution, SystemState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub lon_deg: f64,
    pub lat_deg: f64,
    pub altitude_m: f64,
    pub local_solar_hours: f64,
    pub declination_deg: f64,
    pub temperature_k: f64,
    pub pressure_pa: f64,
    pub pressure_anomaly_pa: f64,
    pub air_density_kg_m3: f64,
    /// Wind components, m/s: east = the direction the body turns toward, north = toward its pole.
    pub wind_east_m_s: f64,
    pub wind_north_m_s: f64,
    pub wind_speed_m_s: f64,
    /// Meteorological direction the wind comes FROM, degrees, 0 = north, clockwise.
    pub wind_from_deg: f64,
    /// Dust optical depth added by storms (0 = only the planet's own veil).
    pub dust_tau: f64,
    /// Visibility through the storm dust alone, m (a large number when there is none).
    pub visibility_m: f64,
    pub humidity: f64,
    pub precipitation_mm_h: f64,
    pub band: Band,
    /// Ids of the systems that weigh more than 1 % here, strongest first.
    pub systems: Vec<u64>,
}

/// Sample the weather at a point. `local_solar_hours` and `declination_deg` come from the body's
/// spin and orbit (spin.rs); `systems` from synoptic::systems_at.
pub fn sample(c: &Climate, systems: &[SystemState], lon_deg: f64, lat_deg: f64, altitude_m: f64, local_solar_hours: f64, declination_deg: f64) -> Sample {
    let temperature = surface_temperature_k(c, lat_deg, local_solar_hours, declination_deg);
    let background = band_wind(c, lat_deg);
    let (mut we, mut wn) = (background.east_m_s, background.north_m_s);
    let (mut dp, mut dust, mut humidity, mut rain) = (0.0, 0.0, 0.0, 0.0);
    let mut weights: Vec<(f64, u64)> = Vec::new();
    for s in systems {
        let k = contribution(c, s, lon_deg, lat_deg);
        if k.weight < 0.01 {
            continue;
        }
        we += k.wind_east_m_s;
        wn += k.wind_north_m_s;
        dp += k.pressure_pa;
        dust += k.dust_tau;
        humidity += k.humidity;
        rain += k.precipitation_mm_h;
        weights.push((k.weight, s.id));
    }
    weights.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let speed = (we * we + wn * wn).sqrt().min(c.extras.wind_scale_max_ms);
    let scale = if speed > 0.0 { speed / (we * we + wn * wn).sqrt() } else { 1.0 };
    let (we, wn) = (we * scale, wn * scale);
    // Hydrostatic pressure at the altitude, plus the systems' anomaly.
    let column = if c.has_atmosphere { c.pressure_pa * (-altitude_m / c.scale_height_m).exp() } else { 0.0 };
    let pressure = column + dp;
    // Koschmieder through a dust layer ~1 km thick: V = 3.912 / β, β = τ / 1 000 m.
    let visibility = if dust > 1e-6 { 3.912 / (dust / 1_000.0) } else { 1.0e6 };
    Sample {
        lon_deg,
        lat_deg,
        altitude_m,
        local_solar_hours,
        declination_deg,
        temperature_k: temperature,
        pressure_pa: pressure,
        pressure_anomaly_pa: dp,
        air_density_kg_m3: air_density(c, altitude_m) * if c.has_atmosphere { pressure / column.max(1e-9) } else { 0.0 },
        wind_east_m_s: we,
        wind_north_m_s: wn,
        wind_speed_m_s: speed,
        wind_from_deg: wind_from_deg(we, wn),
        dust_tau: dust,
        visibility_m: visibility,
        humidity: humidity.min(1.0),
        precipitation_mm_h: rain,
        band: background.band,
        systems: weights.into_iter().map(|(_, id)| id).collect(),
    }
}

/// The direction a wind (east, north) comes FROM: 0 = from the north, 90 = from the east.
pub fn wind_from_deg(east: f64, north: f64) -> f64 {
    if east == 0.0 && north == 0.0 {
        return 0.0;
    }
    ((-east).atan2(-north).to_degrees()).rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_from_follows_the_meteorological_convention() {
        assert!((wind_from_deg(0.0, -1.0) - 0.0).abs() < 1e-9, "blowing south comes from the north");
        assert!((wind_from_deg(-1.0, 0.0) - 90.0).abs() < 1e-9, "blowing west comes from the east");
        assert!((wind_from_deg(1.0, 0.0) - 270.0).abs() < 1e-9, "blowing east comes from the west");
        assert!((wind_from_deg(0.0, 1.0) - 180.0).abs() < 1e-9, "blowing north comes from the south");
    }
}
