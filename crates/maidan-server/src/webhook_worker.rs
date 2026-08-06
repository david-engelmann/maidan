//! Subscribes to the event bus and delivers signed outbound webhooks with retry.

use std::time::Duration;

use maidan_bus::{BusItem, EventStream};
use maidan_types::{EventFilter, EventKind};
use reqwest::Client;
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tracing::{info, warn};

use crate::state::AppState;
use crate::webhooks::{
    build_payload, deliver_http, delivery_backoff, event_kind_from_payload, kinds_match,
    max_attempts_from_env, poll_interval_ms_from_env, resolve_webhook_secret,
};

const DELIVERY_BATCH: i64 = 64;
const RECONNECT_INITIAL: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(5);

pub struct WebhookWorker {
    shutdown: watch::Sender<()>,
    bus_handle: tokio::task::JoinHandle<()>,
    delivery_handle: tokio::task::JoinHandle<()>,
}

impl WebhookWorker {
    pub fn spawn(state: AppState) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let bus_state = state.clone();
        let bus_shutdown = shutdown_rx.clone();
        let bus_handle = tokio::spawn(async move {
            run_bus_consumer(bus_state, bus_shutdown).await;
        });
        let delivery_state = state;
        let delivery_shutdown = shutdown_rx;
        let delivery_handle = tokio::spawn(async move {
            run_delivery_poller(delivery_state, delivery_shutdown).await;
        });
        Self {
            shutdown: shutdown_tx,
            bus_handle,
            delivery_handle,
        }
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.bus_handle.await;
        let _ = self.delivery_handle.await;
    }
}

async fn run_bus_consumer(state: AppState, mut shutdown: watch::Receiver<()>) {
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    let stop_forward = stop_tx.clone();
    tokio::spawn(async move {
        let _ = shutdown.changed().await;
        let _ = stop_forward.send(()).await;
    });

    let mut backoff = RECONNECT_INITIAL;
    loop {
        let filter = EventFilter::all();
        let stream = match state.bus.subscribe(filter).await {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, ?backoff, "webhook bus subscribe failed; retrying");
                if tokio::time::timeout(backoff, stop_rx.recv()).await.is_ok() {
                    return;
                }
                backoff = (backoff * 2).min(RECONNECT_MAX);
                continue;
            }
        };
        backoff = RECONNECT_INITIAL;
        info!("webhook worker attached to bus");
        if consume_bus(stream, &state, &mut stop_rx).await {
            return;
        }
        warn!("webhook bus stream ended; resubscribing");
    }
}

async fn consume_bus(
    mut stream: EventStream,
    state: &AppState,
    stop_rx: &mut mpsc::Receiver<()>,
) -> bool {
    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(BusItem::Event(envelope)) => {
                        if let Err(err) = enqueue_matches(state, envelope.log_id, &envelope.event).await {
                            warn!(error = %err, "webhook enqueue failed");
                        }
                    }
                    Some(BusItem::Lagged { skipped }) => {
                        warn!(skipped, "webhook bus subscriber lagged");
                    }
                    None => return false,
                }
            }
            _ = stop_rx.recv() => {
                info!("webhook bus consumer shutdown");
                return true;
            }
        }
    }
}

async fn enqueue_matches(
    state: &AppState,
    log_id: i64,
    event: &maidan_types::Event,
) -> Result<(), String> {
    let Some(workspace_id) = event.workspace_id() else {
        return Ok(());
    };
    let kind = event.kind();
    // Only this workspace's enabled subscriptions (H1) — was an all-workspaces
    // scan on every event.
    let subs = state
        .store
        .list_enabled_webhook_subscriptions_for_workspace(workspace_id)
        .await
        .map_err(|e| e.to_string())?;
    // Build the payload lazily — only once a subscription actually matches.
    let mut payload: Option<String> = None;
    let mut enqueued = std::collections::HashSet::new();
    for sub in &subs {
        let subscription = &sub.subscription;
        if !kinds_match(subscription, &kind) {
            continue;
        }
        if payload.is_none() {
            payload = Some(build_payload(log_id, event).map_err(|e| e.to_string())?);
        }
        state
            .store
            .enqueue_webhook_delivery(subscription.id, log_id, payload.as_deref().unwrap())
            .await
            .map_err(|e| e.to_string())?;
        enqueued.insert(subscription.id);
    }
    if kind == EventKind::MentionRecorded {
        if let Ok(Some(mention_webhook_id)) = state
            .store
            .get_workspace_mention_webhook_id(workspace_id)
            .await
        {
            // `subs` is already scoped to this workspace (H1).
            if !enqueued.contains(&mention_webhook_id)
                && subs.iter().any(|s| {
                    s.subscription.id == mention_webhook_id && s.subscription.revoked_at.is_none()
                })
            {
                if payload.is_none() {
                    payload = Some(build_payload(log_id, event).map_err(|e| e.to_string())?);
                }
                state
                    .store
                    .enqueue_webhook_delivery(
                        mention_webhook_id,
                        log_id,
                        payload.as_deref().unwrap(),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

async fn run_delivery_poller(state: AppState, mut shutdown: watch::Receiver<()>) {
    let client = Client::new();
    let max_attempts = max_attempts_from_env();
    let interval = Duration::from_millis(poll_interval_ms_from_env());
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(interval) => {
                if let Err(err) = poll_deliveries(&state, &client, max_attempts).await {
                    warn!(error = %err, "webhook delivery poll failed");
                }
            }
        }
    }
}

async fn poll_deliveries(
    state: &AppState,
    client: &Client,
    max_attempts: u32,
) -> Result<(), String> {
    let pending = state
        .store
        .list_pending_webhook_deliveries(DELIVERY_BATCH)
        .await
        .map_err(|e| e.to_string())?;
    for delivery in pending {
        let sub = state
            .store
            .get_webhook_subscription(delivery.subscription_id)
            .await
            .map_err(|e| e.to_string())?;
        let Some(secret) = resolve_webhook_secret(
            &state.webhooks,
            delivery.subscription_id,
            &sub.secret_ciphertext,
        ) else {
            warn!(
                delivery_id = delivery.id,
                webhook = %delivery.subscription_id,
                "skipping delivery: no webhook secret"
            );
            continue;
        };
        let kind = event_kind_from_payload(&delivery.payload);
        match deliver_http(
            client,
            &sub.subscription.url,
            delivery.id,
            kind,
            &secret,
            &delivery.payload,
        )
        .await
        {
            Ok(()) => {
                let _ = state
                    .store
                    .mark_webhook_delivery_delivered(delivery.id)
                    .await;
            }
            Err(err) => {
                let next = chrono::Utc::now() + delivery_backoff(delivery.attempts);
                let attempts = state
                    .store
                    .record_webhook_delivery_attempt(delivery.id, &err, next)
                    .await
                    .map_err(|e| e.to_string())?;
                if attempts >= max_attempts as i32 {
                    let _ = state.store.quarantine_webhook_delivery(delivery.id).await;
                    warn!(
                        delivery_id = delivery.id,
                        attempts,
                        max_attempts,
                        error = %err,
                        "webhook delivery quarantined"
                    );
                }
            }
        }
    }
    Ok(())
}
