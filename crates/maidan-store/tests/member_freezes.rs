//! Member-freeze kill-switch store (Cluster 372, Wave 2 #20): freeze drops the
//! member's active leases + records the freeze; unfreeze clears it. Both backends.
//! `claim_next` refusal is exercised in Cluster 372.2.

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
    let op = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("op");
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
            title: Some("t".into()),
        })
        .await
        .expect("thread");

    // The member claims a thread.
    store
        .assign_thread(thread.id, member.id)
        .await
        .expect("assign");
    assert_eq!(
        store.get_thread(thread.id).await.unwrap().assignee_id,
        Some(member.id)
    );

    // Not frozen yet.
    assert!(!store.is_member_frozen(member.id).await.expect("is_frozen"));

    // Freeze: records the freeze AND drops the lease (releases the claimed thread).
    let (freeze, released) = store
        .freeze_member(member.id, op.id, Some("compromised"))
        .await
        .expect("freeze");
    assert_eq!(freeze.member_id, member.id);
    assert_eq!(freeze.frozen_by, op.id);
    assert_eq!(freeze.reason.as_deref(), Some("compromised"));
    assert_eq!(released, 1, "the one active claim was released");
    assert!(store.is_member_frozen(member.id).await.expect("is_frozen"));
    assert_eq!(
        store.get_thread(thread.id).await.unwrap().assignee_id,
        None,
        "the thread is back in the queue"
    );

    // The freeze is listed for the workspace; get returns it.
    let frozen = store.list_frozen_members(ws.id).await.expect("list");
    assert_eq!(frozen.len(), 1);
    assert_eq!(frozen[0].member_id, member.id);
    assert!(store.get_member_freeze(member.id).await.unwrap().is_some());

    // Re-freezing is idempotent (refreshes the record) and releases nothing new.
    let (_, released2) = store
        .freeze_member(member.id, op.id, None)
        .await
        .expect("refreeze");
    assert_eq!(released2, 0);

    // Unfreeze clears it; a second unfreeze is a no-op.
    assert!(store.unfreeze_member(member.id).await.expect("unfreeze"));
    assert!(!store.unfreeze_member(member.id).await.expect("unfreeze2"));
    assert!(!store.is_member_frozen(member.id).await.expect("is_frozen"));
    assert!(store
        .list_frozen_members(ws.id)
        .await
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn member_freeze_drops_leases_and_toggles_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn member_freeze_drops_leases_and_toggles_postgres() {
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
