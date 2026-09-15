//! Search indexer as a tap projector (Cluster 393.4).

use std::{
    sync::{atomic::Ordering, Arc, Mutex},
    time::Duration,
};

use maidan_bus::InMemoryBus;
use maidan_search::{backfill_search, Indexer, LoggingHandler, SearchTap};
use maidan_store::{prelude::*, run_sqlite_migrations, SqliteStore};
use maidan_types::{
    EventKind, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> (SqliteStore, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    (SqliteStore::new(pool.clone()), pool)
}

async fn seed_with_message(store: &dyn Store) -> maidan_types::StoredEvent {
    let (ws, _) = store
        .create_workspace_with_event(NewWorkspace { name: "idx".into() })
        .await
        .unwrap();
    let (member, _) = store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let (ch, _) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let (th, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let (_, posted) = store
        .post_message_with_event(
            NewMessage {
                thread_id: th.id,
                author_id: member.id,
                body: "hello search tap".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    posted
}

#[tokio::test]
async fn backfill_projects_message_posted_then_live_is_caught_up() {
    let (store, _pool) = sqlite().await;
    let posted = seed_with_message(&store).await;
    assert_eq!(posted.kind, EventKind::MessagePosted);

    let mut tap = SearchTap::new();
    let kinds = Arc::new(Mutex::new(Vec::new()));
    let hw = backfill_search(&store, &mut tap, {
        let kinds = kinds.clone();
        move |row| {
            let kinds = kinds.clone();
            async move {
                kinds.lock().unwrap().push(row.kind);
                Ok(())
            }
        }
    })
    .await
    .expect("backfill");
    assert!(hw >= posted.id);
    assert!(kinds.lock().unwrap().contains(&EventKind::MessagePosted));
    assert!(tap.live_ready(hw));
    assert!(tap.fault.is_none());
}

#[tokio::test]
async fn broken_chain_fails_closed_and_does_not_project_later_messages() {
    let (store, pool) = sqlite().await;
    let posted = seed_with_message(&store).await;
    let events = store
        .list_events_after(posted.workspace_id.unwrap(), 0, 50)
        .await
        .unwrap();
    let early = events
        .iter()
        .find(|e| e.kind == EventKind::MemberJoined)
        .expect("member event");
    let mut payload = early.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(early.id)
        .execute(&pool)
        .await
        .unwrap();

    let mut tap = SearchTap::new();
    let kinds = Arc::new(Mutex::new(Vec::new()));
    let err = backfill_search(&store, &mut tap, {
        let kinds = kinds.clone();
        move |row| {
            let kinds = kinds.clone();
            async move {
                kinds.lock().unwrap().push(row.kind);
                Ok(())
            }
        }
    })
    .await
    .expect_err("tamper must fail closed");
    assert!(err.search_must_rebuild());
    assert!(!kinds.lock().unwrap().contains(&EventKind::MessagePosted));
}

#[tokio::test]
async fn indexer_with_log_backfills_before_live() {
    let (store, _pool) = sqlite().await;
    let posted = seed_with_message(&store).await;
    let store: Arc<dyn Store> = Arc::new(store);
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let handler = Arc::new(LoggingHandler::default());
    let indexer = Indexer::new(bus, handler.clone()).with_log(store).spawn();

    let observed = handler
        .wait_for(Duration::from_secs(2), |log| {
            log.contains(&EventKind::MessagePosted)
        })
        .await
        .expect("backfill should project the retained MessagePosted");
    assert!(observed.contains(&EventKind::MessagePosted));
    assert!(
        !indexer.rebuild_needed.load(Ordering::Relaxed),
        "intact log must not request rebuild"
    );
    let _ = posted;
    indexer.shutdown().await;
}
