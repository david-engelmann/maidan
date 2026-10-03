//! Background data-retention pruning.
//!
//! The event log, audit trail, read notifications, and delivery tables
//! (webhook and automation deliveries, the transactional, egress and mail
//! outboxes, the agent-run DLQ) grow without bound. This sweeper deletes rows
//! past a per-table age, in batches (so a first sweep over a long-unpruned
//! table doesn't lock it). Everything is opt-in: with no
//! `MAIDAN_RETENTION_*_DAYS` set and no workspace policy, nothing is pruned.
//! The usage ledger is not pruned.
//!
//! **Per-workspace retention.** A workspace may set a shorter retention for
//! its messages, events and finished deliveries (`maidan_retention_policies`,
//! never longer than the instance keeps). Each sweep then prunes that
//! workspace's rows past its own cutoff, after the instance sweep. A workspace
//! under legal hold loses nothing either way: the instance delivery sweep and
//! the store's per-workspace prunes check the hold in the deleting statement
//! or transaction.
//!
//! **Event-log safety.** Events are pruned only up to `min_delivery_cursor` —
//! the lowest watermark across all at-least-once consumers — so a lagging
//! durable consumer never loses an undelivered event. The age cutoff (days) is
//! always far older than the delivery stability horizon (seconds), so that
//! floor needs no separate check. Optimistic reconnect replay beyond the
//! retention window is out of scope by design (that's what retention *is*).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::retention_policy::parse_days as parse_days_raw;
use maidan_store::Store;
use maidan_types::{RetentionDays, WorkspaceId};

/// Resolved retention policy. `None` day fields mean "keep forever" for that
/// table.
#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub events_days: Option<u32>,
    pub audit_days: Option<u32>,
    pub deliveries_days: Option<u32>,
    /// Read notifications. Unread rows and any row with a snooze set, lapsed
    /// or not, are never pruned. `None` keeps every notification.
    pub notifications_days: Option<u32>,
    pub sweep: Duration,
    pub batch: i64,
}

fn parse_days(raw: Option<String>) -> Option<u32> {
    parse_days_raw(raw.as_deref())
}

impl RetentionConfig {
    /// What the instance keeps, the ceiling a workspace policy is held to.
    pub fn instance(&self) -> RetentionDays {
        RetentionDays {
            messages_days: None,
            events_days: self.events_days.map(i64::from),
            deliveries_days: self.deliveries_days.map(i64::from),
        }
    }
}

/// Build the policy from the environment. The sweeper always runs, since a
/// workspace can set its own retention at any time; with nothing set, a sweep
/// is one read of the (empty) policy table.
pub fn config_from_env() -> RetentionConfig {
    let events_days = parse_days(std::env::var("MAIDAN_RETENTION_EVENTS_DAYS").ok());
    let audit_days = parse_days(std::env::var("MAIDAN_RETENTION_AUDIT_DAYS").ok());
    let deliveries_days = parse_days(std::env::var("MAIDAN_RETENTION_DELIVERIES_DAYS").ok());
    let notifications_days = parse_days(std::env::var("MAIDAN_RETENTION_NOTIFICATIONS_DAYS").ok());
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
    RetentionConfig {
        events_days,
        audit_days,
        deliveries_days,
        notifications_days,
        sweep: Duration::from_secs(sweep),
        batch,
    }
}

fn cutoff(now: chrono::DateTime<chrono::Utc>, days: u32) -> chrono::DateTime<chrono::Utc> {
    now - chrono::Duration::days(i64::from(days))
}

/// Run one sweep across every configured table. Errors on a single table are
/// logged and do not abort the others.
pub async fn sweep_once(store: &Arc<dyn Store>, cfg: &RetentionConfig) {
    let now = chrono::Utc::now();

    if let Some(days) = cfg.events_days {
        // Floor at the lowest watermark among durable consumers still
        // advancing within the retention window; unbounded when there are none.
        let events_cutoff = cutoff(now, days);
        let max_id = match store.min_delivery_cursor(events_cutoff).await {
            Ok(v) => v.unwrap_or(i64::MAX),
            Err(err) => {
                tracing::warn!(error = %err, "retention: min_delivery_cursor failed; skipping events");
                i64::MIN // prune nothing this round
            }
        };
        let deleted = prune_loop("events", cfg.batch, |limit| {
            store.prune_events(events_cutoff, max_id, limit)
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

    if let Some(days) = cfg.notifications_days {
        let deleted = prune_loop("notifications", cfg.batch, |limit| {
            store.prune_notifications(cutoff(now, days), limit)
        })
        .await;
        record("notifications", deleted);
    }

    if let Some(days) = cfg.deliveries_days {
        let deleted = prune_loop("deliveries", cfg.batch, |limit| {
            store.prune_deliveries(cutoff(now, days), limit)
        })
        .await;
        record("deliveries", deleted);
    }

    match store.list_retention_policies().await {
        Ok(policies) => {
            let instance = cfg.instance();
            for (workspace_id, policy) in policies {
                sweep_workspace(store, cfg.batch, now, workspace_id, &policy, &instance).await;
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "retention: listing workspace policies failed");
        }
    }
}

/// A workspace's own days for one kind of row, when they are shorter than the
/// instance's; otherwise the instance sweep already covers it.
fn stricter(workspace: Option<i64>, instance: Option<i64>) -> Option<u32> {
    let days = workspace?;
    if instance.is_some_and(|ceiling| ceiling <= days) {
        return None;
    }
    u32::try_from(days).ok()
}

async fn sweep_workspace(
    store: &Arc<dyn Store>,
    batch: i64,
    now: chrono::DateTime<chrono::Utc>,
    workspace_id: WorkspaceId,
    policy: &RetentionDays,
    instance: &RetentionDays,
) {
    if let Some(days) = stricter(policy.messages_days, instance.messages_days) {
        let deleted = prune_loop("messages", batch, |limit| {
            store.prune_workspace_messages(workspace_id, cutoff(now, days), limit)
        })
        .await;
        record("messages", deleted);
    }
    if let Some(days) = stricter(policy.events_days, instance.events_days) {
        let deleted = prune_loop("events", batch, |limit| {
            store.prune_workspace_events(workspace_id, cutoff(now, days), limit)
        })
        .await;
        record("events", deleted);
    }
    if let Some(days) = stricter(policy.deliveries_days, instance.deliveries_days) {
        let deleted = prune_loop("deliveries", batch, |limit| {
            store.prune_workspace_deliveries(workspace_id, cutoff(now, days), limit)
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

/// Loop: sweep, then sleep `cfg.sweep`. Spawned once at startup.
pub async fn run(store: Arc<dyn Store>, cfg: RetentionConfig) {
    tracing::info!(
        events_days = ?cfg.events_days,
        audit_days = ?cfg.audit_days,
        deliveries_days = ?cfg.deliveries_days,
        notifications_days = ?cfg.notifications_days,
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
    fn a_workspace_sweep_runs_only_where_it_is_stricter_than_the_instance() {
        assert_eq!(stricter(None, Some(30)), None);
        assert_eq!(stricter(Some(7), None), Some(7));
        assert_eq!(stricter(Some(7), Some(30)), Some(7));
        assert_eq!(stricter(Some(30), Some(30)), None);
        // Set before the operator lowered the instance's: the instance's wins.
        assert_eq!(stricter(Some(60), Some(30)), None);
    }

    #[test]
    fn cutoff_is_days_before_now() {
        let now = chrono::DateTime::from_timestamp(1_000_000_000, 0).unwrap();
        let c = cutoff(now, 10);
        assert_eq!((now - c).num_days(), 10);
    }
}
