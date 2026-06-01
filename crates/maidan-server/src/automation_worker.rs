//! Polls `maidan_automation_deliveries` and dispatches signed HTTP with retry.

use std::time::Duration;

use reqwest::Client;
use tokio::sync::watch;
use tracing::warn;

use crate::automation_delivery::{
    backoff, deliver_pending, max_attempts_from_env, poll_interval_ms_from_env,
};
use crate::metrics;
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
    let client = Client::new();
    let max_attempts = max_attempts_from_env();
    let interval = Duration::from_millis(poll_interval_ms_from_env());
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(interval) => {
                if let Err(err) = poll_once(&state, &client, max_attempts).await {
                    warn!(error = %err, "automation delivery poll failed");
                }
            }
        }
    }
}

async fn poll_once(state: &AppState, client: &Client, max_attempts: u32) -> Result<(), String> {
    let pending = state
        .store
        .list_pending_automation_deliveries(DELIVERY_BATCH)
        .await
        .map_err(|e| e.to_string())?;
    for delivery in pending {
        let start = std::time::Instant::now();
        match deliver_pending(client, state, &delivery).await {
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
