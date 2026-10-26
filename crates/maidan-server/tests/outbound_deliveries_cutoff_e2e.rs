//! Slash-command sends, automation retries and A2A push under the hard
//! deliveries cutoff, on both backends, with two open sign-up tenants. A
//! workspace past its deliveries allowance sends nothing on any of these
//! paths: a slash command is refused with `over_allowance` and nothing is
//! queued, a queued retry is deferred with its attempts unchanged, and a push
//! is not sent. The other tenant's sends go out, and each attempt, failed or
//! not, counts one delivery to that tenant only. Postgres skips when Docker
//! is unavailable.
// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, http::StatusCode, routing::post, Router};
use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{automation_worker, router, AppState, FederationRuntime, SlashRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::*;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

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
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Backend {
        store: Arc::new(SqliteStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::SqliteSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    }
}

struct Api {
    base: String,
    http: reqwest::Client,
    state: AppState,
}

async fn serve(backend: &Backend) -> Api {
    let mut state = AppState::new(
        backend.store.clone(),
        Arc::new(LocalFsStore::new(backend._dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        backend.search.clone(),
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, Some(Arc::new([0x5e; 32]))),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.slash = SlashRuntime::new(Some(Arc::new([0x5e; 32])));
    state.usage_allowance = Arc::new(UsageAllowance::parse("deliveries=10").unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Api {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        state,
    }
}

struct Tenant {
    ws: WorkspaceId,
    token: String,
    thread: ThreadId,
}

async fn signed_up(store: &dyn Store, who: &str) -> Tenant {
    let outcome = store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open: true,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
                metadata: json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
    let ws = outcome.workspace.id;
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: outcome.member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: [
                capability::WORKSPACE_READ,
                capability::WORKSPACE_WRITE,
                capability::MESSAGE_POST,
            ]
            .iter()
            .map(|c| c.to_string())
            .collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("{who}-general"),
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
            created_by: None,
        })
        .await
        .unwrap();
    Tenant {
        ws,
        token: secret.as_str().to_string(),
        thread: thread.id,
    }
}

/// A receiver that answers `status` to every POST and counts them.
async fn receiver(status: StatusCode) -> (SocketAddr, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/r",
            post(move |State(hits): State<Arc<AtomicUsize>>| async move {
                hits.fetch_add(1, Ordering::SeqCst);
                (status, axum::Json(json!({ "text": "ok" })))
            }),
        )
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, hits)
}

fn hits(n: &AtomicUsize) -> usize {
    n.load(Ordering::SeqCst)
}

impl Api {
    async fn slash_command(&self, t: &Tenant, name: &str, addr: SocketAddr) -> SlashCommandId {
        let resp = self
            .http
            .post(format!(
                "{}/workspaces/{}/slash-commands",
                self.base, t.ws.0
            ))
            .bearer_auth(&t.token)
            .json(&json!({
                "name": name,
                "handler_kind": "http",
                "handler_target": format!("http://{addr}/r"),
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: Value = resp.json().await.unwrap();
        serde_json::from_value(body["command"]["id"].clone()).unwrap()
    }

    /// Post `body` in the tenant's thread and return the slash response.
    async fn post(&self, t: &Tenant, body: &str) -> Value {
        let resp = self
            .http
            .post(format!("{}/threads/{}/messages", self.base, t.thread.0))
            .bearer_auth(&t.token)
            .json(&json!({ "body": body }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "post: {}", resp.status());
        let msg: Value = resp.json().await.unwrap();
        msg["metadata"]["slash_response"].clone()
    }

    async fn send_with_push(&self, t: &Tenant, addr: SocketAddr) {
        let resp: Value = self
            .http
            .post(format!("{}/a2a/v1/rpc", self.base))
            .bearer_auth(&t.token)
            .header("A2A-Version", "1.0")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "SendMessage",
                "params": {
                    "message": {
                        "messageId": uuid::Uuid::now_v7().to_string(),
                        "contextId": format!("ctx-{}", t.ws.0),
                        "role": "ROLE_USER",
                        "parts": [{ "text": "ship it" }],
                    },
                    "configuration": {
                        "taskPushNotificationConfig": { "url": format!("http://{addr}/r") }
                    },
                },
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(resp.get("error").is_none(), "SendMessage failed: {resp}");
    }

    /// Deliveries counted for each workspace since the last take.
    fn counted(&self) -> impl Fn(WorkspaceId) -> i64 {
        let taken = self.state.usage_meters.take();
        move |ws| {
            taken
                .iter()
                .filter(|d| d.workspace_id == ws && d.meter == UsageMeter::Deliveries)
                .map(|d| d.quantity)
                .sum()
        }
    }
}

async fn cut_off(api: &Api, ws: WorkspaceId) {
    let store = api.state.store.as_ref();
    store
        .mark_over_allowance(
            &OverAllowance {
                workspace_id: ws,
                meter: UsageMeter::Deliveries,
                month: UsageMonth::of(Utc::now().date_naive()).label(),
                quantity: 11,
                allowance: 10,
                marked_at: Utc::now(),
            },
            &[],
        )
        .await
        .unwrap();
    maidan_server::usage_meters::refresh_cut_off(&api.state).await;
    assert!(api
        .state
        .usage_meters
        .is_cut_off(ws, UsageMeter::Deliveries));
}

async fn wait_for(n: &AtomicUsize, at_least: usize) {
    for _ in 0..100 {
        if hits(n) >= at_least {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("receiver saw {} of {at_least}", hits(n));
}

async fn suite(backend: &Backend) {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let api = serve(backend).await;
    let store = backend.store.as_ref();
    let a = signed_up(store, "ada").await;
    let b = signed_up(store, "bea").await;
    let (a_addr, a_hits) = receiver(StatusCode::OK).await;
    let (b_addr, b_hits) = receiver(StatusCode::OK).await;
    let (down_addr, down_hits) = receiver(StatusCode::SERVICE_UNAVAILABLE).await;
    let a_ping = api.slash_command(&a, "ping", a_addr).await;
    api.slash_command(&b, "ping", b_addr).await;
    api.slash_command(&b, "flaky", down_addr).await;

    cut_off(&api, a.ws).await;
    assert!(!api
        .state
        .usage_meters
        .is_cut_off(b.ws, UsageMeter::Deliveries));
    let _ = api.state.usage_meters.take();

    // The synchronous slash-command send.
    let refused = api.post(&a, "/ping hello").await;
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], "over_allowance");
    assert!(refused.get("delivery_id").is_none(), "nothing is queued");
    assert_eq!(hits(&a_hits), 0, "a cut-off workspace sends nothing");
    assert!(store
        .list_automation_deliveries(a.ws, AutomationDeliveryFilter::Pending, 100)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(api.post(&b, "/ping hello").await["ok"], true);
    let flaky = api.post(&b, "/flaky hello").await;
    assert_eq!(flaky["retrying"], true, "{flaky}");
    assert_eq!((hits(&b_hits), hits(&down_hits)), (1, 1));
    let of = api.counted();
    assert_eq!(of(a.ws), 0, "a refused command is not counted");
    assert_eq!(of(b.ws), 2, "each send counts, the failed one too");

    // Automation retries: a queued retry for A, already tried once.
    let a_retry = store
        .enqueue_automation_delivery(NewAutomationDelivery {
            workspace_id: a.ws,
            source_kind: AutomationSourceKind::SlashCommand,
            source_id: a_ping.0,
            target_url: format!("http://{a_addr}/r"),
            header_name: "X-Maidan-Command".into(),
            header_value: "ping".into(),
            payload: "{}".into(),
        })
        .await
        .unwrap();
    store
        .record_automation_delivery_attempt(
            a_retry,
            "HTTP 503",
            Utc::now() - chrono::Duration::seconds(5),
        )
        .await
        .unwrap();
    // B's failed command was queued; make its retry due now.
    let b_queued = store
        .list_automation_deliveries(b.ws, AutomationDeliveryFilter::Pending, 100)
        .await
        .unwrap();
    assert_eq!(b_queued.len(), 1);
    store
        .defer_automation_delivery(b_queued[0].id, Utc::now() - chrono::Duration::seconds(5))
        .await
        .unwrap();

    automation_worker::poll_once(&api.state, 16).await.unwrap();

    assert_eq!(hits(&a_hits), 0, "a cut-off workspace retries nothing");
    let parked = store.get_automation_delivery(a_retry, a.ws).await.unwrap();
    assert_eq!(parked.attempts, 1, "parking is not an attempt");
    assert!(parked.delivered_at.is_none() && parked.quarantined_at.is_none());
    assert!(
        parked.next_attempt_at > Utc::now() + chrono::Duration::minutes(10),
        "parked past the next polls"
    );
    assert_eq!(hits(&down_hits), 2, "the other tenant's retry is tried");
    let of = api.counted();
    assert_eq!(of(a.ws), 0, "a parked retry is not counted");
    assert_eq!(of(b.ws), 1, "the failed retry counts");

    // A2A push.
    api.send_with_push(&b, b_addr).await;
    api.send_with_push(&a, a_addr).await;
    wait_for(&b_hits, 2).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(hits(&a_hits), 0, "a cut-off workspace is not pushed to");
    let of = api.counted();
    assert_eq!(of(a.ws), 0, "an unsent push is not counted");
    assert_eq!(
        of(b.ws),
        (hits(&b_hits) - 1) as i64,
        "each push attempt counts"
    );
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_sends_no_slash_automation_or_push_http_while_another_does_on_sqlite(
) {
    suite(&sqlite().await).await;
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_sends_no_slash_automation_or_push_http_while_another_does_on_postgres(
) {
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
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    suite(&Backend {
        store: Arc::new(PostgresStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::PostgresSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    })
    .await;
}
