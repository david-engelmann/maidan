//! Workspace presence and typing fan-out for WebSocket subscribers.
//!
//! Single-process by default. When a [`maidan_bus::PresenceNotifier`] is wired,
//! presence/typing fan out **across replicas**: every local change is published
//! as a [`PresenceEvent`], each replica's listener delivers it to its own
//! WebSocket subscribers, and presence state is folded into a merged,
//! TTL-expiring roster so `presence_snapshot` reflects members on any replica.
//! A periodic heartbeat re-announces locally-connected members so a crashed
//! replica's members expire elsewhere within the TTL.
//!
//! Every change is decided and delivered under the hub's lock: the frame goes
//! to local subscribers, and the cross-replica event joins an ordered queue
//! that one task publishes from. Frames and events therefore leave in the
//! order the changes happened, so a subscriber's last word on a member is the
//! member's state. Announcing after releasing the lock let a reconnect's
//! `online` overtake the `offline` of the connection it replaced. A replica
//! ignores its own events coming back from the notifier; its subscribers
//! already had the frame. A member connected here is reported from local
//! state: another replica's word on it changes the merged roster, not what
//! local subscribers are told. The `loom` tests below check the ordering.
//!
//! A heartbeat takes one slot in the queue: the publisher reads the local
//! members when it reaches it, so the heartbeat is never older than a change
//! queued before it, however many members are connected.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, PoisonError, Weak,
    },
    time::{Duration, Instant},
};

use maidan_bus::{PresenceEvent, PresenceEventKind, PresenceNotifier};
use maidan_types::{MemberId, ThreadId, WorkspaceId};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

// loom's lock in this crate's tests under the `loom` feature, so the model
// tests can explore every interleaving of the hub's critical sections. Any
// other build gets std's.
#[cfg(all(test, feature = "loom"))]
use loom::sync::RwLock;
#[cfg(not(all(test, feature = "loom")))]
use std::sync::RwLock;

const EPHEMERAL_CAPACITY: usize = 256;
/// Cross-replica changes waiting for the publisher. Past this (a stalled
/// notifier) new ones are dropped; heartbeats and the TTL sweep repair the
/// other replicas' view once it recovers.
const OUTBOX_CAPACITY: usize = 16_384;
/// How long the publisher waits on one publish before logging it and
/// moving on, so a stalled notifier cannot hold up the queue behind it.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_HEARTBEAT_SECS: u64 = 10;
const DEFAULT_TTL_SECS: u64 = 30;

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceStatus {
    Online,
    Away,
}

impl PresenceStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "online" => Some(Self::Online),
            "away" => Some(Self::Away),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
        }
    }

    fn from_event_kind(kind: &PresenceEventKind) -> Option<Self> {
        match kind {
            PresenceEventKind::Online => Some(Self::Online),
            PresenceEventKind::Away => Some(Self::Away),
            _ => None,
        }
    }

    fn event_kind(self) -> PresenceEventKind {
        match self {
            Self::Online => PresenceEventKind::Online,
            Self::Away => PresenceEventKind::Away,
        }
    }
}

/// What the publisher sends, in queue order.
#[derive(Debug)]
enum Outgoing {
    Change(PresenceEvent),
    /// Re-announce every locally connected member, as they are when the
    /// publisher reaches this.
    Heartbeat,
}

#[derive(Debug, Clone, Serialize)]
pub struct PresenceMember {
    pub member_id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct PresenceRegistration {
    conn_id: u64,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    hub: PresenceHub,
}

impl Drop for PresenceRegistration {
    // Invariant: NO occupancy/store I/O in Drop. `unregister` mutates only the
    // in-memory `PresenceHub` (it holds no `Store` handle), so this stays
    // synchronous and non-blocking. Drop can't be async and runs on whatever
    // task drops the guard; a durable write here would either block that task
    // or be silently lost. Occupancy that must persist (the claim lease /
    // working clock) is written by explicit calls, never a destructor.
    fn drop(&mut self) {
        self.hub
            .unregister(self.conn_id, self.workspace_id, self.member_id);
    }
}

#[derive(Debug)]
struct MemberState {
    status: PresenceStatus,
    connections: u32,
}

/// What one other replica last said about a member (via cross-pod events).
#[derive(Debug)]
struct RemoteMember {
    status: PresenceStatus,
    last_seen: Instant,
}

/// A member's entries, one per replica it is connected to.
type RemoteEntries = HashMap<Uuid, RemoteMember>;

/// A member's status across replicas: online if any replica says online.
/// With `live`, entries older than the TTL are ignored.
fn merged_status(
    entries: &RemoteEntries,
    live: Option<(Instant, Duration)>,
) -> Option<PresenceStatus> {
    entries
        .values()
        .filter(|rm| live.is_none_or(|(now, ttl)| now.duration_since(rm.last_seen) <= ttl))
        .map(|rm| rm.status)
        .reduce(|a, b| {
            if a == PresenceStatus::Online || b == PresenceStatus::Online {
                PresenceStatus::Online
            } else {
                PresenceStatus::Away
            }
        })
}

#[derive(Debug)]
struct WorkspaceRoom {
    tx: broadcast::Sender<String>,
    members: HashMap<MemberId, MemberState>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Workspaces with at least one local subscriber (holds the fan-out channel
    /// and locally-connected members).
    workspaces: HashMap<WorkspaceId, WorkspaceRoom>,
    /// Members present on *other* replicas, per replica, kept fresh by
    /// heartbeats and expired by TTL. Tracked even for workspaces with no
    /// local room so a new local subscriber's snapshot includes them.
    remote: HashMap<WorkspaceId, HashMap<MemberId, RemoteEntries>>,
}

impl Inner {
    fn remote_status(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        live: Option<(Instant, Duration)>,
    ) -> Option<PresenceStatus> {
        self.remote
            .get(&workspace_id)
            .and_then(|members| members.get(&member_id))
            .and_then(|entries| merged_status(entries, live))
    }
}

#[derive(Clone)]
pub struct PresenceHub {
    inner: Arc<RwLock<Inner>>,
    /// This replica's id; stamped on published events so the listener can
    /// tell its own events from other replicas'.
    origin: Uuid,
    notifier: Option<Arc<dyn PresenceNotifier>>,
    /// Cross-replica changes in the order they were decided. A change is
    /// only ever queued under `inner`'s lock.
    outbox: Option<mpsc::Sender<Outgoing>>,
    /// The other end, taken by [`PresenceHub::spawn_tasks`].
    outbox_rx: Arc<Mutex<Option<mpsc::Receiver<Outgoing>>>>,
    /// A heartbeat is in the queue; the next tick does not add another.
    heartbeat_queued: Arc<AtomicBool>,
    /// The queue was full at the last change, so the drop was logged.
    outbox_full: Arc<AtomicBool>,
    ttl: Duration,
    heartbeat: Duration,
}

impl std::fmt::Debug for PresenceHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresenceHub")
            .field("origin", &self.origin)
            .field("distributed", &self.notifier.is_some())
            .field("ttl", &self.ttl)
            .field("heartbeat", &self.heartbeat)
            .finish()
    }
}

impl Default for PresenceHub {
    fn default() -> Self {
        Self::new()
    }
}

impl PresenceHub {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner::default())),
            origin: Uuid::now_v7(),
            notifier: None,
            outbox: None,
            outbox_rx: Arc::new(Mutex::new(None)),
            heartbeat_queued: Arc::new(AtomicBool::new(false)),
            outbox_full: Arc::new(AtomicBool::new(false)),
            ttl: Duration::from_secs(ttl_secs_from_env()),
            heartbeat: Duration::from_secs(heartbeat_secs_from_env()),
        }
    }

    /// Wire cross-replica presence fan-out. Call [`PresenceHub::spawn_tasks`]
    /// afterwards to start the publisher, listener and heartbeat.
    pub fn with_presence_notifier(mut self, notifier: Arc<dyn PresenceNotifier>) -> Self {
        let (tx, rx) = mpsc::channel(OUTBOX_CAPACITY);
        self.notifier = Some(notifier);
        self.outbox = Some(tx);
        self.outbox_rx = Arc::new(Mutex::new(Some(rx)));
        self
    }

    /// Start the cross-replica publisher, listener and heartbeat/TTL-sweep
    /// tasks. No-op when no notifier is wired (single-process mode), and on a
    /// second call.
    pub fn spawn_tasks(&self) {
        let Some(notifier) = self.notifier.clone() else {
            return;
        };
        let Some(outbox) = self
            .outbox_rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        spawn_publisher(
            notifier.clone(),
            outbox,
            Arc::downgrade(&self.inner),
            self.origin,
            self.heartbeat_queued.clone(),
            PUBLISH_TIMEOUT,
        );
        self.spawn_listener(notifier);
        self.spawn_heartbeat();
    }

    /// Current merged presence for one member. Local state wins over a remote
    /// replica; expired remote heartbeats read as offline (`None`).
    pub fn status(&self, workspace_id: WorkspaceId, member_id: MemberId) -> Option<PresenceStatus> {
        let now = Instant::now();
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(local) = inner
            .workspaces
            .get(&workspace_id)
            .and_then(|room| room.members.get(&member_id))
        {
            return Some(local.status);
        }
        inner.remote_status(workspace_id, member_id, Some((now, self.ttl)))
    }

    fn spawn_listener(&self, notifier: Arc<dyn PresenceNotifier>) {
        let hub = self.clone();
        let mut rx = notifier.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => hub.apply_remote_event(event),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "presence listener lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    fn spawn_heartbeat(&self) {
        let hub = self.clone();
        let interval = self.heartbeat;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                hub.heartbeat_local_members();
                hub.sweep_expired_remote();
            }
        });
    }

    /// The current `presence_snapshot` frame for `workspace_id`: what a
    /// subscriber that fell behind the diff stream is sent to resynchronize.
    pub fn snapshot(&self, workspace_id: WorkspaceId) -> String {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        build_snapshot(workspace_id, &inner, self.ttl, Instant::now())
    }

    pub fn register(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> (broadcast::Receiver<String>, PresenceRegistration, String) {
        let conn_id = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
        let (rx, snapshot) = {
            let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
            let room = inner
                .workspaces
                .entry(workspace_id)
                .or_insert_with(|| WorkspaceRoom {
                    tx: broadcast::channel(EPHEMERAL_CAPACITY).0,
                    members: HashMap::new(),
                });
            let entry = room.members.entry(member_id).or_insert(MemberState {
                status: PresenceStatus::Online,
                connections: 0,
            });
            // A first connection, or one that brings an away member back.
            let announce = entry.connections == 0 || entry.status != PresenceStatus::Online;
            entry.connections += 1;
            entry.status = PresenceStatus::Online;
            // Announce before this connection's own receiver exists, so a
            // registrant never receives its own online frame.
            if announce {
                let _ = room.tx.send(presence_payload(
                    workspace_id,
                    member_id,
                    PresenceStatus::Online,
                ));
                self.enqueue(workspace_id, member_id, PresenceEventKind::Online);
            }
            let rx = room.tx.subscribe();
            let snapshot = build_snapshot(workspace_id, &inner, self.ttl, Instant::now());
            (rx, snapshot)
        };
        let reg = PresenceRegistration {
            conn_id,
            workspace_id,
            member_id,
            hub: self.clone(),
        };
        (rx, reg, snapshot)
    }

    fn unregister(&self, _conn_id: u64, workspace_id: WorkspaceId, member_id: MemberId) {
        let now = Instant::now();
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let still_remote = inner.remote_status(workspace_id, member_id, Some((now, self.ttl)));
        let Some(room) = inner.workspaces.get_mut(&workspace_id) else {
            return;
        };
        let Some(entry) = room.members.get_mut(&member_id) else {
            return;
        };
        if entry.connections == 0 {
            return;
        }
        entry.connections -= 1;
        if entry.connections == 0 {
            room.members.remove(&member_id);
            // Still present on another replica: that is the member's state
            // here now. The other replicas hear only that this one is done.
            let frame = match still_remote {
                Some(status) => presence_payload(workspace_id, member_id, status),
                None => offline_payload(workspace_id, member_id),
            };
            let _ = room.tx.send(frame);
            self.enqueue(workspace_id, member_id, PresenceEventKind::Offline);
        }
    }

    pub fn set_presence(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        status: PresenceStatus,
    ) -> bool {
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let Some(room) = inner.workspaces.get_mut(&workspace_id) else {
            return false;
        };
        let Some(entry) = room.members.get_mut(&member_id) else {
            return false;
        };
        if entry.status == status {
            return false;
        }
        entry.status = status;
        let _ = room
            .tx
            .send(presence_payload(workspace_id, member_id, status));
        self.enqueue(workspace_id, member_id, status.event_kind());
        true
    }

    pub fn set_typing(
        &self,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        member_id: MemberId,
        active: bool,
    ) {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(room) = inner.workspaces.get(&workspace_id) {
            let _ = room
                .tx
                .send(typing_payload(workspace_id, thread_id, member_id, active));
        }
        self.enqueue(
            workspace_id,
            member_id,
            PresenceEventKind::Typing {
                thread_id: thread_id.0,
                active,
            },
        );
    }

    /// Queue a local change for the other replicas. Called under `inner`'s
    /// lock, so the queue holds changes in the order they were made. No-op in
    /// single-process mode.
    fn enqueue(&self, workspace_id: WorkspaceId, member_id: MemberId, kind: PresenceEventKind) {
        let Some(outbox) = &self.outbox else {
            return;
        };
        let event = PresenceEvent {
            origin: self.origin,
            workspace_id: workspace_id.0,
            member_id: member_id.0,
            heartbeat: false,
            kind,
        };
        // One warning per full spell, not one per dropped change.
        match outbox.try_send(Outgoing::Change(event)) {
            Ok(()) => {
                if self.outbox_full.swap(false, Ordering::Relaxed) {
                    tracing::info!("presence outbox drained; cross-replica changes resume");
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                if !self.outbox_full.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "presence outbox full; dropping cross-replica changes until it drains"
                    );
                }
            }
        }
    }

    /// Apply a cross-replica event: fold the sending replica's word into its
    /// own entry for the member and, when that changed the member's merged
    /// status, tell the local subscribers. This replica's own events are
    /// skipped: they were delivered locally when they happened.
    fn apply_remote_event(&self, event: PresenceEvent) {
        if event.origin == self.origin {
            return;
        }
        let workspace_id = WorkspaceId(event.workspace_id);
        let member_id = MemberId(event.member_id);
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        if let PresenceEventKind::Typing { thread_id, active } = &event.kind {
            if let Some(room) = inner.workspaces.get(&workspace_id) {
                let _ = room.tx.send(typing_payload(
                    workspace_id,
                    ThreadId(*thread_id),
                    member_id,
                    *active,
                ));
            }
            return;
        }
        // Compared as last announced (expiry is announced by the sweep), so a
        // heartbeat that repeats a status refreshes the TTL silently.
        let before = inner.remote_status(workspace_id, member_id, None);
        let members = inner.remote.entry(workspace_id).or_default();
        let entries = members.entry(member_id).or_default();
        match PresenceStatus::from_event_kind(&event.kind) {
            // Offline: only the sender is done with the member.
            None => {
                entries.remove(&event.origin);
            }
            Some(status) => {
                entries.insert(
                    event.origin,
                    RemoteMember {
                        status,
                        last_seen: Instant::now(),
                    },
                );
            }
        }
        let after = merged_status(entries, None);
        if entries.is_empty() {
            members.remove(&member_id);
        }
        if members.is_empty() {
            inner.remote.remove(&workspace_id);
        }
        let frame = (before != after).then(|| match after {
            Some(status) => presence_payload(workspace_id, member_id, status),
            None => offline_payload(workspace_id, member_id),
        });
        if let (Some(frame), Some(room)) = (frame, inner.workspaces.get(&workspace_id)) {
            // A member connected here is reported from local state.
            if !room.members.contains_key(&member_id) {
                let _ = room.tx.send(frame);
            }
        }
    }

    /// Queue a heartbeat so other replicas refresh their TTL for this
    /// replica's members. One at a time: a tick while one is still queued
    /// adds nothing, and a full queue skips this tick.
    fn heartbeat_local_members(&self) {
        let Some(outbox) = &self.outbox else {
            return;
        };
        if self.heartbeat_queued.swap(true, Ordering::AcqRel) {
            return;
        }
        if outbox.try_send(Outgoing::Heartbeat).is_err() {
            self.heartbeat_queued.store(false, Ordering::Release);
        }
    }

    /// Drop remote entries whose last heartbeat is older than the TTL,
    /// telling local subscribers when that changes a member's status.
    fn sweep_expired_remote(&self) {
        let now = Instant::now();
        let ttl = self.ttl;
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let mut changed = Vec::new();
        for (ws, members) in inner.remote.iter_mut() {
            members.retain(|mid, entries| {
                let before = merged_status(entries, None);
                entries.retain(|_, rm| now.duration_since(rm.last_seen) <= ttl);
                let after = merged_status(entries, None);
                if before != after {
                    changed.push((*ws, *mid, after));
                }
                !entries.is_empty()
            });
        }
        inner.remote.retain(|_, members| !members.is_empty());
        for (ws, mid, status) in changed {
            if let Some(room) = inner.workspaces.get(&ws) {
                if !room.members.contains_key(&mid) {
                    let frame = match status {
                        Some(status) => presence_payload(ws, mid, status),
                        None => offline_payload(ws, mid),
                    };
                    let _ = room.tx.send(frame);
                }
            }
        }
    }
}

/// One heartbeat event per locally connected member, as they are now.
fn heartbeat_events(inner: &RwLock<Inner>, origin: Uuid) -> Vec<PresenceEvent> {
    let inner = inner.read().unwrap_or_else(PoisonError::into_inner);
    inner
        .workspaces
        .iter()
        .flat_map(|(ws, room)| {
            room.members.iter().map(move |(mid, state)| PresenceEvent {
                origin,
                workspace_id: ws.0,
                member_id: mid.0,
                heartbeat: true,
                kind: state.status.event_kind(),
            })
        })
        .collect()
}

/// Publish the queue one event at a time, in order. A heartbeat is read when
/// it is reached, so it is never older than a change queued before it. A
/// publish that takes longer than `timeout` is abandoned.
fn spawn_publisher(
    notifier: Arc<dyn PresenceNotifier>,
    mut outbox: mpsc::Receiver<Outgoing>,
    inner: Weak<RwLock<Inner>>,
    origin: Uuid,
    heartbeat_queued: Arc<AtomicBool>,
    timeout: Duration,
) {
    tokio::spawn(async move {
        while let Some(item) = outbox.recv().await {
            let events = match item {
                Outgoing::Change(event) => vec![event],
                Outgoing::Heartbeat => {
                    heartbeat_queued.store(false, Ordering::Release);
                    let Some(inner) = inner.upgrade() else {
                        break;
                    };
                    heartbeat_events(&inner, origin)
                }
            };
            for event in events {
                match tokio::time::timeout(timeout, notifier.publish_presence(event)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => tracing::warn!(error = %err, "presence publish failed"),
                    Err(_) => tracing::warn!(?timeout, "presence publish timed out"),
                }
            }
        }
    });
}

impl maidan_mcp::PresenceReader for PresenceHub {
    fn member_presence(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> maidan_types::OccupancyPresence {
        match self.status(workspace_id, member_id) {
            Some(PresenceStatus::Online) => maidan_types::OccupancyPresence::Online,
            Some(PresenceStatus::Away) => maidan_types::OccupancyPresence::Away,
            None => maidan_types::OccupancyPresence::Offline,
        }
    }
}

fn ttl_secs_from_env() -> u64 {
    std::env::var("MAIDAN_PRESENCE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_TTL_SECS)
}

fn heartbeat_secs_from_env() -> u64 {
    std::env::var("MAIDAN_PRESENCE_HEARTBEAT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_HEARTBEAT_SECS)
}

fn offline_payload(workspace_id: WorkspaceId, member_id: MemberId) -> String {
    serde_json::json!({
        "type": "presence",
        "workspace_id": workspace_id.0,
        "member_id": member_id.0,
        "status": "offline",
    })
    .to_string()
}

fn presence_payload(
    workspace_id: WorkspaceId,
    member_id: MemberId,
    status: PresenceStatus,
) -> String {
    serde_json::json!({
        "type": "presence",
        "workspace_id": workspace_id.0,
        "member_id": member_id.0,
        "status": status.as_str(),
    })
    .to_string()
}

fn typing_payload(
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    member_id: MemberId,
    active: bool,
) -> String {
    serde_json::json!({
        "type": "typing",
        "workspace_id": workspace_id.0,
        "thread_id": thread_id.0,
        "member_id": member_id.0,
        "active": active,
    })
    .to_string()
}

/// Build the `presence_snapshot` frame: local members merged with non-expired
/// remote members (local wins on duplicate member ids).
fn build_snapshot(workspace_id: WorkspaceId, inner: &Inner, ttl: Duration, now: Instant) -> String {
    let mut merged: HashMap<Uuid, String> = HashMap::new();
    if let Some(members) = inner.remote.get(&workspace_id) {
        for (id, entries) in members {
            if let Some(status) = merged_status(entries, Some((now, ttl))) {
                merged.insert(id.0, status.as_str().to_string());
            }
        }
    }
    if let Some(room) = inner.workspaces.get(&workspace_id) {
        for (id, st) in &room.members {
            merged.insert(id.0, st.status.as_str().to_string());
        }
    }
    let list: Vec<PresenceMember> = merged
        .into_iter()
        .map(|(member_id, status)| PresenceMember { member_id, status })
        .collect();
    serde_json::json!({
        "type": "presence_snapshot",
        "workspace_id": workspace_id.0,
        "members": list,
    })
    .to_string()
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;
    use maidan_bus::InMemoryPresenceNotifier;

    /// The snapshot a lagging subscriber is resent reflects the hub's current
    /// roster, including a member who arrived during the lag.
    #[test]
    fn the_resync_snapshot_reflects_the_current_roster() {
        let hub = PresenceHub::new();
        let ws = WorkspaceId(Uuid::new_v4());
        let first = MemberId(Uuid::new_v4());
        let later = MemberId(Uuid::new_v4());
        let (_rx, _reg, _) = hub.register(ws, first);
        let (_rx2, _reg2, _) = hub.register(ws, later);
        let snapshot = hub.snapshot(ws);
        assert!(snapshot.contains("presence_snapshot"));
        assert!(snapshot.contains(&first.0.to_string()));
        assert!(snapshot.contains(&later.0.to_string()));
    }

    #[test]
    fn snapshot_lists_local_and_remote_members() {
        let mut inner = Inner::default();
        let ws = WorkspaceId(Uuid::new_v4());
        let local_id = MemberId(Uuid::new_v4());
        let remote_id = MemberId(Uuid::new_v4());
        let mut room = WorkspaceRoom {
            tx: broadcast::channel(8).0,
            members: HashMap::new(),
        };
        room.members.insert(
            local_id,
            MemberState {
                status: PresenceStatus::Online,
                connections: 1,
            },
        );
        inner.workspaces.insert(ws, room);
        inner
            .remote
            .entry(ws)
            .or_default()
            .entry(remote_id)
            .or_default()
            .insert(
                Uuid::new_v4(),
                RemoteMember {
                    status: PresenceStatus::Away,
                    last_seen: Instant::now(),
                },
            );
        let json = build_snapshot(ws, &inner, Duration::from_secs(30), Instant::now());
        assert!(json.contains("presence_snapshot"));
        assert!(json.contains(&remote_id.0.to_string()));
        assert!(json.contains("away"));
    }

    #[test]
    fn snapshot_omits_expired_remote_members() {
        let mut inner = Inner::default();
        let ws = WorkspaceId(Uuid::new_v4());
        let remote_id = MemberId(Uuid::new_v4());
        inner
            .remote
            .entry(ws)
            .or_default()
            .entry(remote_id)
            .or_default()
            .insert(
                Uuid::new_v4(),
                RemoteMember {
                    status: PresenceStatus::Online,
                    last_seen: Instant::now() - Duration::from_secs(120),
                },
            );
        let json = build_snapshot(ws, &inner, Duration::from_secs(30), Instant::now());
        assert!(!json.contains(&remote_id.0.to_string()));
    }

    #[test]
    fn status_tracks_local_registration_and_away_state() {
        let hub = PresenceHub::new();
        let ws = WorkspaceId(Uuid::new_v4());
        let member = MemberId(Uuid::new_v4());
        assert_eq!(hub.status(ws, member), None);
        let (_rx, registration, _snapshot) = hub.register(ws, member);
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Online));
        assert!(hub.set_presence(ws, member, PresenceStatus::Away));
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Away));
        drop(registration);
        assert_eq!(hub.status(ws, member), None);
    }

    #[tokio::test]
    async fn presence_fans_out_to_another_hub_over_shared_notifier() {
        let notifier = Arc::new(InMemoryPresenceNotifier::new());
        let hub_a = PresenceHub::new().with_presence_notifier(notifier.clone());
        let hub_b = PresenceHub::new().with_presence_notifier(notifier.clone());
        hub_a.spawn_tasks();
        hub_b.spawn_tasks();

        let ws = WorkspaceId(Uuid::new_v4());
        let member = MemberId(Uuid::new_v4());

        // A local subscriber on hub B for this workspace.
        let (mut rx_b, _reg_b, _snap) = hub_b.register(ws, MemberId(Uuid::new_v4()));

        // A member comes online on hub A → published → hub B's listener delivers.
        let (_rx_a, _reg_a, _snap_a) = hub_a.register(ws, member);

        // Drain until A's member's presence frame arrives.
        let needle = member.0.to_string();
        let found = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match rx_b.recv().await {
                    Ok(frame) if frame.contains(&needle) && frame.contains("presence") => {
                        break true
                    }
                    Ok(_) => continue,
                    Err(_) => break false,
                }
            }
        })
        .await
        .expect("timed out waiting for cross-hub presence");
        assert!(found, "member presence not delivered to the other hub");
    }

    #[tokio::test]
    async fn remote_member_appears_in_new_subscriber_snapshot() {
        let notifier = Arc::new(InMemoryPresenceNotifier::new());
        let hub_a = PresenceHub::new().with_presence_notifier(notifier.clone());
        let hub_b = PresenceHub::new().with_presence_notifier(notifier.clone());
        hub_a.spawn_tasks();
        hub_b.spawn_tasks();

        let ws = WorkspaceId(Uuid::new_v4());
        let member_a = MemberId(Uuid::new_v4());
        // Member online on A; hub B has no local subscriber yet.
        let (_rx_a, _reg_a, _snap_a) = hub_a.register(ws, member_a);

        // Give B's listener a moment to fold the remote member in.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // A new subscriber on B sees the remote member in its snapshot.
        let (_rx_b, _reg_b, snapshot) = hub_b.register(ws, MemberId(Uuid::new_v4()));
        assert!(snapshot.contains(&member_a.0.to_string()));
    }

    #[tokio::test]
    async fn single_process_without_notifier_still_broadcasts_locally() {
        let hub = PresenceHub::new();
        let ws = WorkspaceId(Uuid::new_v4());
        let (_rx, _reg, _snap) = hub.register(ws, MemberId(Uuid::new_v4()));
        let mut rx2 = {
            let inner = hub.inner.read().unwrap();
            inner.workspaces.get(&ws).unwrap().tx.subscribe()
        };
        hub.set_typing(ws, ThreadId(Uuid::new_v4()), MemberId(Uuid::new_v4()), true);
        let frame = tokio::time::timeout(Duration::from_secs(1), rx2.recv())
            .await
            .expect("timed out")
            .expect("closed");
        assert!(frame.contains("typing"));
    }

    #[tokio::test]
    async fn heartbeat_with_unchanged_status_refreshes_ttl_without_refiring() {
        let hub =
            PresenceHub::new().with_presence_notifier(Arc::new(InMemoryPresenceNotifier::new()));
        let ws = WorkspaceId(Uuid::new_v4());
        let member = MemberId(Uuid::new_v4());
        let other_origin = Uuid::new_v4(); // a different replica

        // A local subscriber gives us a room to receive on. No listener is
        // spawned; we drive `apply_remote_event` directly to simulate one.
        let (mut rx, _reg, _snap) = hub.register(ws, MemberId(Uuid::new_v4()));

        // First sighting of the remote member is a change → fans out.
        hub.apply_remote_event(PresenceEvent {
            origin: other_origin,
            workspace_id: ws.0,
            member_id: member.0,
            heartbeat: false,
            kind: PresenceEventKind::Online,
        });
        let first = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out")
            .expect("closed");
        assert!(first.contains(&member.0.to_string()));

        // A heartbeat with the same status must refresh TTL but not re-fire.
        hub.apply_remote_event(PresenceEvent {
            origin: other_origin,
            workspace_id: ws.0,
            member_id: member.0,
            heartbeat: true,
            kind: PresenceEventKind::Online,
        });
        let dup = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
        assert!(
            dup.is_err(),
            "heartbeat should not re-fire presence, got {dup:?}"
        );
    }

    fn distributed() -> PresenceHub {
        PresenceHub::new().with_presence_notifier(Arc::new(InMemoryPresenceNotifier::new()))
    }

    fn remote(
        hub: &PresenceHub,
        origin: Uuid,
        ws: WorkspaceId,
        member: MemberId,
        kind: PresenceEventKind,
    ) {
        hub.apply_remote_event(PresenceEvent {
            origin,
            workspace_id: ws.0,
            member_id: member.0,
            heartbeat: false,
            kind,
        });
    }

    /// What the publisher sends for everything queued so far.
    fn publish(
        hub: &PresenceHub,
        outbox: &mut mpsc::Receiver<Outgoing>,
    ) -> Vec<(PresenceEventKind, bool)> {
        std::iter::from_fn(|| outbox.try_recv().ok())
            .flat_map(|item| match item {
                Outgoing::Change(e) => vec![e],
                Outgoing::Heartbeat => heartbeat_events(&hub.inner, hub.origin),
            })
            .map(|e| (e.kind, e.heartbeat))
            .collect()
    }

    /// The outbox holds changes in the order they were made, and a heartbeat
    /// carries the status the member has when the publisher reaches it.
    #[test]
    fn the_outbox_keeps_changes_and_heartbeats_in_order() {
        let hub = distributed();
        let ws = WorkspaceId(Uuid::new_v4());
        let member = MemberId(Uuid::new_v4());
        let (_rx, registration, _) = hub.register(ws, member);
        hub.set_presence(ws, member, PresenceStatus::Away);
        hub.heartbeat_local_members();
        let mut outbox = hub.outbox_rx.lock().unwrap().take().unwrap();
        let mut published = publish(&hub, &mut outbox);
        drop(registration);
        published.extend(publish(&hub, &mut outbox));
        assert_eq!(
            published,
            vec![
                (PresenceEventKind::Online, false),
                (PresenceEventKind::Away, false),
                (PresenceEventKind::Away, true),
                (PresenceEventKind::Offline, false),
            ]
        );
    }

    /// However many members are connected, a heartbeat takes one queue slot,
    /// and a tick while it is queued adds nothing.
    #[test]
    fn a_heartbeat_takes_one_queue_slot() {
        let hub = distributed();
        let ws = WorkspaceId(Uuid::new_v4());
        let _members: Vec<_> = (0..100)
            .map(|_| hub.register(ws, MemberId(Uuid::new_v4())))
            .collect();
        let mut outbox = hub.outbox_rx.lock().unwrap().take().unwrap();
        while outbox.try_recv().is_ok() {}
        hub.heartbeat_local_members();
        hub.heartbeat_local_members();
        assert!(matches!(outbox.try_recv(), Ok(Outgoing::Heartbeat)));
        assert!(outbox.try_recv().is_err());
        assert_eq!(heartbeat_events(&hub.inner, hub.origin).len(), 100);
    }

    /// A second connection brings an away member back online, and says so.
    #[test]
    fn a_new_connection_announces_an_away_member_back_online() {
        let hub = distributed();
        let ws = WorkspaceId(Uuid::new_v4());
        let (mut rx, _watching, _) = hub.register(ws, MemberId(Uuid::new_v4()));
        let member = MemberId(Uuid::new_v4());
        let (_rx1, _first, _) = hub.register(ws, member);
        hub.set_presence(ws, member, PresenceStatus::Away);
        while rx.try_recv().is_ok() {}
        let (_rx2, _second, _) = hub.register(ws, member);
        let frame = rx.try_recv().expect("the return is announced");
        assert!(frame.contains("\"online\"") && frame.contains(&member.0.to_string()));
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Online));
    }

    /// Another replica's word on a member connected here does not reach
    /// local subscribers: local state is what `status` reports.
    #[test]
    fn a_remote_change_does_not_override_a_local_connection() {
        let hub = distributed();
        let ws = WorkspaceId(Uuid::new_v4());
        let (mut rx, _watching, _) = hub.register(ws, MemberId(Uuid::new_v4()));
        let member = MemberId(Uuid::new_v4());
        let (_rx, _conn, _) = hub.register(ws, member);
        while rx.try_recv().is_ok() {}
        let other = Uuid::new_v4();
        remote(&hub, other, ws, member, PresenceEventKind::Away);
        remote(&hub, other, ws, member, PresenceEventKind::Offline);
        assert!(
            rx.try_recv().is_err(),
            "a remote frame overrode local state"
        );
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Online));
    }

    /// Closing the last local connection of a member still present on
    /// another replica reports that replica's status, not offline.
    #[test]
    fn closing_a_local_connection_reports_the_remote_status() {
        let hub = distributed();
        let ws = WorkspaceId(Uuid::new_v4());
        let (mut rx, _watching, _) = hub.register(ws, MemberId(Uuid::new_v4()));
        let member = MemberId(Uuid::new_v4());
        let (_rx, conn, _) = hub.register(ws, member);
        remote(&hub, Uuid::new_v4(), ws, member, PresenceEventKind::Away);
        while rx.try_recv().is_ok() {}
        drop(conn);
        let frame = rx.try_recv().expect("the close is announced");
        assert!(frame.contains("\"away\""), "{frame}");
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Away));
    }

    /// One replica going offline leaves the member on the others; the
    /// member is online while any replica says online.
    #[test]
    fn an_offline_from_one_replica_keeps_the_member_on_the_others() {
        let hub = distributed();
        let ws = WorkspaceId(Uuid::new_v4());
        let (mut rx, _watching, _) = hub.register(ws, MemberId(Uuid::new_v4()));
        let member = MemberId(Uuid::new_v4());
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        remote(&hub, a, ws, member, PresenceEventKind::Away);
        remote(&hub, b, ws, member, PresenceEventKind::Online);
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Online));
        while rx.try_recv().is_ok() {}

        remote(&hub, b, ws, member, PresenceEventKind::Offline);
        let frame = rx.try_recv().expect("the merged status changed");
        assert!(frame.contains("\"away\""), "{frame}");
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Away));

        remote(&hub, b, ws, member, PresenceEventKind::Offline);
        assert!(rx.try_recv().is_err(), "a repeated offline changed nothing");
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Away));

        remote(&hub, a, ws, member, PresenceEventKind::Offline);
        let frame = rx.try_recv().expect("the last replica is done");
        assert!(frame.contains("\"offline\""), "{frame}");
        assert_eq!(hub.status(ws, member), None);
        assert!(hub.inner.read().unwrap().remote.is_empty());
    }

    /// An expired entry is swept alone; another replica's live entry keeps
    /// the member present.
    #[test]
    fn the_sweep_expires_one_replica_at_a_time() {
        let hub = distributed();
        let ws = WorkspaceId(Uuid::new_v4());
        let (mut rx, _watching, _) = hub.register(ws, MemberId(Uuid::new_v4()));
        let member = MemberId(Uuid::new_v4());
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        remote(&hub, a, ws, member, PresenceEventKind::Online);
        remote(&hub, b, ws, member, PresenceEventKind::Away);
        let long_ago = Instant::now() - Duration::from_secs(7200);
        hub.inner
            .write()
            .unwrap()
            .remote
            .get_mut(&ws)
            .unwrap()
            .get_mut(&member)
            .unwrap()
            .get_mut(&a)
            .unwrap()
            .last_seen = long_ago;
        while rx.try_recv().is_ok() {}
        hub.sweep_expired_remote();
        let frame = rx.try_recv().expect("the merged status changed");
        assert!(frame.contains("\"away\""), "{frame}");
        assert_eq!(hub.status(ws, member), Some(PresenceStatus::Away));
    }

    /// A notifier whose first publish never finishes.
    struct StallsOnce {
        stalled: AtomicBool,
        inner: InMemoryPresenceNotifier,
    }

    #[async_trait::async_trait]
    impl PresenceNotifier for StallsOnce {
        async fn publish_presence(&self, event: PresenceEvent) -> Result<(), maidan_bus::BusError> {
            if !self.stalled.swap(true, Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            self.inner.publish_presence(event).await
        }

        fn subscribe(&self) -> broadcast::Receiver<PresenceEvent> {
            self.inner.subscribe()
        }
    }

    /// A publish that hangs is abandoned; the changes queued behind it go out.
    #[tokio::test]
    async fn a_stalled_publish_does_not_hold_up_the_queue() {
        let notifier = Arc::new(StallsOnce {
            stalled: AtomicBool::new(false),
            inner: InMemoryPresenceNotifier::new(),
        });
        let mut heard = notifier.subscribe();
        let hub = distributed();
        let (tx, rx) = mpsc::channel(8);
        spawn_publisher(
            notifier,
            rx,
            Arc::downgrade(&hub.inner),
            hub.origin,
            hub.heartbeat_queued.clone(),
            Duration::from_millis(50),
        );
        let event = |kind| {
            Outgoing::Change(PresenceEvent {
                origin: hub.origin,
                workspace_id: Uuid::new_v4(),
                member_id: Uuid::new_v4(),
                heartbeat: false,
                kind,
            })
        };
        tx.send(event(PresenceEventKind::Online)).await.unwrap();
        tx.send(event(PresenceEventKind::Away)).await.unwrap();
        let next = tokio::time::timeout(Duration::from_secs(2), heard.recv())
            .await
            .expect("the queue drained past the stalled publish")
            .unwrap();
        assert_eq!(next.kind, PresenceEventKind::Away);
    }

    /// A replica's own events coming back from the notifier are not
    /// delivered again: its subscribers had the frame when it happened.
    #[tokio::test]
    async fn own_events_are_not_delivered_twice() {
        let hub =
            PresenceHub::new().with_presence_notifier(Arc::new(InMemoryPresenceNotifier::new()));
        let ws = WorkspaceId(Uuid::new_v4());
        let (mut rx, _reg, _) = hub.register(ws, MemberId(Uuid::new_v4()));
        let member = MemberId(Uuid::new_v4());
        let (_rx2, _reg2, _) = hub.register(ws, member);
        assert!(rx.try_recv().unwrap().contains(&member.0.to_string()));
        hub.apply_remote_event(PresenceEvent {
            origin: hub.origin,
            workspace_id: ws.0,
            member_id: member.0,
            heartbeat: false,
            kind: PresenceEventKind::Online,
        });
        assert!(rx.try_recv().is_err(), "own event delivered twice");
    }
}

/// Loom models of the hub: every interleaving of its critical sections
/// against each other. The property is the one subscribers rely on: after
/// the dust settles, the last frame a subscriber got about a member, and the
/// last event queued for the other replicas, match the member's state. Run
/// with `cargo test -p maidan-server --features loom --release --lib loom`.
#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::*;
    use loom::thread;
    use maidan_bus::InMemoryPresenceNotifier;

    fn ids() -> (WorkspaceId, MemberId, MemberId) {
        (
            WorkspaceId(Uuid::from_u128(1)),
            MemberId(Uuid::from_u128(2)),
            MemberId(Uuid::from_u128(3)),
        )
    }

    /// The statuses `rx` was told about `member`, in order.
    fn heard(rx: &mut broadcast::Receiver<String>, member: MemberId) -> Vec<String> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|frame| {
                let frame: serde_json::Value = serde_json::from_str(&frame).ok()?;
                (frame["type"] == "presence" && frame["member_id"] == member.0.to_string())
                    .then(|| frame["status"].as_str().map(str::to_owned))
                    .flatten()
            })
            .collect()
    }

    fn state(hub: &PresenceHub, ws: WorkspaceId, member: MemberId) -> &'static str {
        hub.status(ws, member)
            .map_or("offline", PresenceStatus::as_str)
    }

    /// The last event the publisher would send about `member`.
    fn last_queued(hub: &PresenceHub, member: MemberId) -> Option<&'static str> {
        let mut outbox = hub.outbox_rx.lock().unwrap().take()?;
        std::iter::from_fn(|| outbox.try_recv().ok())
            .flat_map(|item| match item {
                Outgoing::Change(e) => vec![e],
                Outgoing::Heartbeat => heartbeat_events(&hub.inner, hub.origin),
            })
            .filter(|e| e.member_id == member.0)
            .last()
            .map(|e| match e.kind {
                PresenceEventKind::Online => "online",
                PresenceEventKind::Away => "away",
                PresenceEventKind::Offline => "offline",
                PresenceEventKind::Typing { .. } => "typing",
            })
    }

    /// A member reconnects while its old connection closes: whichever
    /// order the two take, subscribers (and the other replicas) end on the
    /// member's real state, never a stale `offline` for a connected member.
    fn reconnect(distributed: bool) {
        loom::model(move || {
            let hub = if distributed {
                PresenceHub::new().with_presence_notifier(Arc::new(InMemoryPresenceNotifier::new()))
            } else {
                PresenceHub::new()
            };
            let (ws, member, watcher) = ids();
            let (mut rx, _watching, _) = hub.register(ws, watcher);
            let (_old_rx, old, _) = hub.register(ws, member);

            let closing = thread::spawn(move || drop(old));
            let opening = {
                let hub = hub.clone();
                thread::spawn(move || hub.register(ws, member))
            };
            closing.join().unwrap();
            let (_new_rx, _new, _) = opening.join().unwrap();

            assert_eq!(state(&hub, ws, member), "online");
            assert_eq!(
                heard(&mut rx, member).last().map(String::as_str),
                Some("online")
            );
            if distributed {
                assert_eq!(last_queued(&hub, member), Some("online"));
            }
        });
    }

    #[test]
    fn loom_a_reconnect_never_leaves_a_member_offline() {
        reconnect(false);
    }

    #[test]
    fn loom_a_reconnect_never_leaves_a_member_offline_across_replicas() {
        reconnect(true);
    }

    /// Two status changes race: subscribers and the queue end on the status
    /// the member was left with.
    #[test]
    fn loom_racing_status_changes_end_on_the_final_status() {
        loom::model(|| {
            let hub = PresenceHub::new()
                .with_presence_notifier(Arc::new(InMemoryPresenceNotifier::new()));
            let (ws, member, watcher) = ids();
            let (mut rx, _watching, _) = hub.register(ws, watcher);
            let (_member_rx, _member, _) = hub.register(ws, member);
            let _ = heard(&mut rx, member);

            let away = {
                let hub = hub.clone();
                thread::spawn(move || hub.set_presence(ws, member, PresenceStatus::Away))
            };
            let back = {
                let hub = hub.clone();
                thread::spawn(move || hub.set_presence(ws, member, PresenceStatus::Online))
            };
            away.join().unwrap();
            back.join().unwrap();

            let now = state(&hub, ws, member);
            let frames = heard(&mut rx, member);
            if let Some(last) = frames.last() {
                assert_eq!(last, now);
            }
            if let Some(last) = last_queued(&hub, member) {
                assert_eq!(last, now);
            }
        });
    }

    /// Another replica's member is swept as expired while a fresh heartbeat
    /// for it arrives: subscribers end on what the hub then believes.
    #[test]
    fn loom_a_sweep_racing_a_heartbeat_ends_on_the_hub_view() {
        loom::model(|| {
            let mut hub = PresenceHub::new()
                .with_presence_notifier(Arc::new(InMemoryPresenceNotifier::new()));
            hub.ttl = Duration::from_secs(3600);
            let (ws, member, watcher) = ids();
            let (mut rx, _watching, _) = hub.register(ws, watcher);
            let long_ago = Instant::now()
                .checked_sub(Duration::from_secs(7200))
                .unwrap_or_else(Instant::now);
            let other_replica = Uuid::from_u128(9);
            hub.inner
                .write()
                .unwrap()
                .remote
                .entry(ws)
                .or_default()
                .entry(member)
                .or_default()
                .insert(
                    other_replica,
                    RemoteMember {
                        status: PresenceStatus::Online,
                        last_seen: long_ago,
                    },
                );

            let sweep = {
                let hub = hub.clone();
                thread::spawn(move || hub.sweep_expired_remote())
            };
            let beat = {
                let hub = hub.clone();
                thread::spawn(move || {
                    hub.apply_remote_event(PresenceEvent {
                        origin: other_replica,
                        workspace_id: ws.0,
                        member_id: member.0,
                        heartbeat: true,
                        kind: PresenceEventKind::Online,
                    })
                })
            };
            sweep.join().unwrap();
            beat.join().unwrap();

            let now = state(&hub, ws, member);
            if let Some(last) = heard(&mut rx, member).last() {
                assert_eq!(last, now);
            }
        });
    }
}
