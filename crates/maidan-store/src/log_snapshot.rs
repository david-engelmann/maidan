//! Snapshot assemble + since-LSN catch-up.
//!
//! Shared by REST and MCP so both surfaces hash the same graph and walk the
//! same chain. Complements the hash chain: the retained suffix is still
//! `verify_event_chain`; this covers a pruned prefix with a hashed domain-graph
//! checkpoint plus catch-up pages.

use crate::workspace_export::build_workspace_export;
use crate::{Store, StoreError};
use maidan_types::{CatchUpPage, LogSnapshot, SnapshotGraph, WorkspaceId};

/// Same clamp as REST `GET /workspaces/:id/events`.
pub const CATCH_UP_LIMIT: i64 = 500;

/// Current domain graph + retained-floor/head chain links.
///
/// # The head is read *first*, and that ordering is the correctness property
///
/// A consumer uses this as "here is the world, now tail from `head`", so the
/// pair has to leave no gap. `build_workspace_export` is many queries rather
/// than one transaction, so an event committing while it runs can land after
/// the sub-query that would have shown it — and if the head were read at the
/// end, that event would also be *below* the point the consumer tails from. In
/// neither the snapshot nor the catch-up stream: a silent gap, which is not
/// something an at-least-once consumer can recover from, because nothing tells
/// it to look.
///
/// Reading the head first inverts the error. The graph may now contain events
/// *newer* than the head, and catch-up re-delivers them — overlap, which every
/// consumer of this pair already handles, since at-least-once is the contract
/// the catch-up stream advertises.
///
/// The floor moves the same way for the same reason: read early it can only be
/// staler than reality, so a cursor that has actually been pruned away still
/// fails loudly as `CursorTooOld` instead of being silently accepted.
///
/// A repeatable-read snapshot spanning the whole export would be stricter
/// still, but the `Store` trait is not transactional across methods and the two
/// backends differ; this ordering is correct on both.
pub async fn build_log_snapshot(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    include_graph: bool,
) -> Result<LogSnapshot, StoreError> {
    let floor = store.workspace_event_floor(workspace_id).await?;
    let head = store.workspace_event_head(workspace_id).await?;
    let export = build_workspace_export(store, workspace_id).await?;
    let graph = SnapshotGraph::from(export);
    LogSnapshot::new(workspace_id, floor, head, graph, include_graph)
        .map_err(|e| StoreError::InvalidInput(e.to_string()))
}

/// Events with `id > after_lsn` in this workspace, chain-checked from the
/// predecessor at-or-before `after_lsn`. A pruned-gap cursor is
/// [`StoreError::CursorTooOld`] — refetch the snapshot, never clamp.
pub async fn catch_up_since(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    after_lsn: i64,
    limit: i64,
) -> Result<CatchUpPage, StoreError> {
    if after_lsn < 0 {
        return Err(StoreError::InvalidInput(
            "after_lsn must be non-negative".into(),
        ));
    }
    store.ensure_cursor_fresh(workspace_id, after_lsn).await?;
    let limit = limit.clamp(1, CATCH_UP_LIMIT);
    let events = store
        .list_events_after(workspace_id, after_lsn, limit)
        .await?;
    let truncated = (events.len() as i64) == limit;
    let head = store.workspace_event_head(workspace_id).await?;
    let head_lsn = head.as_ref().map(|h| h.lsn).unwrap_or(0);
    let room_lsn = store.max_event_id().await?;
    let previous = if after_lsn > 0 {
        store
            .workspace_event_at_or_before(workspace_id, after_lsn)
            .await?
    } else {
        None
    };
    Ok(CatchUpPage::new(
        workspace_id,
        after_lsn,
        head_lsn,
        room_lsn,
        events,
        previous.as_ref(),
        truncated,
    ))
}
