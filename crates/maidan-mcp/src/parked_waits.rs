//! A ceiling on the long-polls one workspace may park at once (RED-008).
//!
//! A `wait_for_*` call blocks for up to five minutes by design. Served over
//! HTTP it gives back the server's in-flight permit while it parks, so parked
//! waits no longer fill the instance-wide ceiling every tenant shares; this
//! per-workspace cap is what bounds one tenant's waits instead. A call past
//! the cap is refused at once with a typed rate-limit error rather than
//! queued, so a client backs off instead of piling up.
//!
//! The permit goes back only once the wait is admitted: its capability
//! checked, a place under this cap held, the allowance and its arguments and
//! channel access passed. A wait refused on the way keeps its permit until it
//! answers, so refused calls cannot run their checks outside the ceiling. The
//! count is held in memory, so the cap is per workspace per replica.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use maidan_types::WorkspaceId;

/// The most `wait_for_*` calls one workspace may have parked at once.
pub const MAX_PARKED_WAITS_PER_WORKSPACE: usize = 32;

tokio::task_local! {
    /// How the request being served gives back its in-flight permit, when the
    /// transport holds one for it.
    static RELEASE_PERMIT: Box<dyn Fn() + Send + Sync>;
}

/// Run `request` with `release` as the way to give back its in-flight permit.
/// The server calls it at most once per admitted wait, from the request's own
/// task; a request that never reaches an admitted wait never calls it.
pub async fn with_permit_release<F: Future>(
    release: impl Fn() + Send + Sync + 'static,
    request: F,
) -> F::Output {
    RELEASE_PERMIT.scope(Box::new(release), request).await
}

/// Give back the current request's in-flight permit, if its transport holds
/// one. Outside [`with_permit_release`] (stdio, an auth-disabled test harness,
/// a hook firing) there is nothing to give back.
pub(crate) fn release_permit() {
    let _ = RELEASE_PERMIT.try_with(|release| release());
}

/// Whether a tool is a long-poll that parks until something happens.
pub fn parks(tool: &str) -> bool {
    tool.starts_with("wait_for_")
}

/// Parked waits per workspace. Cloning [`McpServer`](crate::McpServer) shares it.
#[derive(Debug)]
pub struct ParkedWaits {
    max: usize,
    parked: Mutex<HashMap<WorkspaceId, usize>>,
}

/// One parked wait's place under its workspace's cap, given back on drop.
#[derive(Debug)]
pub struct ParkedWait {
    waits: Arc<ParkedWaits>,
    workspace_id: WorkspaceId,
}

impl ParkedWaits {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            parked: Mutex::new(HashMap::new()),
        }
    }

    /// Take a place for one wait in `workspace_id`, or `None` when the
    /// workspace already has `max` parked.
    pub fn park(self: &Arc<Self>, workspace_id: WorkspaceId) -> Option<ParkedWait> {
        // A poisoned lock still holds a consistent count: every writer
        // changes it by one and nothing else.
        let mut parked = self.parked.lock().unwrap_or_else(|e| e.into_inner());
        let count = parked.entry(workspace_id).or_default();
        if *count >= self.max {
            return None;
        }
        *count += 1;
        Some(ParkedWait {
            waits: Arc::clone(self),
            workspace_id,
        })
    }

    /// Waits `workspace_id` has parked right now.
    pub fn parked(&self, workspace_id: WorkspaceId) -> usize {
        let parked = self.parked.lock().unwrap_or_else(|e| e.into_inner());
        parked.get(&workspace_id).copied().unwrap_or(0)
    }
}

impl Default for ParkedWaits {
    fn default() -> Self {
        Self::new(MAX_PARKED_WAITS_PER_WORKSPACE)
    }
}

impl Drop for ParkedWait {
    fn drop(&mut self) {
        let mut parked = self.waits.parked.lock().unwrap_or_else(|e| e.into_inner());
        // The entry goes with its last wait, so the map holds only workspaces
        // that are waiting now, not every workspace that ever waited.
        if let Some(count) = parked.get_mut(&self.workspace_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                parked.remove(&self.workspace_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_at_its_cap_is_refused_and_another_workspace_is_not() {
        let waits = Arc::new(ParkedWaits::new(2));
        let a = WorkspaceId(uuid::Uuid::now_v7());
        let b = WorkspaceId(uuid::Uuid::now_v7());
        let first = waits.park(a);
        let second = waits.park(a);
        assert!(first.is_some() && second.is_some());
        assert!(waits.park(a).is_none(), "a third wait in a is refused");
        assert!(waits.park(b).is_some(), "b has its own cap");
        drop(first);
        assert_eq!(waits.parked(a), 1);
        assert!(
            waits.park(a).is_some(),
            "a finished wait gives its place back"
        );
    }

    #[test]
    fn a_workspace_with_nothing_parked_leaves_no_entry() {
        let waits = Arc::new(ParkedWaits::new(1));
        let a = WorkspaceId(uuid::Uuid::now_v7());
        drop(waits.park(a));
        assert!(waits.parked.lock().unwrap().is_empty());
    }

    #[test]
    fn only_wait_for_tools_park() {
        assert!(parks("wait_for_result"));
        assert!(parks("wait_for_event"));
        assert!(!parks("claim_next_thread"));
        assert!(!parks("await_for_result"));
    }
}
