//! Cluster 324: optional confidence weight on a vote. `POST /messages/:id/votes`
//! accepts a `confidence` in 0..=1 (out of range -> 400); `GET` returns it, and a
//! vote cast without one omits the field. Re-casting updates the confidence.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn vote_confidence_round_trips_and_validates() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "v".into() })
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
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "decision".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let votes_url = format!("{base}/messages/{}/votes", msg.id.0);

    // Cast with confidence.
    let cast = client
        .post(&votes_url)
        .json(&json!({ "member_id": member.id.0, "kind": "approve", "confidence": 0.7 }))
        .send()
        .await
        .unwrap();
    assert_eq!(cast.status(), StatusCode::NO_CONTENT);

    let votes: Value = client
        .get(&votes_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(votes.as_array().unwrap().len(), 1);
    assert_eq!(votes[0]["confidence"], json!(0.7));

    // Re-cast the same kind updates the confidence (count stays 1).
    let recast = client
        .post(&votes_url)
        .json(&json!({ "member_id": member.id.0, "kind": "approve", "confidence": 0.3 }))
        .send()
        .await
        .unwrap();
    assert_eq!(recast.status(), StatusCode::NO_CONTENT);
    let votes: Value = client
        .get(&votes_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(votes.as_array().unwrap().len(), 1);
    assert_eq!(votes[0]["confidence"], json!(0.3));

    // A different kind without confidence omits the field.
    let plain = client
        .post(&votes_url)
        .json(&json!({ "member_id": member.id.0, "kind": "ack" }))
        .send()
        .await
        .unwrap();
    assert_eq!(plain.status(), StatusCode::NO_CONTENT);
    let votes: Value = client
        .get(&votes_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ack = votes
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == json!("ack"))
        .unwrap();
    assert!(
        ack.get("confidence").is_none(),
        "no confidence -> field omitted"
    );

    // Out of range -> 400.
    let bad = client
        .post(&votes_url)
        .json(&json!({ "member_id": member.id.0, "kind": "approve", "confidence": 1.5 }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}
