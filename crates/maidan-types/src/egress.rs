//! Durable projector egress.
//!
//! The Slack (309) and GitHub (312) projectors posted inline and best-effort: a
//! transient 502 dropped the message with a log line and nothing else. A
//! message bound for an external surface is now enqueued in
//! `maidan_egress_outbox` and delivered by a retry/backoff worker — the shape
//! the mail outbox (304) already has.
//!
//! A destination is a typed [`EgressTarget`], persisted as the `(surface,
//! selector)` text pair the allowlist will also key on. The pair always
//! round-trips: the store only ever writes what [`EgressTarget::selector`]
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
    /// Commits to a branch and opens a draft PR. Its own surface, so its own
    /// allowlist rows: a repository blessed for comments is not thereby
    /// writable.
    GithubBranch,
}

impl EgressSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Github => "github",
            Self::GithubBranch => "github_branch",
        }
    }

    /// Decode a persisted discriminator. `None` for anything this build does not
    /// know — a surface added by a newer version, read after a downgrade.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "slack" => Some(Self::Slack),
            "github" => Some(Self::Github),
            "github_branch" => Some(Self::GithubBranch),
            _ => None,
        }
    }
}

impl fmt::Display for EgressSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a row is on the egress outbox.
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
    /// reader is where a producer-supplied channel is checked). `thread_ts`
    /// replies inside that Slack thread; the projector never sets it.
    Slack {
        channel_id: String,
        thread_ts: Option<String>,
    },
    /// A GitHub issue or PR comment. `repo` is `owner/name`; issue and PR numbers
    /// share one namespace, so a PR links exactly like an issue.
    Github { repo: String, issue_number: i64 },
    /// A commit on `branch` of `repo` and a draft PR for it. The PR's base
    /// branch and the commit's parent are read from the result envelope at
    /// send time, so they are not part of the stored destination.
    GithubBranch { repo: String, branch: String },
}

impl EgressTarget {
    pub fn surface(&self) -> EgressSurface {
        match self {
            Self::Slack { .. } => EgressSurface::Slack,
            Self::Github { .. } => EgressSurface::Github,
            Self::GithubBranch { .. } => EgressSurface::GithubBranch,
        }
    }

    /// The persisted per-surface detail: a Slack channel id (with `/<thread_ts>`
    /// for a threaded reply), `owner/name#123`, or `owner/name@branch`.
    pub fn selector(&self) -> String {
        match self {
            Self::Slack {
                channel_id,
                thread_ts: None,
            } => channel_id.clone(),
            Self::Slack {
                channel_id,
                thread_ts: Some(ts),
            } => format!("{channel_id}/{ts}"),
            Self::Github { repo, issue_number } => format!("{repo}#{issue_number}"),
            Self::GithubBranch { repo, branch } => format!("{repo}@{branch}"),
        }
    }

    /// The key this target is *authorized* by in the allowlist, which is
    /// deliberately coarser than [`Self::selector`] on GitHub: an operator
    /// blesses the **repository**, not each issue, because per-issue blessing
    /// would mean an operator ticket per PR. Slack has no such split — a
    /// channel id is already the unit an operator thinks in, and a thread
    /// inside a blessed channel needs no blessing of its own.
    ///
    /// A branch row always names a base (`owner/name@base`, see
    /// [`crate::change_allowlist_selector`]), and the base is not part of this
    /// target, so the bare repository returned here matches no branch row:
    /// authorizing a branch goes through the result's base, and fails closed.
    pub fn allowlist_selector(&self) -> String {
        match self {
            Self::Slack { channel_id, .. } => channel_id.clone(),
            Self::Github { repo, .. } | Self::GithubBranch { repo, .. } => repo.clone(),
        }
    }

    /// Decode a persisted `(surface, selector)` pair. `None` when the selector is
    /// malformed for its surface, which is the caller's cue to dead-letter the row
    /// rather than retry it forever.
    pub fn parse(surface: EgressSurface, selector: &str) -> Option<Self> {
        match surface {
            EgressSurface::Slack => match selector.split_once('/') {
                Some((channel_id, ts)) => {
                    (!channel_id.is_empty() && is_slack_ts(ts)).then(|| Self::Slack {
                        channel_id: channel_id.to_string(),
                        thread_ts: Some(ts.to_string()),
                    })
                }
                None => (!selector.is_empty()).then(|| Self::Slack {
                    channel_id: selector.to_string(),
                    thread_ts: None,
                }),
            },
            EgressSurface::GithubBranch => {
                // A repository name cannot hold `@`; a branch name can.
                let (repo, branch) = selector.split_once('@')?;
                (is_github_repo(repo) && is_branch_name(branch)).then(|| Self::GithubBranch {
                    repo: repo.to_string(),
                    branch: branch.to_string(),
                })
            }
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

/// A handle on an object a sender created on an external surface — a Slack
/// message's `ts`, a GitHub comment's id. It is what makes a re-delivery an
/// **update in place** rather than a second comment.
///
/// Only [`Self::handle`] needs persisting: a delivery row already carries its
/// [`EgressTarget`], and everything else here is derivable from it. So the
/// stored shape is one text column, reconstructed with [`Self::for_target`] —
/// the same "store the narrow thing, decode it back" move as `(surface,
/// selector)`.
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
    /// The commit a change landed as and the pull request that carries it.
    /// Stored as `<commit_sha>#<pull_number>`.
    GithubBranch {
        repo: String,
        commit_sha: String,
        pull_number: i64,
    },
}

impl ExternalRef {
    pub fn surface(&self) -> EgressSurface {
        match self {
            Self::Slack { .. } => EgressSurface::Slack,
            Self::Github { .. } => EgressSurface::Github,
            Self::GithubBranch { .. } => EgressSurface::GithubBranch,
        }
    }

    /// The part a delivery row has to remember: the Slack `ts`, the GitHub
    /// comment id as text, or a change's `<commit_sha>#<pull_number>`.
    pub fn handle(&self) -> String {
        match self {
            Self::Slack { ts, .. } => ts.clone(),
            Self::Github { comment_id, .. } => comment_id.to_string(),
            Self::GithubBranch {
                commit_sha,
                pull_number,
                ..
            } => format!("{commit_sha}#{pull_number}"),
        }
    }

    /// Rebuild a ref from the delivery's target and the stored handle. `None` when
    /// the handle is malformed for its surface — the caller's cue to treat the ref
    /// as lost and fall back to posting (with the hidden-marker recovery path on
    /// GitHub) rather than issuing an update against a guess.
    pub fn for_target(target: &EgressTarget, handle: &str) -> Option<Self> {
        match target {
            EgressTarget::Slack { channel_id, .. } => (!handle.is_empty()).then(|| Self::Slack {
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
            EgressTarget::GithubBranch { repo, .. } => {
                let (commit_sha, number) = handle.split_once('#')?;
                let pull_number: i64 = number.parse().ok()?;
                (is_git_sha(commit_sha) && pull_number > 0).then(|| Self::GithubBranch {
                    repo: repo.clone(),
                    commit_sha: commit_sha.to_string(),
                    pull_number,
                })
            }
        }
    }
}

/// A full git object id: SHA-1 (40 hex) or SHA-256 (64 hex). A short prefix is
/// refused, because a commit named by a prefix can become ambiguous.
pub fn is_git_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A Slack message timestamp, `1699999999.001200`: the id of the message a
/// threaded reply hangs under.
pub fn is_slack_ts(s: &str) -> bool {
    let Some((secs, frac)) = s.split_once('.') else {
        return false;
    };
    !secs.is_empty()
        && !frac.is_empty()
        && secs.bytes().all(|b| b.is_ascii_digit())
        && frac.bytes().all(|b| b.is_ascii_digit())
}

/// `owner/name` in GitHub's own alphabet (letters, digits, `-`, `_`, `.`), so
/// the pair can be put in an API path as it is.
pub fn is_github_repo(s: &str) -> bool {
    let Some((owner, name)) = s.split_once('/') else {
        return false;
    };
    let part = |p: &str| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    part(owner) && part(name)
}

/// A branch name git would accept (`git check-ref-format --branch`), less
/// anything that could reach a URL path as something other than a name.
pub fn is_branch_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with(['/', '-', '.'])
        && !s.ends_with(['/', '.'])
        && !s.ends_with(".lock")
        && !s.contains("..")
        && !s.contains("//")
        && !s.contains("@{")
        && !s.contains("/.")
        && s != "@"
        && !s
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\#%".contains(c))
}

/// A destination a workspace's operator has blessed for egress.
///
/// `surface` is stored as text rather than an [`EgressSurface`], for the same
/// reason [`EgressOutbox`] does: a row written by a newer build and read after
/// a downgrade must still be *listable*, or the operator cannot see the entry
/// they need to revoke. The write side is typed ([`NewEgressTarget`]), so a
/// surface this build cannot deliver to can never be blessed in the first
/// place.
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
        EgressSurface::GithubBranch => {
            let Some((repo, base)) = selector.split_once('@') else {
                return Err("github_branch selector is `owner/name@base`: the repository and the one base change pull requests may target");
            };
            if !is_github_repo(repo) {
                return Err("github_branch selector must start with a repository `owner/name`");
            }
            if !is_branch_name(base) {
                return Err("github_branch selector must end with a base branch name");
            }
            if base.eq_ignore_ascii_case(crate::FORBIDDEN_CHANGE_BASE) {
                return Err("`prod` is never an allowed base for change pull requests");
            }
            if crate::is_change_branch(base) {
                return Err("an agent branch cannot be a base");
            }
            Ok(())
        }
        EgressSurface::Github => {
            if selector.contains('#') {
                return Err(
                    "github selector is a repository `owner/name`, without an issue number",
                );
            }
            if selector.contains('@') {
                return Err("github selector is a repository `owner/name`, without a branch");
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

/// A new delivery to enqueue. Queued `pending`, due now.
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
    /// [`EgressKind::Projector`] for linked-thread relays;
    /// [`EgressKind::Result`] for result delivery. The worker uses
    /// this to decide whether a stored [`ExternalRef`] is an object it may
    /// edit.
    pub kind: EgressKind,
}

/// A claimed delivery the egress worker will attempt. `attempts` includes the
/// current claim; the queue's status / scheduling columns stay internal to the
/// store.
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
    /// The server span the source event was written under.
    pub trace: Option<crate::TraceContext>,
}

impl EgressOutbox {
    /// The typed destination, or `None` when the stored pair does not decode.
    pub fn target(&self) -> Option<EgressTarget> {
        EgressTarget::parse(EgressSurface::parse(&self.surface)?, &self.selector)
    }
}

/// A dead-lettered delivery for the operator DLQ view: a message that exhausted
/// its retries, or whose link was disabled as misconfigured. `last_error` is
/// why the final attempt failed — the surface's own words.
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
                thread_ts: None,
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
            trace: None,
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
            thread_ts: None,
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
            thread_ts: None,
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
                    channel_id: "C1".into(),
                    thread_ts: None
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
                thread_ts: None
            }
            .to_string(),
            "slack:C0123ABCDEF"
        );
    }

    #[test]
    fn branch_and_thread_targets_round_trip_through_their_stored_pair() {
        for target in [
            EgressTarget::GithubBranch {
                repo: "beatgig/bgv3".into(),
                branch: "feature/agent-x@y-1a2b".into(),
            },
            EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
                thread_ts: Some("1699999999.001200".into()),
            },
        ] {
            assert_eq!(round_trip(&target).as_ref(), Some(&target));
        }
        assert_eq!(
            EgressTarget::parse(EgressSurface::Slack, "C0123ABCDEF/not-a-ts"),
            None
        );
        assert_eq!(
            EgressTarget::parse(EgressSurface::GithubBranch, "beatgig/bgv3"),
            None
        );
    }

    #[test]
    fn a_change_ref_round_trips_and_needs_a_full_sha() {
        let target = EgressTarget::GithubBranch {
            repo: "beatgig/bgv3".into(),
            branch: "feature/x".into(),
        };
        let sha = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";
        let reference = ExternalRef::GithubBranch {
            repo: "beatgig/bgv3".into(),
            commit_sha: sha.into(),
            pull_number: 12,
        };
        assert_eq!(reference.handle(), format!("{sha}#12"));
        assert_eq!(
            ExternalRef::for_target(&target, &reference.handle()),
            Some(reference)
        );
        assert_eq!(ExternalRef::for_target(&target, "b5e54f9#12"), None);
        assert_eq!(ExternalRef::for_target(&target, &format!("{sha}#0")), None);
    }

    #[test]
    fn a_branch_blessing_names_a_repository_and_its_base() {
        for selector in ["beatgig/bgv3@dev", "beatgig/agent-skills@main"] {
            assert!(validate_allowlist_selector(EgressSurface::GithubBranch, selector).is_ok());
        }
        for selector in [
            "beatgig/bgv3",
            "beatgig/bgv3@prod",
            "beatgig/bgv3@PROD",
            "beatgig/bgv3@",
            "beatgig/bgv3@feature/agent-x",
            "beatgig/bgv3#1@dev",
            "bgv3@dev",
            "",
        ] {
            assert!(
                validate_allowlist_selector(EgressSurface::GithubBranch, selector).is_err(),
                "expected {selector:?} to be rejected"
            );
        }
        assert_eq!(
            EgressSurface::parse("github_branch"),
            Some(EgressSurface::GithubBranch)
        );
        assert!(
            validate_allowlist_selector(EgressSurface::Github, "beatgig/bgv3@dev").is_err(),
            "a comment row never names a base"
        );
    }
}
