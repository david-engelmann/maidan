//! The change flow: a `pi.change.result/1` becomes a commit on a branch and a
//! draft pull request.
//!
//! A coding seat edits a checkout, but holds no GitHub write credential: it
//! hands back the commit it started from (`base_sha`) and a diff. Maidan,
//! which holds the operator's token, turns that into a commit through the Git
//! Data API (blobs, a tree, a commit, a fast-forward of the branch) and opens a
//! **draft** pull request when none is open for the branch. It never approves,
//! merges, requests a review or comments: a person takes it from the draft.
//!
//! **Refusals are outcomes, not failures.** A branch whose head is not
//! `base_sha`, a result for a different branch, or a diff that does not apply
//! is refused with a reason, recorded on the delivery row and said in Slack.
//! Retrying cannot fix any of them.
//!
//! **Idempotent per thread and result.** The commit message carries a
//! `Maidan-Change: <thread_id>/<digest>` trailer, the digest covering
//! `base_sha` and the diff. A retried or replayed delivery that finds the
//! branch head is already that commit (on top of `base_sha`) reuses it, and the
//! pull request is looked up before one is opened, so a retry makes neither a
//! second commit nor a second pull request.
//!
//! **One force write: the squash-rebase round.** A result with
//! `"rebase": true` carries the whole branch's diff against the base branch's
//! current head. It is rebuilt as one commit on that head and the branch is
//! moved there with `force: true`, the flow's only non-fast-forward write. It
//! happens only when the branch is an agent branch with an open pull request
//! outside the merge queue, `base_sha` is the base's head right now, and the
//! branch head is still the last commit this flow delivered to that pull
//! request, re-read immediately before the move. The move itself goes
//! through [`ForceMovePass`], which only this module's guard can mint.
//!
//! **One comment: the rebase range.** After a squash-rebase moves the
//! branch, the flow comments `Rebased <old_head>..<new_head> onto <base_sha>`
//! on the pull request, and the round records the old head. This is a
//! deliberate exception to "never comments": the force move rewrote the
//! history a reviewer had read, and without the two heads GitHub can no
//! longer show what the rebase changed. It is best effort, like the body
//! edit, so a failed comment never undoes the move.

use maidan_types::{ChangeResult, ThreadId};
use sha2::{Digest, Sha256};

use crate::change_patch::{self, TreeChange};
use crate::github::{GitTreeEntry, GithubError, GithubGit, GithubPull};

/// GitHub's limit on a pull request title.
const TITLE_MAX_CHARS: usize = 256;

/// How long a title made from the thread's opening message may be: a commit
/// subject line, not the whole instruction.
const FALLBACK_TITLE_MAX_CHARS: usize = 72;

/// What a change delivery did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeOutcome {
    /// The branch head is `commit_sha` and `pull` is its open pull request.
    Committed {
        commit_sha: String,
        pull: GithubPull,
        /// For a squash-rebase that moved the branch, the head it moved it
        /// from.
        rebased_from: Option<String>,
        /// What the pull request lacks against the producer's template; see
        /// [`template_warnings`].
        template_warnings: Vec<String>,
    },
    /// Nothing more will be written; the reason is recorded.
    Refused(String),
}

/// One `github_branch` target and the result aimed at it.
pub struct ChangeRequest<'a> {
    pub repo: &'a str,
    pub branch: &'a str,
    pub base: &'a str,
    pub thread_id: ThreadId,
    pub change: &'a ChangeResult,
    /// The thread's opening message (the `!change` instructions): the title
    /// when the result carries none.
    pub opening_message: Option<&'a str>,
    /// The repository's merge-queue label (`MAIDAN_MERGE_QUEUE_LABELS`), if
    /// it has one.
    pub merge_queue_label: Option<&'a str>,
    /// For a squash-rebase round: the commit of the newest round this flow
    /// recorded for the branch's open pull request in this workspace. `None`
    /// when it delivered nothing there, which refuses the rebase.
    pub delivered_head: Option<DeliveredHead>,
}

/// The last commit the change flow delivered to a pull request, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredHead {
    pub pull_number: i64,
    pub commit_sha: String,
}

impl ChangeRequest<'_> {
    /// The result's `title`, else the opening message cut at a word, else the
    /// branch. A missing title never stops a delivery.
    pub fn title(&self) -> String {
        if let Some(title) = self.change.title.as_deref() {
            return clip(first_line(title), TITLE_MAX_CHARS);
        }
        self.opening_message
            .map(|m| clip_at_word(first_line(m), FALLBACK_TITLE_MAX_CHARS))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| self.branch.to_string())
    }

    /// The pull request body: the result's `summary`, else a line naming the
    /// thread and the commit. Mentions are defused either way.
    pub fn body(&self, commit_sha: &str) -> String {
        let body = match self.change.summary.as_deref() {
            Some(summary) => summary.to_string(),
            None => format!(
                "Opened by Maidan for thread {} at commit {commit_sha}.",
                self.thread_id.0
            ),
        };
        crate::egress_body::neutralize_github_mentions(&body)
    }
}

/// The warning for a pull request body with no `Testing` heading.
pub const MISSING_TESTING: &str = "missing_testing";
/// The warning for a pull request whose title and body name no issue key.
pub const MISSING_ISSUE_KEY: &str = "missing_issue_key";

/// What a pull request lacks against the producer's template: a `Testing`
/// heading in the body, and an issue key (`ENG-123`) in the title or body.
/// It flags and never refuses: the change lands as a draft either way, and
/// the person or the mark-ready step decides what a warning holds up.
pub fn template_warnings(title: &str, body: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if !has_testing_heading(body) {
        warnings.push(MISSING_TESTING.to_string());
    }
    if !has_issue_key(title) && !has_issue_key(body) {
        warnings.push(MISSING_ISSUE_KEY.to_string());
    }
    warnings
}

/// The Slack line naming what the pull request lacks, or `None` when it
/// lacks nothing.
pub fn template_note(warnings: &[String]) -> Option<String> {
    let missing: Vec<&str> = warnings
        .iter()
        .map(|code| match code.as_str() {
            MISSING_TESTING => "a Testing section",
            MISSING_ISSUE_KEY => "an issue key (like ENG-123) in the title or body",
            other => other,
        })
        .collect();
    (!missing.is_empty()).then(|| format!("PR template: missing {}.", missing.join(" and ")))
}

/// A Markdown heading reading `Testing`, at any level, outside fenced code:
/// pasted check output can hold a line that starts with `#`.
fn has_testing_heading(body: &str) -> bool {
    let mut fence: Option<&str> = None;
    for line in body.lines().map(str::trim_start) {
        if let Some(open) = fence {
            if line.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = ["```", "~~~"].into_iter().find(|f| line.starts_with(f)) {
            fence = Some(open);
            continue;
        }
        let Some(rest) = line.strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start_matches('#');
        // `#1463` or `#tag` is not a heading.
        if !rest.starts_with([' ', '\t']) {
            continue;
        }
        let text = rest
            .trim()
            .trim_end_matches('#')
            .trim_end()
            .trim_end_matches(':');
        if text.eq_ignore_ascii_case("testing") {
            return true;
        }
    }
    false
}

/// An issue key standing alone: an uppercase letter, then uppercase letters
/// or digits, a hyphen and digits (`ENG-123`, `[ENG-123]`).
fn has_issue_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let word_start = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if !(word_start && bytes[i].is_ascii_uppercase()) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j].is_ascii_uppercase() || bytes[j].is_ascii_digit()) {
            j += 1;
        }
        if j - i >= 2 && bytes.get(j) == Some(&b'-') {
            let mut k = j + 1;
            while k < bytes.len() && bytes[k].is_ascii_digit() {
                k += 1;
            }
            if k > j + 1 && bytes.get(k).is_none_or(|b| !b.is_ascii_alphanumeric()) {
                return true;
            }
        }
        i = j;
    }
    false
}

/// The markers around the section of a pull request body that Maidan keeps
/// current from the producer's `pr_state.body`.
const CURRENT_STATE_START: &str = "<!-- pi:current-state -->";
const CURRENT_STATE_END: &str = "<!-- /pi:current-state -->";
/// Room for the section, well inside GitHub's 65,536-character body.
const CURRENT_STATE_MAX_CHARS: usize = 20_000;

/// `body` with its current-state section replaced by `state`, or the section
/// appended when the markers are missing. Everything outside the markers
/// (people's edits, review bots' summaries) is kept. Mentions are defused,
/// and markers inside `state` are dropped so it cannot nest a section.
pub fn with_current_state(body: &str, state: &str) -> String {
    let state = state
        .replace(CURRENT_STATE_START, "")
        .replace(CURRENT_STATE_END, "");
    let state = crate::egress_body::neutralize_github_mentions(state.trim());
    let state = clip(&state, CURRENT_STATE_MAX_CHARS);
    let section = format!("{CURRENT_STATE_START}\n{state}\n{CURRENT_STATE_END}");
    if let Some(start) = body.find(CURRENT_STATE_START) {
        if let Some(end) = body[start..].find(CURRENT_STATE_END) {
            let end = start + end + CURRENT_STATE_END.len();
            return format!("{}{section}{}", &body[..start], &body[end..]);
        }
    }
    if body.trim().is_empty() {
        section
    } else {
        format!("{}\n\n{section}", body.trim_end())
    }
}

fn first_line(text: &str) -> &str {
    text.trim().lines().next().unwrap_or_default().trim()
}

/// `text` cut to at most `max` characters, at the last space when there is
/// one, with an ellipsis when anything was cut.
fn clip_at_word(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    let kept = match cut.rfind(' ') {
        Some(at) if at > 0 => cut[..at].trim_end(),
        _ => cut.as_str(),
    };
    format!("{kept}…")
}

/// The trailer that names this thread's result in its commit. A
/// squash-rebase round's digest also covers the flag, so a rebase never
/// mistakes a normal round's commit with the same base and diff for its own
/// and reports a rebase that never happened.
pub fn change_trailer(thread_id: ThreadId, base_sha: &str, diff: &str, rebase: bool) -> String {
    let mut hasher = Sha256::new();
    hasher.update(base_sha.as_bytes());
    hasher.update([0u8]);
    hasher.update(diff.as_bytes());
    if rebase {
        hasher.update(b"\0rebase");
    }
    let digest = hex::encode(hasher.finalize());
    format!("Maidan-Change: {}/{}", thread_id.0, &digest[..16])
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Commit the change and make sure a draft pull request is open for it.
///
/// `Err` is a GitHub call that failed and may succeed later (the caller
/// retries, or dead-letters a misconfiguration); `Ok(Refused)` is final.
pub async fn deliver_change(
    git: &dyn GithubGit,
    req: &ChangeRequest<'_>,
) -> Result<ChangeOutcome, GithubError> {
    let refused = |reason: String| Ok(ChangeOutcome::Refused(reason));
    // The last check before GitHub: the only ref this function creates or
    // moves is `req.branch`, and only when it is an agent branch.
    if let Err(reason) = maidan_types::check_change_target(req.branch, req.base) {
        return refused(reason);
    }
    let change = req.change;
    if !change.is_changed() {
        return refused(format!(
            "status `{}`: there is nothing to commit",
            change.status
        ));
    }
    let Some(base_sha) = change.base_sha.as_deref() else {
        return refused("the result has no full `base_sha`".into());
    };
    match change.branch.as_deref() {
        Some(branch) if branch == req.branch => {}
        Some(branch) => {
            return refused(format!(
                "the result is for branch `{branch}`, not the target branch `{}`",
                req.branch
            ))
        }
        None => return refused("the result does not name its `branch`".into()),
    }
    let Some(diff) = change.diff.as_deref() else {
        return refused("the result has no `diff`".into());
    };
    let patches = match change_patch::parse(diff) {
        Ok(patches) => patches,
        Err(err) => return refused(format!("the diff does not apply: {err}")),
    };
    let trailer = change_trailer(req.thread_id, base_sha, diff, change.rebase);

    // An open pull request for the branch is checked before anything is
    // written: one into another base (`prod`, say) refuses the whole change.
    let existing = git.open_pull(req.repo, req.branch).await?;
    if let Some(pull) = existing.as_ref().filter(|p| p.base != req.base) {
        return refused(format!(
            "the open pull request #{} for `{}` targets `{}`, not the allowed base `{}`",
            pull.number, req.branch, pull.base, req.base
        ));
    }
    // A queued pull request lands the head it was queued with as soon as its
    // checks pass, so a commit pushed now would land separately, or not at
    // all. The caller dequeues it first (`DELETE /operator/github/enqueue`).
    if let (Some(label), Some(pull)) = (req.merge_queue_label, existing.as_ref()) {
        if pull.labels.iter().any(|name| name == label) {
            return refused(format!(
                "pull request #{} carries `{label}` and is in the merge queue; dequeue it before pushing another commit",
                pull.number
            ));
        }
    }
    let built = if change.rebase {
        rebase_commit(git, req, base_sha, &patches, &trailer, existing.as_ref()).await?
    } else {
        fast_forward_commit(git, req, base_sha, &patches, &trailer)
            .await?
            .map(|commit| (commit, None))
    };
    let (commit_sha, rebased_from) = match built {
        Ok(built) => built,
        Err(reason) => return refused(reason),
    };

    let opened_now = existing.is_none();
    // What this round opened the pull request with: GitHub's create answer
    // may omit the title and body.
    let mut opened_with = None;
    let pull = match existing {
        Some(pull) => pull,
        None => {
            let title = req.title();
            let mut body = req.body(&commit_sha);
            if let Some(state) = req.change.current_state.as_deref() {
                body = with_current_state(&body, state);
            }
            let created = git
                .create_draft_pull(req.repo, req.branch, req.base, &title, &body)
                .await;
            match created {
                Ok(pull) => {
                    opened_with = Some((title, body));
                    pull
                }
                // Opened by someone else between the read and the write.
                Err(err) if err.is_unprocessable() => {
                    match git.open_pull(req.repo, req.branch).await? {
                        Some(pull) => pull,
                        // GitHub answers 422 for a branch it created a moment
                        // ago and has not indexed yet. The commit is on the
                        // branch and a retry reuses it, so this is a failure
                        // to retry, not a refusal: refused, the change would
                        // sit on its branch with no pull request.
                        None => return Err(err),
                    }
                }
                Err(err) => return Err(err),
            }
        }
    };
    // A pull request found after the commit (opened concurrently) is held to
    // the same rule.
    if pull.base != req.base {
        return refused(format!(
            "committed {commit_sha}, but the open pull request #{} for `{}` targets `{}`, not the allowed base `{}`",
            pull.number, req.branch, pull.base, req.base
        ));
    }
    // Keep the description current. A pull request opened just now already
    // has the section (and the create answer may omit the body). Best effort:
    // the commit stands whether or not the body edit lands, and the next
    // round writes it again.
    let mut current_body = pull.body.clone();
    if let Some(state) = req.change.current_state.as_deref().filter(|_| !opened_now) {
        let body = with_current_state(&pull.body, state);
        if body != pull.body {
            match git.update_pull_body(req.repo, pull.number, &body).await {
                Ok(()) => current_body = body,
                Err(err) => {
                    tracing::warn!(repo = req.repo, pull = pull.number, error = %err, "change flow: pull request body not updated");
                }
            }
        }
    }
    // Checked against the pull request as it now reads, not this round's
    // result: a later round's title and summary never reach an open one.
    let (title, body) = opened_with.unwrap_or_else(|| (pull.title.clone(), current_body));
    let template_warnings = template_warnings(&title, &body);
    if let Some(old_head) = rebased_from.as_deref() {
        let note = rebase_note(old_head, &commit_sha, base_sha);
        if let Err(err) = git.comment_on_pull(req.repo, pull.number, &note).await {
            tracing::warn!(repo = req.repo, pull = pull.number, error = %err, "change flow: rebase range not commented");
        }
    }
    Ok(ChangeOutcome::Committed {
        commit_sha,
        pull,
        rebased_from,
        template_warnings,
    })
}

/// The line that names a squash-rebase's range, on the pull request and in
/// the Slack reply alike.
pub fn rebase_note(old_head: &str, new_head: &str, base_sha: &str) -> String {
    format!("Rebased {old_head}..{new_head} onto {base_sha}")
}

/// What building a round's commit came to: `Ok` is the branch's new head,
/// `Err` a final refusal with nothing written to the branch.
type Built = Result<String, String>;

/// What a squash-rebase came to: `Ok` is the branch's new head and, when it
/// moved the branch, the head it moved it from.
type Rebuilt = Result<(String, Option<String>), String>;

/// A normal round: commit the diff on `base_sha` and fast-forward the branch
/// to it, creating the branch when it does not exist yet.
async fn fast_forward_commit(
    git: &dyn GithubGit,
    req: &ChangeRequest<'_>,
    base_sha: &str,
    patches: &[change_patch::FilePatch],
    trailer: &str,
) -> Result<Built, GithubError> {
    let head = git.branch_head(req.repo, req.branch).await?;
    if let Some(head) = head.as_deref().filter(|head| *head != base_sha) {
        // A retry of this very result finds its own commit on top of
        // `base_sha`; anything else means the branch moved on.
        let commit = git.commit(req.repo, head).await?;
        if commit.parents != [base_sha] || !commit.message.contains(trailer) {
            return Ok(Err(format!(
                "branch `{}` is at {head}, not the result's base_sha {base_sha}",
                req.branch
            )));
        }
        return Ok(Ok(head.to_string()));
    }
    // Read and apply before writing anything, so a diff that does not apply
    // leaves no branch behind.
    let changes = match apply_at(git, req.repo, base_sha, patches).await? {
        Ok(changes) => changes,
        Err(reason) => return Ok(Err(reason)),
    };
    if head.is_none() {
        match git.create_branch(req.repo, req.branch, base_sha).await {
            Ok(()) => {}
            // Either the branch appeared between the read and the write, or
            // GitHub does not know `base_sha`.
            Err(err) if err.is_unprocessable() => {
                match git.branch_head(req.repo, req.branch).await? {
                    Some(now) if now == base_sha => {}
                    Some(now) => {
                        return Ok(Err(format!(
                            "branch `{}` is at {now}, not the result's base_sha {base_sha}",
                            req.branch
                        )))
                    }
                    None => {
                        return Ok(Err(format!(
                        "GitHub refused to create branch `{}` at {base_sha}; is that commit in {}?",
                        req.branch, req.repo
                    )))
                    }
                }
            }
            Err(err) => return Err(err),
        }
    }
    let commit = commit_on(git, req, base_sha, &changes, trailer).await?;
    match git.update_branch(req.repo, req.branch, &commit).await {
        Ok(()) => Ok(Ok(commit)),
        // Only a branch that really moved is a refusal. GitHub also answers
        // 422 for a ref it created a moment ago; with the branch still at
        // `base_sha` the retry finds nothing in its way, and refusing would
        // drop the change.
        Err(err) if err.is_unprocessable() => match git.branch_head(req.repo, req.branch).await? {
            Some(now) if now == base_sha => Err(err),
            _ => Ok(Err(format!(
                "branch `{}` moved while the commit was being made",
                req.branch
            ))),
        },
        Err(err) => Err(err),
    }
}

/// A squash-rebase round: rebuild the whole branch as one commit on the base
/// branch's current head and force-move the branch there. Every refusal
/// comes before any write to the branch.
async fn rebase_commit(
    git: &dyn GithubGit,
    req: &ChangeRequest<'_>,
    base_sha: &str,
    patches: &[change_patch::FilePatch],
    trailer: &str,
    pull: Option<&GithubPull>,
) -> Result<Rebuilt, GithubError> {
    // A rebase rewrites a branch people are reviewing; with no pull request
    // there is nothing it is keeping current, and no delivered head to
    // vouch for the branch.
    let Some(pull) = pull else {
        return Ok(Err(format!(
            "a rebase round needs the open pull request for `{}`, and none is open",
            req.branch
        )));
    };
    let Some(head) = git.branch_head(req.repo, req.branch).await? else {
        return Ok(Err(format!(
            "branch `{}` does not exist, so there is nothing to rebase",
            req.branch
        )));
    };
    let delivered = req
        .delivered_head
        .as_ref()
        .filter(|d| d.pull_number == pull.number);
    // A retry of this very rebase finds its own commit on `base_sha` with
    // this result's trailer, and reuses it whatever the base has done since.
    // When no round records that commit yet, the move went unrecorded, so
    // the head it moved from is still the delivered one.
    let commit = git.commit(req.repo, &head).await?;
    if commit.parents == [base_sha] && commit.message.contains(trailer) {
        let moved_from = delivered
            .map(|d| d.commit_sha.clone())
            .filter(|from| !from.eq_ignore_ascii_case(&head));
        return Ok(Ok((head, moved_from)));
    }
    match delivered {
        Some(delivered) if delivered.commit_sha.eq_ignore_ascii_case(&head) => {}
        Some(delivered) => {
            return Ok(Err(format!(
                "branch `{}` is at {head}, but the last commit Maidan delivered to #{} is {}; someone else pushed, so it is not rebased",
                req.branch, pull.number, delivered.commit_sha
            )))
        }
        None => {
            return Ok(Err(format!(
                "branch `{}` is at {head}, and Maidan has delivered no commit to #{} in this workspace, so it is not rebased",
                req.branch, pull.number
            )))
        }
    }
    // A stale `base_sha` would move the branch backwards past work that has
    // landed on the base since.
    let base_head = git.branch_head(req.repo, req.base).await?;
    if let Err(reason) = check_base_current(req.repo, req.base, base_sha, base_head.as_deref()) {
        return Ok(Err(reason));
    }
    let changes = match apply_at(git, req.repo, base_sha, patches).await? {
        Ok(changes) => changes,
        Err(reason) => return Ok(Err(reason)),
    };
    let commit = commit_on(git, req, base_sha, &changes, trailer).await?;
    match force::move_branch(git, req.repo, req.branch, &head, &commit).await? {
        Ok(()) => Ok(Ok((commit, Some(head)))),
        Err(reason) => Ok(Err(reason)),
    }
}

/// Whether `base_sha` is the base branch's head as just read. SHAs compare
/// case-insensitively, as every other SHA comparison in this flow does: a
/// producer may send the hex in upper case, and the same commit must not read
/// as a stale base.
fn check_base_current(
    repo: &str,
    base: &str,
    base_sha: &str,
    base_head: Option<&str>,
) -> Result<(), String> {
    match base_head {
        Some(head) if head.eq_ignore_ascii_case(base_sha) => Ok(()),
        Some(head) => Err(format!(
            "the rebase's base_sha {base_sha} is not the head of `{base}`, which is at {head}; rebase onto the current base"
        )),
        None => Err(format!("base branch `{base}` does not exist in {repo}")),
    }
}

/// The tree changes `patches` make to the files at `base_sha`, or the reason
/// they do not apply.
async fn apply_at(
    git: &dyn GithubGit,
    repo: &str,
    base_sha: &str,
    patches: &[change_patch::FilePatch],
) -> Result<Result<Vec<TreeChange>, String>, GithubError> {
    let mut changes = Vec::new();
    for patch in patches {
        let old = match &patch.old_path {
            Some(path) => git.file_at(repo, path, base_sha).await?,
            None => None,
        };
        match patch.apply(old.as_deref()) {
            Ok(more) => changes.extend(more),
            Err(err) => return Ok(Err(format!("the diff does not apply: {err}"))),
        }
    }
    Ok(Ok(changes))
}

/// Write `changes` as one commit whose only parent is `base_sha`.
async fn commit_on(
    git: &dyn GithubGit,
    req: &ChangeRequest<'_>,
    base_sha: &str,
    changes: &[TreeChange],
    trailer: &str,
) -> Result<String, GithubError> {
    let base_commit = git.commit(req.repo, base_sha).await?;
    let entries = tree_entries(git, req.repo, changes).await?;
    let tree = git
        .create_tree(req.repo, &base_commit.tree_sha, &entries)
        .await?;
    let message = commit_message(&req.title(), req.change, trailer);
    git.create_commit(req.repo, &message, &tree, &[base_sha.to_string()])
        .await
}

pub use force::ForceMovePass;

/// The force-move guard, alone in its own module so that nothing else, not
/// even the rest of the change flow, can construct a [`ForceMovePass`].
mod force {
    use crate::github::{GithubError, GithubGit};

    /// Leave to force-move one agent branch from one commit to another, once.
    /// Its fields are private to this module and it is neither `Clone` nor
    /// `Default`, so [`GithubGit::force_move_branch`] can only ever be handed
    /// a pass that [`move_branch`] minted after its checks.
    #[derive(Debug)]
    pub struct ForceMovePass {
        branch: String,
        from: String,
        to: String,
    }

    impl ForceMovePass {
        /// The agent branch to move.
        pub fn branch(&self) -> &str {
            &self.branch
        }

        /// The head the branch had when the pass was minted.
        pub fn from(&self) -> &str {
            &self.from
        }

        /// The commit the branch moves to.
        pub fn to(&self) -> &str {
            &self.to
        }
    }

    /// The pass, if `branch` is an agent branch whose head, read just now,
    /// is still `expected`.
    pub(super) fn authorize(
        branch: &str,
        expected: &str,
        observed: Option<&str>,
        to: &str,
    ) -> Result<ForceMovePass, String> {
        if !maidan_types::is_change_branch(branch) {
            return Err(format!(
                "branch `{branch}` is not an agent branch, and only agent branches are ever force-moved"
            ));
        }
        match observed {
            Some(head) if head.eq_ignore_ascii_case(expected) => Ok(ForceMovePass {
                branch: branch.to_string(),
                from: head.to_string(),
                to: to.to_string(),
            }),
            Some(head) => Err(format!(
                "branch `{branch}` moved to {head} while the rebase commit was being made (expected {expected}); nothing was forced"
            )),
            None => Err(format!(
                "branch `{branch}` was deleted while the rebase commit was being made; nothing was forced"
            )),
        }
    }

    /// Re-read the head immediately before the move, then force it.
    ///
    /// **This is not atomic.** GitHub's REST refs API has no compare-and-swap
    /// (`PATCH .../git/refs` takes no expected old value), so a push that
    /// lands between this read and the `PATCH` is overwritten. The window is
    /// one round trip. That is acceptable because the branch is an agent
    /// branch only Maidan delivers to, the pull request is out of the merge
    /// queue, and an overwritten commit stays reachable on GitHub and in the
    /// pusher's checkout. A later round then refuses, since the head no longer
    /// matches what was delivered. Don't rely on this guard as a lock.
    pub(super) async fn move_branch(
        git: &dyn GithubGit,
        repo: &str,
        branch: &str,
        expected: &str,
        to: &str,
    ) -> Result<Result<(), String>, GithubError> {
        let observed = git.branch_head(repo, branch).await?;
        match authorize(branch, expected, observed.as_deref(), to) {
            Ok(pass) => git.force_move_branch(repo, pass).await.map(Ok),
            Err(reason) => Ok(Err(reason)),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const A: &str = "1111111111111111111111111111111111111111";
        const B: &str = "2222222222222222222222222222222222222222";

        #[test]
        fn a_branch_without_the_agent_shape_never_gets_a_pass() {
            for branch in [
                "main",
                "prod",
                "dev",
                "feature/agent-",
                "feature/x",
                "Feature/agent-x",
                "feature/agent-x/../main",
            ] {
                let refused = authorize(branch, A, Some(A), B);
                assert!(refused.is_err(), "{branch} was given a pass");
            }
        }

        #[test]
        fn a_head_that_moved_or_vanished_since_the_check_gets_no_pass() {
            let moved = authorize("feature/agent-x", A, Some(B), B).unwrap_err();
            assert!(moved.contains(B) && moved.contains(A), "{moved}");
            assert!(authorize("feature/agent-x", A, None, B).is_err());
        }

        #[test]
        fn an_agent_branch_still_at_the_delivered_head_gets_a_pass_for_that_move() {
            let pass = authorize("feature/agent-x", A, Some(A), B).unwrap();
            assert_eq!(
                (pass.branch(), pass.from(), pass.to()),
                ("feature/agent-x", A, B)
            );
        }
    }
}

fn commit_message(title: &str, change: &ChangeResult, trailer: &str) -> String {
    match change.summary.as_deref() {
        Some(summary) => format!("{title}\n\n{summary}\n\n{trailer}\n"),
        None => format!("{title}\n\n{trailer}\n"),
    }
}

async fn tree_entries(
    git: &dyn GithubGit,
    repo: &str,
    changes: &[TreeChange],
) -> Result<Vec<GitTreeEntry>, GithubError> {
    let mut entries = Vec::with_capacity(changes.len());
    for change in changes {
        let blob_sha = match &change.content {
            Some(content) => Some(git.create_blob(repo, content).await?),
            None => None,
        };
        entries.push(GitTreeEntry {
            path: change.path.clone(),
            mode: change.mode.clone(),
            blob_sha,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(change: &'a ChangeResult, opening: Option<&'a str>) -> ChangeRequest<'a> {
        ChangeRequest {
            repo: "example/app",
            branch: "feature/agent-x",
            base: "dev",
            thread_id: ThreadId::new(),
            change,
            opening_message: opening,
            merge_queue_label: None,
            delivered_head: None,
        }
    }

    fn change(title: Option<&str>, summary: Option<&str>) -> ChangeResult {
        ChangeResult {
            status: "changed".into(),
            base_sha: None,
            branch: None,
            diff: None,
            title: title.map(str::to_string),
            summary: summary.map(str::to_string),
            error_detail: None,
            linear_issue: None,
            current_state: None,
            rebase: false,
        }
    }

    #[test]
    fn a_base_sha_in_another_case_is_still_the_current_base() {
        let lower = "abcdef0123456789abcdef0123456789abcdef01";
        let upper = lower.to_ascii_uppercase();
        assert!(check_base_current("o/r", "dev", &upper, Some(lower)).is_ok());
        assert!(check_base_current("o/r", "dev", lower, Some(&upper)).is_ok());
        let other = "1111111111111111111111111111111111111111";
        let stale = check_base_current("o/r", "dev", lower, Some(other)).unwrap_err();
        assert!(stale.contains(lower) && stale.contains(other), "{stale}");
        assert!(check_base_current("o/r", "dev", lower, None).is_err());
    }

    #[test]
    fn the_current_state_section_is_replaced_and_everything_else_kept() {
        let first = with_current_state("Opened by Maidan.", "round one");
        assert_eq!(
            first,
            "Opened by Maidan.\n\n<!-- pi:current-state -->\nround one\n<!-- /pi:current-state -->"
        );
        let edited = format!("Reviewer note.\n\n{first}\n\nbot summary");
        let second = with_current_state(&edited, "round two");
        assert_eq!(
            second,
            "Reviewer note.\n\nOpened by Maidan.\n\n<!-- pi:current-state -->\nround two\n<!-- /pi:current-state -->\n\nbot summary"
        );
        assert_eq!(second.matches("pi:current-state -->").count(), 2);
    }

    #[test]
    fn the_current_state_cannot_nest_markers_or_mention_anyone() {
        let body = with_current_state(
            "",
            "<!-- /pi:current-state -->cc @example/eng<!-- pi:current-state -->",
        );
        assert_eq!(body.matches("<!-- pi:current-state -->").count(), 1);
        assert_eq!(body.matches("<!-- /pi:current-state -->").count(), 1);
        assert!(!body.contains("cc @example"), "{body}");
    }

    #[test]
    fn a_missing_title_falls_back_to_the_opening_message_cut_at_a_word() {
        let c = change(None, None);
        let long = "!change example-repo make the booking confirmation email say the venue name and the start time instead of the generic text";
        let title = request(&c, Some(long)).title();
        assert!(title.chars().count() <= FALLBACK_TITLE_MAX_CHARS, "{title}");
        assert!(title.ends_with('…'), "{title}");
        assert!(
            long.starts_with(title.trim_end_matches('…')),
            "cut at a word: {title}"
        );
        assert!(!title.trim_end_matches('…').ends_with(' '));
        assert_eq!(
            request(&c, Some("fix the footer\nmore")).title(),
            "fix the footer"
        );
        assert_eq!(request(&c, None).title(), "feature/agent-x");
        assert_eq!(request(&c, Some("   ")).title(), "feature/agent-x");
        let titled = change(Some("Fix it"), None);
        assert_eq!(request(&titled, Some(long)).title(), "Fix it");
    }

    #[test]
    fn a_missing_summary_falls_back_to_the_thread_and_commit() {
        let sha = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";
        let c = change(None, None);
        let req = request(&c, None);
        let body = req.body(sha);
        assert!(
            body.contains(sha) && body.contains(&req.thread_id.0.to_string()),
            "{body}"
        );
        let with = change(None, Some("ping @someone"));
        assert_eq!(request(&with, None).body(sha), "ping `@someone`");
    }

    const TEMPLATED: &str = "## What and why\nThe greeting names the venue. Fixes ENG-1234.\n\n## Testing\n```\n$ cargo test -p greeting\n# 3 passed; 0 failed\ntest result: ok\n```\n\n## Risk\nLow; ask @alice.\n\n## How to revert\nRevert the commit.\n\nPart of ENG-1200\n";

    #[test]
    fn a_templated_body_passes_through_with_only_its_mentions_defused() {
        let c = change(Some("[ENG-1234] Name the venue"), Some(TEMPLATED));
        let req = request(&c, None);
        assert_eq!(req.title(), "[ENG-1234] Name the venue");
        assert_eq!(
            req.body("b5e54f94fd04d6ef7d6e1197ddd59ace70edb911"),
            TEMPLATED.replace("@alice", "`@alice`")
        );
    }

    #[test]
    fn a_complete_template_has_no_warnings() {
        assert!(template_warnings("[ENG-1234] Name the venue", TEMPLATED).is_empty());
        assert!(template_warnings("Name the venue", TEMPLATED).is_empty());
        assert_eq!(template_note(&[]), None);
    }

    #[test]
    fn a_body_without_testing_or_a_key_gets_both_warnings() {
        let warnings = template_warnings("Greet the world", "Changes the greeting.");
        assert_eq!(warnings, vec![MISSING_TESTING, MISSING_ISSUE_KEY]);
        assert_eq!(
            template_note(&warnings).as_deref(),
            Some("PR template: missing a Testing section and an issue key (like ENG-123) in the title or body.")
        );
    }

    #[test]
    fn the_testing_heading_is_any_level_and_case_but_never_inside_a_fence() {
        for heading in ["## Testing", "### testing", "# TESTING:", "## Testing ##"] {
            assert!(
                has_testing_heading(&format!("x\n{heading}\nran it")),
                "{heading}"
            );
        }
        for body in [
            "Testing: ran it",
            "#Testing",
            "## Testing plan",
            "```\n## Testing\n```",
            "~~~\n## Testing\n~~~",
        ] {
            assert!(!has_testing_heading(body), "{body}");
        }
    }

    #[test]
    fn an_issue_key_must_stand_alone() {
        for text in ["ENG-1", "[ENG-42] Fix", "Fixes ENG-42.", "Part of AB2-7"] {
            assert!(has_issue_key(text), "{text}");
        }
        for text in [
            "eng-42", "E-42", "XENG-", "ENG-", "ENG-42a", "aENG-42", "#1463",
        ] {
            assert!(!has_issue_key(text), "{text}");
        }
    }
}
