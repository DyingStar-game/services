//! Keycloak: the service account (`svc-npc`) that manages the NPC users, and
//! the per-NPC logins (password grant on `npc-login`) whose access tokens the
//! NPCs present to Horizon.

pub mod admin;
pub mod service_token;
pub mod tokens;

pub use admin::KcAdmin;
pub use service_token::ServiceToken;
pub use tokens::TokenManager;
