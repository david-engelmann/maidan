//! The in-memory side of the usage meters. Requests and MCP calls are counted
//! here, under a lock and never with a database write, and the server's
//! `usage_meters` loop flushes the counts to [`crate::Store::add_usage_meters`]
//! in one batch. A process that dies between flushes loses at most one
//! interval's counts: the meters are for an allowance, not for billing.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use chrono::{NaiveDate, Utc};
use maidan_types::{UsageMeter, UsageMeterDay, WorkspaceId};

use crate::error::StoreError;

type Key = (WorkspaceId, NaiveDate, UsageMeter);

/// Counts not yet flushed, per workspace, UTC day and meter.
///
/// It also holds the hard allowance cutoff: the (workspace, meter) pairs an
/// open sign-up workspace is past this month, which the request path, the
/// MCP server and search consult before doing metered work. The usage_meters
/// loop replaces the set each tick from the store.
#[derive(Debug, Default)]
pub struct UsageMeterBuffer {
    counts: Mutex<HashMap<Key, i64>>,
    cut_off: std::sync::RwLock<HashSet<(WorkspaceId, UsageMeter)>>,
}

impl UsageMeterBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `quantity` to `meter` for `workspace_id`, on today's UTC date.
    pub fn record(&self, workspace_id: WorkspaceId, meter: UsageMeter, quantity: i64) {
        self.add((workspace_id, Utc::now().date_naive(), meter), quantity);
    }

    /// Everything counted since the last take, emptying the buffer.
    pub fn take(&self) -> Vec<UsageMeterDay> {
        let drained: Vec<(Key, i64)> = self.lock().drain().collect();
        drained
            .into_iter()
            .map(|((workspace_id, day, meter), quantity)| UsageMeterDay {
                workspace_id,
                day,
                meter,
                quantity,
            })
            .collect()
    }

    /// Put back a batch whose flush failed, so the next flush carries it.
    pub fn restore(&self, batch: Vec<UsageMeterDay>) {
        for row in batch {
            self.add((row.workspace_id, row.day, row.meter), row.quantity);
        }
    }

    /// Whether `workspace_id` is refused `meter`'s work: an open sign-up
    /// workspace past its free allowance on it this month.
    pub fn is_cut_off(&self, workspace_id: WorkspaceId, meter: UsageMeter) -> bool {
        self.cut_off
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&(workspace_id, meter))
    }

    /// Replace the cutoff set with `pairs` (the store's crossings for the
    /// current month, open workspaces only). A new month starts empty.
    pub fn set_cut_off(&self, pairs: impl IntoIterator<Item = (WorkspaceId, UsageMeter)>) {
        let next: HashSet<_> = pairs.into_iter().collect();
        *self
            .cut_off
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = next;
    }

    /// Whether any workspace is cut off on `meter`. The store checks this
    /// first, so a write pays for a workspace lookup only while some
    /// workspace is actually past its allowance.
    pub fn any_cut_off(&self, meter: UsageMeter) -> bool {
        self.cut_off
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|(_, m)| *m == meter)
    }

    /// Every workspace cut off on `meter`, for a background job that filters
    /// them out of its query.
    pub fn cut_off_workspaces(&self, meter: UsageMeter) -> Vec<WorkspaceId> {
        self.cut_off
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, m)| *m == meter)
            .map(|(w, _)| *w)
            .collect()
    }

    /// A meter `workspace_id` is cut off on, if any.
    pub fn cut_off_meter(&self, workspace_id: WorkspaceId) -> Option<UsageMeter> {
        self.cut_off
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(w, _)| *w == workspace_id)
            .map(|(_, m)| *m)
            .min_by_key(|m| m.as_str())
    }

    /// `Err(OverAllowance(meter))` when `workspace_id` is cut off on `meter`.
    pub fn refuse(&self, workspace_id: WorkspaceId, meter: UsageMeter) -> Result<(), StoreError> {
        if self.is_cut_off(workspace_id, meter) {
            return Err(StoreError::OverAllowance(meter));
        }
        Ok(())
    }

    fn add(&self, key: Key, quantity: i64) {
        *self.lock().entry(key).or_insert(0) += quantity;
    }

    // A panic while the lock was held leaves plain integers behind, so a
    // poisoned map is still sound to use.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Key, i64>> {
        self.counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The origin an import gets, or `OverAllowance` when it must be refused.
///
/// An import is refused while the caller's workspace, or the workspace a
/// restore overwrites, is cut off on any meter: importing is how a cut-off
/// workspace would otherwise start over. The new workspace takes the
/// caller's origin, so only an operator caller (`bypass`) makes an operator
/// workspace; a caller whose workspace cannot be found counts as open. A
/// restore over an existing workspace keeps that workspace's origin, which
/// the store reads in the import transaction.
pub async fn import_origin<S: crate::Store + ?Sized>(
    store: &S,
    bypass: bool,
    caller: WorkspaceId,
    restore_target: Option<WorkspaceId>,
) -> Result<maidan_types::WorkspaceOrigin, StoreError> {
    let meters = store.usage_meters();
    for workspace_id in std::iter::once(caller).chain(restore_target) {
        if let Some(meter) = meters.cut_off_meter(workspace_id) {
            return Err(StoreError::OverAllowance(meter));
        }
    }
    if bypass {
        return Ok(maidan_types::WorkspaceOrigin::Operator);
    }
    match store.workspace_origin(caller).await {
        Ok(origin) => Ok(origin),
        Err(StoreError::NotFound) => Ok(maidan_types::WorkspaceOrigin::Open),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_add_up_per_workspace_and_meter_and_a_take_empties_the_buffer() {
        let buffer = UsageMeterBuffer::new();
        let (a, b) = (
            WorkspaceId(uuid::Uuid::now_v7()),
            WorkspaceId(uuid::Uuid::now_v7()),
        );
        buffer.record(a, UsageMeter::Requests, 1);
        buffer.record(a, UsageMeter::Requests, 2);
        buffer.record(a, UsageMeter::McpCalls, 1);
        buffer.record(b, UsageMeter::Requests, 5);
        let mut rows = buffer.take();
        rows.sort_by_key(|row| (row.workspace_id == b, row.meter));
        let got: Vec<_> = rows
            .iter()
            .map(|r| (r.workspace_id, r.meter, r.quantity))
            .collect();
        assert_eq!(
            got,
            vec![
                (a, UsageMeter::Requests, 3),
                (a, UsageMeter::McpCalls, 1),
                (b, UsageMeter::Requests, 5),
            ]
        );
        assert!(buffer.take().is_empty());
        buffer.restore(rows);
        assert_eq!(buffer.take().len(), 3, "a failed flush is put back");
    }
}
