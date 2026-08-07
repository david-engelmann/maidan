//! Cross-process fan-out of MCP resource-update URIs (Cluster 102).
//!
//! MCP `resources/subscribe` notifications (`notifications/resources/updated`)
//! must reach a subscriber regardless of which server replica handled the
//! mutation. The event log already crosses processes via [`crate::PostgresBus`];
//! this is the sibling channel for *resource* URIs, which are derived from a
//! mutation rather than carried by a domain [`maidan_types::Event`].
//!
//! Contract: [`ResourceNotifier::publish_uris`] broadcasts the **unfiltered**
//! set of `maidan://` URIs touched by a mutation to every process. Each process
//! receives them via its [`ResourceNotifier::subscribe`] receiver and applies
//! its **own** local subscription filter before delivering to clients. There is
//! a single delivery path — even the originating process delivers via the
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
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::error::BusError;

const RESOURCE_CHANNEL: &str = "maidan_resource_updated";
/// Postgres `NOTIFY` payloads are capped at ~8 KB; stay safely under it.
const PAYLOAD_LIMIT: usize = 7990;

/// Backend-agnostic cross-process channel for MCP resource-update URIs.
#[async_trait]
pub trait ResourceNotifier: Send + Sync {
    /// Broadcast the URIs touched by a mutation to every process. The set is
    /// unfiltered; subscribers apply their own subscription filter on receipt.
    /// An empty set is a no-op.
    async fn publish_uris(&self, uris: Vec<String>) -> Result<(), BusError>;

    /// Receiver of cross-process URI batches for this process. A batch is the
    /// URI set from one [`publish_uris`](ResourceNotifier::publish_uris) call.
    fn subscribe(&self) -> broadcast::Receiver<Vec<String>>;
}

/// In-process resource notifier (single process / SQLite / tests).
#[derive(Clone)]
pub struct InMemoryResourceNotifier {
    tx: broadcast::Sender<Vec<String>>,
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
    async fn publish_uris(&self, uris: Vec<String>) -> Result<(), BusError> {
        if uris.is_empty() {
            return Ok(());
        }
        // `send` errors only with zero receivers; that is not a failure for
        // fire-and-forget fan-out.
        let _ = self.tx.send(uris);
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Vec<String>> {
        self.tx.subscribe()
    }
}

/// Postgres `LISTEN`/`NOTIFY` resource notifier for multi-process fan-out.
#[derive(Clone)]
pub struct PostgresResourceNotifier {
    pool: PgPool,
    local: broadcast::Sender<Vec<String>>,
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
                    Ok(note) => match serde_json::from_str::<Vec<String>>(note.payload()) {
                        Ok(uris) if !uris.is_empty() => {
                            let _ = listener_tx.send(uris);
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
    async fn publish_uris(&self, uris: Vec<String>) -> Result<(), BusError> {
        if uris.is_empty() {
            return Ok(());
        }
        for batch in chunk_within_limit(uris, PAYLOAD_LIMIT) {
            let payload = serde_json::to_string(&batch)?;
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(RESOURCE_CHANNEL)
                .bind(&payload)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Vec<String>> {
        self.local.subscribe()
    }
}

/// Split `uris` into batches whose JSON serialization stays under `limit`
/// bytes. URIs are short, so in practice a mutation's set is a single batch;
/// this is a safety net for the NOTIFY payload cap. A lone URI that would
/// exceed `limit` is still emitted on its own (the DB rejects oversize NOTIFY,
/// surfacing as a publish error rather than silent loss).
fn chunk_within_limit(uris: Vec<String>, limit: usize) -> Vec<Vec<String>> {
    let mut batches: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for uri in uris {
        current.push(uri);
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

    #[tokio::test]
    async fn in_memory_round_trip_delivers_uris_to_subscriber() {
        let notifier = InMemoryResourceNotifier::new();
        let mut rx = notifier.subscribe();
        notifier
            .publish_uris(vec![
                "maidan://threads/abc".into(),
                "maidan://channels/def".into(),
            ])
            .await
            .unwrap();
        let got = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(got, vec!["maidan://threads/abc", "maidan://channels/def"]);
    }

    #[tokio::test]
    async fn empty_publish_is_a_no_op() {
        let notifier = InMemoryResourceNotifier::new();
        let mut rx = notifier.subscribe();
        notifier.publish_uris(vec![]).await.unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn publish_without_subscribers_does_not_error() {
        let notifier = InMemoryResourceNotifier::new();
        notifier
            .publish_uris(vec!["maidan://workspaces/x".into()])
            .await
            .unwrap();
    }

    #[test]
    fn chunk_keeps_small_sets_in_one_batch() {
        let uris = vec![
            "maidan://threads/1".to_string(),
            "maidan://channels/2".to_string(),
            "maidan://workspaces/3".to_string(),
        ];
        let batches = chunk_within_limit(uris.clone(), PAYLOAD_LIMIT);
        assert_eq!(batches, vec![uris]);
    }

    #[test]
    fn chunk_splits_when_over_limit() {
        // Tiny limit forces one URI per batch.
        let uris = vec![
            "maidan://threads/aaaaaaaa".to_string(),
            "maidan://threads/bbbbbbbb".to_string(),
            "maidan://threads/cccccccc".to_string(),
        ];
        let batches = chunk_within_limit(uris, 20);
        assert_eq!(batches.len(), 3);
        for b in batches {
            assert_eq!(b.len(), 1);
        }
    }
}
