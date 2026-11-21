use std::sync::Arc;

use crate::{auth::Auth, npc::manager::NpcManager};

/// Shared state of the admin API.
#[derive(Clone)]
pub struct AppState {
    pub mgr: Arc<NpcManager>,
    pub auth: Arc<Auth>,
}
