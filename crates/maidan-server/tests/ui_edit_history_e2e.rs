//! UI v5 edit history: message edits API and shell markers.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    EditMessage, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    std::net::SocketAddr,
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (addr, client, store, server)
}

#[tokio::test]
async fn ui_v5_edit_history_shell_and_session_edits_api() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let html = client
        .get(format!("{base}/ui/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains(r#"data-ui-version="7""#));
    assert!(html.contains("load-edit-history"));
    assert!(html.contains("edit-history-list"));

    let ws = store
        .create_workspace(NewWorkspace {
            name: "edit-ui".into(),
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
    let ch = store
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
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: alice.id,
            body: "version one".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    store
        .edit_message(
            msg.id,
            alice.id,
            EditMessage {
                body: "version two".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
        )
        .await
        .unwrap();

    let edits: Vec<serde_json::Value> = client
        .get(format!("{base}/ui/api/messages/{}/edits", msg.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["body_before"], "version one");
    assert_eq!(edits[0]["body_after"], "version two");

    let via_http: Vec<serde_json::Value> = client
        .get(format!("{base}/messages/{}/edits", msg.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(via_http.len(), 1);

    server.abort();
}
