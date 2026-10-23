//! The one guard every audited write passes: the audit row's scope must be
//! the workspace of what the write changed.
//!
//! `AuditScope` makes each writer choose a scope; this checks the choice. An
//! audited write names what it changed (an [`AuditOwner`]); its backend
//! resolves that to a workspace on the write's own transaction, and
//! [`check`] refuses the write when the event's scope differs, so the change
//! and its mis-scoped row roll back together. Both backends write audit rows
//! only through `audit::append_scoped`, which calls this; a contract test
//! fails when an audited method reaches the audit table any other way.

use maidan_types::{
    AuditScope, ChannelId, MemberId, MessageId, NewAuditEvent, ThreadId, WorkspaceId,
};

use crate::error::StoreError;

/// What an audited write changed, which decides whose its audit row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditOwner {
    /// A workspace, or a row that names its workspace.
    Workspace(WorkspaceId),
    /// An instance-wide change: the row is in no workspace.
    Instance,
    /// A member: the row is in the member's workspace.
    Member(MemberId),
    /// A channel: the row is in the channel's workspace.
    Channel(ChannelId),
    /// A thread: the row is in its channel's workspace.
    Thread(ThreadId),
    /// A message: the row is in its thread's channel's workspace.
    Message(MessageId),
}

impl From<Option<WorkspaceId>> for AuditOwner {
    fn from(ws: Option<WorkspaceId>) -> Self {
        ws.map_or(AuditOwner::Instance, AuditOwner::Workspace)
    }
}

impl From<WorkspaceId> for AuditOwner {
    fn from(ws: WorkspaceId) -> Self {
        AuditOwner::Workspace(ws)
    }
}

impl From<AuditScope> for AuditOwner {
    fn from(scope: AuditScope) -> Self {
        match scope {
            AuditScope::Workspace(ws) => AuditOwner::Workspace(ws),
            AuditScope::Instance => AuditOwner::Instance,
        }
    }
}

/// The lookup, per owner kind, that names the owner's workspace. `$1` binds
/// on both backends.
pub(crate) const MEMBER_WORKSPACE: &str = "SELECT workspace_id FROM maidan_members WHERE id = $1";
pub(crate) const CHANNEL_WORKSPACE: &str = "SELECT workspace_id FROM maidan_channels WHERE id = $1";
pub(crate) const THREAD_WORKSPACE: &str = "SELECT c.workspace_id FROM maidan_threads t \
     JOIN maidan_channels c ON c.id = t.channel_id WHERE t.id = $1";

pub(crate) const MESSAGE_WORKSPACE: &str = "SELECT c.workspace_id FROM maidan_messages m \
     JOIN maidan_threads t ON t.id = m.thread_id \
     JOIN maidan_channels c ON c.id = t.channel_id WHERE m.id = $1";

/// Refuse `event` unless its scope is `expected`, the workspace of what the
/// write changed (`None` when that is instance-wide or no longer exists).
pub(crate) fn check(
    expected: Option<WorkspaceId>,
    event: &NewAuditEvent,
) -> Result<(), StoreError> {
    let stated = event.scope.workspace_id();
    if stated == expected {
        return Ok(());
    }
    crate::attribution::count_audit_write_failure();
    let name =
        |ws: Option<WorkspaceId>| ws.map_or_else(|| "instance".to_string(), |w| w.0.to_string());
    Err(StoreError::InvalidInput(format!(
        "audit scope {} does not match the workspace the `{}` write changed ({})",
        name(stated),
        event.action,
        name(expected)
    )))
}
