//! Cluster 358.3 (T1/T5): the budget envelope over REST — set a budget, report
//! usage, and when a claimed run goes over budget it is stopped (claim released +
//! ClaimFailed) and dead-lettered, observable via `GET /channels/:cid/dlq`.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn report_usage_over_budget_stops_and_dead_letters() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));

    let ws = store
        .create_workspace(NewWorkspace { name: "b".into() })
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
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap();
    // A claimed run to stop.
    store.assign_thread(thread.id, agent.id).await.unwrap();

    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");
    let tid = thread.id.0;
    let cid = channel.id.0;

    // Set a token budget.
    let set: Value = client
        .put(format!("{base}/threads/{tid}/budget"))
        .json(&json!({ "max_tokens": 100 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(set["max_tokens"], 100);
    assert_eq!(set["used_tokens"], 0);

    // Report under budget → not stopped.
    let under: Value = client
        .post(format!("{base}/threads/{tid}/usage"))
        .json(&json!({ "tokens": 50 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(under["stopped"], json!(false));
    assert_eq!(under["budget"]["used_tokens"], 50);

    // Report over budget → stopped, reason tokens.
    let over: Value = client
        .post(format!("{base}/threads/{tid}/usage"))
        .json(&json!({ "tokens": 60 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(over["stopped"], json!(true));
    assert_eq!(over["reason"], json!("tokens"));
    assert_eq!(over["budget"]["used_tokens"], 110);

    // The claim was released.
    assert_eq!(
        store.get_thread(thread.id).await.unwrap().assignee_id,
        None,
        "claim released on stop"
    );

    // The dead-letter queue shows the stopped run.
    let dlq: Value = client
        .get(format!("{base}/channels/{cid}/dlq"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dlq = dlq.as_array().unwrap();
    assert_eq!(dlq.len(), 1);
    assert_eq!(dlq[0]["reason"], json!("tokens"));
    assert_eq!(dlq[0]["member_id"], json!(agent.id.0));
    assert_eq!(dlq[0]["used_tokens"], 110);

    // GET the budget back.
    let got: Value = client
        .get(format!("{base}/threads/{tid}/budget"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["used_tokens"], 110);
}
