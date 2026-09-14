//! Durable projector egress (Cluster 377).
//!
//! The Slack (309) and GitHub (312) projectors posted inline and best-effort: a
//! transient 502 dropped the message with a log line and nothing else. A message
//! bound for an external surface is now enqueued in `maidan_egress_outbox` and
//! delivered by a retry/backoff worker — the shape the mail outbox (304) already
//! has.
//!
//! A destination is a typed [`EgressTarget`], persisted as the `(surface,
//! selector)` text pair the Cluster-378 allowlist will also key on. The pair
//! always round-trips: the store only ever writes what [`EgressTarget::selector`]
//! produced, and the worker decodes it back with [`EgressTarget::parse`].

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{EgressOutboxId, EgressTargetId, ThreadId, WorkspaceId};

/// An external surface a projector can deliver to. Lowercase identifiers, as in
/// the `deliver_to` grammar pinned in `docs/Result Delivery.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum EgressSurface {
    Slack,
    Github,
}

impl EgressSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Github => "github",
        }
    }

    /// Decode a persisted discriminator. `None` for anything this build does not
    /// know — a surface added by a newer version, read after a downgrade.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "slack" => Some(Self::Slack),
            "github" => Some(Self::Github),
            _ => None,
        }
    }
}

impl fmt::Display for EgressSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a row is on the egress outbox (Cluster 379.4).
///
/// Projector posts and result deliveries can aim at the same GitHub issue (a
/// linked thread *and* a `deliver_to` target). Update-in-place is a result
/// behaviour — without this discriminator a projector `MessagePosted` would
/// PATCH the result comment. Unknown values decode as [`Self::Projector`]:
/// posting a second comment is the conservative direction; editing the wrong
/// object is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EgressKind {
    #[default]
    Projector,
    Result,
}

impl EgressKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Projector => "projector",
            Self::Result => "result",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "result" => Self::Result,
            _ => Self::Projector,
        }
    }
}

impl fmt::Display for EgressKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where one queued delivery goes, with the per-surface detail the sender needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressTarget {
    /// A Slack channel id (`C…`/`G…` in practice — the projector takes whatever
    /// the operator linked, so the id is not re-validated here; the `deliver_to`
    /// reader is where a producer-supplied channel is checked).
    Slack { channel_id: String },
    /// A GitHub issue or PR comment. `repo` is `owner/name`; issue and PR numbers
    /// share one namespace, so a PR links exactly like an issue.
    Github { repo: String, issue_number: i64 },
}

impl EgressTarget {
    pub fn surface(&self) -> EgressSurface {
        match self {
            Self::Slack { .. } => EgressSurface::Slack,
            Self::Github { .. } => EgressSurface::Github,
        }
    }

    /// The persisted per-surface detail: a Slack channel id, or `owner/name#123`.
    pub fn selector(&self) -> String {
        match self {
            Self::Slack { channel_id } => channel_id.clone(),
            Self::Github { repo, issue_number } => format!("{repo}#{issue_number}"),
        }
    }

    /// The key this target is *authorized* by in the Cluster-378 allowlist, which
    /// is deliberately coarser than [`Self::selector`] on GitHub: an operator
    /// blesses the **repository**, not each issue, because per-issue blessing
    /// would mean an operator ticket per PR. Slack has no such split — a channel
    /// id is already the unit an operator thinks in.
    pub fn allowlist_selector(&self) -> String {
        match self {
            Self::Slack { channel_id } => channel_id.clone(),
            Self::Github { repo, .. } => repo.clone(),
        }
    }

    /// Decode a persisted `(surface, selector)` pair. `None` when the selector is
    /// malformed for its surface, which is the caller's cue to dead-letter the row
    /// rather than retry it forever.
    pub fn parse(surface: EgressSurface, selector: &str) -> Option<Self> {
        match surface {
            EgressSurface::Slack => (!selector.is_empty()).then(|| Self::Slack {
                channel_id: selector.to_string(),
            }),
            EgressSurface::Github => {
                let (repo, number) = selector.rsplit_once('#')?;
                let issue_number: i64 = number.parse().ok()?;
                // `owner/name` with both halves present, and a real issue number.
                let (owner, name) = repo.split_once('/')?;
                (!owner.is_empty() && !name.is_empty() && issue_number > 0).then(|| Self::Github {
                    repo: repo.to_string(),
                    issue_number,
                })
            }
        }
    }
}

impl fmt::Display for EgressTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.surface(), self.selector())
    }
}

/// A handle on an object a sender created on an external surface (Cluster 378.2)
/// — a Slack message's `ts`, a GitHub comment's id. It is what makes a re-delivery
/// an **update in place** rather than a second comment.
///
/// Only [`Self::handle`] needs persisting: a delivery row already carries its
/// [`EgressTarget`], and everything else here is derivable from it. So the stored
/// shape is one text column, reconstructed with [`Self::for_target`] — the same
/// "store the narrow thing, decode it back" move as `(surface, selector)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalRef {
    /// `chat.update` addresses a message by channel **and** `ts`; the `ts` alone
    /// is not enough.
    Slack { channel_id: String, ts: String },
    /// `PATCH /repos/{repo}/issues/comments/{id}` addresses a comment by
    /// repository and comment id — **not** by issue. The issue number the comment
    /// hangs under is deliberately absent: it is not needed to edit the comment,
    /// and carrying it would invite keying an update on the wrong thing.
    Github { repo: String, comment_id: i64 },
}

impl ExternalRef {
    pub fn surface(&self) -> EgressSurface {
        match self {
            Self::Slack { .. } => EgressSurface::Slack,
            Self::Github { .. } => EgressSurface::Github,
        }
    }

    /// The part a delivery row has to remember: the Slack `ts`, or the GitHub
    /// comment id as text.
    pub fn handle(&self) -> String {
        match self {
            Self::Slack { ts, .. } => ts.clone(),
            Self::Github { comment_id, .. } => comment_id.to_string(),
        }
    }

    /// Rebuild a ref from the delivery's target and the stored handle. `None` when
    /// the handle is malformed for its surface — the caller's cue to treat the ref
    /// as lost and fall back to posting (with the hidden-marker recovery path on
    /// GitHub) rather than issuing an update against a guess.
    pub fn for_target(target: &EgressTarget, handle: &str) -> Option<Self> {
        match target {
            EgressTarget::Slack { channel_id } => (!handle.is_empty()).then(|| Self::Slack {
                channel_id: channel_id.clone(),
                ts: handle.to_string(),
            }),
            EgressTarget::Github { repo, .. } => {
                let comment_id: i64 = handle.parse().ok()?;
                (comment_id > 0).then(|| Self::Github {
                    repo: repo.clone(),
                    comment_id,
                })
            }
        }
    }
}

/// A destination a workspace's operator has blessed for egress (Cluster 378.1).
///
/// `surface` is stored as text rather than an [`EgressSurface`], for the same
/// reason [`EgressOutbox`] does: a row written by a newer build and read after a
/// downgrade must still be *listable*, or the operator cannot see the entry they
/// need to revoke. The write side is typed ([`NewEgressTarget`]), so a surface
/// this build cannot deliver to can never be blessed in the first place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AllowedEgressTarget {
    pub id: EgressTargetId,
    pub workspace_id: WorkspaceId,
    pub surface: String,
    pub selector: String,
    pub created_at: DateTime<Utc>,
}

/// A destination to bless. Typed on the way in — see [`AllowedEgressTarget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEgressTarget {
    pub workspace_id: WorkspaceId,
    pub surface: EgressSurface,
    pub selector: String,
}

/// Validate an operator-supplied allowlist selector, returning why it was
/// rejected. Pure, so the store can enforce it on every write path and the
/// reasons can be unit-tested without a database.
///
/// The Slack rule is the load-bearing one: a channel **id** (`C…`/`G…`), never a
/// `#name`. Names are mutable and ambiguous, and an allowlist keyed on a mutable
/// name is not an allowlist — the channel a name points at can change under the
/// blessing. See `docs/Result Delivery.md`.
pub fn validate_allowlist_selector(
    surface: EgressSurface,
    selector: &str,
) -> Result<(), &'static str> {
    if selector.trim() != selector || selector.is_empty() {
        return Err("selector must be non-empty and free of surrounding whitespace");
    }
    match surface {
        EgressSurface::Slack => {
            if selector.starts_with('#') {
                return Err("slack selector must be a channel id (C…/G…), not a #name");
            }
            if !selector.starts_with('C') && !selector.starts_with('G') {
                return Err("slack selector must be a channel id starting with C or G");
            }
            if selector.len() < 2 {
                return Err("slack selector is too short to be a channel id");
            }
            Ok(())
        }
        EgressSurface::Github => {
            if selector.contains('#') {
                return Err(
                    "github selector is a repository `owner/name`, without an issue number",
                );
            }
            let Some((owner, name)) = selector.split_once('/') else {
                return Err("github selector must be `owner/name`");
            };
            if owner.is_empty() || name.is_empty() || name.contains('/') {
                return Err("github selector must be `owner/name`");
            }
            Ok(())
        }
    }
}

/// A new delivery to enqueue (Cluster 377.1). Queued `pending`, due now.
///
/// `source_log_id` is the `maidan_events` row that caused the send. It is the
/// dedup key together with the target: every replica runs the notification
/// router's bus consumer, so all of them enqueue and exactly one row survives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEgressOutbox {
    pub workspace_id: WorkspaceId,
    pub thread_id: ThreadId,
    pub source_log_id: i64,
    pub target: EgressTarget,
    pub body: String,
    /// [`EgressKind::Projector`] for linked-thread relays; [`EgressKind::Result`]
    /// for Cluster 379 result delivery. The worker uses this to decide whether
    /// a stored [`ExternalRef`] is an object it may edit.
    pub kind: EgressKind,
}

/// A claimed delivery the egress worker will attempt (Cluster 377.1). `attempts`
/// includes the current claim; the queue's status / scheduling columns stay
/// internal to the store.
///
/// The destination is carried as the stored text pair rather than a decoded
/// [`EgressTarget`]: a row that does not decode has to reach the worker to be
/// dead-lettered, and a claim that failed to decode would instead lease the row
/// forward forever, wedging the queue behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressOutbox {
    pub id: EgressOutboxId,
    pub workspace_id: WorkspaceId,
    pub thread_id: ThreadId,
    pub surface: String,
    pub selector: String,
    pub body: String,
    pub attempts: i64,
    pub kind: EgressKind,
}

impl EgressOutbox {
    /// The typed destination, or `None` when the stored pair does not decode.
    pub fn target(&self) -> Option<EgressTarget> {
        EgressTarget::parse(EgressSurface::parse(&self.surface)?, &self.selector)
    }
}

/// A dead-lettered delivery for the operator DLQ view (Cluster 377.4): a message
/// that exhausted its retries, or whose link was disabled as misconfigured.
/// `last_error` is why the final attempt failed — the surface's own words.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeadEgress {
    pub id: EgressOutboxId,
    pub workspace_id: WorkspaceId,
    pub thread_id: ThreadId,
    pub surface: String,
    pub selector: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(target: &EgressTarget) -> Option<EgressTarget> {
        EgressTarget::parse(target.surface(), &target.selector())
    }

    #[test]
    fn a_target_round_trips_through_its_stored_pair() {
        for target in [
            EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
            },
            EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 3915,
            },
        ] {
            assert_eq!(round_trip(&target).as_ref(), Some(&target));
        }
    }

    #[test]
    fn a_repo_name_containing_a_hash_still_splits_on_the_last_one() {
        let target = EgressTarget::Github {
            repo: "owner/na#me".into(),
            issue_number: 7,
        };
        assert_eq!(target.selector(), "owner/na#me#7");
        assert_eq!(round_trip(&target).as_ref(), Some(&target));
    }

    #[test]
    fn a_malformed_selector_does_not_decode() {
        for selector in [
            "example/repo",     // no issue number
            "example/repo#",    // empty issue number
            "example/repo#nan", // non-numeric
            "example/repo#0",   // issue numbers start at 1
            "example/repo#-1",
            "widgets#12", // no owner
            "/widgets#12",
            "example/#12",
        ] {
            assert_eq!(
                EgressTarget::parse(EgressSurface::Github, selector),
                None,
                "expected {selector} to be rejected"
            );
        }
        assert_eq!(EgressTarget::parse(EgressSurface::Slack, ""), None);
    }

    #[test]
    fn an_unknown_surface_does_not_decode() {
        assert_eq!(EgressSurface::parse("discord"), None);
        let row = EgressOutbox {
            id: EgressOutboxId::new(),
            workspace_id: WorkspaceId::new(),
            thread_id: ThreadId::new(),
            surface: "discord".into(),
            selector: "whatever".into(),
            body: "hi".into(),
            attempts: 1,
            kind: EgressKind::Projector,
        };
        assert_eq!(row.target(), None);
    }

    #[test]
    fn an_unknown_outbox_kind_decodes_as_projector() {
        assert_eq!(EgressKind::parse("result"), EgressKind::Result);
        assert_eq!(EgressKind::parse("projector"), EgressKind::Projector);
        assert_eq!(
            EgressKind::parse("inline-comment"),
            EgressKind::Projector,
            "unknown must not take the update-in-place path"
        );
        assert_eq!(EgressKind::default(), EgressKind::Projector);
    }

    #[test]
    fn a_github_target_is_authorized_by_its_repository_not_its_issue() {
        let target = EgressTarget::Github {
            repo: "example/repo".into(),
            issue_number: 42,
        };
        assert_eq!(target.selector(), "example/repo#42");
        assert_eq!(target.allowlist_selector(), "example/repo");
        // The projection is what an operator's one blessing has to cover, so it
        // must also be a selector the allowlist would accept.
        assert!(
            validate_allowlist_selector(EgressSurface::Github, &target.allowlist_selector())
                .is_ok()
        );
    }

    #[test]
    fn a_slack_target_is_authorized_by_the_same_channel_id_it_delivers_to() {
        let target = EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into(),
        };
        assert_eq!(target.selector(), target.allowlist_selector());
        assert!(
            validate_allowlist_selector(EgressSurface::Slack, &target.allowlist_selector()).is_ok()
        );
    }

    #[test]
    fn an_allowlist_selector_must_be_an_id_not_a_name() {
        for (surface, selector) in [
            (EgressSurface::Slack, "#general"),
            (EgressSurface::Slack, "general"),
            (EgressSurface::Slack, "C"),
            (EgressSurface::Slack, ""),
            (EgressSurface::Slack, " C0123ABCDEF"),
            (EgressSurface::Slack, "C0123ABCDEF "),
            // An issue number is not part of the authorization grain.
            (EgressSurface::Github, "example/repo#42"),
            (EgressSurface::Github, "widgets"),
            (EgressSurface::Github, "/widgets"),
            (EgressSurface::Github, "example/"),
            (EgressSurface::Github, "example/repo/extra"),
            (EgressSurface::Github, ""),
        ] {
            assert!(
                validate_allowlist_selector(surface, selector).is_err(),
                "expected {surface}:{selector:?} to be rejected"
            );
        }
        for (surface, selector) in [
            (EgressSurface::Slack, "C0123ABCDEF"),
            (EgressSurface::Slack, "G0123ABCDEF"),
            (EgressSurface::Github, "example/repo"),
        ] {
            assert!(
                validate_allowlist_selector(surface, selector).is_ok(),
                "expected {surface}:{selector:?} to be accepted"
            );
        }
    }

    #[test]
    fn an_external_ref_round_trips_through_its_target_and_stored_handle() {
        let slack_target = EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into(),
        };
        let slack_ref = ExternalRef::Slack {
            channel_id: "C0123ABCDEF".into(),
            ts: "1699999999.001200".into(),
        };
        assert_eq!(slack_ref.handle(), "1699999999.001200");
        assert_eq!(
            ExternalRef::for_target(&slack_target, &slack_ref.handle()).as_ref(),
            Some(&slack_ref)
        );

        let gh_target = EgressTarget::Github {
            repo: "example/repo".into(),
            issue_number: 3915,
        };
        let gh_ref = ExternalRef::Github {
            repo: "example/repo".into(),
            comment_id: 998877,
        };
        assert_eq!(gh_ref.handle(), "998877");
        assert_eq!(
            ExternalRef::for_target(&gh_target, &gh_ref.handle()).as_ref(),
            Some(&gh_ref),
            "the repo comes from the target, the comment id from the handle"
        );
        assert_eq!(gh_ref.surface(), EgressSurface::Github);
        assert_eq!(slack_ref.surface(), EgressSurface::Slack);
    }

    #[test]
    fn a_malformed_handle_does_not_rebuild_a_ref() {
        let gh_target = EgressTarget::Github {
            repo: "example/repo".into(),
            issue_number: 1,
        };
        for handle in ["", "nan", "0", "-5", "12.5"] {
            assert_eq!(
                ExternalRef::for_target(&gh_target, handle),
                None,
                "expected github handle {handle:?} to be rejected"
            );
        }
        assert_eq!(
            ExternalRef::for_target(
                &EgressTarget::Slack {
                    channel_id: "C1".into()
                },
                ""
            ),
            None
        );
    }

    #[test]
    fn a_target_displays_as_its_surface_qualified_delivery_selector() {
        assert_eq!(
            EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 42,
            }
            .to_string(),
            "github:example/repo#42"
        );
        assert_eq!(
            EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
            }
            .to_string(),
            "slack:C0123ABCDEF"
        );
    }
}
