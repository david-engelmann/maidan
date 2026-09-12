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
                repo: "beatgig/bgv3".into(),
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
            "beatgig/bgv3",     // no issue number
            "beatgig/bgv3#",    // empty issue number
            "beatgig/bgv3#nan", // non-numeric
            "beatgig/bgv3#0",   // issue numbers start at 1
            "beatgig/bgv3#-1",
            "bgv3#12", // no owner
            "/bgv3#12",
            "beatgig/#12",
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
        };
        assert_eq!(row.target(), None);
    }

    #[test]
    fn a_github_target_is_authorized_by_its_repository_not_its_issue() {
        let target = EgressTarget::Github {
            repo: "beatgig/bgv3".into(),
            issue_number: 3915,
        };
        assert_eq!(target.selector(), "beatgig/bgv3#3915");
        assert_eq!(target.allowlist_selector(), "beatgig/bgv3");
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
            (EgressSurface::Github, "beatgig/bgv3#3915"),
            (EgressSurface::Github, "bgv3"),
            (EgressSurface::Github, "/bgv3"),
            (EgressSurface::Github, "beatgig/"),
            (EgressSurface::Github, "beatgig/bgv3/extra"),
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
            (EgressSurface::Github, "beatgig/bgv3"),
        ] {
            assert!(
                validate_allowlist_selector(surface, selector).is_ok(),
                "expected {surface}:{selector:?} to be accepted"
            );
        }
    }

    #[test]
    fn a_target_displays_as_its_surface_qualified_delivery_selector() {
        assert_eq!(
            EgressTarget::Github {
                repo: "beatgig/bgv3".into(),
                issue_number: 3915,
            }
            .to_string(),
            "github:beatgig/bgv3#3915"
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
