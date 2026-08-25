//! Workspace import (Cluster 270): export a workspace, then import the bundle
//! back — `mode=new` remaps to a fresh workspace; `mode=restore` preserves ids
//! (409 if it already exists, unless `force` erases it first). `token:admin` gated.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
    WorkspaceId,
};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
    tokio::task::JoinHandle<()>,
) {
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
        false,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store, server)
}

#[tokio::test]
async fn import_remaps_in_new_mode_and_restores_ids_with_conflict_and_force() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    // Seed a workspace with a private channel, a thread, and a message.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "orig".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: alice.id,
            body: "hello import".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let admin_secret = {
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id: alice.id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![capability::TOKEN_ADMIN.into()],
                expires_at: None,
            })
            .await
            .unwrap();
        secret
    };
    let auth = format!("Bearer {}", admin_secret.as_str());

    // Export the workspace bundle.
    let bundle: serde_json::Value = client
        .get(format!("{base}/workspaces/{}/export", ws.id.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // --- mode=new: remaps to a fresh workspace ---
    let resp = client
        .post(format!("{base}/workspaces/import?mode=new"))
        .header("authorization", &auth)
        .json(&bundle)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let result: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(result["mode"], "new");
    let new_ws_id: uuid::Uuid = result["workspace_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(new_ws_id, ws.id.0, "new mode assigns a fresh workspace id");

    // The fresh workspace holds the same content under new ids.
    let new_ws = WorkspaceId(new_ws_id);
    assert_eq!(store.list_members(new_ws).await.unwrap().len(), 1);
    let new_channels = store.list_channels(new_ws).await.unwrap();
    assert_eq!(new_channels.len(), 1);
    assert!(new_channels[0].private);
    assert_ne!(new_channels[0].id, channel.id, "channel id remapped");
    let new_threads = store.list_threads(new_channels[0].id).await.unwrap();
    assert_eq!(new_threads.len(), 1);
    let new_msgs = store.list_messages(new_threads[0].id, 100).await.unwrap();
    assert_eq!(new_msgs[0].body, "hello import");

    // --- mode=restore while the original still exists: 409 ---
    let conflict = client
        .post(format!("{base}/workspaces/import?mode=restore"))
        .header("authorization", &auth)
        .json(&bundle)
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    // --- mode=restore&force: erase the existing workspace and restore same ids ---
    let forced = client
        .post(format!("{base}/workspaces/import?mode=restore&force=true"))
        .header("authorization", &auth)
        .json(&bundle)
        .send()
        .await
        .unwrap();
    assert_eq!(forced.status(), StatusCode::OK);
    let forced_result: serde_json::Value = forced.json().await.unwrap();
    assert_eq!(
        forced_result["workspace_id"].as_str().unwrap(),
        ws.id.0.to_string(),
        "restore preserves the original workspace id"
    );
    // The original ids are back with their content.
    let restored_channels = store.list_channels(ws.id).await.unwrap();
    assert_eq!(restored_channels.len(), 1);
    assert_eq!(restored_channels[0].id, channel.id, "channel id preserved");
    let restored_msgs = store.list_messages(thread.id, 100).await.unwrap();
    assert_eq!(restored_msgs[0].body, "hello import");

    server.abort();
}
