//! The simulation loop: every tick, evaluate the systems of every body with a Horizon uuid,
//! compare with what Horizon holds, and publish the difference (create / update / delete). The
//! evaluation itself is pure (weather-core); the loop only keeps the list of what was sent, so a
//! bridge that reconnects can be given the whole picture again.

use crate::state::{Body, Shared};
use ds_weather_proto::{create_envelope, delete_envelope, system_uuid, update_envelope, SystemKind, SystemParams, Vec3f, WeatherSystemData, SCENENAME};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};
use uuid::Uuid;
use weather_core::spin::{declination_deg, dir_of};
use weather_core::synoptic::{lattice, slots_in_band, system_in_slot, systems_at, Kind, SystemState};

pub fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// East and north unit vectors of the ground at (lon, lat), in the body-fixed frame. East is the
/// direction the body turns toward: INcreasing longitude in this frame (spin.rs, dir_of).
pub fn local_axes(lon_deg: f64, lat_deg: f64) -> (Vec3f, Vec3f) {
    let (lon, lat) = (lon_deg.to_radians(), lat_deg.to_radians());
    let east = Vec3f { x: -lon.sin(), y: 0.0, z: -lon.cos() };
    let north = Vec3f { x: -lat.sin() * lon.cos(), y: lat.cos(), z: lat.sin() * lon.sin() };
    (east, north)
}

fn surface_position(radius_m: f64, lon_deg: f64, lat_deg: f64) -> Vec3f {
    let d = dir_of(lon_deg, lat_deg);
    Vec3f { x: d.x * radius_m, y: d.y * radius_m, z: d.z * radius_m }
}

fn kind_of(k: Kind) -> SystemKind {
    match k {
        Kind::Depression => SystemKind::Depression,
        Kind::Anticyclone => SystemKind::Anticyclone,
        Kind::DustStorm => SystemKind::DustStorm,
        Kind::RainFront => SystemKind::RainFront,
    }
}

/// The published form of one system at `t`, with its keyframes ahead.
pub fn to_object(body: &Body, parent_uuid: &str, s: &SystemState, t: f64, decl_deg: f64, keyframe_dt_s: f64, keyframes: usize) -> WeatherSystemData {
    let c = &body.climate;
    let (east, north) = local_axes(s.centre_lon_deg, s.centre_lat_deg);
    let velocity = Vec3f {
        x: east.x * s.drift_east_m_s + north.x * s.drift_north_m_s,
        y: east.y * s.drift_east_m_s + north.y * s.drift_north_m_s,
        z: east.z * s.drift_east_m_s + north.z * s.drift_north_m_s,
    };
    // Keyframes: where the same system will be every keyframe_dt_s, until it dies (then it stays
    // put at zero intensity).
    let l = lattice(c);
    let slots = slots_in_band(&l, c, l.bands[s.band as usize]);
    let mut track = Vec::with_capacity(keyframes + 1);
    let mut intensity_track = Vec::with_capacity(keyframes + 1);
    let mut last = surface_position(c.radius_m, s.centre_lon_deg, s.centre_lat_deg);
    for k in 0..=keyframes {
        let tk = t + k as f64 * keyframe_dt_s;
        match system_in_slot(c, &l, s.band, s.slot, slots, tk, decl_deg) {
            Some(f) if f.id == s.id => {
                last = surface_position(c.radius_m, f.centre_lon_deg, f.centre_lat_deg);
                track.push(last);
                intensity_track.push(f.intensity);
            }
            _ => {
                track.push(last);
                intensity_track.push(0.0);
            }
        }
    }
    WeatherSystemData {
        name: format!("{}-{:?}-{:04x}", body.inputs.name, s.kind, s.id & 0xffff),
        parent_id: parent_uuid.to_string(),
        scenename: SCENENAME.to_string(),
        kind: kind_of(s.kind),
        position: surface_position(c.radius_m, s.centre_lon_deg, s.centre_lat_deg),
        rotation: Vec3f::default(),
        radius_m: 2.0 * s.sigma_m,
        intensity: s.intensity,
        pressure_anomaly_pa: s.pressure_anomaly_pa,
        velocity,
        from_timestamp: t,
        keyframe_dt_s,
        track,
        intensity_track,
        params: SystemParams { wind_peak_ms: s.wind_peak_m_s, dust_tau: s.dust_tau, precipitation_mm_h: s.precipitation_mm_h, humidity: s.humidity },
        born_at: s.born_at,
        dies_at: s.dies_at,
    }
}

/// Everything alive on a body at `t`, keyed by object uuid.
pub fn snapshot(state: &Shared, body: &Body, t: f64) -> HashMap<Uuid, WeatherSystemData> {
    let mut out = HashMap::new();
    let Some(parent) = state.planet_uuid(&body.inputs.name) else { return out };
    let decl = declination_deg(body.inputs.tilt_deg.to_radians(), state.to_star(body, t));
    for s in systems_at(&body.climate, t, decl) {
        let uuid = system_uuid(&body.inputs.name, s.id);
        out.insert(uuid, to_object(body, &parent, &s, t, decl, state.config.keyframe_dt_s, state.config.keyframes));
    }
    out
}

/// One tick: publish the difference between what Horizon holds and what the model says now.
pub fn tick(state: &Shared, t: f64) -> (usize, usize, usize) {
    let mut wanted: HashMap<Uuid, WeatherSystemData> = HashMap::new();
    for body in state.bodies.iter().filter(|b| b.climate.has_atmosphere) {
        wanted.extend(snapshot(state, body, t));
    }
    let mut published = state.published.lock().unwrap();
    let (mut created, mut updated, mut deleted) = (0, 0, 0);
    let mut next: HashMap<Uuid, (WeatherSystemData, f64)> = HashMap::with_capacity(wanted.len());
    for (uuid, data) in wanted {
        match published.get(&uuid) {
            None => {
                created += 1;
                send(state, &create_envelope(uuid, &data));
                next.insert(uuid, (data, t));
            }
            Some((old, sent_at)) => {
                let due = t - sent_at >= state.config.update_every_s;
                let moved = (old.intensity - data.intensity).abs() > 0.05 || old.kind != data.kind;
                if due || moved {
                    updated += 1;
                    send(state, &update_envelope(uuid, &data));
                    next.insert(uuid, (data, t));
                } else {
                    next.insert(uuid, (old.clone(), *sent_at));
                }
            }
        }
    }
    let gone: Vec<Uuid> = published.keys().filter(|u| !next.contains_key(u)).copied().collect();
    for uuid in gone {
        deleted += 1;
        send(state, &delete_envelope(uuid));
    }
    *published = next;
    state.metrics.ticks.inc();
    state.metrics.systems_alive.set(published.len() as i64);
    (created, updated, deleted)
}

/// Re-send everything to a bridge that (re)connected: creates only (idempotent in Horizon: a system
/// it already knows is kept, one it lost is made), and every system is marked due for an update at
/// the next tick. An update sent right behind the create was too early: a freshly booted Horizon
/// queues the create until the parent planet exists, and answered "Unknown props uuid" (231 times
/// on 2026-10-06).
pub fn replay(state: &Shared) -> usize {
    let mut published = state.published.lock().unwrap();
    for (uuid, (data, sent_at)) in published.iter_mut() {
        send(state, &create_envelope(*uuid, data));
        *sent_at = f64::NEG_INFINITY;
    }
    published.len()
}

fn send(state: &Shared, env: &ds_weather_proto::BridgeEnvelope) {
    if let Ok(text) = serde_json::to_string(env) {
        // No receiver = no bridge connected yet; the next replay catches up.
        let _ = state.tx.send(text);
        state.metrics.envelopes_sent.inc();
    }
}

pub async fn run(state: Shared) {
    let period = Duration::from_secs_f64(state.config.tick_s.max(1.0));
    let mut interval = tokio::time::interval(period);
    let mut without_uuids = 0u32;
    loop {
        interval.tick().await;
        if state.planet_uuids.read().map(|m| m.is_empty()).unwrap_or(true) {
            without_uuids += 1;
            if without_uuids % 4 == 1 {
                if let Err(e) = state.refresh_planet_uuids().await {
                    info!(error = %e, "celestial service not reachable yet: no planet uuids, nothing published");
                }
            }
            if state.planet_uuids.read().map(|m| m.is_empty()).unwrap_or(true) {
                continue;
            }
        }
        let t = now_unix();
        let (c, u, d) = tick(&state, t);
        debug!(t, created = c, updated = u, deleted = d, "tick");
        if c + d > 0 {
            info!(created = c, deleted = d, alive = u + c, "systems changed");
        }
    }
}
