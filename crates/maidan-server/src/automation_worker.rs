//! Polls `maidan_automation_deliveries` and dispatches signed HTTP with retry.

use std::time::Duration;

use tokio::sync::watch;
use tracing::warn;

use crate::automation_delivery::{
    backoff, deliver_pending, max_attempts_from_env, poll_interval_ms_from_env,
};
use crate::metrics;
use crate::retry_budget::{deferred_until, Attempt};
use crate::state::AppState;

const DELIVERY_BATCH: i64 = 64;

pub struct AutomationDeliveryWorker {
    shutdown: watch::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl AutomationDeliveryWorker {
    pub fn spawn(state: AppState) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let handle = tokio::spawn(run(state, shutdown_rx));
        Self {
            shutdown: shutdown_tx,
            handle,
        }
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.handle.await;
    }
}

async fn run(state: AppState, mut shutdown: watch::Receiver<()>) {
    let max_attempts = max_attempts_from_env();
    let interval = Duration::from_millis(poll_interval_ms_from_env());
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(interval) => {
                if let Err(err) = poll_once(&state, max_attempts).await {
                    warn!(error = %err, "automation delivery poll failed");
                }
            }
        }
    }
}

/// One pass over the due deliveries: send each, reschedule or quarantine a
/// failure, and defer a retry the host's retry budget refuses. The spawned
/// worker calls this every poll interval.
pub async fn poll_once(state: &AppState, max_attempts: u32) -> Result<(), String> {
    let pending = state
        .store
        .list_pending_automation_deliveries(DELIVERY_BATCH)
        .await
        .map_err(|e| e.to_string())?;
    for delivery in pending {
        if let Some(host) = crate::retry_budget::host_of(&delivery.target_url) {
            let attempt = Attempt::after(i64::from(delivery.attempts));
            if let Some(until) = deferred_until(&state.retry_budget, "automation", &host, attempt) {
                if let Err(err) = state
                    .store
                    .defer_automation_delivery(delivery.id, until)
                    .await
                {
                    // Still due, so the next poll asks the budget again.
                    warn!(delivery_id = delivery.id, error = %err, "automation deferral failed");
                }
                continue;
            }
        }
        let start = std::time::Instant::now();
        match maidan_store::trace::maybe_scope(
            delivery.trace.clone(),
            deliver_pending(state, &delivery),
        )
        .await
        {
            Ok(()) => {
                metrics::record_automation_delivery(true);
                let _ = state
                    .store
                    .mark_automation_delivery_delivered(delivery.id)
                    .await;
            }
            Err(err) => {
                metrics::record_automation_delivery(false);
                let next = chrono::Utc::now() + backoff(delivery.attempts);
                let attempts = state
                    .store
                    .record_automation_delivery_attempt(delivery.id, &err, next)
                    .await
                    .map_err(|e| e.to_string())?;
                if attempts >= max_attempts as i32 {
                    let _ = state
                        .store
                        .quarantine_automation_delivery(delivery.id)
                        .await;
                    warn!(
                        delivery_id = delivery.id,
                        attempts,
                        max_attempts,
                        error = %err,
                        "automation delivery quarantined"
                    );
                }
            }
        }
        metrics::record_automation_delivery_duration(start.elapsed());
    }
    Ok(())
}
