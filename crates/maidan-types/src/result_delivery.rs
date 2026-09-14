//! Result-delivery state (Cluster 379.1) — one row per `(thread, target)`.
//!
//! Cluster 377's `maidan_egress_outbox` is *transport*: claim, retry, backoff,
//! dead-letter. This is *intent and identity*, and it is a separate table
//! because it answers three questions the queue cannot:
//!
//! 1. **Who delivers?** The notification router runs on every replica, so one
//!    `ThreadResultSet` reaches all of them. [`ResultDelivery`] is the shared row
//!    they contend for, so exactly one wins — the Cluster-238 lesson, which cost
//!    a 3× duplicate delivery when it was learned the first time.
//! 2. **Is this new?** [`ResultDelivery::armed_revision`] is the newest
//!    `ThreadResult::produced_at` this row has ever accepted. A *newer* result
//!    re-arms it; one already seen is a no-op. That is what makes a re-review an
//!    update and a replayed event a nothing.
//! 3. **Update what?** [`ResultDelivery::external_ref`] is the Cluster-378.2
//!    [`ExternalRef::handle`](crate::ExternalRef::handle) — a Slack `ts`, a
//!    GitHub comment id.
//!
//! **Why two revision watermarks.** `armed_revision` (seen) and
//! `delivered_revision` (landed) look redundant until you try to arm with one.
//! Comparing only against `delivered_revision` cannot tell a second replica
//! reporting the *same* revision — where both read `NULL` and one must lose —
//! from a genuinely *newer* result arriving while a delivery is still in flight,
//! which must win or that result is silently dropped. Arming is therefore a
//! single monotonic test against `armed_revision`, and `delivered_revision`
//! stays a truthful record of what actually reached the surface.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::egress::{EgressSurface, EgressTarget, ExternalRef};
use crate::ids::{ResultDeliveryId, ThreadId};

/// How one target's delivery currently stands.
///
/// Stored as text and read back as text, for the reason [`crate::AllowedEgressTarget`]
/// does the same: a row written by a newer build must stay *readable* by an older
/// one, because this is the row a producer reads to find out where its result
/// went. Writes go through the typed store methods, so only these values are ever
/// written.
pub mod status {
    /// Armed: a revision is waiting to be delivered.
    pub const PENDING: &str = "pending";
    /// Delivered, and `external_ref` is how to edit it next time.
    pub const DELIVERED: &str = "delivered";
    /// The transport gave up (the egress queue dead-lettered it).
    pub const FAILED: &str = "failed";
    /// Deliberately not delivered — an unknown surface, or a target the
    /// workspace has not blessed. **Not an error**: `docs/Result Delivery.md`
    /// makes "delivered nowhere" a normal outcome, so it is recorded rather than
    /// hidden, and `last_error` carries the reason for the producer to read.
    pub const SKIPPED: &str = "skipped";
}

/// One `(thread, target)` delivery's durable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResultDelivery {
    pub id: ResultDeliveryId,
    pub thread_id: ThreadId,
    pub surface: String,
    pub selector: String,
    pub status: String,
    /// The external object we created, if we could address it — a Slack `ts`, a
    /// GitHub comment id. `None` means "post next time", with the hidden body
    /// marker as the recovery path rather than a guess.
    pub external_ref: Option<String>,
    /// The newest `ThreadResult::produced_at` this row has accepted. Monotonic:
    /// arming only ever moves it forward, which is the dedup.
    pub armed_revision: DateTime<Utc>,
    /// The newest revision that actually reached the surface. `None` = never
    /// delivered.
    pub delivered_revision: Option<DateTime<Utc>>,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ResultDelivery {
    /// The typed destination, or `None` when the stored pair does not decode —
    /// the same tolerance [`crate::EgressOutbox::target`] has, and for the same
    /// reason: a row an older build cannot read must still be *listable* by the
    /// delivery-status API rather than sinking the whole response.
    pub fn target(&self) -> Option<EgressTarget> {
        EgressTarget::parse(EgressSurface::parse(&self.surface)?, &self.selector)
    }

    /// The handle on the external object, rebuilt against this row's target.
    /// `None` when nothing was stored or the pair/handle does not decode, which
    /// is the caller's cue to post rather than update.
    pub fn reference(&self) -> Option<ExternalRef> {
        ExternalRef::for_target(&self.target()?, self.external_ref.as_deref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(surface: &str, selector: &str, external_ref: Option<&str>) -> ResultDelivery {
        ResultDelivery {
            id: ResultDeliveryId::new(),
            thread_id: ThreadId::new(),
            surface: surface.into(),
            selector: selector.into(),
            status: status::DELIVERED.into(),
            external_ref: external_ref.map(str::to_string),
            armed_revision: Utc::now(),
            delivered_revision: None,
            attempts: 1,
            last_error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn a_stored_row_decodes_back_to_its_target_and_reference() {
        let gh = row("github", "example/repo#42", Some("998877"));
        assert_eq!(
            gh.target(),
            Some(EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 3915
            })
        );
        assert_eq!(
            gh.reference(),
            Some(ExternalRef::Github {
                repo: "example/repo".into(),
                comment_id: 998877
            }),
            "the repo comes from the target, the comment id from the stored handle"
        );

        let slack = row("slack", "C0123ABCDEF", Some("1699999999.001200"));
        assert_eq!(
            slack.reference(),
            Some(ExternalRef::Slack {
                channel_id: "C0123ABCDEF".into(),
                ts: "1699999999.001200".into()
            })
        );
    }

    #[test]
    fn a_row_with_no_usable_handle_has_no_reference_so_the_caller_posts() {
        // Never delivered.
        assert_eq!(row("github", "example/repo#1", None).reference(), None);
        // Delivered, but the handle is junk — post rather than PATCH a guess.
        assert_eq!(
            row("github", "example/repo#1", Some("not-an-id")).reference(),
            None
        );
        // A surface this build does not know: no target, so no reference, and
        // `target()` says so without panicking.
        let unknown = row("discord", "whatever", Some("123"));
        assert_eq!(unknown.target(), None);
        assert_eq!(unknown.reference(), None);
    }
}
