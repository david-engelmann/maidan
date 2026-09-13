//! Explicit dispatch-block store (Cluster 386, Wave 2 #27): set/clear/get +
//! channel list over the closed `BlockedReason` enum. Both backends. Zero
//! blast: `claim_next` is not yet gated (386.2).

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{BlockedReason, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
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
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
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
    let t1 = mk("t1").await;
    let t2 = mk("t2").await;

    assert!(store.get_thread_block(t1.id).await.expect("get").is_none());
    assert!(store
        .list_blocked_threads(channel.id)
        .await
        .expect("list")
        .is_empty());

    let set = store
        .set_thread_block(t1.id, BlockedReason::Gate, member.id)
        .await
        .expect("set");
    assert_eq!(set.thread_id, t1.id);
    assert_eq!(set.reason, BlockedReason::Gate);
    assert_eq!(set.set_by, member.id);

    let got = store
        .get_thread_block(t1.id)
        .await
        .expect("get")
        .expect("blocked");
    assert_eq!(got.reason, BlockedReason::Gate);

    // Re-set upserts the reason (closed enum, not a free-text park).
    store
        .set_thread_block(t1.id, BlockedReason::Human, member.id)
        .await
        .expect("reset");
    assert_eq!(
        store
            .get_thread_block(t1.id)
            .await
            .expect("get")
            .expect("blocked")
            .reason,
        BlockedReason::Human
    );

    // Every closed reason is persistable.
    for &reason in BlockedReason::ALL {
        store
            .set_thread_block(t2.id, reason, member.id)
            .await
            .expect("set each reason");
        assert_eq!(
            store
                .get_thread_block(t2.id)
                .await
                .expect("get")
                .expect("blocked")
                .reason,
            reason
        );
    }

    let listed = store.list_blocked_threads(channel.id).await.expect("list");
    assert_eq!(listed.len(), 2);

    // Clear returns the row once, then None (idempotent).
    let cleared = store
        .clear_thread_block(t1.id)
        .await
        .expect("clear")
        .expect("had a block");
    assert_eq!(cleared.reason, BlockedReason::Human);
    assert!(store
        .clear_thread_block(t1.id)
        .await
        .expect("clear again")
        .is_none());
    assert!(store.get_thread_block(t1.id).await.expect("get").is_none());
    assert_eq!(
        store
            .list_blocked_threads(channel.id)
            .await
            .expect("list")
            .len(),
        1,
        "only t2 remains blocked"
    );
}

#[tokio::test]
async fn thread_blocks_set_clear_get_list_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn thread_blocks_set_clear_get_list_postgres() {
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
