//! Client for the `resourcesDynamic` service — the authority on each planet's
//! rotation period (`rotation_h`). We fetch the whole planet catalogue over its
//! REST API once at startup (and refresh periodically), then resolve each NPC's
//! planet to its rotation period locally.
//!
//! We deliberately do NOT ask resourcesDynamic for the time-of-day each tick:
//! its rotation phase is a pure function of wall-clock time with no epoch offset
//! (`spinAngle = 2π · unix_seconds / (rotation_h · 3600)`), so once we know
//! `rotation_h` the [`PlanetClock`](crate::clock::PlanetClock) computes the hour
//! locally and drift-free. resourcesDynamic is only the source of the parameter.

use std::{collections::HashMap, sync::Arc, time::Duration};

use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Resolved rotation parameters for one planet.
#[derive(Debug, Clone)]
pub struct PlanetInfo {
    pub uuid: Option<String>,
    /// Rotation period in hours — the length of one planetary day.
    pub rotation_h: f64,
}

/// One planet row as returned by `GET /api/planets`. drizzle serialises the
/// schema's JS property names, so keys are camelCase (`internalName`,
/// `rotationH`).
#[derive(Debug, Deserialize)]
struct PlanetRow {
    #[serde(default)]
    uuid: Option<String>,
    name: String,
    #[serde(rename = "internalName")]
    internal_name: String,
    #[serde(rename = "rotationH")]
    rotation_h: f64,
}

#[derive(Debug, Deserialize)]
struct PlanetsResponse {
    #[serde(default)]
    data: Vec<PlanetRow>,
}

/// Lookup of planets by `name` and `internal_name` (both lowercased), so a
/// roster can reference either `"Tarsis 4"` or `"tarsis_4"`.
#[derive(Debug, Default)]
pub struct PlanetCatalog {
    by_key: HashMap<String, PlanetInfo>,
}

impl PlanetCatalog {
    fn from_rows(rows: Vec<PlanetRow>) -> Self {
        let mut by_key = HashMap::new();
        for r in rows {
            let info = PlanetInfo {
                uuid: r.uuid.clone(),
                rotation_h: r.rotation_h,
            };
            by_key.insert(r.name.to_lowercase(), info.clone());
            by_key.insert(r.internal_name.to_lowercase(), info);
        }
        Self { by_key }
    }

    /// Rotation parameters for a planet named `key` (its `name` or
    /// `internal_name`, case-insensitive), if known.
    pub fn get(&self, key: &str) -> Option<&PlanetInfo> {
        self.by_key.get(&key.to_lowercase())
    }

    /// Rotation period for `key`, falling back to `fallback` when the planet (or
    /// the whole catalogue) is unknown — so NPCs keep running even if
    /// resourcesDynamic is unreachable.
    pub fn rotation_h_or(&self, key: &str, fallback: f64) -> f64 {
        self.get(key).map(|p| p.rotation_h).unwrap_or(fallback)
    }

    fn len(&self) -> usize {
        // Two keys per planet (name + internal_name).
        self.by_key.len() / 2
    }
}

/// Fetch the planet catalogue from `GET {http_base}/api/planets`.
async fn fetch(http_base: &str) -> anyhow::Result<PlanetCatalog> {
    let url = format!("{}/api/planets", http_base.trim_end_matches('/'));
    let resp: PlanetsResponse = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(PlanetCatalog::from_rows(resp.data))
}

/// A shared, periodically-refreshed planet catalogue. Cheap to clone (`Arc`).
#[derive(Clone)]
pub struct PlanetCatalogHandle {
    inner: Arc<RwLock<PlanetCatalog>>,
}

impl PlanetCatalogHandle {
    /// Build a handle and populate it once from resourcesDynamic. A failed fetch
    /// is non-fatal: the catalogue stays empty and callers fall back to the
    /// configured default rotation period.
    pub async fn bootstrap(http_base: &str) -> Self {
        let catalog = match fetch(http_base).await {
            Ok(c) => {
                info!(planets = c.len(), base = %http_base, "loaded planet catalogue from resourcesDynamic");
                c
            }
            Err(e) => {
                warn!(error = %e, base = %http_base, "could not load planet catalogue; using fallback rotation period");
                PlanetCatalog::default()
            }
        };
        Self {
            inner: Arc::new(RwLock::new(catalog)),
        }
    }

    /// An empty catalogue (every lookup falls back), without any fetch.
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(PlanetCatalog::default())),
        }
    }

    /// Rotation period for `planet_key`, or `fallback` when unknown.
    pub async fn rotation_h_or(&self, planet_key: &str, fallback: f64) -> f64 {
        self.inner.read().await.rotation_h_or(planet_key, fallback)
    }

    /// Spawn a background task that re-fetches the catalogue every `interval`,
    /// swapping it in on success and leaving the previous one on failure.
    pub fn spawn_refresh(&self, http_base: String, interval: Duration) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await; // consume the immediate first tick
            loop {
                ticker.tick().await;
                match fetch(&http_base).await {
                    Ok(c) => {
                        let n = c.len();
                        *inner.write().await = c;
                        info!(planets = n, "refreshed planet catalogue");
                    }
                    Err(e) => warn!(error = %e, "planet catalogue refresh failed; keeping previous"),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> PlanetCatalog {
        PlanetCatalog::from_rows(vec![
            PlanetRow {
                uuid: Some("uuid-4".into()),
                name: "Tarsis 4".into(),
                internal_name: "tarsis_4".into(),
                rotation_h: 25.0,
            },
            PlanetRow {
                uuid: Some("uuid-3".into()),
                name: "Tarsis 3".into(),
                internal_name: "tarsis_3".into(),
                rotation_h: 24.0,
            },
        ])
    }

    #[test]
    fn resolves_by_name_and_internal_name_case_insensitively() {
        let c = catalog();
        assert_eq!(c.get("Tarsis 4").unwrap().rotation_h, 25.0);
        assert_eq!(c.get("tarsis_4").unwrap().rotation_h, 25.0);
        assert_eq!(c.get("TARSIS_4").unwrap().rotation_h, 25.0);
        assert_eq!(c.get("tarsis_3").unwrap().rotation_h, 24.0);
    }

    #[test]
    fn falls_back_when_unknown() {
        let c = catalog();
        assert_eq!(c.rotation_h_or("tarsis_4", 99.0), 25.0);
        assert_eq!(c.rotation_h_or("no_such_planet", 99.0), 99.0);
        assert_eq!(PlanetCatalog::default().rotation_h_or("tarsis_4", 25.0), 25.0);
    }

    #[test]
    fn parses_rest_payload_shape() {
        let raw = r#"{"success":true,"count":1,"data":[
            {"id":5,"uuid":"abc","name":"Tarsis 4","internalName":"tarsis_4",
             "tidalLocked":false,"tiltRad":0.26,"rotationH":25.0}
        ]}"#;
        let resp: PlanetsResponse = serde_json::from_str(raw).unwrap();
        let c = PlanetCatalog::from_rows(resp.data);
        assert_eq!(c.get("tarsis_4").unwrap().rotation_h, 25.0);
        assert_eq!(c.get("tarsis_4").unwrap().uuid.as_deref(), Some("abc"));
    }
}
