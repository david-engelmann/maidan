//! Operator reindex embeddings job API (Cluster 87.0).

use std::sync::Arc;
use std::time::Duration;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_search::{hash_embedding, model_name, sqlite_pool_options, Search, SqliteSearch};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};

#[tokio::test]
async fn operator_reindex_job_indexes_workspace_messages() {
    let pool = sqlite_pool_options()
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
    let search: Arc<dyn Search> = Arc::new(SqliteSearch::new(pool));
    let ws = store
        .create_workspace(NewWorkspace {
            name: "reindex-job".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
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
    let th = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let body = "operator reindex target phrase";
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: member.id,
            body: body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(
        store.clone(),
        artifacts,
        bus,
        search.clone(),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let started: serde_json::Value = client
        .post(format!("{base}/operator/reindex-embeddings"))
        .json(&serde_json::json!({ "workspace_id": ws.id.0 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(started["status"], "running");
    let job_id = started["job_id"].as_str().unwrap();

    let mut completed = false;
    for _ in 0..50 {
        let job: serde_json::Value = client
            .get(format!("{base}/operator/reindex-embeddings/{job_id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if job["status"] == "completed" {
            assert_eq!(job["processed"], 1);
            assert_eq!(job["failed"], 0);
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(completed, "reindex job did not complete");

    let hits = search
        .semantic_search(
            ws.id,
            &hash_embedding(body),
            5,
            &maidan_search::SearchFilters::default(),
            model_name(),
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id, msg.id);

    server.abort();
}
