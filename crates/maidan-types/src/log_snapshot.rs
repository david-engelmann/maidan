//! Snapshot + since-LSN catch-up (Cluster 393, Wave 3 #33 B18).
//!
//! ATProto `getRepo`-shaped, **not** MST/CAR. A peer that missed a pruned
//! prefix takes a verified domain-graph checkpoint at the retained floor
//! (or current head) and catches up by walking the Cluster 392 hash chain
//! from that checkpoint's [`EventLink`].
//!
//! Cluster 392 verifies the **retained suffix**. This module covers the
//! **pruned prefix**: the graph *is* the history the log no longer holds.
//! Authorship of a fabricated-but-consistent snapshot is Cluster 391's
//! signed export, not this hash. `$type` is the contract; breaking
//! changes are `/2`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cursor::cursor_is_too_old;
use crate::event_chain::{
    chain_hash, content_hash_of, genesis_hash, verify_link, ChainBreakReason, ChainVerifyReport,
    EventChainError, EventLink, EVENT_CHAIN_ALG,
};
use crate::events::StoredEvent;
use crate::ids::WorkspaceId;
use crate::models::{
    ExportChannel, Member, Message, MessageEdit, Pin, Reference, Thread, Workspace, WorkspaceExport,
};

/// Observable `$type` for a log snapshot. Breaking changes are `/2`.
pub const LOG_SNAPSHOT_TYPE: &str = "maidan.event-log.snapshot/1";

/// Observable `$type` for a since-LSN catch-up page.
pub const CATCH_UP_TYPE: &str = "maidan.event-log.catch-up/1";

/// Domain graph committed by a snapshot. Cluster 187/391 export minus
/// `format_version` / `exported_at` so two snapshots of the same tables
/// hash the same.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotGraph {
    pub workspace: Workspace,
    pub members: Vec<Member>,
    pub channels: Vec<ExportChannel>,
    pub threads: Vec<Thread>,
    pub messages: Vec<Message>,
    pub message_edits: Vec<MessageEdit>,
    pub pins: Vec<Pin>,
    pub references: Vec<Reference>,
}

impl From<WorkspaceExport> for SnapshotGraph {
    fn from(export: WorkspaceExport) -> Self {
        Self {
            workspace: export.workspace,
            members: export.members,
            channels: export.channels,
            threads: export.threads,
            messages: export.messages,
            message_edits: export.message_edits,
            pins: export.pins,
            references: export.references,
        }
    }
}

impl SnapshotGraph {
    pub fn hash(&self) -> Result<String, EventChainError> {
        content_hash_of(self)
    }
}

/// Verified checkpoint a peer uses instead of a pruned prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogSnapshot {
    #[serde(rename = "$type")]
    pub type_id: String,
    pub workspace_id: WorkspaceId,
    /// Workspace head included in this snapshot (`0` if the log is empty).
    pub as_of_lsn: i64,
    /// Oldest retained event id (`0` if empty). Catch-up after a cursor
    /// strictly behind this floor is [`crate::CursorTooOld`].
    pub floor_lsn: i64,
    pub from_genesis: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<EventLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<EventLink>,
    /// SHA-256 of canonical JSON of [`SnapshotGraph`] (no `exported_at`).
    pub graph_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub graph: Option<SnapshotGraph>,
}

impl LogSnapshot {
    pub fn new(
        workspace_id: WorkspaceId,
        floor: Option<EventLink>,
        head: Option<EventLink>,
        graph: SnapshotGraph,
        include_graph: bool,
    ) -> Result<Self, EventChainError> {
        let graph_hash = graph.hash()?;
        let from_genesis = match &floor {
            None => true,
            Some(link) => link.prev_hash == genesis_hash(),
        };
        Ok(Self {
            type_id: LOG_SNAPSHOT_TYPE.to_string(),
            workspace_id,
            as_of_lsn: head.as_ref().map(|h| h.lsn).unwrap_or(0),
            floor_lsn: floor.as_ref().map(|f| f.id).unwrap_or(0),
            from_genesis,
            head,
            floor,
            graph_hash,
            graph: include_graph.then_some(graph),
        })
    }

    /// REST path a 409 `must_refetch` points at.
    pub fn path(workspace_id: WorkspaceId) -> String {
        format!("/workspaces/{}/snapshot", workspace_id.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SnapshotBreakReason {
    GraphHashMismatch,
    HeadLsnMismatch,
    FloorLsnMismatch,
    MalformedHash,
}

impl SnapshotBreakReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GraphHashMismatch => "graph_hash_mismatch",
            Self::HeadLsnMismatch => "head_lsn_mismatch",
            Self::FloorLsnMismatch => "floor_lsn_mismatch",
            Self::MalformedHash => "malformed_hash",
        }
    }
}

/// Fail-closed snapshot check. Content is hashed; the host is not trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SnapshotVerifyReport {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<SnapshotBreakReason>,
}

/// Walk `snapshot` without trusting the host. Missing `graph` still checks
/// envelope consistency (lsn vs links, well-formed hash).
pub fn verify_snapshot(snapshot: &LogSnapshot) -> SnapshotVerifyReport {
    if snapshot.type_id != LOG_SNAPSHOT_TYPE {
        return SnapshotVerifyReport {
            ok: false,
            reason: Some(SnapshotBreakReason::MalformedHash),
        };
    }
    if !snapshot.graph_hash.starts_with("sha256:") {
        return SnapshotVerifyReport {
            ok: false,
            reason: Some(SnapshotBreakReason::MalformedHash),
        };
    }
    let as_of = snapshot.head.as_ref().map(|h| h.lsn).unwrap_or(0);
    if snapshot.as_of_lsn != as_of {
        return SnapshotVerifyReport {
            ok: false,
            reason: Some(SnapshotBreakReason::HeadLsnMismatch),
        };
    }
    let floor = snapshot.floor.as_ref().map(|f| f.id).unwrap_or(0);
    if snapshot.floor_lsn != floor {
        return SnapshotVerifyReport {
            ok: false,
            reason: Some(SnapshotBreakReason::FloorLsnMismatch),
        };
    }
    if let Some(graph) = &snapshot.graph {
        match graph.hash() {
            Ok(hash) if hash == snapshot.graph_hash => {}
            Ok(_) => {
                return SnapshotVerifyReport {
                    ok: false,
                    reason: Some(SnapshotBreakReason::GraphHashMismatch),
                };
            }
            Err(_) => {
                return SnapshotVerifyReport {
                    ok: false,
                    reason: Some(SnapshotBreakReason::MalformedHash),
                };
            }
        }
    }
    SnapshotVerifyReport {
        ok: true,
        reason: None,
    }
}

/// One page of events after a snapshot (or after a prior page).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatchUpPage {
    #[serde(rename = "$type")]
    pub type_id: String,
    pub workspace_id: WorkspaceId,
    /// Exclusive cursor: events have `id > after_lsn`.
    pub after_lsn: i64,
    /// Workspace head at page build (`0` if empty). Live waits until
    /// `events.last.id` (or `after_lsn` when empty) >= this.
    pub head_lsn: i64,
    /// Global room head (Cluster 390 `Maidan-Room-LSN`). Not a catch-up
    /// cursor — other tenants move it.
    pub room_lsn: i64,
    pub events: Vec<StoredEvent>,
    pub chain: ChainVerifyReport,
    pub truncated: bool,
}

impl CatchUpPage {
    pub fn new(
        workspace_id: WorkspaceId,
        after_lsn: i64,
        head_lsn: i64,
        room_lsn: i64,
        events: Vec<StoredEvent>,
        previous: Option<&EventLink>,
        truncated: bool,
    ) -> Self {
        let chain = verify_catch_up(previous, &events);
        Self {
            type_id: CATCH_UP_TYPE.to_string(),
            workspace_id,
            after_lsn,
            head_lsn,
            room_lsn,
            events,
            chain,
            truncated,
        }
    }

    pub fn ok(&self) -> bool {
        self.chain.ok
    }
}

/// Whether `after_lsn` can catch up from the retained floor. Same rule as
/// Cluster 388 — a pruned-gap cursor must refetch a snapshot, never clamp.
pub fn catch_up_allowed(after_lsn: i64, floor_lsn: Option<i64>) -> bool {
    !cursor_is_too_old(after_lsn, floor_lsn)
}

/// Verify `events` continue from `previous` (the snapshot head or the last
/// event of the prior page). Empty is ok. Does not treat a retained floor
/// as genesis unless `previous` is `None` and the first row starts there.
pub fn verify_catch_up(previous: Option<&EventLink>, events: &[StoredEvent]) -> ChainVerifyReport {
    let links: Vec<EventLink> = events.iter().map(StoredEvent::link).collect();
    let payloads: Vec<Value> = events.iter().map(|e| e.payload.clone()).collect();
    verify_chain_from(previous, &links, &payloads)
}

/// Walk `links` chained from an optional predecessor. Used by catch-up;
/// Cluster 392 `verify_chain` stays the retained-suffix walker (no
/// predecessor).
pub fn verify_chain_from(
    previous: Option<&EventLink>,
    links: &[EventLink],
    payloads: &[Value],
) -> ChainVerifyReport {
    let genesis = genesis_hash();
    if links.len() != payloads.len() {
        return ChainVerifyReport {
            ok: false,
            algorithm: EVENT_CHAIN_ALG.to_string(),
            genesis,
            checked: 0,
            head: previous.cloned(),
            from_genesis: previous.is_none(),
            break_at: links.first().map(|l| l.id),
            reason: Some(ChainBreakReason::MalformedHash),
        };
    }
    if links.is_empty() {
        return ChainVerifyReport {
            ok: true,
            algorithm: EVENT_CHAIN_ALG.to_string(),
            genesis,
            checked: 0,
            head: previous.cloned(),
            from_genesis: previous.is_none(),
            break_at: None,
            reason: None,
        };
    }

    let from_genesis = previous.is_none() && links[0].prev_hash == genesis;
    let mut prev = previous;
    for (i, (link, payload)) in links.iter().zip(payloads.iter()).enumerate() {
        let floor_genesis = previous.is_none() && from_genesis && i == 0;
        if let Err(reason) = verify_link(link, payload, prev, floor_genesis) {
            return ChainVerifyReport {
                ok: false,
                algorithm: EVENT_CHAIN_ALG.to_string(),
                genesis,
                checked: i as u32,
                head: prev.cloned(),
                from_genesis,
                break_at: Some(link.id),
                reason: Some(reason),
            };
        }
        prev = Some(link);
    }
    ChainVerifyReport {
        ok: true,
        algorithm: EVENT_CHAIN_ALG.to_string(),
        genesis,
        checked: links.len() as u32,
        head: links.last().cloned(),
        from_genesis,
        break_at: None,
        reason: None,
    }
}

/// Expected `prev_hash` of the first catch-up event after `head`.
pub fn catch_up_prev_hash(head: Option<&EventLink>) -> String {
    match head {
        None => genesis_hash(),
        Some(link) => chain_hash(&link.prev_hash, &link.content_hash, link.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_chain::{link_for, verify_chain};
    use crate::ids::MemberId;
    use crate::models::MemberKind;
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn payload(n: u32) -> Value {
        json!({"kind": "message_posted", "n": n})
    }

    fn stored(link: EventLink, payload: Value) -> StoredEvent {
        StoredEvent {
            id: link.id,
            lsn: link.lsn,
            kind: crate::EventKind::MessagePosted,
            workspace_id: Some(WorkspaceId(Uuid::from_u128(1))),
            channel_id: None,
            thread_id: None,
            payload,
            occurred_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            prev_hash: link.prev_hash,
            content_hash: link.content_hash,
        }
    }

    fn graph() -> SnapshotGraph {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let ws = Workspace {
            id: WorkspaceId(Uuid::from_u128(1)),
            name: "room".into(),
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        };
        SnapshotGraph {
            workspace: ws.clone(),
            members: vec![Member {
                id: MemberId(Uuid::from_u128(2)),
                workspace_id: ws.id,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Human,
                created_at: now,
                updated_at: now,
                tombstoned_at: None,
            }],
            channels: vec![],
            threads: vec![],
            messages: vec![],
            message_edits: vec![],
            pins: vec![],
            references: vec![],
        }
    }

    #[test]
    fn snapshot_hashes_without_exported_at() {
        let g = graph();
        let h1 = g.hash().unwrap();
        let mut export = WorkspaceExport {
            format_version: 1,
            exported_at: Utc::now(),
            workspace: g.workspace.clone(),
            members: g.members.clone(),
            channels: g.channels.clone(),
            threads: g.threads.clone(),
            messages: g.messages.clone(),
            message_edits: g.message_edits.clone(),
            pins: g.pins.clone(),
            references: g.references.clone(),
        };
        let g2 = SnapshotGraph::from(export.clone());
        assert_eq!(g2.hash().unwrap(), h1);
        export.exported_at = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        assert_eq!(SnapshotGraph::from(export).hash().unwrap(), h1);
    }

    #[test]
    fn snapshot_verify_ok_and_tamper() {
        let g = graph();
        let p = payload(1);
        let head = link_for(4, &p, None).unwrap();
        let snap = LogSnapshot::new(
            WorkspaceId(Uuid::from_u128(1)),
            Some(head.clone()),
            Some(head.clone()),
            g.clone(),
            true,
        )
        .unwrap();
        assert_eq!(snap.type_id, LOG_SNAPSHOT_TYPE);
        assert_eq!(snap.as_of_lsn, 4);
        assert_eq!(snap.floor_lsn, 4);
        assert!(snap.from_genesis);
        assert!(verify_snapshot(&snap).ok);

        let mut bad = snap.clone();
        bad.graph.as_mut().unwrap().workspace.name = "nope".into();
        let report = verify_snapshot(&bad);
        assert!(!report.ok);
        assert_eq!(report.reason, Some(SnapshotBreakReason::GraphHashMismatch));

        let header_only = LogSnapshot::new(
            WorkspaceId(Uuid::from_u128(1)),
            Some(head.clone()),
            Some(head),
            g,
            false,
        )
        .unwrap();
        assert!(header_only.graph.is_none());
        assert!(verify_snapshot(&header_only).ok);

        let mut lsn_bad = header_only;
        lsn_bad.as_of_lsn = 99;
        assert_eq!(
            verify_snapshot(&lsn_bad).reason,
            Some(SnapshotBreakReason::HeadLsnMismatch)
        );
    }

    #[test]
    fn catch_up_from_snapshot_head_then_tamper_fails_closed() {
        let p1 = payload(1);
        let p2 = payload(2);
        let e1 = link_for(10, &p1, None).unwrap();
        let e2 = link_for(11, &p2, Some(&e1)).unwrap();
        let s2 = stored(e2.clone(), p2.clone());

        let page = CatchUpPage::new(
            WorkspaceId(Uuid::from_u128(1)),
            10,
            11,
            11,
            vec![s2.clone()],
            Some(&e1),
            false,
        );
        assert_eq!(page.type_id, CATCH_UP_TYPE);
        assert!(page.ok(), "{:?}", page.chain);
        assert_eq!(page.chain.checked, 1);
        assert!(!page.chain.from_genesis);
        assert_eq!(page.chain.head.as_ref(), Some(&e2));

        let mut broken = s2.clone();
        broken.payload = payload(99);
        let bad = verify_catch_up(Some(&e1), std::slice::from_ref(&broken));
        assert!(!bad.ok);
        assert_eq!(bad.reason, Some(ChainBreakReason::ContentHashMismatch));

        let mut skip = s2;
        skip.prev_hash = crate::genesis_hash();
        let skip_report = verify_catch_up(Some(&e1), std::slice::from_ref(&skip));
        assert!(!skip_report.ok);
        assert_eq!(skip_report.reason, Some(ChainBreakReason::PrevHashMismatch));
    }

    #[test]
    fn empty_catch_up_is_ok_at_head() {
        let p = payload(1);
        let head = link_for(5, &p, None).unwrap();
        let report = verify_catch_up(Some(&head), &[]);
        assert!(report.ok);
        assert_eq!(report.checked, 0);
        assert_eq!(report.head.as_ref(), Some(&head));
        assert!(!report.from_genesis);
    }

    #[test]
    fn catch_up_without_previous_matches_verify_chain() {
        let p1 = payload(1);
        let p2 = payload(2);
        let e1 = link_for(1, &p1, None).unwrap();
        let e2 = link_for(2, &p2, Some(&e1)).unwrap();
        let events = vec![
            stored(e1.clone(), p1.clone()),
            stored(e2.clone(), p2.clone()),
        ];
        let from_catch = verify_catch_up(None, &events);
        let from_chain = verify_chain(&[e1, e2], &[p1, p2]);
        assert_eq!(from_catch, from_chain);
        assert!(from_catch.from_genesis);
    }

    #[test]
    fn pruned_gap_is_not_catch_up_allowed() {
        assert!(catch_up_allowed(0, Some(100)));
        assert!(catch_up_allowed(99, Some(100)));
        assert!(!catch_up_allowed(50, Some(100)));
        assert!(catch_up_allowed(50, None));
    }

    #[test]
    fn snapshot_path_is_workspace_scoped() {
        let id = WorkspaceId(Uuid::from_u128(1));
        assert_eq!(
            LogSnapshot::path(id),
            format!("/workspaces/{}/snapshot", id.0)
        );
    }

    #[test]
    fn catch_up_prev_hash_matches_chain() {
        let p = payload(1);
        let e = link_for(3, &p, None).unwrap();
        assert_eq!(catch_up_prev_hash(None), genesis_hash());
        assert_eq!(
            catch_up_prev_hash(Some(&e)),
            chain_hash(&e.prev_hash, &e.content_hash, e.id)
        );
    }
}
