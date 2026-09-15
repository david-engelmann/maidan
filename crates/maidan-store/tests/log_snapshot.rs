//! Cluster 393: snapshot + since-LSN catch-up. Both backends.

use maidan_store::{
    build_log_snapshot, catch_up_since, prelude::*, run_sqlite_migrations, StoreError,
};
use maidan_types::{
    verify_snapshot, ChainBreakReason, Event, MemberKind, NewChannel, NewMember, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn seed(store: &dyn Store) -> maidan_types::WorkspaceId {
    let (ws, _) = store
        .create_workspace_with_event(NewWorkspace {
            name: "snap-ws".into(),
        })
        .await
        .expect("ws");
    let _ = store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let _ = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    ws.id
}

async fn extra_event(
    store: &dyn Store,
    ws: maidan_types::WorkspaceId,
) -> maidan_types::StoredEvent {
    let member = store
        .list_members(ws)
        .await
        .expect("members")
        .into_iter()
        .next()
        .expect("one");
    store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws,
            member,
        })
        .await
        .expect("append")
}

async fn run_suite(store: &dyn Store) {
    let unknown =
        build_log_snapshot(store, maidan_types::WorkspaceId(uuid::Uuid::nil()), true).await;
    assert!(matches!(unknown, Err(StoreError::NotFound)));

    let ws = seed(store).await;
    let snap = build_log_snapshot(store, ws, true).await.expect("snapshot");
    assert_eq!(snap.workspace_id, ws);
    assert!(snap.from_genesis);
    assert!(snap.as_of_lsn > 0);
    assert_eq!(snap.as_of_lsn, snap.head.as_ref().unwrap().lsn);
    assert_eq!(snap.floor_lsn, snap.floor.as_ref().unwrap().id);
    assert!(snap.graph.is_some());
    assert!(verify_snapshot(&snap).ok);

    let header = build_log_snapshot(store, ws, false).await.expect("header");
    assert!(header.graph.is_none());
    assert_eq!(header.graph_hash, snap.graph_hash);
    assert!(verify_snapshot(&header).ok);

    let page = catch_up_since(store, ws, 0, 50).await.expect("from 0");
    assert!(page.ok(), "{:?}", page.chain);
    assert!(page.chain.from_genesis);
    assert!(!page.events.is_empty());
    assert_eq!(page.head_lsn, snap.as_of_lsn);
    assert!(page.room_lsn >= page.head_lsn);

    let at_head = catch_up_since(store, ws, snap.as_of_lsn, 50)
        .await
        .expect("at head");
    assert!(at_head.ok(), "{:?}", at_head.chain);
    assert!(at_head.events.is_empty());
    assert!(!at_head.truncated);

    let extra = extra_event(store, ws).await;
    let caught = catch_up_since(store, ws, snap.as_of_lsn, 50)
        .await
        .expect("catch-up");
    assert!(caught.ok(), "{:?}", caught.chain);
    assert_eq!(caught.events.len(), 1);
    assert_eq!(caught.events[0].id, extra.id);
    assert!(!caught.chain.from_genesis);
}

async fn assert_broken_catch_up(store: &dyn Store, ws: maidan_types::WorkspaceId, id: i64) {
    let broken = catch_up_since(store, ws, 0, 50).await.expect("broken");
    assert!(!broken.ok());
    assert_eq!(broken.chain.break_at, Some(id));
    assert_eq!(
        broken.chain.reason,
        Some(ChainBreakReason::ContentHashMismatch)
    );
}

async fn assert_pruned_prefix(
    store: &dyn Store,
    ws: maidan_types::WorkspaceId,
    first: i64,
    floor: i64,
) {
    let snap = build_log_snapshot(store, ws, true)
        .await
        .expect("after prune");
    assert!(!snap.from_genesis);
    assert_eq!(snap.floor_lsn, floor);
    assert!(verify_snapshot(&snap).ok);

    let err = catch_up_since(store, ws, first, 50).await;
    match err {
        Err(StoreError::CursorTooOld {
            after_id,
            oldest_id,
        }) => {
            assert_eq!(after_id, first);
            assert_eq!(oldest_id, floor);
        }
        other => panic!("expected CursorTooOld, got {other:?}"),
    }

    let from_floor = catch_up_since(store, ws, floor - 1, 50)
        .await
        .expect("adjacent");
    assert!(from_floor.ok(), "{:?}", from_floor.chain);
    assert!(!from_floor.events.is_empty());
}

#[tokio::test]
async fn snapshot_then_catch_up_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn catch_up_tamper_fails_closed_sqlite() {
    let store = sqlite().await;
    let ws = seed(&store).await;
    let stored = extra_event(&store, ws).await;
    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper");
    assert_broken_catch_up(&store, ws, stored.id).await;
}

#[tokio::test]
async fn pruned_prefix_requires_snapshot_sqlite() {
    let store = sqlite().await;
    let ws = seed(&store).await;
    let events = store.list_events_after(ws, 0, 50).await.expect("list");
    assert!(events.len() >= 3);
    sqlx::query("DELETE FROM maidan_events WHERE id = ?")
        .bind(events[0].id)
        .execute(store.pool())
        .await
        .expect("delete 0");
    sqlx::query("DELETE FROM maidan_events WHERE id = ?")
        .bind(events[1].id)
        .execute(store.pool())
        .await
        .expect("delete 1");
    assert_pruned_prefix(&store, ws, events[0].id, events[2].id).await;
}

async fn postgres() -> Option<PostgresStore> {
    use maidan_store::run_postgres_migrations;
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some(PostgresStore::new(pool))
}

#[tokio::test]
async fn snapshot_then_catch_up_postgres() {
    let Some(store) = postgres().await else {
        return;
    };
    run_suite(&store).await;
}

#[tokio::test]
async fn catch_up_tamper_fails_closed_postgres() {
    let Some(store) = postgres().await else {
        return;
    };
    let ws = seed(&store).await;
    let stored = extra_event(&store, ws).await;
    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = $1 WHERE id = $2")
        .bind(&payload)
        .bind(stored.id)
        .execute(store.pool())
        .await
        .expect("tamper");
    assert_broken_catch_up(&store, ws, stored.id).await;
}

#[tokio::test]
async fn pruned_prefix_requires_snapshot_postgres() {
    let Some(store) = postgres().await else {
        return;
    };
    let ws = seed(&store).await;
    let events = store.list_events_after(ws, 0, 50).await.expect("list");
    assert!(events.len() >= 3);
    sqlx::query("DELETE FROM maidan_events WHERE id = $1")
        .bind(events[0].id)
        .execute(store.pool())
        .await
        .expect("delete 0");
    sqlx::query("DELETE FROM maidan_events WHERE id = $1")
        .bind(events[1].id)
        .execute(store.pool())
        .await
        .expect("delete 1");
    assert_pruned_prefix(&store, ws, events[0].id, events[2].id).await;
}
