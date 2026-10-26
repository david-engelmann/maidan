//! C13 end to end on both backends: an A2A peer's usage report, carried in
//! message metadata under the PayerStamp extension, lands in the same
//! claim-fenced ledger as `POST /threads/{id}/usage`, and every delivery task
//! names the context's cache-affinity key. Postgres skips when Docker is
//! unavailable.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{
    ClaimLeaseId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace, ThreadId, WorkspaceId,
};
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
    let state = AppState::new(
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    H {
        base: format!("http://{addr}"),
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
    async fn peer(&self, name: &str, caps: &[&str]) -> Peer {
        let ws = self
            .store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id;
        let member = self
            .store
            .create_member(NewMember {
                workspace_id: ws,
                handle: "peer".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap()
            .id;
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: ws,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: caps.iter().map(|c| c.to_string()).collect(),
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

    async fn thread(&self, ws: WorkspaceId, parent: Option<ThreadId>) -> ThreadId {
        let channel = match parent {
            Some(parent) => self.store.get_thread(parent).await.unwrap().channel_id,
            None => {
                self.store
                    .create_channel(NewChannel {
                        workspace_id: ws,
                        name: format!("c-{}", uuid::Uuid::now_v7().simple()),
                        topic: None,
                        private: false,
                    })
                    .await
                    .unwrap()
                    .id
            }
        };
        self.store
            .create_thread(NewThread {
                channel_id: channel,
                parent_thread_id: parent,
                title: Some(format!("task {}", uuid::Uuid::now_v7())),
                description: None,
                created_by: None,
            })
            .await
            .unwrap()
            .id
    }

    async fn claim(&self, thread: ThreadId, member: MemberId) -> ClaimLeaseId {
        self.store
            .claim_thread(thread, member)
            .await
            .unwrap()
            .thread
            .claim_lease_id
            .unwrap()
    }

    async fn send(&self, peer: &Peer, context: ThreadId, metadata: Option<Value>) -> Value {
        let mut msg = json!({
            "messageId": uuid::Uuid::now_v7().to_string(),
            "role": "ROLE_USER",
            "contextId": context.0.to_string(),
            "parts": [{ "text": "progress" }],
        });
        if let Some(metadata) = metadata {
            msg["metadata"] = metadata;
            msg["extensions"] = json!([EXT]);
        }
        self.http
            .post(format!("{}/a2a/v1/rpc", self.base))
            .bearer_auth(&peer.token)
            .header("A2A-Version", "1.0")
            .header("A2A-Extensions", EXT)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
                           "params": { "message": msg } }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn ledger(&self, ws: WorkspaceId) -> usize {
        self.store
            .list_workspace_usage_ledger(ws, 100)
            .await
            .unwrap()
            .len()
    }
}

fn report(id: uuid::Uuid, lease: ClaimLeaseId, input: i64) -> Value {
    json!({ EXT: {
        "usage_report_id": id,
        "claim_lease_id": lease.0,
        "model": "peer/model",
        "tokens": { "input": input, "output": 0, "cache_read": 0,
                    "cache_write_5m": 0, "cache_write_1h": 0 },
        "usd_micros": 0,
        "price_snapshot": {
            "input_usd_micros_per_million": 0,
            "output_usd_micros_per_million": 0,
            "cache_read_usd_micros_per_million": 0,
            "cache_write_5m_usd_micros_per_million": 0,
            "cache_write_1h_usd_micros_per_million": 0
        },
        "turns": 1,
        "evidence": {}
    }})
}

fn code(resp: &Value) -> i64 {
    resp["error"]["code"].as_i64().unwrap_or(0)
}

async fn a_peers_spend_lands_in_the_ledger_and_never_in_another_workspace(h: H) {
    let a = h.peer("a", CAPS).await;
    let b = h.peer("b", CAPS).await;
    let parent = h.thread(a.ws, None).await;
    let child = h.thread(a.ws, Some(parent)).await;
    let lease = h.claim(child, a.member).await;
    let id = uuid::Uuid::now_v7();

    let resp = h.send(&a, child, Some(report(id, lease, 120))).await;
    assert!(resp.get("error").is_none(), "{resp}");
    let task = &resp["result"]["task"];
    assert_eq!(task["contextId"], json!(child.0.to_string()));
    assert_eq!(
        task["metadata"]["maidan"]["affinityKey"],
        json!(parent.0.to_string()),
        "a child's affinity key is its parent's thread"
    );
    let stamp = &task["metadata"][EXT];
    assert_eq!(stamp["usage_report_id"], json!(id));
    assert_eq!(stamp["thread_id"], json!(child.0));
    assert_eq!(stamp["stamp"]["payer"], json!(a.ws.0));
    assert_eq!(stamp["stamp"]["reporter"], json!(a.member.0));
    assert_eq!(stamp["stamp"]["tokens"]["input"], 120);
    assert_eq!(h.ledger(a.ws).await, 1);

    // A retry with the same report id is the same ledger row.
    let again = h.send(&a, child, Some(report(id, lease, 120))).await;
    assert!(again.get("error").is_none(), "{again}");
    assert_eq!(
        again["result"]["task"]["metadata"][EXT]["usage_report_id"],
        json!(id)
    );
    assert_eq!(h.ledger(a.ws).await, 1);

    // Another workspace naming A's thread and lease binds its own context
    // and is refused by the claim fence: nothing lands in either ledger.
    let theirs = h
        .send(&b, child, Some(report(uuid::Uuid::now_v7(), lease, 9)))
        .await;
    assert_eq!(code(&theirs), -32602, "{theirs}");
    assert_eq!(h.ledger(a.ws).await, 1);
    assert_eq!(h.ledger(b.ws).await, 0);

    // A lease the caller doesn't hold is refused the same way.
    let stale = ClaimLeaseId(uuid::Uuid::now_v7());
    let refused = h
        .send(&a, child, Some(report(uuid::Uuid::now_v7(), stale, 9)))
        .await;
    assert_eq!(code(&refused), -32602, "{refused}");
    assert_eq!(h.ledger(a.ws).await, 1);

    // A plain message records nothing; a top-level thread is its own key.
    let plain = h.send(&a, parent, None).await;
    assert!(plain.get("error").is_none(), "{plain}");
    let task = &plain["result"]["task"];
    assert!(task["metadata"].get(EXT).is_none(), "{plain}");
    assert_eq!(
        task["metadata"]["maidan"]["affinityKey"],
        json!(parent.0.to_string())
    );

    // Reporting needs thread:transition, as on REST.
    let reader = h
        .peer(
            "c",
            &[
                capability::WORKSPACE_READ,
                capability::WORKSPACE_WRITE,
                capability::MESSAGE_POST,
            ],
        )
        .await;
    let theirs = h.thread(reader.ws, None).await;
    let lease = h.claim(theirs, reader.member).await;
    let denied = h
        .send(
            &reader,
            theirs,
            Some(report(uuid::Uuid::now_v7(), lease, 1)),
        )
        .await;
    assert_eq!(code(&denied), -32000, "{denied}");
    assert_eq!(h.ledger(reader.ws).await, 0);

    let card: Value = h
        .http
        .get(format!("{}/.well-known/agent-card.json", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        card["capabilities"]["extensions"],
        json!([{ "uri": EXT, "description": card["capabilities"]["extensions"][0]["description"], "required": false }])
    );
}

#[tokio::test]
async fn a_peers_spend_lands_in_the_ledger_and_never_in_another_workspace_on_sqlite() {
    a_peers_spend_lands_in_the_ledger_and_never_in_another_workspace(sqlite().await).await;
}

#[tokio::test]
async fn a_peers_spend_lands_in_the_ledger_and_never_in_another_workspace_on_postgres() {
    if let Some(h) = postgres().await {
        a_peers_spend_lands_in_the_ledger_and_never_in_another_workspace(h).await;
    }
}
