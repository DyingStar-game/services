use std::sync::Arc;

use axum::extract::FromRef;
use sqlx::PgPool;

use crate::auth::Auth;
use crate::clients::Clients;
use crate::config::Config;

/// State shared across all handlers.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: Arc<Config>,
    pub clients: Clients,
    pub auth: Arc<Auth>,
}

impl FromRef<AppState> for PgPool {
    fn from_ref(state: &AppState) -> PgPool {
        state.pool.clone()
    }
}
