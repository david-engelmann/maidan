//! The waiter-result contract — a **tolerant reader** for the envelope an
//! external result producer writes with `set_thread_result`.
//!
//! The result is opaque JSON owned by the producer. Maidan parses only the
//! fields it *routes on* and ignores everything else, so a producer adding a
//! field never breaks delivery. The grammar is pinned in `docs/Result
//! Delivery.md` and frozen at [`WAITER_RESULT_SCHEMA`]; the authoritative
//! fixture is committed at `tests/fixtures/waiter_result_v1.json`, so a
//! producer-side grammar change breaks a test in this crate rather than a
//! delivery in production.
//!
//! **Tolerance has a floor.** `schema` (or `$type`, the same NSID)
//! is the discriminator: an unrecognized one means no delivery is attempted at
//! all, because routing on an envelope we do not understand is how you deliver
//! the wrong bytes to the wrong place. Past that, missing optional fields
//! degrade rather than fail.
//!
//! `head_sha` and `findings[].{file,line_range,body}` are read so inline review
//! comments can be placed. The summary-comment path is unchanged: an envelope
//! without those fields still delivers. The GitHub review is posted from them.
//!
//! `findings[].severity` is also read (a **namespaced/free
//! string**, not a closed enum) so a reviewed [`EXAMPLE_REVIEW_RESULT_KIND`]
//! envelope with any `critical` finding maps onto
//! [`ReviewDecision::RequestChanges`] — the decision the close-gate already
//! understands. Severity is walked on the raw `findings` array: a critical
//! finding that is unusable as an inline comment still arms the adapter.

use serde::Serialize;
use serde_json::Value;

use crate::egress::{is_branch_name, is_git_sha, is_github_repo, is_slack_ts, EgressTarget};
use crate::review::ReviewDecision;

/// The frozen envelope discriminator. A different value is inert, not an error.
pub const WAITER_RESULT_SCHEMA: &str = "maidan.waiter.result/1";

/// The namespaced producer shape for a code-review waiter result.
///
/// A **string, not an enum** — the same rule as the result facet. Compare
/// this constant; do not close the `result_kind` set.
pub const EXAMPLE_REVIEW_RESULT_KIND: &str = "example.review.result/1";

/// Namespaced producer shape for a plan waiter result (generic example).
/// Same rule as [`EXAMPLE_REVIEW_RESULT_KIND`]: a string, not an enum.
pub const EXAMPLE_PLAN_RESULT_KIND: &str = "example.plan.result/1";

/// The finding severity that maps to [`ReviewDecision::RequestChanges`]. A free string on the wire; only this
/// exact value arms the adapter (`warning` / `info` / future words do not).
pub const FINDING_SEVERITY_CRITICAL: &str = "critical";

/// Note stored on the review row when the adapter writes `request_changes`.
/// Human-readable so `list_reviews` shows *why* the land is blocked; the
/// close-gate itself only reads `decision`.
pub const CRITICAL_REVIEW_NOTE: &str = "critical finding in example.review.result/1";

/// The only `status` that delivers the producer's own bytes. Any other value
/// gets a short Maidan-authored failure notice built from `status` alone —
/// never silence, and never rendered as a clean pass.
pub const STATUS_REVIEWED: &str = "reviewed";

/// GitHub `event` for a Maidan-authored pull-request review. Maidan delivers
/// findings; it does not approve or request-changes on the producer's behalf.
/// The result summary is a separate issue comment, not this review's body.
pub const GITHUB_REVIEW_EVENT_COMMENT: &str = "COMMENT";

/// Check name on `POST /repos/{repo}/check-runs`. One name, so a re-delivery
/// is another run of the same check rather than a second product.
pub const GITHUB_CHECK_NAME: &str = "maidan";

/// One entry of the producer's `deliver_to` routing list.
///
/// [`Self::Unknown`] is the load-bearing variant: rule 2 of the pinned grammar
/// says an unknown `surface` is **skipped with a recorded warning, never an
/// error**, so a newer producer naming a surface this build cannot reach stays
/// backward-compatible. It carries the surface string so the skip can say which
/// one it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "surface", rename_all = "snake_case")]
pub enum DeliverTarget {
    /// `repo` is `owner/name`; `pr` is an issue **or** PR number — GitHub shares
    /// one namespace, so a PR comments exactly like an issue.
    Github {
        repo: String,
        pr: i64,
    },
    /// A channel **id** (`C…`/`G…`), never a `#name`: a name is mutable, and the
    /// channel it points at can change under a delivery. `thread_ts` replies in
    /// that Slack thread; the allowlist still blesses the channel.
    Slack {
        channel: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_ts: Option<String>,
    },
    /// Commit a [`PI_CHANGE_RESULT_KIND`] diff to `branch` of `repo` and open a
    /// draft PR against `base` when none is open for the branch.
    GithubBranch {
        repo: String,
        branch: String,
        base: String,
    },
    Unknown(String),
}

impl DeliverTarget {
    /// The surface name as the producer wrote it.
    pub fn surface(&self) -> &str {
        match self {
            Self::Github { .. } => "github",
            Self::Slack { .. } => "slack",
            Self::GithubBranch { .. } => "github_branch",
            Self::Unknown(surface) => surface,
        }
    }

    /// Project a producer-selected target onto the delivery target the egress
    /// queue speaks.
    ///
    /// `None` for a surface this build does not know, and for a target whose
    /// per-surface detail is missing or unusable — both of which are a *skip*
    /// with a recorded reason, not a failure. This is where the producer's
    /// vocabulary stops and Maidan's transport begins.
    pub fn to_egress_target(&self) -> Option<EgressTarget> {
        match self {
            Self::Github { repo, pr } => {
                let (owner, name) = repo.split_once('/')?;
                (!owner.is_empty() && !name.is_empty() && !repo.contains('#') && *pr > 0).then(
                    || EgressTarget::Github {
                        repo: repo.clone(),
                        issue_number: *pr,
                    },
                )
            }
            Self::Slack { channel, thread_ts } => {
                // A thread id that is not a Slack ts is unusable rather than
                // dropped: posting top-level would answer in the wrong place.
                let thread_ok = thread_ts.as_deref().is_none_or(is_slack_ts);
                (!channel.is_empty()
                    && !channel.starts_with('#')
                    && !channel.contains('/')
                    && thread_ok)
                    .then(|| EgressTarget::Slack {
                        channel_id: channel.clone(),
                        thread_ts: thread_ts.clone(),
                    })
            }
            Self::GithubBranch { repo, branch, base } => (is_github_repo(repo)
                && is_branch_name(branch)
                && is_branch_name(base))
            .then(|| EgressTarget::GithubBranch {
                repo: repo.clone(),
                branch: branch.clone(),
            }),
            Self::Unknown(_) => None,
        }
    }

    /// The selector the workspace allowlist must hold for this target, or
    /// `None` when the target does not project. A branch target is blessed
    /// with its base (`owner/name@base`), so a repository blessed for one base
    /// cannot take a pull request into another.
    pub fn allowlist_selector(&self) -> Option<String> {
        match self {
            Self::GithubBranch { repo, base, .. } => {
                self.to_egress_target()?;
                Some(change_allowlist_selector(repo, base))
            }
            other => other.to_egress_target().map(|t| t.allowlist_selector()),
        }
    }

    /// The `(surface, selector)` pair a skip is recorded under when this target
    /// cannot be projected onto an [`EgressTarget`].
    ///
    /// Known surfaces keep the producer-supplied detail so the skip is
    /// addressable (`github` / `owner/name#123`, `slack` / `#general`). An
    /// unknown surface records under its name with an empty selector — there
    /// is no delivery grain to store, but the row still has to exist, because
    /// "we skipped your target" and "we lost it" are different answers.
    pub fn skip_fingerprint(&self) -> (String, String) {
        match self {
            Self::Github { repo, pr } => ("github".into(), format!("{repo}#{pr}")),
            Self::Slack {
                channel,
                thread_ts: None,
            } => ("slack".into(), channel.clone()),
            Self::Slack {
                channel,
                thread_ts: Some(ts),
            } => ("slack".into(), format!("{channel}/{ts}")),
            Self::GithubBranch { repo, branch, .. } => {
                ("github_branch".into(), format!("{repo}@{branch}"))
            }
            Self::Unknown(surface) => (surface.clone(), String::new()),
        }
    }
}

/// Inclusive 1-indexed range on the **post-image** file at envelope
/// `head_sha` — the file as that commit left it, not a diff-hunk offset.
///
/// On GitHub this is the **RIGHT** side of the pull-request split view
/// ([`GithubDiffSide::Right`]). `LEFT` is deletions that no longer exist in
/// the after-state; a finding that quotes a line in the resulting file is
/// never LEFT. Resolving the live PR head instead of `head_sha` can place
/// every comment on a newer commit than was reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FindingLineRange {
    pub start: u32,
    pub end: u32,
}

impl FindingLineRange {
    /// `None` when the range is not a 1-indexed inclusive span (`start < 1`
    /// or `end < start`).
    pub fn new(start: u32, end: u32) -> Option<Self> {
        (start >= 1 && end >= start).then_some(Self { start, end })
    }

    /// GitHub `line` — the last line of the inclusive range.
    pub fn github_line(self) -> u32 {
        self.end
    }

    /// GitHub `start_line` when the range spans more than one line. `None`
    /// for a single-line finding so the review POST omits it.
    pub fn github_start_line(self) -> Option<u32> {
        (self.start != self.end).then_some(self.start)
    }
}

/// GitHub pull-review `side`. Findings are always [`Self::Right`]: they
/// describe the file after `head_sha`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GithubDiffSide {
    #[serde(rename = "RIGHT")]
    Right,
}

impl GithubDiffSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Right => "RIGHT",
        }
    }
}

/// One producer finding Maidan can turn into an inline review comment.
/// `severity` is the land-gate projection (a free string, not an
/// enum). Other finding fields (`quoted_line`, `category`, …) stay in the
/// stored JSON; the comment body is the producer's `body` as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaiterFinding {
    pub file: String,
    pub line_range: FindingLineRange,
    pub body: String,
    /// Producer severity (`critical`, `warning`, …). Absent when the
    /// finding omitted it or sent `""`. Not a closed enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

impl WaiterFinding {
    /// Project onto GitHub's `comments[]` item (`path`, `line`, `side`,
    /// `body`, plus `start_line` when the range is multi-line).
    pub fn to_github_review_comment(&self) -> GithubReviewComment {
        GithubReviewComment {
            path: self.file.clone(),
            line: self.line_range.github_line(),
            start_line: self.line_range.github_start_line(),
            side: GithubDiffSide::Right,
            body: self.body.clone(),
        }
    }
}

/// Coordinates for `POST /repos/{repo}/pulls/{n}/reviews` `comments[]`. The
/// egress worker posts these; the mapping is pinned by a fixture test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GithubReviewComment {
    pub path: String,
    pub line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    pub side: GithubDiffSide,
    pub body: String,
}

/// The routable projection of a producer's result envelope. Everything the
/// producer carries that Maidan does not route on — `corroboration`,
/// `per_seat`, `cost_usd`, … — stays in the stored result and is deliberately
/// absent here. `head_sha` and `findings` are present because inline comments
/// have to be placed, not just forwarded. `run_id` is not a delivery field
/// either; it is lineage, read via [`crate::run_id_from_payload`], not this
/// struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaiterResult {
    /// Which producer shape this is, e.g. `example.review.result/1`. A **namespaced
    /// string, not an enum** — a closed enum would need editing every time a
    /// waiter product ships a new result kind.
    pub result_kind: String,
    pub status: String,
    /// Absent or empty is **valid and normal**: thread-only, delivered nowhere.
    pub deliver_to: Vec<DeliverTarget>,
    /// The GitHub delivery body — producer-authored trusted markdown.
    pub rendered: Option<String>,
    /// One line. The Slack body and the notification title.
    pub summary: Option<String>,
    /// A backlink appended to every delivery.
    pub view_url: Option<String>,
    /// A human back-reference echoed into the delivered body. **A string**
    /// (`owner/name#123`) — not to be confused with `deliver_to[].pr`, which is
    /// an integer. Two different fields with the same name and different types.
    pub pr: Option<String>,
    /// The commit the review was computed against. This is GitHub's `commit_id`
    /// for inline comments. **There is no helper that resolves a PR's current
    /// head** — that function must not exist; a newer head than was reviewed
    /// would misplace every comment. Absent ⇒ inline comments are skipped; the
    /// summary comment still posts.
    pub head_sha: Option<String>,
    /// Findings that have a file, a body, and a valid post-image
    /// [`FindingLineRange`]. Malformed entries are skipped, not an error.
    pub findings: Vec<WaiterFinding>,
}

impl WaiterResult {
    /// Whether this result delivers the producer's own bytes. A non-`reviewed`
    /// status is surfaced as a failure notice instead — never silence, never a
    /// clean pass.
    pub fn is_reviewed(&self) -> bool {
        self.status == STATUS_REVIEWED
    }

    /// GitHub `commit_id` for an inline review. **Only** the envelope's
    /// `head_sha` — never the live PR head.
    pub fn review_commit_id(&self) -> Option<&str> {
        self.head_sha.as_deref()
    }

    /// Inline comments the worker POSTs. Empty when every finding was
    /// unusable; that is a skip, not a failure of the summary path.
    pub fn github_review_comments(&self) -> Vec<GithubReviewComment> {
        self.findings
            .iter()
            .map(WaiterFinding::to_github_review_comment)
            .collect()
    }
}

/// `conclusion` on a completed Maidan check run.
///
/// `success` means the result was `reviewed` and no finding was `critical`.
/// It does **not** approve the pull request. `failure` means the result was
/// not accepted, or a critical finding is present (including one too incomplete
/// to post as an inline comment). It does **not** request changes: that remains
/// a review decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubCheckConclusion {
    Success,
    Failure,
}

impl GithubCheckConclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

/// A completed check run addressed at the envelope `head_sha`.
///
/// There is no queued or in-progress variant here: those would fire on claim,
/// and this value is built only from a result. There is no field for a live
/// PR head, and there must not be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubCheckRun {
    pub name: String,
    pub head_sha: String,
    pub conclusion: GithubCheckConclusion,
    pub title: String,
    pub summary: String,
    pub details_url: Option<String>,
}

/// The check run for this result, or `None` when it has no envelope `head_sha`.
///
/// `raw` is the stored envelope, not the parsed findings list: a `critical`
/// finding with no file or line still fails the check, the same way it arms
/// the land gate. A `success` conclusion does not approve the pull request.
pub fn github_check_run(waiter: &WaiterResult, raw: &Value) -> Option<GithubCheckRun> {
    let head_sha = waiter.review_commit_id()?.to_string();
    let critical = findings_contain_critical(raw);
    let failure = !waiter.is_reviewed() || critical;
    let conclusion = if failure {
        GithubCheckConclusion::Failure
    } else {
        GithubCheckConclusion::Success
    };
    let title = if !waiter.is_reviewed() {
        "Maidan result did not pass"
    } else if critical {
        "Maidan found a critical issue"
    } else {
        "Maidan result"
    };
    Some(GithubCheckRun {
        name: GITHUB_CHECK_NAME.to_string(),
        head_sha,
        conclusion,
        title: title.to_string(),
        summary: check_run_summary(waiter, !waiter.is_reviewed(), critical),
        details_url: waiter.view_url.as_deref().and_then(https_details_url),
    })
}

fn check_run_summary(waiter: &WaiterResult, not_reviewed: bool, critical: bool) -> String {
    let lead = if not_reviewed {
        format!(
            "Maidan did not accept this result (status `{}`). This check does not request changes on the pull request.",
            waiter.status
        )
    } else if critical {
        "Maidan found a critical issue. This check does not request changes on the pull request; the thread records that."
            .to_string()
    } else {
        "Maidan accepted this result. This check does not approve the pull request.".to_string()
    };
    match waiter
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
    {
        Some(summary) => clip_chars(&format!("{lead}\n\n{summary}"), 4_000),
        None => lead,
    }
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn https_details_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.starts_with("https://") && !url.chars().any(char::is_whitespace) && url.len() <= 2_048 {
        Some(url.to_string())
    } else {
        None
    }
}

/// Map a waiter envelope onto the decision the close-gate already understands.
///
/// `Some(RequestChanges)` when:
/// - the envelope parses (`schema = maidan.waiter.result/1`),
/// - `result_kind` is exactly [`EXAMPLE_REVIEW_RESULT_KIND`] (string compare,
///   not an enum),
/// - `status` is `reviewed`,
/// - any raw `findings[]` entry has `severity = "critical"` — including
///   entries that are unusable as inline comments (no file / body /
///   `line_range`).
///
/// `None` otherwise. This function never returns [`ReviewDecision::Approve`]: a
/// clean re-review does not auto-land. A human resolves by submitting an
/// approve through the existing review surface.
pub fn review_decision_from_waiter(value: &Value) -> Option<ReviewDecision> {
    let parsed = parse_waiter_result(value)?;
    if parsed.result_kind != EXAMPLE_REVIEW_RESULT_KIND || !parsed.is_reviewed() {
        return None;
    }
    findings_contain_critical(value).then_some(ReviewDecision::RequestChanges)
}

/// Walk the raw `findings` array. Usability-for-inline is the filter; a
/// critical finding still counts here without `file`/`body`.
fn findings_contain_critical(value: &Value) -> bool {
    value
        .get("findings")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry.get("severity").and_then(Value::as_str) == Some(FINDING_SEVERITY_CRITICAL)
            })
        })
}

/// The producer shape for a coding result that becomes a commit: Pi's
/// `pi.change.result/1`. Same rule as [`EXAMPLE_REVIEW_RESULT_KIND`]: compare
/// the string, do not close the set.
pub const PI_CHANGE_RESULT_KIND: &str = "pi.change.result/1";

/// The only change status that writes to GitHub. `no_change`, `seat_error`
/// and `content_blocked` are recorded and answered in Slack, never committed.
pub const CHANGE_STATUS_CHANGED: &str = "changed";

/// Every branch the change flow may write starts with this.
pub const CHANGE_BRANCH_PREFIX: &str = "feature/agent-";

/// Branches the change flow never writes and never targets as a base,
/// whatever the allowlist says. Hard-coded on purpose: the token the flow
/// runs with can push to all of them, so the guard has to be in this code,
/// not in configuration or in GitHub's settings.
pub const PROTECTED_BRANCHES: [&str; 5] = ["prod", "main", "master", "staging", "dev"];

/// The base no change is ever opened against, whatever the allowlist says.
pub const FORBIDDEN_CHANGE_BASE: &str = "prod";

/// Whether `branch` is `feature/agent-[a-z0-9][a-z0-9-]*`.
pub fn is_change_branch(branch: &str) -> bool {
    let Some(slug) = branch.strip_prefix(CHANGE_BRANCH_PREFIX) else {
        return false;
    };
    let mut chars = slug.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The hard-coded rules for a `github_branch` target, checked before the
/// allowlist and again before any GitHub write. `Err` is the reason recorded
/// on the delivery.
pub fn check_change_target(branch: &str, base: &str) -> Result<(), String> {
    let protected = |name: &str| {
        PROTECTED_BRANCHES
            .iter()
            .any(|p| p.eq_ignore_ascii_case(name))
    };
    if protected(branch) {
        return Err(format!(
            "branch `{branch}` is protected; the change flow writes only `{CHANGE_BRANCH_PREFIX}…` branches"
        ));
    }
    if !is_change_branch(branch) {
        return Err(format!(
            "branch `{branch}` does not match `{CHANGE_BRANCH_PREFIX}[a-z0-9][a-z0-9-]*`"
        ));
    }
    if base.eq_ignore_ascii_case(FORBIDDEN_CHANGE_BASE) {
        return Err(format!(
            "base `{base}` is never allowed for a change pull request"
        ));
    }
    if !is_branch_name(base) {
        return Err(format!("base `{base}` is not a branch name"));
    }
    if branch == base {
        return Err(format!("branch `{branch}` is its own base"));
    }
    Ok(())
}

/// The allowlist selector that blesses change pull requests into `base` of
/// `repo`: `owner/name@base`. One row per repository and base.
pub fn change_allowlist_selector(repo: &str, base: &str) -> String {
    format!("{repo}@{base}")
}

/// The change flow's per-repository base map for the mark-ready flip
/// (maidan#1253): these repositories open agent pull requests only into the
/// mapped base. Hard-coded on purpose, like [`PROTECTED_BRANCHES`]: the token
/// the flow runs with can push anywhere, so the guard has to be in this code,
/// not in configuration or in GitHub's settings.
const MARK_READY_BASES: [(&str, &str); 5] = [
    ("bgv3", "dev"),
    ("relay", "dev"),
    ("dawn", "dev"),
    ("wax", "dev"),
    ("agent-skills", "main"),
];

/// The mark-ready half of the base guard: the shape rules from
/// [`check_change_target`], plus the per-repository base map. A repository in
/// [`MARK_READY_BASES`] flips only into its mapped base; a repository outside
/// the map is governed by the workspace egress allowlist, which fails closed.
pub fn check_mark_ready_target(repo: &str, branch: &str, base: &str) -> Result<(), String> {
    check_change_target(branch, base)?;
    let name = repo.rsplit('/').next().unwrap_or(repo);
    match MARK_READY_BASES
        .iter()
        .find(|(r, _)| r.eq_ignore_ascii_case(name))
    {
        Some((_, want)) if !base.eq_ignore_ascii_case(want) => Err(format!(
            "`{repo}` opens agent pull requests only into `{want}`; `{base}` is not its base"
        )),
        _ => Ok(()),
    }
}

/// The fields of a [`PI_CHANGE_RESULT_KIND`] envelope that a commit is built
/// from. Read straight off the envelope, like `head_sha`: the base commit is
/// the producer's, never one looked up from the thread or the branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeResult {
    pub status: String,
    /// The commit the diff applies to, captured before any edit. `None` when
    /// absent or not a full SHA: a short prefix could name another commit.
    pub base_sha: Option<String>,
    /// The branch the producer worked on, echoed verbatim; it must equal the
    /// `github_branch` target's `branch`.
    pub branch: Option<String>,
    /// A git diff that applies with `git apply`.
    pub diff: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
}

impl ChangeResult {
    pub fn is_changed(&self) -> bool {
        self.status == CHANGE_STATUS_CHANGED
    }
}

/// Read a change result. `None` unless this is a waiter envelope whose
/// `result_kind` is [`PI_CHANGE_RESULT_KIND`]; past that, every field is
/// optional here and the delivery decides what it needs.
pub fn parse_change_result(value: &Value) -> Option<ChangeResult> {
    let waiter = parse_waiter_result(value)?;
    if waiter.result_kind != PI_CHANGE_RESULT_KIND {
        return None;
    }
    let obj = value.as_object()?;
    let text = |key: &str| {
        obj.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    Some(ChangeResult {
        status: waiter.status,
        base_sha: text("base_sha").filter(|s| is_git_sha(s)),
        branch: text("branch"),
        diff: text("diff"),
        title: text("title"),
        summary: text("summary"),
    })
}

/// Whether this object is a `maidan.waiter.result/1` envelope.
///
/// `$type` is an alias of `schema` (same NSID). Either field matching
/// [`WAITER_RESULT_SCHEMA`] is enough; if both are present they must agree. A
/// `/2` (or any other value) is inert — breaking changes are a new type, not a
/// silent accept. The committed producer fixture still uses `schema` only; this
/// does not rewrite it.
fn waiter_schema_matches(obj: &serde_json::Map<String, Value>) -> bool {
    let schema = obj.get("schema").and_then(Value::as_str);
    let type_id = obj.get("$type").and_then(Value::as_str);
    match (schema, type_id) {
        (Some(s), Some(t)) => s == WAITER_RESULT_SCHEMA && t == WAITER_RESULT_SCHEMA,
        (Some(s), None) => s == WAITER_RESULT_SCHEMA,
        (None, Some(t)) => t == WAITER_RESULT_SCHEMA,
        (None, None) => false,
    }
}

/// Read the routable fields out of a producer's result.
///
/// `None` when this is not an envelope we recognize — a missing or unexpected
/// `schema`/`$type`, or a non-object — which means no delivery is attempted rather
/// than a delivery attempted on a guess. Anything past the discriminator degrades
/// instead of failing: an absent `deliver_to` is an empty list, and the optional
/// bodies are `None`. Unusable `findings` / `head_sha` are skipped the same way
/// so a 379 summary delivery still happens.
pub fn parse_waiter_result(value: &Value) -> Option<WaiterResult> {
    let obj = value.as_object()?;
    if !waiter_schema_matches(obj) {
        return None;
    }
    // `result_kind` and `status` are the two fields every routing decision reads,
    // so an envelope without them is not routable even though the schema matched.
    let result_kind = obj.get("result_kind").and_then(Value::as_str)?.to_string();
    let status = obj.get("status").and_then(Value::as_str)?.to_string();

    let deliver_to = obj
        .get("deliver_to")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(parse_deliver_target).collect())
        .unwrap_or_default();

    let string_field = |key: &str| {
        obj.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let findings = obj
        .get("findings")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(parse_finding).collect())
        .unwrap_or_default();

    Some(WaiterResult {
        result_kind,
        status,
        deliver_to,
        rendered: string_field("rendered"),
        summary: string_field("summary"),
        view_url: string_field("view_url"),
        pr: string_field("pr"),
        head_sha: obj.get("head_sha").and_then(parse_head_sha),
        findings,
    })
}

/// Git SHA-1 (40 hex) or SHA-256 (64 hex). Anything else — empty, a short
/// prefix, a live-PR URL — is absent, so 380.2 will not guess a commit.
fn parse_head_sha(value: &Value) -> Option<String> {
    let s = value.as_str()?.trim();
    let ok_len = s.len() == 40 || s.len() == 64;
    (ok_len && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_string())
}

fn parse_finding(entry: &Value) -> Option<WaiterFinding> {
    let obj = entry.as_object()?;
    let file = obj
        .get("file")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let body = obj
        .get("body")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let line_range = obj.get("line_range").and_then(parse_line_range)?;
    let severity = obj
        .get("severity")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(WaiterFinding {
        file: file.to_string(),
        line_range,
        body: body.to_string(),
        severity,
    })
}

fn parse_line_range(value: &Value) -> Option<FindingLineRange> {
    let obj = value.as_object()?;
    let start = parse_line_number(obj.get("start")?)?;
    let end = parse_line_number(obj.get("end")?)?;
    FindingLineRange::new(start, end)
}

fn parse_line_number(value: &Value) -> Option<u32> {
    let n = value.as_u64()?;
    u32::try_from(n).ok().filter(|n| *n >= 1)
}

/// One `deliver_to` entry. `None` only when the entry has no `surface` string at
/// all — a known surface missing its detail still yields [`DeliverTarget::Unknown`]
/// so the disposition is *recorded* rather than the entry vanishing, which is the
/// difference between "we skipped your target" and "we lost it".
fn parse_deliver_target(entry: &Value) -> Option<DeliverTarget> {
    let obj = entry.as_object()?;
    let surface = obj.get("surface").and_then(Value::as_str)?;
    let unknown = || Some(DeliverTarget::Unknown(surface.to_string()));
    match surface {
        "github" => {
            let Some(repo) = obj.get("repo").and_then(Value::as_str) else {
                return unknown();
            };
            let Some(pr) = obj.get("pr").and_then(Value::as_i64) else {
                return unknown();
            };
            Some(DeliverTarget::Github {
                repo: repo.to_string(),
                pr,
            })
        }
        "slack" => match obj.get("channel").and_then(Value::as_str) {
            Some(channel) => Some(DeliverTarget::Slack {
                channel: channel.to_string(),
                thread_ts: obj
                    .get("thread_ts")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }),
            None => unknown(),
        },
        "github_branch" => {
            let field = |key: &str| obj.get(key).and_then(Value::as_str).map(str::to_string);
            match (field("repo"), field("branch"), field("base")) {
                (Some(repo), Some(branch), Some(base)) => {
                    Some(DeliverTarget::GithubBranch { repo, branch, base })
                }
                _ => unknown(),
            }
        }
        other => Some(DeliverTarget::Unknown(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unrecognized_envelope_is_inert() {
        // No delivery is attempted on a schema we do not understand, rather than
        // routing on a guess.
        for value in [
            json!({}),
            json!({ "status": "reviewed", "result_kind": "x/1" }),
            json!({ "schema": "maidan.waiter.result/2", "result_kind": "x/1", "status": "reviewed" }),
            json!({ "$type": "maidan.waiter.result/2", "result_kind": "x/1", "status": "reviewed" }),
            json!({
                "schema": WAITER_RESULT_SCHEMA,
                "$type": "maidan.waiter.result/2",
                "result_kind": "x/1",
                "status": "reviewed"
            }),
            json!({ "schema": 1, "result_kind": "x/1", "status": "reviewed" }),
            json!("a string, not an envelope"),
            json!([]),
        ] {
            assert_eq!(parse_waiter_result(&value), None, "for {value}");
        }
        // Schema matches, but the two fields every routing decision reads are missing.
        assert_eq!(
            parse_waiter_result(&json!({ "schema": WAITER_RESULT_SCHEMA })),
            None
        );
        assert_eq!(
            parse_waiter_result(
                &json!({ "schema": WAITER_RESULT_SCHEMA, "result_kind": "example.review.result/1" })
            ),
            None,
            "no status to route on"
        );
    }

    #[test]
    fn an_empty_deliver_to_is_valid_and_normal() {
        // "Delivered nowhere" is a supported outcome, not a misconfiguration —
        // so this parses cleanly rather than erroring.
        let minimal = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "reviewed"
        });
        let parsed = parse_waiter_result(&minimal).expect("parses");
        assert!(parsed.deliver_to.is_empty());
        assert!(parsed.is_reviewed());
        assert_eq!(parsed.rendered, None);
        assert_eq!(parsed.summary, None);
        assert_eq!(parsed.head_sha, None);
        assert!(parsed.findings.is_empty());
        assert_eq!(parsed.review_commit_id(), None);
        assert!(parsed.github_review_comments().is_empty());

        let explicit = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "reviewed",
            "deliver_to": []
        });
        assert!(parse_waiter_result(&explicit)
            .expect("parses")
            .deliver_to
            .is_empty());
    }

    #[test]
    fn type_is_an_alias_of_schema() {
        let via_type = json!({
            "$type": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "reviewed"
        });
        let parsed = parse_waiter_result(&via_type).expect("parses from $type alone");
        assert_eq!(parsed.result_kind, EXAMPLE_REVIEW_RESULT_KIND);

        let both = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "$type": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "reviewed"
        });
        assert!(parse_waiter_result(&both).is_some());
    }

    #[test]
    fn an_unknown_surface_is_recorded_not_dropped_and_not_an_error() {
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "reviewed",
            "deliver_to": [
                { "surface": "discord", "webhook": "https://x.test/hook" },
                { "surface": "github", "repo": "acme/widgets", "pr": 7 },
                // A known surface missing its detail: still recorded, because a
                // vanished entry and a skipped one are not the same answer.
                { "surface": "github" },
                { "surface": "slack" },
                // No surface at all: nothing to record under.
                { "repo": "acme/widgets", "pr": 8 }
            ]
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert_eq!(
            parsed.deliver_to,
            vec![
                DeliverTarget::Unknown("discord".into()),
                DeliverTarget::Github {
                    repo: "acme/widgets".into(),
                    pr: 7
                },
                DeliverTarget::Unknown("github".into()),
                DeliverTarget::Unknown("slack".into()),
            ],
            "one unroutable target never sinks the others — partial delivery is the model"
        );
        assert_eq!(parsed.deliver_to[0].surface(), "discord");
    }

    #[test]
    fn a_target_projects_onto_the_egress_target_the_queue_speaks() {
        assert_eq!(
            DeliverTarget::Github {
                repo: "example/repo".into(),
                pr: 3915
            }
            .to_egress_target(),
            Some(EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 3915
            })
        );
        assert_eq!(
            DeliverTarget::Slack {
                channel: "C0123ABCDEF".into(),
                thread_ts: None
            }
            .to_egress_target(),
            Some(EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
                thread_ts: None
            })
        );
        assert_eq!(
            DeliverTarget::Unknown("discord".into()).to_egress_target(),
            None
        );
        assert_eq!(
            DeliverTarget::Unknown("discord".into()).skip_fingerprint(),
            ("discord".into(), String::new()),
            "an unknown surface still has a row to record the skip on"
        );
        assert_eq!(
            DeliverTarget::Slack {
                channel: "#general".into(),
                thread_ts: None
            }
            .skip_fingerprint(),
            ("slack".into(), "#general".into()),
            "a #name is unusable as a destination but the skip keeps the bytes the producer wrote"
        );
    }

    #[test]
    fn a_malformed_target_does_not_project_so_it_is_skipped_not_delivered() {
        for target in [
            DeliverTarget::Github {
                repo: "widgets".into(),
                pr: 1,
            },
            DeliverTarget::Github {
                repo: "/widgets".into(),
                pr: 1,
            },
            DeliverTarget::Github {
                repo: "example/".into(),
                pr: 1,
            },
            // An issue-qualified repo is a producer mistake, not a destination.
            DeliverTarget::Github {
                repo: "example/repo#42".into(),
                pr: 3915,
            },
            DeliverTarget::Github {
                repo: "example/repo".into(),
                pr: 0,
            },
            DeliverTarget::Github {
                repo: "example/repo".into(),
                pr: -1,
            },
            // A mutable name is not an addressable channel.
            DeliverTarget::Slack {
                channel: "#general".into(),
                thread_ts: None,
            },
            DeliverTarget::Slack {
                channel: String::new(),
                thread_ts: None,
            },
        ] {
            assert_eq!(
                target.to_egress_target(),
                None,
                "expected {target:?} not to project"
            );
        }
    }

    #[test]
    fn a_non_reviewed_status_parses_but_does_not_deliver_the_producers_bytes() {
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "failed",
            "deliver_to": [{ "surface": "slack", "channel": "C1" }],
            "rendered": "should not be delivered as a clean pass"
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert!(!parsed.is_reviewed());
        assert_eq!(parsed.status, "failed");
        assert_eq!(
            parsed.deliver_to.len(),
            1,
            "the routing list still parses — the failure notice goes to the same targets"
        );
    }

    #[test]
    fn an_empty_string_body_reads_as_absent() {
        // A producer sending "" is telling us it has nothing, not that the body
        // is the empty string; treating them differently would post a blank comment.
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "reviewed",
            "rendered": "",
            "summary": "",
            "view_url": "",
            "pr": ""
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert_eq!(parsed.rendered, None);
        assert_eq!(parsed.summary, None);
        assert_eq!(parsed.view_url, None);
        assert_eq!(parsed.pr, None);
        assert_eq!(parsed.head_sha, None);
    }

    #[test]
    fn unknown_producer_fields_are_ignored_rather_than_rejected() {
        let value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "reviewed",
            "summary": "ok",
            "a_field_from_a_future_producer": { "nested": [1, 2, 3] },
            "findings": [{ "severity": "critical" }],
            "cost_usd": 0.65
        });
        let parsed = parse_waiter_result(&value).expect("parses");
        assert_eq!(parsed.summary.as_deref(), Some("ok"));
        assert!(
            parsed.findings.is_empty(),
            "a finding without file/body/line_range is skipped, not an error"
        );
    }

    #[test]
    fn head_sha_is_the_only_commit_id_and_garbage_is_absent() {
        let sha = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";
        let parsed = parse_waiter_result(&json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "reviewed",
            "head_sha": sha,
            // A live-PR URL is not a commit. Must not become commit_id.
            "html_url": "https://github.com/acme/widgets/pull/7",
        }))
        .expect("parses");
        assert_eq!(parsed.review_commit_id(), Some(sha));

        for bad in [
            json!(""),
            json!("HEAD"),
            json!("deadbeef"),
            json!("not-a-sha-at-all-even-though-long-enough-xxxx"),
            json!(1),
        ] {
            let parsed = parse_waiter_result(&json!({
                "schema": WAITER_RESULT_SCHEMA,
                "result_kind": "example.review.result/1",
                "status": "reviewed",
                "head_sha": bad,
            }))
            .expect("parses");
            assert_eq!(
                parsed.review_commit_id(),
                None,
                "expected {bad} not to become commit_id"
            );
        }
    }

    #[test]
    fn findings_project_onto_github_right_side_post_image_comments() {
        let parsed = parse_waiter_result(&json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "reviewed",
            "findings": [
                {
                    "file": "auth.py",
                    "body": "bypass",
                    "line_range": { "start": 2, "end": 4 },
                    "severity": "critical"
                },
                {
                    "file": "auth.py",
                    "body": "one line",
                    "line_range": { "start": 7, "end": 7 }
                },
                { "severity": "critical" },
                {
                    "file": "auth.py",
                    "body": "zero is not a line",
                    "line_range": { "start": 0, "end": 1 }
                },
                {
                    "file": "auth.py",
                    "body": "inverted",
                    "line_range": { "start": 9, "end": 3 }
                },
                { "file": "", "body": "x", "line_range": { "start": 1, "end": 1 } },
                { "file": "x.rs", "body": "", "line_range": { "start": 1, "end": 1 } }
            ]
        }))
        .expect("parses");
        assert_eq!(parsed.findings.len(), 2);
        assert_eq!(
            parsed.findings[0].severity.as_deref(),
            Some(FINDING_SEVERITY_CRITICAL),
            "severity is a free string projected off the finding, not an enum"
        );
        assert_eq!(parsed.findings[1].severity, None);

        let comments = parsed.github_review_comments();
        assert_eq!(comments[0].path, "auth.py");
        assert_eq!(comments[0].line, 4);
        assert_eq!(comments[0].start_line, Some(2));
        assert_eq!(comments[0].side, GithubDiffSide::Right);
        assert_eq!(comments[0].side.as_str(), "RIGHT");
        assert_eq!(comments[0].body, "bypass");

        assert_eq!(comments[1].line, 7);
        assert_eq!(
            comments[1].start_line, None,
            "a single-line finding omits start_line"
        );
        assert_eq!(GITHUB_REVIEW_EVENT_COMMENT, "COMMENT");
    }

    #[test]
    fn a_reviewed_pi_review_with_any_critical_finding_is_request_changes() {
        // The load-bearing map. result_kind is compared as a
        // namespaced string — a closed enum would need editing every time a
        // waiter product ships a new kind.
        let critical = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "reviewed",
            "findings": [
                { "severity": "warning" },
                { "severity": FINDING_SEVERITY_CRITICAL }
            ]
        });
        assert_eq!(
            review_decision_from_waiter(&critical),
            Some(ReviewDecision::RequestChanges)
        );

        // A critical finding that 380 would skip (no file/body/line_range)
        // still arms the close-gate adapter.
        let unusable = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "reviewed",
            "findings": [{ "severity": "critical" }]
        });
        assert_eq!(
            review_decision_from_waiter(&unusable),
            Some(ReviewDecision::RequestChanges)
        );
        assert!(
            parse_waiter_result(&unusable)
                .expect("parses")
                .findings
                .is_empty(),
            "380 still skips the unusable finding; 383 still reads its severity"
        );
    }

    #[test]
    fn a_review_without_critical_or_the_wrong_kind_does_not_request_changes() {
        let warning_only = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "reviewed",
            "findings": [{ "severity": "warning" }]
        });
        assert_eq!(review_decision_from_waiter(&warning_only), None);

        let no_findings = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "reviewed"
        });
        assert_eq!(review_decision_from_waiter(&no_findings), None);

        // A different namespaced kind with a critical finding is not this
        // adapter — we do not close an enum of result kinds.
        let other_kind = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": "example.plan.result/1",
            "status": "reviewed",
            "findings": [{ "severity": "critical" }]
        });
        assert_eq!(review_decision_from_waiter(&other_kind), None);

        let not_reviewed = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "failed",
            "findings": [{ "severity": "critical" }]
        });
        assert_eq!(review_decision_from_waiter(&not_reviewed), None);

        assert_eq!(
            review_decision_from_waiter(&json!({ "schema": "nope" })),
            None,
            "an unrecognized envelope is inert, same as delivery"
        );
        assert_ne!(
            review_decision_from_waiter(&json!({
                "schema": WAITER_RESULT_SCHEMA,
                "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
                "status": "reviewed",
                "findings": [{ "severity": "critical" }]
            })),
            Some(ReviewDecision::Approve),
            "the adapter never auto-approves; a human resolves"
        );
    }

    fn reviewed(head_sha: Option<&str>, findings: Value) -> Value {
        let mut value = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "reviewed",
            "summary": "auth bypass",
            "view_url": "https://producer.example.test/r/1",
            "findings": findings,
        });
        if let Some(sha) = head_sha {
            value["head_sha"] = json!(sha);
        }
        value
    }

    #[test]
    fn a_check_run_uses_only_the_envelope_sha() {
        let sha = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";
        let raw = reviewed(Some(sha), json!([]));
        let waiter = parse_waiter_result(&raw).unwrap();
        let check = github_check_run(&waiter, &raw).unwrap();
        assert_eq!(check.head_sha, sha);
        assert_eq!(check.name, GITHUB_CHECK_NAME);
        assert_eq!(check.conclusion, GithubCheckConclusion::Success);
        assert_eq!(check.conclusion.as_str(), "success");
        assert!(check.summary.contains("does not approve"));
        assert_eq!(
            check.details_url.as_deref(),
            Some("https://producer.example.test/r/1")
        );

        let raw = reviewed(None, json!([]));
        let waiter = parse_waiter_result(&raw).unwrap();
        assert!(
            github_check_run(&waiter, &raw).is_none(),
            "no head_sha ⇒ no check; a live PR head must not be fetched"
        );
    }

    #[test]
    fn a_critical_finding_fails_the_check_even_when_it_cannot_be_an_inline_comment() {
        let sha = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";
        let raw = reviewed(Some(sha), json!([{ "severity": "critical" }]));
        let waiter = parse_waiter_result(&raw).unwrap();
        assert!(waiter.findings.is_empty());
        let check = github_check_run(&waiter, &raw).unwrap();
        assert_eq!(check.conclusion, GithubCheckConclusion::Failure);
        assert_eq!(check.title, "Maidan found a critical issue");
        assert!(check.summary.contains("does not request changes"));
    }

    #[test]
    fn a_result_that_was_not_reviewed_fails_the_check() {
        let sha = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";
        let raw = json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
            "status": "failed",
            "head_sha": sha,
            "view_url": "http://insecure.example.test/r",
        });
        let waiter = parse_waiter_result(&raw).unwrap();
        let check = github_check_run(&waiter, &raw).unwrap();
        assert_eq!(check.conclusion, GithubCheckConclusion::Failure);
        assert_eq!(check.title, "Maidan result did not pass");
        assert!(check.summary.contains("status `failed`"));
        assert_eq!(check.details_url, None, "only https details urls are sent");
    }

    const SHA: &str = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";

    fn change_envelope(deliver_to: Value) -> Value {
        json!({
            "schema": WAITER_RESULT_SCHEMA,
            "result_kind": PI_CHANGE_RESULT_KIND,
            "status": "changed",
            "base_sha": SHA,
            "branch": "feature/agent-fix-1a2b",
            "diff": "diff --git a/a b/a\n",
            "title": "Fix the thing",
            "summary": "It was broken.",
            "deliver_to": deliver_to,
        })
    }

    #[test]
    fn a_github_branch_target_parses_and_projects_onto_its_own_surface() {
        let value = change_envelope(json!([
            {"surface":"github_branch","repo":"beatgig/bgv3","branch":"feature/agent-fix-1a2b","base":"dev"},
            {"surface":"slack","channel":"C0123ABCDEF","thread_ts":"1699999999.001200"},
        ]));
        let parsed = parse_waiter_result(&value).expect("parses");
        let branch = &parsed.deliver_to[0];
        assert_eq!(
            branch,
            &DeliverTarget::GithubBranch {
                repo: "beatgig/bgv3".into(),
                branch: "feature/agent-fix-1a2b".into(),
                base: "dev".into(),
            }
        );
        let egress = branch.to_egress_target().expect("usable");
        assert_eq!(
            egress.to_string(),
            "github_branch:beatgig/bgv3@feature/agent-fix-1a2b"
        );
        assert_eq!(egress.allowlist_selector(), "beatgig/bgv3");
        assert_eq!(egress.surface().as_str(), "github_branch");

        let slack = parsed.deliver_to[1].to_egress_target().expect("usable");
        assert_eq!(
            slack,
            EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
                thread_ts: Some("1699999999.001200".into()),
            }
        );
        assert_eq!(
            slack.allowlist_selector(),
            "C0123ABCDEF",
            "a thread rides its channel's blessing"
        );
    }

    #[test]
    fn an_unusable_branch_or_thread_target_is_skipped_not_delivered() {
        for entry in [
            json!({"surface":"github_branch","repo":"beatgig","branch":"feature/x","base":"dev"}),
            json!({"surface":"github_branch","repo":"beatgig/bgv3","branch":"feature/../x","base":"dev"}),
            json!({"surface":"github_branch","repo":"beatgig/bgv3","branch":"feature/x","base":"de v"}),
            json!({"surface":"github_branch","repo":"beatgig/bgv3","branch":"x.lock","base":"dev"}),
            json!({"surface":"slack","channel":"C0123ABCDEF","thread_ts":"yesterday"}),
        ] {
            let target = parse_deliver_target(&entry).expect("recorded");
            assert!(
                target.to_egress_target().is_none(),
                "expected {entry} to be unusable"
            );
        }
        assert_eq!(
            parse_deliver_target(&json!({"surface":"github_branch","repo":"beatgig/bgv3"})),
            Some(DeliverTarget::Unknown("github_branch".into())),
            "a branch target missing its detail is still recorded"
        );
    }

    #[test]
    fn a_change_result_reads_its_commit_fields_and_refuses_a_short_sha() {
        let change = parse_change_result(&change_envelope(json!([]))).expect("a change result");
        assert!(change.is_changed());
        assert_eq!(change.base_sha.as_deref(), Some(SHA));
        assert_eq!(change.branch.as_deref(), Some("feature/agent-fix-1a2b"));
        assert_eq!(change.title.as_deref(), Some("Fix the thing"));

        let mut short = change_envelope(json!([]));
        short["base_sha"] = json!("b5e54f9");
        assert_eq!(parse_change_result(&short).unwrap().base_sha, None);

        let mut review = change_envelope(json!([]));
        review["result_kind"] = json!(EXAMPLE_REVIEW_RESULT_KIND);
        assert!(parse_change_result(&review).is_none());
    }

    #[test]
    fn the_change_branch_rules_are_hard_coded_and_name_their_reason() {
        assert!(check_change_target("feature/agent-fix-greeting-1a2b", "dev").is_ok());
        assert!(check_change_target("feature/agent-9", "main").is_ok());
        for (branch, base, why) in [
            ("main", "dev", "protected"),
            ("prod", "dev", "protected"),
            ("dev", "main", "protected"),
            ("Master", "dev", "protected"),
            ("feature/fix-greeting", "dev", "does not match"),
            ("feature/agent-", "dev", "does not match"),
            ("feature/agent--x", "dev", "does not match"),
            ("feature/agent-Fix", "dev", "does not match"),
            ("feature/agent-x/y", "dev", "does not match"),
            ("feature/agent-x", "prod", "never allowed"),
            ("feature/agent-x", "PROD", "never allowed"),
            ("feature/agent-x", "de v", "not a branch name"),
            ("feature/agent-x", "feature/agent-x", "its own base"),
        ] {
            let err = check_change_target(branch, base).unwrap_err();
            assert!(err.contains(why), "{branch} -> {base}: {err}");
        }
    }

    #[test]
    fn the_mark_ready_base_map_names_its_repo_and_base() {
        // Mapped repos flip only into their mapped base.
        for (repo, base) in [
            ("david-engelmann/bgv3", "dev"),
            ("david-engelmann/relay", "dev"),
            ("david-engelmann/dawn", "dev"),
            ("david-engelmann/wax", "dev"),
            ("david-engelmann/agent-skills", "main"),
        ] {
            assert!(
                check_mark_ready_target(repo, "feature/agent-x", base).is_ok(),
                "{repo} -> {base}"
            );
        }
        // A mapped repo into any other base names the mapped base.
        for (repo, base, want) in [
            ("david-engelmann/wax", "main", "dev"),
            ("david-engelmann/bgv3", "staging", "dev"),
            ("david-engelmann/agent-skills", "dev", "main"),
        ] {
            let err = check_mark_ready_target(repo, "feature/agent-x", base).unwrap_err();
            assert!(err.contains(want), "{repo} -> {base}: {err}");
            assert!(err.contains("only into"), "{repo} -> {base}: {err}");
        }
        // Outside the map the shape rules still apply and the allowlist
        // governs: a well-formed target passes here.
        assert!(check_mark_ready_target("o/repo", "feature/agent-x", "dev").is_ok());
        assert!(check_mark_ready_target("o/repo", "feature/agent-x", "prod").is_err());
        assert!(check_mark_ready_target("o/repo", "hotfix/x", "dev").is_err());
    }

    #[test]
    fn a_branch_target_is_blessed_with_its_base() {
        let target = DeliverTarget::GithubBranch {
            repo: "beatgig/bgv3".into(),
            branch: "feature/agent-x".into(),
            base: "dev".into(),
        };
        assert_eq!(
            target.allowlist_selector().as_deref(),
            Some("beatgig/bgv3@dev")
        );
        let slack = DeliverTarget::Slack {
            channel: "C0123ABCDEF".into(),
            thread_ts: Some("1.2".into()),
        };
        assert_eq!(slack.allowlist_selector().as_deref(), Some("C0123ABCDEF"));
    }
}
