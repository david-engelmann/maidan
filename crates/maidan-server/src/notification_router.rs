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
        Event::ClaimExpired {
            workspace_id,
            channel_id,
            thread_id,
            member_id,
            thread,
            ..
        } => {
            // W1 (Cluster 355): an expired claim means the task is stuck — its
            // holder's lease lapsed and it was reclaimed. If the thread has a
            // durable owner, notify them so they can re-steer or reassign. The
            // dead holder is the actor. Un-owned threads notify no one (the
            // occupancy view / `wait_for_claim_expired` already surface expiry).
            if let Some(owner_id) = thread.owner_id {
                notify(
                    state,
                    *workspace_id,
                    owner_id,
                    EventKind::ClaimExpired,
                    log_id,
                    Some(*channel_id),
                    Some(*thread_id),
                    None,
                    Some(*member_id),
                )
                .await?;
            }
        }
        Event::ThreadLanded {
            workspace_id,
            channel_id,
            thread_id,
            ..
        } => {
            // G-dev-7 (Cluster 361): the work landed (its linked PR merged). Reach
            // the people accountable for or watching the thread — its durable owner
            // (if any) plus its followers. Per-recipient mutes are honored by
            // `notify`. No member actor: the merger is a GitHub login, not a member.
            // A PR merge is infrequent (not a hot path like MessagePosted), so a
            // per-recipient loop over the small union is fine.
            let mut recipients: HashSet<MemberId> = HashSet::new();
            if let Ok(thread) = state.store.get_thread(*thread_id).await {
                if let Some(owner_id) = thread.owner_id {
                    recipients.insert(owner_id);
                }
            }
            match state.store.thread_followers(*thread_id).await {
                Ok(followers) => recipients.extend(followers),
                Err(err) => tracing::warn!(error = %err, "thread_landed: follower lookup failed"),
            }
            for member_id in recipients {
                notify(
                    state,
                    *workspace_id,
                    member_id,
                    EventKind::ThreadLanded,
                    log_id,
                    Some(*channel_id),
                    Some(*thread_id),
                    None,
                    None,
                )
                .await?;
            }
        }
        Event::WaitTimedOut {
            workspace_id,
            channel_id,
            thread_id,
            ..
        } => {
            // G2/G4 (Cluster 364): a thread's wait timed out unsatisfied — the room
            // must reach a human (never invent a decision). Notify the thread's
            // durable owner (the accountable party) if one is set. Mute-honoring via
            // `notify`; no member actor (the timer fired).
            if let Ok(thread) = state.store.get_thread(*thread_id).await {
                if let Some(owner_id) = thread.owner_id {
                    notify(
                        state,
                        *workspace_id,
                        owner_id,
                        EventKind::WaitTimedOut,
                        log_id,
                        Some(*channel_id),
                        Some(*thread_id),
                        None,
                        None,
                    )
                    .await?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Fan a `MessagePosted` out to its followers (Cluster 344 de-serialized this off
/// the router; Cluster 349 collapsed it to a batch). The router is a serial bus
/// consumer, so a widely followed message must not head-of-line-block the pipeline:
/// mutes resolve in one query (Cluster 348) and the unmuted set is written in one
/// batch insert, so a fan-out to N followers is ~2 store round trips regardless of
/// N. A store error short-circuits (matching the prior `?`-in-loop behaviour).
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
    // Cluster 348: resolve mutes for the whole recipient set in ONE query (was one
    // `is_notification_muted` per recipient). Cluster 349: write the unmuted set in
    // ONE `INSERT … ON CONFLICT DO NOTHING RETURNING` (was one insert per recipient,
    // concurrently). Together this collapses the fan-out to ~2 store round trips
    // (mute filter + batch insert) regardless of follower count. `create_notifications_batch`
    // returns only the rows it actually inserted, so we meter + email exactly the new
    // notifications (a dedup collision from a replay / second replica is skipped).
    let recipients: Vec<MemberId> = recipients.into_iter().collect();
    let muted: HashSet<MemberId> = state
        .store
        .filter_muted_members(EventKind::MessagePosted, &recipients)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    for _ in &muted {
        crate::metrics::record_notification_suppressed("muted");
    }
    // Leaf mute (Cluster 356, F7): members who muted this thread are dropped from the
    // fan-out in one batch query, alongside the kind-mute filter above.
    let thread_muted: HashSet<MemberId> = state
        .store
        .thread_muters(thread_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    for m in &thread_muted {
        if !muted.contains(m) && recipients.contains(m) {
            crate::metrics::record_notification_suppressed("thread_muted");
        }
    }
    // Per-channel mute (Cluster 357, N3): members who muted this channel are dropped
    // too — a `MessagePosted` is the firehose that channel mute silences (a mention,
    // which breaks through, is a distinct `MentionRecorded` event, not this path).
    let channel_muted: HashSet<MemberId> = state
        .store
        .channel_muters(channel_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    for m in &channel_muted {
        if !muted.contains(m) && !thread_muted.contains(m) && recipients.contains(m) {
            crate::metrics::record_notification_suppressed("channel_muted");
        }
    }
    let new_rows: Vec<NewNotification> = recipients
        .into_iter()
        .filter(|m| !muted.contains(m) && !thread_muted.contains(m) && !channel_muted.contains(m))
        .map(|member_id| NewNotification {
            workspace_id,
            member_id,
            kind: EventKind::MessagePosted,
            source_log_id,
            channel_id: Some(channel_id),
            thread_id: Some(thread_id),
            message_id: Some(message_id),
            actor_id: Some(author_id),
        })
        .collect();
    if new_rows.is_empty() {
        return Ok(());
    }
    let created = state
        .store
        .create_notifications_batch(&new_rows)
        .await
        .map_err(|e| e.to_string())?;
    for n in &created {
        crate::metrics::record_notification_created(n.kind.as_str());
        // Off-platform email (Cluster 249), only when a transport is configured —
        // spawned so a slow SMTP send never blocks routing (best-effort, not retried).
        if state.mail.is_some() {
            let st = state.clone();
            let (member_id, kind, log_id) = (n.member_id, n.kind, n.source_log_id);
            tokio::spawn(async move {
                deliver_notification_email(&st, member_id, kind, log_id).await;
            });
        }
    }
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
    // Leaf mute (Cluster 356, F7): a member who muted this specific thread is not
    // notified about it, even for an otherwise-unmuted kind. A thread mute is the
    // strongest scope — it suppresses even a mention (you're done with this thread).
    if let Some(tid) = thread_id {
        if state
            .store
            .is_thread_muted(member_id, tid)
            .await
            .map_err(|e| e.to_string())?
        {
            crate::metrics::record_notification_suppressed("thread_muted");
            return Ok(false);
        }
    }
    // Per-channel mute (Cluster 357, N3): a member who muted this channel is not
    // notified about its firehose — EXCEPT a `MentionRecorded` breaks through (you
    // muted the noise but still want to be named). The thread mute above already
    // covered the "even mentions" case; an explicit kind mute (top) always wins.
    if kind != EventKind::MentionRecorded {
        if let Some(cid) = channel_id {
            if state
                .store
                .is_channel_muted(member_id, cid)
                .await
                .map_err(|e| e.to_string())?
            {
                crate::metrics::record_notification_suppressed("channel_muted");
                return Ok(false);
            }
        }
    }
    write_notification(
        state,
        workspace_id,
        member_id,
        kind,
        source_log_id,
        channel_id,
        thread_id,
        message_id,
        actor_id,
    )
    .await
}

/// Write one per-recipient notification for an **already-unmuted** recipient
/// (Cluster 348) — the tail of [`notify`], split out so the `MessagePosted`
/// fan-out can batch the mute check once and then write concurrently. Returns
/// whether a row was written (a dedup collision returns `false`); on a new row,
/// meters it and best-effort-spawns the off-platform email (Cluster 249).
#[allow(clippy::too_many_arguments)]
async fn write_notification(
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
