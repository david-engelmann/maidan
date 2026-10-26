//! The gRPC A2A binding under the hard request-allowance cutoff, on both
//! backends, with two open sign-up tenants. The gRPC server has no HTTP
//! request layer, so it runs the same cutoff and request metering as the
//! bearer middleware itself, for every operation: a cut-off workspace is
//! refused `RESOURCE_EXHAUSTED` before anything is posted, and its refusals
//! are not counted; the other tenant's calls go through and are counted.
//! Postgres skips when Docker is unavailable.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::a2a_grpc::generated::a2a_service_client::A2aServiceClient;
use maidan_server::a2a_grpc::generated::{ListTasksRequest, SendMessageRequest};
use maidan_server::{AppState, FederationRuntime};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

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
    state: AppState,
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
    H {
        state,
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
}

type Client = A2aServiceClient<tonic::transport::Channel>;

async fn grpc(h: &H) -> Client {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let svc = maidan_server::a2a_grpc::service(h.state.clone());
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(svc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap()
    });
    A2aServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

fn signed<T>(peer: &Peer, message: T) -> tonic::Request<T> {
    let mut req = tonic::Request::new(message);
    req.metadata_mut()
        .insert("a2a-version", "1.0".parse().unwrap());
    req.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", peer.token).parse().unwrap(),
    );
    req
}

fn message(context: ThreadId) -> SendMessageRequest {
    serde_json::from_value(json!({ "message": {
        "messageId": uuid::Uuid::now_v7().to_string(),
        "role": "ROLE_USER",
        "contextId": context.0.to_string(),
        "parts": [{ "text": "progress" }],
    }}))
    .unwrap()
}

/// Requests counted for `ws` since the last take.
fn counted(h: &H, ws: WorkspaceId) -> i64 {
    h.state
        .usage_meters
        .take()
        .into_iter()
        .filter(|d| d.workspace_id == ws && d.meter == UsageMeter::Requests)
        .map(|d| d.quantity)
        .sum()
}

async fn past_its_request_allowance_a_workspace_is_refused_every_grpc_call_while_another_is_not(
    h: H,
) {
    let a = h.tenant("ada").await;
    let b = h.tenant("bea").await;
    let (a_thread, _) = h.claimed(&a).await;
    let (b_thread, _) = h.claimed(&b).await;
    h.cut_off(a.ws, UsageMeter::Requests).await;
    let mut client = grpc(&h).await;
    let _ = h.state.usage_meters.take();

    let sent = client.send_message(signed(&a, message(a_thread))).await;
    let status = sent.expect_err("a cut-off workspace's gRPC send is refused");
    assert_eq!(status.code(), tonic::Code::ResourceExhausted, "{status}");
    assert!(
        status.message().contains("free allowance for requests"),
        "{status}"
    );
    let listed = client
        .list_tasks(signed(&a, ListTasksRequest::default()))
        .await;
    assert_eq!(
        listed.expect_err("every operation is refused").code(),
        tonic::Code::ResourceExhausted
    );
    assert!(
        h.store
            .list_messages(a_thread, 50)
            .await
            .unwrap()
            .is_empty(),
        "a refused send posts nothing"
    );
    assert_eq!(counted(&h, a.ws), 0, "a refusal is not counted");

    client
        .send_message(signed(&b, message(b_thread)))
        .await
        .expect("the other tenant still sends");
    client
        .list_tasks(signed(&b, ListTasksRequest::default()))
        .await
        .expect("the other tenant still lists");
    assert_eq!(counted(&h, b.ws), 2, "each gRPC call is one request");
}

#[tokio::test]
async fn past_its_request_allowance_a_workspace_is_refused_every_grpc_call_on_sqlite() {
    past_its_request_allowance_a_workspace_is_refused_every_grpc_call_while_another_is_not(
        sqlite().await,
    )
    .await;
}

#[tokio::test]
async fn past_its_request_allowance_a_workspace_is_refused_every_grpc_call_on_postgres() {
    let Some(h) = postgres().await else { return };
    past_its_request_allowance_a_workspace_is_refused_every_grpc_call_while_another_is_not(h).await;
}
