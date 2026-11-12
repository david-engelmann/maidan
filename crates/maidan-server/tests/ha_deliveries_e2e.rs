//! CONC-003: two replicas against one Postgres send one POST per event.
//!
//! Each replica runs what `main.rs` runs in every pod: the webhook worker,
//! the FSM hook worker and the automation delivery worker, each on its own
//! `PostgresBus`, whose LISTEN hands every event to both. Before the event
//! keys and the claims, each replica enqueued its own webhook and FSM hook
//! delivery, both pollers sent every due row, and both ran each MCP-tool hook.
//!
//! The lagging case puts the second replica on its lag-resume path: its bus
//! turns every event into `Lagged`, so it finds the event by reading the log,
//! as a replica that fell behind does, while the first takes it live.

// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::{extract::State, routing::post, Router};
use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{encrypt_peer_secret, expand_set, hash_secret, TokenSecret, HUMAN_ADMIN};
use maidan_bus::{BusError, BusItem, EventBus, EventStream, PostgresBus};
use maidan_server::{
    automation_worker::AutomationDeliveryWorker, fsm_hook_worker::FsmHookWorker,
    outbox_relay::OutboxRelay, webhook_worker::WebhookWorker, AppState, FsmHookRuntime,
    WebhookRuntime,
};
use maidan_store::{prelude::*, OutboxBackend};
use maidan_types::{
    ApiTokenId, BusEnvelope, Event, EventFilter, MemberKind, NewApiToken, NewChannel, NewFsmHook,
    NewMember, NewThread, NewWebhookSubscription, NewWorkspace, SlashHandlerKind, ThreadState,
    UsageMeter, WorkspaceId,
};
use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const KEY: [u8; 32] = [0x15; 32];

/// A bus that has fallen behind on every event: each one arrives as `Lagged`,
/// so the subscriber resumes from the log.
struct LaggingBus(Arc<dyn EventBus>);

#[async_trait]
impl EventBus for LaggingBus {
    async fn publish(&self, envelope: BusEnvelope) -> Result<(), BusError> {
        self.0.publish(envelope).await
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError> {
        let stream = self.0.subscribe(filter).await?;
        Ok(Box::pin(stream.map(|item| match item {
            BusItem::Event(_) => BusItem::Lagged { skipped: 1 },
            lagged => lagged,
        })))
    }
}

#[derive(Clone, Default)]
struct Received {
    webhook: Arc<AtomicUsize>,
    fsm: Arc<AtomicUsize>,
}

async fn count_webhook(State(r): State<Received>) -> &'static str {
    r.webhook.fetch_add(1, Ordering::SeqCst);
    "ok"
}

async fn count_fsm(State(r): State<Received>) -> &'static str {
    r.fsm.fetch_add(1, Ordering::SeqCst);
    "ok"
}

struct Replica {
    store: Arc<dyn Store>,
    workers: Option<(WebhookWorker, FsmHookWorker, AutomationDeliveryWorker)>,
    _dir: tempfile::TempDir,
}

impl Replica {
    async fn start(pool: &PgPool, lagging: bool) -> (Self, Arc<PostgresBus>) {
        let bus = Arc::new(
            PostgresBus::connect(pool.clone(), maidan_store::test_support::dev_keys())
                .await
                .expect("bus"),
        );
        let seen: Arc<dyn EventBus> = if lagging {
            Arc::new(LaggingBus(bus.clone()))
        } else {
            bus.clone()
        };
        // Its own store, as a pod has: its own usage-meter buffer included.
        let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = AppState::for_tests(
            store.clone(),
            Arc::new(LocalFsStore::new(dir.path())),
            seen,
            Arc::new(maidan_search::PostgresSearch::new(pool.clone())),
        );
        state.webhooks = WebhookRuntime::new(Some(Arc::new(KEY)));
        state.fsm_hooks = FsmHookRuntime::new(Some(Arc::new(KEY)));
        let workers = (
            WebhookWorker::spawn(state.clone()),
            FsmHookWorker::spawn(state.clone()),
            AutomationDeliveryWorker::spawn(state),
        );
        (
            Self {
                store,
                workers: Some(workers),
                _dir: dir,
            },
            bus,
        )
    }

    fn mcp_calls(&self, ws: WorkspaceId) -> i64 {
        self.store
            .usage_meters()
            .take()
            .into_iter()
            .filter(|row| row.workspace_id == ws && row.meter == UsageMeter::McpCalls)
            .map(|row| row.quantity)
            .sum()
    }

    async fn stop(mut self) {
        if let Some((webhook, fsm, automation)) = self.workers.take() {
            webhook.shutdown().await;
            fsm.shutdown().await;
            automation.shutdown().await;
        }
    }
}

async fn postgres() -> Option<(testcontainers::ContainerAsync<Postgres>, PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    // Two listeners, six workers and the test's own writes.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    maidan_store::run_postgres_migrations(&pool)
        .await
        .expect("migrate");
    Some((container, pool))
}

/// A workspace with a webhook on thread state changes, an HTTP FSM hook and an
/// MCP-tool FSM hook, both registered by an administrator's token.
async fn seed(store: &dyn Store, receiver: &str) -> (WorkspaceId, Event) {
    let ws = store
        .create_workspace(NewWorkspace { name: "ha".into() })
        .await
        .expect("workspace")
        .id;
    let admin = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
        .id;
    let token: ApiTokenId = store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: admin,
            app_installation_id: None,
            token_hash: hash_secret(TokenSecret::generate().as_str()),
            label: None,
            capabilities: expand_set(HUMAN_ADMIN).expect("caps"),
            expires_at: None,
        })
        .await
        .expect("token")
        .id;
    let sealed = encrypt_peer_secret("whsec", &KEY).expect("seal");
    store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws,
            url: format!("{receiver}/webhook"),
            label: None,
            event_kinds: vec!["thread_state_changed".into()],
            secret_ciphertext: sealed.clone(),
            created_by_token: Some(token),
        })
        .await
        .expect("webhook");
    for (handler_kind, handler_target, secret_ciphertext) in [
        (
            SlashHandlerKind::Http,
            format!("{receiver}/fsm"),
            sealed.clone(),
        ),
        (
            SlashHandlerKind::McpTool,
            "get_review_requirement".to_string(),
            String::new(),
        ),
    ] {
        store
            .create_fsm_hook(NewFsmHook {
                workspace_id: ws,
                label: None,
                from_state: None,
                to_state: None,
                handler_kind,
                handler_target,
                secret_ciphertext,
                created_by_token: Some(token),
            })
            .await
            .expect("fsm hook");
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    let event = Event::ThreadStateChanged {
        occurred_at: chrono::Utc::now(),
        workspace_id: ws,
        channel_id: channel.id,
        thread_id: thread.id,
        actor_id: admin,
        from_state: ThreadState::Open,
        to_state: ThreadState::InReview,
        thread,
        previous_assignee_id: None,
    };
    (ws, event)
}

/// Wait for `count` to reach one, then long enough for any second POST (both
/// pollers tick every 50 ms) before reading it.
async fn settled(count: &AtomicUsize) -> usize {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while count.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    count.load(Ordering::SeqCst)
}

async fn run(lagging: bool) {
    let Some((_container, pool)) = postgres().await else {
        return;
    };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    #[allow(unsafe_code)] // edition 2024 makes env writes unsafe; test setup only
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let received = Received::default();
    let app = Router::new()
        .route("/webhook", post(count_webhook))
        .route("/fsm", post(count_fsm))
        .with_state(received.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let receiver = format!("http://{}", listener.local_addr().expect("addr"));
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let (a, bus) = Replica::start(&pool, false).await;
    let (b, _) = Replica::start(&pool, lagging).await;
    let (ws, event) = seed(a.store.as_ref(), &receiver).await;
    // Both replicas' consumers attach and read the log head first.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let stored = a.store.append_event(&event).await.expect("append");
    OutboxRelay::new(
        OutboxBackend::postgres(pool.clone(), maidan_store::test_support::dev_keys()),
        bus,
    )
    .run_once()
    .await
    .expect("relay");

    assert_eq!(settled(&received.webhook).await, 1, "webhook POSTs");
    assert_eq!(settled(&received.fsm).await, 1, "FSM hook POSTs");
    assert_eq!(a.mcp_calls(ws) + b.mcp_calls(ws), 1, "MCP-tool hook runs");
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM maidan_webhook_deliveries WHERE log_id = $1")
            .bind(stored.id)
            .fetch_one(&pool)
            .await
            .expect("webhook rows");
    assert_eq!(rows, 1, "webhook delivery rows");
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM maidan_automation_deliveries WHERE log_id = $1")
            .bind(stored.id)
            .fetch_one(&pool)
            .await
            .expect("hook rows");
    assert_eq!(rows, 1, "FSM hook delivery rows");

    a.stop().await;
    b.stop().await;
    mock.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_replicas_send_one_post_per_event() {
    run(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replica_resuming_from_the_log_sends_no_second_post() {
    run(true).await;
}
