//! Equal timestamps list in id order on both backends.

use chrono::{DateTime, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{NewChannel, NewThread, NewWorkspace, RefSide};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

async fn assert_tied_refs(
    store: &dyn Store,
    set_created_at: impl AsyncFn(Uuid, Uuid, DateTime<Utc>),
) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "ties".into(),
        })
        .await
        .expect("ws");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
        })
        .await
        .expect("thread");
    let hi = Uuid::from_u128(0x200);
    let lo = Uuid::from_u128(0x100);
    let when = DateTime::from_timestamp(1_700_000_000, 0).expect("when");
    // Insert the higher id first so insertion order is the reverse of id order.
    set_created_at(hi, thread.id.0, when).await;
    set_created_at(lo, thread.id.0, when).await;
    let listed = store
        .list_references_from(RefSide::Thread, thread.id.0)
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, lo);
    assert_eq!(listed[1].id, hi);
}

#[tokio::test]
async fn tied_references_sort_by_id_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    assert_tied_refs(&store, async |id, src, when| {
        sqlx::query(
            "INSERT INTO maidan_references (id, src_kind, src_id, dst_kind, dst_id, relation, created_at)
             VALUES (?, 'thread', ?, 'thread', ?, 'relates_to', ?)",
        )
        .bind(id)
        .bind(src)
        .bind(Uuid::now_v7())
        .bind(when)
        .execute(&pool)
        .await
        .expect("insert");
    })
    .await;
}

#[tokio::test]
async fn tied_references_sort_by_id_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    let store = PostgresStore::for_tests(pool.clone());
    assert_tied_refs(&store, async |id, src, when| {
        sqlx::query(
            "INSERT INTO maidan_references (id, src_kind, src_id, dst_kind, dst_id, relation, created_at)
             VALUES ($1, 'thread', $2, 'thread', $3, 'relates_to', $4)",
        )
        .bind(id)
        .bind(src)
        .bind(Uuid::now_v7())
        .bind(when)
        .execute(&pool)
        .await
        .expect("insert");
    })
    .await;
}
