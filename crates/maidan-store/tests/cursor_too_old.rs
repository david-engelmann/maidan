//! CursorTooOld fails loud when a subscribe cursor points into a pruned gap.
//! Both backends. No silent clamp to the oldest remaining row.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{Event, MemberKind, NewChannel, NewMember, NewWorkspace};
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

async fn seed(store: &dyn Store) -> (maidan_types::WorkspaceId, Vec<i64>) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "cursor-ws".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let e1 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("e1");
    let e2 = store
        .append_event(&Event::ChannelCreated {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel: ch,
        })
        .await
        .expect("e2");
    let e3 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member,
        })
        .await
        .expect("e3");
    (ws.id, vec![e1.id, e2.id, e3.id])
}

async fn run_suite(store: &dyn Store) {
    let (ws, ids) = seed(store).await;
    assert_eq!(ids.len(), 3);
    let oldest = store.min_event_id(ws).await.expect("min");
    assert_eq!(oldest, Some(ids[0]));

    store
        .ensure_cursor_fresh(ws, 0)
        .await
        .expect("fresh after_id=0 is never too old");
    store
        .ensure_cursor_fresh(ws, ids[0])
        .await
        .expect("cursor at the oldest retained row is adjacent/ok");

    // Prune the first two rows so the remaining log starts at ids[2].
    let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
    let deleted = store.prune_events(cutoff, ids[1], 10).await.expect("prune");
    assert!(deleted >= 2, "pruned the gap, got {deleted}");

    let oldest = store.min_event_id(ws).await.expect("min after prune");
    assert_eq!(oldest, Some(ids[2]));

    store
        .ensure_cursor_fresh(ws, 0)
        .await
        .expect("fresh subscriber still ok after prune");
    store
        .ensure_cursor_fresh(ws, ids[1])
        .await
        .expect("adjacent resume (after_id+1 == oldest) is not too old");

    let err = store
        .ensure_cursor_fresh(ws, ids[0])
        .await
        .expect_err("cursor inside the pruned gap must fail loud");
    match err {
        StoreError::CursorTooOld {
            after_id,
            oldest_id,
        } => {
            assert_eq!(after_id, ids[0]);
            assert_eq!(oldest_id, ids[2]);
        }
        other => panic!("expected CursorTooOld, got {other:?}"),
    }

    // Cross-workspace page still returns the surviving row.
    let global = store.list_events_after_global(0, 50).await.expect("global");
    assert!(
        global.iter().any(|e| e.id == ids[2]),
        "surviving event is in the global page"
    );
    assert!(
        !global.iter().any(|e| e.id == ids[0] || e.id == ids[1]),
        "pruned events must not reappear"
    );
}

#[tokio::test]
async fn cursor_too_old_fails_loud_on_pruned_gap_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn cursor_too_old_fails_loud_on_pruned_gap_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
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
            return;
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
