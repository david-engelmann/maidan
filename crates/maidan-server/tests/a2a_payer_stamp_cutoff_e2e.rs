//! A2A PayerStamp under the hard allowance cutoff, on both backends, with
//! two open sign-up tenants. A report over A2A is held to the same rule as
//! `POST /threads/{id}/usage`: past its request allowance a workspace is
//! refused both before anything is recorded; past its stored-bytes allowance
//! both still record the spend the peer already incurred, while the A2A
//! message itself is refused. The other tenant is untouched either way.
//! Postgres skips when Docker is unavailable.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const EXT: &str =
    "https://github.com/david-engelmann/maidan/blob/main/docs/Protocols.md#payerstamp-usage-extension-v1";

const CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::MESSAGE_POST,
    capability::THREAD_TRANSITION,
];

struct Peer {
    ws: WorkspaceId,
    member: MemberId,
    token: String,
}

struct H {
    base: String,
    state: AppState,
    http: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
    _container: Option<testcontainers::ContainerAsync<Postgres>>,
}

async fn serve(
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    container: Option<testcontainers::ContainerAsync<Postgres>>,
) -> H {
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.usage_allowance =
        Arc::new(UsageAllowance::parse("requests=1000000,stored_bytes=1000000").unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    H {
        base: format!("http://{addr}"),
        state,
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        _dir: dir,
        _container: container,
    }
}

async fn sqlite() -> H {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    serve(
        store,
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        None,
    )
    .await
}

async fn postgres() -> Option<H> {
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    Some(
        serve(
            store,
            Arc::new(maidan_search::PostgresSearch::new(pool)),
            Some(container),
        )
        .await,
    )
}

impl H {
    /// An open sign-up workspace with one member holding a token.
    async fn tenant(&self, who: &str) -> Peer {
        let outcome = self
            .store
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
        let (ws, member) = (outcome.workspace.id, outcome.member.id);
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: ws,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: CAPS.iter().map(|c| c.to_string()).collect(),
                expires_at: None,
            })
            .await
            .unwrap();
        Peer {
            ws,
            member,
            token: secret.as_str().to_string(),
        }
    }

    /// A claimed thread in `ws`, and its lease.
    async fn claimed(&self, peer: &Peer) -> (ThreadId, ClaimLeaseId) {
        let channel = self
            .store
            .create_channel(NewChannel {
                workspace_id: peer.ws,
                name: format!("c-{}", uuid::Uuid::now_v7().simple()),
                topic: None,
                private: false,
            })
            .await
            .unwrap()
            .id;
        let thread = self
            .store
            .create_thread(NewThread {
                channel_id: channel,
                parent_thread_id: None,
                title: Some("task".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap()
            .id;
        let lease = self
            .store
            .claim_thread(thread, peer.member)
            .await
            .unwrap()
            .thread
            .claim_lease_id
            .unwrap();
        (thread, lease)
    }

    async fn cut_off(&self, ws: WorkspaceId, meter: UsageMeter) {
        let mark = OverAllowance {
            workspace_id: ws,
            meter,
            month: UsageMonth::of(Utc::now().date_naive()).label(),
            quantity: 2_000_000,
            allowance: 1_000_000,
            marked_at: Utc::now(),
        };
        self.store.mark_over_allowance(&mark, &[]).await.unwrap();
        maidan_server::usage_meters::refresh_cut_off(&self.state).await;
    }

    async fn send(&self, peer: &Peer, context: ThreadId, report: Value) -> (StatusCode, Value) {
        let msg = json!({
            "messageId": uuid::Uuid::now_v7().to_string(),
            "role": "ROLE_USER",
            "contextId": context.0.to_string(),
            "parts": [{ "text": "progress" }],
            "metadata": report,
            "extensions": [EXT],
        });
        let res = self
            .http
            .post(format!("{}/a2a/v1/rpc", self.base))
            .bearer_auth(&peer.token)
            .header("A2A-Version", "1.0")
            .header("A2A-Extensions", EXT)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
                           "params": { "message": msg } }))
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn post_usage(&self, peer: &Peer, thread: ThreadId, body: Value) -> StatusCode {
        self.http
            .post(format!("{}/threads/{}/usage", self.base, thread.0))
            .bearer_auth(&peer.token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn ledger(&self, ws: WorkspaceId) -> usize {
        self.store
            .list_workspace_usage_ledger(ws, 100)
            .await
            .unwrap()
            .len()
    }
}

fn body(lease: ClaimLeaseId) -> Value {
    json!({
        "usage_report_id": uuid::Uuid::now_v7(),
        "claim_lease_id": lease.0,
        "model": "peer/model",
        "tokens": { "input": 10, "output": 0, "cache_read": 0,
                    "cache_write_5m": 0, "cache_write_1h": 0 },
        "usd_micros": 0,
        "price_snapshot": {
            "input_usd_micros_per_million": 0,
            "output_usd_micros_per_million": 0,
            "cache_read_usd_micros_per_million": 0,
            "cache_write_5m_usd_micros_per_million": 0,
            "cache_write_1h_usd_micros_per_million": 0
        },
        "turns": 1
    })
}

fn stamped(lease: ClaimLeaseId) -> Value {
    let mut report = body(lease);
    report["evidence"] = json!({});
    json!({ EXT: report })
}

async fn past_its_request_allowance_a_report_is_refused_over_a2a_and_rest_alike(h: H) {
    let a = h.tenant("ada").await;
    let b = h.tenant("bea").await;
    let (a_thread, a_lease) = h.claimed(&a).await;
    let (b_thread, b_lease) = h.claimed(&b).await;
    h.cut_off(a.ws, UsageMeter::Requests).await;

    let (status, resp) = h.send(&a, a_thread, stamped(a_lease)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{resp}");
    assert!(
        resp.to_string().contains("free allowance for requests"),
        "{resp}"
    );
    assert_eq!(
        h.post_usage(&a, a_thread, body(a_lease)).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(h.ledger(a.ws).await, 0, "a refused report records nothing");
    assert_eq!(
        h.store
            .get_a2a_context_thread(a.ws, &a_thread.0.to_string())
            .await
            .unwrap(),
        None,
        "a refused send binds no context"
    );

    let (status, resp) = h.send(&b, b_thread, stamped(b_lease)).await;
    assert!(status.is_success() && resp.get("error").is_none(), "{resp}");
    assert_eq!(h.ledger(b.ws).await, 1, "the other tenant still records");
}

async fn past_its_stored_bytes_a_report_still_records_over_a2a_and_rest_alike(h: H) {
    let a = h.tenant("ada").await;
    let b = h.tenant("bea").await;
    let (a_thread, a_lease) = h.claimed(&a).await;
    let (b_thread, b_lease) = h.claimed(&b).await;
    h.cut_off(a.ws, UsageMeter::StoredBytes).await;

    assert!(h.post_usage(&a, a_thread, body(a_lease)).await.is_success());
    assert_eq!(h.ledger(a.ws).await, 1);

    // The spend is recorded before the post; the post adds bytes and is
    // refused, naming the meter. A retry with the same report id is the
    // same row, so nothing is counted twice.
    let report = stamped(a_lease);
    let (_, resp) = h.send(&a, a_thread, report.clone()).await;
    assert!(
        resp.to_string().contains("free allowance for stored_bytes"),
        "{resp}"
    );
    assert_eq!(h.ledger(a.ws).await, 2);
    let (_, again) = h.send(&a, a_thread, report).await;
    assert!(
        again
            .to_string()
            .contains("free allowance for stored_bytes"),
        "{again}"
    );
    assert_eq!(h.ledger(a.ws).await, 2, "a retry is the same ledger row");

    let (status, resp) = h.send(&b, b_thread, stamped(b_lease)).await;
    assert!(status.is_success() && resp.get("error").is_none(), "{resp}");
    assert_eq!(h.ledger(b.ws).await, 1, "the other tenant is untouched");
}

#[tokio::test]
async fn past_its_request_allowance_a_report_is_refused_over_a2a_and_rest_alike_on_sqlite() {
    past_its_request_allowance_a_report_is_refused_over_a2a_and_rest_alike(sqlite().await).await;
}

#[tokio::test]
async fn past_its_request_allowance_a_report_is_refused_over_a2a_and_rest_alike_on_postgres() {
    let Some(h) = postgres().await else { return };
    past_its_request_allowance_a_report_is_refused_over_a2a_and_rest_alike(h).await;
}

#[tokio::test]
async fn past_its_stored_bytes_a_report_still_records_over_a2a_and_rest_alike_on_sqlite() {
    past_its_stored_bytes_a_report_still_records_over_a2a_and_rest_alike(sqlite().await).await;
}

#[tokio::test]
async fn past_its_stored_bytes_a_report_still_records_over_a2a_and_rest_alike_on_postgres() {
    let Some(h) = postgres().await else { return };
    past_its_stored_bytes_a_report_still_records_over_a2a_and_rest_alike(h).await;
}
