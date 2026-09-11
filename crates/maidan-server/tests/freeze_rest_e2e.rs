//! Member-freeze kill-switch over HTTP (Cluster 372.3, Wave 2 #20). Auth ENABLED
//! (the `frozen_by` FK + real `token:admin` checks): freeze drops the member's
//! lease, list/get surface it, unfreeze lifts it, and a non-admin token is denied.

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
use serde_json::Value;
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
async fn freeze_member_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let op = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, agent.id).await.unwrap();

    let admin = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![capability::TOKEN_ADMIN.into()],
    )
    .await;
    let admin_h = format!("Bearer {admin}");

    // Freeze the agent → drops the lease (released 1).
    let resp = client
        .post(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .json(&serde_json::json!({ "reason": "compromised" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let result: Value = resp.json().await.unwrap();
    assert_eq!(result["released"], 1);
    assert_eq!(result["freeze"]["member_id"], agent.id.0.to_string());
    // The claimed thread is back in the queue.
    assert_eq!(store.get_thread(thread.id).await.unwrap().assignee_id, None);

    // Get + list surface the freeze.
    let got = client
        .get(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::OK);
    let list: Value = client
        .get(format!("{base}/workspaces/{}/frozen-members", ws.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    // A non-admin token is denied (403).
    let plain = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;
    let denied = client
        .post(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", format!("Bearer {plain}"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    // Unfreeze → gone (404 on repeat get / unfreeze).
    let del = client
        .delete(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let gone = client
        .get(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}
