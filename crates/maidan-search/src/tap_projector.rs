//! Search as a tap projector (Cluster 393, Wave 3 #33 B19).
//!
//! The indexer is not the log. It must verify every backfill page, drain
//! history before live, filter to [`SEARCH_PROJECTOR_KINDS`], and fail
//! closed on a gap or chain break — never warn-and-continue with a
//! silently diverged index. Rebuild from the messages table (the
//! domain snapshot) rather than serving a gapped projection.

use std::collections::HashMap;

use maidan_types::{
    cursor_is_too_old, history_caught_up, verify_catch_up, EventKind, EventLink, StoredEvent,
    TapFault, TapSurface, WorkspaceId, SEARCH_PROJECTOR_KINDS,
};

/// Per-workspace chain cursor the search projector walks during backfill.
///
/// **Faults are per workspace** (Cluster 402.1). The chain itself always was —
/// `last_link` is keyed by workspace — but the fault was a single `Option`, so
/// one tenant's break made `ingest` refuse every subsequent row of *every*
/// tenant, and the indexer's retry loop then re-walked the whole log forever
/// behind exponential backoff. One workspace with a broken chain stopped search
/// indexing for the entire instance, which is a far larger blast radius than
/// the failure it was reacting to.
///
/// A faulted workspace stops being projected and stays that way; the others
/// carry on. `MissingHistory` and a pruned-gap cursor are still whole-tap
/// faults — they are statements about the log, not about one tenant.
#[derive(Debug, Default)]
pub struct SearchTap {
    last_link: HashMap<WorkspaceId, EventLink>,
    pub history_hw: i64,
    /// Workspaces whose chain broke. Keyed, not a single flag.
    faults: HashMap<WorkspaceId, TapFault>,
    /// A fault that is not attributable to one workspace (a missing or pruned
    /// log), which does stop the whole tap.
    pub fault: Option<TapFault>,
}

impl SearchTap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn surface() -> TapSurface {
        TapSurface::Search
    }

    pub fn is_search_kind(kind: EventKind) -> bool {
        SEARCH_PROJECTOR_KINDS.contains(&kind)
    }

    /// Verify `row` continues its workspace's chain. Returns whether the handler
    /// should project it (message posted/edited/tombstoned).
    ///
    /// A chain break faults **that workspace** and returns `Ok(false)` from then
    /// on: the row is not projected, the rest of the log keeps flowing, and the
    /// high-water still advances so the tap does not stall the instance over one
    /// tenant. `Err` is reserved for a fault that is not one tenant's — see
    /// [`Self::fault`].
    ///
    /// The break is still loud: [`Self::faulted_workspaces`] names them, and the
    /// caller reports them. Silence would be the actual danger, since a faulted
    /// workspace's index stops advancing while the rest of the room looks
    /// healthy.
    pub fn ingest(&mut self, row: &StoredEvent) -> Result<bool, TapFault> {
        if let Some(fault) = &self.fault {
            return Err(fault.clone());
        }
        // Already broken: keep walking, project nothing for this tenant.
        if let Some(ws) = row.workspace_id {
            if self.faults.contains_key(&ws) {
                self.history_hw = self.history_hw.max(row.id);
                return Ok(false);
            }
        }
        let previous = row.workspace_id.and_then(|ws| self.last_link.get(&ws));
        let report = verify_catch_up(previous, std::slice::from_ref(row));
        if !report.ok {
            let fault = TapFault::ChainBreak {
                break_at: report.break_at,
                reason: report
                    .reason
                    .unwrap_or(maidan_types::ChainBreakReason::MalformedHash),
            };
            match row.workspace_id {
                // Attributable: isolate it.
                Some(ws) => {
                    self.faults.insert(ws, fault);
                    self.history_hw = self.history_hw.max(row.id);
                    return Ok(false);
                }
                // A row with no workspace cannot be isolated, so it is a
                // statement about the log and stops the tap.
                None => {
                    self.fault = Some(fault.clone());
                    return Err(fault);
                }
            }
        }
        if let Some(ws) = row.workspace_id {
            self.last_link.insert(ws, row.link());
        }
        self.history_hw = self.history_hw.max(row.id);
        Ok(Self::is_search_kind(row.kind))
    }

    /// Workspaces whose chain broke, with the fault. Empty is healthy.
    pub fn faulted_workspaces(&self) -> Vec<(WorkspaceId, TapFault)> {
        let mut out: Vec<_> = self.faults.iter().map(|(ws, f)| (*ws, f.clone())).collect();
        out.sort_by_key(|(ws, _)| ws.0);
        out
    }

    /// Whether any workspace is faulted.
    pub fn has_workspace_fault(&self) -> bool {
        !self.faults.is_empty()
    }

    /// A resume cursor in a pruned gap must rebuild, never clamp.
    pub fn gap_fault(
        after_id: i64,
        oldest_id: i64,
        workspace_id: Option<WorkspaceId>,
    ) -> Option<TapFault> {
        if !cursor_is_too_old(after_id, Some(oldest_id)) {
            return None;
        }
        Some(match workspace_id {
            Some(ws) => TapFault::cursor_too_old(after_id, oldest_id, ws),
            None => TapFault::MissingHistory,
        })
    }

    pub fn live_ready(&self, head_lsn: i64) -> bool {
        history_caught_up(self.history_hw, head_lsn)
    }
}

/// Drain the durable log into `tap` + `on_project`. Stops on the first
/// chain break or pruned-gap cursor. Returns the history high-water.
pub async fn backfill_search<F, Fut>(
    store: &dyn maidan_store::Store,
    tap: &mut SearchTap,
    mut on_project: F,
) -> Result<i64, TapFault>
where
    F: FnMut(StoredEvent) -> Fut,
    Fut: std::future::Future<Output = Result<(), TapFault>>,
{
    let oldest = store
        .list_events_after_global(0, 1)
        .await
        .map_err(|_| TapFault::MissingHistory)?;
    if let Some(row) = oldest.first() {
        if let Some(fault) = SearchTap::gap_fault(tap.history_hw, row.id, row.workspace_id) {
            tap.fault = Some(fault.clone());
            return Err(fault);
        }
    }
    let mut after_id = 0_i64;
    loop {
        let page = store
            .list_events_after_global(after_id, maidan_store::LAG_RESUME_BATCH)
            .await
            .map_err(|_| TapFault::MissingHistory)?;
        if page.is_empty() {
            return Ok(tap.history_hw);
        }
        let short = (page.len() as i64) < maidan_store::LAG_RESUME_BATCH;
        for row in page {
            after_id = after_id.max(row.id);
            let project = tap.ingest(&row)?;
            if project {
                on_project(row).await?;
            }
        }
        if short {
            let head = store
                .max_event_id()
                .await
                .map_err(|_| TapFault::MissingHistory)?;
            if tap.live_ready(head) || after_id >= head {
                return Ok(tap.history_hw);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use maidan_types::{link_for, EventKind, WorkspaceId};
    use serde_json::json;
    use uuid::Uuid;

    fn stored(id: i64, payload: serde_json::Value, prev: Option<&EventLink>) -> StoredEvent {
        let link = link_for(id, &payload, prev).unwrap();
        StoredEvent {
            id: link.id,
            lsn: link.lsn,
            kind: EventKind::MessagePosted,
            workspace_id: Some(WorkspaceId(Uuid::from_u128(1))),
            channel_id: None,
            thread_id: None,
            payload,
            occurred_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            prev_hash: link.prev_hash,
            content_hash: link.content_hash,
        }
    }

    #[test]
    fn ingest_ok_then_tamper_fails_closed() {
        let p1 = json!({"kind": "message_posted", "n": 1});
        let p2 = json!({"kind": "message_posted", "n": 2});
        let e1 = stored(1, p1, None);
        let e2 = stored(2, p2, Some(&e1.link()));
        let mut tap = SearchTap::new();
        assert!(tap.ingest(&e1).unwrap());
        assert!(tap.ingest(&e2).unwrap());
        assert!(tap.live_ready(2));

        let mut broken = e2.clone();
        broken.payload = json!({"kind": "message_posted", "n": 99});
        let mut tap2 = SearchTap::new();
        tap2.ingest(&e1).unwrap();
        // Cluster 402.1: a tamper faults *that workspace* rather than the tap.
        // It still refuses to project — a diverged index is the thing we will
        // not serve — but the walk continues so other tenants keep indexing.
        assert!(
            !tap2.ingest(&broken).unwrap(),
            "a broken chain must not be projected"
        );
        assert!(tap2.has_workspace_fault(), "and it must be recorded");
        let (ws, fault) = tap2.faulted_workspaces().into_iter().next().unwrap();
        assert_eq!(ws, WorkspaceId(Uuid::from_u128(1)));
        assert!(fault.search_must_rebuild());
        assert!(matches!(fault, TapFault::ChainBreak { .. }));
        assert!(
            !tap2.ingest(&e2).unwrap(),
            "the fault sticks for that workspace; no more projecting it"
        );
    }

    /// The bug this isolation exists for: one tenant's broken chain used to stop
    /// search indexing for every tenant, because the fault was a single
    /// `Option` while the chain itself was already per-workspace.
    #[test]
    fn a_broken_workspace_does_not_stop_the_others() {
        let ws_a = WorkspaceId(Uuid::from_u128(1));
        let ws_b = WorkspaceId(Uuid::from_u128(2));
        let in_ws = |id: i64, ws: WorkspaceId, n: i64, prev: Option<&EventLink>| {
            let payload = json!({"kind": "message_posted", "n": n});
            let link = link_for(id, &payload, prev).unwrap();
            StoredEvent {
                id: link.id,
                lsn: link.lsn,
                kind: EventKind::MessagePosted,
                workspace_id: Some(ws),
                channel_id: None,
                thread_id: None,
                payload,
                occurred_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
                prev_hash: link.prev_hash,
                content_hash: link.content_hash,
            }
        };

        let mut tap = SearchTap::new();
        let a1 = in_ws(1, ws_a, 1, None);
        let b1 = in_ws(2, ws_b, 1, None);
        assert!(tap.ingest(&a1).unwrap());
        assert!(tap.ingest(&b1).unwrap());

        // A's chain breaks.
        let mut a_broken = in_ws(3, ws_a, 2, Some(&a1.link()));
        a_broken.payload = json!({"kind": "message_posted", "n": 999});
        assert!(!tap.ingest(&a_broken).unwrap());

        // B keeps indexing — this is the whole point.
        let b2 = in_ws(4, ws_b, 2, Some(&b1.link()));
        assert!(
            tap.ingest(&b2).unwrap(),
            "a healthy workspace must keep projecting while another is faulted"
        );
        assert_eq!(
            tap.faulted_workspaces().len(),
            1,
            "only the broken workspace is faulted"
        );
        assert_eq!(tap.faulted_workspaces()[0].0, ws_a);

        // And the high-water still advances, so the tap does not stall the
        // instance waiting on a tenant that cannot recover without a rebuild.
        assert!(
            tap.live_ready(4),
            "history high-water advanced past the break"
        );
    }

    #[test]
    fn pruned_gap_is_rebuild_not_clamp() {
        let ws = WorkspaceId(Uuid::from_u128(1));
        assert!(SearchTap::gap_fault(0, 100, Some(ws)).is_none());
        let fault = SearchTap::gap_fault(50, 100, Some(ws)).unwrap();
        assert!(fault.search_must_rebuild());
        match fault {
            TapFault::CursorTooOld { snapshot, .. } => {
                assert!(snapshot.ends_with("/snapshot"));
            }
            other => panic!("expected CursorTooOld, got {other:?}"),
        }
    }

    #[test]
    fn live_waits_for_workspace_head_not_a_higher_room() {
        let mut tap = SearchTap::new();
        tap.history_hw = 10;
        assert!(tap.live_ready(10));
        assert!(!tap.live_ready(12));
        assert_eq!(SearchTap::surface(), TapSurface::Search);
        assert!(SearchTap::is_search_kind(EventKind::MessagePosted));
        assert!(!SearchTap::is_search_kind(EventKind::WorkspaceCreated));
    }
}
