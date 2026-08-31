//! Subscribes to the event bus and writes per-recipient notification rows
//! (Cluster 238, Program C — Arc G/H). Where the webhook worker fans events to
//! per-workspace HTTP sinks, this resolves an event to the *members* it concerns
//! and writes one `maidan_notifications` row each — the per-recipient delivery
//! layer the unified inbox reads. Routes @mentions (Cluster 238) and, for
//! followers, new messages in a followed channel/thread (Cluster 245), honoring
//! each recipient's mute preferences (Cluster 242).
//!
//! Every server replica runs this consumer, so the same event reaches each; the
//! write goes through `create_notification_if_absent` (unique on
//! `(member_id, source_log_id)`), so a replay or a second replica cannot
//! double-notify. A `MentionRecorded` and a `MessagePosted` are distinct events
//! (distinct `log_id`s), so a member mentioned in a channel they *also* follow
//! gets both a mention notification and a message-posted one — per-kind mute
//! (`message_posted`) is the control for follow-noise.

use std::collections::HashSet;
use std::time::Duration;

use maidan_bus::{BusItem, EventStream};
use maidan_types::{
    ChannelId, Event, EventFilter, EventKind, MemberId, MessageId, NewNotification, ThreadId,
    WorkspaceId,
};
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tracing::{info, warn};

use crate::state::AppState;

const RECONNECT_INITIAL: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(5);

/// Max concurrent per-recipient notification writes for one `MessagePosted`
/// fan-out (Cluster 344). The router is a serial bus consumer, so a widely
/// followed message previously head-of-line-blocked the whole pipeline on
/// `2 × followers` sequential store round-trips; a bounded `buffer_unordered`
/// de-serializes it while capping pool fan-out (mirrors Cluster 199).
const NOTIFY_FANOUT_CONCURRENCY: usize = 8;

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
/// notification row for each — `MentionRecorded` → the mentioned member (Cluster
/// 238); `MessagePosted` → the followers of its channel/thread minus the author
/// (Cluster 245). Each write is mute-checked (Cluster 242) and deduped on
/// `(member_id, source_log_id)`, so event replays and multiple replicas don't
/// double-notify.
pub async fn route_event(state: &AppState, log_id: i64, event: &Event) -> Result<(), String> {
    match event {
        Event::MentionRecorded {
            workspace_id,
            thread_id,
            message_id,
            member_id,
            ..
        } => {
            // The mention event carries no channel; resolve it (best-effort) so the
            // inbox can render + RBAC-scope the notification.
            let channel_id = state
                .store
                .get_thread(*thread_id)
                .await
                .ok()
                .map(|t| t.channel_id);
            notify(
                state,
                *workspace_id,
                *member_id,
                EventKind::MentionRecorded,
                log_id,
                channel_id,
                Some(*thread_id),
                Some(*message_id),
                None,
            )
            .await?;
        }
        Event::MessagePosted {
            workspace_id,
            channel_id,
            thread_id,
            dm_conversation_id,
            message,
            ..
        } => {
            // DMs live in the shared `__dm__` channel and aren't "followed" — skip.
            if dm_conversation_id.is_some() {
                return Ok(());
            }
            // Slack projector egress (Cluster 309): if this thread is linked to a
            // Slack channel and the message didn't originate in Slack, relay it out.
            // Best-effort + a no-op unless a Slack sender is configured.
            crate::slack::route_message_to_slack(state, *thread_id, message).await;
            // GitHub projector egress (Cluster 312): same, for a linked issue/PR.
            crate::github::route_message_to_github(state, *thread_id, message).await;
            // Followers of the channel and/or the thread, minus the author (you
            // don't get notified of your own message). The set dedups a member who
            // follows both; the DB unique index is the cross-replica backstop.
            let mut recipients: HashSet<MemberId> = HashSet::new();
            for m in state
                .store
                .channel_followers(*channel_id)
                .await
                .map_err(|e| e.to_string())?
            {
                recipients.insert(m);
            }
            for m in state
                .store
                .thread_followers(*thread_id)
                .await
                .map_err(|e| e.to_string())?
            {
                recipients.insert(m);
            }
            recipients.remove(&message.author_id);
            fan_out_message_posted(
                state,
                *workspace_id,
                recipients,
                log_id,
                *channel_id,
                *thread_id,
                message.id,
                message.author_id,
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

/// Fan a `MessagePosted` out to its followers with bounded concurrency
/// (Cluster 344) — the serial per-recipient `notify` loop head-of-line-blocked
/// the router. Order is irrelevant (each write targets a distinct recipient row),
/// so `buffer_unordered` is used; a store error on any recipient short-circuits
/// (matching the prior `?`-in-loop behaviour). Futures imports are function-scoped
/// so the module's `tokio_stream::StreamExt` (the bus `.next()`) stays unambiguous.
#[allow(clippy::too_many_arguments)]
async fn fan_out_message_posted(
    state: &AppState,
    workspace_id: WorkspaceId,
    recipients: HashSet<MemberId>,
    source_log_id: i64,
    channel_id: ChannelId,
    thread_id: ThreadId,
    message_id: MessageId,
    author_id: MemberId,
) -> Result<(), String> {
    use futures::stream::{self, StreamExt, TryStreamExt};
    // Map on the plain iterator (Iterator::map) — mapping on the stream would be
    // ambiguous between the module's tokio_stream::StreamExt and futures::StreamExt.
    let writes = recipients.into_iter().map(|member_id| {
        notify(
            state,
            workspace_id,
            member_id,
            EventKind::MessagePosted,
            source_log_id,
            Some(channel_id),
            Some(thread_id),
            Some(message_id),
            Some(author_id),
        )
    });
    stream::iter(writes)
        .buffer_unordered(NOTIFY_FANOUT_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(())
}

/// Write one per-recipient notification unless the recipient has muted `kind`
/// (Cluster 242). Returns whether a row was written (a mute or a dedup collision
/// returns `false`).
#[allow(clippy::too_many_arguments)]
async fn notify(
    state: &AppState,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    kind: EventKind,
    source_log_id: i64,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    message_id: Option<MessageId>,
    actor_id: Option<MemberId>,
) -> Result<bool, String> {
    if state
        .store
        .is_notification_muted(member_id, kind)
        .await
        .map_err(|e| e.to_string())?
    {
        crate::metrics::record_notification_suppressed("muted");
        return Ok(false);
    }
    let created = state
        .store
        .create_notification_if_absent(NewNotification {
            workspace_id,
            member_id,
            kind,
            source_log_id,
            channel_id,
            thread_id,
            message_id,
            actor_id,
        })
        .await
        .map_err(|e| e.to_string())?;
    if created.is_some() {
        crate::metrics::record_notification_created(kind.as_str());
        // Off-platform email (Cluster 249), only when a transport is configured.
        // Spawned so a slow/failing SMTP send never blocks event routing —
        // best-effort (a failure is logged + metered, not retried).
        if state.mail.is_some() {
            let st = state.clone();
            tokio::spawn(async move {
                deliver_notification_email(&st, member_id, kind, source_log_id).await;
            });
        }
    }
    Ok(created.is_some())
}

/// The "recently active" window for presence-aware email routing (Cluster 253),
/// in seconds, from `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS`. When a positive value is
/// set, a notification email is skipped if the recipient was last seen within the
/// window — they are online and will see the in-app notification, so the email
/// would be redundant. Unset or `0` disables the guard: every opted-in recipient
/// is emailed, the Cluster-249 behaviour (so this is a zero-change opt-in). Read
/// per call — cheap, and the send is already off the event-routing hot path.
fn presence_skip_window_secs() -> Option<i64> {
    std::env::var("MAIDAN_EMAIL_PRESENCE_WINDOW_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&s| s > 0)
}

/// Deliver one notification to a member by email, if a transport is configured and
/// the member has a delivery address on file (Cluster 249). Best-effort: a send
/// failure is logged + metered, never retried (a durable retrying queue is a
/// follow-up). Extracted so a test can await it directly rather than racing the
/// spawned task in [`notify`].
pub async fn deliver_notification_email(
    state: &AppState,
    member_id: MemberId,
    kind: EventKind,
    source_log_id: i64,
) {
    // Only enqueue when a transport is configured — the mail_worker (Cluster 305)
    // does the actual send, so a queue with no sender would just pile up.
    if state.mail.is_none() {
        return;
    }
    let address = match state.store.get_member_email(member_id).await {
        Ok(Some(a)) => a.email,
        Ok(None) => return, // member hasn't opted in / provided an address
        Err(err) => {
            warn!(error = %err, "notification email: address lookup failed");
            return;
        }
    };
    // Digest mode (Cluster 255): a member in digest mode gets a periodic rollup
    // from the sweeper instead of a per-notification email — the two are mutually
    // exclusive, so suppress the immediate send here. A lookup error falls through
    // and sends (the immediate email is the safer default on an uncertain mode).
    match state.store.get_delivery_mode(member_id).await {
        Ok(maidan_types::EmailDeliveryMode::Digest) => {
            crate::metrics::record_email_delivered("skipped_digest");
            return;
        }
        Ok(maidan_types::EmailDeliveryMode::Immediate) => {}
        Err(err) => {
            warn!(error = %err, "notification email: delivery-mode lookup failed");
        }
    }
    // Presence-aware routing (Cluster 253): if the recipient was seen within the
    // configured window, skip the email — they are active and will see the in-app
    // notification. A negative idle (clock skew, last-seen in the future) counts
    // as active too. A lookup error falls through and sends (never drop an email
    // over a transient read). Opt-in: unset/0 window sends as before.
    if let Some(window_secs) = presence_skip_window_secs() {
        match state.store.get_member_last_seen(member_id).await {
            Ok(Some(last_seen)) => {
                let idle = chrono::Utc::now().signed_duration_since(last_seen);
                if idle.num_seconds() < window_secs {
                    crate::metrics::record_email_delivered("skipped_present");
                    return;
                }
            }
            Ok(None) => {} // never seen -> not active -> send
            Err(err) => {
                warn!(error = %err, "notification email: last-seen lookup failed");
            }
        }
    }
    let subject = "New Maidan notification".to_string();
    let body = format!(
        "You have a new notification in Maidan ({}). Open Maidan to view it.\n\n\
         (event #{})",
        kind.as_str(),
        source_log_id
    );
    // Durable delivery (Cluster 305): enqueue to the mail outbox and let the
    // mail_worker send with retry/backoff + dead-lettering, instead of a
    // best-effort send that drops the email on a transient SMTP failure.
    match state
        .store
        .enqueue_mail(maidan_types::NewMailOutbox {
            to_address: address,
            subject,
            body,
        })
        .await
    {
        Ok(_) => crate::metrics::record_email_delivered("enqueued"),
        Err(err) => {
            warn!(error = %err, "notification email: enqueue failed");
            crate::metrics::record_email_delivered("failed");
        }
    }
}
