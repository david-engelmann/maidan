//! What both backends' cancel writes on the thread, so the notice reads the
//! same on either.

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
