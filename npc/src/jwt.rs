//! Minimal JWT generator for NPC authentication.
//!
//! Produces a signed HS256 token with the payload Horizon expects:
//! ```json
//! { "sub": "<uuid>", "preferred_username": "<name>", "exp": <unix_ts> }
//! ```
//!
//! `exp` is set to one hour from now. The token is regenerated before every
//! Horizon reconnect, so it is always fresh.

use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;

use crate::error::{NpcError, Result};

#[derive(Debug, Serialize)]
struct NpcClaims<'a> {
    sub: &'a str,
    preferred_username: &'a str,
    exp: u64,
}

/// Generate a HS256-signed JWT for an NPC.
///
/// * `uuid`   – NPC's stable identifier (used as `sub`).
/// * `name`   – Display name (used as `preferred_username`).
/// * `secret` – HMAC-SHA256 signing key (must match Horizon's expectation).
pub fn generate(uuid: &str, name: &str, secret: &str) -> Result<String> {
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + 3600; // 1 hour; regenerated on every reconnect

    let claims = NpcClaims {
        sub: uuid,
        preferred_username: name,
        exp,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| NpcError::Config(format!("JWT signing failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Claims {
        sub: String,
        preferred_username: String,
        exp: u64,
    }

    #[test]
    fn round_trip() {
        let token = generate("uuid-123", "Alice", "secret").unwrap();

        let mut val = Validation::new(Algorithm::HS256);
        val.validate_exp = true;

        let data = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(b"secret"),
            &val,
        )
        .unwrap();

        assert_eq!(data.claims.sub, "uuid-123");
        assert_eq!(data.claims.preferred_username, "Alice");
        assert!(data.claims.exp > 0);
    }
}

