//! Workspace export (Cluster 187): `token:admin` gets the whole content
//! bundle; a plain reader is denied.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
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
async fn export_requires_token_admin_and_returns_the_content_graph() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "exp-ws".into(),
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
            private: false,
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
            body: "hello export".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let mint = |caps: Vec<String>| {
        let secret = TokenSecret::generate();
        let store = store.clone();
        let member_id = alice.id;
        let ws_id = ws.id;
        let hash = hash_secret(secret.as_str());
        async move {
            store
                .create_api_token(NewApiToken {
                    workspace_id: ws_id,
                    member_id,
                    app_installation_id: None,
                    token_hash: hash,
                    label: None,
                    capabilities: caps,
                    expires_at: None,
                })
                .await
                .unwrap();
            secret
        }
    };

    // A plain reader is denied.
    let reader = mint(vec![capability::WORKSPACE_READ.into()]).await;
    let denied = client
        .get(format!("{base}/workspaces/{}/export", ws.id.0))
        .header("authorization", format!("Bearer {}", reader.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    // token:admin gets the bundle.
    let admin = mint(vec![capability::TOKEN_ADMIN.into()]).await;
    let ok = client
        .get(format!("{base}/workspaces/{}/export", ws.id.0))
        .header("authorization", format!("Bearer {}", admin.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let bundle: serde_json::Value = ok.json().await.unwrap();

    assert_eq!(bundle["format_version"], 1);
    assert_eq!(
        bundle["workspace"]["id"].as_str().unwrap(),
        ws.id.0.to_string()
    );
    assert_eq!(bundle["members"].as_array().unwrap().len(), 1);
    assert_eq!(bundle["channels"].as_array().unwrap().len(), 1);
    assert_eq!(
        bundle["channels"][0]["channel"]["name"].as_str().unwrap(),
        "general"
    );
    assert_eq!(bundle["threads"].as_array().unwrap().len(), 1);
    let messages = bundle["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["body"].as_str().unwrap(), "hello export");

    server.abort();
}
