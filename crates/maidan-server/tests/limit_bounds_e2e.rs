//! A client-supplied `limit` reaches SQL as `LIMIT ?` and the rows come back
//! through `fetch_all`. Four list endpoints and all three search modes passed
//! it through unbounded, so `?limit=9223372036854775807` asked the server to
//! materialise a table into memory.
//!
//! The repo's own convention — `clamp(1, 500)` — was already in use three
//! functions away in the same file.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

/// 501 messages, then ask for every row in the universe. A bounded endpoint
/// returns the cap; an unbounded one returns everything it has.
#[tokio::test]
async fn an_absurd_limit_is_clamped_not_honoured() {
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

    let ws = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "author".into(),
            display_name: None,
            kind: MemberKind::Agent,
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
            title: Some("busy".into()),
        })
        .await
        .unwrap();

    // One past the cap, so "returned everything" and "returned the cap" differ.
    for i in 0..501 {
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: format!("message {i}"),
                content: None,
                metadata: serde_json::Value::Null,
            })
            .await
            .unwrap();
    }

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(16)),
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

    let client = reqwest::Client::new();
    let bearer = format!("Bearer {}", secret.as_str());
    let url = format!("http://{addr}/threads/{}/messages", thread.id.0);

    let rows: Value = client
        .get(format!("{url}?limit={}", i64::MAX))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        rows.as_array().unwrap().len(),
        500,
        "an unbounded limit must be capped, not honoured"
    );

    // A sane limit is still honoured exactly — the clamp is a ceiling, not a
    // rewrite.
    let rows: Value = client
        .get(format!("{url}?limit=7"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 7);

    // And zero or negative cannot produce an error or an empty page by
    // accident: the floor is 1.
    let rows: Value = client
        .get(format!("{url}?limit=0"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
}
