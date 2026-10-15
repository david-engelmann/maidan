//! Cancelling a task: closing it without its work being done.
//!
//! A cancel closes the thread, parks it from dispatch with the reason, and
//! ends any live claim in one transaction, so the worker holding it learns on
//! its next renew. Either a caller with `thread:transition` may cancel, or the
//! member who created the thread ([`crate::Thread::created_by`]): a producer
//! may take back its own task without being able to move anyone else's.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::events::StoredEvent;
use crate::ids::{MemberId, ThreadId};
use crate::models::{Thread, ThreadState};

/// The longest cancel reason, in characters.
pub const CANCEL_REASON_MAX_CHARS: usize = 500;

/// What lets the caller cancel. Capabilities are the server's to read; the
/// store only needs to know whether to hold the caller to the creator rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelAuthority {
    /// The caller holds `thread:transition`, which may close any thread it
    /// can reach.
    Transition,
    /// The caller may cancel only a thread it created.
    Creator,
}

/// The record of a cancel, kept so a repeat answers with the first one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadCancellation {
    pub thread_id: ThreadId,
    pub reason: String,
    pub cancelled_by: MemberId,
    pub cancelled_at: DateTime<Utc>,
}

/// A cancelled thread: the closed row and why it was cancelled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CancelledThread {
    pub thread: Thread,
    pub cancellation: ThreadCancellation,
}

/// What a cancel did.
#[derive(Debug)]
pub enum CancelOutcome {
    /// The thread is cancelled now. The events committed with it, to publish:
    /// `ThreadStateChanged`, `ThreadCancelled` and the notice's
    /// `MessagePosted`.
    Cancelled {
        cancelled: CancelledThread,
        events: Vec<StoredEvent>,
    },
    /// It was cancelled before; nothing changed, and the first cancel stands.
    AlreadyCancelled(CancelledThread),
    /// The caller may cancel only threads it created, and did not create
    /// this one.
    NotCreator,
    /// It already reached `state` some other way, so there is nothing to
    /// cancel.
    AlreadyFinished(ThreadState),
}

/// Trim a cancel reason and check it is 1 to [`CANCEL_REASON_MAX_CHARS`]
/// characters, so every surface refuses the same reasons.
pub fn validated_cancel_reason(reason: &str) -> Result<&str, String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("reason must not be empty".into());
    }
    if reason.chars().count() > CANCEL_REASON_MAX_CHARS {
        return Err(format!(
            "reason must be at most {CANCEL_REASON_MAX_CHARS} characters"
        ));
    }
    Ok(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_trimmed_and_bounded() {
        assert_eq!(validated_cancel_reason("  superseded  "), Ok("superseded"));
        assert!(validated_cancel_reason("   ").is_err());
        let at_limit = "a".repeat(CANCEL_REASON_MAX_CHARS);
        assert!(validated_cancel_reason(&at_limit).is_ok());
        let over = "é".repeat(CANCEL_REASON_MAX_CHARS + 1);
        assert!(validated_cancel_reason(&over).is_err());
    }
}
