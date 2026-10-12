use axum::{http::{header, StatusCode}, response::IntoResponse, Json};
use serde_json::json;

pub async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

pub async fn openapi() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], include_str!("../../openapi.yaml"))
}
