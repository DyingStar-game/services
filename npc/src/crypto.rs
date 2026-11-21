//! NPC passwords at rest. The service needs them in clear to log each NPC in
//! (password grant), so they are sealed, not hashed: AES-256-GCM with a random
//! nonce, the NPC's uuid as associated data (a blob copied onto another row
//! does not open), key from `NPC_PASSWORD_KEY`.
//!
//! Blob layout: nonce (12 bytes) || ciphertext + tag.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rand::{Rng, distributions::Uniform};
use uuid::Uuid;

use crate::error::{NpcError, Result};

const NONCE_LEN: usize = 12;

/// Id of the key the current blobs are sealed with (`npcs.password_key_id`),
/// so a key rotation can tell old blobs from new ones.
pub const KEY_ID: i16 = 1;

#[derive(Clone)]
pub struct PasswordBox {
    cipher: Aes256Gcm,
}

impl PasswordBox {
    /// `key_b64`: base64 of 32 random bytes (`openssl rand -base64 32`).
    pub fn from_base64(key_b64: &str) -> Result<Self> {
        let key = STANDARD
            .decode(key_b64.trim())
            .map_err(|e| NpcError::Config(format!("NPC_PASSWORD_KEY is not base64: {e}")))?;
        if key.len() != 32 {
            return Err(NpcError::Config(format!(
                "NPC_PASSWORD_KEY must decode to 32 bytes, got {}",
                key.len()
            )));
        }
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| NpcError::Config(format!("NPC_PASSWORD_KEY: {e}")))?;
        Ok(Self { cipher })
    }

    pub fn seal(&self, id: Uuid, plaintext: &str) -> Result<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, Payload { msg: plaintext.as_bytes(), aad: id.as_bytes() })
            .map_err(|_| NpcError::Config("password sealing failed".into()))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ct);
        Ok(blob)
    }

    pub fn open(&self, id: Uuid, blob: &[u8]) -> Result<String> {
        if blob.len() <= NONCE_LEN {
            return Err(NpcError::Config("sealed password too short".into()));
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: id.as_bytes() })
            .map_err(|_| NpcError::Config("sealed password does not open (wrong key or row)".into()))?;
        String::from_utf8(pt).map_err(|_| NpcError::Config("sealed password is not UTF-8".into()))
    }
}

/// A fresh NPC password: 32 characters from letters, digits and a few symbols,
/// so it passes the realm policy (length, not the username, not the email).
pub fn random_password() -> String {
    const CHARSET: &[u8] =
        b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_.!@#%+=";
    let mut rng = rand::thread_rng();
    let dist = Uniform::from(0..CHARSET.len());
    (0..32).map(|_| CHARSET[rng.sample(dist)] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed() -> PasswordBox {
        PasswordBox::from_base64(&STANDARD.encode([7u8; 32])).unwrap()
    }

    #[test]
    fn round_trip() {
        let b = boxed();
        let id = Uuid::new_v4();
        let blob = b.seal(id, "s3cret!").unwrap();
        assert_eq!(b.open(id, &blob).unwrap(), "s3cret!");
    }

    #[test]
    fn other_row_does_not_open() {
        let b = boxed();
        let blob = b.seal(Uuid::new_v4(), "s3cret!").unwrap();
        assert!(b.open(Uuid::new_v4(), &blob).is_err());
    }

    #[test]
    fn tampered_blob_does_not_open() {
        let b = boxed();
        let id = Uuid::new_v4();
        let mut blob = b.seal(id, "s3cret!").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(b.open(id, &blob).is_err());
    }

    #[test]
    fn rejects_short_keys() {
        assert!(PasswordBox::from_base64(&STANDARD.encode([1u8; 16])).is_err());
        assert!(PasswordBox::from_base64("not base64 !").is_err());
    }

    #[test]
    fn random_passwords_are_long_and_distinct() {
        let a = random_password();
        assert_eq!(a.chars().count(), 32);
        assert_ne!(a, random_password());
    }
}
