use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type Result<T> = std::result::Result<T, AppError>;

/// Error returned by handlers and services. Serialised like the TypeScript game services:
/// `{ "error": "<CODE>", "message": "<text>" }`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("database migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("{0}")]
    NotFound(String),

    #[error("{1}")]
    Conflict(&'static str, String),

    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Unauthorized(String),

    #[error("{0}")]
    Forbidden(String),

    /// A call to another service failed. `status` is the upstream HTTP status (0 = transport).
    #[error("{service} returned {status} {code}: {message}")]
    Upstream {
        service: &'static str,
        status: u16,
        code: String,
        message: String,
    },
}

impl AppError {
    pub fn conflict(code: &'static str, msg: impl Into<String>) -> Self {
        Self::Conflict(code, msg.into())
    }

    /// Upstream error code, when this is an upstream error.
    pub fn upstream_code(&self) -> Option<&str> {
        match self {
            Self::Upstream { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn upstream_status(&self) -> Option<u16> {
        match self {
            Self::Upstream { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl From<sqlx::types::uuid::Error> for AppError {
    fn from(e: sqlx::types::uuid::Error) -> Self {
        Self::BadRequest(e.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            Self::Conflict(code, _) => (StatusCode::CONFLICT, *code),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            Self::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            Self::Upstream { status, .. } if *status == 404 => (StatusCode::NOT_FOUND, "UPSTREAM_NOT_FOUND"),
            Self::Upstream { status, .. } if *status == 409 => (StatusCode::CONFLICT, "UPSTREAM_CONFLICT"),
            Self::Upstream { .. } => (StatusCode::BAD_GATEWAY, "UPSTREAM_ERROR"),
            Self::Db(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                (StatusCode::CONFLICT, "ALREADY_EXISTS")
            }
            Self::Db(sqlx::Error::Database(db)) if db.is_foreign_key_violation() || db.is_check_violation() => {
                (StatusCode::BAD_REQUEST, "INVALID_REFERENCE")
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }
        // Upstream errors keep the upstream code so callers can react to it.
        let code = match &self {
            Self::Upstream { code, status, .. } if *status == 409 || *status == 404 => code.clone(),
            _ => code.to_string(),
        };
        (status, Json(json!({ "error": code, "message": self.to_string() }))).into_response()
    }
}
