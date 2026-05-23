//! Migration 0004: thread transition log and `in_review` state.

use chrono::Utc;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadState};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

async fn seed_thread(store: &dyn Store) -> (maidan_types::ThreadId, maidan_types::MemberId) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "fsm-ws".to_string(),
        })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "actor".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "fsm-ch".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            title: Some("fsm-thread".to_string()),
        })
        .await
        .expect("thread");
    (thread.id, member.id)
}

#[tokio::test]
async fn sqlite_migration_0004_records_thread_transitions() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("enable foreign keys");
    run_sqlite_migrations(&pool)
        .await
        .expect("apply sqlite migrations");

    let store = SqliteStore::new(pool.clone());
    let (thread_id, actor_id) = seed_thread(&store).await;

    sqlx::query("UPDATE maidan_threads SET state = 'in_review', updated_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(thread_id.0)
        .execute(&pool)
        .await
        .expect("set in_review");

    let transition_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO maidan_thread_transitions
            (id, thread_id, from_state, to_state, actor_id, occurred_at)
         VALUES (?, ?, 'open', 'in_review', ?, ?)",
    )
    .bind(transition_id)
    .bind(thread_id.0)
    .bind(actor_id.0)
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect("insert transition");

    let thread = store.get_thread(thread_id).await.expect("get thread");
    assert_eq!(thread.state, ThreadState::InReview);

    let row: (String, String) =
        sqlx::query_as("SELECT from_state, to_state FROM maidan_thread_transitions WHERE id = ?")
            .bind(transition_id)
            .fetch_one(&pool)
            .await
            .expect("fetch transition");
    assert_eq!(row.0, "open");
    assert_eq!(row.1, "in_review");
}

#[tokio::test]
async fn postgres_migration_0004_records_thread_transitions() {
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
            eprintln!("skipping postgres_migration_0004: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect postgres");
    run_postgres_migrations(&pool)
        .await
        .expect("apply postgres migrations");

    let store = PostgresStore::new(pool.clone());
    let (thread_id, actor_id) = seed_thread(&store).await;

    sqlx::query("UPDATE maidan_threads SET state = 'in_review', updated_at = NOW() WHERE id = $1")
        .bind(thread_id.0)
        .execute(&pool)
        .await
        .expect("set in_review");

    let transition_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO maidan_thread_transitions
            (id, thread_id, from_state, to_state, actor_id)
         VALUES ($1, $2, 'open', 'in_review', $3)",
    )
    .bind(transition_id)
    .bind(thread_id.0)
    .bind(actor_id.0)
    .execute(&pool)
    .await
    .expect("insert transition");

    let thread = store.get_thread(thread_id).await.expect("get thread");
    assert_eq!(thread.state, ThreadState::InReview);

    let row: (String, String) =
        sqlx::query_as("SELECT from_state, to_state FROM maidan_thread_transitions WHERE id = $1")
            .bind(transition_id)
            .fetch_one(&pool)
            .await
            .expect("fetch transition");
    assert_eq!(row.0, "open");
    assert_eq!(row.1, "in_review");
}
