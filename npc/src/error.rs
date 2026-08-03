use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NpcError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Horizon protocol error: {0}")]
    Protocol(String),

    #[error("NPC '{0}' already exists")]
    AlreadyExists(String),

    #[error("NPC '{0}' not found")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

impl IntoResponse for NpcError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            NpcError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            NpcError::AlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
            NpcError::Json(_) | NpcError::Yaml(_) | NpcError::Config(_) => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type Result<T> = std::result::Result<T, NpcError>;
