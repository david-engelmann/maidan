//! Cluster 329: immutable context snapshot artifact. `POST /threads/:id/context/
//! snapshot` freezes the assembled pack into the content-addressed artifact store;
//! it is fetchable, tamper-evident (sha256), and deduped (identical packs share a
//! blob). Auth ENABLED (`uploaded_by` is the caller — a NOT-NULL FK).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::Value;
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
                capability::ARTIFACT_UPLOAD.into(),
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
async fn context_snapshot_freezes_the_pack_as_an_artifact() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "cs".into() })
        .await
        .unwrap();
    let member = store
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
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("decide".into()),
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "context to freeze".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");
    let snap_url = format!("{base}/threads/{}/context/snapshot", thread.id.0);

    // Freeze the pack.
    let snap = client
        .post(&snap_url)
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(snap.status(), StatusCode::CREATED);
    let artifact: Value = snap.json().await.unwrap();
    assert_eq!(artifact["kind"], serde_json::json!("context_snapshot"));
    assert_eq!(artifact["mime_type"], serde_json::json!("application/json"));
    let sha = artifact["sha256"].as_str().unwrap().to_string();
    assert!(artifact["size_bytes"].as_i64().unwrap() > 0);

    // The frozen bytes are fetchable and ARE the pack.
    let blob = client
        .get(format!("{base}/artifacts/{sha}"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(blob.status(), StatusCode::OK);
    let pack: Value = blob.json().await.unwrap();
    assert_eq!(
        pack["messages"].as_array().unwrap()[0]["body"],
        serde_json::json!("context to freeze")
    );
    assert_eq!(pack["thread"]["id"], serde_json::json!(thread.id.0));

    // Deduped: an identical snapshot yields the same sha (same bytes, one blob).
    let snap2: Value = client
        .post(&snap_url)
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        snap2["sha256"].as_str().unwrap(),
        sha,
        "identical pack dedups"
    );

    // Without artifact:upload the freeze is forbidden.
    let ro = mint_read_only(store.as_ref(), ws.id, member.id).await;
    let denied = client
        .post(&snap_url)
        .header("Authorization", format!("Bearer {ro}"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

async fn mint_read_only(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}
