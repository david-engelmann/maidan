//! Cross-process fan-out of MCP resource updates.
//!
//! MCP `resources/subscribe` notifications (`notifications/resources/updated`)
//! must reach a subscriber regardless of which server replica handled the
//! mutation. The event log already crosses processes via
//! [`crate::PostgresBus`]; this is the sibling channel for *resource* updates,
//! which are derived from a mutation rather than carried by a domain
//! [`maidan_types::Event`].
//!
//! Contract: [`ResourceNotifier::publish`] broadcasts the **unfiltered** set of
//! [`ResourceUpdate`]s a mutation produced to every process. Each carries the
//! workspace the mutation happened in, because a URI alone does not say whose
//! change it was: artifacts are content-addressed and shared across
//! workspaces, so the same `maidan://artifacts/{sha}` names a resource in
//! several tenants. Each process receives the batch via its
//! [`ResourceNotifier::subscribe`] receiver and applies its **own** local
//! subscriptions and access checks before delivering to clients. There is a
//! single delivery path — even the originating process delivers via the
//! receiver loop, not directly — so no de-duplication is needed.
//!
//! Two implementations mirror [`crate::EventBus`]:
//!
//! - [`InMemoryResourceNotifier`] — in-process tokio broadcast (single-process /
//!   SQLite / tests).
//! - [`PostgresResourceNotifier`] — Postgres `LISTEN`/`NOTIFY` for multi-process
//!   fan-out. Delivery is at-most-once (as with the event bus); a dropped
//!   notification is reconciled by the client re-reading the resource.

use async_trait::async_trait;
use maidan_types::WorkspaceId;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::error::BusError;

const RESOURCE_CHANNEL: &str = "maidan_resource_updated";
/// Postgres `NOTIFY` payloads are capped at ~8 KB; stay safely under it.
const PAYLOAD_LIMIT: usize = 7990;

/// One `maidan://` URI a mutation touched, and the workspace the mutation
/// happened in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceUpdate {
    pub workspace_id: WorkspaceId,
    pub uri: String,
}

impl ResourceUpdate {
    pub fn new(workspace_id: WorkspaceId, uri: impl Into<String>) -> Self {
        Self {
            workspace_id,
            uri: uri.into(),
        }
    }
}

/// Backend-agnostic cross-process channel for MCP resource updates.
#[async_trait]
pub trait ResourceNotifier: Send + Sync {
    /// Broadcast the updates a mutation produced to every process. The set is
    /// unfiltered; each process applies its own subscriptions on receipt. An
    /// empty set is a no-op.
    async fn publish(&self, updates: Vec<ResourceUpdate>) -> Result<(), BusError>;

    /// Receiver of cross-process update batches for this process. A batch is
    /// the set from one [`publish`](ResourceNotifier::publish) call.
    fn subscribe(&self) -> broadcast::Receiver<Vec<ResourceUpdate>>;
}

/// In-process resource notifier (single process / SQLite / tests).
#[derive(Clone)]
pub struct InMemoryResourceNotifier {
    tx: broadcast::Sender<Vec<ResourceUpdate>>,
}

impl InMemoryResourceNotifier {
    pub fn new() -> Self {
        Self::with_capacity(crate::broadcast_cap_from_env())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
}

impl Default for InMemoryResourceNotifier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ResourceNotifier for InMemoryResourceNotifier {
    async fn publish(&self, updates: Vec<ResourceUpdate>) -> Result<(), BusError> {
        if updates.is_empty() {
            return Ok(());
        }
        // `send` errors only with zero receivers; that is not a failure for
        // fire-and-forget fan-out.
        let _ = self.tx.send(updates);
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Vec<ResourceUpdate>> {
        self.tx.subscribe()
    }
}

/// Postgres `LISTEN`/`NOTIFY` resource notifier for multi-process fan-out.
#[derive(Clone)]
pub struct PostgresResourceNotifier {
    pool: PgPool,
    local: broadcast::Sender<Vec<ResourceUpdate>>,
}

impl PostgresResourceNotifier {
    /// Connect and start the `LISTEN` fan-in task on `maidan_resource_updated`.
    pub async fn connect(pool: PgPool) -> Result<Self, BusError> {
        let (tx, _) = broadcast::channel(crate::broadcast_cap_from_env());

        let listener_tx = tx.clone();
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.listen(RESOURCE_CHANNEL).await?;
        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(note) => match serde_json::from_str::<Vec<ResourceUpdate>>(note.payload()) {
                        Ok(updates) if !updates.is_empty() => {
                            let _ = listener_tx.send(updates);
                        }
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                payload = note.payload(),
                                "drop resource-notify payload"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "resource-notify listener errored; sleeping then retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Ok(Self { pool, local: tx })
    }

    pub fn channel() -> &'static str {
        RESOURCE_CHANNEL
    }
}

#[async_trait]
impl ResourceNotifier for PostgresResourceNotifier {
    async fn publish(&self, updates: Vec<ResourceUpdate>) -> Result<(), BusError> {
        if updates.is_empty() {
            return Ok(());
        }
        for batch in chunk_within_limit(updates, PAYLOAD_LIMIT) {
            let payload = serde_json::to_string(&batch)?;
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(RESOURCE_CHANNEL)
                .bind(&payload)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Vec<ResourceUpdate>> {
        self.local.subscribe()
    }
}

/// Split `updates` into batches whose JSON serialization stays under `limit`
/// bytes. URIs are short, so in practice a mutation's set is a single batch;
/// this is a safety net for the NOTIFY payload cap. A lone update that would
/// exceed `limit` is still emitted on its own (the DB rejects oversize NOTIFY,
/// surfacing as a publish error rather than silent loss).
fn chunk_within_limit(updates: Vec<ResourceUpdate>, limit: usize) -> Vec<Vec<ResourceUpdate>> {
    let mut batches: Vec<Vec<ResourceUpdate>> = Vec::new();
    let mut current: Vec<ResourceUpdate> = Vec::new();
    for update in updates {
        current.push(update);
        // `[...]` JSON length; cheap upper-bound check via re-serialization.
        let len = serde_json::to_string(&current)
            .map(|s| s.len())
            .unwrap_or(0);
        if len > limit && current.len() > 1 {
            if let Some(last) = current.pop() {
                batches.push(std::mem::take(&mut current));
                current.push(last);
            }
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn update(uri: &str) -> ResourceUpdate {
        ResourceUpdate::new(WorkspaceId(uuid::Uuid::nil()), uri)
    }

    #[tokio::test]
    async fn in_memory_round_trip_delivers_updates_to_subscriber() {
        let notifier = InMemoryResourceNotifier::new();
        let mut rx = notifier.subscribe();
        let sent = vec![
            update("maidan://threads/abc"),
            update("maidan://channels/def"),
        ];
        notifier.publish(sent.clone()).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(got, sent);
    }

    #[tokio::test]
    async fn empty_publish_is_a_no_op() {
        let notifier = InMemoryResourceNotifier::new();
        let mut rx = notifier.subscribe();
        notifier.publish(vec![]).await.unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn publish_without_subscribers_does_not_error() {
        let notifier = InMemoryResourceNotifier::new();
        notifier
            .publish(vec![update("maidan://workspaces/x")])
            .await
            .unwrap();
    }

    #[test]
    fn chunk_keeps_small_sets_in_one_batch() {
        let updates = vec![
            update("maidan://threads/1"),
            update("maidan://channels/2"),
            update("maidan://workspaces/3"),
        ];
        let batches = chunk_within_limit(updates.clone(), PAYLOAD_LIMIT);
        assert_eq!(batches, vec![updates]);
    }

    #[test]
    fn chunk_splits_when_over_limit() {
        // A limit below one serialized update forces one update per batch.
        let updates = vec![
            update("maidan://threads/aaaaaaaa"),
            update("maidan://threads/bbbbbbbb"),
            update("maidan://threads/cccccccc"),
        ];
        let batches = chunk_within_limit(updates, 20);
        assert_eq!(batches.len(), 3);
        for b in batches {
            assert_eq!(b.len(), 1);
        }
    }
}
