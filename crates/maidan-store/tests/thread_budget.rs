//! Per-thread budget envelope store (Cluster 358, T1/T5): set/get maxima,
//! accumulate reported usage (creating the row on first report), and the pure
//! `exceeded` check. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BudgetLimits, BudgetReason, ChannelId, MemberKind, NewChannel, NewDlqEntry, NewMember,
    NewThread, NewWorkspace, ThreadId, UsageDelta,
};
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
            name: "budget".into(),
        })
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .expect("thread");

    // No budget until one is set or usage is reported.
    assert!(store
        .get_thread_budget(thread.id)
        .await
        .expect("get0")
        .is_none());

    // Set maxima.
    let b = store
        .set_thread_budget(
            thread.id,
            BudgetLimits {
                max_tokens: Some(1000),
                max_usd_micros: Some(500_000),
                max_turns: Some(10),
                max_wall_secs: Some(3600),
            },
        )
        .await
        .expect("set budget");
    assert_eq!(b.max_tokens, Some(1000));
    assert_eq!(b.used_tokens, 0);

    // Report usage — accumulates, preserving maxima.
    let b = store
        .add_thread_usage(
            thread.id,
            UsageDelta {
                tokens: 400,
                usd_micros: 100_000,
                turns: 3,
            },
        )
        .await
        .expect("usage 1");
    assert_eq!(b.used_tokens, 400);
    assert_eq!(b.max_tokens, Some(1000), "usage report preserves maxima");
    assert_eq!(b.exceeded(None), None, "under budget");

    // A second report accumulates on top.
    let b = store
        .add_thread_usage(
            thread.id,
            UsageDelta {
                tokens: 700,
                usd_micros: 0,
                turns: 0,
            },
        )
        .await
        .expect("usage 2");
    assert_eq!(b.used_tokens, 1100);
    assert_eq!(
        b.exceeded(None),
        Some(BudgetReason::Tokens),
        "over the token budget"
    );

    // Re-setting maxima preserves accumulated usage.
    let b = store
        .set_thread_budget(
            thread.id,
            BudgetLimits {
                max_tokens: Some(5000),
                ..Default::default()
            },
        )
        .await
        .expect("raise budget");
    assert_eq!(b.used_tokens, 1100, "re-set preserves usage");
    assert_eq!(b.max_usd_micros, None, "omitted dims cleared to unbounded");
    assert_eq!(
        b.exceeded(None),
        None,
        "raised token budget no longer binds"
    );

    // Wall dimension: exceeded only when elapsed >= max_wall_secs.
    let b = store
        .set_thread_budget(
            thread.id,
            BudgetLimits {
                max_wall_secs: Some(60),
                ..Default::default()
            },
        )
        .await
        .expect("wall budget");
    assert_eq!(b.exceeded(Some(59)), None, "under wall budget");
    assert_eq!(
        b.exceeded(Some(60)),
        Some(BudgetReason::Wall),
        "at/over wall budget"
    );
    assert_eq!(b.exceeded(None), None, "no wall check when not working");

    // Usage on a thread with no prior budget row creates it (no maxima).
    let thread2 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task2".into()),
        })
        .await
        .expect("thread2");
    let b2 = store
        .add_thread_usage(
            thread2.id,
            UsageDelta {
                tokens: 5,
                usd_micros: 0,
                turns: 1,
            },
        )
        .await
        .expect("usage on unbudgeted thread");
    assert_eq!(b2.used_tokens, 5);
    assert_eq!(b2.max_tokens, None);
    assert_eq!(b2.exceeded(None), None, "no maxima → nothing binds");

    // A thread with no budget/usage is None.
    let missing = store
        .get_thread_budget(ThreadId(uuid::Uuid::new_v4()))
        .await
        .expect("get missing");
    assert!(missing.is_none());

    // Agent-work DLQ (Cluster 358.2): record a dead-lettered run + list it.
    assert!(store
        .list_channel_dlq(channel.id, 10)
        .await
        .expect("dlq empty")
        .is_empty());
    let e = store
        .record_dlq_entry(&NewDlqEntry {
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: member.id,
            reason: BudgetReason::Tokens.as_str().into(),
            used_tokens: 1100,
            used_usd_micros: 100_000,
            used_turns: 3,
        })
        .await
        .expect("record dlq");
    assert_eq!(e.reason, "tokens");
    assert_eq!(e.thread_id, thread.id);
    assert_eq!(e.used_tokens, 1100);
    let listed = store
        .list_channel_dlq(channel.id, 10)
        .await
        .expect("dlq list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, e.id);
    assert_eq!(listed[0].member_id, member.id);
    // A different channel's DLQ is isolated.
    assert!(store
        .list_channel_dlq(ChannelId(uuid::Uuid::new_v4()), 10)
        .await
        .expect("other channel dlq")
        .is_empty());
}

#[tokio::test]
async fn thread_budget_set_get_accumulate_and_exceed_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn thread_budget_set_get_accumulate_and_exceed_postgres() {
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
