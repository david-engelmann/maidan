//! Cluster 377.2: the projector-egress worker. A queued delivery is posted by
//! `egress_worker::sweep_once` and not re-sent; a failed post is *rescheduled*
//! (not dropped, not dead-lettered on the first failure), so a transient Slack or
//! GitHub outage survives where the old inline post lost the message; an
//! undecodable destination dead-letters immediately instead of burning eight
//! attempts; and a deployment with no projector sender leaves the queue alone.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{BusItem, InMemoryBus};
use maidan_server::{
    egress_worker,
    slack::{route_message_to_slack, SlackError, SlackSender},
    AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressTarget, Event, EventFilter, EventKind, ExternalRef, MemberKind, NewChannel,
    NewEgressOutbox, NewMember, NewMessage, NewSlackChannelLink, NewThread, NewWorkspace, ThreadId,
    WorkspaceId,
};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

struct CountingSlack {
    attempts: AtomicUsize,
    /// What every post returns: `None` succeeds.
    fail_with: Option<SlackError>,
    sent: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl SlackSender for CountingSlack {
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        assert_eq!(thread_ts, None, "the projector egress posts top-level");
        match &self.fail_with {
            Some(SlackError::Http(m)) => return Err(SlackError::Http(m.clone())),
            Some(SlackError::Api(m)) => return Err(SlackError::Api(m.clone())),
            None => {}
        }
        let mut sent = self.sent.lock().unwrap();
        sent.push((channel.into(), text.into()));
        Ok(Some(ExternalRef::Slack {
            channel_id: channel.into(),
            ts: format!("17000000{:02}.000100", sent.len()),
        }))
    }

    async fn update_message(
        &self,
        _channel: &str,
        _ts: &str,
        _text: &str,
    ) -> Result<(), SlackError> {
        unreachable!("the projector egress never updates; that is Cluster 379.4")
    }
}

fn slack_sender(fail_with: Option<SlackError>) -> Arc<CountingSlack> {
    Arc::new(CountingSlack {
        attempts: AtomicUsize::new(0),
        fail_with,
        sent: Mutex::new(Vec::new()),
    })
}

/// An `AppState` over an in-memory store, with `sender` attached when given. A
/// `None` sender is a deployment with the projector turned off. The pool comes
/// back too, so one test can write a row `EgressTarget` cannot construct.
async fn state_with(sender: Option<Arc<CountingSlack>>) -> (AppState, Arc<dyn Store>, SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    if let Some(sender) = sender {
        state.attach_slack_sender(sender);
    }
    (state, store, pool)
}

/// A workspace + channel + thread for the outbox's foreign keys.
async fn scope(store: &dyn Store) -> (WorkspaceId, ThreadId) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    (ws.id, thread.id)
}

fn queued(ws: WorkspaceId, thread: ThreadId, log_id: i64) -> NewEgressOutbox {
    NewEgressOutbox {
        workspace_id: ws,
        thread_id: thread,
        source_log_id: log_id,
        target: EgressTarget::Slack {
            channel_id: "C1".into(),
        },
        body: "hi from maidan".into(),
        kind: maidan_types::EgressKind::Projector,
    }
}

#[tokio::test]
async fn worker_delivers_a_queued_message_once() {
    let sender = slack_sender(None);
    let (state, store, _pool) = state_with(Some(sender.clone())).await;
    let (ws, thread) = scope(store.as_ref()).await;
    store.enqueue_egress(queued(ws, thread, 1)).await.unwrap();

    let stats = egress_worker::sweep_once(&state).await;
    assert_eq!(stats.sent, 1);
    assert_eq!(sender.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        sender.sent.lock().unwrap().as_slice(),
        [("C1".to_string(), "hi from maidan".to_string())]
    );

    // Delivered -> not re-claimed on the next sweep.
    let again = egress_worker::sweep_once(&state).await;
    assert_eq!(again.sent, 0);
    assert_eq!(
        sender.attempts.load(Ordering::SeqCst),
        1,
        "a delivered message is not re-posted"
    );
}

#[tokio::test]
async fn worker_reschedules_on_failure_instead_of_dropping() {
    let sender = slack_sender(Some(SlackError::Http("simulated outage".into())));
    let (state, store, _pool) = state_with(Some(sender.clone())).await;
    let (ws, thread) = scope(store.as_ref()).await;
    store.enqueue_egress(queued(ws, thread, 1)).await.unwrap();

    // A first failure reschedules (attempts 1 < max), it does not dead-letter —
    // this is the whole point of 377.2: the old inline post dropped it here.
    let stats = egress_worker::sweep_once(&state).await;
    assert_eq!(stats.retried, 1);
    assert_eq!(stats.dead, 0);
    assert_eq!(sender.attempts.load(Ordering::SeqCst), 1);

    // Rescheduled with backoff (~30s out), so an immediate re-sweep does nothing.
    let again = egress_worker::sweep_once(&state).await;
    assert_eq!(again.retried, 0);
    assert_eq!(again.sent, 0);
    assert_eq!(
        sender.attempts.load(Ordering::SeqCst),
        1,
        "not retried until the backoff elapses"
    );
    assert_eq!(
        store.count_dead_egress().await.unwrap(),
        0,
        "a single failure never dead-letters"
    );
}

#[tokio::test]
async fn an_undecodable_destination_dead_letters_without_a_post() {
    let sender = slack_sender(None);
    let (state, store, pool) = state_with(Some(sender.clone())).await;
    let (ws, thread) = scope(store.as_ref()).await;
    // A GitHub selector with no issue number cannot address anything. Written
    // directly, since `EgressTarget` can't construct one — this is the shape a
    // downgrade or a hand-edited row would leave behind.
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_egress_outbox
           (id, workspace_id, thread_id, source_log_id, surface, selector, body,
            status, attempts, next_attempt_at, created_at, updated_at)
         VALUES (?, ?, ?, 1, 'github', 'not-a-repo', 'b', 'pending', 0, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(ws.0)
    .bind(thread.0)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let stats = egress_worker::sweep_once(&state).await;
    assert_eq!(stats.dead, 1, "an unroutable row dead-letters on sight");
    assert_eq!(stats.retried, 0, "and is never rescheduled");
    assert_eq!(sender.attempts.load(Ordering::SeqCst), 0);
    assert_eq!(store.count_dead_egress().await.unwrap(), 1);
}

/// Cluster 377.3: retry-then-disable. An auth/config-class failure is not
/// retried — the link is turned off, the delivery dead-letters, a
/// `ProjectorMisconfigured` says so, and later messages stop queueing.
#[tokio::test]
async fn a_misconfigured_link_is_disabled_announced_and_stops_queueing() {
    let sender = slack_sender(Some(SlackError::Api("channel_not_found".into())));
    let (state, store, _pool) = state_with(Some(sender.clone())).await;
    let (ws, thread) = scope(store.as_ref()).await;
    let agent = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store.get_thread(thread).await.unwrap().channel_id;
    store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C1".into(),
            workspace_id: ws,
            channel_id: channel,
            thread_id: thread,
            member_id: agent.id,
        })
        .await
        .unwrap();
    let mut events = state
        .bus
        .subscribe(EventFilter::all().with_kinds([EventKind::ProjectorMisconfigured]))
        .await
        .unwrap();

    store.enqueue_egress(queued(ws, thread, 1)).await.unwrap();
    let stats = egress_worker::sweep_once(&state).await;
    assert_eq!(stats.disabled, 1, "the link was turned off");
    assert_eq!(stats.dead, 1, "and the delivery dead-lettered");
    assert_eq!(stats.retried, 0, "a revoked destination is never retried");
    assert_eq!(sender.attempts.load(Ordering::SeqCst), 1, "posted once");

    // The link is disabled, naming when.
    let link = store
        .get_slack_channel_link("C1")
        .await
        .unwrap()
        .expect("link");
    assert!(link.disabled_at.is_some());

    // ...and it was announced, with the operator's actual diagnostic.
    let item = tokio::time::timeout(std::time::Duration::from_secs(5), events.next())
        .await
        .expect("timeout waiting for ProjectorMisconfigured")
        .expect("subscriber ended without event");
    let BusItem::Event(envelope) = item else {
        panic!("expected event, got lag or end");
    };
    match envelope.event {
        Event::ProjectorMisconfigured {
            workspace_id,
            channel_id,
            thread_id,
            surface,
            selector,
            error,
            ..
        } => {
            assert_eq!(workspace_id, ws);
            assert_eq!(channel_id, Some(channel));
            assert_eq!(thread_id, thread);
            assert_eq!(surface, "slack");
            assert_eq!(selector, "C1");
            assert!(error.contains("channel_not_found"), "got {error}");
        }
        other => panic!("expected ProjectorMisconfigured, got {other:?}"),
    }
    // Durable too, not just a live notification.
    let stored = store
        .get_stored_event(envelope.log_id)
        .await
        .expect("event logged");
    assert_eq!(stored.kind, EventKind::ProjectorMisconfigured);
    assert_eq!(stored.payload["selector"], "C1");

    // A later message into the disabled link does not queue at all, so the
    // dead-letter queue stops growing one row per message.
    let message = store
        .post_message_with_event(
            NewMessage {
                thread_id: thread,
                author_id: agent.id,
                body: "after the break".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap()
        .0;
    route_message_to_slack(&state, 2, thread, &message).await;
    assert_eq!(
        egress_worker::sweep_once(&state).await,
        egress_worker::EgressSweepStats::default(),
        "nothing was queued for a disabled link"
    );
    assert_eq!(store.count_dead_egress().await.unwrap(), 1);

    // Re-linking is the re-enable path — no separate route.
    store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C1".into(),
            workspace_id: ws,
            channel_id: channel,
            thread_id: thread,
            member_id: agent.id,
        })
        .await
        .unwrap();
    assert!(store
        .get_slack_channel_link("C1")
        .await
        .unwrap()
        .expect("link")
        .disabled_at
        .is_none());
    route_message_to_slack(&state, 3, thread, &message).await;
    assert_eq!(
        egress_worker::sweep_once(&state).await.disabled,
        1,
        "a re-linked channel queues and delivers again (and breaks again here)"
    );
}

#[tokio::test]
async fn a_deployment_without_a_projector_sender_leaves_the_queue_alone() {
    let (state, store, _pool) = state_with(None).await;
    let (ws, thread) = scope(store.as_ref()).await;
    store.enqueue_egress(queued(ws, thread, 1)).await.unwrap();

    assert_eq!(
        egress_worker::sweep_once(&state).await,
        egress_worker::EgressSweepStats::default()
    );
    assert_eq!(
        store.count_dead_egress().await.unwrap(),
        0,
        "an unconfigured projector must not burn the queue's attempts"
    );
    // The row is untouched, so configuring a sender later still delivers it.
    assert!(store
        .claim_next_due_egress(chrono::Utc::now(), 300)
        .await
        .unwrap()
        .is_some());
}
