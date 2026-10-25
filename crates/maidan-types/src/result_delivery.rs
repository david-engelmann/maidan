//! Result-delivery state — one row per `(thread, target)`.
//!
//! The `maidan_egress_outbox` is *transport*: claim, retry, backoff,
//! dead-letter. This is *intent and identity*, and it is a separate table
//! because it answers three questions the queue cannot:
//!
//! 1. **Who delivers?** The notification router runs on every replica, so one
//!    `ThreadResultSet` reaches all of them. [`ResultDelivery`] is the shared row
//!    they contend for, so exactly one wins — the lesson, which cost
//!    a 3× duplicate delivery when it was learned the first time.
//! 2. **Is this new?** [`ResultDelivery::armed_revision`] is the newest
//!    `ThreadResult::produced_at` this row has ever accepted. A *newer* result
//!    re-arms it; one already seen is a no-op. That is what makes a re-review an
//!    update and a replayed event a nothing.
//! 3. **Update what?** [`ResultDelivery::external_ref`] is the
//!    [`ExternalRef::handle`](crate::ExternalRef::handle) — a Slack `ts`, a
//!    GitHub comment id.
//!
//! **Why two revision watermarks.** `armed_revision` (seen) and
//! `delivered_revision` (landed) look redundant until you try to arm with one.
//! Comparing only against `delivered_revision` cannot tell a second replica
//! reporting the *same* revision — where both read `NULL` and one must lose —
//! from a genuinely *newer* result arriving while a delivery is still in
//! flight, which must win or that result is silently dropped. Arming is
//! therefore a single monotonic test against `armed_revision`, and
//! `delivered_revision` stays a truthful record of what actually reached the
//! surface.

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
    /// The pull request a `github_branch` delivery landed as, decoded from the
    /// `<commit_sha>#<pull_number>` handle in [`Self::external_ref`]. `None`
    /// for every other surface, and for a branch row whose handle does not
    /// decode. Derived when a row is materialized from storage (see
    /// [`Self::derive_pull_fields`]); never persisted.
    #[serde(default)]
    pub pr_number: Option<i64>,
    /// The commit a `github_branch` delivery landed, decoded from the same
    /// handle as [`Self::pr_number`].
    #[serde(default)]
    pub head_sha: Option<String>,
    /// The pull request's web URL, built from the target's repository and
    /// [`Self::pr_number`]. Derived alongside it; `None` whenever it is.
    #[serde(default)]
    pub pr_url: Option<String>,
    /// The newest `ThreadResult::produced_at` this row has accepted. Monotonic:
    /// arming only ever moves it forward, which is the dedup.
    pub armed_revision: DateTime<Utc>,
    /// The newest revision that actually reached the surface. `None` = never
    /// delivered.
    pub delivered_revision: Option<DateTime<Utc>>,
    pub attempts: i64,
    pub last_error: Option<String>,
    /// For a change: what its pull request lacks against the producer's
    /// template (`missing_testing`, `missing_issue_key`). A flag, not a
    /// failure: the change landed either way. Absent when nothing is missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub template_warnings: Vec<String>,
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

    /// Decode the stored handle into the derived pull-request fields
    /// ([`Self::pr_number`], [`Self::head_sha`], [`Self::pr_url`]). Only a
    /// `github_branch` row whose handle decodes has them; anything else keeps
    /// the `None`s. The store calls this wherever it materializes a row, so a
    /// read never has to re-derive — and a row built in memory (a test, a
    /// replay in flight) can call it too rather than copying the decode.
    pub fn derive_pull_fields(&mut self) {
        if let Some(ExternalRef::GithubBranch {
            repo,
            commit_sha,
            pull_number,
        }) = self.reference()
        {
            self.pr_number = Some(pull_number);
            self.head_sha = Some(commit_sha);
            self.pr_url = Some(format!("https://github.com/{repo}/pull/{pull_number}"));
        }
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
            pr_number: None,
            head_sha: None,
            pr_url: None,
            armed_revision: Utc::now(),
            delivered_revision: None,
            attempts: 1,
            last_error: None,
            template_warnings: Vec::new(),
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
                issue_number: 42
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
    fn a_branch_row_derives_its_pull_request_fields_from_the_handle() {
        const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
        let mut branch = row(
            "github_branch",
            "example/repo@feature",
            Some(&format!("{SHA}#42")),
        );
        branch.derive_pull_fields();
        assert_eq!(branch.pr_number, Some(42));
        assert_eq!(branch.head_sha.as_deref(), Some(SHA));
        assert_eq!(
            branch.pr_url.as_deref(),
            Some("https://github.com/example/repo/pull/42")
        );
        // The raw handle stays as stored — the fields are a view of it.
        assert_eq!(
            branch.external_ref.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567#42")
        );

        // The derived triple serializes alongside the row; a non-branch row
        // serializes the same fields as nulls.
        let value = serde_json::to_value(&branch).unwrap();
        assert_eq!(value["pr_number"], serde_json::json!(42));
        assert_eq!(
            value["pr_url"],
            serde_json::json!("https://github.com/example/repo/pull/42")
        );
        let slack = row("slack", "C0123ABCDEF", Some("1699999999.001200"));
        let value = serde_json::to_value(&slack).unwrap();
        assert_eq!(value["pr_number"], serde_json::Value::Null);
        assert_eq!(value["head_sha"], serde_json::Value::Null);
        assert_eq!(value["pr_url"], serde_json::Value::Null);
    }

    #[test]
    fn a_branch_row_with_an_undecodable_handle_derives_nothing() {
        for handle in [
            None,
            Some("not-a-handle"),
            Some("abc123#42"),
            Some("0123456789abcdef0123456789abcdef01234567#0"),
        ] {
            let mut branch = row("github_branch", "example/repo@feature", handle);
            branch.derive_pull_fields();
            assert_eq!(branch.pr_number, None, "handle {handle:?}");
            assert_eq!(branch.head_sha, None, "handle {handle:?}");
            assert_eq!(branch.pr_url, None, "handle {handle:?}");
        }
        // A comment delivery's numeric handle is not a pull reference either.
        let mut gh = row("github", "example/repo#42", Some("998877"));
        gh.derive_pull_fields();
        assert_eq!(gh.pr_number, None);
        assert_eq!(gh.pr_url, None);
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
