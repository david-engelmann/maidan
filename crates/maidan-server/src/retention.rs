//! Background data-retention pruning (Cluster 186).
//!
//! The event log, audit trail, and delivery tables grow without bound. This
//! sweeper deletes rows past a per-table age, in batches (so a first sweep over
//! a long-unpruned table doesn't lock it). Everything is opt-in: with no
//! `MAIDAN_RETENTION_*_DAYS` set, nothing runs.
//!
//! **Event-log safety.** Events are pruned only up to `min_delivery_cursor` —
//! the lowest watermark across all at-least-once consumers — so a lagging
//! durable consumer never loses an undelivered event. The age cutoff (days) is
//! always far older than the delivery stability horizon (seconds), so that floor
//! needs no separate check. Optimistic reconnect replay beyond the retention
//! window is out of scope by design (that's what retention *is*).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::Store;

/// Resolved retention policy. `None` day fields mean "keep forever" for that
/// table.
#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub events_days: Option<u32>,
    pub audit_days: Option<u32>,
    pub deliveries_days: Option<u32>,
    pub sweep: Duration,
    pub batch: i64,
}

fn parse_days(raw: Option<String>) -> Option<u32> {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&d| d > 0)
}

/// Build the policy from the environment, or `None` when no table has a
/// retention set (the sweeper is not started).
pub fn config_from_env() -> Option<RetentionConfig> {
    let events_days = parse_days(std::env::var("MAIDAN_RETENTION_EVENTS_DAYS").ok());
    let audit_days = parse_days(std::env::var("MAIDAN_RETENTION_AUDIT_DAYS").ok());
    let deliveries_days = parse_days(std::env::var("MAIDAN_RETENTION_DELIVERIES_DAYS").ok());
    if events_days.is_none() && audit_days.is_none() && deliveries_days.is_none() {
        return None;
    }
    let sweep = std::env::var("MAIDAN_RETENTION_SWEEP_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(86_400);
    let batch = std::env::var("MAIDAN_RETENTION_BATCH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&b| b > 0)
        .unwrap_or(5_000);
    Some(RetentionConfig {
        events_days,
        audit_days,
        deliveries_days,
        sweep: Duration::from_secs(sweep),
        batch,
    })
}

fn cutoff(now: chrono::DateTime<chrono::Utc>, days: u32) -> chrono::DateTime<chrono::Utc> {
    now - chrono::Duration::days(i64::from(days))
}

/// Run one sweep across every configured table. Errors on a single table are
/// logged and do not abort the others.
pub async fn sweep_once(store: &Arc<dyn Store>, cfg: &RetentionConfig) {
    let now = chrono::Utc::now();

    if let Some(days) = cfg.events_days {
        // Floor at the lowest durable-delivery watermark; unbounded when there
        // are no at-least-once consumers.
        let max_id = match store.min_delivery_cursor().await {
            Ok(v) => v.unwrap_or(i64::MAX),
            Err(err) => {
                tracing::warn!(error = %err, "retention: min_delivery_cursor failed; skipping events");
                i64::MIN // prune nothing this round
            }
        };
        let deleted = prune_loop("events", cfg.batch, |limit| {
            store.prune_events(cutoff(now, days), max_id, limit)
        })
        .await;
        record("events", deleted);
    }

    if let Some(days) = cfg.audit_days {
        let deleted = prune_loop("audit", cfg.batch, |limit| {
            store.prune_audit(cutoff(now, days), limit)
        })
        .await;
        record("audit", deleted);
    }

    if let Some(days) = cfg.deliveries_days {
        let deleted = prune_loop("deliveries", cfg.batch, |limit| {
            store.prune_deliveries(cutoff(now, days), limit)
        })
        .await;
        record("deliveries", deleted);
    }
}

/// Call `prune(batch)` repeatedly until a page comes back short (table drained
/// for this cutoff). Each page deletes rows, so the matching set shrinks and the
/// loop terminates.
async fn prune_loop<F, Fut>(table: &str, batch: i64, mut prune: F) -> u64
where
    F: FnMut(i64) -> Fut,
    Fut: std::future::Future<Output = Result<u64, maidan_store::StoreError>>,
{
    let mut total = 0u64;
    loop {
        match prune(batch).await {
            Ok(n) => {
                total += n;
                if n < batch as u64 {
                    break;
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, table, "retention: prune failed");
                break;
            }
        }
    }
    if total > 0 {
        tracing::info!(table, pruned = total, "retention swept");
    }
    total
}

fn record(table: &str, pruned: u64) {
    if pruned > 0 {
        crate::metrics::record_retention_pruned(table, pruned);
    }
}

/// Loop: sweep, then sleep `cfg.sweep`. Spawned once at startup when retention is
/// configured.
pub async fn run(store: Arc<dyn Store>, cfg: RetentionConfig) {
    tracing::info!(
        events_days = ?cfg.events_days,
        audit_days = ?cfg.audit_days,
        deliveries_days = ?cfg.deliveries_days,
        sweep_secs = cfg.sweep.as_secs(),
        "retention sweeper started"
    );
    loop {
        sweep_once(&store, &cfg).await;
        tokio::time::sleep(cfg.sweep).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_days_filters_zero_and_junk() {
        assert_eq!(parse_days(None), None);
        assert_eq!(parse_days(Some("0".into())), None);
        assert_eq!(parse_days(Some("  ".into())), None);
        assert_eq!(parse_days(Some("nope".into())), None);
        assert_eq!(parse_days(Some("30".into())), Some(30));
    }

    #[test]
    fn cutoff_is_days_before_now() {
        let now = chrono::DateTime::from_timestamp(1_000_000_000, 0).unwrap();
        let c = cutoff(now, 10);
        assert_eq!((now - c).num_days(), 10);
    }
}
