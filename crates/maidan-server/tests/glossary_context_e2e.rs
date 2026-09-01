//! Cluster 323: the workspace glossary rides the context pack. A thread's
//! `GET /threads/:id/context` carries the workspace's definitions by default
//! (opt out with `include_glossary=false`); a `GET /workspaces/:wid/context`
//! carries it once at the top level, not repeated per nested thread.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewGlossaryTerm, NewMember, NewThread, NewWorkspace};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn glossary_rides_the_context_pack() {
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
        .create_workspace(NewWorkspace { name: "g".into() })
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
    store
        .set_glossary_term(NewGlossaryTerm {
            workspace_id: ws.id,
            term: "TTL".into(),
            definition: "time to live".into(),
            aliases: vec!["expiry".into()],
            created_by: member.id,
        })
        .await
        .unwrap();

    // Thread context includes the glossary by default.
    let ctx: Value = client
        .get(format!("{base}/threads/{}/context", thread.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let glossary = ctx["glossary"].as_array().expect("glossary present");
    assert_eq!(glossary.len(), 1);
    assert_eq!(glossary[0]["term"], serde_json::json!("TTL"));
    assert_eq!(glossary[0]["definition"], serde_json::json!("time to live"));

    // Opt out drops it entirely (skip_serializing_if).
    let ctx_off: Value = client
        .get(format!(
            "{base}/threads/{}/context?include_glossary=false",
            thread.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        ctx_off.get("glossary").is_none(),
        "glossary dropped when off"
    );

    // Workspace context carries the glossary once at the top, not per thread.
    let wctx: Value = client
        .get(format!("{base}/workspaces/{}/context", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        wctx["glossary"]
            .as_array()
            .expect("top-level glossary")
            .len(),
        1
    );
    let threads = wctx["threads"].as_array().unwrap();
    assert!(!threads.is_empty());
    for t in threads {
        assert!(
            t.get("glossary").is_none(),
            "nested thread must not repeat the glossary"
        );
    }
}
