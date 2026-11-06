//! Encrypt federation peer bearer secrets for at-rest storage (outbound poll).

use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce,
};
use thiserror::Error;

const NONCE_LEN: usize = 12;
/// ChaCha20-Poly1305 always appends a 16-byte Poly1305 tag.
const TAG_LEN: usize = 16;
/// Plaintext ceiling for a peer/webhook secret at encrypt time.
const MAX_SECRET_PLAINTEXT_BYTES: usize = 32_768;
/// Base64-encoded ciphertext ceiling before decode (covers nonce+ct+tag for
/// the plaintext ceiling, with headroom for encoding expansion).
const MAX_SECRET_CIPHERTEXT_B64_LEN: usize = 65_536;

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
    maidan_types::parse_key_32(raw).map_err(|_| PeerSecretError::InvalidKey)
}

pub fn encrypt_peer_secret(plaintext: &str, key: &[u8; 32]) -> Result<String, PeerSecretError> {
    if plaintext.len() > MAX_SECRET_PLAINTEXT_BYTES {
        return Err(PeerSecretError::InvalidCiphertext);
    }
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

/// Decryption fallback keys for **key rotation**: old keys kept available for
/// decrypt after the primary (`FEDERATION_ENCRYPTION_KEY`) is rotated. Set once
/// at startup; empty when no rotation is in progress.
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

/// Decode and frame-check once. `InvalidCiphertext` fails before any key is tried.
fn decode_secret_blob(encoded: &str) -> Result<Vec<u8>, PeerSecretError> {
    let trimmed = encoded.trim();
    if trimmed.len() > MAX_SECRET_CIPHERTEXT_B64_LEN {
        return Err(PeerSecretError::InvalidCiphertext);
    }
    let blob = STANDARD
        .decode(trimmed)
        .map_err(|_| PeerSecretError::InvalidCiphertext)?;
    // Nonce + Poly1305 tag is the minimum AEAD framing; a shorter blob can
    // never authenticate.
    if blob.len() < NONCE_LEN + TAG_LEN {
        return Err(PeerSecretError::InvalidCiphertext);
    }
    Ok(blob)
}

fn decrypt_blob(blob: &[u8], key: &[u8; 32]) -> Result<String, PeerSecretError> {
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| PeerSecretError::InvalidKey)?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| PeerSecretError::DecryptFailed)?;
    String::from_utf8(plain).map_err(|_| PeerSecretError::DecryptFailed)
}

/// Try `keys` in order; return the first successful decrypt. AEAD authentication
/// makes trying the wrong key safe — it fails cleanly rather than returning
/// garbage — so a keyring can attempt several keys without corruption.
/// Framing is validated once up front so `InvalidCiphertext` never walks the
/// keyring (and an empty keyring returns [`PeerSecretError::DecryptFailed`]
/// after that framing check).
pub fn decrypt_peer_secret_multi(
    encoded: &str,
    keys: &[[u8; 32]],
) -> Result<String, PeerSecretError> {
    let blob = decode_secret_blob(encoded)?;
    if keys.is_empty() {
        return Err(PeerSecretError::DecryptFailed);
    }
    let mut last = PeerSecretError::DecryptFailed;
    for key in keys {
        match decrypt_blob(&blob, key) {
            Ok(plain) => return Ok(plain),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// Decrypt with the runtime `primary` key first, then the process-wide rotation
/// fallbacks. New ciphertexts made with the primary decrypt on the first try;
/// ciphertexts made with a pre-rotation key decrypt via a fallback.
pub fn decrypt_peer_secret_rotating(
    encoded: &str,
    primary: &[u8; 32],
) -> Result<String, PeerSecretError> {
    let blob = decode_secret_blob(encoded)?;
    match decrypt_blob(&blob, primary) {
        Ok(plain) => Ok(plain),
        Err(primary_err) => match DECRYPT_FALLBACK_KEYS.get() {
            Some(fallbacks) if !fallbacks.is_empty() => {
                let mut last = primary_err;
                for key in fallbacks {
                    match decrypt_blob(&blob, key) {
                        Ok(plain) => return Ok(plain),
                        Err(err) => last = err,
                    }
                }
                Err(last)
            }
            _ => Err(primary_err),
        },
    }
}

pub fn decrypt_peer_secret(encoded: &str, key: &[u8; 32]) -> Result<String, PeerSecretError> {
    let blob = decode_secret_blob(encoded)?;
    decrypt_blob(&blob, key)
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
        assert_eq!(
            decrypt_peer_secret_multi(&ct, &[wrong, good]).expect("multi"),
            "rotate-me"
        );
        assert!(decrypt_peer_secret_multi(&ct, &[wrong]).is_err());
        assert!(matches!(
            decrypt_peer_secret_multi(&ct, &[]),
            Err(PeerSecretError::DecryptFailed)
        ));
    }

    #[test]
    fn parse_decrypt_keys_reads_a_list_and_rejects_bad() {
        let hex = "cd".repeat(32);
        let b64 = STANDARD.encode([0xef; 32]);
        let keys = parse_decrypt_keys(&format!(" {hex}, {b64} ")).expect("keys");
        assert_eq!(keys, vec![[0xcd; 32], [0xef; 32]]);
        assert!(parse_decrypt_keys("").expect("empty").is_empty());
        assert!(parse_decrypt_keys(&format!("{hex},not-a-key")).is_err());
    }

    #[test]
    fn oversized_plaintext_is_rejected() {
        let huge = "x".repeat(MAX_SECRET_PLAINTEXT_BYTES + 1);
        assert!(matches!(
            encrypt_peer_secret(&huge, &test_key()),
            Err(PeerSecretError::InvalidCiphertext)
        ));
    }

    #[test]
    fn oversized_ciphertext_b64_is_rejected_before_decode() {
        let huge = "A".repeat(MAX_SECRET_CIPHERTEXT_B64_LEN + 1);
        assert!(matches!(
            decrypt_peer_secret(&huge, &test_key()),
            Err(PeerSecretError::InvalidCiphertext)
        ));
        assert!(matches!(
            decrypt_peer_secret_multi(&huge, &[test_key()]),
            Err(PeerSecretError::InvalidCiphertext)
        ));
    }

    #[test]
    fn short_blob_under_nonce_plus_tag_is_rejected() {
        // 27 bytes < 28 (nonce+tag); base64 of 27 zero bytes.
        let short = STANDARD.encode([0u8; NONCE_LEN + TAG_LEN - 1]);
        assert!(matches!(
            decrypt_peer_secret(&short, &test_key()),
            Err(PeerSecretError::InvalidCiphertext)
        ));
    }

    #[test]
    fn invalid_ciphertext_short_circuits_rotation_without_trying_keys() {
        let junk = "!!!not-base64!!!";
        // Empty keyring still surfaces InvalidCiphertext (framing), not DecryptFailed.
        assert!(matches!(
            decrypt_peer_secret_multi(junk, &[]),
            Err(PeerSecretError::InvalidCiphertext)
        ));
        let wrong = [0x55; 32];
        assert!(matches!(
            decrypt_peer_secret_multi(junk, &[wrong, test_key()]),
            Err(PeerSecretError::InvalidCiphertext)
        ));
    }
}
