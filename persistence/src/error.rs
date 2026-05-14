use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

/// Application-level errors.
///
/// Most layers of the application return `anyhow::Result` directly for
/// convenience.  This enum is only used where we need structured matching
/// (e.g. the WebSocket and REST handler layers).
#[derive(Debug, Error)]
pub enum AppError {
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Database(e.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Json(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            AppError::Database(msg) | AppError::WebSocket(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
