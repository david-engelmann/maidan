//! The progress clock behind stuck detection, shared by both backends'
//! board reads.
//!
//! A board needs to tell a quiet agent from a working one without trusting
//! the agent to say so, and the lease cannot tell it: a hung agent whose
//! heartbeat still runs keeps renewing. So progress is only what the holder
//! did on the thread itself. Whether that progress is old enough to call the
//! thread stalled is the server's policy (`MAIDAN_STALL_SECS`), not the
//! store's, so the store reports the instant and the server judges it.

use chrono::{DateTime, Utc};
use maidan_types::{ClaimStallReason, Thread, ThreadState};

/// Columns a board read selects beside the thread row, with the thread
/// aliased `t`: the holder's latest message on the thread and its active
/// declaration. Guarded by the holder so an unheld thread costs no lookup.
pub(crate) const PROGRESS_COLUMNS: &str = "
                CASE WHEN t.assignee_id IS NOT NULL AND t.work_started_at IS NOT NULL THEN (
                   SELECT MAX(m.posted_at) FROM maidan_messages m
                   WHERE m.thread_id = t.id AND m.author_id = t.assignee_id
                 ) END AS holder_posted_at,
                CASE WHEN t.assignee_id IS NOT NULL AND t.work_started_at IS NOT NULL THEN (
                   SELECT s.declared_at FROM maidan_thread_status s
                   WHERE s.thread_id = t.id AND s.declared_by = t.assignee_id
                 ) END AS holder_declared_at";

/// Set [`Thread::last_progress_at`] from the row's thread and the two
/// [`PROGRESS_COLUMNS`]. Only an open thread held on a lease its holder
/// acknowledged has a progress clock: a lease-less assignment is a person's,
/// and an unacknowledged claim is the reaper's `ClaimUnacknowledged`.
pub(crate) fn set_last_progress(
    thread: &mut Thread,
    holder_posted_at: Option<DateTime<Utc>>,
    holder_declared_at: Option<DateTime<Utc>>,
) {
    let working = thread.state == ThreadState::Open
        && thread.assignee_id.is_some()
        && thread.assignment_expires_at.is_some();
    thread.last_progress_at = match (working, thread.work_started_at) {
        (true, Some(started)) => [holder_posted_at, holder_declared_at]
            .into_iter()
            .flatten()
            .fold(started, DateTime::max)
            .into(),
        _ => None,
    };
}

/// The holder's active declaration beside the thread row, aliased like
/// [`PROGRESS_COLUMNS`]: a holder that said it is waiting is not stalled.
pub(crate) const HOLDER_STATUS_COLUMN: &str = "
                (SELECT s.status FROM maidan_thread_status s
                 WHERE s.thread_id = t.id AND s.declared_by = t.assignee_id) AS holder_status";

/// A candidate row read by `report_stalled_claims` on either backend.
pub(crate) struct StallCandidate {
    pub work_started_at: Option<DateTime<Utc>>,
    pub holder_posted_at: Option<DateTime<Utc>>,
    pub holder_declared_at: Option<DateTime<Utc>>,
    pub holder_status: Option<String>,
}

impl StallCandidate {
    /// When the holder last showed progress: its acknowledgement, own message
    /// or own declaration. `None` while the claim is unacknowledged.
    pub fn last_progress_at(&self) -> Option<DateTime<Utc>> {
        self.work_started_at.map(|started| {
            [self.holder_posted_at, self.holder_declared_at]
                .into_iter()
                .flatten()
                .fold(started, DateTime::max)
        })
    }

    /// Whether this live claim is due a `ClaimStalled` for `reason`. The SQL
    /// already kept leases that lapse before `cutoff` for
    /// [`ClaimStallReason::LeaseLapsing`]; [`ClaimStallReason::NoProgress`]
    /// needs progress older than `cutoff` and no declared wait.
    pub fn is_due(&self, reason: ClaimStallReason, cutoff: DateTime<Utc>) -> bool {
        match reason {
            ClaimStallReason::LeaseLapsing => true,
            ClaimStallReason::NoProgress => {
                let quiet = self
                    .last_progress_at()
                    .is_some_and(|progress| progress < cutoff);
                let waiting = self
                    .holder_status
                    .as_deref()
                    .is_some_and(|status| status != "working");
                quiet && !waiting
            }
        }
    }
}

/// The per-claim fencing-token column that records `reason` was reported, and
/// the extra filter its scan adds (bound to the cutoff).
pub(crate) fn stall_scan(reason: ClaimStallReason) -> (&'static str, &'static str) {
    match reason {
        ClaimStallReason::NoProgress => (
            "stall_reported_lease_id",
            "t.work_started_at IS NOT NULL AND t.work_started_at < ",
        ),
        ClaimStallReason::LeaseLapsing => ("lapse_warned_lease_id", "t.assignment_expires_at < "),
    }
}
