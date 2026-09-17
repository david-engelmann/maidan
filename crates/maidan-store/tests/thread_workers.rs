//! Cluster 401.1: the durable record of who held a thread. Both backends.
//!
//! The defect this exists for: both governance gates test the thread's **live**
//! `assignee_id`, and releasing a claim sets that to NULL — so an agent could do
//! the work, release, and then approve it as a qualifying third party. The
//! ledger answers "ever held", and the assertion that matters is the one after
//! the release.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "workers".into(),
        })
        .await
        .expect("ws");
    let member = |handle: &'static str| async move {
        store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .expect("member")
    };
    let alice = member("w-alice").await;
    let bob = member("w-bob").await;
    let carol = member("w-carol").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "w-c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("work".into()),
        })
        .await
        .expect("thread");

    // Nobody has held it yet.
    assert!(!store
        .has_worked_thread(thread.id, alice.id)
        .await
        .expect("empty"));
    assert!(store
        .list_thread_workers(thread.id)
        .await
        .expect("empty list")
        .is_empty());

    // A claim records the holder.
    let claimed = store
        .claim_thread(thread.id, alice.id)
        .await
        .expect("claim");
    assert!(claimed.claimed);
    assert!(store
        .has_worked_thread(thread.id, alice.id)
        .await
        .expect("after claim"));

    // **The assertion this table exists for.** Releasing clears `assignee_id`,
    // which is exactly what made the gate's exclusion vacuous. The ledger must
    // not forget.
    store.unassign_thread(thread.id).await.expect("release");
    let live = store.get_thread(thread.id).await.expect("thread");
    assert!(
        live.assignee_id.is_none(),
        "the release really did clear the live column"
    );
    assert!(
        store
            .has_worked_thread(thread.id, alice.id)
            .await
            .expect("after release"),
        "releasing a claim must not erase who did the work"
    );

    // A second holder accumulates rather than replacing — one member cannot
    // overwrite the record by handing the thread on.
    store
        .assign_thread(thread.id, bob.id)
        .await
        .expect("assign bob");
    let workers = store.list_thread_workers(thread.id).await.expect("both");
    assert_eq!(workers.len(), 2, "the ledger accumulates: {workers:?}");
    assert!(workers.contains(&alice.id) && workers.contains(&bob.id));

    // Re-holding is idempotent: "ever" is the question, not "how often".
    store
        .assign_thread(thread.id, alice.id)
        .await
        .expect("reassign alice");
    assert_eq!(
        store
            .list_thread_workers(thread.id)
            .await
            .expect("still 2")
            .len(),
        2,
        "re-holding must not duplicate"
    );

    // Someone who never held it stays clean — the gate must not exclude an
    // eligible reviewer.
    assert!(!store
        .has_worked_thread(thread.id, carol.id)
        .await
        .expect("carol"));

    // Scoped to the thread: holding one thread says nothing about another.
    let other = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("other".into()),
        })
        .await
        .expect("other thread");
    assert!(!store
        .has_worked_thread(other.id, alice.id)
        .await
        .expect("other thread"));
}

#[tokio::test]
async fn a_release_does_not_erase_who_did_the_work_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_release_does_not_erase_who_did_the_work_postgres() {
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
    run_suite(&PostgresStore::new(pool)).await;
}
