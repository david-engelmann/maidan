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
