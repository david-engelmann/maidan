//! Ed25519 sign / verify for [`maidan_types::SignedExport`] (Cluster 391).
//!
//! The operator holds a 32-byte seed (`MAIDAN_EXPORT_SIGNING_KEY`, hex or
//! base64). The public key travels in the artifact so a blank instance can
//! check integrity without calling the origin. Authenticity is optional:
//! `MAIDAN_EXPORT_VERIFY_KEYS` pins expected public keys; when unset,
//! verification uses the embedded key only (tamper-evident, not a trust
//! anchor). Fail closed on every mismatch.

use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use maidan_types::{
    canonical_json, hex_decode, hex_encode, reject_export_secrets, statement_value, SignedExport,
    SignedExportError, TokenPolicy, SIGNED_EXPORT_ALG, SIGNED_EXPORT_TYPE,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

/// Operator signing key derived from a 32-byte seed.
#[derive(Clone)]
pub struct ExportSigningKey {
    signing: SigningKey,
}

impl ExportSigningKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(&seed),
        }
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    pub fn public_key_hex(&self) -> String {
        hex_encode(&self.public_key_bytes())
    }
}

#[derive(Debug, Error)]
pub enum ExportSignError {
    #[error(transparent)]
    Envelope(#[from] SignedExportError),
    #[error("MAIDAN_EXPORT_SIGNING_KEY is not set")]
    MissingKey,
    #[error("MAIDAN_EXPORT_SIGNING_KEY must decode to 32 bytes (base64 or 64-char hex)")]
    InvalidKey,
    #[error(
        "MAIDAN_EXPORT_VERIFY_KEYS entry must be a 32-byte public key (64-char hex or base64)"
    )]
    InvalidVerifyKey,
}

#[derive(Debug, Error)]
pub enum ExportVerifyError {
    #[error(transparent)]
    Envelope(#[from] SignedExportError),
    #[error("signed export $type is not {SIGNED_EXPORT_TYPE}")]
    WrongType,
    #[error("signed export alg is not {SIGNED_EXPORT_ALG}")]
    WrongAlg,
    #[error("signed export token_policy is not tokens_die_on_export")]
    WrongTokenPolicy,
    #[error("content_sha256 does not match the canonical statement")]
    HashMismatch,
    #[error("export signature is invalid")]
    BadSignature,
    #[error("export public key is not in MAIDAN_EXPORT_VERIFY_KEYS")]
    KeyNotAllowed,
    #[error("export public key is not 32 bytes")]
    BadPublicKey,
    #[error("export signature is not 64 bytes")]
    BadSignatureLength,
}

pub fn sign_export(
    key: &ExportSigningKey,
    payload: serde_json::Value,
) -> Result<SignedExport, ExportSignError> {
    reject_export_secrets(&payload)?;
    let mut envelope = SignedExport {
        type_id: SIGNED_EXPORT_TYPE.into(),
        alg: SIGNED_EXPORT_ALG.into(),
        token_policy: TokenPolicy::TokensDieOnExport,
        public_key: key.public_key_hex(),
        signed_at: chrono::Utc::now(),
        payload,
        content_sha256: String::new(),
        signature: String::new(),
    };
    let hash = statement_hash(&envelope)?;
    envelope.content_sha256 = hex_encode(&hash);
    let sig = key.signing.sign(&hash);
    envelope.signature = hex_encode(&sig.to_bytes());
    Ok(envelope)
}

/// Verify a signed export. `expected` is an optional allowlist of public
/// keys (the destination operator's pin). `None` or empty = integrity
/// against the embedded key only.
pub fn verify_export(
    envelope: &SignedExport,
    expected: Option<&[[u8; 32]]>,
) -> Result<(), ExportVerifyError> {
    if envelope.type_id != SIGNED_EXPORT_TYPE {
        return Err(ExportVerifyError::WrongType);
    }
    if envelope.alg != SIGNED_EXPORT_ALG {
        return Err(ExportVerifyError::WrongAlg);
    }
    if envelope.token_policy != TokenPolicy::TokensDieOnExport {
        return Err(ExportVerifyError::WrongTokenPolicy);
    }
    reject_export_secrets(&envelope.payload)?;

    let hash = statement_hash(envelope)?;
    let claimed = hex_decode(&envelope.content_sha256)?;
    if claimed.len() != 32 || !bool::from(hash.as_slice().ct_eq(&claimed)) {
        return Err(ExportVerifyError::HashMismatch);
    }

    let pk_bytes = decode_public_key(&envelope.public_key)?;
    if let Some(allowed) = expected {
        if !allowed.is_empty() && !allowed.iter().any(|k| bool::from(k.ct_eq(&pk_bytes))) {
            return Err(ExportVerifyError::KeyNotAllowed);
        }
    }

    let sig_bytes = hex_decode(&envelope.signature)?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| ExportVerifyError::BadSignatureLength)?;
    let verifying =
        VerifyingKey::from_bytes(&pk_bytes).map_err(|_| ExportVerifyError::BadPublicKey)?;
    let signature = Signature::from_bytes(&sig_arr);
    verifying
        .verify(&hash, &signature)
        .map_err(|_| ExportVerifyError::BadSignature)
}

fn statement_hash(envelope: &SignedExport) -> Result<[u8; 32], SignedExportError> {
    let stmt = statement_value(envelope)?;
    let canonical = canonical_json(&stmt)?;
    Ok(Sha256::digest(&canonical).into())
}

fn decode_public_key(raw: &str) -> Result<[u8; 32], ExportVerifyError> {
    let bytes = hex_decode(raw)?;
    bytes
        .try_into()
        .map_err(|_| ExportVerifyError::BadPublicKey)
}

/// Parse `MAIDAN_EXPORT_SIGNING_KEY`. Missing is `None` (export refuses).
pub fn export_signing_key_from_env() -> Result<Option<ExportSigningKey>, ExportSignError> {
    match std::env::var("MAIDAN_EXPORT_SIGNING_KEY") {
        Ok(raw) => parse_seed_bytes(&raw).map(|seed| Some(ExportSigningKey::from_seed(seed))),
        Err(_) => Ok(None),
    }
}

/// Parse `MAIDAN_EXPORT_VERIFY_KEYS` (comma-separated public keys). A
/// malformed entry is a hard error — silently dropping a pin would accept
/// a stranger's key.
pub fn export_verify_keys_from_env() -> Result<Vec<[u8; 32]>, ExportSignError> {
    match std::env::var("MAIDAN_EXPORT_VERIFY_KEYS") {
        Ok(raw) => parse_verify_keys(&raw),
        Err(_) => Ok(Vec::new()),
    }
}

fn parse_seed_bytes(raw: &str) -> Result<[u8; 32], ExportSignError> {
    parse_32(raw).map_err(|_| ExportSignError::InvalidKey)
}

fn parse_verify_keys(raw: &str) -> Result<Vec<[u8; 32]>, ExportSignError> {
    let mut keys = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        keys.push(parse_32(trimmed).map_err(|_| ExportSignError::InvalidVerifyKey)?);
    }
    Ok(keys)
}

fn parse_32(raw: &str) -> Result<[u8; 32], ()> {
    let trimmed = raw.trim();
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        let decoded = hex_decode(trimmed).map_err(|_| ())?;
        return decoded.try_into().map_err(|_| ());
    }
    let bytes = STANDARD.decode(trimmed).map_err(|_| ())?;
    bytes.try_into().map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_key() -> ExportSigningKey {
        ExportSigningKey::from_seed([0x11; 32])
    }

    fn payload() -> serde_json::Value {
        json!({
            "format_version": 1,
            "workspace": {"name": "room"},
            "members": [],
            "channels": []
        })
    }

    #[test]
    fn sign_then_verify_succeeds() {
        let key = test_key();
        let signed = sign_export(&key, payload()).unwrap();
        assert_eq!(signed.type_id, SIGNED_EXPORT_TYPE);
        assert_eq!(signed.token_policy, TokenPolicy::TokensDieOnExport);
        assert_eq!(signed.public_key, key.public_key_hex());
        verify_export(&signed, None).unwrap();
        verify_export(&signed, Some(&[key.public_key_bytes()])).unwrap();
    }

    #[test]
    fn bit_flip_in_payload_fails() {
        let signed = sign_export(&test_key(), payload()).unwrap();
        let mut tampered = signed;
        tampered.payload["workspace"]["name"] = json!("evil");
        assert!(matches!(
            verify_export(&tampered, None),
            Err(ExportVerifyError::HashMismatch)
        ));
    }

    #[test]
    fn bad_signature_fails() {
        let signed = sign_export(&test_key(), payload()).unwrap();
        let mut tampered = signed;
        let mut sig = hex_decode(&tampered.signature).unwrap();
        sig[0] ^= 0x01;
        tampered.signature = hex_encode(&sig);
        assert!(matches!(
            verify_export(&tampered, None),
            Err(ExportVerifyError::BadSignature)
        ));
    }

    #[test]
    fn wrong_verify_keyring_fails() {
        let signed = sign_export(&test_key(), payload()).unwrap();
        let other = ExportSigningKey::from_seed([0x22; 32]);
        assert!(matches!(
            verify_export(&signed, Some(&[other.public_key_bytes()])),
            Err(ExportVerifyError::KeyNotAllowed)
        ));
    }

    #[test]
    fn forbidden_secret_field_fails_sign_and_verify() {
        let leaked = json!({"members": [{"token_hash": "abc"}]});
        assert!(sign_export(&test_key(), leaked.clone()).is_err());
        let mut signed = sign_export(&test_key(), payload()).unwrap();
        signed.payload = leaked;
        // Hash will also fail; reject_secrets runs first.
        assert!(matches!(
            verify_export(&signed, None),
            Err(ExportVerifyError::Envelope(
                SignedExportError::ForbiddenField(_)
            ))
        ));
    }

    #[test]
    fn parse_seed_hex_and_base64() {
        let hex = "11".repeat(32);
        let key = ExportSigningKey::from_seed(parse_seed_bytes(&hex).unwrap());
        assert_eq!(key.public_key_hex().len(), 64);
        let b64 = STANDARD.encode([0x11u8; 32]);
        parse_seed_bytes(&b64).unwrap();
        assert!(parse_seed_bytes("short").is_err());
    }
}
