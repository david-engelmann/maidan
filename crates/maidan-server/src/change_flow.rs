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

/// The trailer that names this thread's result in its commit.
pub fn change_trailer(thread_id: ThreadId, base_sha: &str, diff: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(base_sha.as_bytes());
    hasher.update([0u8]);
    hasher.update(diff.as_bytes());
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
    let trailer = change_trailer(req.thread_id, base_sha, diff);

    // An open pull request for the branch is checked before anything is
    // written: one into another base (`prod`, say) refuses the whole change.
    let existing = git.open_pull(req.repo, req.branch).await?;
    if let Some(pull) = existing.as_ref().filter(|p| p.base != req.base) {
        return refused(format!(
            "the open pull request #{} for `{}` targets `{}`, not the allowed base `{}`",
            pull.number, req.branch, pull.base, req.base
        ));
    }
    let head = git.branch_head(req.repo, req.branch).await?;
    let commit_sha = match head {
        Some(head) if head != base_sha => {
            // A retry of this very result finds its own commit on top of
            // `base_sha`; anything else means the branch moved on.
            let commit = git.commit(req.repo, &head).await?;
            if commit.parents != [base_sha] || !commit.message.contains(&trailer) {
                return refused(format!(
                    "branch `{}` is at {head}, not the result's base_sha {base_sha}",
                    req.branch
                ));
            }
            head
        }
        head => {
            // Read and apply before writing anything, so a diff that does not
            // apply leaves no branch behind.
            let mut changes = Vec::new();
            for patch in &patches {
                let old = match &patch.old_path {
                    Some(path) => git.file_at(req.repo, path, base_sha).await?,
                    None => None,
                };
                match patch.apply(old.as_deref()) {
                    Ok(more) => changes.extend(more),
                    Err(err) => return refused(format!("the diff does not apply: {err}")),
                }
            }
            if head.is_none() {
                match git.create_branch(req.repo, req.branch, base_sha).await {
                    Ok(()) => {}
                    // Either the branch appeared between the read and the
                    // write, or GitHub does not know `base_sha`.
                    Err(err) if err.is_unprocessable() => {
                        match git.branch_head(req.repo, req.branch).await? {
                            Some(now) if now == base_sha => {}
                            Some(now) => {
                                return refused(format!(
                                    "branch `{}` is at {now}, not the result's base_sha {base_sha}",
                                    req.branch
                                ))
                            }
                            None => {
                                return refused(format!(
                                    "GitHub refused to create branch `{}` at {base_sha}; is that commit in {}?",
                                    req.branch, req.repo
                                ))
                            }
                        }
                    }
                    Err(err) => return Err(err),
                }
            }
            let base_commit = git.commit(req.repo, base_sha).await?;
            let entries = tree_entries(git, req.repo, &changes).await?;
            let tree = git
                .create_tree(req.repo, &base_commit.tree_sha, &entries)
                .await?;
            let message = commit_message(&req.title(), change, &trailer);
            let commit = git
                .create_commit(req.repo, &message, &tree, &[base_sha.to_string()])
                .await?;
            match git.update_branch(req.repo, req.branch, &commit).await {
                Ok(()) => commit,
                Err(err) if err.is_unprocessable() => {
                    return refused(format!(
                        "branch `{}` moved while the commit was being made",
                        req.branch
                    ))
                }
                Err(err) => return Err(err),
            }
        }
    };

    let pull = match existing {
        Some(pull) => pull,
        None => {
            let title = req.title();
            let body = req.body(&commit_sha);
            match git
                .create_draft_pull(req.repo, req.branch, req.base, &title, &body)
                .await
            {
                Ok(pull) => pull,
                // Opened by someone else between the read and the write.
                Err(err) if err.is_unprocessable() => {
                    match git.open_pull(req.repo, req.branch).await? {
                        Some(pull) => pull,
                        None => {
                            return refused(format!(
                            "committed {commit_sha}, but GitHub refused a pull request into `{}`",
                            req.base
                        ))
                        }
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
    Ok(ChangeOutcome::Committed { commit_sha, pull })
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
        }
    }

    #[test]
    fn a_missing_title_falls_back_to_the_opening_message_cut_at_a_word() {
        let c = change(None, None);
        let long = "!change bgv3 make the booking confirmation email say the venue name and the start time instead of the generic text";
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
}
