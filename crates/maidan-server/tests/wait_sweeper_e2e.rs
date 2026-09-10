//! Wait sweeper (Cluster 364, G2/G4): a due wait fires — the sweeper emits
//! `WaitTimedOut`, parks the thread (Park policy), and the notification router
//! notifies the owner. Never a decision.

use std::sync::Arc;

use chrono::{Duration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{notification_router, wait_sweeper, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EscalationPolicy, Event, EventKind, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn wait_sweeper_fires_due_waits_parks_and_notifies() {
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
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let mk_member = |h: &str| NewMember {
        workspace_id: ws.id,
        handle: h.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    let owner = store.create_member(mk_member("owner")).await.unwrap();
    let agent = store.create_member(mk_member("agent")).await.unwrap();
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
            title: Some("waiting".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();

    // A wait already past its deadline, Park policy, created by the agent.
    store
        .set_thread_wait(
            thread.id,
            Utc::now() - Duration::seconds(5),
            EscalationPolicy::Park,
            Some("blocked on external"),
            agent.id,
        )
        .await
        .unwrap();

    // The sweep fires exactly one wait.
    assert_eq!(wait_sweeper::sweep_once(&state).await, 1);

    // The thread is now parked from dispatch (Park policy).
    let parked = store
        .get_thread_unclaimable(thread.id)
        .await
        .unwrap()
        .expect("parked");
    assert!(parked.reason.contains("blocked on external"));

    // A `WaitTimedOut` event was emitted on the thread.
    let events = store.list_events_after(ws.id, 0, 100).await.unwrap();
    let timed_out = events
        .iter()
        .find(|e| e.kind == EventKind::WaitTimedOut)
        .expect("a WaitTimedOut was emitted");
    assert_eq!(timed_out.thread_id, Some(thread.id));
    assert_eq!(timed_out.payload["policy"], "park");

    // A second sweep fires nothing (the wait is already fired).
    assert_eq!(wait_sweeper::sweep_once(&state).await, 0);

    // The notification router notifies the thread's owner (never a decision).
    let event = Event::WaitTimedOut {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        policy: "park".into(),
        reason: Some("blocked on external".into()),
    };
    notification_router::route_event(&state, 1, &event)
        .await
        .unwrap();
    let notes = store.list_notifications(owner.id, false, 10).await.unwrap();
    assert_eq!(notes.len(), 1, "the owner is notified their wait timed out");
    assert_eq!(notes[0].kind, EventKind::WaitTimedOut);
    assert_eq!(notes[0].thread_id, Some(thread.id));
}
