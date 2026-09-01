//! Member inbox: @mention routing, unread cursor, DM coexistence.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (addr, client, server)
}

#[tokio::test]
async fn at_mention_in_channel_populates_inbox_and_mark_read_clears_unread() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "inbox-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let alice: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bob: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "bob", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap();

    let ch: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general", "private": false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let th: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "t1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let post: Value = client
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({
            "author_id": alice_id,
            "body": "hey @bob check this"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(post["body"], "hey @bob check this");

    let inbox: Value = client
        .get(format!("{base}/members/{bob_id}/inbox"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inbox["unread_count"].as_i64(), Some(1));
    let items = inbox["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["author_handle"], "alice");
    assert!(items[0]["unread"].as_bool().unwrap());

    let created_at = items[0]["created_at"].as_str().unwrap();
    let after_read: Value = client
        .post(format!("{base}/members/{bob_id}/inbox/read"))
        .json(&json!({"read_through": created_at}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_read["unread_count"].as_i64(), Some(0));
    assert!(!after_read["items"][0]["unread"].as_bool().unwrap());

    server.abort();
}

#[tokio::test]
async fn dm_message_with_at_mention_still_routes_to_inbox() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "dm-inbox"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let a: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "a", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let b: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "b", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();

    let dm: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/dm"))
        .json(&json!({
            "member_id": a_id,
            "other_member_id": b_id
        }))
        .send()
        .await
        .unwrap()
        .assert_status(StatusCode::OK)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dm_id = dm["id"].as_str().unwrap();

    client
        .post(format!("{base}/dm/{dm_id}/messages"))
        .json(&json!({
            "author_id": a_id,
            "body": "dm ping @b"
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let inbox: Value = client
        .get(format!("{base}/members/{b_id}/inbox"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inbox["unread_count"].as_i64(), Some(1));
    assert_eq!(inbox["items"][0]["message_body"], "dm ping @b");

    server.abort();
}

trait AssertStatus {
    async fn assert_status(self, status: StatusCode) -> Result<reqwest::Response, reqwest::Error>;
}

impl AssertStatus for reqwest::Response {
    async fn assert_status(
        self,
        expected: StatusCode,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let code = self.status();
        assert_eq!(
            code,
            expected,
            "body={}",
            self.text().await.unwrap_or_default()
        );
        Ok(self)
    }
}
