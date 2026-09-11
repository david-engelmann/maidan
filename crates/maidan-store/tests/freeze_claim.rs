//! Freeze enforcement in `claim_next` (Cluster 372.2, Wave 2 #20): a frozen
//! member is refused a claim (the freeze `NOT EXISTS` clause), and unfreezing
//! restores it. Both backends, both claim variants (base + `_with_event`).

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
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    // Two claimable threads (one per claim variant below).
    for _ in 0..2 {
        store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: None,
            })
            .await
            .expect("thread");
    }

    // Freeze the member (no claims to drop yet).
    store
        .freeze_member(member.id, member.id, Some("kill-switch"))
        .await
        .expect("freeze");

    // Both claim variants refuse a frozen claimer.
    assert!(store
        .claim_next_thread(channel.id, member.id, None)
        .await
        .expect("claim")
        .is_none());
    let (claimed, _) = store
        .claim_next_thread_with_event(channel.id, member.id, None)
        .await
        .expect("claim_with_event");
    assert!(
        claimed.is_none(),
        "frozen member is refused by claim_next_with_event"
    );

    // Unfreeze → claim_next works again.
    assert!(store.unfreeze_member(member.id).await.expect("unfreeze"));
    let ok = store
        .claim_next_thread(channel.id, member.id, None)
        .await
        .expect("claim after unfreeze");
    assert!(ok.is_some(), "an unfrozen member claims normally");
    assert_eq!(ok.unwrap().assignee_id, Some(member.id));
}

#[tokio::test]
async fn frozen_member_cannot_claim_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn frozen_member_cannot_claim_postgres() {
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
    run_suite(&store).await;
}
