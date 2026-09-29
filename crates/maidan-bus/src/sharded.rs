//! Workspace-sharded broadcast fan-out.
//!
//! The buses used one broadcast channel: every publish woke *every* subscriber,
//! which then filter-and-discarded the events for other workspaces — O(total
//! subscribers) per event regardless of relevance. [`ShardedBroadcast`] routes
//! a publish only to the subscribers that could match it: the event's workspace
//! shard, plus a global shard for cross-workspace subscribers (operators, or
//! any filter without a `workspace_id`). A workspace-scoped subscriber
//! subscribes to its workspace shard and never even sees another workspace's
//! traffic.
//!
//! This is an optimization *under* the existing [`EventFilter`] — the filter
//! still runs on each delivered event (for channel/thread/kind narrowing), it
//! just runs on far fewer events. Correctness is unchanged: a workspace-scoped
//! filter never matched another workspace's events anyway, and events with no
//! workspace go to the global shard (which is where the only subscribers that
//! could match them live).

use std::collections::HashMap;

use maidan_types::{BusEnvelope, EventFilter, WorkspaceId};
use tokio::sync::broadcast;

use crate::sync::{channel_step, Mutex, MutexGuard};

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

    fn lock(&self) -> MutexGuard<'_, HashMap<WorkspaceId, broadcast::Sender<BusEnvelope>>> {
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
                channel_step();
                let _ = self.global.send(envelope.clone());
                channel_step();
                let _ = tx.send(envelope);
            }
            None => {
                channel_step();
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
            None => {
                channel_step();
                self.global.subscribe()
            }
        }
    }

    /// The number of live workspace shards (test/observability aid).
    pub fn shard_count(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(all(test, not(feature = "loom")))]
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
            attribution: None,
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

/// Loom models of the fan-out: every interleaving of its lock against
/// concurrent subscribes, publishes and receiver drops. The broadcast
/// channels are tokio's, which tokio model-checks itself; what is modelled
/// here is the shard map. Run with
/// `cargo test -p maidan-bus --features loom --release --lib loom`.
#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::*;
    use chrono::Utc;
    use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use loom::sync::Arc;
    use loom::thread;
    use maidan_types::{Event, Workspace};

    fn ws_event(ws: WorkspaceId, log_id: i64) -> BusEnvelope {
        BusEnvelope {
            log_id,
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
            attribution: None,
        }
    }

    fn scoped(ws: WorkspaceId) -> EventFilter {
        EventFilter {
            workspace_id: Some(ws),
            ..EventFilter::default()
        }
    }

    fn ids(rx: &mut broadcast::Receiver<BusEnvelope>) -> Vec<i64> {
        std::iter::from_fn(|| rx.try_recv().ok().map(|e| e.log_id)).collect()
    }

    /// A publish that starts after `subscribe` returned reaches that
    /// subscriber, while another workspace's subscriber is being created and
    /// pruned beside it, and never reaches the other workspace.
    #[test]
    fn loom_a_subscriber_gets_what_is_published_after_it_subscribed() {
        loom::model(|| {
            let bus = Arc::new(ShardedBroadcast::new(8));
            let a = WorkspaceId(uuid::Uuid::from_u128(1));
            let b = WorkspaceId(uuid::Uuid::from_u128(2));
            let subscribed = Arc::new(AtomicBool::new(false));

            let subscriber = {
                let (bus, subscribed) = (bus.clone(), subscribed.clone());
                thread::spawn(move || {
                    let rx = bus.subscribe(&scoped(a));
                    subscribed.store(true, Ordering::SeqCst);
                    rx
                })
            };
            let neighbour = {
                let bus = bus.clone();
                thread::spawn(move || bus.subscribe(&scoped(b)))
            };
            let publisher = {
                let (bus, subscribed) = (bus.clone(), subscribed.clone());
                thread::spawn(move || {
                    let after = subscribed.load(Ordering::SeqCst);
                    bus.publish(ws_event(a, 1));
                    after
                })
            };

            let mut rx_a = subscriber.join().unwrap();
            let mut rx_b = neighbour.join().unwrap();
            let published_after_subscribe = publisher.join().unwrap();
            let got = ids(&mut rx_a);
            if published_after_subscribe {
                assert_eq!(got, vec![1]);
            }
            assert!(got.len() <= 1);
            assert!(ids(&mut rx_b).is_empty(), "B heard A's event");
        });
    }

    /// Two receivers subscribe to a workspace whose shard has lost its last
    /// receiver: no prune removes a shard a receiver is on, so a publish made after both
    /// subscribes reaches both, and dead shards do not pile up.
    #[test]
    fn loom_pruning_a_dead_shard_never_strands_a_new_subscriber() {
        loom::model(|| {
            let bus = Arc::new(ShardedBroadcast::new(8));
            let a = WorkspaceId(uuid::Uuid::from_u128(1));
            // A shard whose only receiver has gone: the next subscribe prunes it.
            drop(bus.subscribe(&scoped(a)));
            let subscribed = Arc::new(AtomicUsize::new(0));

            let joiners: Vec<_> = (0..2)
                .map(|_| {
                    let (bus, subscribed) = (bus.clone(), subscribed.clone());
                    thread::spawn(move || {
                        let rx = bus.subscribe(&scoped(a));
                        subscribed.fetch_add(1, Ordering::SeqCst);
                        rx
                    })
                })
                .collect();
            let publisher = {
                let (bus, subscribed) = (bus.clone(), subscribed.clone());
                thread::spawn(move || {
                    let after = subscribed.load(Ordering::SeqCst) == 2;
                    bus.publish(ws_event(a, 7));
                    after
                })
            };

            let mut receivers: Vec<_> = joiners.into_iter().map(|j| j.join().unwrap()).collect();
            if publisher.join().unwrap() {
                for rx in &mut receivers {
                    assert_eq!(ids(rx), vec![7], "a subscriber was stranded");
                }
            }
            assert_eq!(bus.shard_count(), 1);
        });
    }

    /// A cross-workspace subscriber sees every publish made after it
    /// subscribed, in publish order, whichever workspace it was for.
    #[test]
    fn loom_a_global_subscriber_sees_every_workspace_in_order() {
        loom::model(|| {
            let bus = Arc::new(ShardedBroadcast::new(8));
            let a = WorkspaceId(uuid::Uuid::from_u128(1));
            let b = WorkspaceId(uuid::Uuid::from_u128(2));
            let _scoped_a = bus.subscribe(&scoped(a));
            let subscribed = Arc::new(AtomicBool::new(false));

            let subscriber = {
                let (bus, subscribed) = (bus.clone(), subscribed.clone());
                thread::spawn(move || {
                    let rx = bus.subscribe(&EventFilter::default());
                    subscribed.store(true, Ordering::SeqCst);
                    rx
                })
            };
            let publisher = {
                let (bus, subscribed) = (bus.clone(), subscribed.clone());
                thread::spawn(move || {
                    let after = subscribed.load(Ordering::SeqCst);
                    bus.publish(ws_event(a, 1));
                    bus.publish(ws_event(b, 2));
                    after
                })
            };

            let mut rx = subscriber.join().unwrap();
            let published_after_subscribe = publisher.join().unwrap();
            let got = ids(&mut rx);
            if published_after_subscribe {
                assert_eq!(got, vec![1, 2]);
            } else {
                assert!(got == vec![1, 2] || got == vec![2] || got.is_empty());
            }
        });
    }
}
