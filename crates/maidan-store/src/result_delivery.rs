//! Operator replay for a result-delivery row (Cluster 379.5).
//!
//! Arming (`arm_result_delivery`) is the "is this a new result?" predicate and
//! only wins when `revision > armed_revision`. Replay of the *same* revision
//! would therefore always lose, so this path does not arm. It reopens the
//! existing row as `pending` and enqueues a **new** outbox row: the outbox
//! unique key is `(source_log_id, surface, selector)`, so reusing the original
//! event's log id is a no-op if that row still exists (even `dead`).
//!
//! **`deliver_to` selects; the workspace allowlist authorizes** — the same
//! check as the 379.3 trigger. A replay against a target the workspace has
//! not blessed stays skipped (a recorded normal outcome). An unroutable row
//! (unknown surface, unusable detail) cannot be enqueued; the caller maps
//! that to 400 / InvalidParams.

use chrono::Utc;
use maidan_types::{
    EgressKind, NewEgressOutbox, ResultDelivery, ResultDeliveryId, ThreadId, WorkspaceId,
};

use crate::error::StoreError;
use crate::store::Store;

/// How [`replay_result_delivery`] resolved one operator replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultDeliveryReplay {
    /// Allowlist passed; a new outbox row is pending.
    Enqueued(ResultDelivery),
    /// Still unblessed — status stays `skipped`, nothing enqueued.
    Skipped(ResultDelivery),
    /// The stored pair does not decode to an [`maidan_types::EgressTarget`].
    Unroutable(ResultDelivery),
}

/// Replay `(thread_id, id)` onto the egress outbox.
///
/// `body` is the snapshot the worker will send if it cannot rebuild from the
/// current thread result (the worker prefers a live rebuild so a replay after
/// a newer result ships current bytes). `None` means the id is not on this
/// thread.
pub async fn replay_result_delivery(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    id: ResultDeliveryId,
    body: String,
) -> Result<Option<ResultDeliveryReplay>, StoreError> {
    let Some(row) = store.get_result_delivery_by_id(thread_id, id).await? else {
        return Ok(None);
    };
    let Some(target) = row.target() else {
        return Ok(Some(ResultDeliveryReplay::Unroutable(row)));
    };
    let allowed = store
        .is_egress_target_allowed(workspace_id, target.surface(), &target.allowlist_selector())
        .await?;
    if !allowed {
        store
            .mark_result_delivery_skipped(row.id, "target not in the workspace egress allowlist")
            .await?;
        let skipped = store
            .get_result_delivery_by_id(thread_id, id)
            .await?
            .unwrap_or(row);
        return Ok(Some(ResultDeliveryReplay::Skipped(skipped)));
    }
    let Some(pending) = store.prepare_result_delivery_replay(thread_id, id).await? else {
        return Ok(None);
    };
    enqueue_replay(store, workspace_id, thread_id, &target, body).await?;
    Ok(Some(ResultDeliveryReplay::Enqueued(pending)))
}

async fn enqueue_replay(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    target: &maidan_types::EgressTarget,
    body: String,
) -> Result<(), StoreError> {
    // Synthetic log ids live in a range event-log ids (small, positive,
    // monotonic) never occupy, so a replay cannot collide with the original
    // enqueue's unique key.
    let mut source_log_id = synthetic_source_log_id();
    for _ in 0..8 {
        let queued = store
            .enqueue_egress(NewEgressOutbox {
                workspace_id,
                thread_id,
                source_log_id,
                target: target.clone(),
                body: body.clone(),
                kind: EgressKind::Result,
            })
            .await?;
        if queued.is_some() {
            return Ok(());
        }
        source_log_id = source_log_id.saturating_add(1);
    }
    Err(StoreError::Conflict(
        "could not enqueue a unique result-delivery replay".into(),
    ))
}

fn synthetic_source_log_id() -> i64 {
    // Event-log ids are small positive integers. Negated nanos are unique at
    // operator pace and cannot collide with a real `log_id`.
    match Utc::now().timestamp_nanos_opt() {
        Some(ns) if ns > 0 => -ns,
        Some(ns) => ns,
        None => -Utc::now().timestamp_millis().saturating_mul(1_000),
    }
}
