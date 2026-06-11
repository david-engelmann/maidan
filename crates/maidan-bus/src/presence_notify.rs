//! Cross-process presence/typing fan-out (Cluster 103).
//!
//! Sibling of [`crate::resource_notify`]: where that channel carries resource
//! URIs, this one carries typed [`PresenceEvent`]s so presence, typing, and the
//! workspace roster stay consistent across server replicas. The server's
//! `PresenceHub` publishes a `PresenceEvent` on every local change (and on a
//! periodic heartbeat); each replica's listener fans the event out to its local
//! WebSocket subscribers and folds presence state into a merged, TTL-expiring
//! roster view.
//!
//! `PresenceEvent::origin` identifies the publishing replica so a receiver can
//! skip its **own** members in the merged remote view while still delivering to
//! local subscribers — a single delivery path, no de-duplication.
//!
//! Two implementations mirror [`crate::EventBus`]: [`InMemoryPresenceNotifier`]
//! (single process / SQLite / tests) and [`PostgresPresenceNotifier`]
//! (`LISTEN`/`NOTIFY` on `maidan_presence`). Delivery is at-most-once; a dropped
//! event is reconciled by the next heartbeat (or the TTL sweep).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::error::BusError;

const PRESENCE_CHANNEL: &str = "maidan_presence";
/// Postgres `NOTIFY` payloads are capped at ~8 KB; a `PresenceEvent` is far
/// smaller, but reject anything pathological rather than fail at the DB.
const PAYLOAD_LIMIT: usize = 7990;
const BROADCAST_CAP: usize = 1024;

/// The kind of presence change carried by a [`PresenceEvent`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PresenceEventKind {
    Online,
    Away,
    Offline,
    Typing { thread_id: Uuid, active: bool },
}

/// A presence/typing change broadcast across replicas.
///
/// Raw `Uuid`s (not typed IDs) keep the wire envelope independent of the
/// domain-type crate; the server maps them back to `WorkspaceId`/`MemberId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceEvent {
    /// Replica that originated this event.
    pub origin: Uuid,
    pub workspace_id: Uuid,
    pub member_id: Uuid,
    /// A periodic re-announcement (not a state change): receivers refresh the
    /// member's TTL but suppress live fan-out unless the status actually
    /// changed. `false` for genuine register/status/typing transitions.
    #[serde(default)]
    pub heartbeat: bool,
    #[serde(flatten)]
    pub kind: PresenceEventKind,
}

/// Backend-agnostic cross-process channel for presence/typing events.
#[async_trait]
pub trait PresenceNotifier: Send + Sync {
    /// Broadcast a presence/typing event to every replica (including this one).
    async fn publish_presence(&self, event: PresenceEvent) -> Result<(), BusError>;

    /// Receiver of cross-process presence events for this process.
    fn subscribe(&self) -> broadcast::Receiver<PresenceEvent>;
}

/// In-process presence notifier (single process / SQLite / tests).
#[derive(Clone)]
pub struct InMemoryPresenceNotifier {
    tx: broadcast::Sender<PresenceEvent>,
}

impl InMemoryPresenceNotifier {
    pub fn new() -> Self {
        Self::with_capacity(BROADCAST_CAP)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
}

impl Default for InMemoryPresenceNotifier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PresenceNotifier for InMemoryPresenceNotifier {
    async fn publish_presence(&self, event: PresenceEvent) -> Result<(), BusError> {
        let _ = self.tx.send(event);
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<PresenceEvent> {
        self.tx.subscribe()
    }
}

/// Postgres `LISTEN`/`NOTIFY` presence notifier for multi-process fan-out.
#[derive(Clone)]
pub struct PostgresPresenceNotifier {
    pool: PgPool,
    local: broadcast::Sender<PresenceEvent>,
}

impl PostgresPresenceNotifier {
    /// Connect and start the `LISTEN` fan-in task on `maidan_presence`.
    pub async fn connect(pool: PgPool) -> Result<Self, BusError> {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);

        let listener_tx = tx.clone();
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.listen(PRESENCE_CHANNEL).await?;
        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(note) => match serde_json::from_str::<PresenceEvent>(note.payload()) {
                        Ok(event) => {
                            let _ = listener_tx.send(event);
                        }
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                payload = note.payload(),
                                "drop presence-notify payload"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "presence-notify listener errored; sleeping then retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Ok(Self { pool, local: tx })
    }

    pub fn channel() -> &'static str {
        PRESENCE_CHANNEL
    }
}

#[async_trait]
impl PresenceNotifier for PostgresPresenceNotifier {
    async fn publish_presence(&self, event: PresenceEvent) -> Result<(), BusError> {
        let payload = serde_json::to_string(&event)?;
        if payload.len() > PAYLOAD_LIMIT {
            return Err(BusError::PayloadTooLarge(payload.len()));
        }
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(PRESENCE_CHANNEL)
            .bind(&payload)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<PresenceEvent> {
        self.local.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn event(kind: PresenceEventKind) -> PresenceEvent {
        PresenceEvent {
            origin: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            member_id: Uuid::new_v4(),
            heartbeat: false,
            kind,
        }
    }

    #[tokio::test]
    async fn in_memory_round_trip_delivers_event() {
        let notifier = InMemoryPresenceNotifier::new();
        let mut rx = notifier.subscribe();
        let ev = event(PresenceEventKind::Online);
        notifier.publish_presence(ev.clone()).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(got, ev);
    }

    #[test]
    fn typing_event_serde_round_trips_through_flattened_kind() {
        let ev = event(PresenceEventKind::Typing {
            thread_id: Uuid::new_v4(),
            active: true,
        });
        let json = serde_json::to_string(&ev).unwrap();
        // `kind` is flattened: the discriminator and its fields sit alongside
        // the envelope fields.
        assert!(json.contains("\"kind\":\"typing\""));
        assert!(json.contains("\"active\":true"));
        let back: PresenceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn status_events_serde_round_trip() {
        for kind in [
            PresenceEventKind::Online,
            PresenceEventKind::Away,
            PresenceEventKind::Offline,
        ] {
            let ev = event(kind);
            let json = serde_json::to_string(&ev).unwrap();
            let back: PresenceEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ev);
        }
    }

    #[tokio::test]
    async fn publish_without_subscribers_does_not_error() {
        let notifier = InMemoryPresenceNotifier::new();
        notifier
            .publish_presence(event(PresenceEventKind::Offline))
            .await
            .unwrap();
    }
}
