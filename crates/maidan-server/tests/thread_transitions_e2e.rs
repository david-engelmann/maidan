//! A thread's transition history over REST.
//!
//! The transition rows were recorded from the start but had no read route,
//! so no surface could show who moved a thread and when. The route lists
//! them oldest first for a member of the workspace, honours `limit`, and
//! answers 404 to another tenant, for whom the thread does not exist.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::THREAD_TRANSITION,
];

struct World {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    channel: maidan_types::ChannelId,
    alice: MemberId,
    alice_tok: String,
    bob_tok: String,
    _dir: tempfile::TempDir,
}

async fn mint(store: &Arc<dyn Store>, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
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
    secret.as_str().to_string()
}

async fn world() -> World {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();

    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws_a.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
    let bob = store
        .create_member(NewMember {
            workspace_id: ws_b.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws_a.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let alice_tok = mint(&store, ws_a.id, alice).await;
    let bob_tok = mint(&store, ws_b.id, bob).await;

    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });

    World {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        store,
        channel: channel.id,
        alice,
        alice_tok,
        bob_tok,
        _dir: dir,
    }
}

impl World {
    async fn get(&self, token: &str, path: &str) -> (StatusCode, Value) {
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn post(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    /// A thread alice has claimed, handed to review, and closed: two
    /// recorded transitions.
    async fn worked_thread(&self) -> ThreadId {
        let thread = self
            .store
            .create_thread(NewThread {
                channel_id: self.channel,
                parent_thread_id: None,
                title: Some("work".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
        let (status, body) = self
            .post(
                &self.alice_tok,
                &format!("/threads/{}/assignee/claim", thread.id.0),
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let lease = self
            .store
            .claim_lease_for(thread.id, self.alice)
            .await
            .unwrap();
        let resp = self
            .client
            .put(format!("{}/threads/{}/result", self.base, thread.id.0))
            .bearer_auth(&self.alice_tok)
            .json(&json!({ "result": { "status": "done" }, "claim_lease_id": lease.0 }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{}", resp.status());
        for action in ["start_review", "close"] {
            let (status, body) = self
                .post(
                    &self.alice_tok,
                    &format!("/threads/{}", thread.id.0),
                    json!({ "action": action }),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{action}: {body}");
        }
        thread.id
    }
}

#[tokio::test]
async fn transition_history_lists_moves_oldest_first_for_a_member() {
    let w = world().await;
    let thread = w.worked_thread().await;

    let (status, rows) = w
        .get(&w.alice_tok, &format!("/threads/{}/transitions", thread.0))
        .await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    let rows = rows.as_array().expect("a JSON array of transitions");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["from_state"], "open");
    assert_eq!(rows[0]["to_state"], "in_review");
    assert_eq!(rows[0]["actor_id"], w.alice.0.to_string());
    assert_eq!(rows[0]["thread_id"], thread.0.to_string());
    assert!(rows[0]["occurred_at"].is_string());
    assert_eq!(rows[1]["from_state"], "in_review");
    assert_eq!(rows[1]["to_state"], "closed");
}

#[tokio::test]
async fn transition_history_honours_the_limit() {
    let w = world().await;
    let thread = w.worked_thread().await;

    let (status, rows) = w
        .get(
            &w.alice_tok,
            &format!("/threads/{}/transitions?limit=1", thread.0),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    let rows = rows.as_array().expect("a JSON array of transitions");
    assert_eq!(rows.len(), 1, "{rows:?}");
    // Oldest first: the capped page holds the first move, not the last.
    assert_eq!(rows[0]["to_state"], "in_review");
}

#[tokio::test]
async fn transition_history_is_hidden_from_another_tenant() {
    let w = world().await;
    let thread = w.worked_thread().await;

    let (status, _) = w
        .get(&w.bob_tok, &format!("/threads/{}/transitions", thread.0))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = w
        .get(
            &w.alice_tok,
            &format!("/threads/{}/transitions", uuid::Uuid::now_v7()),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
