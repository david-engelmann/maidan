//! Thread owner axis (Cluster 355, W1): a durable owner distinct from the
//! assignee/claimer. `set_thread_owner` sets or clears it, orthogonal to the
//! claim state; a missing/tombstoned thread is `NotFound`.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId};
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

async fn run_owner_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "own".into() })
        .await
        .expect("ws");
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("human");
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("agent");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .expect("thread");

    // A fresh thread has no owner.
    assert_eq!(thread.owner_id, None);

    // Set the owner (a human); it persists and is orthogonal to assignment.
    let owned = store
        .set_thread_owner(thread.id, Some(human.id))
        .await
        .expect("set owner");
    assert_eq!(owned.owner_id, Some(human.id));
    assert_eq!(
        owned.assignee_id, None,
        "owner does not touch the claim axis"
    );

    // Assigning the claimer leaves the owner intact — the two axes are distinct.
    let assigned = store
        .assign_thread(thread.id, agent.id)
        .await
        .expect("assign");
    assert_eq!(assigned.assignee_id, Some(agent.id));
    assert_eq!(
        assigned.owner_id,
        Some(human.id),
        "assign preserves the owner"
    );

    // Re-read confirms persistence.
    let got = store.get_thread(thread.id).await.expect("get");
    assert_eq!(got.owner_id, Some(human.id));

    // Clearing the owner (None) works.
    let cleared = store
        .set_thread_owner(thread.id, None)
        .await
        .expect("clear");
    assert_eq!(cleared.owner_id, None);

    // An unknown thread is NotFound.
    let missing = store
        .set_thread_owner(ThreadId(uuid::Uuid::new_v4()), Some(human.id))
        .await;
    assert!(matches!(missing, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn thread_owner_is_set_cleared_and_orthogonal_to_assignment_sqlite() {
    let store = sqlite().await;
    run_owner_suite(&store).await;
}

#[tokio::test]
async fn thread_owner_is_set_cleared_and_orthogonal_to_assignment_postgres() {
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
    run_owner_suite(&store).await;
}
