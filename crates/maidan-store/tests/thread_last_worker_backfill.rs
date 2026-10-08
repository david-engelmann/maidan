//! Migration 0124's backfill: rows that existed before it are numbered in the
//! order they were first held, ties broken by member id, and whoever holds the
//! thread now is numbered last. Re-run on rows with no number, both backends.

use chrono::{DateTime, Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

const SQLITE_0124: &str = include_str!("../../../migrations/sqlite/0124_thread_last_worker.sql");
const POSTGRES_0124: &str =
    include_str!("../../../migrations/postgres/0124_thread_last_worker.sql");

/// The migration after its `ALTER TABLE`: the column already exists here.
fn backfill(migration: &str) -> &str {
    let alter = migration.find("ALTER TABLE").expect("an ALTER");
    let end = alter + migration[alter..].find(';').expect("its end");
    &migration[end + 1..]
}

struct Seeded {
    held: ThreadId,
    unheld: ThreadId,
    /// `held`'s workers: first, then two tied on time, smaller id first.
    early: Uuid,
    tied_low: Uuid,
    tied_high: Uuid,
    /// `unheld`'s workers in the order they first held it.
    unheld_order: [Uuid; 2],
}

async fn seed(store: &dyn Store) -> Seeded {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let mut ids = Vec::new();
    for handle in ["a", "b", "c", "d", "e"] {
        let m = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .expect(handle);
        ids.push(m.id.0);
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = |title: &'static str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
    };
    let held = store.create_thread(thread("held")).await.expect("held").id;
    let unheld = store
        .create_thread(thread("unheld"))
        .await
        .expect("unheld")
        .id;
    let (low, high) = if ids[1] < ids[2] {
        (ids[1], ids[2])
    } else {
        (ids[2], ids[1])
    };
    Seeded {
        held,
        unheld,
        early: ids[0],
        tied_low: low,
        tied_high: high,
        unheld_order: [ids[3], ids[4]],
    }
}

/// A worker row: thread, member, first held.
type Worker = (ThreadId, Uuid, DateTime<Utc>);

/// The workers with their first-held time, and the thread's assignee, as the
/// rows stood before 0124.
fn rows(s: &Seeded) -> (Vec<Worker>, (ThreadId, Uuid)) {
    let t0 = Utc::now() - Duration::hours(1);
    let t1 = t0 + Duration::minutes(1);
    (
        vec![
            (s.held, s.tied_high, t1),
            (s.held, s.early, t0),
            (s.held, s.tied_low, t1),
            (s.unheld, s.unheld_order[1], t1),
            (s.unheld, s.unheld_order[0], t0),
        ],
        // The first worker re-claimed the thread and holds it now.
        (s.held, s.early),
    )
}

fn assert_order(s: &Seeded, seqs: &[(Uuid, Uuid, i64)]) {
    let seq = |thread: ThreadId, member: Uuid| {
        seqs.iter()
            .find(|(t, m, _)| *t == thread.0 && *m == member)
            .map(|r| r.2)
            .expect("a numbered row")
    };
    assert_eq!(seq(s.held, s.early), 4, "the current holder is last");
    assert_eq!(seq(s.held, s.tied_low), 2, "a tie goes to the smaller id");
    assert_eq!(seq(s.held, s.tied_high), 3);
    assert_eq!(seq(s.unheld, s.unheld_order[0]), 1);
    assert_eq!(
        seq(s.unheld, s.unheld_order[1]),
        2,
        "no holder: the latest is last"
    );
}

#[tokio::test]
async fn the_0124_backfill_orders_workers_by_first_hold_and_puts_the_holder_last_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    let s = seed(&store).await;
    let (workers, (thread, holder)) = rows(&s);
    for (t, m, at) in workers {
        sqlx::query(
            "INSERT INTO maidan_thread_workers (thread_id, member_id, first_held_at) VALUES (?1, ?2, ?3)",
        )
        .bind(t.0)
        .bind(m)
        .bind(at.format("%Y-%m-%d %H:%M:%S").to_string())
        .execute(&pool)
        .await
        .expect("worker");
    }
    sqlx::query("UPDATE maidan_threads SET assignee_id = ?1 WHERE id = ?2")
        .bind(holder)
        .bind(thread.0)
        .execute(&pool)
        .await
        .expect("holder");

    sqlx::raw_sql(backfill(SQLITE_0124))
        .execute(&pool)
        .await
        .expect("re-run the 0124 backfill");
    let seqs: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT thread_id, member_id, last_held_seq FROM maidan_thread_workers")
            .fetch_all(&pool)
            .await
            .expect("read");
    assert_order(&s, &seqs);
}

#[tokio::test]
async fn the_0124_backfill_orders_workers_by_first_hold_and_puts_the_holder_last_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    let store = PostgresStore::for_tests(pool.clone());
    let s = seed(&store).await;
    let (workers, (thread, holder)) = rows(&s);
    for (t, m, at) in workers {
        sqlx::query(
            "INSERT INTO maidan_thread_workers (thread_id, member_id, first_held_at) VALUES ($1, $2, $3)",
        )
        .bind(t.0)
        .bind(m)
        .bind(at)
        .execute(&pool)
        .await
        .expect("worker");
    }
    sqlx::query("UPDATE maidan_threads SET assignee_id = $1 WHERE id = $2")
        .bind(holder)
        .bind(thread.0)
        .execute(&pool)
        .await
        .expect("holder");

    sqlx::raw_sql(backfill(POSTGRES_0124))
        .execute(&pool)
        .await
        .expect("re-run the 0124 backfill");
    let seqs: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT thread_id, member_id, last_held_seq FROM maidan_thread_workers")
            .fetch_all(&pool)
            .await
            .expect("read");
    assert_order(&s, &seqs);
}
