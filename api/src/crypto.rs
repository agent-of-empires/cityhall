//! Symmetric encryption for secrets stored in the database (currently the SMTP
//! password). Uses AES-256-GCM with a key supplied via `CITYHALL_SECRET_KEY`
//! (base64-encoded, 32 bytes). Ciphertext is stored as base64 of
//! `nonce (12 bytes) || ciphertext`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use crate::error::AppError;

const KEY_ENV: &str = "CITYHALL_SECRET_KEY";
const NONCE_LEN: usize = 12;

/// Load and validate the 32-byte key from the environment.
fn cipher() -> Result<Aes256Gcm, AppError> {
    let raw = std::env::var(KEY_ENV).map_err(|_| {
        AppError::BadRequest(
            "CITYHALL_SECRET_KEY is not set; it is required to store SMTP credentials",
        )
    })?;
    let bytes = B64
        .decode(raw.trim())
        .map_err(|_| AppError::Internal("CITYHALL_SECRET_KEY is not valid base64"))?;
    let key = Key::<Aes256Gcm>::try_from(bytes.as_slice())
        .map_err(|_| AppError::Internal("CITYHALL_SECRET_KEY must decode to 32 bytes"))?;
    Ok(Aes256Gcm::new(&key))
}

/// Whether a usable secret key is configured. Used to gate flows that need to
/// read or write encrypted secrets before attempting them.
pub fn key_available() -> bool {
    cipher().is_ok()
}

pub fn encrypt(plaintext: &str) -> Result<String, AppError> {
    let cipher = cipher()?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes).map_err(|_| AppError::Internal("secure RNG failure"))?;
    let nonce: Nonce<_> = nonce_bytes.into();
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| AppError::Internal("failed to encrypt secret"))?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok(B64.encode(blob))
}

pub fn decrypt(encoded: &str) -> Result<String, AppError> {
    let cipher = cipher()?;
    let blob = B64
        .decode(encoded)
        .map_err(|_| AppError::Internal("stored secret is not valid base64"))?;
    if blob.len() < NONCE_LEN {
        return Err(AppError::Internal("stored secret is malformed"));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce_arr: [u8; NONCE_LEN] = nonce_bytes
        .try_into()
        .map_err(|_| AppError::Internal("stored secret is malformed"))?;
    let nonce: Nonce<_> = nonce_arr.into();
    let plaintext = cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| AppError::Internal("failed to decrypt secret (wrong key?)"))?;
    String::from_utf8(plaintext).map_err(|_| AppError::Internal("decrypted secret is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A single test because these all mutate the same process-wide env var;
    // running them as separate parallel tests would race on the key.
    #[test]
    fn encrypt_decrypt_and_missing_key() {
        // Missing key: no encryption possible.
        std::env::remove_var(KEY_ENV);
        assert!(!key_available());
        assert!(encrypt("x").is_err());

        // With a fixed 32-byte key: round-trips, and reuses distinct nonces.
        std::env::set_var(KEY_ENV, B64.encode([7u8; 32]));
        let secret = "hunter2 \u{1f510} unicode";
        let blob = encrypt(secret).unwrap();
        assert_ne!(blob, secret);
        assert_eq!(decrypt(&blob).unwrap(), secret);
        assert_ne!(encrypt("same").unwrap(), encrypt("same").unwrap());

        std::env::remove_var(KEY_ENV);
    }
}
