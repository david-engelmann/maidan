//! HMAC-signed subscribe resume tokens (WS + MCP SSE).

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use hmac::{Hmac, Mac};
use maidan_types::EventFilter;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::config::ConfigError;
use crate::session::session_secret_from_env;

type HmacSha256 = Hmac<Sha256>;

/// Fixed secret for integration tests (`AppState::for_tests`).
pub const TEST_SUBSCRIBE_RESUME_SECRET: &[u8] = b"test-subscribe-resume-secret-32b!!";

const MAX_PAYLOAD_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum SubscribeResumeError {
    #[error("resume token payload too large")]
    PayloadTooLarge,

    #[error("malformed resume token")]
    Malformed,

    #[error("resume token expired")]
    Expired,

    #[error("resume token signature invalid")]
    InvalidSignature,

    #[error("{0}")]
    Internal(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct ResumePayload {
    filter: EventFilter,
    after_id: i64,
    exp: i64,
}

pub fn secret_from_env() -> Result<Arc<[u8]>, ConfigError> {
    if let Ok(raw) = std::env::var("MAIDAN_SUBSCRIBE_RESUME_SECRET") {
        return validate_secret(&raw);
    }
    session_secret_from_env()
}

pub fn ttl_secs_from_env() -> u64 {
    std::env::var("MAIDAN_SUBSCRIBE_RESUME_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3600)
}

fn validate_secret(raw: &str) -> Result<Arc<[u8]>, ConfigError> {
    if raw.len() < 32 {
        return Err(ConfigError::Invalid(
            "MAIDAN_SUBSCRIBE_RESUME_SECRET",
            "must be at least 32 bytes".into(),
        ));
    }
    Ok(Arc::from(raw.as_bytes()))
}

pub fn sign_resume_token(
    filter: &EventFilter,
    after_id: i64,
    secret: &[u8],
    ttl_secs: u64,
) -> Result<String, SubscribeResumeError> {
    let exp = Utc::now().timestamp() + i64::try_from(ttl_secs).unwrap_or(i64::MAX);
    let payload = ResumePayload {
        filter: filter.clone(),
        after_id,
        exp,
    };
    let json =
        serde_json::to_vec(&payload).map_err(|e| SubscribeResumeError::Internal(e.to_string()))?;
    if json.len() > MAX_PAYLOAD_BYTES {
        return Err(SubscribeResumeError::PayloadTooLarge);
    }
    let encoded = URL_SAFE_NO_PAD.encode(&json);
    let mac = mac_for_payload(encoded.as_bytes(), secret);
    Ok(format!("{}.{}", encoded, hex::encode(mac)))
}

pub fn verify_resume_token(
    token: &str,
    secret: &[u8],
) -> Result<(EventFilter, i64), SubscribeResumeError> {
    let (encoded, mac_hex) = token
        .split_once('.')
        .ok_or(SubscribeResumeError::Malformed)?;
    let json = URL_SAFE_NO_PAD
        .decode(encoded.as_bytes())
        .map_err(|_| SubscribeResumeError::Malformed)?;
    if json.len() > MAX_PAYLOAD_BYTES {
        return Err(SubscribeResumeError::PayloadTooLarge);
    }
    let expected = mac_for_payload(encoded.as_bytes(), secret);
    let actual = hex::decode(mac_hex).map_err(|_| SubscribeResumeError::Malformed)?;
    if actual.len() != expected.len() || !bool::from(actual.ct_eq(&expected)) {
        return Err(SubscribeResumeError::InvalidSignature);
    }
    let payload: ResumePayload =
        serde_json::from_slice(&json).map_err(|_| SubscribeResumeError::Malformed)?;
    if payload.exp < Utc::now().timestamp() {
        return Err(SubscribeResumeError::Expired);
    }
    if payload.after_id < 0 {
        return Err(SubscribeResumeError::Malformed);
    }
    Ok((payload.filter, payload.after_id))
}

fn mac_for_payload(encoded: &[u8], secret: &[u8]) -> Vec<u8> {
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("subscribe resume secret length checked at init");
    mac.update(encoded);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::WorkspaceId;

    #[test]
    fn resume_token_rejects_expired_payload() {
        let filter = EventFilter::workspace(WorkspaceId(uuid::Uuid::new_v4()));
        let mut token = sign_resume_token(&filter, 0, TEST_SUBSCRIBE_RESUME_SECRET, 1).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(matches!(
            verify_resume_token(&token, TEST_SUBSCRIBE_RESUME_SECRET),
            Err(SubscribeResumeError::Expired)
        ));
        let _ = &mut token;
    }

    #[test]
    fn resume_token_round_trips_and_rejects_tampering() {
        let filter = EventFilter::workspace(WorkspaceId(uuid::Uuid::new_v4()));
        let signed = sign_resume_token(&filter, 42, TEST_SUBSCRIBE_RESUME_SECRET, 3600).unwrap();
        let (f, after_id) = verify_resume_token(&signed, TEST_SUBSCRIBE_RESUME_SECRET).unwrap();
        assert_eq!(after_id, 42);
        assert_eq!(f.workspace_id, filter.workspace_id);

        let mut tampered = signed.clone();
        if let Some((_, mac)) = tampered.rsplit_once('.') {
            let mut bad_mac = mac.to_string();
            bad_mac.replace_range(0..1, "a");
            tampered = format!("{}.{}", tampered.rsplit_once('.').unwrap().0, bad_mac);
        }
        assert!(verify_resume_token(&tampered, TEST_SUBSCRIBE_RESUME_SECRET).is_err());
    }
}
