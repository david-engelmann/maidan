//! Resume a lagged bus consumer from the durable event log.
//!
//! `RecvError::Lagged` / `BusItem::Lagged` must never silently drop: drain `id
//! > after_id` globally (the bus is not workspace-scoped) and re-handle.

use crate::error::StoreError;
use crate::store::Store;
use maidan_types::StoredEvent;

pub const LAG_RESUME_BATCH: i64 = 256;

/// Page the global event log after `after_id` and invoke `on_page` for each
/// batch. Returns the high-water id (unchanged when the log has nothing new).
pub async fn resume_from_log<F, Fut>(
    store: &dyn Store,
    mut after_id: i64,
    mut on_page: F,
) -> Result<i64, StoreError>
where
    F: FnMut(Vec<StoredEvent>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    loop {
        let page = store
            .list_events_after_global(after_id, LAG_RESUME_BATCH)
            .await?;
        if page.is_empty() {
            return Ok(after_id);
        }
        let short = (page.len() as i64) < LAG_RESUME_BATCH;
        after_id = page.last().map(|e| e.id).unwrap_or(after_id);
        on_page(page).await;
        if short {
            return Ok(after_id);
        }
    }
}
