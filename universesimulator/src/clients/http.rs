//! Shared HTTP transport for the game services' internal APIs.

use std::sync::Arc;

use reqwest::Method;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use super::token::{AuthHeader, TokenProvider};
use crate::error::{AppError, Result};

#[derive(Clone)]
pub struct Upstream {
    pub name: &'static str,
    base_url: String,
    http: reqwest::Client,
    token: Option<Arc<TokenProvider>>,
}

impl Upstream {
    pub fn new(name: &'static str, base_url: &str, http: reqwest::Client, token: Option<Arc<TokenProvider>>) -> Self {
        Self { name, base_url: base_url.trim_end_matches('/').to_string(), http, token }
    }

    pub async fn send<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        let mut req = self.http.request(method, format!("{}{}", self.base_url, path));
        if let Some(token) = &self.token {
            req = match token.header().await? {
                AuthHeader::Bearer(t) => req.bearer_auth(t),
                AuthHeader::InternalKey(k) => req.header("X-Internal-Key", k),
            };
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        let res = req.send().await.map_err(|e| AppError::Upstream {
            service: self.name,
            status: 0,
            code: "UNREACHABLE".into(),
            message: e.to_string(),
        })?;
        let status = res.status();
        if !status.is_success() {
            let body: Value = res.json().await.unwrap_or(Value::Null);
            return Err(AppError::Upstream {
                service: self.name,
                status: status.as_u16(),
                code: body["error"].as_str().unwrap_or("UNKNOWN").to_string(),
                message: body["message"].as_str().unwrap_or_default().to_string(),
            });
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return serde_json::from_value(Value::Null).map_err(|e| self.decode_error(e));
        }
        res.json().await.map_err(|e| AppError::Upstream {
            service: self.name,
            status: status.as_u16(),
            code: "DECODE".into(),
            message: e.to_string(),
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send::<Value, T>(Method::GET, path, None).await
    }

    pub async fn post<B: Serialize + ?Sized, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        self.send(Method::POST, path, Some(body)).await
    }

    pub async fn put<B: Serialize + ?Sized, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        self.send(Method::PUT, path, Some(body)).await
    }

    fn decode_error(&self, e: serde_json::Error) -> AppError {
        AppError::Upstream { service: self.name, status: 0, code: "DECODE".into(), message: e.to_string() }
    }
}

/// Maps an upstream 404 to `None`.
pub fn optional<T>(res: Result<T>) -> Result<Option<T>> {
    match res {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.upstream_status() == Some(404) => Ok(None),
        Err(e) => Err(e),
    }
}
