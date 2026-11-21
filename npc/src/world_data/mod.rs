//! Static world data of the planet the NPCs live on: villages (POIs) and the
//! road network, from the QGIS exports — plus the routing choices built on
//! them (which Factory village each Mining village drives to).
//!
//! Files (under `WORLD_DATA_DIR`, default `config/world`):
//!
//! - `<planet>/poi.json`   ← `DyingStar/assets/qgis/export/<planet>_poi.json`
//! - `<planet>/roads.json` ← `DyingStar/assets/qgis/export/<planet>_roads_buffered.json`
//!
//! and the routing config `ROUTING_CONFIG` (default `config/routing.yaml`).
//! Everything is computed once at startup; the planet frame used throughout
//! is the one villages and vehicles are placed in (parent = the planet).

pub mod geo;
pub mod poi;
pub mod road_graph;

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::horizon::messages::Vec3;
use poi::{PoiFile, Village, VillageKind, Villages};
use road_graph::{RoadGraph, Route};

/// `ROUTING_CONFIG` file.
#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    #[serde(default = "default_planet")]
    pub planet: String,
    /// Vertices of two roads closer than this are one intersection.
    #[serde(default = "default_snap")]
    pub road_snap_m: f64,
    /// A village whose centre lies further than this from every road is not
    /// reachable by road (it is never picked as a destination).
    #[serde(default = "default_max_access")]
    pub max_access_m: f64,
    /// Cap on the NPC's driving speed, whatever the road allows.
    #[serde(default = "default_max_speed")]
    pub max_speed_kmh: f64,
    /// Speed on the off-road stretches (garage ↔ road ↔ village).
    #[serde(default = "default_offroad_speed")]
    pub offroad_speed_kmh: f64,
    /// Static overrides: Mining village → Factory village it drives to. A
    /// Mining village not listed goes to the Factory nearest by road.
    #[serde(default)]
    pub factory_for: HashMap<String, String>,
}

fn default_planet() -> String {
    "tarsis_3".into()
}
fn default_snap() -> f64 {
    road_graph::DEFAULT_SNAP_M
}
fn default_max_access() -> f64 {
    2_000.0
}
fn default_max_speed() -> f64 {
    50.0
}
fn default_offroad_speed() -> f64 {
    15.0
}

impl Default for RoutingConfig {
    fn default() -> Self {
        serde_yaml::from_str("{}").expect("all fields defaulted")
    }
}

/// Which Factory a Mining village drives to, and why.
#[derive(Debug, Clone, Serialize)]
pub struct FactoryChoice {
    pub factory: String,
    /// Road cost from the Mining village to it (≈ metres; `None` for a static
    /// override that the network could not price).
    pub road_cost_m: Option<f64>,
    /// `true` when it comes from `factory_for` in the routing config.
    pub configured: bool,
}

pub struct WorldData {
    pub config: RoutingConfig,
    pub villages: Villages,
    pub graph: RoadGraph,
    /// Mining village name → chosen Factory.
    pub factory_for: BTreeMap<String, FactoryChoice>,
}

impl WorldData {
    /// The process-wide world data, loaded once. `None` when the files are
    /// missing or invalid (logged): actions that need it then never run.
    pub fn shared() -> Option<Arc<WorldData>> {
        static SHARED: OnceLock<Option<Arc<WorldData>>> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                let dir = std::env::var("WORLD_DATA_DIR").unwrap_or_else(|_| "config/world".into());
                let cfg = std::env::var("ROUTING_CONFIG")
                    .unwrap_or_else(|_| "config/routing.yaml".into());
                match WorldData::load(Path::new(&dir), Path::new(&cfg)) {
                    Ok(w) => Some(Arc::new(w)),
                    Err(e) => {
                        warn!(error = format!("{e:#}"), dir, "no world data; driving actions disabled");
                        None
                    }
                }
            })
            .clone()
    }

    pub fn load(dir: &Path, routing_path: &Path) -> anyhow::Result<Self> {
        let config: RoutingConfig = match fs::read_to_string(routing_path) {
            Ok(t) => serde_yaml::from_str(&t)
                .with_context(|| format!("parsing {}", routing_path.display()))?,
            Err(_) => {
                info!(path = %routing_path.display(), "no routing config; using defaults");
                RoutingConfig::default()
            }
        };
        let pdir = dir.join(&config.planet);
        let poi_path = pdir.join("poi.json");
        let roads_path = pdir.join("roads.json");
        let poi: PoiFile = serde_json::from_str(
            &fs::read_to_string(&poi_path).with_context(|| format!("reading {}", poi_path.display()))?,
        )
        .with_context(|| format!("parsing {}", poi_path.display()))?;
        let roads = road_graph::parse_roads(
            &fs::read_to_string(&roads_path)
                .with_context(|| format!("reading {}", roads_path.display()))?,
        )
        .with_context(|| format!("parsing {}", roads_path.display()))?;
        Ok(Self::build(config, &poi, roads))
    }

    pub fn build(config: RoutingConfig, poi: &PoiFile, roads: Vec<road_graph::RoadDef>) -> Self {
        let villages = Villages::from_file(poi);
        let n_roads = roads.len();
        let graph = RoadGraph::build(roads, villages.radius_m, config.road_snap_m);
        let mut w = Self { config, villages, graph, factory_for: BTreeMap::new() };
        w.factory_for = w.choose_factories();
        info!(
            planet = %w.config.planet,
            villages = w.villages.all.len(),
            roads = n_roads,
            nodes = w.graph.node_count(),
            edges = w.graph.edge_count(),
            mining_with_factory = w.factory_for.len(),
            "world data loaded"
        );
        w
    }

    /// For every Mining village: the configured Factory, else the one nearest
    /// by road among those within `max_access_m` of the network.
    fn choose_factories(&self) -> BTreeMap<String, FactoryChoice> {
        let factories: Vec<(&Village, road_graph::Location)> = self
            .villages
            .of_kind(VillageKind::Factory)
            .filter_map(|f| {
                let loc = self.graph.locate(f.unit)?;
                (loc.offset_m <= self.config.max_access_m).then_some((f, loc))
            })
            .collect();
        let targets: Vec<road_graph::Location> = factories.iter().map(|(_, l)| *l).collect();

        let mut out = BTreeMap::new();
        for m in self.villages.of_kind(VillageKind::Mining) {
            if let Some(f) = self.config.factory_for.iter().find(|(k, _)| k.eq_ignore_ascii_case(&m.name)) {
                if self.villages.by_name(f.1).is_none_or(|v| v.kind != VillageKind::Factory) {
                    warn!(mining = %m.name, factory = %f.1, "factory_for names no Factory village; ignored");
                } else {
                    out.insert(
                        m.name.clone(),
                        FactoryChoice { factory: f.1.clone(), road_cost_m: None, configured: true },
                    );
                    continue;
                }
            }
            let Some(from) = self.graph.locate(m.unit).filter(|l| l.offset_m <= self.config.max_access_m)
            else {
                continue;
            };
            let costs = self.graph.costs_to(&from, &targets);
            let best = costs
                .iter()
                .enumerate()
                .filter(|(_, c)| c.is_finite())
                .min_by(|a, b| a.1.total_cmp(b.1));
            if let Some((i, c)) = best {
                out.insert(
                    m.name.clone(),
                    FactoryChoice {
                        factory: factories[i].0.name.clone(),
                        road_cost_m: Some(*c),
                        configured: false,
                    },
                );
            }
        }
        out
    }

    pub fn radius_m(&self) -> f64 {
        self.villages.radius_m
    }

    /// The Mining village an NPC standing at `pos` (planet frame) belongs to:
    /// the nearest one, if `pos` lies within twice its radius.
    pub fn home_mining_village(&self, pos: Vec3) -> Option<&Village> {
        let (v, d) = self.villages.nearest(VillageKind::Mining, pos)?;
        (d <= (2.0 * v.radius_m).max(2_000.0)).then_some(v)
    }

    /// Destination of a trip from Mining village `mining`: `explicit` when
    /// given (an NPC-level override), else the routing choice.
    pub fn factory_destination(&self, mining: &str, explicit: Option<&str>) -> Option<&Village> {
        let name = match explicit {
            Some(n) => n.to_string(),
            None => self.factory_for.get(mining)?.factory.clone(),
        };
        self.villages.by_name(&name).filter(|v| v.kind == VillageKind::Factory)
    }

    /// Road route between two planet-frame points, with off-road stretches at
    /// both ends.
    pub fn route(&self, from: Vec3, to: Vec3) -> Option<Route> {
        self.graph.route_points(from, to, self.config.offroad_speed_kmh)
    }

    /// The road point nearest `p` (planet frame, on the reference sphere).
    pub fn road_point_near(&self, p: Vec3) -> Option<Vec3> {
        let l = self.graph.locate(p)?;
        Some(geo::scale(geo::lonlat_to_unit(l.lonlat.0, l.lonlat.1), self.radius_m()))
    }

    /// A trip made of an off-road stretch onto the road, the road, and an
    /// off-road stretch off it: `start_leg` runs from the vehicle to the road
    /// (its last point), `end_leg` from the road (its first point) to the
    /// target. Off-road stretches are driven at `offroad_speed_kmh`. `None`
    /// when the two road points are not connected.
    pub fn route_via(&self, start_leg: &[Vec3], end_leg: &[Vec3]) -> Option<Route> {
        let (entry, exit) = (*start_leg.last()?, *end_leg.first()?);
        let off = road_graph::Stretch::offroad(self.config.offroad_speed_kmh);
        let mut legs: Vec<((f64, f64), road_graph::Stretch)> =
            start_leg.iter().map(|p| (geo::unit_to_lonlat(*p), off)).collect();
        let road = self.graph.road_legs(entry, exit)?;
        legs.extend(road);
        legs.extend(end_leg.iter().map(|p| (geo::unit_to_lonlat(*p), off)));
        Some(self.graph.densify_legs(&legs))
    }

    /// Planet-frame position (on the reference sphere) of a village centre.
    pub fn village_position(&self, v: &Village) -> Vec3 {
        geo::scale(v.unit, self.radius_m())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real tarsis_3 data shipped in the repo loads, and every Mining
    /// village that has a road gets a Factory.
    #[test]
    fn loads_the_shipped_tarsis_3_data() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let w = WorldData::load(&root.join("config/world"), &root.join("config/routing.yaml"))
            .expect("tarsis_3 world data shipped");
        assert_eq!(w.config.planet, "tarsis_3");
        assert!(w.villages.of_kind(VillageKind::Mining).count() > 50);
        assert!(w.graph.node_count() > 1000);
        assert!(
            w.factory_for.len() > 30,
            "only {} mining villages got a factory",
            w.factory_for.len()
        );
        // Pick one and route it end to end.
        let (m, choice) = w.factory_for.iter().next().unwrap();
        let from = w.village_position(w.villages.by_name(m).unwrap());
        let to = w.village_position(w.villages.by_name(&choice.factory).unwrap());
        let r = w.route(from, to).expect("route exists");
        assert!(r.length_m > 1_000.0);
        assert!(r.points.len() > 10);
    }

    #[test]
    fn static_override_wins() {
        let poi: PoiFile = serde_json::from_value(serde_json::json!({
            "planet_radius": 6356000.0,
            "pois": [
                {"name": "M0001", "poi_type": "mining village", "radius": 1000.0, "lon": 0.0, "lat": 0.0},
                {"name": "F0001", "poi_type": "factory village", "radius": 2000.0, "lon": 0.05, "lat": 0.0},
                {"name": "F0002", "poi_type": "factory village", "radius": 2000.0, "lon": 0.5, "lat": 0.0}
            ]
        }))
        .unwrap();
        let roads = vec![road_graph::RoadDef {
            road_type: "road".into(),
            speed_kmh: 50.0,
            width_m: 6.0,
            lanes: 2,
            centerline: vec![(0.0, 0.0), (0.05, 0.0), (0.5, 0.0)],
        }];
        let w = WorldData::build(RoutingConfig::default(), &poi, roads.clone());
        assert_eq!(w.factory_for["M0001"].factory, "F0001");
        let mut cfg = RoutingConfig::default();
        cfg.factory_for.insert("M0001".into(), "F0002".into());
        let w = WorldData::build(cfg, &poi, roads);
        assert_eq!(w.factory_for["M0001"].factory, "F0002");
        assert!(w.factory_for["M0001"].configured);
    }
}
