//! Per-NPC Keycloak logins. Each NPC holds an access token (presented to
//! Horizon at every connection) and a refresh token; the password grant only
//! runs when both are gone, so reconnects do not pile up Keycloak sessions.
//!
//! A password Keycloak no longer accepts (expired by the realm policy, or out
//! of step with the database) is repaired once: reset through the admin API,
//! stored, retried. Repeated failures back off (up to 10 min) so the realm's
//! brute-force protection is never tripped.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

use super::KcAdmin;
use crate::{
    crypto::{PasswordBox, random_password},
    db,
    error::{NpcError, Result},
    events::EventSink,
};

/// Reuse an access token while it has at least this much life left (or half
/// its lifetime, for short-lived tokens).
const MIN_REMAINING: Duration = Duration::from_secs(600);
const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(600);

#[derive(Default)]
struct Entry {
    access: Option<(String, Instant)>,
    refresh: Option<(String, Instant)>,
    failures: u32,
    retry_at: Option<Instant>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    refresh_expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct TokenError {
    error: Option<String>,
    error_description: Option<String>,
}

enum GrantError {
    /// Keycloak refused the credentials (`invalid_grant`).
    Rejected(String),
    /// Anything else: network, 5xx, misconfigured client.
    Other(NpcError),
}

pub struct TokenManager {
    http: reqwest::Client,
    token_url: String,
    logout_url: String,
    client_id: String,
    client_secret: String,
    pool: PgPool,
    passwords: PasswordBox,
    kc: Arc<KcAdmin>,
    events: EventSink,
    cache: DashMap<Uuid, Arc<Mutex<Entry>>>,
}

impl TokenManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http: reqwest::Client,
        token_url: String,
        client_id: String,
        client_secret: String,
        pool: PgPool,
        passwords: PasswordBox,
        kc: Arc<KcAdmin>,
        events: EventSink,
    ) -> Self {
        let logout_url = token_url.replace("/token", "/logout");
        Self {
            http,
            token_url,
            logout_url,
            client_id,
            client_secret,
            pool,
            passwords,
            kc,
            events,
            cache: DashMap::new(),
        }
    }

    /// A valid access token for NPC `id` (Keycloak user `login`).
    pub async fn token_for(&self, id: Uuid, login: &str) -> Result<String> {
        let entry = Arc::clone(&self.cache.entry(id).or_default());
        let mut e = entry.lock().await;
        let now = Instant::now();

        if let Some((token, exp)) = &e.access {
            if *exp > now + MIN_REMAINING {
                return Ok(token.clone());
            }
        }
        if let Some((refresh, exp)) = e.refresh.take() {
            if exp > now {
                match self.grant(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)]).await {
                    Ok(t) => return Ok(store(&mut e, t)),
                    Err(_) => info!(npc = %login, "refresh token refused; logging in again"),
                }
            }
        }
        if let Some(at) = e.retry_at {
            if at > now {
                return Err(NpcError::upstream(
                    "keycloak",
                    0,
                    format!("login of {login} backing off for {}s", (at - now).as_secs()),
                ));
            }
        }

        match self.password_login(id, login).await {
            Ok(t) => {
                let was_failing = e.failures > 0;
                e.failures = 0;
                e.retry_at = None;
                if was_failing {
                    let _ = db::npcs::set_kc_state(&self.pool, id, "ready", None).await;
                }
                Ok(store(&mut e, t))
            }
            Err(err) => {
                e.failures += 1;
                let backoff = BACKOFF_BASE.saturating_mul(1 << e.failures.min(5)).min(BACKOFF_MAX);
                e.retry_at = Some(Instant::now() + backoff);
                Err(err)
            }
        }
    }

    /// Password grant, repairing the password once if Keycloak rejects it.
    async fn password_login(&self, id: Uuid, login: &str) -> Result<TokenResponse> {
        let row = db::npcs::require(&self.pool, id).await?;
        let password = self.passwords.open(id, &row.password_enc)?;
        match self.grant(&password_form(login, &password)).await {
            Ok(t) => return Ok(t),
            Err(GrantError::Other(e)) => return Err(e),
            Err(GrantError::Rejected(reason)) => {
                warn!(npc = %login, %reason, "Keycloak rejected the stored password; resetting it");
            }
        }

        let fresh = random_password();
        if let Err(e) = self.rotate(id, &fresh).await {
            self.mark_error(id, &format!("password reset failed: {e}")).await;
            return Err(e);
        }
        self.events.warn(id, "credentials_rotated", "password rejected by Keycloak; reset", None);
        match self.grant(&password_form(login, &fresh)).await {
            Ok(t) => Ok(t),
            Err(GrantError::Rejected(reason)) => {
                self.mark_error(id, &format!("login refused after reset: {reason}")).await;
                Err(NpcError::upstream("keycloak", 400, reason))
            }
            Err(GrantError::Other(e)) => Err(e),
        }
    }

    /// Set a new password in Keycloak, then in the database.
    pub async fn rotate(&self, id: Uuid, password: &str) -> Result<()> {
        self.kc.reset_password(id, password).await?;
        let sealed = self.passwords.seal(id, password)?;
        db::npcs::set_password(&self.pool, id, &sealed).await?;
        self.cache.remove(&id);
        Ok(())
    }

    async fn mark_error(&self, id: Uuid, error: &str) {
        let _ = db::npcs::set_kc_state(&self.pool, id, "error", Some(error)).await;
        self.events.emit(Some(id), "login_failed", "error", error.to_string(), Some(json!({ "error": error })));
    }

    async fn grant(&self, form: &[(&str, &str)]) -> std::result::Result<TokenResponse, GrantError> {
        let mut full = vec![("client_id", self.client_id.as_str()), ("client_secret", self.client_secret.as_str())];
        full.extend_from_slice(form);
        let res = self
            .http
            .post(&self.token_url)
            .form(&full)
            .send()
            .await
            .map_err(|e| GrantError::Other(NpcError::upstream("keycloak", 0, e.to_string())))?;
        let status = res.status();
        if status.is_success() {
            return res
                .json()
                .await
                .map_err(|e| GrantError::Other(NpcError::upstream("keycloak", 0, e.to_string())));
        }
        let body: TokenError = res.json().await.unwrap_or(TokenError { error: None, error_description: None });
        let reason = format!(
            "{}: {}",
            body.error.as_deref().unwrap_or("error"),
            body.error_description.as_deref().unwrap_or("")
        );
        if status.as_u16() == 400 && body.error.as_deref() == Some("invalid_grant") {
            Err(GrantError::Rejected(reason))
        } else {
            Err(GrantError::Other(NpcError::upstream("keycloak", status.as_u16(), reason)))
        }
    }

    /// Drop the cached tokens of an NPC and end its Keycloak session (best
    /// effort, on despawn and delete).
    pub async fn forget(&self, id: Uuid) {
        let Some((_, entry)) = self.cache.remove(&id) else { return };
        let refresh = entry.lock().await.refresh.take();
        if let Some((refresh, _)) = refresh {
            let _ = self
                .http
                .post(&self.logout_url)
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("client_secret", self.client_secret.as_str()),
                    ("refresh_token", refresh.as_str()),
                ])
                .send()
                .await;
        }
    }
}

/// No `scope`: `npc-login` deliberately lacks the `openid`/`profile` client
/// scopes (their username mapper would override `preferred_username`); its own
/// mappers put `sub`, the display name and the realm roles in the token.
fn password_form<'a>(login: &'a str, password: &'a str) -> [(&'a str, &'a str); 3] {
    [("grant_type", "password"), ("username", login), ("password", password)]
}

/// Cache a fresh token pair, returning the access token.
fn store(e: &mut Entry, t: TokenResponse) -> String {
    let now = Instant::now();
    let life = Duration::from_secs(t.expires_in.unwrap_or(300));
    // A short-lived token is still reused for half its life.
    let exp = now + life;
    let reuse_until = if life > MIN_REMAINING * 2 { exp } else { now + life / 2 + MIN_REMAINING };
    e.access = Some((t.access_token.clone(), reuse_until));
    e.refresh = t
        .refresh_token
        .map(|r| (r, now + Duration::from_secs(t.refresh_expires_in.unwrap_or(0))));
    t.access_token
}
