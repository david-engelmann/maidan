//! Task-schedule management over HTTP (Cluster 228): create / list / pause-resume
//! / delete. Runs with auth ENABLED so `created_by` is a real member and channel
//! access is exercised.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
            ],
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
async fn task_schedule_crud_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
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
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");

    // Create a recurring schedule.
    let resp = client
        .post(format!("{base}/workspaces/{}/task-schedules", ws.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "channel_id": channel.id.0, "title": "nightly", "interval_secs": 3600 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Value = resp.json().await.unwrap();
    assert_eq!(created["title"], json!("nightly"));
    assert_eq!(created["interval_secs"], json!(3600));
    assert_eq!(created["active"], json!(true));
    let sched_id = created["id"].as_str().unwrap().to_string();

    // List.
    let list: Value = client
        .get(format!("{base}/workspaces/{}/task-schedules", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Pause.
    let paused: Value = client
        .put(format!("{base}/task-schedules/{sched_id}"))
        .header("Authorization", &bearer)
        .json(&json!({ "active": false }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(paused["active"], json!(false));

    // Delete, then it's gone (404 on repeat).
    let del = client
        .delete(format!("{base}/task-schedules/{sched_id}"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let del2 = client
        .delete(format!("{base}/task-schedules/{sched_id}"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NOT_FOUND);

    // Validation: non-positive interval and empty title are 400.
    for bad in [
        json!({ "channel_id": channel.id.0, "title": "x", "interval_secs": 0 }),
        json!({ "channel_id": channel.id.0, "title": "   " }),
    ] {
        let r = client
            .post(format!("{base}/workspaces/{}/task-schedules", ws.id.0))
            .header("Authorization", &bearer)
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "bad body: {bad}");
    }
}
