//! Cluster 346: projector link-management REST — link/list/unlink a Slack channel
//! or GitHub issue to a Maidan thread. Without this surface the Slack/GitHub
//! projector egress could never fire (the link table could not be populated). The
//! test proves a created link is exactly what the egress reverse-lookup reads.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn projector_links_link_list_and_unlink() {
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
    let bus = Arc::new(InMemoryBus::new());

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "g".into(),
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

    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");
    let wid = ws.id.0;

    // --- Slack ---
    let created: Value = client
        .post(format!("{base}/workspaces/{wid}/slack-links"))
        .json(&json!({
            "slack_channel_id": "C12345",
            "thread_id": thread.id.0,
            "member_id": member.id.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // channel_id / workspace_id are derived from the thread, not the client.
    assert_eq!(created["channel_id"], channel.id.0.to_string());
    assert_eq!(created["workspace_id"], wid.to_string());

    // The egress reverse-lookup (what the projector reads) now resolves.
    let by_thread = store
        .get_slack_channel_link_by_thread(thread.id)
        .await
        .unwrap()
        .expect("egress lookup finds the link");
    assert_eq!(by_thread.slack_channel_id, "C12345");

    let list: Value = client
        .get(format!("{base}/workspaces/{wid}/slack-links"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    let del = client
        .delete(format!("{base}/workspaces/{wid}/slack-links/C12345"))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    assert!(store
        .get_slack_channel_link_by_thread(thread.id)
        .await
        .unwrap()
        .is_none());
    // A second unlink is a 404.
    let del2 = client
        .delete(format!("{base}/workspaces/{wid}/slack-links/C12345"))
        .send()
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NOT_FOUND);

    // --- GitHub ---
    let gh: Value = client
        .post(format!("{base}/workspaces/{wid}/github-links"))
        .json(&json!({
            "repo": "acme/widgets",
            "issue_number": 42,
            "thread_id": thread.id.0,
            "member_id": member.id.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(gh["channel_id"], channel.id.0.to_string());
    let gh_by_thread = store
        .get_github_issue_link_by_thread(thread.id)
        .await
        .unwrap()
        .expect("egress lookup finds the github link");
    assert_eq!(gh_by_thread.repo, "acme/widgets");
    assert_eq!(gh_by_thread.issue_number, 42);

    // Unlink via the query pair.
    let gdel = client
        .delete(format!("{base}/workspaces/{wid}/github-links"))
        .query(&[("repo", "acme/widgets"), ("issue_number", "42")])
        .send()
        .await
        .unwrap();
    assert_eq!(gdel.status(), StatusCode::NO_CONTENT);
    assert!(store
        .get_github_issue_link_by_thread(thread.id)
        .await
        .unwrap()
        .is_none());
    // Missing query params → 400 (not a silent no-op).
    let bad = client
        .delete(format!("{base}/workspaces/{wid}/github-links"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}
