//! Service-account token for outbound calls (client credentials), cached until
//! 30 s before expiry. Same scheme as universesimulator's `clients/token.rs`.

use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex;

use crate::error::{NpcError, Result};

pub struct ServiceToken {
    http: reqwest::Client,
    token_url: String,
    client_id: String,
    client_secret: String,
    cache: Mutex<Option<(String, Instant)>>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

impl ServiceToken {
    pub fn new(http: reqwest::Client, token_url: String, client_id: String, client_secret: String) -> Self {
        Self { http, token_url, client_id, client_secret, cache: Mutex::new(None) }
    }

    pub fn configured(&self) -> bool {
        !self.client_secret.is_empty()
    }

    pub async fn bearer(&self) -> Result<String> {
        if !self.configured() {
            return Err(NpcError::Config("SERVICE_CLIENT_SECRET is not set".into()));
        }
        let mut cache = self.cache.lock().await;
        if let Some((token, expires)) = cache.as_ref() {
            if Instant::now() < *expires {
                return Ok(token.clone());
            }
        }
        let res = self
            .http
            .post(&self.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|e| NpcError::upstream("keycloak", 0, e.to_string()))?;
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            return Err(NpcError::upstream("keycloak", status, format!("service token refused: {body}")));
        }
        let body: TokenResponse =
            res.json().await.map_err(|e| NpcError::upstream("keycloak", 0, e.to_string()))?;
        let ttl = body.expires_in.unwrap_or(60).saturating_sub(30).max(30);
        *cache = Some((body.access_token.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(body.access_token)
    }

    /// Forget the cached token (after a 401).
    pub async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }
}
