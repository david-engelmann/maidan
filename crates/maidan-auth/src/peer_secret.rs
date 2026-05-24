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
}
