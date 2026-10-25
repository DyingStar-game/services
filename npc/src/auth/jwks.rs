//! Keycloak JWKS cache: keys are loaded on first use and reloaded when a token carries an
//! unknown `kid`, at most once per minute.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::{NpcError, Result};

const RELOAD_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    #[serde(rename = "use")]
    usage: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

pub struct Jwks {
    url: String,
    http: reqwest::Client,
    keys: RwLock<(HashMap<String, DecodingKey>, Option<Instant>)>,
}

impl Jwks {
    pub fn new(url: String, http: reqwest::Client) -> Self {
        Self { url, http, keys: RwLock::new((HashMap::new(), None)) }
    }

    pub async fn key(&self, kid: &str) -> Result<DecodingKey> {
        if let Some(k) = self.keys.read().await.0.get(kid) {
            return Ok(k.clone());
        }
        let mut guard = self.keys.write().await;
        let recently = guard.1.is_some_and(|t| t.elapsed() < RELOAD_COOLDOWN);
        if !recently {
            guard.0 = self.fetch().await?;
            guard.1 = Some(Instant::now());
        }
        guard.0.get(kid).cloned().ok_or_else(|| NpcError::Unauthorized("unknown signing key".into()))
    }

    async fn fetch(&self) -> Result<HashMap<String, DecodingKey>> {
        let set: JwkSet = self
            .http
            .get(&self.url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| NpcError::Unauthorized(format!("JWKS unavailable: {e}")))?
            .json()
            .await
            .map_err(|e| NpcError::Unauthorized(format!("JWKS invalid: {e}")))?;
        Ok(set
            .keys
            .into_iter()
            .filter(|k| k.kty == "RSA" && k.usage.as_deref().unwrap_or("sig") == "sig")
            .filter_map(|k| {
                let key = DecodingKey::from_rsa_components(k.n.as_deref()?, k.e.as_deref()?).ok()?;
                Some((k.kid?, key))
            })
            .collect())
    }
}
