//! Authentication of the admin API callers, ported from universesimulator
//! (ADR 0011): Keycloak tokens of staff members, forwarded by the adminpanel
//! BFF. Reads need a moderation role, writes `admin` or `supervisor`.

mod jwks;

use std::collections::HashSet;

use axum::{
    extract::{Request, State},
    http::HeaderMap,
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    config::Config,
    error::{NpcError, Result},
    state::AppState,
};

pub use jwks::Jwks;

/// Moderation roles, lowest first.
const MODERATION: [&str; 3] = ["moderator", "admin", "supervisor"];

/// The authenticated staff member, inserted in the request extensions.
#[derive(Debug, Clone)]
pub struct Caller {
    pub id: Uuid,
    pub username: Option<String>,
    pub roles: HashSet<String>,
}

impl Caller {
    /// Who to record in `created_by` / `updated_by`.
    pub fn label(&self) -> String {
        self.username.clone().unwrap_or_else(|| self.id.to_string())
    }

    fn moderation_level(&self) -> Option<usize> {
        MODERATION.iter().rposition(|r| self.roles.contains(*r))
    }

    /// `admin` or `supervisor`: required for every write.
    pub fn require_admin(&self) -> Result<()> {
        match self.moderation_level() {
            Some(l) if l >= 1 => Ok(()),
            _ => Err(NpcError::Forbidden("admin role required".into())),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: Option<String>,
    preferred_username: Option<String>,
    #[serde(default)]
    realm_access: RoleSet,
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
    bypass: bool,
}

impl Auth {
    pub fn new(cfg: &Config, http: reqwest::Client) -> Self {
        Self {
            jwks: Jwks::new(cfg.oidc_jwks_url.clone(), http),
            issuer: cfg.oidc_issuer.clone(),
            audience: cfg.oidc_audience.clone(),
            bypass: cfg.auth_dev_bypass && !cfg.is_production(),
        }
    }

    async fn caller(&self, headers: &HeaderMap) -> Result<Caller> {
        if self.bypass {
            if let Some(id) = header(headers, "x-player-id") {
                let id = Uuid::parse_str(id)
                    .map_err(|_| NpcError::Unauthorized("X-Player-Id must be a UUID".into()))?;
                let roles = header(headers, "x-player-roles")
                    .map(|r| r.split(',').map(|s| s.trim().to_string()).collect())
                    .unwrap_or_default();
                return Ok(Caller { id, username: Some("dev-bypass".into()), roles });
            }
        }
        let token = bearer(headers)?;
        let header = decode_header(token).map_err(|_| NpcError::Unauthorized("malformed token".into()))?;
        let kid = header.kid.ok_or_else(|| NpcError::Unauthorized("token without kid".into()))?;
        let key = self.jwks.key(&kid).await?;
        let mut v = Validation::new(Algorithm::RS256);
        v.set_issuer(&[&self.issuer]);
        match &self.audience {
            Some(aud) => v.set_audience(&[aud]),
            None => v.validate_aud = false,
        }
        let claims = decode::<Claims>(token, &key, &v)
            .map(|d| d.claims)
            .map_err(|e| NpcError::Unauthorized(format!("invalid token: {e}")))?;
        let id = claims
            .sub
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| NpcError::Unauthorized("sub must be a UUID".into()))?;
        Ok(Caller {
            id,
            username: claims.preferred_username,
            roles: claims.realm_access.roles.into_iter().collect(),
        })
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn bearer(headers: &HeaderMap) -> Result<&str> {
    header(headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| NpcError::Unauthorized("bearer token required".into()))
}

/// `/api/admin/*`: a staff member with a moderation role. Writes also call
/// [`Caller::require_admin`].
pub async fn admin_layer(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let caller = match st.auth.caller(req.headers()).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    if caller.moderation_level().is_none() {
        return NpcError::Forbidden("moderation role required".into()).into_response();
    }
    req.extensions_mut().insert(caller);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(roles: &[&str]) -> Caller {
        Caller { id: Uuid::new_v4(), username: None, roles: roles.iter().map(|r| r.to_string()).collect() }
    }

    #[test]
    fn moderation_hierarchy() {
        assert!(caller(&["moderator"]).require_admin().is_err());
        assert!(caller(&["admin"]).require_admin().is_ok());
        assert!(caller(&["supervisor"]).require_admin().is_ok());
        assert!(caller(&["player", "npc"]).moderation_level().is_none());
    }
}
