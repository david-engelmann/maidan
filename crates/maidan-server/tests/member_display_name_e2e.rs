//! Setting a member's display name over REST.
//!
//! Creation was the only way to get one, so a member created without a
//! display name stayed nameless in every surface that shows one. The route
//! sets or clears the caller's own name, refuses another member's, and
//! answers 404 to another tenant, for whom the member does not exist.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct World {
    base: String,
    client: reqwest::Client,
    alice: MemberId,
    alice_tok: String,
    alice_read_tok: String,
    carol_tok: String,
    bob_tok: String,
    _dir: tempfile::TempDir,
}

async fn mint(store: &Arc<dyn Store>, ws: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
    let secret = TokenSecret::generate();
    store
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
    let member = |ws: WorkspaceId, handle: &str| NewMember {
        workspace_id: ws,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    let alice = store
        .create_member(member(ws_a.id, "alice"))
        .await
        .unwrap()
        .id;
    let carol = store
        .create_member(member(ws_a.id, "carol"))
        .await
        .unwrap()
        .id;
    let bob = store
        .create_member(member(ws_b.id, "bob"))
        .await
        .unwrap()
        .id;
    let rw = &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE];
    let alice_tok = mint(&store, ws_a.id, alice, rw).await;
    let alice_read_tok = mint(&store, ws_a.id, alice, &[capability::WORKSPACE_READ]).await;
    let carol_tok = mint(&store, ws_a.id, carol, rw).await;
    let bob_tok = mint(&store, ws_b.id, bob, rw).await;

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
        alice,
        alice_tok,
        alice_read_tok,
        carol_tok,
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

    async fn patch(&self, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let resp = self
            .client
            .patch(format!("{}{path}", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    fn alice_path(&self) -> String {
        format!("/members/{}", self.alice.0)
    }
}

#[tokio::test]
async fn a_member_sets_and_clears_their_own_display_name() {
    let w = world().await;
    let (status, body) = w
        .patch(
            &w.alice_tok,
            &w.alice_path(),
            json!({ "display_name": "  Alice A.  " }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], json!("Alice A."), "{body}");

    let (status, body) = w.get(&w.alice_tok, &w.alice_path()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], json!("Alice A."), "{body}");

    let (status, body) = w
        .patch(
            &w.alice_tok,
            &w.alice_path(),
            json!({ "display_name": " " }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], Value::Null, "{body}");
}

#[tokio::test]
async fn another_members_name_is_not_yours_to_set() {
    let w = world().await;
    let (status, _) = w
        .patch(
            &w.carol_tok,
            &w.alice_path(),
            json!({ "display_name": "Not Alice" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn setting_a_name_takes_workspace_write() {
    let w = world().await;
    let (status, _) = w
        .patch(
            &w.alice_read_tok,
            &w.alice_path(),
            json!({ "display_name": "Alice" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn another_tenants_member_and_an_unknown_id_are_404() {
    let w = world().await;
    let (status, _) = w
        .patch(
            &w.bob_tok,
            &w.alice_path(),
            json!({ "display_name": "Alice" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = w
        .patch(
            &w.alice_tok,
            &format!("/members/{}", uuid::Uuid::now_v7()),
            json!({ "display_name": "Ghost" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
