//! Inbound authentication, ported from the TypeScript game services (ADR 0011):
//! player tokens, moderation roles, service-account tokens and dev bypasses.

mod jwks;

use std::collections::HashSet;

use axum::{
    extract::{Request, State},
    http::HeaderMap,
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use serde::Deserialize;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::config::Config;
use crate::error::{AppError, Result};
use crate::state::AppState;

pub use jwks::Jwks;

/// Service capability roles (`usim:*`).
pub const ROLE_READ: &str = "usim:read";
pub const ROLE_WRITE: &str = "usim:write";
pub const ROLE_DELIVER: &str = "usim:deliver";

/// Moderation roles, lowest first.
const MODERATION: [&str; 3] = ["moderator", "admin", "supervisor"];

/// The authenticated caller, inserted in the request extensions by the auth middlewares.
#[derive(Debug, Clone)]
pub enum Caller {
    Player { id: Uuid, roles: HashSet<String> },
    Service { client_id: String, roles: HashSet<String>, all_roles: bool },
}

impl Caller {
    pub fn label(&self) -> String {
        match self {
            Self::Player { id, .. } => format!("player:{id}"),
            Self::Service { client_id, .. } => format!("service:{client_id}"),
        }
    }

    pub fn require_role(&self, role: &str) -> Result<()> {
        match self {
            Self::Service { roles, all_roles, .. } if *all_roles || roles.contains(role) => Ok(()),
            _ => Err(AppError::Forbidden(format!("role {role} required"))),
        }
    }

    fn moderation_level(&self) -> Option<usize> {
        match self {
            Self::Player { roles, .. } => MODERATION.iter().rposition(|r| roles.contains(*r)),
            Self::Service { .. } => None,
        }
    }

    /// `admin` or `supervisor`: required for every admin write.
    pub fn require_admin(&self) -> Result<()> {
        match self.moderation_level() {
            Some(l) if l >= 1 => Ok(()),
            _ => Err(AppError::Forbidden("admin role required".into())),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: Option<String>,
    azp: Option<String>,
    client_id: Option<String>,
    preferred_username: Option<String>,
    #[serde(default)]
    realm_access: RoleSet,
    #[serde(default)]
    resource_access: std::collections::HashMap<String, RoleSet>,
}

#[derive(Debug, Default, Deserialize)]
struct RoleSet {
    #[serde(default)]
    roles: Vec<String>,
}

pub struct Auth {
    jwks: Jwks,
    issuer: String,
    audience: Option<String>,
    service_audience: String,
    service_clients: HashSet<String>,
    internal_key: Option<String>,
    player_bypass: bool,
    internal_bypass: bool,
}

impl Auth {
    pub fn new(cfg: &Config) -> Self {
        Self {
            jwks: Jwks::new(cfg.oidc_jwks_url.clone(), reqwest::Client::new()),
            issuer: cfg.oidc_issuer.clone(),
            audience: cfg.oidc_audience.clone(),
            service_audience: cfg.oidc_service_audience.clone(),
            service_clients: cfg.internal_service_clients.iter().cloned().collect(),
            internal_key: cfg.internal_api_key.clone(),
            player_bypass: cfg.auth_dev_bypass && !cfg.is_production(),
            internal_bypass: cfg.internal_dev_bypass && !cfg.is_production(),
        }
    }

    async fn verify(&self, token: &str, audience: Option<&str>) -> Result<Claims> {
        let header = decode_header(token).map_err(|_| AppError::Unauthorized("malformed token".into()))?;
        let kid = header.kid.ok_or_else(|| AppError::Unauthorized("token without kid".into()))?;
        let key = self.jwks.key(&kid).await?;
        let mut v = Validation::new(Algorithm::RS256);
        v.set_issuer(&[&self.issuer]);
        match audience {
            Some(aud) => v.set_audience(&[aud]),
            None => v.validate_aud = false,
        }
        decode::<Claims>(token, &key, &v)
            .map(|d| d.claims)
            .map_err(|e| AppError::Unauthorized(format!("invalid token: {e}")))
    }

    pub async fn player(&self, headers: &HeaderMap) -> Result<Caller> {
        if self.player_bypass {
            if let Some(id) = header(headers, "x-player-id") {
                let id = Uuid::parse_str(id).map_err(|_| AppError::Unauthorized("X-Player-Id must be a UUID".into()))?;
                let roles = header(headers, "x-player-roles")
                    .map(|r| r.split(',').map(|s| s.trim().to_string()).collect())
                    .unwrap_or_default();
                return Ok(Caller::Player { id, roles });
            }
        }
        let token = bearer(headers)?;
        let claims = self.verify(token, self.audience.as_deref()).await?;
        let id = claims
            .sub
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| AppError::Unauthorized("sub must be a UUID".into()))?;
        Ok(Caller::Player { id, roles: claims.realm_access.roles.into_iter().collect() })
    }

    pub async fn service(&self, headers: &HeaderMap) -> Result<Caller> {
        if self.internal_bypass {
            if let (Some(given), Some(expected)) = (header(headers, "x-internal-key"), &self.internal_key) {
                if bool::from(given.as_bytes().ct_eq(expected.as_bytes())) {
                    return Ok(Caller::Service { client_id: "dev-bypass".into(), roles: HashSet::new(), all_roles: true });
                }
                return Err(AppError::Unauthorized("invalid internal key".into()));
            }
        }
        let token = bearer(headers)?;
        let mut claims = self.verify(token, Some(&self.service_audience)).await?;
        let client_id = claims.azp.clone().or(claims.client_id.clone()).unwrap_or_default();
        if !self.service_clients.contains(&client_id) {
            return Err(AppError::Forbidden(format!("service client {client_id} not allowed")));
        }
        if claims.sub.as_deref().and_then(|s| Uuid::parse_str(s).ok()).is_none() {
            return Err(AppError::Unauthorized("sub must be a UUID".into()));
        }
        if claims.preferred_username.as_deref().is_some_and(|u| !u.starts_with("service-account-")) {
            return Err(AppError::Forbidden("not a service account".into()));
        }
        let mut roles: HashSet<String> = std::mem::take(&mut claims.realm_access.roles).into_iter().collect();
        if let Some(r) = claims.resource_access.remove(&self.service_audience) {
            roles.extend(r.roles);
        }
        Ok(Caller::Service { client_id, roles, all_roles: false })
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn bearer(headers: &HeaderMap) -> Result<&str> {
    header(headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("bearer token required".into()))
}

// ── Middlewares ──────────────────────────────────────────────────────────────

/// `/api/*`: any authenticated player.
pub async fn player_layer(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    match st.auth.player(req.headers()).await {
        Ok(caller) => {
            req.extensions_mut().insert(caller);
            next.run(req).await
        }
        Err(e) => e.into_response(),
    }
}

/// `/api/admin/*`: a player with a moderation role. Writes also call [`Caller::require_admin`].
pub async fn admin_layer(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let caller = match st.auth.player(req.headers()).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    if caller.moderation_level().is_none() {
        return AppError::Forbidden("moderation role required".into()).into_response();
    }
    req.extensions_mut().insert(caller);
    next.run(req).await
}

/// `/api/internal/*`: a service account with at least `usim:read`; handlers check write roles.
pub async fn service_layer(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let caller = match st.auth.service(req.headers()).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    if req.method() == axum::http::Method::GET {
        if let Err(e) = caller.require_role(ROLE_READ) {
            return e.into_response();
        }
    }
    req.extensions_mut().insert(caller);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(roles: &[&str]) -> Caller {
        Caller::Player { id: Uuid::new_v4(), roles: roles.iter().map(|r| r.to_string()).collect() }
    }

    #[test]
    fn moderation_hierarchy() {
        assert!(player(&["moderator"]).require_admin().is_err());
        assert!(player(&["admin"]).require_admin().is_ok());
        assert!(player(&["supervisor"]).require_admin().is_ok());
        assert!(player(&[]).moderation_level().is_none());
    }

    #[test]
    fn service_roles() {
        let s = Caller::Service { client_id: "svc-game".into(), roles: [ROLE_READ.to_string()].into(), all_roles: false };
        assert!(s.require_role(ROLE_READ).is_ok());
        assert!(s.require_role(ROLE_DELIVER).is_err());
        assert!(player(&["admin"]).require_role(ROLE_READ).is_err());
    }
}
