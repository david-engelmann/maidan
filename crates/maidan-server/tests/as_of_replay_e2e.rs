//! Cluster 326: as-of context replay. `GET /threads/:id/context?as_of=<event_id>`
//! reconstructs the thread as it stood at that event-log id — from the immutable
//! event log, so a since-edited message shows its as-of body and a since-tombstoned
//! message reappears. Deterministic; no fresh search.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EditMessage, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn as_of_replay_reconstructs_the_thread_at_a_point() {
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
        .create_workspace(NewWorkspace { name: "r".into() })
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

    let new_msg = |body: &str| NewMessage {
        thread_id: thread.id,
        author_id: member.id,
        body: body.into(),
        metadata: json!({}),
        content: None,
    };
    // Post two messages, capturing the event-log id of each (the `_with_event`
    // path appends the MessagePosted event and returns it).
    let (msg1, _e1) = store
        .post_message_with_event(new_msg("v1"), None)
        .await
        .unwrap();
    let (_msg2, e2) = store
        .post_message_with_event(new_msg("second"), None)
        .await
        .unwrap();
    // Edit msg1 after the as-of point.
    let (_edited, e3) = store
        .edit_message_with_event(
            msg1.id,
            member.id,
            EditMessage {
                body: "v2".into(),
                metadata: json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    // Tombstone the second message after e3.
    let e4 = store
        .tombstone_message_with_event(_msg2.id, None)
        .await
        .unwrap();

    let ctx = |q: &str| {
        let client = client.clone();
        let url = format!("{base}/threads/{}/context{q}", thread.id.0);
        async move {
            client
                .get(url)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    // As-of e2 (after both posts, before the edit + tombstone): msg1 shows its
    // ORIGINAL body, msg2 is present.
    let at_e2 = ctx(&format!("?as_of={}", e2.id)).await;
    let msgs = at_e2["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "both messages existed at e2");
    let m1 = msgs.iter().find(|m| m["id"] == json!(msg1.id.0)).unwrap();
    assert_eq!(m1["body"], json!("v1"), "as-of body is the original");
    // As-of packs omit the glossary (current vocabulary, not thread history).
    assert!(at_e2.get("glossary").is_none());

    // As-of e3 (after the edit, before the tombstone): msg1 shows the edited body,
    // msg2 still present.
    let at_e3 = ctx(&format!("?as_of={}", e3.id)).await;
    let msgs = at_e3["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    let m1 = msgs.iter().find(|m| m["id"] == json!(msg1.id.0)).unwrap();
    assert_eq!(m1["body"], json!("v2"), "as-of e3 body is edited");

    // As-of e3 (before tombstone) shows msg2; as-of e4 (the tombstone) drops it.
    let at_e4 = ctx(&format!("?as_of={}", e4.id)).await;
    assert_eq!(
        at_e4["messages"].as_array().unwrap().len(),
        1,
        "the tombstoned message is gone as of its tombstone event"
    );

    // Live pack: msg1 edited, msg2 tombstoned (gone).
    let live = ctx("").await;
    let msgs = live["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["body"], json!("v2"));

    // Unknown as_of id -> 404.
    let bad = client
        .get(format!(
            "{base}/threads/{}/context?as_of=999999999",
            thread.id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::NOT_FOUND);
}
