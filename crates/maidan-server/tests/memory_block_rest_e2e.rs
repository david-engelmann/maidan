//! Attachable labeled memory blocks over HTTP (Cluster 373.2, Wave 2 #21, H11).
//! Auth ENABLED (owner_id is a real member FK): create/get/list, full-rewrite
//! set_value, read-only + over-limit → 400, attach/detach on a thread, delete,
//! and a read-only token denied on a write (403).

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
async fn memory_blocks_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
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

    let writer = mint(
        store.as_ref(),
        ws.id,
        owner.id,
        vec![
            capability::WORKSPACE_WRITE.into(),
            capability::WORKSPACE_READ.into(),
        ],
    )
    .await;
    let wh = format!("Bearer {writer}");

    // Create a block (limit 10).
    let created: Value = client
        .post(format!("{base}/workspaces/{}/memory-blocks", ws.id.0))
        .header("Authorization", &wh)
        .json(&json!({ "label": "shared", "char_limit": 10, "value": "hello" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["label"], "shared");
    assert_eq!(created["value"], "hello");
    let block_id = created["id"].as_str().unwrap().to_string();

    // Get + list.
    let got = client
        .get(format!(
            "{base}/workspaces/{}/memory-blocks/{block_id}",
            ws.id.0
        ))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::OK);
    let list: Value = client
        .get(format!("{base}/workspaces/{}/memory-blocks", ws.id.0))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Full-rewrite the value.
    let updated: Value = client
        .put(format!(
            "{base}/workspaces/{}/memory-blocks/{block_id}",
            ws.id.0
        ))
        .header("Authorization", &wh)
        .json(&json!({ "value": "world" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["value"], "world");

    // Over the char limit → 400.
    let too_long = client
        .put(format!(
            "{base}/workspaces/{}/memory-blocks/{block_id}",
            ws.id.0
        ))
        .header("Authorization", &wh)
        .json(&json!({ "value": "this is way too long" }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_long.status(), StatusCode::BAD_REQUEST);

    // A read-only block refuses writes → 400.
    let ro: Value = client
        .post(format!("{base}/workspaces/{}/memory-blocks", ws.id.0))
        .header("Authorization", &wh)
        .json(&json!({ "label": "frozen", "read_only": true, "value": "x" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ro_id = ro["id"].as_str().unwrap().to_string();
    let refused = client
        .put(format!(
            "{base}/workspaces/{}/memory-blocks/{ro_id}",
            ws.id.0
        ))
        .header("Authorization", &wh)
        .json(&json!({ "value": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

    // Attach to a thread → 204; the thread lists it.
    let attach = client
        .post(format!(
            "{base}/threads/{}/memory-blocks/{block_id}",
            thread.id.0
        ))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap();
    assert_eq!(attach.status(), StatusCode::NO_CONTENT);
    let attached: Value = client
        .get(format!("{base}/threads/{}/memory-blocks", thread.id.0))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(attached.as_array().unwrap().len(), 1);
    assert_eq!(attached[0]["id"], block_id);

    // Detach → 204; empty. Repeat detach → 404.
    let detach = client
        .delete(format!(
            "{base}/threads/{}/memory-blocks/{block_id}",
            thread.id.0
        ))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap();
    assert_eq!(detach.status(), StatusCode::NO_CONTENT);
    let empty: Value = client
        .get(format!("{base}/threads/{}/memory-blocks", thread.id.0))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(empty.as_array().unwrap().is_empty());

    // Delete a block → 204; get → 404.
    let del = client
        .delete(format!(
            "{base}/workspaces/{}/memory-blocks/{block_id}",
            ws.id.0
        ))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let gone = client
        .get(format!(
            "{base}/workspaces/{}/memory-blocks/{block_id}",
            ws.id.0
        ))
        .header("Authorization", &wh)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    // A read-only token (workspace:read only) is denied on create (403).
    let reader = mint(
        store.as_ref(),
        ws.id,
        owner.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;
    let denied = client
        .post(format!("{base}/workspaces/{}/memory-blocks", ws.id.0))
        .header("Authorization", format!("Bearer {reader}"))
        .json(&json!({ "label": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}
