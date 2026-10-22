//! `wait_for_event`: block until the first event of any of the named kinds
//! the caller may read, in its workspace or one of its threads, so a session
//! waits on what it cares about instead of polling. The single-kind
//! `wait_for_*` tools are this with the kind fixed; they share
//! [`super::thread::next_visible_event`].

use std::collections::HashSet;

use maidan_auth::AuthContext;
use maidan_types::{EventKind, ThreadId, WorkspaceId};
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use super::thread::{next_visible_event, EventWait};
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitForEventArgs {
    /// Omitted, the caller's own workspace; never every workspace.
    #[serde(default)]
    workspace_id: Option<uuid::Uuid>,
    kinds: Vec<String>,
    /// Access is checked before dispatch.
    #[serde(default)]
    thread_id: Option<uuid::Uuid>,
    #[serde(default)]
    timeout_ms: Option<i64>,
    #[serde(default)]
    after_log_id: Option<i64>,
}

/// The kinds as [`EventKind`]s. An unknown name is refused by name rather
/// than dropped: a wait on a kind that never fires would only time out.
fn parse_kinds(names: &[String]) -> Result<HashSet<EventKind>, McpError> {
    if names.is_empty() {
        return Err(McpError::InvalidParams(
            "kinds: name at least one event kind".into(),
        ));
    }
    names
        .iter()
        .map(|name| {
            serde_json::from_value::<EventKind>(Value::String(name.clone()))
                .map_err(|_| McpError::InvalidParams(format!("kinds: unknown event kind `{name}`")))
        })
        .collect()
}

pub(super) async fn wait_for_event(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForEventArgs = super::parse_args(args)?;
    let kinds = parse_kinds(&a.kinds)?;
    if a.timeout_ms == Some(0) && a.after_log_id.is_none() {
        return Err(McpError::InvalidParams(
            "timeout_ms: 0 only replays, so it needs after_log_id".into(),
        ));
    }
    let workspace_id = a.workspace_id.map_or(auth.workspace_id, WorkspaceId);
    auth.ensure_workspace(workspace_id)?;
    let found = next_visible_event(
        server,
        auth,
        EventWait {
            tool: "wait_for_event",
            workspace_id,
            kinds,
            channel_id: None,
            thread_id: a.thread_id.map(ThreadId),
            timeout_ms: a.timeout_ms,
            // Zero only replays: a caller catching up asks for no wait.
            min_ms: 0,
            since_log_id: a.after_log_id,
        },
    )
    .await?;
    Ok(match found {
        Some((log_id, event)) => content_json(&json!({ "log_id": log_id, "event": event })),
        None => content_json(&Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_kinds;

    #[test]
    fn an_unknown_or_missing_kind_is_refused_by_name() {
        let err = parse_kinds(&["thread_landed".into(), "thread_explode".into()]).unwrap_err();
        assert!(err.to_string().contains("`thread_explode`"), "{err}");
        assert!(parse_kinds(&[]).is_err());
        assert_eq!(parse_kinds(&["thread_landed".into()]).unwrap().len(), 1);
    }
}
