//! The first frame of a `/ws/subscribe` connection, and the HMAC-signed resume
//! token it may carry (issued on the WebSocket and the MCP SSE stream alike).
//!
//! Everything a subscriber sends before its credential is checked is decoded
//! here, without I/O, so the decoder can be fuzzed
//! (`fuzz/fuzz_targets/ws_subscribe.rs`). What the frame asks for is still
//! narrowed to the caller's workspace and grants by the server afterwards.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use hmac::{Hmac, Mac};
use maidan_types::EventFilter;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const MAX_PAYLOAD_BYTES: usize = 4096;

/// The JSON body of a subscriber's first frame.
#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeFrame {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub resume_token: Option<String>,
    #[serde(default)]
    pub filter: EventFilter,
    /// Replay persisted events with `id > after_id` before attaching to the bus.
    #[serde(default)]
    pub after_id: i64,
    /// Optional durable consumer id; server skips replay at or below stored cursor.
    #[serde(default)]
    pub consumer_id: Option<String>,
    /// When set with `filter.workspace_id`, enables presence/typing fan-out.
    #[serde(default)]
    pub member_id: Option<Uuid>,
    /// Opt into gap-free at-least-once delivery: cursor-driven reconcile
    /// instead of the optimistic live path. Requires `filter.workspace_id` and
    /// `consumer_id`; adds a stability-window latency floor on fresh events.
    #[serde(default)]
    pub at_least_once: bool,
    /// Opt into lean event frames: domain-event frames carry only `{log_id,
    /// kind,...ids}` — a "something happened, go fetch" pointer — instead of
    /// the full serialized event, saving tokens for an agent that tails for
    /// activity and reads on demand.
    #[serde(default)]
    pub lean: bool,
}

/// Why a subscribe frame asks for nothing the server can give.
#[derive(Debug, thiserror::Error)]
pub enum SubscribeRefusal {
    #[error("subscribe resume not configured on server")]
    ResumeNotConfigured,

    #[error("invalid resume_token: {0}")]
    InvalidResumeToken(SubscribeResumeError),

    #[error("resume token requires filter.workspace_id for replay")]
    ResumeReplayNeedsWorkspace,

    #[error("after_id must be non-negative")]
    NegativeAfterId,

    #[error("after_id requires filter.workspace_id for replay")]
    ReplayNeedsWorkspace,
}

/// The filter and replay cursor a frame asks for: its resume token's, when it
/// carries one, else its own. `resume_secret` is `None` when the server issues
/// no resume tokens.
pub fn resolve_subscribe(
    frame: &SubscribeFrame,
    resume_secret: Option<&[u8]>,
) -> Result<(EventFilter, i64), SubscribeRefusal> {
    if let Some(token) = frame.resume_token.as_deref().filter(|t| !t.is_empty()) {
        let secret = resume_secret.ok_or(SubscribeRefusal::ResumeNotConfigured)?;
        let (filter, after_id) =
            verify_resume_token(token, secret).map_err(SubscribeRefusal::InvalidResumeToken)?;
        if after_id > 0 && filter.workspace_id.is_none() {
            return Err(SubscribeRefusal::ResumeReplayNeedsWorkspace);
        }
        return Ok((filter, after_id));
    }
    if frame.after_id < 0 {
        return Err(SubscribeRefusal::NegativeAfterId);
    }
    if frame.after_id > 0 && frame.filter.workspace_id.is_none() {
        return Err(SubscribeRefusal::ReplayNeedsWorkspace);
    }
    Ok((frame.filter.clone(), frame.after_id))
}

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

pub fn sign_resume_token(
    filter: &EventFilter,
    after_id: i64,
    secret: &[u8],
    ttl_secs: u64,
) -> Result<String, SubscribeResumeError> {
    let exp = Utc::now()
        .timestamp()
        .saturating_add(i64::try_from(ttl_secs).unwrap_or(i64::MAX));
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
    let mut mac = HmacSha256::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC-SHA256 accepts any key length"));
    mac.update(encoded);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::WorkspaceId;

    const SECRET: &[u8] = b"test-subscribe-resume-secret-32b!!";

    #[test]
    fn resume_token_rejects_expired_payload() {
        let filter = EventFilter::workspace(WorkspaceId(uuid::Uuid::now_v7()));
        let token = sign_resume_token(&filter, 0, SECRET, 1).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(matches!(
            verify_resume_token(&token, SECRET),
            Err(SubscribeResumeError::Expired)
        ));
    }

    #[test]
    fn resume_token_round_trips_and_rejects_tampering() {
        let filter = EventFilter::workspace(WorkspaceId(uuid::Uuid::now_v7()));
        let signed = sign_resume_token(&filter, 42, SECRET, 3600).unwrap();
        let (f, after_id) = verify_resume_token(&signed, SECRET).unwrap();
        assert_eq!(after_id, 42);
        assert_eq!(f.workspace_id, filter.workspace_id);

        let mut tampered = signed.clone();
        tampered.pop();
        tampered.push('x');
        assert!(verify_resume_token(&tampered, SECRET).is_err());
    }

    #[test]
    fn a_ttl_past_the_end_of_time_signs_a_token_that_does_not_expire() {
        let filter = EventFilter::workspace(WorkspaceId(uuid::Uuid::now_v7()));
        let signed = sign_resume_token(&filter, 1, SECRET, u64::MAX).unwrap();
        assert_eq!(verify_resume_token(&signed, SECRET).unwrap().1, 1);
    }

    fn frame(json: serde_json::Value) -> SubscribeFrame {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn a_frame_without_a_resume_token_asks_for_its_own_filter_and_cursor() {
        let ws = uuid::Uuid::now_v7();
        let (filter, after_id) = resolve_subscribe(
            &frame(serde_json::json!({ "filter": { "workspace_id": ws }, "after_id": 5 })),
            None,
        )
        .unwrap();
        assert_eq!(filter.workspace_id, Some(WorkspaceId(ws)));
        assert_eq!(after_id, 5);
        assert!(matches!(
            resolve_subscribe(&frame(serde_json::json!({ "after_id": -1 })), None),
            Err(SubscribeRefusal::NegativeAfterId)
        ));
        assert!(matches!(
            resolve_subscribe(&frame(serde_json::json!({ "after_id": 1 })), None),
            Err(SubscribeRefusal::ReplayNeedsWorkspace)
        ));
    }

    #[test]
    fn a_resume_token_outranks_the_frame_and_needs_a_secret() {
        let ws = WorkspaceId(uuid::Uuid::now_v7());
        let token = sign_resume_token(&EventFilter::workspace(ws), 9, SECRET, 3600).unwrap();
        let sub = frame(serde_json::json!({ "resume_token": token, "after_id": -4 }));
        let (filter, after_id) = resolve_subscribe(&sub, Some(SECRET)).unwrap();
        assert_eq!((filter.workspace_id, after_id), (Some(ws), 9));
        assert!(matches!(
            resolve_subscribe(&sub, None),
            Err(SubscribeRefusal::ResumeNotConfigured)
        ));
        assert!(matches!(
            resolve_subscribe(&sub, Some(b"another-secret-of-thirty-two-bytes")),
            Err(SubscribeRefusal::InvalidResumeToken(
                SubscribeResumeError::InvalidSignature
            ))
        ));
    }
}
