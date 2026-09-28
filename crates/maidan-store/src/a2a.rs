//! A2A task, context and push notification config persistence types.

use chrono::{DateTime, Utc};
use maidan_types::WorkspaceId;

/// One persisted A2A task. `task_json` is the protocol `Task` without any
/// message words: history and status messages are rendered from the sealed
/// message log on read, so shredding a message leaves no copy here.
#[derive(Debug, Clone, PartialEq)]
pub struct A2aTaskRow {
    pub id: String,
    pub workspace_id: WorkspaceId,
    /// The task's status timestamp; list order and cursors key on it.
    pub updated_at: DateTime<Utc>,
    pub task_json: serde_json::Value,
}

/// A task to write: `status_at` becomes the row's `updated_at`.
#[derive(Debug, Clone)]
pub struct A2aTaskWrite<'a> {
    pub workspace_id: WorkspaceId,
    pub task_id: &'a str,
    pub context_id: Option<&'a str>,
    pub state: &'a str,
    pub status_at: DateTime<Utc>,
    pub task_json: serde_json::Value,
}

/// A page of a workspace's tasks, newest status first, ties broken by id
/// descending. `before` is the keyset cursor: the `(updated_at, id)` of the
/// last row of the previous page.
#[derive(Debug, Clone, Default)]
pub struct A2aTaskQuery<'a> {
    pub context_id: Option<&'a str>,
    pub state: Option<&'a str>,
    /// Keep tasks whose status changed at or after this instant.
    pub updated_since: Option<DateTime<Utc>>,
    pub before: Option<(DateTime<Utc>, &'a str)>,
    pub limit: i64,
}

/// A stored push notification config. Secrets are ciphertext sealed by the
/// server's at-rest key; the store never sees them in the clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2aPushConfigRow {
    pub task_id: String,
    pub config_id: String,
    pub url: String,
    pub token_ciphertext: Option<String>,
    pub auth_scheme: Option<String>,
    pub auth_credentials_ciphertext: Option<String>,
}
