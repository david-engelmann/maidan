//! Member inbox: @mention routing, unread cursor, DM coexistence.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{
    router, subscribe_resume, test_support, test_support::Tenant, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MemberKind;
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn spawn(
    name: &str,
) -> (
    SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
    Tenant,
    tokio::task::JoinHandle<()>,
) {
    let pool = maidan_search::sqlite_pool_options()
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
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let tenant = test_support::tenant(store.as_ref(), name).await;
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, tenant.client(), store, tenant, server)
}

#[tokio::test]
async fn at_mention_in_channel_populates_inbox_and_mark_read_clears_unread() {
    let (addr, client, store, tenant, server) = spawn("inbox-ws").await;
    let base = format!("http://{addr}");
    let workspace_id = tenant.workspace_id.0;

    let alice = tenant
        .member(store.as_ref(), "alice", MemberKind::Human)
        .await;
    let bob = tenant
        .member(store.as_ref(), "bob", MemberKind::Agent)
        .await;
    let bob_id = bob.member_id.0;

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

    let post: Value = alice
        .client()
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({
            "body": "hey @bob check this"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(post["body"], "hey @bob check this");

    let inbox: Value = bob
        .client()
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
    let after_read: Value = bob
        .client()
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
    let (addr, _client, store, tenant, server) = spawn("dm-inbox").await;
    let base = format!("http://{addr}");
    let workspace_id = tenant.workspace_id.0;

    let a = tenant.member(store.as_ref(), "a", MemberKind::Human).await;
    let b = tenant.member(store.as_ref(), "b", MemberKind::Human).await;
    let b_id = b.member_id.0;

    let dm: Value = a
        .client()
        .post(format!("{base}/workspaces/{workspace_id}/dm"))
        .json(&json!({ "other_member_id": b_id }))
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

    a.client()
        .post(format!("{base}/dm/{dm_id}/messages"))
        .json(&json!({
            "body": "dm ping @b"
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let inbox: Value = b
        .client()
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
