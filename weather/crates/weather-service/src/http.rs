//! The HTTP side: liveness, Prometheus, and the map API that the admin panel, the website and the
//! game's star map draw from (every instant, past or future, is a plain function).

use crate::sim::{now_unix, snapshot};
use crate::state::Shared;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use prometheus::{Encoder, TextEncoder};
use serde::Deserialize;
use serde_json::json;
use weather_core::sampler::sample;
use weather_core::spin::{declination_deg, substellar_point, local_solar_hours};
use weather_core::synoptic::systems_at;

pub async fn health() -> &'static str {
    "ok"
}

pub async fn metrics(State(state): State<Shared>) -> impl IntoResponse {
    let mut buf = Vec::new();
    let encoder = TextEncoder::new();
    let _ = encoder.encode(&state.metrics.registry.gather(), &mut buf);
    let _ = encoder.encode(&prometheus::gather(), &mut buf);
    ([(axum::http::header::CONTENT_TYPE, encoder.format_type().to_owned())], buf)
}

#[derive(Deserialize)]
pub struct TimeQuery {
    pub t: Option<f64>,
}

#[derive(Deserialize)]
pub struct PointQuery {
    pub lon: f64,
    pub lat: f64,
    #[serde(default)]
    pub alt: f64,
    pub t: Option<f64>,
}

#[derive(Deserialize)]
pub struct FieldsQuery {
    pub t: Option<f64>,
    #[serde(default = "default_layer")]
    pub layer: String,
    #[serde(default = "default_w")]
    pub w: usize,
    #[serde(default = "default_h")]
    pub h: usize,
}

fn default_layer() -> String {
    "pressure".into()
}
fn default_w() -> usize {
    180
}
fn default_h() -> usize {
    90
}

#[derive(Deserialize)]
pub struct ForecastQuery {
    pub t0: Option<f64>,
    pub t1: Option<f64>,
    #[serde(default = "default_step")]
    pub step: f64,
}

fn default_step() -> f64 {
    3600.0
}

pub async fn bodies(State(state): State<Shared>) -> Json<serde_json::Value> {
    let list: Vec<_> = state
        .bodies
        .iter()
        .map(|b| {
            let c = &b.climate;
            json!({
                "name": b.inputs.name, "kind": b.inputs.kind, "horizon_uuid": state.planet_uuid(&b.inputs.name),
                "has_atmosphere": c.has_atmosphere, "pressure_pa": c.pressure_pa, "t_surface_k": c.t_surface_k,
                "scale_height_m": c.scale_height_m, "rho0_kg_m3": c.rho0_kg_m3, "hadley_edge_deg": c.hadley_edge_deg,
                "cells": c.cells, "v10_scale_m_s": c.v10_scale_m_s, "v10_threshold_m_s": c.v10_threshold_m_s,
                "rotation_period_s": c.rotation_period_s, "tilt_deg": c.tilt_deg, "seed": c.seed,
            })
        })
        .collect();
    Json(json!({ "system": state.config.system_name, "bodies": list }))
}

pub async fn climate(State(state): State<Shared>, Path(body): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let b = state.body(&body).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::to_value(&b.climate).unwrap_or_default()))
}

pub async fn systems(State(state): State<Shared>, Path(body): Path<String>, Query(q): Query<TimeQuery>) -> Result<Json<serde_json::Value>, StatusCode> {
    let b = state.body(&body).ok_or(StatusCode::NOT_FOUND)?;
    let t = q.t.unwrap_or_else(now_unix);
    let decl = declination_deg(b.inputs.tilt_deg.to_radians(), state.to_star(b, t));
    let list = systems_at(&b.climate, t, decl);
    let objects = snapshot(&state, b, t);
    Ok(Json(json!({ "body": b.inputs.name, "t": t, "declination_deg": decl, "count": list.len(), "systems": list,
        "objects": objects.iter().map(|(u, d)| json!({"uuid": u.to_string(), "name": d.name, "position": d.position})).collect::<Vec<_>>() })))
}

pub async fn point(State(state): State<Shared>, Path(body): Path<String>, Query(q): Query<PointQuery>) -> Result<Json<serde_json::Value>, StatusCode> {
    let b = state.body(&body).ok_or(StatusCode::NOT_FOUND)?;
    let t = q.t.unwrap_or_else(now_unix);
    let to_star = state.to_star(b, t);
    let decl = declination_deg(b.inputs.tilt_deg.to_radians(), to_star);
    let (sub_lon, _) = substellar_point(b.inputs.tilt_deg.to_radians(), b.inputs.rotation_period_s, t, to_star);
    let hours = local_solar_hours(q.lon, sub_lon);
    let list = systems_at(&b.climate, t, decl);
    let s = sample(&b.climate, &list, q.lon, q.lat, q.alt, hours, decl);
    Ok(Json(json!({ "body": b.inputs.name, "t": t, "sample": s })))
}

/// One layer on an equirectangular grid (w × h, longitude −180 → 180 left to right, latitude
/// 90 → −90 top to bottom), as a flat array row by row.
pub async fn fields(State(state): State<Shared>, Path(body): Path<String>, Query(q): Query<FieldsQuery>) -> Result<Json<serde_json::Value>, StatusCode> {
    let b = state.body(&body).ok_or(StatusCode::NOT_FOUND)?;
    let (w, h) = (q.w.clamp(4, 720), q.h.clamp(2, 360));
    let t = q.t.unwrap_or_else(now_unix);
    let to_star = state.to_star(b, t);
    let decl = declination_deg(b.inputs.tilt_deg.to_radians(), to_star);
    let (sub_lon, _) = substellar_point(b.inputs.tilt_deg.to_radians(), b.inputs.rotation_period_s, t, to_star);
    let list = systems_at(&b.climate, t, decl);
    let mut values = Vec::with_capacity(w * h);
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for j in 0..h {
        let lat = 90.0 - (j as f64 + 0.5) * 180.0 / h as f64;
        for i in 0..w {
            let lon = -180.0 + (i as f64 + 0.5) * 360.0 / w as f64;
            let s = sample(&b.climate, &list, lon, lat, 0.0, local_solar_hours(lon, sub_lon), decl);
            let v = match q.layer.as_str() {
                "pressure" => s.pressure_pa,
                "pressure_anomaly" => s.pressure_anomaly_pa,
                "wind_speed" => s.wind_speed_m_s,
                "wind_east" | "wind_u" => s.wind_east_m_s,
                "wind_north" | "wind_v" => s.wind_north_m_s,
                "temp_air" | "temperature" => s.temperature_k,
                "dust" | "dust_tau" => s.dust_tau,
                "visibility" => s.visibility_m,
                "humidity" => s.humidity,
                "precipitation" | "rain" => s.precipitation_mm_h,
                "air_density" => s.air_density_kg_m3,
                "solar_hours" => s.local_solar_hours,
                _ => return Err(StatusCode::BAD_REQUEST),
            };
            min = min.min(v);
            max = max.max(v);
            values.push(v);
        }
    }
    Ok(Json(json!({ "body": b.inputs.name, "t": t, "layer": q.layer, "w": w, "h": h, "min": min, "max": max, "declination_deg": decl, "substellar_lon_deg": sub_lon, "values": values })))
}

/// The systems at every `step` between t0 and t1 (compact: centre, kind, intensity).
pub async fn forecast(State(state): State<Shared>, Path(body): Path<String>, Query(q): Query<ForecastQuery>) -> Result<Json<serde_json::Value>, StatusCode> {
    let b = state.body(&body).ok_or(StatusCode::NOT_FOUND)?;
    let t0 = q.t0.unwrap_or_else(now_unix);
    let t1 = q.t1.unwrap_or(t0 + 86_400.0);
    let step = q.step.max(60.0);
    if (t1 - t0) / step > 2000.0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut frames = Vec::new();
    let mut t = t0;
    while t <= t1 {
        let decl = declination_deg(b.inputs.tilt_deg.to_radians(), state.to_star(b, t));
        let list: Vec<_> = systems_at(&b.climate, t, decl)
            .into_iter()
            .map(|s| json!({"id": s.id, "kind": s.kind, "lon": s.centre_lon_deg, "lat": s.centre_lat_deg, "sigma_m": s.sigma_m, "intensity": s.intensity, "dp_pa": s.pressure_anomaly_pa}))
            .collect();
        frames.push(json!({ "t": t, "systems": list }));
        t += step;
    }
    Ok(Json(json!({ "body": b.inputs.name, "t0": t0, "t1": t1, "step": step, "frames": frames })))
}
