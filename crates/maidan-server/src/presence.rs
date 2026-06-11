//! Workspace presence and typing fan-out for WebSocket subscribers.
//!
//! Single-process by default. When a [`maidan_bus::PresenceNotifier`] is wired
//! (Cluster 103), presence/typing fan out **across replicas**: every local
//! change is published as a [`PresenceEvent`], each replica's listener delivers
//! it to its own WebSocket subscribers, and presence state is folded into a
//! merged, TTL-expiring roster so `presence_snapshot` reflects members on any
//! replica. A periodic heartbeat re-announces locally-connected members so a
//! crashed replica's members expire elsewhere within the TTL.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

use maidan_bus::{PresenceEvent, PresenceEventKind, PresenceNotifier};
use maidan_types::{MemberId, ThreadId, WorkspaceId};
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

const EPHEMERAL_CAPACITY: usize = 256;
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

/// A member known to be present on another replica (via cross-pod events).
#[derive(Debug)]
struct RemoteMember {
    status: PresenceStatus,
    last_seen: Instant,
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
    /// Members present on *other* replicas, kept fresh by heartbeats and
    /// expired by TTL. Tracked even for workspaces with no local room so a new
    /// local subscriber's snapshot includes them.
    remote: HashMap<WorkspaceId, HashMap<MemberId, RemoteMember>>,
}

#[derive(Clone)]
pub struct PresenceHub {
    inner: Arc<RwLock<Inner>>,
    /// This replica's id; stamped on published events so the listener can skip
    /// its own members in the remote view while still delivering locally.
    origin: Uuid,
    notifier: Option<Arc<dyn PresenceNotifier>>,
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
            origin: Uuid::new_v4(),
            notifier: None,
            ttl: Duration::from_secs(ttl_secs_from_env()),
            heartbeat: Duration::from_secs(heartbeat_secs_from_env()),
        }
    }

    /// Wire cross-replica presence fan-out (Cluster 103). Call
    /// [`PresenceHub::spawn_tasks`] afterwards to start the listener + heartbeat.
    pub fn with_presence_notifier(mut self, notifier: Arc<dyn PresenceNotifier>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Start the cross-replica listener and heartbeat/TTL-sweep tasks. No-op
    /// when no notifier is wired (single-process mode).
    pub fn spawn_tasks(&self) {
        let Some(notifier) = self.notifier.clone() else {
            return;
        };
        self.spawn_listener(notifier);
        self.spawn_heartbeat();
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

    pub fn register(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> (broadcast::Receiver<String>, PresenceRegistration, String) {
        let conn_id = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
        let distributed = self.notifier.is_some();
        let (rx, snapshot, first_conn) = {
            let mut inner = self.inner.write().expect("presence lock");
            let first_conn = {
                let room =
                    inner
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
                let first_conn = entry.connections == 0;
                entry.connections += 1;
                entry.status = PresenceStatus::Online;
                // Single-process: announce arrival to existing subscribers
                // *before* this connection's own receiver exists, so a
                // registrant never receives its own online frame. The
                // distributed path publishes after the lock (below).
                if first_conn && !distributed {
                    let _ = room.tx.send(presence_payload(
                        workspace_id,
                        member_id,
                        PresenceStatus::Online,
                    ));
                }
                first_conn
            };
            let rx = inner
                .workspaces
                .get(&workspace_id)
                .expect("room just inserted")
                .tx
                .subscribe();
            let snapshot = build_snapshot(workspace_id, &inner, self.ttl, Instant::now());
            (rx, snapshot, first_conn)
        };
        if first_conn && distributed {
            self.publish_to_notifier(workspace_id, member_id, PresenceStatus::Online.event_kind());
        }
        let reg = PresenceRegistration {
            conn_id,
            workspace_id,
            member_id,
            hub: self.clone(),
        };
        (rx, reg, snapshot)
    }

    fn unregister(&self, _conn_id: u64, workspace_id: WorkspaceId, member_id: MemberId) {
        let last_conn = {
            let mut inner = self.inner.write().expect("presence lock");
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
                true
            } else {
                false
            }
        };
        if last_conn {
            self.announce(workspace_id, member_id, PresenceEventKind::Offline, || {
                offline_payload(workspace_id, member_id)
            });
        }
    }

    pub fn set_presence(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        status: PresenceStatus,
    ) -> bool {
        {
            let mut inner = self.inner.write().expect("presence lock");
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
        }
        self.announce(workspace_id, member_id, status.event_kind(), || {
            presence_payload(workspace_id, member_id, status)
        });
        true
    }

    pub fn set_typing(
        &self,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        member_id: MemberId,
        active: bool,
    ) {
        self.announce(
            workspace_id,
            member_id,
            PresenceEventKind::Typing {
                thread_id: thread_id.0,
                active,
            },
            || typing_payload(workspace_id, thread_id, member_id, active),
        );
    }

    /// Deliver a local change. With a notifier, publish it cross-replica and let
    /// the listener fan out (single delivery path). Without one, broadcast to
    /// the local room directly (legacy single-process behavior).
    fn announce(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        kind: PresenceEventKind,
        local_frame: impl FnOnce() -> String,
    ) {
        if self.notifier.is_some() {
            self.publish_to_notifier(workspace_id, member_id, kind);
        } else {
            self.broadcast_local(workspace_id, local_frame());
        }
    }

    /// Publish a (non-heartbeat) change to the cross-replica notifier. No-op
    /// without a notifier. Fire-and-forget: the async publish runs on the
    /// current runtime; the listener handles delivery (incl. this replica).
    fn publish_to_notifier(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        kind: PresenceEventKind,
    ) {
        let Some(notifier) = self.notifier.clone() else {
            return;
        };
        let event = PresenceEvent {
            origin: self.origin,
            workspace_id: workspace_id.0,
            member_id: member_id.0,
            heartbeat: false,
            kind,
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = notifier.publish_presence(event).await;
            });
        }
    }

    /// Apply a cross-replica event: update the merged remote view (for events
    /// from other replicas) and fan the frame out to local subscribers.
    fn apply_remote_event(&self, event: PresenceEvent) {
        let workspace_id = WorkspaceId(event.workspace_id);
        let member_id = MemberId(event.member_id);
        let from_self = event.origin == self.origin;

        let frame = match &event.kind {
            PresenceEventKind::Typing { thread_id, active } => {
                typing_payload(workspace_id, ThreadId(*thread_id), member_id, *active)
            }
            PresenceEventKind::Offline => offline_payload(workspace_id, member_id),
            kind => {
                let status =
                    PresenceStatus::from_event_kind(kind).unwrap_or(PresenceStatus::Online);
                presence_payload(workspace_id, member_id, status)
            }
        };

        // Fan out to local subscribers only on an actual change. Heartbeats
        // refresh the remote TTL silently; a status that genuinely changed (or a
        // newly-seen member) still propagates.
        let changed = if from_self {
            // Own members: deliver real local changes; suppress heartbeats.
            !event.heartbeat
        } else {
            let mut inner = self.inner.write().expect("presence lock");
            match &event.kind {
                PresenceEventKind::Typing { .. } => true,
                PresenceEventKind::Offline => inner
                    .remote
                    .get_mut(&workspace_id)
                    .map(|members| members.remove(&member_id).is_some())
                    .unwrap_or(false),
                kind => {
                    let status =
                        PresenceStatus::from_event_kind(kind).unwrap_or(PresenceStatus::Online);
                    let members = inner.remote.entry(workspace_id).or_default();
                    let changed = members.get(&member_id).map(|rm| rm.status) != Some(status);
                    members.insert(
                        member_id,
                        RemoteMember {
                            status,
                            last_seen: Instant::now(),
                        },
                    );
                    changed
                }
            }
        };

        if changed {
            self.broadcast_local(workspace_id, frame);
        }
    }

    /// Re-announce this replica's locally-connected members so other replicas
    /// refresh their TTL for them.
    fn heartbeat_local_members(&self) {
        let beats: Vec<(WorkspaceId, MemberId, PresenceStatus)> = {
            let inner = self.inner.read().expect("presence lock");
            inner
                .workspaces
                .iter()
                .flat_map(|(ws, room)| {
                    room.members
                        .iter()
                        .map(move |(mid, st)| (*ws, *mid, st.status))
                })
                .collect()
        };
        let Some(notifier) = self.notifier.clone() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let origin = self.origin;
        handle.spawn(async move {
            for (ws, mid, status) in beats {
                let _ = notifier
                    .publish_presence(PresenceEvent {
                        origin,
                        workspace_id: ws.0,
                        member_id: mid.0,
                        heartbeat: true,
                        kind: status.event_kind(),
                    })
                    .await;
            }
        });
    }

    /// Drop remote members whose last heartbeat is older than the TTL, emitting
    /// an offline frame to any local subscribers.
    fn sweep_expired_remote(&self) {
        let now = Instant::now();
        let ttl = self.ttl;
        let expired: Vec<(WorkspaceId, MemberId)> = {
            let mut inner = self.inner.write().expect("presence lock");
            let mut expired = Vec::new();
            for (ws, members) in inner.remote.iter_mut() {
                members.retain(|mid, rm| {
                    let keep = now.duration_since(rm.last_seen) <= ttl;
                    if !keep {
                        expired.push((*ws, *mid));
                    }
                    keep
                });
            }
            inner.remote.retain(|_, members| !members.is_empty());
            expired
        };
        for (ws, mid) in expired {
            self.broadcast_local(ws, offline_payload(ws, mid));
        }
    }

    fn broadcast_local(&self, workspace_id: WorkspaceId, payload: String) {
        if let Some(room) = self
            .inner
            .read()
            .expect("presence lock")
            .workspaces
            .get(&workspace_id)
        {
            let _ = room.tx.send(payload);
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
        for (id, rm) in members {
            if now.duration_since(rm.last_seen) <= ttl {
                merged.insert(id.0, rm.status.as_str().to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_bus::InMemoryPresenceNotifier;

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
        inner.remote.entry(ws).or_default().insert(
            remote_id,
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
        inner.remote.entry(ws).or_default().insert(
            remote_id,
            RemoteMember {
                status: PresenceStatus::Online,
                last_seen: Instant::now() - Duration::from_secs(120),
            },
        );
        let json = build_snapshot(ws, &inner, Duration::from_secs(30), Instant::now());
        assert!(!json.contains(&remote_id.0.to_string()));
    }

    #[tokio::test]
    async fn presence_fans_out_to_another_hub_over_shared_notifier() {
        let notifier = Arc::new(InMemoryPresenceNotifier::new());
        let hub_a = PresenceHub::new().with_presence_notifier(notifier.clone());
        let hub_b = PresenceHub::new().with_presence_notifier(notifier.clone());
        hub_b.spawn_tasks();

        let ws = WorkspaceId(Uuid::new_v4());
        let member = MemberId(Uuid::new_v4());

        // A local subscriber on hub B for this workspace.
        let (mut rx_b, _reg_b, _snap) = hub_b.register(ws, MemberId(Uuid::new_v4()));

        // A member comes online on hub A → published → hub B's listener delivers.
        let (_rx_a, _reg_a, _snap_a) = hub_a.register(ws, member);

        // B may first echo its own subscriber's join (single delivery path), so
        // drain until A's member's presence frame arrives.
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
}
