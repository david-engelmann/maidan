//! `Idempotency-Key` on writes (draft-ietf-httpapi-idempotency-key-header).
//!
//! An agent that times out cannot tell whether its post, claim or upload
//! landed, so it retries, and before this a retry ran again. A write carrying
//! `Idempotency-Key` now runs at most once per caller and key: the first
//! request reserves the key and its response is stored; a retry with the same
//! key and the same request gets that response back with
//! `Idempotent-Replayed: true`. The same key on a different request is a 422;
//! a retry while the first is still running is a 409.
//!
//! What is kept: a response the retry should see again, which is any 2xx or
//! 4xx except the "not now" ones (408, 409, 425, 429). Those, a 5xx, a
//! stream, or a body over [`MAX_STORED_BODY`] release the key, so the retry
//! runs again. Keys are scoped to the caller (workspace and acting member)
//! and live [`RETENTION`]. A request without the header, a read, an
//! unauthenticated route, and the SCIM and A2A protocol routes (which answer
//! in their own error envelopes) are untouched.

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use maidan_auth::AuthContext;
use maidan_store::{IdempotencyReservation, NewIdempotencyKey, StoredResponse};
use sha2::{Digest, Sha256};

use crate::{error::ApiError, state::AppState};

pub const IDEMPOTENCY_KEY: &str = "idempotency-key";
/// The header as the API documents it.
pub const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";
pub const IDEMPOTENT_REPLAYED: &str = "idempotent-replayed";
/// How long a key and its stored response are kept.
pub const RETENTION: Duration = Duration::hours(24);
/// How long a first request may hold its key before a retry may take it over.
pub const LOCK: Duration = Duration::minutes(5);
/// The largest response body kept for replay.
pub const MAX_STORED_BODY: usize = 1024 * 1024;
const MAX_KEY_LEN: usize = 255;

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_KEY_LEN && key.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// The request as the key's fingerprint sees it: method, path and query,
/// and body. Headers are left out, so a retry with a fresh trace id matches.
pub fn fingerprint(method: &Method, path_and_query: &str, body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(method.as_str().as_bytes());
    h.update(b"\n");
    h.update(path_and_query.as_bytes());
    h.update(b"\n");
    h.update(body);
    hex::encode(h.finalize())
}

/// Answers a retry should not get back for a day: server errors, and the
/// 4xx that say "not now" rather than "no" (timeout, conflict with the
/// current state such as a WIP limit, too early, rate limited). Releasing
/// costs only a rerun, which for these answers is what the caller wants.
fn transient(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT
                | StatusCode::CONFLICT
                | StatusCode::TOO_EARLY
                | StatusCode::TOO_MANY_REQUESTS
        )
}

fn replay(stored: StoredResponse) -> Response {
    let mut res = Response::new(Body::from(stored.body));
    *res.status_mut() = StatusCode::from_u16(stored.status).unwrap_or(StatusCode::OK);
    if let Some(ct) = stored
        .content_type
        .and_then(|ct| HeaderValue::from_str(&ct).ok())
    {
        res.headers_mut().insert(header::CONTENT_TYPE, ct);
    }
    res.headers_mut()
        .insert(IDEMPOTENT_REPLAYED, HeaderValue::from_static("true"));
    res
}

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let writes = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    let Some(raw) = req.headers().get(IDEMPOTENCY_KEY).cloned() else {
        return next.run(req).await;
    };
    let Some(auth) = req.extensions().get::<AuthContext>().cloned() else {
        return next.run(req).await;
    };
    // SCIM and A2A answer errors in their own envelopes (SCIM JSON,
    // JSON-RPC / AIP-193); a problem+json refusal from here would break their
    // clients, and neither protocol defines this header.
    let path = req.uri().path();
    let own_envelope = path.starts_with("/scim/") || path.starts_with("/a2a/");
    if !writes || auth.bypass || own_envelope {
        return next.run(req).await;
    }
    let Some(key) = raw
        .to_str()
        .ok()
        .filter(|k| valid_key(k))
        .map(str::to_string)
    else {
        return ApiError::BadRequest(format!(
            "Idempotency-Key must be 1 to {MAX_KEY_LEN} visible ASCII characters"
        ))
        .into_response();
    };

    let (parts, body) = req.into_parts();
    let body = match to_bytes(body, crate::app::max_body_bytes_from_env()).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::PayloadTooLarge("request body exceeds the limit".into())
                .into_response()
        }
    };
    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| parts.uri.path().to_string());
    let print = fingerprint(&parts.method, &path, &body);
    let now = Utc::now();
    let new = NewIdempotencyKey {
        workspace_id: auth.workspace_id,
        actor_id: auth.actor_id,
        key: key.clone(),
        fingerprint: print.clone(),
        locked_until: now + LOCK,
        expires_at: now + RETENTION,
    };
    let lease = match state.store.reserve_idempotency_key(&new).await {
        Err(err) => return ApiError::from(err).into_response(),
        Ok(IdempotencyReservation::InFlight { fingerprint }) => {
            return if fingerprint == print {
                ApiError::IdempotencyKeyInFlight.into_response()
            } else {
                ApiError::IdempotencyKeyReused.into_response()
            };
        }
        Ok(IdempotencyReservation::Completed {
            fingerprint,
            response,
        }) => {
            return if fingerprint == print {
                metrics::counter!("maidan_idempotent_replays_total").increment(1);
                replay(response)
            } else {
                ApiError::IdempotencyKeyReused.into_response()
            };
        }
        Ok(IdempotencyReservation::Reserved { lease }) => lease,
    };

    let response = next.run(Request::from_parts(parts, Body::from(body))).await;
    let release = || async {
        if let Err(err) = state
            .store
            .release_idempotency_key(auth.workspace_id, auth.actor_id, &key, &lease)
            .await
        {
            tracing::warn!(%err, "idempotency key release failed; it lapses at its lock");
        }
    };
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let streaming = content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("text/event-stream"));
    if transient(response.status()) || streaming {
        release().await;
        return response;
    }
    let (res_parts, res_body) = response.into_parts();
    let bytes = match to_bytes(res_body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            release().await;
            return ApiError::Internal(format!("response body: {err}")).into_response();
        }
    };
    if bytes.len() > MAX_STORED_BODY {
        release().await;
    } else {
        let stored = StoredResponse {
            status: res_parts.status.as_u16(),
            content_type,
            body: bytes.to_vec(),
        };
        if let Err(err) = state
            .store
            .complete_idempotency_key(auth.workspace_id, auth.actor_id, &key, &lease, &stored)
            .await
        {
            // The write happened; a retry now waits out the lock, then runs
            // again. Say so rather than hide it.
            tracing::error!(%err, "idempotency key completion failed");
        }
    }
    Response::from_parts(res_parts, Body::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_visible_ascii_up_to_255() {
        assert!(valid_key("a"));
        assert!(valid_key(&"k".repeat(255)));
        assert!(!valid_key(""));
        assert!(!valid_key(&"k".repeat(256)));
        assert!(!valid_key("has space"));
        assert!(!valid_key("tab\t"));
    }

    #[test]
    fn not_now_answers_are_released_not_kept() {
        for s in [500, 503, 408, 409, 425, 429] {
            assert!(transient(StatusCode::from_u16(s).unwrap()), "{s}");
        }
        for s in [200, 201, 204, 400, 403, 404, 422] {
            assert!(!transient(StatusCode::from_u16(s).unwrap()), "{s}");
        }
    }

    #[test]
    fn the_fingerprint_covers_method_path_and_body() {
        let base = fingerprint(&Method::POST, "/threads/1/messages", b"{}");
        assert_eq!(
            base,
            fingerprint(&Method::POST, "/threads/1/messages", b"{}")
        );
        assert_ne!(
            base,
            fingerprint(&Method::PUT, "/threads/1/messages", b"{}")
        );
        assert_ne!(
            base,
            fingerprint(&Method::POST, "/threads/2/messages", b"{}")
        );
        assert_ne!(
            base,
            fingerprint(&Method::POST, "/threads/1/messages?x=1", b"{}")
        );
        assert_ne!(
            base,
            fingerprint(&Method::POST, "/threads/1/messages", b"{\"a\":1}")
        );
    }
}
