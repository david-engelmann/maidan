//! Spawn-budget config over HTTP (Cluster 376.4, Wave 2 #23). Auth ENABLED so
//! the capability gate is exercised: set/get the three axes, prove the *set*
//! budget is what the Cluster-376.2 gate enforces (a child past `max_children`
//! is 409), prove clearing it re-opens spawning, and prove a token without
//! `workspace:write` cannot change it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn spawn_budget_config_over_http_drives_the_gate() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let operator = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let parent = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parent".into()),
        })
        .await
        .unwrap();

    let admin = mint(
        store.as_ref(),
        ws.id,
        operator.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;
    let reader = mint(
        store.as_ref(),
        ws.id,
        operator.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;

    let set = |token: String, body: Value| {
        let (client, base, wid) = (client.clone(), base.clone(), ws.id.0);
        async move {
            client
                .put(format!("{base}/workspaces/{wid}/spawn-budget"))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };
    let get = |token: String| {
        let (client, base, wid) = (client.clone(), base.clone(), ws.id.0);
        async move {
            client
                .get(format!("{base}/workspaces/{wid}/spawn-budget"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
        }
    };
    let spawn_child = |token: String| {
        let (client, base, cid, pid) = (client.clone(), base.clone(), channel.id.0, parent.id.0);
        async move {
            client
                .post(format!("{base}/channels/{cid}/threads"))
                .bearer_auth(token)
                .json(&json!({ "title": "child", "parent_thread_id": pid }))
                .send()
                .await
                .unwrap()
        }
    };

    // Unset = unlimited on every axis (no 404).
    let before = get(admin.clone()).await;
    assert_eq!(before.status(), StatusCode::OK);
    let before: Value = before.json().await.unwrap();
    assert!(before["max_children"].is_null());
    assert!(before["max_depth"].is_null());
    assert!(before["max_tools"].is_null());

    // Set all three axes; the response echoes what was stored.
    let put = set(
        admin.clone(),
        json!({ "max_children": 1, "max_depth": 2, "max_tools": 8 }),
    )
    .await;
    assert_eq!(put.status(), StatusCode::OK);
    let put: Value = put.json().await.unwrap();
    assert_eq!(put["max_children"], 1);
    assert_eq!(put["max_depth"], 2);
    assert_eq!(put["max_tools"], 8);
    let read_back: Value = get(reader.clone()).await.json().await.unwrap();
    assert_eq!(read_back, put, "GET must read back what PUT stored");

    // The configured cap is the one the gate enforces: one child is fine, the
    // second is refused (409 Conflict — SpawnRejected).
    assert_eq!(
        spawn_child(admin.clone()).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        spawn_child(admin.clone()).await.status(),
        StatusCode::CONFLICT,
        "a 2nd child past max_children=1 must be refused"
    );

    // A partial PUT is a full replace: naming only max_tools clears the rest,
    // so the child that was just refused goes through.
    let put: Value = set(admin.clone(), json!({ "max_tools": 8 }))
        .await
        .json()
        .await
        .unwrap();
    assert!(put["max_children"].is_null());
    assert_eq!(put["max_tools"], 8);
    assert_eq!(
        spawn_child(admin.clone()).await.status(),
        StatusCode::CREATED
    );

    // `{}` clears the budget row entirely.
    assert_eq!(set(admin.clone(), json!({})).await.status(), StatusCode::OK);
    assert!(store.get_spawn_budget(ws.id).await.unwrap().is_none());

    // A negative axis is a client error, and read-only tokens cannot set it.
    assert_eq!(
        set(admin.clone(), json!({ "max_depth": -1 }))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        set(reader.clone(), json!({ "max_children": 2 }))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}
