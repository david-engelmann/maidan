//! Flushing the usage meters. Requests (counted in the auth layer) and MCP
//! calls (counted in the MCP server) accumulate in
//! [`maidan_store::usage_meters::UsageMeterBuffer`]; this loop writes them to
//! the per-workspace daily rollup in one batch every interval, and once more
//! on shutdown. A crash between flushes loses at most one interval's counts.

use std::time::Duration;

use crate::state::AppState;

/// Default flush interval (`MAIDAN_USAGE_METER_FLUSH_SECS`).
pub const DEFAULT_FLUSH_SECS: u64 = 60;

/// The flush interval from the environment: the default when unset or
/// unparsable, at least one second.
pub fn interval_from_env() -> Duration {
    interval_from_raw(std::env::var("MAIDAN_USAGE_METER_FLUSH_SECS").ok())
}

fn interval_from_raw(raw: Option<String>) -> Duration {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_FLUSH_SECS)
        .max(1);
    Duration::from_secs(secs)
}

/// Paths that are not metered: probes and the scrape, which say nothing
/// about what a workspace uses.
pub fn unmetered(path: &str) -> bool {
    path == "/metrics" || path == "/health" || path.starts_with("/health/")
}

/// Count for the workspace of `member_id`, for work known by its member
/// (a web push). A failed member lookup counts nothing.
pub async fn record_for_member(
    state: &AppState,
    member_id: maidan_types::MemberId,
    meter: maidan_types::UsageMeter,
    quantity: i64,
) {
    match state.store.get_member(member_id).await {
        Ok(member) => state
            .usage_meters
            .record(member.workspace_id, meter, quantity),
        Err(err) => tracing::debug!(error = %err, "usage meters: member not found; not counted"),
    }
}

/// Write everything counted since the last flush. A failed write puts the
/// batch back, so the next flush carries it; a write that committed but whose
/// answer was lost is then counted twice, which an allowance can bear. A count
/// for a workspace erased since is dropped by the store. Returns the rows
/// taken.
pub async fn flush_once(state: &AppState) -> usize {
    let batch = state.usage_meters.take();
    if batch.is_empty() {
        return 0;
    }
    match state.store.add_usage_meters(&batch).await {
        Ok(()) => batch.len(),
        Err(err) => {
            tracing::warn!(error = %err, rows = batch.len(), "usage meters: flush failed; kept for the next one");
            state.usage_meters.restore(batch);
            0
        }
    }
}

/// Workspaces measured for stored bytes per tick, so a large instance's
/// first pass of the day is spread over several ticks.
pub const MEASURE_PER_TICK: i64 = 25;

/// Measure what each workspace stores, once a UTC day: the workspaces with no
/// `stored_bytes` row today, up to [`MEASURE_PER_TICK`]. Returns how many were
/// measured. A failed measurement is retried next tick.
pub async fn measure_stored_bytes(state: &AppState) -> usize {
    let today = chrono::Utc::now().date_naive();
    let meter = maidan_types::UsageMeter::StoredBytes;
    let due = match state
        .store
        .list_unmeasured_workspaces(today, meter, MEASURE_PER_TICK)
        .await
    {
        Ok(due) => due,
        Err(err) => {
            tracing::warn!(error = %err, "usage meters: workspaces to measure not read");
            return 0;
        }
    };
    let mut batch = Vec::with_capacity(due.len());
    for workspace_id in due {
        match state.store.workspace_stored_bytes(workspace_id).await {
            Ok(quantity) => batch.push(maidan_types::UsageMeterDay {
                workspace_id,
                day: today,
                meter,
                quantity,
            }),
            Err(err) => {
                tracing::warn!(error = %err, workspace_id = %workspace_id.0, "usage meters: stored bytes not measured");
            }
        }
    }
    match state.store.set_usage_meters(&batch).await {
        Ok(()) => batch.len(),
        Err(err) => {
            tracing::warn!(error = %err, "usage meters: stored bytes not written");
            0
        }
    }
}

/// One tick: flush the counters, then measure stored bytes where due.
pub async fn tick(state: &AppState) -> (usize, usize) {
    (flush_once(state).await, measure_stored_bytes(state).await)
}

/// The flush loop, spawned by `main.rs`.
pub async fn run(state: AppState, interval: Duration) {
    tracing::info!(
        interval_secs = interval.as_secs(),
        "usage meter flush started"
    );
    let _watched = crate::ops_alert::start_loop("usage_meters", interval);
    loop {
        tokio::time::sleep(interval).await;
        if let Some((rows, measured)) =
            crate::ops_alert::guarded_tick("usage_meters", tick(&state)).await
        {
            tracing::debug!(rows, measured, "usage meters tick");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flush_interval_defaults_and_is_at_least_a_second() {
        let default = Duration::from_secs(DEFAULT_FLUSH_SECS);
        assert_eq!(interval_from_raw(None), default);
        assert_eq!(interval_from_raw(Some("soon".into())), default);
        assert_eq!(
            interval_from_raw(Some(" 15 ".into())),
            Duration::from_secs(15)
        );
        assert_eq!(interval_from_raw(Some("0".into())), Duration::from_secs(1));
    }

    #[test]
    fn probes_and_the_scrape_are_not_metered() {
        for path in ["/metrics", "/health", "/health/live", "/health/ready"] {
            assert!(unmetered(path), "{path}");
        }
        for path in ["/workspaces", "/mcp", "/healthcheck-like"] {
            assert!(!unmetered(path), "{path}");
        }
    }
}
