//! Query-count regression for A2A `ListTasks`.
//!
//! `ListTasks` decides thread access in its store queries, so what one call
//! runs does not depend on how many tasks the caller cannot read. Before, it
//! fetched a page's worth of rows at a time and dropped the unreadable ones,
//! so a caller behind a few hundred hidden tasks cost a few hundred queries a
//! page. Counted as `sqlx::query` tracing events, as
//! `context_query_count_e2e` does; this is its own binary because the counter
//! is the process's global subscriber.

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations, A2aTaskWrite};
use maidan_types::{
    ChannelMemberRole, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace, ThreadId, WorkspaceId,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// Counts every `sqlx::query` tracing event (one per executed statement).
#[derive(Clone, Default)]
struct QueryCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for QueryCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target().starts_with("sqlx::query") {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

async fn member(store: &dyn Store, workspace_id: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn thread(store: &dyn Store, workspace_id: WorkspaceId, private: bool) -> ThreadId {
    let channel = store
        .create_channel(NewChannel {
            workspace_id,
            name: if private { "secret" } else { "general" }.into(),
            topic: None,
            private,
        })
        .await
        .unwrap();
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
        })
        .await
        .unwrap()
        .id
}

/// A completed task on `thread_id`, stamped now: each seeded task is newer
/// than the last.
async fn task(store: &dyn Store, workspace_id: WorkspaceId, thread_id: ThreadId) {
    tokio::time::sleep(Duration::from_millis(2)).await;
    let id = uuid::Uuid::now_v7().to_string();
    let at = Utc::now();
    store
        .upsert_a2a_task(A2aTaskWrite {
            workspace_id,
            task_id: &id,
            context_id: Some(&thread_id.0.to_string()),
            thread_id: Some(thread_id),
            state: "TASK_STATE_COMPLETED",
            status_at: at,
            task_json: json!({
                "id": id,
                "contextId": thread_id.0.to_string(),
                "status": {
                    "state": "TASK_STATE_COMPLETED",
                    "timestamp": at.to_rfc3339_opts(SecondsFormat::Millis, true),
                },
            }),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn list_tasks_query_count_is_independent_of_hidden_tasks() {
    let counter = QueryCounter::default();
    tracing_subscriber::registry().with(counter.clone()).init();

    // One connection, never pinged on acquire: a pool-issued statement the
    // test does not control would count as a phantom query.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .test_before_acquire(false)
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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ws = store
        .create_workspace(NewWorkspace { name: "qc".into() })
        .await
        .unwrap()
        .id;
    let caller = member(store.as_ref(), ws, "caller").await;
    let insider = member(store.as_ref(), ws, "insider").await;
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: caller,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::MESSAGE_POST.to_string()],
            expires_at: None,
        })
        .await
        .unwrap();
    let open = thread(store.as_ref(), ws, false).await;
    let hidden = thread(store.as_ref(), ws, true).await;
    let private = store.get_thread(hidden).await.unwrap().channel_id;
    store
        .add_channel_member(private, insider, ChannelMemberRole::Member)
        .await
        .unwrap();

    let http = reqwest::Client::new();
    let list = || async {
        let resp: Value = http
            .post(format!("http://{addr}/a2a/v1/rpc"))
            .bearer_auth(secret.as_str())
            .header("A2A-Version", "1.0")
            .json(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "ListTasks",
                "params": { "pageSize": 1, "historyLength": 0 },
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let result = resp["result"].clone();
        assert_eq!(result["tasks"].as_array().map(Vec::len), Some(1), "{resp}");
        result
    };
    let measure = || async {
        // Warm the statement cache so neither measured call counts a
        // one-time prepare.
        list().await;
        counter.0.store(0, Ordering::SeqCst);
        let result = list().await;
        (counter.0.load(Ordering::SeqCst), result)
    };

    // Two tasks the caller can read, the oldest, behind hidden ones.
    for _ in 0..2 {
        task(store.as_ref(), ws, open).await;
    }
    for _ in 0..4 {
        task(store.as_ref(), ws, hidden).await;
    }
    let (few, result) = measure().await;
    assert_eq!(result["totalSize"], 2);
    assert!(few >= 3, "expected several queries per listing, got {few}");

    for _ in 0..200 {
        task(store.as_ref(), ws, hidden).await;
    }
    let (many, result) = measure().await;
    assert_eq!(result["totalSize"], 2);
    assert_ne!(
        result["nextPageToken"], "",
        "a second readable task follows"
    );

    // A per-batch scan past 200 more hidden tasks costs ~100 more queries at
    // a page of one; one query of slack cannot mask it.
    assert!(
        many <= few + 1,
        "ListTasks query count must not grow with hidden tasks (few={few}, many={many})"
    );
}
