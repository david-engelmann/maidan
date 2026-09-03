//! Cluster 351: `GET /channels/:cid/occupancy` reports the two-clocks view —
//! queued / claimed / working / blocked — over HTTP, splitting held work by the
//! working clock (whether the holder has acknowledged via `acknowledge_claim`).

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn occupancy_splits_claimed_from_working_over_http() {
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

    // Set the scenario up through the store (the counts logic is store-tested on
    // both backends; this test proves the HTTP route surfaces it).
    let ws = store
        .create_workspace(NewWorkspace {
            name: "occ-ws".into(),
        })
        .await
        .unwrap();
    let member = store
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
            name: "occ".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    // queued: unassigned.
    let _queued = store.create_thread(mk("queued")).await.unwrap();
    // claimed: assigned, not acknowledged.
    let claimed = store.create_thread(mk("claimed")).await.unwrap();
    store.assign_thread(claimed.id, member.id).await.unwrap();
    // working: assigned AND acknowledged.
    let working = store.create_thread(mk("working")).await.unwrap();
    let working = store.assign_thread(working.id, member.id).await.unwrap();
    store
        .acknowledge_claim(working.id, member.id, working.claim_lease_id.unwrap())
        .await
        .unwrap();

    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let occ: serde_json::Value = client
        .get(format!("http://{addr}/channels/{}/occupancy", channel.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(occ["open"], serde_json::json!(3));
    assert_eq!(occ["queued"], serde_json::json!(1));
    assert_eq!(
        occ["claimed"],
        serde_json::json!(1),
        "assigned, not acknowledged"
    );
    assert_eq!(
        occ["working"],
        serde_json::json!(1),
        "assigned + acknowledged"
    );
    assert_eq!(occ["blocked"], serde_json::json!(0));
}
