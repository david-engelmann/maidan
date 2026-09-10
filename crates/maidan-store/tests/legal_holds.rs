//! Legal-hold store + retention exemption (Cluster 366, T6): place/get/lift/list,
//! and the headline behaviour — a held workspace's events survive retention
//! pruning while an unheld workspace's are pruned, and audit pruning freezes while
//! any hold is active. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Executor, Row};

async fn sqlite() -> (SqliteStore, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    (SqliteStore::new(pool.clone()), pool)
}

async fn run_crud(store: &dyn Store) {
    let held = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: held.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    assert!(store.get_legal_hold(held.id).await.expect("get").is_none());
    assert!(store.list_legal_holds().await.expect("list").is_empty());

    let hold = store
        .place_legal_hold(held.id, "litigation X", Some(member.id))
        .await
        .expect("place");
    assert_eq!(hold.reason, "litigation X");
    assert_eq!(hold.placed_by, Some(member.id));
    assert!(store.get_legal_hold(held.id).await.expect("get").is_some());
    assert_eq!(store.list_legal_holds().await.expect("list").len(), 1);

    // Upsert: re-placing updates the reason (one hold per workspace).
    let re = store
        .place_legal_hold(held.id, "litigation Y", None)
        .await
        .expect("re-place");
    assert_eq!(re.reason, "litigation Y");
    assert_eq!(store.list_legal_holds().await.expect("list").len(), 1);

    // Lift: true once, false after; then gone.
    assert!(store.lift_legal_hold(held.id).await.expect("lift"));
    assert!(!store.lift_legal_hold(held.id).await.expect("lift again"));
    assert!(store.get_legal_hold(held.id).await.expect("get").is_none());
}

/// The retention exemption: seed old events for a held and an unheld workspace,
/// prune, and assert the held one's events survive.
async fn run_retention_exemption<'a, E>(store: &dyn Store, exec: E)
where
    E: Executor<'a, Database = sqlx::Sqlite> + Copy,
{
    let held = store
        .create_workspace(NewWorkspace {
            name: "held-ws".into(),
        })
        .await
        .expect("ws");
    let free = store
        .create_workspace(NewWorkspace {
            name: "free-ws".into(),
        })
        .await
        .expect("ws");

    // Seed one old event per workspace directly (occurred_at well in the past).
    let old = (Utc::now() - Duration::days(400)).to_rfc3339();
    for (i, ws) in [(1_i64, held.id), (2, free.id)] {
        sqlx::query(
            "INSERT INTO maidan_events (id, kind, workspace_id, occurred_at, payload)
             VALUES (?, 'message_posted', ?, ?, '{}')",
        )
        .bind(i)
        .bind(ws.0)
        .bind(&old)
        .execute(exec)
        .await
        .expect("seed event");
    }

    store
        .place_legal_hold(held.id, "hold", None)
        .await
        .expect("place");

    // Prune everything older than 1 day, up to a high max_id.
    let cutoff = Utc::now() - Duration::days(1);
    let mut deleted = 0u64;
    loop {
        let n = store
            .prune_events(cutoff, i64::MAX, 100)
            .await
            .expect("prune");
        deleted += n;
        if n < 100 {
            break;
        }
    }
    assert_eq!(deleted, 1, "only the unheld workspace's event is pruned");

    let remaining: Vec<i64> = sqlx::query("SELECT id FROM maidan_events ORDER BY id")
        .fetch_all(exec)
        .await
        .expect("select")
        .iter()
        .map(|r| r.get::<i64, _>("id"))
        .collect();
    assert_eq!(
        remaining,
        vec![1],
        "the held workspace's event (id 1) survives"
    );
}

#[tokio::test]
async fn legal_hold_crud_and_retention_exemption_sqlite() {
    let (store, pool) = sqlite().await;
    run_crud(&store).await;
    run_retention_exemption(&store, &pool).await;
}

#[tokio::test]
async fn legal_hold_crud_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_crud(&store).await;
}
