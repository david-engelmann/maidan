//! Subscribes to the event bus and writes per-recipient notification rows
//! (Cluster 238, Program C — Arc G). Where the webhook worker fans events to
//! per-workspace HTTP sinks, this resolves an event to the *members* it concerns
//! and writes one `maidan_notifications` row each — the per-recipient delivery
//! layer the unified inbox reads. Currently routes @mentions; preferences +
//! follows (Arc H) and more event kinds layer on later.
//!
//! Every server replica runs this consumer, so the same event reaches each; the
//! write goes through `create_notification_if_absent` (unique on
//! `(member_id, source_log_id)`), so a replay or a second replica cannot
//! double-notify.

use std::time::Duration;

use maidan_bus::{BusItem, EventStream};
use maidan_types::{Event, EventFilter, EventKind, NewNotification};
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tracing::{info, warn};

use crate::state::AppState;

const RECONNECT_INITIAL: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(5);

pub struct NotificationRouter {
    shutdown: watch::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl NotificationRouter {
    pub fn spawn(state: AppState) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let handle = tokio::spawn(async move {
            run_bus_consumer(state, shutdown_rx).await;
        });
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

async fn run_bus_consumer(state: AppState, mut shutdown: watch::Receiver<()>) {
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    let stop_forward = stop_tx.clone();
    tokio::spawn(async move {
        let _ = shutdown.changed().await;
        let _ = stop_forward.send(()).await;
    });

    let mut backoff = RECONNECT_INITIAL;
    loop {
        let stream = match state.bus.subscribe(EventFilter::all()).await {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, ?backoff, "notification router bus subscribe failed; retrying");
                if tokio::time::timeout(backoff, stop_rx.recv()).await.is_ok() {
                    return;
                }
                backoff = (backoff * 2).min(RECONNECT_MAX);
                continue;
            }
        };
        backoff = RECONNECT_INITIAL;
        info!("notification router attached to bus");
        if consume_bus(stream, &state, &mut stop_rx).await {
            return;
        }
        warn!("notification router bus stream ended; resubscribing");
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
                        if let Err(err) = route_event(state, envelope.log_id, &envelope.event).await {
                            warn!(error = %err, "notification routing failed");
                        }
                    }
                    Some(BusItem::Lagged { skipped }) => {
                        warn!(skipped, "notification router bus subscriber lagged");
                    }
                    None => return false,
                }
            }
            _ = stop_rx.recv() => {
                info!("notification router bus consumer shutdown");
                return true;
            }
        }
    }
}

/// Resolve an event to the members it concerns and write a per-recipient
/// notification row for each. Currently: `MentionRecorded` → the mentioned member.
/// Deduped on `(member_id, source_log_id)` via `create_notification_if_absent`, so
/// event replays and multiple replicas don't double-notify.
pub async fn route_event(state: &AppState, log_id: i64, event: &Event) -> Result<(), String> {
    if let Event::MentionRecorded {
        workspace_id,
        thread_id,
        message_id,
        member_id,
        ..
    } = event
    {
        // The mention event carries no channel; resolve it (best-effort) so the
        // inbox can render + RBAC-scope the notification.
        let channel_id = state
            .store
            .get_thread(*thread_id)
            .await
            .ok()
            .map(|t| t.channel_id);
        let new = NewNotification {
            workspace_id: *workspace_id,
            member_id: *member_id,
            kind: EventKind::MentionRecorded,
            source_log_id: log_id,
            channel_id,
            thread_id: Some(*thread_id),
            message_id: Some(*message_id),
            actor_id: None,
        };
        let created = state
            .store
            .create_notification_if_absent(new)
            .await
            .map_err(|e| e.to_string())?;
        if created.is_some() {
            crate::metrics::record_notification_created(EventKind::MentionRecorded.as_str());
        }
    }
    Ok(())
}
