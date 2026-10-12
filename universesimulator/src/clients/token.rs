//! Service-account token for outbound calls (client credentials, ADR 0011).
//! Port of `mission/src/services/economy.client.ts`: the token is cached until 30 s before
//! expiry; without a secret, dev falls back to the `X-Internal-Key` header.

use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex;

use crate::error::{AppError, Result};

pub enum AuthHeader {
    Bearer(String),
    InternalKey(String),
}

pub struct TokenProvider {
    http: reqwest::Client,
    token_url: String,
    client_id: String,
    client_secret: Option<String>,
    internal_key: Option<String>,
    cache: Mutex<Option<(String, Instant)>>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

impl TokenProvider {
    pub fn new(
        http: reqwest::Client,
        token_url: String,
        client_id: String,
        client_secret: Option<String>,
        internal_key: Option<String>,
    ) -> Self {
        Self { http, token_url, client_id, client_secret, internal_key, cache: Mutex::new(None) }
    }

    pub async fn header(&self) -> Result<AuthHeader> {
        let Some(secret) = &self.client_secret else {
            return self.internal_key.clone().map(AuthHeader::InternalKey).ok_or_else(|| {
                AppError::Config("SERVICE_CLIENT_SECRET or OUTBOUND_INTERNAL_API_KEY is required".into())
            });
        };
        let mut cache = self.cache.lock().await;
        if let Some((token, expires)) = cache.as_ref() {
            if Instant::now() < *expires {
                return Ok(AuthHeader::Bearer(token.clone()));
            }
        }
        let res = self
            .http
            .post(&self.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", secret.as_str()),
            ])
            .send()
            .await
            .map_err(|e| upstream(0, e.to_string()))?;
        if !res.status().is_success() {
            return Err(upstream(res.status().as_u16(), "token request refused".into()));
        }
        let body: TokenResponse = res.json().await.map_err(|e| upstream(0, e.to_string()))?;
        let ttl = body.expires_in.unwrap_or(60).saturating_sub(30).max(30);
        *cache = Some((body.access_token.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(AuthHeader::Bearer(body.access_token))
    }
}

fn upstream(status: u16, message: String) -> AppError {
    AppError::Upstream { service: "keycloak", status, code: "TOKEN_ERROR".into(), message }
}
