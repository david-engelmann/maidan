//! Projector / broadcast lag token (Cluster 390, Wave 3 #30).
//!
//! Stamps `Maidan-Room-LSN` with **the caller's room** high-water so a client can
//! compare its last-seen `log_id` to the head it is actually chasing.
//!
//! Scoped in Cluster 398.8. It previously reported the instance-wide `MAX(id)`,
//! which made the number incomparable to anything a client had seen: a fully
//! caught-up projector could never reach it, because the remaining gap was other
//! tenants' writes. It also handed every tenant the instance's total event
//! volume, and rode outbound webhooks to third parties.
//!
//! **Not** [`crate::consistency`]: that header is a Postgres WAL LSN, replica-
//! gated, and answers read-your-writes. This header is always on (SQLite too),
//! decimal, and answers "how far behind is my projector?" Do not parse one as
//! the other.

use axum::{
    extract::{Request, State},
    http::{header::HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use maidan_store::Store;
use maidan_types::{RoomLsn, ROOM_LSN_HEADER};

use crate::state::AppState;

pub use maidan_types::ROOM_LSN_HEADER as HEADER;

/// The room a response belongs to, handed from an auth middleware to this one
/// through the **response** extensions (Cluster 398.8).
///
/// The Room-LSN layer is outside every auth layer — auth is applied per-router,
/// this is applied to the whole API — so it has no `AuthContext` on the way in
/// and cannot learn the caller's workspace by itself. Rather than move the
/// stamp inside each of the three routers that authenticate differently
/// (bearer, peer, session), each of them attaches the resolved workspace on the
/// way out. Absent means "no room" and the header is omitted, which is also
/// what an unauthenticated response should carry.
#[derive(Clone, Copy, Debug)]
pub struct RoomScope(pub maidan_types::WorkspaceId);

fn skip_path(path: &str) -> bool {
    matches!(
        path,
        "/health"
            | "/health/live"
            | "/health/ready"
            | "/metrics"
            | "/openapi.json"
            | "/ui"
            | "/ui/"
    ) || path.starts_with("/.well-known/")
}

/// Best-effort **instance** head. `None` if the store read fails (fail-open: no
/// header).
///
/// Used by `subscribe_ack` callers that have no workspace in hand. Prefer
/// [`current_for_room`] wherever the room is known — see its doc for why the
/// instance head is the wrong number to hand a projector.
pub async fn current(store: &dyn Store) -> Option<i64> {
    match store.max_event_id().await {
        Ok(id) => Some(RoomLsn::from_max_id(id).as_i64()),
        Err(err) => {
            tracing::warn!(error = %err, "room lsn: max_event_id failed");
            None
        }
    }
}

/// The head of **one room's** log (Cluster 398.8).
///
/// The header answers "how far behind is my projector?", which only works if the
/// number is comparable to a `log_id` the client has actually seen — and a client
/// only ever sees its own workspace's events. Reporting the instance-wide head
/// meant a fully caught-up projector could never reach it, because the gap was
/// other tenants' writes; it also told every tenant the instance's total event
/// volume, and rode outbound webhooks to third parties.
///
/// An empty room is `0`, matching `RoomLsn::from_max_id`.
pub async fn current_for_room(
    store: &dyn Store,
    workspace_id: maidan_types::WorkspaceId,
) -> Option<i64> {
    match store.workspace_event_head(workspace_id).await {
        Ok(head) => Some(RoomLsn::from_max_id(head.map(|l| l.id).unwrap_or(0)).as_i64()),
        Err(err) => {
            tracing::warn!(error = %err, "room lsn: workspace_event_head failed");
            None
        }
    }
}

fn insert_header(headers: &mut axum::http::HeaderMap, room_lsn: i64) {
    if let Ok(value) = HeaderValue::from_str(&room_lsn.to_string()) {
        headers.insert(HeaderName::from_static(ROOM_LSN_HEADER), value);
    }
}

/// Stamp `Maidan-Room-LSN` on a response. Used by subscribe_ack callers that
/// already have the value, and by the middleware.
pub fn stamp(headers: &mut axum::http::HeaderMap, room_lsn: i64) {
    insert_header(headers, room_lsn);
}

/// Whether a response is one we should not spend an event-log read on
/// (Cluster 397.9).
///
/// A rejected request has no room head to report, and querying for one hands an
/// unauthenticated caller a database round-trip per attempt. This layer also
/// used to sit *outside* the rate limiter, so a 429 — the response whose entire
/// job is to stop work — still paid for a `MAX(id)`. The limiter is now the
/// outer layer, and this is the belt to that braces: 401/403 come from auth
/// middleware further in, which the limiter does not shield.
fn skip_status(status: axum::http::StatusCode) -> bool {
    matches!(
        status,
        axum::http::StatusCode::UNAUTHORIZED
            | axum::http::StatusCode::FORBIDDEN
            | axum::http::StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
}

/// Always-on companion to [`crate::consistency::middleware`]. Queries
/// `Store::max_event_id` after the handler (the room head at response time).
/// Skips liveness/metrics/static/docs paths so a process-alive probe never
/// waits on the event log, and skips rejected responses so a refused request
/// costs no read.
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let skip = skip_path(req.uri().path());
    let mut resp = next.run(req).await;
    if skip || skip_status(resp.status()) {
        return resp;
    }
    // Scoped to the caller's room (Cluster 398.8). No scope means the response
    // was not produced for an authenticated room, so there is no head to report
    // — which also keeps the header off pre-auth responses.
    let scope = resp.extensions().get::<RoomScope>().copied();
    let lsn = match scope {
        Some(RoomScope(workspace_id)) => current_for_room(state.store.as_ref(), workspace_id).await,
        // `AUTH_DISABLED` resolves every caller to the cross-workspace bypass
        // context, which has no room — but it also means the deployment is
        // single-tenant by configuration, so the instance head *is* the room
        // head. Outside that, no scope means no authenticated room and the
        // header is omitted rather than guessed.
        None if state.auth_disabled => current(state.store.as_ref()).await,
        None => return resp,
    };
    if let Some(lsn) = lsn {
        stamp(resp.headers_mut(), lsn);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_and_metrics_are_skipped() {
        assert!(skip_path("/health"));
        assert!(skip_path("/health/live"));
        assert!(skip_path("/metrics"));
        assert!(skip_path("/openapi.json"));
        assert!(skip_path("/.well-known/maidan.json"));
        assert!(skip_path("/ui"));
        assert!(!skip_path("/ui/api/workspaces/x/events"));
        assert!(!skip_path("/workspaces/x/events"));
        assert!(!skip_path("/ws/subscribe"));
        assert!(!skip_path("/mcp/stream"));
        assert!(!skip_path("/a2a/v1/rpc"));
        assert!(!skip_path("/integrations/slack/events"));
    }

    #[test]
    fn rejected_responses_do_not_pay_for_an_event_log_read() {
        use axum::http::StatusCode;
        assert!(skip_status(StatusCode::UNAUTHORIZED));
        assert!(skip_status(StatusCode::FORBIDDEN));
        assert!(skip_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(skip_status(StatusCode::INTERNAL_SERVER_ERROR));
        // A real answer still reports the head.
        assert!(!skip_status(StatusCode::OK));
        assert!(!skip_status(StatusCode::CREATED));
        assert!(!skip_status(StatusCode::NOT_FOUND));
        assert!(!skip_status(StatusCode::CONFLICT));
    }
}
