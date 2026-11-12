//! Subscribes to the event bus and delivers signed outbound webhooks with retry.

use std::time::Duration;

use maidan_bus::{BusItem, EventStream};
use maidan_types::{EventFilter, EventKind};
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tracing::{info, warn};

use crate::registration_owner::NotActing;
use crate::retry_budget::{deferred_until, Attempt};
use crate::state::AppState;
use crate::webhooks::{
    build_payload, deliver_http, delivery_backoff, event_kind_from_payload, kinds_match,
    max_attempts_from_env, resolve_webhook_secret, POLL_INTERVAL_MS,
};

const DELIVERY_BATCH: i64 = 64;
/// How long a delivery waits when its creator's authority could not be
/// checked (a suspended workspace, a failed lookup) before it is tried again.
const CREATOR_HELD_DEFER_SECS: i64 = 60;
/// How far forward a claim leases a delivery. A send takes at most the egress
/// timeout (10 s); a poller that dies mid-send frees the row after this.
const DELIVERY_LEASE_SECS: i64 = 120;
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
        let seed = match crate::event_stream::attach_watermark(&state).await {
            Ok(head) => head,
            Err(err) => {
                warn!(error = %err, ?backoff, "webhook could not read the log head; retrying");
                if tokio::time::timeout(backoff, stop_rx.recv()).await.is_ok() {
                    return;
                }
                backoff = (backoff * 2).min(RECONNECT_MAX);
                continue;
            }
        };
        if consume_bus(stream, seed, &state, &mut stop_rx).await {
            return;
        }
        warn!("webhook bus stream ended; resubscribing");
    }
}

async fn consume_bus(
    mut stream: EventStream,
    seed: i64,
    state: &AppState,
    stop_rx: &mut mpsc::Receiver<()>,
) -> bool {
    let mut watermark: i64 = seed;
    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(BusItem::Event(envelope)) => {
                        watermark = watermark.max(envelope.log_id);
                        if let Err(err) = enqueue_matches(state, envelope.log_id, &envelope.event).await {
                            warn!(error = %err, "webhook enqueue failed");
                        }
                    }
                    Some(BusItem::Lagged { skipped }) => {
                        warn!(skipped, watermark, "webhook bus subscriber lagged; resuming from log");
                        crate::subscribe_metrics::record_lag_resume("webhook", "lagged");
                        match maidan_store::resume_from_log(state.store.as_ref(), watermark, |page| {
                            let state = state.clone();
                            async move {
                                for row in page {
                                    match crate::event_stream::envelope_from_stored(&row) {
                                        Ok(env) => {
                                            if let Err(err) = enqueue_matches(&state, env.log_id, &env.event).await {
                                                warn!(error = %err, "webhook enqueue failed on lag resume");
                                            }
                                        }
                                        Err(err) => {
                                            warn!(error = %err, id = row.id, "skip stored event on webhook lag resume");
                                        }
                                    }
                                }
                            }
                        })
                        .await
                        {
                            Ok(hw) => {
                                watermark = watermark.max(hw);
                                crate::subscribe_metrics::record_lag_resume("webhook", "resumed");
                            }
                            Err(err) => {
                                warn!(error = %err, "webhook lag resume from log failed");
                                crate::subscribe_metrics::record_lag_resume("webhook", "failed");
                            }
                        }
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
    // The workspace's mention route also gets every mention, whatever kinds
    // its subscription names.
    let mention_webhook_id = if kind == EventKind::MentionRecorded {
        state
            .store
            .get_workspace_mention_webhook_id(workspace_id)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    // Build the payload lazily — only once a subscription actually matches.
    let mut payload: Option<String> = None;
    for sub in &subs {
        let subscription = &sub.subscription;
        if !kinds_match(subscription, &kind) && mention_webhook_id != Some(subscription.id) {
            continue;
        }
        // A subscription delivers for the token that registered it, and only
        // what that token may read: a private channel or a DM its creator is
        // not in stays out of the creator's webhook (SEC-017).
        let Ok(creator) = creator_or_retire(state, sub).await else {
            continue;
        };
        // `workspace:admin` registers a subscription; reading what it carries
        // takes `workspace:read` as well.
        if !creator.has_capability(maidan_auth::capability::WORKSPACE_READ)
            || !crate::agui_stream::event_visible(state.store.as_ref(), &creator, event).await
        {
            continue;
        }
        if payload.is_none() {
            payload = Some(build_payload(log_id, event).map_err(|e| e.to_string())?);
        }
        let Some(body) = payload.as_deref() else {
            continue;
        };
        state
            .store
            .enqueue_webhook_delivery(subscription.id, log_id, body)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Whether `creator` may still read the event at `log_id`, which a queued
/// delivery carries. `None` when the event could not be read back, so the
/// delivery waits rather than being dropped on a failed lookup.
async fn creator_may_read(
    state: &AppState,
    creator: &maidan_auth::AuthContext,
    log_id: i64,
) -> Option<bool> {
    if !creator.has_capability(maidan_auth::capability::WORKSPACE_READ) {
        return Some(false);
    }
    let stored = match state.store.get_stored_event(log_id).await {
        Ok(stored) => stored,
        // An event that is gone cannot be shown to be readable.
        Err(maidan_store::StoreError::NotFound) => return Some(false),
        Err(_) => return None,
    };
    let Ok(envelope) = crate::event_stream::envelope_from_stored(&stored) else {
        return Some(false);
    };
    Some(crate::agui_stream::event_visible(state.store.as_ref(), creator, &envelope.event).await)
}

/// The authority `sub` delivers with: its creator's, while it could still
/// register the subscription (`workspace:admin`). A subscription whose
/// creator's authority has ended is revoked, once, with the reason in the
/// audit trail, so it neither fires again nor writes a row per event; one
/// whose check could not finish is held this time only.
async fn creator_or_retire(
    state: &AppState,
    sub: &maidan_types::WebhookSubscriptionWithSecret,
) -> Result<maidan_auth::AuthContext, NotActing> {
    let subscription = &sub.subscription;
    match crate::registration_owner::creator_authority(
        state,
        sub.created_by_token,
        subscription.workspace_id,
        maidan_auth::capability::WORKSPACE_ADMIN,
    )
    .await
    {
        Ok(creator) => Ok(creator),
        Err(NotActing::Held(reason)) => {
            warn!(webhook = %subscription.id, reason, "webhook held");
            Err(NotActing::Held(reason))
        }
        Err(NotActing::Ended(reason)) => {
            crate::webhooks::forget_webhook_secret(&state.webhooks.secrets, subscription.id);
            match state
                .store
                .revoke_webhook_subscription(subscription.workspace_id, subscription.id)
                .await
            {
                Ok(_) => {
                    crate::registration_owner::record_skip(
                        state,
                        subscription.workspace_id,
                        "webhook",
                        Some(subscription.id.0),
                        reason,
                        serde_json::json!({ "revoked": true }),
                    )
                    .await;
                }
                // Another replica, or an earlier delivery, revoked it first
                // and recorded why.
                Err(maidan_store::StoreError::NotFound) => {}
                Err(err) => {
                    warn!(webhook = %subscription.id, error = %err, reason, "webhook revoke failed");
                }
            }
            Err(NotActing::Ended(reason))
        }
    }
}

async fn run_delivery_poller(state: AppState, mut shutdown: watch::Receiver<()>) {
    let max_attempts = max_attempts_from_env();
    let interval = Duration::from_millis(POLL_INTERVAL_MS);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(interval) => {
                if let Err(err) = poll_deliveries(&state, max_attempts).await {
                    warn!(error = %err, "webhook delivery poll failed");
                }
            }
        }
    }
}

/// One pass over the due deliveries: send each, reschedule or quarantine a
/// failure, and defer a retry the host's retry budget refuses. A workspace
/// cut off on `deliveries` sends nothing and its deliveries are deferred;
/// every attempt that goes out counts one `deliveries` unit. The spawned
/// poller calls this every poll interval.
///
/// Every replica runs this poller, so each delivery is claimed (leased) one
/// at a time before it is sent, and a row another replica holds is skipped.
pub async fn poll_deliveries(state: &AppState, max_attempts: u32) -> Result<(), String> {
    for _ in 0..DELIVERY_BATCH {
        let Some(delivery) = state
            .store
            .claim_next_webhook_delivery(DELIVERY_LEASE_SECS)
            .await
            .map_err(|e| e.to_string())?
        else {
            break;
        };
        let sub = match state
            .store
            .get_webhook_subscription(delivery.subscription_id)
            .await
        {
            Ok(sub) => sub,
            Err(maidan_store::StoreError::NotFound) => {
                // Returning here would stall every later row in the batch on
                // each poll, and the row itself can never be delivered.
                crate::webhooks::forget_webhook_secret(
                    &state.webhooks.secrets,
                    delivery.subscription_id,
                );
                let _ = state.store.quarantine_webhook_delivery(delivery.id).await;
                warn!(
                    delivery_id = delivery.id,
                    webhook = %delivery.subscription_id,
                    "quarantined delivery for deleted webhook subscription"
                );
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        if sub.subscription.revoked_at.is_some() {
            // The pending query does not read `revoked_at`, so a revoked
            // hook's queued retries would otherwise keep going out. Resolving
            // its secret first would also put it back in the cache.
            crate::webhooks::forget_webhook_secret(
                &state.webhooks.secrets,
                delivery.subscription_id,
            );
            let _ = state.store.quarantine_webhook_delivery(delivery.id).await;
            warn!(
                delivery_id = delivery.id,
                webhook = %delivery.subscription_id,
                "quarantined delivery for revoked webhook"
            );
            continue;
        }
        // A delivery queued while its creator held authority does not go out
        // after it ended: a retry can come hours after the event.
        // Nor does one whose event its creator can no longer read, such as
        // a message in a private channel the creator has since left.
        let readable = match creator_or_retire(state, &sub).await {
            Ok(creator) => creator_may_read(state, &creator, delivery.log_id).await,
            Err(NotActing::Ended(_)) => Some(false),
            Err(NotActing::Held(_)) => None,
        };
        match readable {
            Some(true) => {}
            Some(false) => {
                let _ = state.store.quarantine_webhook_delivery(delivery.id).await;
                continue;
            }
            None => {
                let until = chrono::Utc::now() + chrono::Duration::seconds(CREATOR_HELD_DEFER_SECS);
                if let Err(err) = state.store.defer_webhook_delivery(delivery.id, until).await {
                    warn!(delivery_id = delivery.id, error = %err, "webhook deferral failed");
                }
                continue;
            }
        }
        let Some(secret) = resolve_webhook_secret(
            &state.webhooks,
            delivery.subscription_id,
            &sub.secret_ciphertext,
        ) else {
            // A bare `continue` left the row due with no attempt recorded, so
            // it came back first in every batch, forever. Count it as a failed
            // attempt and let the ceiling quarantine it.
            let err = "missing or undecryptable webhook secret";
            warn!(
                delivery_id = delivery.id,
                webhook = %delivery.subscription_id,
                "{err}"
            );
            let next = chrono::Utc::now() + delivery_backoff(delivery.attempts);
            let attempts = state
                .store
                .record_webhook_delivery_attempt(delivery.id, err, next)
                .await
                .map_err(|e| e.to_string())?;
            if attempts >= max_attempts as i32 {
                let _ = state.store.quarantine_webhook_delivery(delivery.id).await;
                warn!(
                    delivery_id = delivery.id,
                    attempts, max_attempts, "webhook delivery quarantined (undecryptable secret)"
                );
            }
            continue;
        };
        let workspace_id = sub.subscription.workspace_id;
        // The hard allowance cutoff: an open sign-up workspace past its
        // deliveries allowance sends nothing. The delivery is deferred, not
        // failed, so it keeps its attempt count and goes out once the cutoff
        // lifts.
        if state
            .usage_meters
            .is_cut_off(workspace_id, maidan_types::UsageMeter::Deliveries)
        {
            let until = chrono::Utc::now() + crate::usage_meters::CUT_OFF_DEFER;
            if let Err(err) = state.store.defer_webhook_delivery(delivery.id, until).await {
                warn!(delivery_id = delivery.id, error = %err, "webhook deferral failed");
            }
            tracing::info!(
                delivery_id = delivery.id,
                workspace_id = %workspace_id.0,
                reason = "over_allowance",
                "webhook delivery parked; workspace past its free allowance for deliveries"
            );
            continue;
        }
        if let Some(host) = crate::retry_budget::host_of(&sub.subscription.url) {
            let attempt = Attempt::after(i64::from(delivery.attempts));
            if let Some(until) = deferred_until(&state.retry_budget, "webhook", &host, attempt) {
                if let Err(err) = state.store.defer_webhook_delivery(delivery.id, until).await {
                    // The claim's lease holds the row, so the budget is asked
                    // again once the lease runs out.
                    warn!(delivery_id = delivery.id, error = %err, "webhook deferral failed");
                }
                continue;
            }
        }
        let kind = event_kind_from_payload(&delivery.payload);
        // Egress SecretBroker: substitute `secret://` refs with real values
        // only when the target host is allowlisted; the plaintext is resolved
        // at send time and never persisted.
        let body = crate::secret_broker::substitute_for_egress(
            state,
            sub.subscription.workspace_id,
            &sub.subscription.url,
            &delivery.payload,
        )
        .await;
        let room_lsn =
            crate::room_lsn::current_for_room(state.store.as_ref(), sub.subscription.workspace_id)
                .await;
        let sent = maidan_store::trace::maybe_scope(
            delivery.trace.clone(),
            deliver_http(
                &sub.subscription.url,
                delivery.id,
                kind,
                &secret,
                &body,
                room_lsn,
            ),
        )
        .await;
        // Every attempt is outbound HTTP the workspace made the server send,
        // so a failed one counts too.
        state
            .usage_meters
            .record(workspace_id, maidan_types::UsageMeter::Deliveries, 1);
        match sent {
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
