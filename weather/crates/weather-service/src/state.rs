//! What the service holds in memory: the bodies with their derived climate, the Horizon uuids of
//! the planets (resolved from the celestial service, because they change at every import), what
//! has been published, and the channel every connected bridge listens to.

use crate::config::Config;
use ds_weather_proto::WeatherSystemData;
use crate::atmosphere::AtmosphereRun;
use prometheus::{GaugeVec, IntCounter, IntCounterVec, IntGauge, Opts, Registry};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;
use tracing::{info, warn};
use uuid::Uuid;
use weather_core::bodies;
use weather_core::climate::{derive, BodyInputs, BodyKind, Climate};
use weather_core::math::Vec3;

pub struct Body {
    pub inputs: BodyInputs,
    pub climate: Climate,
    /// Index of the planet this moon orbits (its own index for a planet).
    pub star_orbit_of: usize,
}

pub struct Metrics {
    pub registry: Registry,
    pub ticks: IntCounter,
    pub systems_alive: IntGauge,
    pub envelopes_sent: IntCounter,
    pub bridges_connected: IntGauge,
    /// Atmosphere model, per body (empty while the model is off).
    pub atmo_model_day: GaugeVec,
    pub atmo_max_wind: GaugeVec,
    pub atmo_lag_s: GaugeVec,
    pub atmo_rollbacks: IntCounterVec,
}

impl Metrics {
    fn new() -> Self {
        let registry = Registry::new();
        let ticks = IntCounter::new("weather_ticks_total", "Simulation ticks").unwrap();
        let systems_alive = IntGauge::new("weather_systems_alive", "Weather systems alive, all bodies").unwrap();
        let envelopes_sent = IntCounter::new("weather_envelopes_sent_total", "Envelopes broadcast to bridges").unwrap();
        let bridges_connected = IntGauge::new("weather_bridges_connected", "Connected ds_bridge clients").unwrap();
        for c in [&ticks, &envelopes_sent] {
            registry.register(Box::new(c.clone())).unwrap();
        }
        for g in [&systems_alive, &bridges_connected] {
            registry.register(Box::new(g.clone())).unwrap();
        }
        let gauge = |name: &str, help: &str| {
            let g = GaugeVec::new(Opts::new(name, help), &["body"]).unwrap();
            registry.register(Box::new(g.clone())).unwrap();
            g
        };
        let atmo_model_day = gauge("weather_atmosphere_model_day", "Atmosphere model time, in model days (the body's solar day)");
        let atmo_max_wind = gauge("weather_atmosphere_max_wind_m_s", "Largest wind of the atmosphere model, any cell and layer (m/s)");
        let atmo_lag_s = gauge("weather_atmosphere_lag_s", "How far the atmosphere model is behind the wall clock (model seconds; 0 when in pace)");
        let atmo_rollbacks = IntCounterVec::new(Opts::new("weather_atmosphere_rollbacks_total", "Roll-backs of the atmosphere model's guard"), &["body"]).unwrap();
        registry.register(Box::new(atmo_rollbacks.clone())).unwrap();
        Self { registry, ticks, systems_alive, envelopes_sent, bridges_connected, atmo_model_day, atmo_max_wind, atmo_lag_s, atmo_rollbacks }
    }
}

pub struct AppState {
    pub config: Config,
    pub bodies: Vec<Body>,
    /// Horizon uuid by body name (lower-case), from the celestial service.
    pub planet_uuids: RwLock<HashMap<String, String>>,
    /// What Horizon currently holds, by object uuid, with the time it was last sent.
    pub published: Mutex<HashMap<Uuid, (WeatherSystemData, f64)>>,
    /// Serialised envelopes to every connected bridge.
    pub tx: broadcast::Sender<String>,
    pub metrics: Metrics,
    /// The atmosphere model's runs, by body name in lower case (empty when the model is off).
    pub atmosphere: HashMap<String, Arc<AtmosphereRun>>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn load(config: Config) -> anyhow::Result<Shared> {
        let system = std::fs::read_to_string(&config.system_json).map_err(|e| anyhow::anyhow!("{}: {e}", config.system_json))?;
        let climate = std::fs::read_to_string(&config.climate_json).ok();
        if climate.is_none() {
            warn!(path = %config.climate_json, "no climate extras file: every body uses the defaults");
        }
        let inputs = bodies::load(&system, climate.as_deref()).map_err(anyhow::Error::msg)?;
        let mut bodies = Vec::new();
        let mut last_planet = 0usize;
        for (i, b) in inputs.into_iter().enumerate() {
            if b.kind == BodyKind::Planet {
                last_planet = i;
            }
            let climate = derive(&b);
            info!(body = %climate.name, air = climate.has_atmosphere, cells = climate.cells, hadley = format!("{:.0}", climate.hadley_edge_deg), v10 = format!("{:.1}", climate.v10_scale_m_s), "climate derived");
            bodies.push(Body { inputs: b, climate, star_orbit_of: last_planet });
        }
        let (tx, _) = broadcast::channel(4096);
        let atmosphere = crate::atmosphere::plan(&config.atmosphere, &bodies);
        Ok(Arc::new(AppState { config, bodies, planet_uuids: RwLock::new(HashMap::new()), published: Mutex::new(HashMap::new()), tx, metrics: Metrics::new(), atmosphere }))
    }

    pub fn body(&self, name: &str) -> Option<&Body> {
        self.bodies.iter().find(|b| b.inputs.name.eq_ignore_ascii_case(name))
    }

    /// Direction body → star in the parent frame at `t`: minus the orbital position of the planet
    /// (a moon shares its planet's star direction, close enough for seasons).
    pub fn to_star(&self, body: &Body, t: f64) -> Vec3 {
        let planet = &self.bodies[body.star_orbit_of];
        planet.inputs.orbit.solver(planet.inputs.mass_kg).position_at(t).scale(-1.0)
    }

    pub fn planet_uuid(&self, name: &str) -> Option<String> {
        self.planet_uuids.read().ok()?.get(&name.to_lowercase()).cloned()
    }

    /// Ask the celestial service which uuid each planet and moon carries in Horizon. Called at boot
    /// and whenever a bridge (re)connects: the celestial import regenerates them.
    pub async fn refresh_planet_uuids(&self) -> anyhow::Result<usize> {
        let url = format!("{}/api/planets", self.config.celestial_http.trim_end_matches('/'));
        // Plain HTTP inside the cluster; blocking client on a worker thread (no TLS, no C toolchain).
        let fetch_url = url.clone();
        let v: serde_json::Value = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
            Ok(ureq::get(&fetch_url).timeout(std::time::Duration::from_secs(10)).call()?.into_json()?)
        })
        .await??;
        let mut found = HashMap::new();
        let mut planet_uuids = Vec::new();
        if let Some(list) = v.get("data").and_then(|d| d.as_array()) {
            for p in list {
                if let (Some(uuid), Some(name)) = (p.get("uuid").and_then(|u| u.as_str()), p.get("name").and_then(|n| n.as_str())) {
                    found.insert(name.to_lowercase(), uuid.to_string());
                    planet_uuids.push(uuid.to_string());
                }
            }
        }
        // Moons are only served with their planet: GET /api/planets/<uuid>.
        for puuid in planet_uuids {
            let detail_url = format!("{}/{}", url, puuid);
            let detail: anyhow::Result<serde_json::Value> = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
                Ok(ureq::get(&detail_url).timeout(std::time::Duration::from_secs(10)).call()?.into_json()?)
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|r| r);
            if let Ok(d) = detail {
                if let Some(moons) = d.get("data").and_then(|p| p.get("moons")).and_then(|m| m.as_array()) {
                    for m in moons {
                        if let (Some(uuid), Some(name)) = (m.get("uuid").and_then(|u| u.as_str()), m.get("name").and_then(|n| n.as_str())) {
                            found.insert(name.to_lowercase(), uuid.to_string());
                        }
                    }
                }
            }
        }
        let n = found.len();
        if n > 0 {
            if let Ok(mut map) = self.planet_uuids.write() {
                *map = found;
            }
        }
        info!(url, planets = n, "planet uuids refreshed");
        Ok(n)
    }
}
