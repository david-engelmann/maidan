//! A2A task, context and push notification config persistence types.

use chrono::{DateTime, Utc};
use maidan_types::{ApprovalGateId, MemberId, ThreadId, WorkspaceId};

/// One persisted A2A task. `task_json` is the protocol `Task` without any
/// message words: history and status messages are rendered from the sealed
/// message log on read, so shredding a message leaves no copy here.
#[derive(Debug, Clone, PartialEq)]
pub struct A2aTaskRow {
    pub id: String,
    pub workspace_id: WorkspaceId,
    /// The thread whose readers may see the task; `None` for a task on no
    /// thread, which the whole workspace may see.
    pub thread_id: Option<ThreadId>,
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
    /// The thread the task's context names; access to the task follows it.
    pub thread_id: Option<ThreadId>,
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
    /// Keep only the tasks this member may read: those on a thread it can
    /// read, and those on no thread. `None` keeps every task, for a caller
    /// that bypasses auth.
    pub readable_by: Option<MemberId>,
}

/// A page of a workspace's pending approval gates in `ListTasks` order:
/// newest first, ties broken by id descending. Gate timestamps are stored at
/// millisecond precision, the precision task positions and page tokens carry,
/// so the store's order is the listing's order.
#[derive(Debug, Clone, Default)]
pub struct PendingGateQuery {
    /// Keep only the gates attached to this thread.
    pub thread_id: Option<ThreadId>,
    /// Keep gates opened at or after this instant.
    pub created_since: Option<DateTime<Utc>>,
    /// The keyset cursor. `(at, Some(id))` keeps the gates that sort after
    /// the gate at `(at, id)`; `(at, None)` keeps those opened before `at`.
    pub before: Option<(DateTime<Utc>, Option<ApprovalGateId>)>,
    pub limit: i64,
    /// Keep only the gates this member may read, as
    /// [`A2aTaskQuery::readable_by`] does for tasks.
    pub readable_by: Option<MemberId>,
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
