//! Thread wait-timer store (Cluster 364, G2/G4): set/get/cancel + the atomic
//! fire-once `claim_next_due_wait`. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{EscalationPolicy, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
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
            handle: "a".into(),
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
    let mk = |title: &str| {
        let cid = channel.id;
        let title = title.to_string();
        async move {
            store
                .create_thread(NewThread {
                    channel_id: cid,
                    parent_thread_id: None,
                    title: Some(title),
                })
                .await
                .expect("thread")
        }
    };
    let past = mk("past").await; // deadline already lapsed
    let future = mk("future").await; // deadline far off

    // No wait by default.
    assert!(store.get_thread_wait(past.id).await.expect("get").is_none());

    // Set a wait already due, and one far in the future.
    let due = Utc::now() - Duration::seconds(5);
    let set = store
        .set_thread_wait(
            past.id,
            due,
            EscalationPolicy::Park,
            Some("blocked"),
            member.id,
        )
        .await
        .expect("set");
    assert_eq!(set.on_timeout, EscalationPolicy::Park);
    assert_eq!(set.reason.as_deref(), Some("blocked"));
    assert!(set.fired_at.is_none());
    store
        .set_thread_wait(
            future.id,
            Utc::now() + Duration::hours(1),
            EscalationPolicy::Notify,
            None,
            member.id,
        )
        .await
        .expect("set future");

    // The sweeper claims the due one (fires it) and nothing else.
    let claimed = store
        .claim_next_due_wait(Utc::now())
        .await
        .expect("claim")
        .expect("one due");
    assert_eq!(claimed.thread_id, past.id, "only the due wait fires");
    assert!(claimed.fired_at.is_some(), "claim stamps fired_at");

    // A second claim finds nothing (the future one isn't due; the past one fired).
    assert!(store
        .claim_next_due_wait(Utc::now())
        .await
        .expect("claim2")
        .is_none());

    // The fired wait reads back with fired_at set; re-setting resets it.
    assert!(store
        .get_thread_wait(past.id)
        .await
        .expect("get")
        .expect("wait")
        .fired_at
        .is_some());
    let reset = store
        .set_thread_wait(
            past.id,
            Utc::now() + Duration::hours(1),
            EscalationPolicy::Notify,
            None,
            member.id,
        )
        .await
        .expect("reset");
    assert!(reset.fired_at.is_none(), "re-setting is a fresh timer");

    // Cancel: true once, false after.
    assert!(store.cancel_thread_wait(past.id).await.expect("cancel"));
    assert!(!store
        .cancel_thread_wait(past.id)
        .await
        .expect("cancel again"));
    assert!(store.get_thread_wait(past.id).await.expect("get").is_none());
}

#[tokio::test]
async fn thread_wait_set_get_cancel_and_claim_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn thread_wait_set_get_cancel_and_claim_postgres() {
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
