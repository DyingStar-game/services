//! Villages and cities of a planet, from the QGIS POI export
//! (`DyingStar/assets/qgis/export/<planet>_poi.json`, copied under
//! `WORLD_DATA_DIR/<planet>/poi.json`).
//!
//! File shape (only the keys read here):
//!
//! ```json
//! { "planet_name": "tarsis_3", "planet_radius": 6356000.0,
//!   "pois": [{ "id": 1, "name": "M0001", "poi_type": "mining village",
//!              "radius": 1000.0, "lon": 12.3, "lat": -4.5 }] }
//! ```

use serde::Deserialize;

use super::geo;
use crate::horizon::messages::Vec3;

/// What a POI is, from its `poi_type`. Mining villages are named `Mxxxx`,
/// factory villages `Fxxxx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VillageKind {
    Mining,
    Factory,
    RailwayCity,
    City,
    Other,
}

impl VillageKind {
    fn from_poi_type(t: &str) -> Self {
        match t.trim().to_ascii_lowercase().as_str() {
            "mining village" => Self::Mining,
            "factory village" => Self::Factory,
            "railway city" => Self::RailwayCity,
            "city" => Self::City,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Village {
    pub id: i64,
    pub name: String,
    pub kind: VillageKind,
    pub lon: f64,
    pub lat: f64,
    /// Unit direction of the centre, planet frame.
    #[serde(skip)]
    pub unit: Vec3,
    pub radius_m: f64,
}

#[derive(Debug, Deserialize)]
pub struct PoiFile {
    #[serde(default)]
    pub planet_name: Option<String>,
    pub planet_radius: f64,
    #[serde(default)]
    pub pois: Vec<RawPoi>,
}

#[derive(Debug, Deserialize)]
pub struct RawPoi {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub poi_type: String,
    #[serde(default)]
    pub radius: Option<f64>,
    pub lon: f64,
    pub lat: f64,
}

impl Village {
    pub fn from_raw(p: &RawPoi) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            kind: VillageKind::from_poi_type(&p.poi_type),
            lon: p.lon,
            lat: p.lat,
            unit: geo::lonlat_to_unit(p.lon, p.lat),
            radius_m: p.radius.unwrap_or(0.0),
        }
    }
}

/// Every village of one planet.
#[derive(Debug, Default)]
pub struct Villages {
    pub radius_m: f64,
    pub all: Vec<Village>,
}

impl Villages {
    pub fn from_file(f: &PoiFile) -> Self {
        Self {
            radius_m: f.planet_radius,
            all: f.pois.iter().map(Village::from_raw).collect(),
        }
    }

    pub fn by_name(&self, name: &str) -> Option<&Village> {
        self.all.iter().find(|v| v.name.eq_ignore_ascii_case(name))
    }

    pub fn of_kind(&self, kind: VillageKind) -> impl Iterator<Item = &Village> {
        self.all.iter().filter(move |v| v.kind == kind)
    }

    /// The village of `kind` whose centre is closest (great circle) to `pos`
    /// (planet frame; only its direction matters), with that distance.
    pub fn nearest(&self, kind: VillageKind, pos: Vec3) -> Option<(&Village, f64)> {
        self.of_kind(kind)
            .map(|v| (v, geo::arc_m(v.unit, pos, self.radius_m)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn villages() -> Villages {
        let f: PoiFile = serde_json::from_value(serde_json::json!({
            "planet_radius": 6356000.0,
            "pois": [
                {"id": 1, "name": "M0001", "poi_type": "mining village", "radius": 1000.0, "lon": 0.0, "lat": 0.0},
                {"id": 2, "name": "F0001", "poi_type": "factory village", "radius": 2000.0, "lon": 0.1, "lat": 0.0},
                {"id": 3, "name": "F0002", "poi_type": "factory village", "radius": 2000.0, "lon": 1.0, "lat": 0.0},
                {"id": 4, "name": "city_1", "poi_type": "city", "lon": 0.01, "lat": 0.0, "elevation": null}
            ]
        }))
        .unwrap();
        Villages::from_file(&f)
    }

    #[test]
    fn kinds_come_from_poi_type() {
        let v = villages();
        assert_eq!(v.by_name("m0001").unwrap().kind, VillageKind::Mining);
        assert_eq!(v.of_kind(VillageKind::Factory).count(), 2);
        assert_eq!(v.by_name("city_1").unwrap().kind, VillageKind::City);
    }

    #[test]
    fn nearest_picks_by_great_circle() {
        let v = villages();
        let here = geo::lonlat_to_unit(0.02, 0.0);
        let (f, d) = v.nearest(VillageKind::Factory, here).unwrap();
        assert_eq!(f.name, "F0001");
        assert!(d > 8_000.0 && d < 9_500.0, "d = {d}");
    }
}
