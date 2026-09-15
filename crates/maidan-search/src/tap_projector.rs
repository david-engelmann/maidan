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
#[derive(Debug, Default)]
pub struct SearchTap {
    last_link: HashMap<WorkspaceId, EventLink>,
    pub history_hw: i64,
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

    /// Verify `row` continues this workspace's chain. Returns whether
    /// the handler should project it (message posted/edited/tombstoned).
    pub fn ingest(&mut self, row: &StoredEvent) -> Result<bool, TapFault> {
        if let Some(fault) = &self.fault {
            return Err(fault.clone());
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
            self.fault = Some(fault.clone());
            return Err(fault);
        }
        if let Some(ws) = row.workspace_id {
            self.last_link.insert(ws, row.link());
        }
        self.history_hw = self.history_hw.max(row.id);
        Ok(Self::is_search_kind(row.kind))
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
        let err = tap2.ingest(&broken).unwrap_err();
        assert!(err.search_must_rebuild());
        assert!(matches!(err, TapFault::ChainBreak { .. }));
        assert!(
            tap2.ingest(&e2).is_err(),
            "fault sticks; no more projecting"
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
