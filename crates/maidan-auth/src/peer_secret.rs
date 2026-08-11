//! Encrypt federation peer bearer secrets for at-rest storage (outbound poll).

use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce,
};
use thiserror::Error;

const NONCE_LEN: usize = 12;

#[derive(Debug, Error)]
pub enum PeerSecretError {
    #[error("FEDERATION_ENCRYPTION_KEY is not set")]
    MissingKey,
    #[error("FEDERATION_ENCRYPTION_KEY must decode to 32 bytes (base64 or 64-char hex)")]
    InvalidKey,
    #[error("invalid outbound secret ciphertext")]
    InvalidCiphertext,
    #[error("decryption failed")]
    DecryptFailed,
}

pub fn encryption_key_from_env() -> Result<[u8; 32], PeerSecretError> {
    let raw =
        std::env::var("FEDERATION_ENCRYPTION_KEY").map_err(|_| PeerSecretError::MissingKey)?;
    parse_key_bytes(&raw)
}

fn parse_key_bytes(raw: &str) -> Result<[u8; 32], PeerSecretError> {
    let trimmed = raw.trim();
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut key = [0u8; 32];
        for (i, chunk) in trimmed.as_bytes().chunks(2).enumerate() {
            if i >= 32 {
                return Err(PeerSecretError::InvalidKey);
            }
            let s = std::str::from_utf8(chunk).map_err(|_| PeerSecretError::InvalidKey)?;
            key[i] = u8::from_str_radix(s, 16).map_err(|_| PeerSecretError::InvalidKey)?;
        }
        return Ok(key);
    }
    let bytes = STANDARD
        .decode(trimmed)
        .map_err(|_| PeerSecretError::InvalidKey)?;
    bytes.try_into().map_err(|_| PeerSecretError::InvalidKey)
}

pub fn encrypt_peer_secret(plaintext: &str, key: &[u8; 32]) -> Result<String, PeerSecretError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| PeerSecretError::InvalidKey)?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    chacha20poly1305::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| PeerSecretError::InvalidCiphertext)?;
    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(STANDARD.encode(blob))
}

/// Decryption fallback keys for **key rotation** (Cluster 189): old keys kept
/// available for decrypt after the primary (`FEDERATION_ENCRYPTION_KEY`) is
/// rotated. Set once at startup; empty when no rotation is in progress.
static DECRYPT_FALLBACK_KEYS: std::sync::OnceLock<Vec<[u8; 32]>> = std::sync::OnceLock::new();

/// Install the process-wide decrypt fallback keys (idempotent; first call wins).
pub fn init_decrypt_fallback_keys(keys: Vec<[u8; 32]>) {
    let _ = DECRYPT_FALLBACK_KEYS.set(keys);
}

/// Parse `FEDERATION_DECRYPT_KEYS` — a comma-separated list of old encryption
/// keys (each base64 or 64-char hex, same encoding as the primary) to try on
/// decrypt during a rotation. A malformed entry is a hard error (silently
/// dropping an old key would make its ciphertexts undecryptable).
pub fn decrypt_fallback_keys_from_env() -> Result<Vec<[u8; 32]>, PeerSecretError> {
    match std::env::var("FEDERATION_DECRYPT_KEYS") {
        Ok(raw) => parse_decrypt_keys(&raw),
        Err(_) => Ok(Vec::new()),
    }
}

fn parse_decrypt_keys(raw: &str) -> Result<Vec<[u8; 32]>, PeerSecretError> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_key_bytes)
        .collect()
}

/// Try `keys` in order; return the first successful decrypt. AEAD authentication
/// makes trying the wrong key safe — it fails cleanly rather than returning
/// garbage — so a keyring can attempt several keys without corruption.
pub fn decrypt_peer_secret_multi(
    encoded: &str,
    keys: &[[u8; 32]],
) -> Result<String, PeerSecretError> {
    let mut last = PeerSecretError::DecryptFailed;
    for key in keys {
        match decrypt_peer_secret(encoded, key) {
            Ok(plain) => return Ok(plain),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// Decrypt with the runtime `primary` key first, then the process-wide rotation
/// fallbacks (Cluster 189). New ciphertexts made with the primary decrypt on the
/// first try; ciphertexts made with a pre-rotation key decrypt via a fallback.
pub fn decrypt_peer_secret_rotating(
    encoded: &str,
    primary: &[u8; 32],
) -> Result<String, PeerSecretError> {
    match decrypt_peer_secret(encoded, primary) {
        Ok(plain) => Ok(plain),
        Err(primary_err) => match DECRYPT_FALLBACK_KEYS.get() {
            Some(fallbacks) if !fallbacks.is_empty() => {
                decrypt_peer_secret_multi(encoded, fallbacks)
            }
            _ => Err(primary_err),
        },
    }
}

pub fn decrypt_peer_secret(encoded: &str, key: &[u8; 32]) -> Result<String, PeerSecretError> {
    let blob = STANDARD
        .decode(encoded.trim())
        .map_err(|_| PeerSecretError::InvalidCiphertext)?;
    if blob.len() <= NONCE_LEN {
        return Err(PeerSecretError::InvalidCiphertext);
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| PeerSecretError::InvalidKey)?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| PeerSecretError::DecryptFailed)?;
    String::from_utf8(plain).map_err(|_| PeerSecretError::DecryptFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [0x11; 32]
    }

    #[test]
    fn round_trip_encrypt_decrypt() {
        let ct = encrypt_peer_secret("maid_test_secret", &test_key()).expect("encrypt");
        let plain = decrypt_peer_secret(&ct, &test_key()).expect("decrypt");
        assert_eq!(plain, "maid_test_secret");
    }

    #[test]
    fn parse_hex_key() {
        let hex = "ab".repeat(32);
        let key = parse_key_bytes(&hex).expect("hex key");
        assert_eq!(key, [0xab; 32]);
    }

    #[test]
    fn decrypt_rejects_wrong_key() {
        let ct = encrypt_peer_secret("secret", &test_key()).expect("encrypt");
        let wrong = [0x22; 32];
        assert!(decrypt_peer_secret(&ct, &wrong).is_err());
    }

    #[test]
    fn multi_tries_keys_until_one_works() {
        let good = [0x33; 32];
        let wrong = [0x44; 32];
        let ct = encrypt_peer_secret("rotate-me", &good).expect("encrypt");
        // A ciphertext made with `good` decrypts when `good` is anywhere in the
        // keyring (here, after a wrong key) — the rotation case.
        assert_eq!(
            decrypt_peer_secret_multi(&ct, &[wrong, good]).expect("multi"),
            "rotate-me"
        );
        // No matching key fails cleanly.
        assert!(decrypt_peer_secret_multi(&ct, &[wrong]).is_err());
        assert!(decrypt_peer_secret_multi(&ct, &[]).is_err());
    }

    #[test]
    fn parse_decrypt_keys_reads_a_list_and_rejects_bad() {
        let hex = "cd".repeat(32);
        let b64 = STANDARD.encode([0xef; 32]);
        let keys = parse_decrypt_keys(&format!(" {hex}, {b64} ")).expect("keys");
        assert_eq!(keys, vec![[0xcd; 32], [0xef; 32]]);
        assert!(parse_decrypt_keys("").expect("empty").is_empty());
        // A malformed entry is a hard error (don't silently drop an old key).
        assert!(parse_decrypt_keys(&format!("{hex},not-a-key")).is_err());
    }
}
