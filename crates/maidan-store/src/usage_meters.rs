//! The in-memory side of the usage meters. Requests and MCP calls are counted
//! here, under a lock and never with a database write, and the server's
//! `usage_meters` loop flushes the counts to [`crate::Store::add_usage_meters`]
//! in one batch. A process that dies between flushes loses at most one
//! interval's counts: the meters are for an allowance, not for billing.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use chrono::{NaiveDate, Utc};
use maidan_types::{UsageMeter, UsageMeterDay, WorkspaceId};

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
