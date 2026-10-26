//! D-A for destroying, replacing and preserving workspace data: purge, erase,
//! import, message purge and legal hold. Each writes its audit row in the
//! change's transaction, so a failed write aborts the change. The destructive
//! ones refuse a held workspace inside that same transaction.

use maidan_types::{
    ContentKeyring, LegalHold, LegalHoldId, MemberId, MessageId, NewAuditEvent, PreservedMessage,
    WorkspaceEraseResult, WorkspaceId, WorkspaceImport, WorkspacePurgeResult,
};
use sqlx::PgPool;

use super::{audit, erase_workspace, import, legal_hold, messages, purge_workspace};
use crate::{error::StoreError, AuditFor};

pub async fn purge_workspace(
    pool: &PgPool,
    keys: &ContentKeyring,
    workspace_id: WorkspaceId,
    audit_for: AuditFor<WorkspacePurgeResult>,
) -> Result<WorkspacePurgeResult, StoreError> {
    let mut tx = pool.begin().await?;
    let result = purge_workspace::purge_on(&mut tx, keys, workspace_id).await?;
    audit::append_scoped(&mut tx, workspace_id, audit_for(&result)).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn erase_workspace(
    pool: &PgPool,
    keys: &ContentKeyring,
    workspace_id: WorkspaceId,
    audit_for: AuditFor<WorkspaceEraseResult>,
) -> Result<WorkspaceEraseResult, StoreError> {
    let mut tx = pool.begin().await?;
    let result = erase_workspace::erase_on(&mut tx, keys, workspace_id).await?;
    audit::append_scoped(&mut tx, workspace_id, audit_for(&result)).await?;
    tx.commit().await?;
    Ok(result)
}

/// With `replace_existing`, the workspace the import names is erased first,
/// in the same transaction: a failed import leaves it as it was, not gone.
pub async fn import_workspace(
    pool: &PgPool,
    keys: &ContentKeyring,
    bundle: &WorkspaceImport,
    replace_existing: bool,
    origin: maidan_types::WorkspaceOrigin,
    event: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    // A replaced workspace keeps its own origin, read before the erase: a
    // forced restore of an open workspace is still open.
    // So do its metered usage and allowance crossings, which the erase
    // would otherwise cascade away: a restore is not a new month.
    let (origin, kept_usage) = if replace_existing {
        let kept = import::origin_on(&mut tx, bundle.workspace.id).await?;
        let usage = import::keep_usage_on(&mut tx, bundle.workspace.id).await?;
        erase_workspace::erase_on(&mut tx, keys, bundle.workspace.id).await?;
        (kept.unwrap_or(origin), Some(usage))
    } else {
        (origin, None)
    };
    import::import_on(&mut tx, bundle, origin).await?;
    if let Some(usage) = kept_usage {
        import::restore_usage_on(&mut tx, bundle.workspace.id, usage).await?;
    }
    audit::append_scoped(&mut tx, bundle.workspace.id, event).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn purge_message(
    pool: &PgPool,
    id: MessageId,
    event: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    // Scope first: the message, and so its workspace, is gone after the purge.
    audit::append_scoped(&mut tx, crate::audit_scope::AuditOwner::Message(id), event).await?;
    messages::purge_on(&mut tx, id).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn place_legal_hold(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    reason: &str,
    placed_by: Option<MemberId>,
    audit_for: AuditFor<LegalHold>,
) -> Result<LegalHold, StoreError> {
    let mut tx = pool.begin().await?;
    let hold = legal_hold::place_on(&mut tx, workspace_id, reason, placed_by).await?;
    audit::append_scoped(&mut tx, workspace_id, audit_for(&hold)).await?;
    tx.commit().await?;
    Ok(hold)
}

/// Nothing is recorded when there was no hold to lift.
pub async fn lift_legal_hold(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    hold_id: LegalHoldId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let lifted = legal_hold::lift_on(&mut tx, workspace_id, hold_id).await?;
    if let Some(disposal) = &lifted {
        audit::append_scoped(&mut tx, workspace_id, disposal.recorded_in(event)).await?;
    }
    tx.commit().await?;
    Ok(lifted.is_some())
}

/// What the workspace's hold kept of withdrawn messages; the read is recorded
/// first (SQLite twin).
pub async fn read_preserved_messages(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    event: NewAuditEvent,
) -> Result<Vec<PreservedMessage>, StoreError> {
    let mut tx = pool.begin().await?;
    audit::append_scoped(&mut tx, workspace_id, event).await?;
    let preserved = legal_hold::preserved_on(&mut tx, workspace_id).await?;
    tx.commit().await?;
    Ok(preserved)
}
