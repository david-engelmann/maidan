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
    // Cluster 402.1: the backfill completes rather than aborting the whole tap,
    // but the tampered workspace is faulted and **nothing from it is
    // projected**. The safety property is unchanged — a diverged chain is never
    // served — what changed is that one tenant's break no longer stops indexing
    // for every other tenant.
    backfill_search(&store, &mut tap, {
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
    .expect("a per-workspace break no longer aborts the tap");
    assert!(
        tap.has_workspace_fault(),
        "the tamper must still be caught, not ignored"
    );
    let (_, fault) = tap.faulted_workspaces().into_iter().next().unwrap();
    assert!(fault.search_must_rebuild());
    assert!(
        !kinds.lock().unwrap().contains(&EventKind::MessagePosted),
        "a diverged chain must never be projected"
    );
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

/// Cluster 402.2: a resumed backfill projects only what is new, and still
/// verifies it.
///
/// `backfill_search` walked from id 0 on every start, resubscribe and `Lagged`,
/// re-projecting all history each time — on Postgres that means re-embedding it.
#[tokio::test]
async fn a_resumed_backfill_projects_only_new_events() {
    let (store, _pool) = sqlite().await;
    seed_with_message(&store).await;

    // First pass, from genesis.
    let mut tap = SearchTap::new();
    let first = Arc::new(Mutex::new(Vec::new()));
    let hw = backfill_search(&store, &mut tap, {
        let seen = first.clone();
        move |row| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(row.id);
                Ok(())
            }
        }
    })
    .await
    .expect("first pass");
    let projected_first = first.lock().unwrap().len();
    assert!(projected_first > 0, "the first pass projects history");

    // A second pass resumed at the high-water projects nothing — there is
    // nothing new. This is the whole point: a restart is not a reindex.
    let mut resumed = SearchTap::new();
    resumed.resume_at(hw);
    let second = Arc::new(Mutex::new(Vec::new()));
    backfill_search(&store, &mut resumed, {
        let seen = second.clone();
        move |row| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(row.id);
                Ok(())
            }
        }
    })
    .await
    .expect("resumed pass");
    assert!(
        second.lock().unwrap().is_empty(),
        "a resume must not re-project history it already indexed"
    );

    // New work after the cursor is picked up, and only that.
    let posted = seed_with_message(&store).await;
    let mut tail = SearchTap::new();
    tail.resume_at(hw);
    let third = Arc::new(Mutex::new(Vec::new()));
    backfill_search(&store, &mut tail, {
        let seen = third.clone();
        move |row| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(row.id);
                Ok(())
            }
        }
    })
    .await
    .expect("tail pass");
    let ids = third.lock().unwrap().clone();
    assert!(
        ids.contains(&posted.id),
        "the new message must be projected: {ids:?}"
    );
    assert!(
        ids.iter().all(|id| *id > hw),
        "a resume must project nothing at or below its cursor: {ids:?}"
    );
}

/// A resume still checks that the first row after the cursor **chains from its
/// real predecessor** — the property the seeded link exists for.
///
/// `verify_link` only compares `prev_hash` when it has a previous link. With
/// none, `from_genesis` is false for a mid-chain row, so the `prev_hash` branch
/// is skipped entirely: an unseeded resume would accept a row pointing at a
/// predecessor that was deleted or reordered. A payload tamper is caught either
/// way by `content_hash`, which is why this test corrupts the **link**, not the
/// payload — the first version of it corrupted the payload, passed with the
/// seeding removed, and so proved nothing.
#[tokio::test]
async fn a_resumed_backfill_verifies_the_link_to_its_predecessor() {
    let (store, pool) = sqlite().await;
    let first = seed_with_message(&store).await;
    let ws = first.workspace_id.expect("workspace-scoped");

    let mut tap = SearchTap::new();
    let hw = backfill_search(&store, &mut tap, |_row| async { Ok(()) })
        .await
        .expect("first pass");

    // A second message in the *same* workspace, so it genuinely continues that
    // workspace's chain rather than starting a new one.
    let (_, second) = store
        .post_message_with_event(
            NewMessage {
                thread_id: first.thread_id.expect("thread"),
                author_id: maidan_types::MemberId(store.list_members(ws).await.unwrap()[0].id.0),
                body: "second".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    assert!(second.id > hw, "the new event is past the cursor");

    // Break the *link*: point it at a predecessor it does not have. The payload
    // and its content_hash stay consistent, so only a prev_hash check can see
    // this.
    sqlx::query("UPDATE maidan_events SET prev_hash = ? WHERE id = ?")
        .bind(maidan_types::genesis_hash())
        .bind(second.id)
        .execute(&pool)
        .await
        .unwrap();

    let mut resumed = SearchTap::new();
    resumed.resume_at(hw);
    let projected = Arc::new(Mutex::new(Vec::new()));
    backfill_search(&store, &mut resumed, {
        let seen = projected.clone();
        move |row| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(row.id);
                Ok(())
            }
        }
    })
    .await
    .expect("resume completes; the break is per-workspace");
    assert!(
        resumed.has_workspace_fault(),
        "a resumed walk must verify the first row against its real predecessor"
    );
    assert!(
        !projected.lock().unwrap().contains(&second.id),
        "and must not project a row whose chain link is wrong"
    );
}

/// A payload tamper after the cursor is caught too, by `content_hash`.
#[tokio::test]
async fn a_resumed_backfill_still_catches_a_tamper_after_the_cursor() {
    let (store, pool) = sqlite().await;
    let first_msg = seed_with_message(&store).await;

    let mut tap = SearchTap::new();
    let hw = backfill_search(&store, &mut tap, |_row| async { Ok(()) })
        .await
        .expect("first pass");
    assert!(hw >= first_msg.id);

    // New event after the cursor, then tamper with it.
    let posted = seed_with_message(&store).await;
    let mut payload = posted.payload.clone();
    payload["tampered"] = serde_json::json!(true);
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(posted.id)
        .execute(&pool)
        .await
        .unwrap();

    let mut resumed = SearchTap::new();
    resumed.resume_at(hw);
    let projected = Arc::new(Mutex::new(Vec::new()));
    backfill_search(&store, &mut resumed, {
        let seen = projected.clone();
        move |row| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(row.id);
                Ok(())
            }
        }
    })
    .await
    .expect("resume completes; the break is per-workspace");
    assert!(
        resumed.has_workspace_fault(),
        "a tamper after the cursor must still be caught on a resumed walk"
    );
    assert!(
        !projected.lock().unwrap().contains(&posted.id),
        "and the tampered row must not be projected"
    );
}
