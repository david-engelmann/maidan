//! Idempotency keys for retried writes: the types the store trades in.
//!
//! A caller sends `Idempotency-Key` on a write. The first request with a key
//! reserves it; the response it produces is stored, and a retry with the same
//! key and the same request gets that response back instead of running again.
//! A different request under the same key is refused, and so is a retry while
//! the first is still running.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, WorkspaceId};

/// A reservation request: this caller wants to run this request under `key`.
#[derive(Debug, Clone)]
pub struct NewIdempotencyKey {
    pub workspace_id: WorkspaceId,
    pub actor_id: MemberId,
    pub key: String,
    /// SHA-256 over the request's method, path, query and body.
    pub fingerprint: String,
    /// How long the first request may hold the key before a retry may take
    /// it over (a crashed request must not hold it until `expires_at`).
    pub locked_until: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// A response kept for replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// What a reservation found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyReservation {
    /// The key is this request's: run it, then complete or release.
    Reserved,
    /// Another request holds the key and has not finished.
    InFlight { fingerprint: String },
    /// A request finished under this key; this is its response.
    Completed {
        fingerprint: String,
        response: StoredResponse,
    },
}

/// The row a key resolves to, as both backends read it.
pub(crate) type KeyRow = (String, Option<i32>, Option<String>, Option<Vec<u8>>);

pub(crate) fn reservation_from(row: KeyRow) -> IdempotencyReservation {
    match row {
        (fingerprint, Some(status), content_type, body) => IdempotencyReservation::Completed {
            fingerprint,
            response: StoredResponse {
                status: u16::try_from(status).unwrap_or(500),
                content_type,
                body: body.unwrap_or_default(),
            },
        },
        (fingerprint, None, _, _) => IdempotencyReservation::InFlight { fingerprint },
    }
}

/// How many lapsed rows one reservation clears on its way in, so the table
/// stays bounded without a separate sweeper and no reservation pays for a
/// large backlog.
pub(crate) const PRUNE_BATCH: i64 = 100;
