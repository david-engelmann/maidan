//! Workspace-sharded broadcast fan-out (Cluster 201).
//!
//! The buses used one broadcast channel: every publish woke *every* subscriber,
//! which then filter-and-discarded the events for other workspaces — O(total
//! subscribers) per event regardless of relevance. [`ShardedBroadcast`] routes a
//! publish only to the subscribers that could match it: the event's workspace
//! shard, plus a global shard for cross-workspace subscribers (operators, or any
//! filter without a `workspace_id`). A workspace-scoped subscriber subscribes to
//! its workspace shard and never even sees another workspace's traffic.
//!
//! This is an optimization *under* the existing [`EventFilter`] — the filter
//! still runs on each delivered event (for channel/thread/kind narrowing), it
//! just runs on far fewer events. Correctness is unchanged: a workspace-scoped
//! filter never matched another workspace's events anyway, and events with no
//! workspace go to the global shard (which is where the only subscribers that
//! could match them live).

use std::collections::HashMap;
use std::sync::Mutex;

use maidan_types::{BusEnvelope, EventFilter, WorkspaceId};
use tokio::sync::broadcast;

/// A broadcast fan-out sharded by workspace. Cheap to clone the handles it hands
/// out; hold one behind an `Arc` and share it across bus clones.
#[derive(Debug)]
pub struct ShardedBroadcast {
    capacity: usize,
    /// Receives every event — for subscribers whose filter pins no workspace.
    global: broadcast::Sender<BusEnvelope>,
    /// Per-workspace channels, created lazily when a workspace first gains a
    /// subscriber and pruned when it loses its last one.
    shards: Mutex<HashMap<WorkspaceId, broadcast::Sender<BusEnvelope>>>,
}

impl ShardedBroadcast {
    pub fn new(capacity: usize) -> Self {
        let (global, _) = broadcast::channel(capacity);
        Self {
            capacity,
            global,
            shards: Mutex::new(HashMap::new()),
        }
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<WorkspaceId, broadcast::Sender<BusEnvelope>>> {
        self.shards.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Deliver an envelope to the global shard and — when the event is
    /// workspace-scoped and that workspace has live subscribers — its workspace
    /// shard. The map lock is held only for an O(1) lookup; the actual sends
    /// happen after it is released.
    pub fn publish(&self, envelope: BusEnvelope) {
        let ws_shard = envelope
            .event
            .workspace_id()
            .and_then(|ws| self.lock().get(&ws).cloned());
        match ws_shard {
            // Both shards receive → one clone (a `send` moves the value).
            Some(tx) => {
                let _ = self.global.send(envelope.clone());
                let _ = tx.send(envelope);
            }
            None => {
                let _ = self.global.send(envelope);
            }
        }
    }

    /// A receiver scoped to `filter`: the workspace shard when the filter pins a
    /// workspace, else the global shard. The shard is created/subscribed under
    /// the map lock, so a concurrent prune can't drop a shard that just gained
    /// this receiver (its `receiver_count` is ≥ 1 before the lock is released).
    /// Dead shards (no receivers) are pruned here — subscribe is far rarer than
    /// publish, so the `retain` scan stays off the hot path.
    pub fn subscribe(&self, filter: &EventFilter) -> broadcast::Receiver<BusEnvelope> {
        match filter.workspace_id {
            Some(ws) => {
                let mut shards = self.lock();
                shards.retain(|_, tx| tx.receiver_count() > 0);
                shards
                    .entry(ws)
                    .or_insert_with(|| broadcast::channel(self.capacity).0)
                    .subscribe()
            }
            None => self.global.subscribe(),
        }
    }

    /// The number of live workspace shards (test/observability aid).
    pub fn shard_count(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use maidan_types::{Event, Workspace};

    fn ws_event(ws: WorkspaceId) -> BusEnvelope {
        BusEnvelope {
            log_id: 1,
            event: Event::WorkspaceCreated {
                occurred_at: Utc::now(),
                workspace: Workspace {
                    id: ws,
                    name: "w".into(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    tombstoned_at: None,
                },
            },
        }
    }

    fn workspace_filter(ws: WorkspaceId) -> EventFilter {
        EventFilter {
            workspace_id: Some(ws),
            ..EventFilter::default()
        }
    }

    #[tokio::test]
    async fn workspace_subscriber_only_sees_its_own_workspace() {
        let bus = ShardedBroadcast::new(16);
        let a = WorkspaceId(uuid::Uuid::new_v4());
        let b = WorkspaceId(uuid::Uuid::new_v4());
        let mut rx_a = bus.subscribe(&workspace_filter(a));
        let mut rx_b = bus.subscribe(&workspace_filter(b));

        bus.publish(ws_event(a));

        // A's shard got the event; B's did not.
        assert!(
            rx_a.try_recv().is_ok(),
            "workspace A subscriber sees A's event"
        );
        assert!(
            rx_b.try_recv().is_err(),
            "workspace B subscriber never sees A's event"
        );
    }

    #[tokio::test]
    async fn global_subscriber_sees_every_workspace() {
        let bus = ShardedBroadcast::new(16);
        let a = WorkspaceId(uuid::Uuid::new_v4());
        let b = WorkspaceId(uuid::Uuid::new_v4());
        let mut global = bus.subscribe(&EventFilter::default());

        bus.publish(ws_event(a));
        bus.publish(ws_event(b));

        assert!(global.try_recv().is_ok());
        assert!(global.try_recv().is_ok());
    }

    #[tokio::test]
    async fn shards_are_pruned_when_their_subscribers_drop() {
        let bus = ShardedBroadcast::new(16);
        let a = WorkspaceId(uuid::Uuid::new_v4());
        let rx = bus.subscribe(&workspace_filter(a));
        assert_eq!(bus.shard_count(), 1);
        drop(rx);
        // Next subscribe prunes the now-receiverless shard, then recreates one.
        let b = WorkspaceId(uuid::Uuid::new_v4());
        let _rx_b = bus.subscribe(&workspace_filter(b));
        assert_eq!(bus.shard_count(), 1, "dead shard pruned, only B remains");
    }
}
