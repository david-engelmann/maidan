//! Cluster 338: `post_message` still routes `@handle` mentions after the
//! round-trip reduction (the redundant `resolve_message_chain` was dropped and a
//! no-`@handle` post now short-circuits before any store work). A post that
//! mentions a member emits exactly one `MentionRecorded` for that member; a plain
//! post emits `MessagePosted` and no `MentionRecorded`.

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{Event, EventFilter, EventKind};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

/// Drain every event available within a short window and return them in order.
async fn drain(
    subscriber: &mut (impl StreamExt<Item = BusItem> + Unpin),
    window: Duration,
) -> Vec<Event> {
    let mut events = Vec::new();
    let _ = tokio::time::timeout(window, async {
        while let Some(item) = subscriber.next().await {
            if let BusItem::Event(envelope) = item {
                events.push(envelope.event);
            }
        }
    })
    .await;
    events
}

#[tokio::test]
async fn post_message_routes_at_handles_and_skips_plain_posts() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let mut subscriber = bus.subscribe(EventFilter::all()).await.unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus.clone(), search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "mentions"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap().to_string();
    let author: serde_json::Value = client
        .post(format!("{base}/workspaces/{wid}/members"))
        .json(&json!({"handle": "author", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author_id = author["id"].as_str().unwrap().to_string();
    let target: serde_json::Value = client
        .post(format!("{base}/workspaces/{wid}/members"))
        .json(&json!({"handle": "mentioned", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let target_id = target["id"].as_str().unwrap().to_string();
    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{wid}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap();
    let th: serde_json::Value = client
        .post(format!("{base}/channels/{cid}/threads"))
        .json(&json!({"title": "t"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();

    // Drain the setup events (workspace/member/member/channel/thread).
    let _ = drain(&mut subscriber, Duration::from_millis(300)).await;

    // A post that mentions @mentioned → MessagePosted + exactly one MentionRecorded
    // for the target member.
    client
        .post(format!("{base}/threads/{tid}/messages"))
        .json(&json!({"author_id": author_id, "body": "hey @mentioned please look"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let after_mention = drain(&mut subscriber, Duration::from_millis(500)).await;
    let mentions: Vec<&Event> = after_mention
        .iter()
        .filter(|e| e.kind() == EventKind::MentionRecorded)
        .collect();
    assert_eq!(
        mentions.len(),
        1,
        "expected exactly one MentionRecorded, got {after_mention:?}"
    );
    if let Event::MentionRecorded { member_id, .. } = mentions[0] {
        assert_eq!(member_id.0.to_string(), target_id);
    } else {
        panic!("not a MentionRecorded");
    }

    // A plain post → MessagePosted, and no MentionRecorded (the short-circuit).
    client
        .post(format!("{base}/threads/{tid}/messages"))
        .json(&json!({"author_id": author_id, "body": "just a plain note, no handles"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let after_plain = drain(&mut subscriber, Duration::from_millis(500)).await;
    assert!(
        after_plain
            .iter()
            .any(|e| e.kind() == EventKind::MessagePosted),
        "plain post should still emit MessagePosted"
    );
    assert!(
        !after_plain
            .iter()
            .any(|e| e.kind() == EventKind::MentionRecorded),
        "plain post must not emit MentionRecorded, got {after_plain:?}"
    );

    server.abort();
}
