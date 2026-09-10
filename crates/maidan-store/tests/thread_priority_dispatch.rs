//! Fair-dispatch ordering (Cluster 365.2, G3): `claim_next` orders by an aged
//! effective rank = base priority + one boost per hour waited. Proves (1) higher
//! priority jumps the FIFO queue, (2) equal priority keeps the FIFO (oldest-first)
//! tiebreak, and (3) aging lets a long-waiting low-priority task overtake a newer
//! higher-priority one — the anti-starvation property. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
};
use sqlx::sqlite::SqlitePoolOptions;

/// The aging window (must match the `/ 3600` divisor inlined in the claim_next
/// ORDER BY in both backends): one effective-rank boost per hour a thread waits.
const AGING_WINDOW_SECS: i64 = 3600;

struct Fixture {
    channel: ChannelId,
    member: MemberId,
}

async fn seed(store: &dyn Store) -> Fixture {
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
    Fixture {
        channel: channel.id,
        member: member.id,
    }
}

async fn mk(store: &dyn Store, channel: ChannelId, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
        })
        .await
        .expect("thread")
        .id
}

async fn claim(store: &dyn Store, fx: &Fixture) -> Option<ThreadId> {
    store
        .claim_next_thread(fx.channel, fx.member, None)
        .await
        .expect("claim")
        .map(|t| t.id)
}

/// Priority jumps the FIFO queue; equal priority keeps oldest-first. No backdating
/// needed — everything is created within a second, so the age boost is 0 for all.
async fn run_ordering(store: &dyn Store) {
    let fx = seed(store).await;
    // Created oldest→newest; without priority t1 would dispatch first.
    let t1 = mk(store, fx.channel, "t1").await;
    let t2 = mk(store, fx.channel, "t2").await;
    let t3 = mk(store, fx.channel, "t3").await;
    store
        .set_thread_priority(t3, 10, fx.member)
        .await
        .expect("p3");
    store
        .set_thread_priority(t2, 5, fx.member)
        .await
        .expect("p2");
    // Higher priority first, despite t1 being oldest: t3 (10) → t2 (5) → t1 (0).
    assert_eq!(claim(store, &fx).await, Some(t3), "highest priority first");
    assert_eq!(claim(store, &fx).await, Some(t2), "then next priority");
    assert_eq!(
        claim(store, &fx).await,
        Some(t1),
        "then the default-priority one"
    );
    assert_eq!(claim(store, &fx).await, None, "queue drained");

    // Equal priority (all default 0) keeps the FIFO oldest-first tiebreak.
    let fx2 = seed(store).await;
    let a = mk(store, fx2.channel, "a").await;
    let b = mk(store, fx2.channel, "b").await;
    assert_eq!(
        claim(store, &fx2).await,
        Some(a),
        "older wins at equal rank"
    );
    assert_eq!(claim(store, &fx2).await, Some(b));
}

/// Aging: a long-waiting low-priority task overtakes a newer higher-priority one.
/// `backdate` sets a thread's `created_at` (per-backend raw SQL) so the age boost
/// is deterministic without waiting a real hour.
async fn run_aging<F, Fut>(store: &dyn Store, backdate: F)
where
    F: Fn(ThreadId, chrono::DateTime<Utc>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let fx = seed(store).await;
    let old_low = mk(store, fx.channel, "old_low").await; // priority 0, waited 3h
    let new_high = mk(store, fx.channel, "new_high").await; // priority 2, just now
    store
        .set_thread_priority(new_high, 2, fx.member)
        .await
        .expect("p");
    // old_low waited 3 windows → effective rank 0 + 3 = 3 > new_high's 2 + 0.
    backdate(
        old_low,
        Utc::now() - Duration::seconds(3 * AGING_WINDOW_SECS),
    )
    .await;
    assert_eq!(
        claim(store, &fx).await,
        Some(old_low),
        "aging overtakes the newer higher-priority task (no starvation)"
    );
    assert_eq!(claim(store, &fx).await, Some(new_high));
}

#[tokio::test]
async fn fair_dispatch_orders_by_aged_priority_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool.clone());
    run_ordering(&store).await;
    run_aging(&store, |tid, when| {
        let pool = pool.clone();
        async move {
            sqlx::query("UPDATE maidan_threads SET created_at = ? WHERE id = ?")
                .bind(when.to_rfc3339())
                .bind(tid.0)
                .execute(&pool)
                .await
                .expect("backdate");
        }
    })
    .await;
}

#[tokio::test]
async fn fair_dispatch_orders_by_aged_priority_postgres() {
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
    let store = PostgresStore::new(pool.clone());
    run_ordering(&store).await;
    run_aging(&store, |tid, when| {
        let pool = pool.clone();
        async move {
            sqlx::query("UPDATE maidan_threads SET created_at = $1 WHERE id = $2")
                .bind(when)
                .bind(tid.0)
                .execute(&pool)
                .await
                .expect("backdate");
        }
    })
    .await;
}
