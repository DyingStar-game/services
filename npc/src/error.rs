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

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Horizon protocol error: {0}")]
    Protocol(String),

    #[error("NPC '{0}' already exists")]
    AlreadyExists(String),

    #[error("{0} not found")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Conflict(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    /// A call to Keycloak or universesimulator failed.
    #[error("{service} error ({status}): {message}")]
    Upstream {
        service: &'static str,
        status: u16,
        message: String,
    },
}

impl NpcError {
    pub fn upstream(service: &'static str, status: u16, message: impl Into<String>) -> Self {
        Self::Upstream { service, status, message: message.into() }
    }
}

impl IntoResponse for NpcError {
    fn into_response(self) -> Response {
        let status = match &self {
            NpcError::NotFound(_) => StatusCode::NOT_FOUND,
            NpcError::AlreadyExists(_) | NpcError::Conflict(_) => StatusCode::CONFLICT,
            NpcError::Json(_) | NpcError::Yaml(_) | NpcError::Config(_) | NpcError::BadRequest(_) => {
                StatusCode::BAD_REQUEST
            }
            NpcError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            NpcError::Forbidden(_) => StatusCode::FORBIDDEN,
            NpcError::Upstream { .. } => StatusCode::BAD_GATEWAY,
            // Constraint violations are the caller's doing: duplicate name or
            // storekeeper slot (409), unknown pattern or inconsistent role (400).
            NpcError::Db(sqlx::Error::Database(e)) => match e.code().as_deref() {
                Some("23505") => StatusCode::CONFLICT,
                Some("23503") | Some("23514") => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }
        // Same shape as the other game services: `{ "error": "<CODE>", "message": "<text>" }`.
        let code = match (&self, status) {
            (NpcError::Upstream { .. }, _) => "UPSTREAM_ERROR",
            (_, StatusCode::NOT_FOUND) => "NOT_FOUND",
            (_, StatusCode::CONFLICT) => "CONFLICT",
            (_, StatusCode::BAD_REQUEST) => "BAD_REQUEST",
            (_, StatusCode::UNAUTHORIZED) => "UNAUTHORIZED",
            (_, StatusCode::FORBIDDEN) => "FORBIDDEN",
            _ => "INTERNAL_ERROR",
        };
        (status, Json(json!({ "error": code, "message": self.to_string() }))).into_response()
    }
}

pub type Result<T> = std::result::Result<T, NpcError>;
