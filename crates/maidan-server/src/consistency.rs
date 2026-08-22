//! Read-replica causality token (Cluster 263, Program D).
//!
//! After a successful mutating request, stamp the primary's current WAL LSN on the
//! response as `Maidan-Consistency-Token`. A client echoes it on a later read, and
//! the read router (Cluster 264) serves that read from a replica only once the
//! replica has replayed past the token — otherwise the primary. This is what makes
//! replica reads safe: a client never reads staler than its own writes.
//!
//! The LSN is captured *after* the handler, so it may be a little *ahead* of the
//! write's exact commit LSN (another write can advance it in between). That is
//! safe — the token is never *behind* the write, so a read gated on it is never
//! stale; at worst it waits slightly longer or falls back to the primary.

use axum::{
    extract::{Request, State},
    http::{header::HeaderName, HeaderValue, Method},
    middleware::Next,
    response::Response,
};

use crate::state::AppState;

/// The response/request header carrying the WAL-LSN causality token.
pub const CONSISTENCY_TOKEN_HEADER: &str = "maidan-consistency-token";

fn is_mutation(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    )
}

/// Stamp `Maidan-Consistency-Token` on successful mutations when a read replica is
/// configured. A no-op otherwise (no replica → the token is unused, so skip the
/// extra `pg_current_wal_lsn()` round-trip entirely).
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mutation = is_mutation(req.method());
    let mut resp = next.run(req).await;

    if state.read_replica_enabled && mutation && resp.status().is_success() {
        match state.store.write_lsn().await {
            Ok(Some(lsn)) => {
                if let Ok(value) = HeaderValue::from_str(&lsn.to_pg_str()) {
                    resp.headers_mut()
                        .insert(HeaderName::from_static(CONSISTENCY_TOKEN_HEADER), value);
                }
            }
            Ok(None) => {} // backend without an LSN (SQLite) — no token
            Err(err) => tracing::warn!(error = %err, "consistency token: write_lsn failed"),
        }
    }
    resp
}
