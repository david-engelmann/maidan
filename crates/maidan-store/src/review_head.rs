//! A verdict names the change's head (S4b): the SQL both backends share to
//! read the head a verdict is given against and to dismiss approvals a push
//! left behind.

/// The change a review on `thread` is about: its parent when `thread` is a
/// review child, else `thread` itself.
fn reviewed_change(thread: &str) -> String {
    format!(
        "COALESCE((SELECT t.parent_thread_id FROM maidan_threads t
                   JOIN maidan_thread_attributes a ON a.thread_id = t.id
                   WHERE t.id = {thread} AND a.kind = 'review'), {thread})"
    )
}

/// The full head SHA of the pull tracked on the change reviewed on `thread`,
/// last read first; NULL when no pull is tracked.
pub(crate) fn current_head(thread: &str) -> String {
    let change = reviewed_change(thread);
    format!(
        "(SELECT ci.head_sha FROM maidan_change_pulls cp
          JOIN maidan_pull_ci ci ON ci.workspace_id = cp.workspace_id
           AND ci.repo = cp.repo AND ci.pull_number = cp.pull_number
          WHERE cp.thread_id = {change}
          ORDER BY ci.updated_at DESC LIMIT 1)"
    )
}

/// Postgres: share-lock the pull rows tracked on the change reviewed on
/// `thread`, before reading its head. A CI read holds them `FOR UPDATE`
/// while it moves the head and dismisses stale approvals, so a verdict
/// either sees the new head or lands before the dismissal runs (#1574).
pub(crate) fn lock_change_pulls(thread: &str) -> String {
    let change = reviewed_change(thread);
    format!("SELECT 1 FROM maidan_change_pulls WHERE thread_id = {change} FOR SHARE")
}

/// Dismiss, at `now`, every standing approval on change `change` or its
/// review children that named a head other than `head`.
pub(crate) fn dismiss_stale(change: &str, head: &str, now: &str) -> String {
    format!(
        "UPDATE maidan_thread_reviews SET dismissed_at = {now}
         WHERE decision = 'approve' AND dismissed_at IS NULL
           AND head_sha IS NOT NULL AND head_sha <> {head}
           AND (thread_id = {change} OR thread_id IN (
               SELECT t.id FROM maidan_threads t
               JOIN maidan_thread_attributes a ON a.thread_id = t.id
               WHERE t.parent_thread_id = {change} AND a.kind = 'review'))"
    )
}
