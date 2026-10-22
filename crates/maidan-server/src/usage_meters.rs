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

/// The flush loop, spawned by `main.rs`.
pub async fn run(state: AppState, interval: Duration) {
    tracing::info!(
        interval_secs = interval.as_secs(),
        "usage meter flush started"
    );
    let _watched = crate::ops_alert::start_loop("usage_meters", interval);
    loop {
        tokio::time::sleep(interval).await;
        if let Some(rows) = crate::ops_alert::guarded_tick("usage_meters", flush_once(&state)).await
        {
            tracing::debug!(rows, "usage meters flushed");
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
