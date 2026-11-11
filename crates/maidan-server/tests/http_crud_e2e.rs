//! End-to-end coverage for the HTTP CRUD surface.
//!
//! Spins up the axum router against either a Postgres testcontainer or
//! an in-memory SQLite store, exercises every route, and asserts the
//! response shape. Both backends pass identical scenarios so backend
//! drift surfaces immediately.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, test_support, AppState};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewMember};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    store: Arc<dyn Store>,
    _store_dir: tempfile::TempDir,
    _container: Option<testcontainers::ContainerAsync<Postgres>>,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn_postgres() -> Option<Harness> {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    Some(launch(store, search, Some(container)).await)
}

async fn spawn_sqlite() -> Harness {
    let pool = maidan_search::sqlite_pool_options()
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    launch(store, search, None).await
}

async fn launch(
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    container: Option<testcontainers::ContainerAsync<Postgres>>,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        server,
        store,
        _store_dir: dir,
        _container: container,
    }
}

async fn run_suite(h: &Harness) {
    let base = h.base();

    // `POST /workspaces` is a bootstrap route, closed here; the workspace and
    // its admin are seeded through the store.
    let tenant = test_support::tenant(h.store.as_ref(), "acme").await;
    let admin = tenant.client();
    let workspace_id = tenant.workspace_id.0.to_string();

    // get workspace
    let resp = admin
        .get(format!("{base}/workspaces/{workspace_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // get non-existent → 404 with problem+json. Another workspace's id is a
    // 403 for this token, so the missing resource is one the token could see.
    let bogus = uuid::Uuid::new_v4();
    let resp = admin
        .get(format!("{base}/members/{bogus}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/problem+json"
    );
    let problem: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(problem["status"], 404);
    assert!(problem["type"].as_str().unwrap().contains("not-found"));

    // create members: a person through the store (`POST
    // /workspaces/{wid}/members` is a bootstrap route), an agent over HTTP
    let (alice_member, _) = h
        .store
        .create_member_with_event(NewMember {
            workspace_id: tenant.workspace_id,
            handle: "alice".into(),
            display_name: Some("Alice".into()),
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let resp = admin
        .post(format!("{base}/workspaces/{workspace_id}/agents"))
        .json(&json!({"handle": "bot"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bot: serde_json::Value = resp.json().await.unwrap();
    let alice_id = alice_member.id.0.to_string();
    let bot_id = bot["member"]["id"].as_str().unwrap().to_string();
    let alice = test_support::bearer_client(
        &test_support::admin_token(h.store.as_ref(), alice_member.id).await,
    );
    let bot = test_support::bearer_client(
        &test_support::admin_token(
            h.store.as_ref(),
            MemberId(uuid::Uuid::parse_str(&bot_id).unwrap()),
        )
        .await,
    );

    // duplicate handle → 409 with problem+json
    let resp = admin
        .post(format!("{base}/workspaces/{workspace_id}/agents"))
        .json(&json!({"handle": "alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let problem: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(problem["status"], 409);

    // list members
    let members: Vec<serde_json::Value> = admin
        .get(format!("{base}/workspaces/{workspace_id}/members"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // The admin the workspace was seeded with, alice and bot.
    assert_eq!(members.len(), 3);

    // get single member
    let _: serde_json::Value = admin
        .get(format!("{base}/members/{alice_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // create channel
    let channel: serde_json::Value = admin
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general", "topic": "everything"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = channel["id"].as_str().unwrap().to_string();

    // duplicate channel name → 409
    let resp = admin
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // list channels
    let channels: Vec<serde_json::Value> = admin
        .get(format!("{base}/workspaces/{workspace_id}/channels"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(channels.len(), 1);

    // create thread
    let thread: serde_json::Value = admin
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "kickoff"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = thread["id"].as_str().unwrap().to_string();

    // list threads
    let threads: Vec<serde_json::Value> = admin
        .get(format!("{base}/channels/{channel_id}/threads"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(threads.len(), 1);

    // post messages
    let msg1: serde_json::Value = alice
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"body": "hello", "metadata": {"client": "test"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msg1_id = msg1["id"].as_str().unwrap().to_string();
    let msg2: serde_json::Value = bot
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"body": "world"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msg2_id = msg2["id"].as_str().unwrap().to_string();

    // list messages with limit
    let listed: Vec<serde_json::Value> = admin
        .get(format!("{base}/threads/{thread_id}/messages?limit=10"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["body"], "hello");
    assert_eq!(listed[0]["metadata"]["client"], "test");

    // edit message (author)
    let edited: serde_json::Value = alice
        .patch(format!("{base}/messages/{msg1_id}"))
        .json(&json!({"body": "hello edited"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edited["body"], "hello edited");
    assert!(edited["edited_at"].as_str().is_some());
    assert_eq!(edited["metadata"]["client"], "test");

    let history: Vec<serde_json::Value> = admin
        .get(format!("{base}/messages/{msg1_id}/edits"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["body_before"], "hello");
    assert_eq!(history[0]["body_after"], "hello edited");

    // mention
    let resp = alice
        .post(format!("{base}/messages/{msg1_id}/mentions"))
        .json(&json!({"member_id": bot_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // A member's mentions are their own: only the bot reads the bot's.
    let mentions: Vec<serde_json::Value> = bot
        .get(format!("{base}/members/{bot_id}/mentions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(mentions.len(), 1);

    // vote
    let resp = bot
        .post(format!("{base}/messages/{msg1_id}/votes"))
        .json(&json!({"kind": "approve"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let votes: Vec<serde_json::Value> = admin
        .get(format!("{base}/messages/{msg1_id}/votes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(votes.len(), 1);
    assert_eq!(votes[0]["kind"], "approve");

    // reference
    let reference: serde_json::Value = admin
        .post(format!("{base}/references"))
        .json(&json!({
            "src_kind": "message",
            "src_id": msg2_id,
            "dst_kind": "message",
            "dst_id": msg1_id,
            "relation": "replies-to"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(reference["id"].as_str().is_some());

    let refs: Vec<serde_json::Value> = admin
        .get(format!(
            "{base}/references?src_kind=message&src_id={msg2_id}"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(refs.len(), 1);

    // tombstone msg2
    let resp = bot
        .delete(format!("{base}/messages/{msg2_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let remaining: Vec<serde_json::Value> = admin
        .get(format!("{base}/threads/{thread_id}/messages"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["id"], msg1_id);

    // malformed JSON → 400 problem+json
    let resp = admin
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/problem+json"
    );
}

#[tokio::test]
async fn postgres_backend() {
    let Some(h) = spawn_postgres().await else {
        return;
    };
    run_suite(&h).await;
    h.shutdown().await;
}

#[tokio::test]
async fn sqlite_backend() {
    let h = spawn_sqlite().await;
    run_suite(&h).await;
    h.shutdown().await;
}
