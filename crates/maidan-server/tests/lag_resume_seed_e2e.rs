//! A bus consumer resumes from where it attached, not from id 1.
//!
//! Every always-on consumer declared `let mut watermark: i64 = 0;` *inside* its
//! consume loop, and the loop is re-entered on every resubscribe. A
//! `BusItem::Lagged` arriving before the first event therefore resumed from 0 —
//! replaying the entire global log, across every workspace. For
//! `fsm_hook_worker` that means re-firing every historical hook through
//! `dispatch_mcp_tool` with `AuthContext::bypass()`; for `webhook_worker`,
//! re-POSTing all history to every tenant's endpoint.
//!
//! This pins the consequence directly: resuming from the attach watermark
//! replays nothing, while resuming from 0 replays everything.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{event_stream::attach_watermark, AppState, FederationRuntime};
use maidan_store::{prelude::*, resume_from_log, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Fixture {
    state: AppState,
    store: Arc<dyn Store>,
    thread: maidan_types::ThreadId,
    member: maidan_types::MemberId,
}

async fn state_with_history(events: usize) -> Fixture {
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

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
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
    for i in 0..events {
        let (_, stored) = store
            .post_message_with_event(
                NewMessage {
                    thread_id: thread.id,
                    author_id: member.id,
                    body: format!("m{i}"),
                    metadata: json!({}),
                    content: None,
                },
                None,
            )
            .await
            .unwrap();
        let _ = stored;
    }

    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    std::mem::forget(dir);
    Fixture {
        state,
        store,
        thread: thread.id,
        member: member.id,
    }
}

async fn replayed_from(store: &dyn Store, after_id: i64) -> usize {
    let mut seen = 0usize;
    resume_from_log(store, after_id, |page| {
        seen += page.len();
        async {}
    })
    .await
    .unwrap();
    seen
}

#[tokio::test]
async fn a_consumer_attaching_to_a_populated_log_replays_nothing_on_lag() {
    let Fixture { state, store, .. } = state_with_history(5).await;

    // There is real history to replay.
    let head = store.max_event_id().await.unwrap();
    assert!(
        head > 0,
        "the log must be populated for this test to mean anything"
    );

    // The old behaviour: a Lagged before the first event resumed from 0.
    let from_zero = replayed_from(store.as_ref(), 0).await;
    assert_eq!(
        from_zero, head as usize,
        "resuming from 0 replays the whole global log — this is what the bug did"
    );

    // The fix: attach at the head, so the same Lagged replays nothing.
    let seed = attach_watermark(&state).await.unwrap();
    assert_eq!(seed, head, "a consumer attaches at the current head");
    assert_eq!(
        replayed_from(store.as_ref(), seed).await,
        0,
        "a consumer that has missed nothing must replay nothing"
    );
}

/// The seed only suppresses history — a genuine gap after attaching still
/// resumes, which is the whole point's lag resume.
#[tokio::test]
async fn events_appended_after_attach_are_still_replayed() {
    let Fixture {
        state,
        store,
        thread,
        member,
    } = state_with_history(3).await;
    let seed = attach_watermark(&state).await.unwrap();

    // Three more land while this subscriber is wedged.
    for i in 0..3 {
        store
            .post_message_with_event(
                NewMessage {
                    thread_id: thread,
                    author_id: member,
                    body: format!("late{i}"),
                    metadata: json!({}),
                    content: None,
                },
                None,
            )
            .await
            .unwrap();
    }

    assert_eq!(
        replayed_from(store.as_ref(), seed).await,
        3,
        "the missed window, and only the missed window"
    );
}
