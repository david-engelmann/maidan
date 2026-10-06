//! A destination that recovers gets its backlog of retries at the retry
//! budget's pace, not all at once.
//!
//! Each worker here faces a host that has just come back: a queue of
//! deliveries that already failed once and are all due, plus a couple of
//! fresh ones. One pass sends at most the budget's burst of retries; the rest
//! are deferred with their attempt counts as they were, none dead-lettered,
//! and every first attempt still goes out. The webhook, automation, mail and
//! egress queues all live in the store, so each suite runs on SQLite and on
//! Postgres.

// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::{extract::State, http::StatusCode, routing::post, Router};
use chrono::{Duration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    automation_worker, egress_worker, mail_worker,
    retry_budget::{ManualClock, RetryBudget},
    slack::{SlackError, SlackSender},
    webhook_worker, AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::{
    AutomationSourceKind, EgressKind, EgressTarget, ExternalRef, NewAutomationDelivery, NewChannel,
    NewEgressOutbox, NewFsmHook, NewMailOutbox, NewThread, NewWebhookSubscription, NewWorkspace,
    SlashHandlerKind, ThreadState, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

/// The budget every suite runs under: three retries per host, and a clock
/// that never moves, so nothing refills during the pass.
const BURST: u32 = 3;
/// Deliveries that failed while the host was down, all due now.
const BACKLOG: usize = 8;
/// Deliveries created after the host came back.
const FRESH: usize = 2;

struct Backend {
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    _dir: tempfile::TempDir,
}

async fn sqlite() -> Backend {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Backend {
        store: Arc::new(SqliteStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::SqliteSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    }
}

fn state_on(backend: &Backend) -> AppState {
    let artifacts = Arc::new(LocalFsStore::new(backend._dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let mut state = AppState::for_tests(
        backend.store.clone(),
        artifacts,
        bus,
        backend.search.clone(),
    );
    state.retry_budget = Arc::new(RetryBudget::with_clock(
        BURST,
        1,
        Arc::new(ManualClock::new()),
    ));
    state
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

/// A receiver that has recovered: it answers every POST with 200 and counts them.
async fn receiver() -> (SocketAddr, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    async fn handle(State(hits): State<Arc<AtomicUsize>>) -> StatusCode {
        hits.fetch_add(1, Ordering::SeqCst);
        StatusCode::OK
    }
    let app = Router::new()
        .route("/hook", post(handle))
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, hits)
}

async fn webhook_suite(backend: &Backend) {
    std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1");
    let state = state_on(backend);
    let store = backend.store.as_ref();
    let ws = workspace(store, "hooks").await;
    let (addr, hits) = receiver().await;
    let sub = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws,
            url: format!("http://{addr}/hook"),
            label: None,
            event_kinds: vec!["message_posted".into()],
            secret_ciphertext: "x".into(),
        })
        .await
        .unwrap();
    maidan_server::webhooks::remember_webhook_secret(&state.webhooks.secrets, sub.id, "s".into());
    let mut backlog = Vec::new();
    for log_id in 0..BACKLOG as i64 {
        let id = store
            .enqueue_webhook_delivery(sub.id, log_id, "{}")
            .await
            .unwrap();
        store
            .record_webhook_delivery_attempt(id, "HTTP 503", Utc::now() - Duration::seconds(5))
            .await
            .unwrap();
        backlog.push(id);
    }
    for log_id in 0..FRESH as i64 {
        store
            .enqueue_webhook_delivery(sub.id, 100 + log_id, "{}")
            .await
            .unwrap();
    }

    webhook_worker::poll_deliveries(&state, 16).await.unwrap();

    assert_eq!(
        hits.load(Ordering::SeqCst),
        BURST as usize + FRESH,
        "the recovering receiver got the budgeted retries and every first attempt"
    );
    let pending = store
        .list_webhook_deliveries(ws, AutomationDeliveryFilter::Pending, 100)
        .await
        .unwrap();
    assert_eq!(pending.len(), BACKLOG - BURST as usize);
    for row in &pending {
        assert!(backlog.contains(&row.id), "only retries are deferred");
        assert_eq!(row.attempts, 1, "a deferral does not count as an attempt");
        assert!(row.next_attempt_at > Utc::now(), "deferred into the future");
    }
    assert!(store
        .list_webhook_deliveries(ws, AutomationDeliveryFilter::DeadLetter, 100)
        .await
        .unwrap()
        .is_empty());
}

async fn automation_suite(backend: &Backend) {
    std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1");
    let state = state_on(backend);
    let store = backend.store.as_ref();
    let ws = workspace(store, "auto").await;
    let (addr, hits) = receiver().await;
    let target = format!("http://{addr}/hook");
    let hook = store
        .create_fsm_hook(NewFsmHook {
            workspace_id: ws,
            label: None,
            from_state: Some(ThreadState::Open),
            to_state: Some(ThreadState::InReview),
            handler_kind: SlashHandlerKind::Http,
            handler_target: target.clone(),
            secret_ciphertext: "x".into(),
        })
        .await
        .unwrap();
    maidan_server::fsm_hooks::remember_fsm_secret(&state.fsm_hooks.secrets, hook.id, "s".into());
    let delivery = || NewAutomationDelivery {
        workspace_id: ws,
        source_kind: AutomationSourceKind::FsmHook,
        source_id: hook.id.0,
        target_url: target.clone(),
        header_name: "X-Maidan-Event".into(),
        header_value: "thread_state_changed".into(),
        payload: "{}".into(),
    };
    let mut backlog = Vec::new();
    for _ in 0..BACKLOG {
        let id = store.enqueue_automation_delivery(delivery()).await.unwrap();
        store
            .record_automation_delivery_attempt(id, "HTTP 503", Utc::now() - Duration::seconds(5))
            .await
            .unwrap();
        backlog.push(id);
    }
    for _ in 0..FRESH {
        store.enqueue_automation_delivery(delivery()).await.unwrap();
    }

    automation_worker::poll_once(&state, 16).await.unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), BURST as usize + FRESH);
    let pending = store
        .list_automation_deliveries(ws, AutomationDeliveryFilter::Pending, 100)
        .await
        .unwrap();
    assert_eq!(pending.len(), BACKLOG - BURST as usize);
    for row in &pending {
        assert!(backlog.contains(&row.id), "only retries are deferred");
        assert_eq!(row.attempts, 1, "a deferral does not count as an attempt");
        assert!(row.next_attempt_at > Utc::now(), "deferred into the future");
    }
    assert!(store
        .list_automation_deliveries(ws, AutomationDeliveryFilter::DeadLetter, 100)
        .await
        .unwrap()
        .is_empty());
}

struct RecoveredRelay {
    sent: AtomicUsize,
}

#[async_trait::async_trait]
impl maidan_server::mail::MailTransport for RecoveredRelay {
    async fn send(
        &self,
        _to: &str,
        _subject: &str,
        _body: &str,
    ) -> Result<(), maidan_server::mail::MailError> {
        self.sent.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn mail_suite(backend: &Backend) {
    let mut state = state_on(backend);
    let relay = Arc::new(RecoveredRelay {
        sent: AtomicUsize::new(0),
    });
    state.attach_mail(relay.clone());
    let store = backend.store.as_ref();
    let mail = |to: String| NewMailOutbox {
        workspace_id: None,
        source_log_id: None,
        to_address: to,
        subject: "s".into(),
        body: "b".into(),
    };
    let mut backlog = Vec::new();
    for i in 0..BACKLOG {
        let id = store
            .enqueue_mail(mail(format!("r{i}@example.com")))
            .await
            .unwrap()
            .unwrap();
        backlog.push(id);
    }
    // Each fails its first attempt while the relay is down.
    for _ in 0..BACKLOG {
        store
            .claim_next_due_mail(Utc::now(), 120)
            .await
            .unwrap()
            .unwrap();
    }
    for id in backlog {
        store
            .mark_mail_failed(id, "relay down", Some(Utc::now() - Duration::seconds(5)))
            .await
            .unwrap();
    }
    for i in 0..FRESH {
        store
            .enqueue_mail(mail(format!("f{i}@example.com")))
            .await
            .unwrap();
    }

    let stats = mail_worker::sweep_once(&state).await;

    assert_eq!(stats.sent as usize, BURST as usize + FRESH);
    assert_eq!(stats.deferred as usize, BACKLOG - BURST as usize);
    assert_eq!((stats.retried, stats.dead), (0, 0));
    assert_eq!(relay.sent.load(Ordering::SeqCst), BURST as usize + FRESH);
    assert_eq!(store.count_dead_mail().await.unwrap(), 0);
    // Nothing more is due now, and once the deferral has passed every held-back
    // entry comes back on its second attempt: one failure, then this one.
    assert!(store
        .claim_next_due_mail(Utc::now(), 120)
        .await
        .unwrap()
        .is_none());
    let later = Utc::now() + Duration::seconds(61);
    for _ in 0..BACKLOG - BURST as usize {
        let entry = store
            .claim_next_due_mail(later, 120)
            .await
            .unwrap()
            .expect("a deferred entry is still queued");
        assert_eq!(
            entry.attempts, 2,
            "the deferral did not count as an attempt"
        );
    }
}

struct RecoveredSlack {
    posts: AtomicUsize,
}

#[async_trait::async_trait]
impl SlackSender for RecoveredSlack {
    async fn post_message(
        &self,
        channel: &str,
        _text: &str,
        _thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        let n = self.posts.fetch_add(1, Ordering::SeqCst);
        Ok(Some(ExternalRef::Slack {
            channel_id: channel.into(),
            ts: format!("1700000000.{n:06}"),
        }))
    }

    async fn update_message(
        &self,
        _channel: &str,
        _ts: &str,
        _text: &str,
    ) -> Result<(), SlackError> {
        unreachable!("projector egress never updates")
    }
}

async fn egress_suite(backend: &Backend) {
    let mut state = state_on(backend);
    let slack = Arc::new(RecoveredSlack {
        posts: AtomicUsize::new(0),
    });
    state.attach_slack_sender(slack.clone());
    let store = backend.store.as_ref();
    let ws = workspace(store, "egress").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
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
            description: None,
        })
        .await
        .unwrap();
    let queued = |log_id: i64| NewEgressOutbox {
        workspace_id: ws,
        thread_id: thread.id,
        source_log_id: log_id,
        target: EgressTarget::Slack {
            channel_id: "C1".into(),
            thread_ts: None,
        },
        body: "hi".into(),
        kind: EgressKind::Projector,
    };
    let mut backlog = Vec::new();
    for log_id in 0..BACKLOG as i64 {
        backlog.push(store.enqueue_egress(queued(log_id)).await.unwrap().unwrap());
    }
    // Each fails its first post while Slack is down.
    for _ in 0..BACKLOG {
        store
            .claim_next_due_egress(Utc::now(), 120)
            .await
            .unwrap()
            .unwrap();
    }
    for id in backlog {
        store
            .mark_egress_failed(id, "HTTP 502", Some(Utc::now() - Duration::seconds(5)))
            .await
            .unwrap();
    }
    for log_id in 0..FRESH as i64 {
        store.enqueue_egress(queued(100 + log_id)).await.unwrap();
    }

    let stats = egress_worker::sweep_once(&state).await;

    assert_eq!(stats.sent as usize, BURST as usize + FRESH);
    assert_eq!(stats.deferred as usize, BACKLOG - BURST as usize);
    assert_eq!((stats.retried, stats.dead), (0, 0));
    assert_eq!(slack.posts.load(Ordering::SeqCst), BURST as usize + FRESH);
    assert_eq!(store.count_dead_egress().await.unwrap(), 0);
    assert!(store
        .claim_next_due_egress(Utc::now(), 120)
        .await
        .unwrap()
        .is_none());
    let later = Utc::now() + Duration::seconds(61);
    for _ in 0..BACKLOG - BURST as usize {
        let entry = store
            .claim_next_due_egress(later, 120)
            .await
            .unwrap()
            .expect("a deferred delivery is still queued");
        assert_eq!(
            entry.attempts, 2,
            "the deferral did not count as an attempt"
        );
    }
}

#[tokio::test]
async fn a_recovering_webhook_receiver_gets_only_the_budgeted_retries_sqlite() {
    webhook_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_recovering_automation_receiver_gets_only_the_budgeted_retries_sqlite() {
    automation_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_recovering_mail_relay_gets_only_the_budgeted_retries_sqlite() {
    mail_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_recovering_slack_api_gets_only_the_budgeted_retries_sqlite() {
    egress_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_recovering_destination_gets_only_the_budgeted_retries_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let backend = Backend {
        store: Arc::new(PostgresStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::PostgresSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    };
    webhook_suite(&backend).await;
    automation_suite(&backend).await;
    mail_suite(&backend).await;
    egress_suite(&backend).await;
}
