//! What both backends' cancel writes on the thread, so the notice reads the
//! same on either.

use maidan_types::ThreadId;

/// The body of the message a cancel posts on its thread.
pub(crate) fn notice_body(reason: &str) -> String {
    format!("Cancelled: {reason}")
}

/// Its metadata: `notice` names it the way the other notices on a thread are
/// named, so a reader can tell it from what people wrote.
pub(crate) fn notice_metadata(reason: &str) -> serde_json::Value {
    serde_json::json!({ "notice": "cancelled", "reason": reason })
}

/// The last comment Maidan leaves on a cancelled task's pull request as it
/// closes it. The branch is named as kept so nobody reads the close as the
/// work being thrown away.
pub(crate) fn cancelled_pull_comment(reason: &str) -> String {
    withdrawn_pull_comment(&format!("was cancelled: {reason}"))
}

/// The same for a task closed as abandoned, duplicate or superseded.
pub(crate) fn closed_pull_comment(reason: &str) -> String {
    withdrawn_pull_comment(&format!("was closed as {reason}"))
}

fn withdrawn_pull_comment(how: &str) -> String {
    format!(
        "Maidan closed this pull request because its task {how}. It is out of the merge \
         queue; its branch is left as it is."
    )
}

/// The notice a pull request handoff posts on the superseded thread: the
/// pull request was not withdrawn, so whoever reads the closed thread learns
/// where the work went rather than looking for a close on GitHub.
pub(crate) fn handed_away_body(repo: &str, pull_number: i64, successor: ThreadId) -> String {
    format!(
        "Pull request {repo}#{pull_number} moved to {}, the task that supersedes this one. \
         It stays open, in the merge queue if it was queued; that task's rounds deliver to it now.",
        successor.0
    )
}

/// The notice on the successor, which now answers for the pull request: its
/// next round reaches it, and ending this task without landing withdraws it.
pub(crate) fn handed_to_body(repo: &str, pull_number: i64, superseded: ThreadId) -> String {
    format!(
        "Pull request {repo}#{pull_number} moved here from {}, which this task supersedes. \
         This task's rounds deliver to it now, and a cancel or close without landing withdraws it.",
        superseded.0
    )
}

/// Both notices' metadata, so a reader can find the move without parsing the
/// body.
pub(crate) fn handoff_metadata(
    repo: &str,
    pull_number: i64,
    from: ThreadId,
    to: ThreadId,
) -> serde_json::Value {
    serde_json::json!({
        "notice": "pull_handed_off",
        "repo": repo,
        "pull_number": pull_number,
        "from_thread_id": from.0,
        "to_thread_id": to.0,
    })
}
