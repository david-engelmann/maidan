//! Ephemeral workspace presence and typing fan-out for WebSocket subscribers.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
};

use maidan_types::{MemberId, ThreadId, WorkspaceId};
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

const EPHEMERAL_CAPACITY: usize = 256;

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

#[derive(Debug)]
struct WorkspaceRoom {
    tx: broadcast::Sender<String>,
    members: HashMap<MemberId, MemberState>,
}

#[derive(Debug, Default)]
struct Inner {
    workspaces: HashMap<WorkspaceId, WorkspaceRoom>,
}

#[derive(Debug, Clone)]
pub struct PresenceHub {
    inner: Arc<RwLock<Inner>>,
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
        }
    }

    pub fn register(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> (broadcast::Receiver<String>, PresenceRegistration, String) {
        let conn_id = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
        let snapshot = {
            let mut inner = self.inner.write().expect("presence lock");
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
            let first_conn = entry.connections == 0;
            entry.connections += 1;
            entry.status = PresenceStatus::Online;
            if first_conn {
                let payload = presence_payload(workspace_id, member_id, PresenceStatus::Online);
                let _ = room.tx.send(payload);
            }
            presence_snapshot_payload(workspace_id, &room.members)
        };
        let rx = self
            .inner
            .read()
            .expect("presence lock")
            .workspaces
            .get(&workspace_id)
            .expect("room just inserted")
            .tx
            .subscribe();
        let reg = PresenceRegistration {
            conn_id,
            workspace_id,
            member_id,
            hub: self.clone(),
        };
        (rx, reg, snapshot)
    }

    fn unregister(&self, _conn_id: u64, workspace_id: WorkspaceId, member_id: MemberId) {
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
        let payload = if entry.connections == 0 {
            room.members.remove(&member_id);
            Some(offline_payload(workspace_id, member_id))
        } else {
            None
        };
        drop(inner);
        if let Some(payload) = payload {
            self.broadcast(workspace_id, payload);
        }
    }

    pub fn set_presence(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        status: PresenceStatus,
    ) -> bool {
        let payload = {
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
            presence_payload(workspace_id, member_id, status)
        };
        self.broadcast(workspace_id, payload);
        true
    }

    pub fn set_typing(
        &self,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        member_id: MemberId,
        active: bool,
    ) {
        let payload = typing_payload(workspace_id, thread_id, member_id, active);
        self.broadcast(workspace_id, payload);
    }

    fn broadcast(&self, workspace_id: WorkspaceId, payload: String) {
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

fn presence_snapshot_payload(
    workspace_id: WorkspaceId,
    members: &HashMap<MemberId, MemberState>,
) -> String {
    let list: Vec<PresenceMember> = members
        .iter()
        .map(|(id, st)| PresenceMember {
            member_id: id.0,
            status: st.status.as_str().to_string(),
        })
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

    #[test]
    fn snapshot_payload_lists_members() {
        let mut members = HashMap::new();
        let id = MemberId(Uuid::new_v4());
        members.insert(
            id,
            MemberState {
                status: PresenceStatus::Away,
                connections: 1,
            },
        );
        let ws = WorkspaceId(Uuid::new_v4());
        let json = presence_snapshot_payload(ws, &members);
        assert!(json.contains("presence_snapshot"));
        assert!(json.contains("away"));
    }
}
