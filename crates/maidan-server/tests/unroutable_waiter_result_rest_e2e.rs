//! `PUT /threads/{id}/result` refuses a `maidan.waiter.result/1` envelope
//! with no `result_kind` or `status`. Result routing drops such an envelope,
//! so storing it left a result that was never delivered and nobody was told
//! (#1649). The refusal names the missing field and stores nothing; a
//! routable envelope and a result that is not an envelope still store.

use maidan_store::test_support::SetResultClaiming;
use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn a_waiter_result_without_result_kind_or_status_is_refused_with_400_and_not_stored() {
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
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let worker = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "worker".into(),
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
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, worker.id).await.unwrap();
    let lease = store.claim_lease_for(thread.id, worker.id).await.unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: worker.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::THREAD_TRANSITION.into(),
            ],
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
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    let client = reqwest::Client::new();
    let put = |result: Value| {
        client
            .put(format!("http://{addr}/threads/{}/result", thread.id.0))
            .bearer_auth(secret.as_str())
            .json(&json!({ "claim_lease_id": lease.0, "result": result }))
            .send()
    };

    let refused = put(json!({
        "schema": "maidan.waiter.result/1",
        "deliver_to": [{ "surface": "github", "repo": "o/r", "pr": 1 }],
        "rendered": "a review",
    }))
    .await
    .unwrap();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let text = refused.text().await.unwrap();
    assert!(
        text.contains("`result_kind` and `status`"),
        "the refusal names both missing fields: {text}"
    );
    assert!(
        store.get_thread_result(thread.id).await.unwrap().is_none(),
        "a refused result is not stored"
    );

    let not_an_envelope = put(json!({ "answer": 42 })).await.unwrap();
    assert_eq!(not_an_envelope.status(), StatusCode::OK);
    let routable = put(json!({
        "schema": "maidan.waiter.result/1",
        "result_kind": "example.review.result/1",
        "status": "reviewed",
    }))
    .await
    .unwrap();
    assert_eq!(routable.status(), StatusCode::OK);
}
