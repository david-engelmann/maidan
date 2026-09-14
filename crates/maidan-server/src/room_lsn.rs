//! Projector / broadcast lag token (Cluster 390, Wave 3 #30).
//!
//! Stamps `Maidan-Room-LSN` with the event-log high-water (`MAX(id)`, `0` when
//! empty) so clients can compare last-seen `log_id` to the room head.
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

/// Best-effort room head. `None` if the store read fails (fail-open: no header).
pub async fn current(store: &dyn Store) -> Option<i64> {
    match store.max_event_id().await {
        Ok(id) => Some(RoomLsn::from_max_id(id).as_i64()),
        Err(err) => {
            tracing::warn!(error = %err, "room lsn: max_event_id failed");
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

/// Always-on companion to [`crate::consistency::middleware`]. Queries
/// `Store::max_event_id` after the handler (the room head at response time).
/// Skips liveness/metrics/static/docs paths so a process-alive probe never
/// waits on the event log.
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let skip = skip_path(req.uri().path());
    let mut resp = next.run(req).await;
    if skip {
        return resp;
    }
    if let Some(lsn) = current(state.store.as_ref()).await {
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
}
