//! Outcome checks: whether a merged agent change held.
//!
//! Rounds, time to ready and cost say how fast agent work lands, not whether
//! it held. After a merge, the change sweep reads three things from GitHub
//! for [`maidan_types::OUTCOME_WINDOW_DAYS`]: whether a revert of it landed
//! on the base, which later merged pull requests say they fix it (or name its
//! Linear issue), and, once, how much a person added (commits the change
//! flow did not deliver, reviews and review comments by accounts that are not
//! bots). GitHub cannot time a person's effort, so these are proxies.
//!
//! The reads share the token's hourly budget with the change flow, so they
//! come in one batch per workspace an hour, each pull request at most once a
//! day (`Store::list_change_outcomes_due`), and a batch reads the base
//! branch's commits and its closed pull requests once per repository and
//! base, matching every due pull request against the same two lists rather
//! than searching for each.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use maidan_types::{MergedOutcome, OutcomeDue, WorkspaceId};

use crate::github::{GithubCommitBrief, GithubGit, GithubMergedPull, GithubReviewActivity};
use crate::state::AppState;

/// Pull requests checked per workspace per batch.
pub const OUTCOME_BATCH_LIMIT: i64 = 25;

/// How far before a reference the words that make it a fix are looked for.
const FIX_WORDS_WINDOW: usize = 40;

/// Check the merged pull requests in `workspace_id` that are due. Best
/// effort: a failed read leaves that part of the outcome as it was, and the
/// pull request is checked again tomorrow.
pub async fn check_outcomes(state: &AppState, workspace_id: WorkspaceId) {
    let Some(git) = state.github_sender.as_ref().and_then(|s| s.git()) else {
        return;
    };
    let now = Utc::now();
    let due = match state
        .store
        .list_change_outcomes_due(workspace_id, now, OUTCOME_BATCH_LIMIT)
        .await
    {
        Ok(due) => due,
        Err(err) => {
            tracing::warn!(error = %err, "outcome check: due pull requests not read");
            return;
        }
    };
    let mut lists = BaseLists::for_due(&due);
    for item in due {
        let outcome = check_one(git, &mut lists, &item).await;
        if let Err(err) = state
            .store
            .record_change_outcome(&item.repo, item.pull_number, &outcome, now)
            .await
        {
            tracing::warn!(repo = item.repo, pull_number = item.pull_number, error = %err, "outcome check: not recorded");
        }
    }
}

/// The base branch's recent commits and merged pull requests, read once per
/// repository and base in a batch. `None` is a read that failed.
type Lists = (
    Option<Vec<GithubCommitBrief>>,
    Option<Vec<GithubMergedPull>>,
);

struct BaseLists {
    /// The oldest due merge into each repository and base: one commit read
    /// from there serves every due pull request into it.
    since: HashMap<(String, String), DateTime<Utc>>,
    read: HashMap<(String, String), Lists>,
}

impl BaseLists {
    fn for_due(due: &[OutcomeDue]) -> Self {
        let mut since: HashMap<(String, String), DateTime<Utc>> = HashMap::new();
        for item in due {
            if let Some(base) = &item.base_branch {
                let oldest = since
                    .entry((item.repo.clone(), base.clone()))
                    .or_insert(item.merged_at);
                *oldest = (*oldest).min(item.merged_at);
            }
        }
        Self {
            since,
            read: HashMap::new(),
        }
    }

    async fn get(&mut self, git: &dyn GithubGit, repo: &str, base: &str) -> &Lists {
        let key = (repo.to_string(), base.to_string());
        if !self.read.contains_key(&key) {
            let since = self.since.get(&key).copied().unwrap_or_else(Utc::now);
            let commits = git.branch_commits_since(repo, base, since).await;
            let pulls = git.merged_pulls_into(repo, base).await;
            if let Err(err) = commits.as_ref() {
                tracing::debug!(repo, base, error = %err, "outcome check: base commits not read");
            }
            if let Err(err) = pulls.as_ref() {
                tracing::debug!(repo, base, error = %err, "outcome check: merged pull requests not read");
            }
            self.read.insert(key.clone(), (commits.ok(), pulls.ok()));
        }
        &self.read[&key]
    }
}

async fn check_one(git: &dyn GithubGit, lists: &mut BaseLists, item: &OutcomeDue) -> MergedOutcome {
    let mut outcome = item.outcome.clone();
    if outcome.human_reviews.is_none() {
        read_human_effort(git, item, &mut outcome).await;
    }
    let Some(base) = item.base_branch.as_deref() else {
        return outcome;
    };
    let (commits, pulls) = lists.get(git, &item.repo, base).await;
    if outcome.reverted_at.is_none() {
        let by_pull = pulls
            .as_deref()
            .and_then(|pulls| reverting_pull(pulls, &item.repo, item.pull_number, item.merged_at))
            .map(|pull| (pull.merged_at, format!("#{}", pull.number)));
        let by_commit = || {
            commits
                .as_deref()
                .and_then(|commits| {
                    reverting_commit(
                        commits,
                        &item.repo,
                        item.pull_number,
                        outcome.merge_commit_sha.as_deref(),
                    )
                })
                // The list does not say when the commit landed; this check is
                // within a day of it.
                .map(|commit| (Utc::now(), commit.sha.clone()))
        };
        if let Some((at, reference)) = by_pull.or_else(by_commit) {
            outcome.reverted_at = Some(at);
            outcome.revert_ref = Some(reference);
        }
    }
    if let Some(pulls) = pulls.as_deref() {
        for fix in followup_fixes(
            pulls,
            &item.repo,
            item.pull_number,
            item.merged_at,
            item.linear_issue.as_deref(),
        ) {
            if !outcome.followup_fixes.contains(&fix) {
                outcome.followup_fixes.push(fix);
            }
        }
        outcome.followup_fixes.sort_unstable();
    }
    outcome
}

/// Read, once, what a person added: the merge commit (a revert names it),
/// commits the change flow did not deliver, and reviews by people. Left
/// unread on any failure, so tomorrow's check tries again.
async fn read_human_effort(git: &dyn GithubGit, item: &OutcomeDue, outcome: &mut MergedOutcome) {
    let read = async {
        let brief = git.pull_brief(&item.repo, item.pull_number).await?;
        let commits = git.pull_commit_shas(&item.repo, item.pull_number).await?;
        let activity = git
            .pull_review_activity(&item.repo, item.pull_number)
            .await?;
        Ok::<_, crate::github::GithubError>((brief.merge_commit_sha, commits, activity))
    };
    match read.await {
        Ok((merge_commit_sha, commits, activity)) => {
            outcome.merge_commit_sha = merge_commit_sha;
            outcome.human_commits = Some(human_commits(&commits, &item.round_commits));
            let (reviews, first) = human_reviews(&activity);
            outcome.human_reviews = Some(reviews);
            outcome.first_human_review_at = first;
        }
        Err(err) => {
            tracing::debug!(repo = item.repo, pull_number = item.pull_number, error = %err, "outcome check: human effort not read");
        }
    }
}

/// Commits on the pull request that are not among the rounds the change flow
/// delivered.
pub fn human_commits(on_pull: &[String], delivered: &[String]) -> i64 {
    on_pull
        .iter()
        .filter(|sha| !delivered.iter().any(|d| d.eq_ignore_ascii_case(sha)))
        .count() as i64
}

/// Reviews and review comments by accounts that are not bots, and when the
/// first came.
pub fn human_reviews(activity: &[GithubReviewActivity]) -> (i64, Option<DateTime<Utc>>) {
    let human: Vec<&GithubReviewActivity> = activity.iter().filter(|a| !a.bot).collect();
    (human.len() as i64, human.iter().filter_map(|a| a.at).min())
}

/// A merged pull request that reverts `pull_number`: titled `Revert "…"`,
/// merged after it, and naming it (`Reverts owner/repo#N` in the body, as
/// GitHub's revert button writes, or `(#N)` in the quoted title).
pub fn reverting_pull<'a>(
    pulls: &'a [GithubMergedPull],
    repo: &str,
    pull_number: i64,
    merged_at: DateTime<Utc>,
) -> Option<&'a GithubMergedPull> {
    pulls.iter().find(|pull| {
        pull.number != pull_number
            && pull.merged_at > merged_at
            && is_revert_title(&pull.title)
            && (names_number(
                &pull.body.to_ascii_lowercase(),
                &format!("reverts {}#", repo.to_ascii_lowercase()),
                pull_number,
            ) || reverted_title(&pull.title)
                .is_some_and(|title| names_number(title, "(#", pull_number)))
    })
}

/// A commit on the base that reverts the merge: git's default message names
/// the merge commit (`This reverts commit <sha>.`), and a squash-merged
/// revert keeps `Revert "… (#N)"` or `Reverts owner/repo#N`.
pub fn reverting_commit<'a>(
    commits: &'a [GithubCommitBrief],
    repo: &str,
    pull_number: i64,
    merge_commit_sha: Option<&str>,
) -> Option<&'a GithubCommitBrief> {
    commits.iter().find(|commit| {
        let message = commit.message.to_ascii_lowercase();
        let subject = commit.message.lines().next().unwrap_or_default();
        merge_commit_sha.is_some_and(|sha| {
            message.contains(&format!("this reverts commit {}", sha.to_ascii_lowercase()))
        }) || reverted_title(subject).is_some_and(|title| names_number(title, "(#", pull_number))
            || names_number(
                &message,
                &format!("reverts {}#", repo.to_ascii_lowercase()),
                pull_number,
            )
    })
}

/// Later merged pull requests (within the window, not reverts, not the pull
/// request itself) whose title or body says it fixes or follows up
/// `pull_number` (`fixes #N`, `follow-up to #N`, the pull request's URL), or
/// names the same Linear issue.
pub fn followup_fixes(
    pulls: &[GithubMergedPull],
    repo: &str,
    pull_number: i64,
    merged_at: DateTime<Utc>,
    linear_issue: Option<&str>,
) -> Vec<i64> {
    let until = merged_at + chrono::Duration::days(maidan_types::OUTCOME_WINDOW_DAYS);
    let mut fixes: Vec<i64> = pulls
        .iter()
        .filter(|pull| {
            pull.number != pull_number
                && pull.merged_at > merged_at
                && pull.merged_at <= until
                && !is_revert_title(&pull.title)
        })
        .filter(|pull| {
            let text = format!("{}\n{}", pull.title, pull.body);
            says_it_fixes(&text, repo, pull_number)
                || linear_issue.is_some_and(|key| names_key(&text, key))
        })
        .map(|pull| pull.number)
        .collect();
    fixes.sort_unstable();
    fixes.dedup();
    fixes
}

fn is_revert_title(title: &str) -> bool {
    reverted_title(title).is_some()
}

/// The title quoted in `Revert "<title>"`, without the revert's own `(#M)`
/// after the closing quote.
fn reverted_title(subject: &str) -> Option<&str> {
    let rest = subject.trim_start().strip_prefix("Revert \"")?;
    rest.rfind('"').map(|end| &rest[..end])
}

/// Whether `text` has `prefix` followed by exactly `number` (no further
/// digit), with no letter or digit right before it.
fn names_number(text: &str, prefix: &str, number: i64) -> bool {
    !positions_of_number(text, prefix, number).is_empty()
}

/// Where `prefix` followed by exactly `number` starts in `text`.
fn positions_of_number(text: &str, prefix: &str, number: i64) -> Vec<usize> {
    let needle = format!("{prefix}{number}");
    let bytes = text.as_bytes();
    text.match_indices(needle.as_str())
        .map(|(at, _)| at)
        .filter(|&at| {
            let before = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
            let after = bytes
                .get(at + needle.len())
                .is_none_or(|b| !b.is_ascii_digit());
            before && after
        })
        .collect()
}

/// A reference to `pull_number` (`#N`, `owner/repo#N`, or its URL) with
/// "fix" or "follow" in the words just before it.
fn says_it_fixes(text: &str, repo: &str, pull_number: i64) -> bool {
    let lower = text.to_ascii_lowercase();
    let repo = repo.to_ascii_lowercase();
    let pull_url = format!("{repo}/pull/");
    let mut references = positions_of_number(&lower, "#", pull_number);
    references.extend(positions_of_number(
        &lower,
        &format!("{repo}#"),
        pull_number,
    ));
    references.extend(positions_of_number(&lower, &pull_url, pull_number));
    references.into_iter().any(|at| {
        let mut start = at.saturating_sub(FIX_WORDS_WINDOW);
        while !lower.is_char_boundary(start) {
            start += 1;
        }
        let before = &lower[start..at];
        before.contains("fix") || before.contains("follow")
    })
}

/// Whether `text` names the issue `key` standing alone.
fn names_key(text: &str, key: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices(key).any(|(at, _)| {
        let end = at + key.len();
        (at == 0 || !bytes[at - 1].is_ascii_alphanumeric())
            && bytes.get(end).is_none_or(|b| !b.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "example/app";

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + hours * 3600, 0).unwrap()
    }

    fn merged(number: i64, title: &str, body: &str, hours: i64) -> GithubMergedPull {
        GithubMergedPull {
            number,
            title: title.into(),
            body: body.into(),
            merged_at: at(hours),
        }
    }

    fn commit(sha: &str, message: &str) -> GithubCommitBrief {
        GithubCommitBrief {
            sha: sha.into(),
            message: message.into(),
        }
    }

    #[test]
    fn a_revert_pull_names_the_pull_it_reverts() {
        let pulls = [
            merged(
                20,
                "Revert \"Greet the world (#7)\"",
                "Reverts example/app#7",
                5,
            ),
            merged(
                21,
                "Revert \"Something else (#70)\"",
                "Reverts example/app#70",
                6,
            ),
        ];
        assert_eq!(
            reverting_pull(&pulls, REPO, 7, at(0)).map(|p| p.number),
            Some(20)
        );
        assert_eq!(
            reverting_pull(&pulls, REPO, 70, at(0)).map(|p| p.number),
            Some(21)
        );
        assert!(
            reverting_pull(&pulls, REPO, 7, at(10)).is_none(),
            "merged before it"
        );
        assert!(reverting_pull(&pulls, REPO, 8, at(0)).is_none());
    }

    #[test]
    fn a_revert_commit_names_the_merge_commit_or_the_pull() {
        let merge = "a".repeat(40);
        let commits = [
            commit("c1", "Fix typo (#9)"),
            commit(
                "c2",
                &format!("Revert \"Greet\"\n\nThis reverts commit {merge}."),
            ),
        ];
        assert_eq!(
            reverting_commit(&commits, REPO, 7, Some(&merge)).map(|c| c.sha.as_str()),
            Some("c2")
        );
        assert!(reverting_commit(&commits, REPO, 7, None).is_none());
        let squashed = [commit("c3", "Revert \"Greet the world (#7)\" (#20)")];
        assert!(reverting_commit(&squashed, REPO, 7, None).is_some());
        assert!(
            reverting_commit(&squashed, REPO, 20, None).is_none(),
            "the revert's own number is outside the quote"
        );
        let bodied = [commit("c4", "Undo greeting (#21)\n\nReverts Example/App#7")];
        assert!(reverting_commit(&bodied, REPO, 7, None).is_some());
        assert!(reverting_commit(&bodied, REPO, 70, None).is_none());
    }

    #[test]
    fn a_followup_says_it_fixes_or_names_the_same_issue() {
        let pulls = [
            merged(30, "Fix the greeting edge case", "Fixes #7", 2),
            merged(
                31,
                "Tidy",
                "Follow-up to https://github.com/example/app/pull/7.",
                3,
            ),
            merged(32, "Unrelated", "Part of #7", 4),
            merged(33, "Rename", "fixes #70 and #77", 5),
            merged(34, "Handle the empty name", "ENG-1234: the empty case", 6),
            merged(
                35,
                "Revert \"Greet the world (#7)\"",
                "Fixes #7 by reverting it",
                7,
            ),
            merged(36, "Old fix", "Fixes #7", -1),
            merged(37, "Late fix", "Fixes #7", 24 * 31),
            merged(38, "Same key, longer", "XENG-1234 and ENG-12345", 8),
        ];
        assert_eq!(
            followup_fixes(&pulls, REPO, 7, at(0), Some("ENG-1234")),
            vec![30, 31, 34]
        );
        assert_eq!(followup_fixes(&pulls, REPO, 7, at(0), None), vec![30, 31]);
    }

    #[test]
    fn human_effort_skips_delivered_commits_and_bots() {
        let delivered = vec!["AAA".to_string(), "bbb".to_string()];
        let on_pull = vec!["aaa".to_string(), "ccc".to_string()];
        assert_eq!(human_commits(&on_pull, &delivered), 1);
        let activity = [
            GithubReviewActivity {
                bot: true,
                at: Some(at(1)),
            },
            GithubReviewActivity {
                bot: false,
                at: Some(at(3)),
            },
            GithubReviewActivity {
                bot: false,
                at: Some(at(2)),
            },
        ];
        assert_eq!(human_reviews(&activity), (2, Some(at(2))));
        assert_eq!(human_reviews(&[]), (0, None));
    }
}
