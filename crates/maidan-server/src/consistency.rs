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
use maidan_types::Lsn;

use crate::state::AppState;

/// The response/request header carrying the WAL-LSN causality token.
pub const CONSISTENCY_TOKEN_HEADER: &str = "maidan-consistency-token";

fn is_mutation(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    )
}

/// Read-replica causality middleware (Clusters 263–264):
/// - **GET/HEAD** requests run inside a read-consistency scope so store reads can
///   route to the replica, honoring the client's `Maidan-Consistency-Token` (a
///   token routes to the replica only once it has replayed that far, else the
///   primary — read-your-writes). Mutation handlers are *not* scoped, so any
///   read-then-write inside them stays on the primary.
/// - **mutations** stamp the primary's current WAL LSN on a 2xx response as
///   `Maidan-Consistency-Token`, the token a client echoes on its next read.
///
/// All of this is gated on a configured read replica (`read_replica_enabled`); with
/// no replica it is a pure pass-through (no header parse, no LSN round-trip).
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if !state.read_replica_enabled {
        return next.run(req).await;
    }

    let method = req.method().clone();

    if matches!(method, Method::GET | Method::HEAD) {
        // Scope reads to the client's causality token (if any) so the store can
        // route them to the replica when it has caught up.
        let token = req
            .headers()
            .get(CONSISTENCY_TOKEN_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(Lsn::from_pg_str);
        return maidan_store::postgres::with_read_consistency(token, next.run(req)).await;
    }

    let mut resp = next.run(req).await;
    if is_mutation(&method) && resp.status().is_success() {
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
