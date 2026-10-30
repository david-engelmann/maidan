//! `seat_costs` (#1771): per-seat cost summed from the usage ledger by the
//! member that reported it, in a window, never another workspace's. Both
//! backends.

use chrono::{Duration as ChronoDuration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewChannel, NewMember, NewThread, NewUsageLedgerEntry, NewWorkspace,
    PriceSnapshot, SeatCostQuery, TokenUsage, UsageEvidence, WorkspaceId,
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
    SqliteStore::for_tests(pool)
}

/// One micro-USD per uncached input token.
fn price() -> PriceSnapshot {
    PriceSnapshot {
        input_usd_micros_per_million: 1_000_000,
        ..Default::default()
    }
}

async fn seat(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
        .id
}

/// `member` claims a fresh thread and reports one call per entry of `inputs`.
async fn work(store: &dyn Store, ws: WorkspaceId, member: MemberId, inputs: &[i64]) {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("work-{}", uuid::Uuid::now_v7().simple()),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some(format!("seat task {}", channel.name)),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    let claimed = store.claim_thread(thread.id, member).await.expect("claim");
    let lease = claimed.thread.claim_lease_id.expect("claim lease");
    for &input in inputs {
        store
            .report_accounted_usage(&NewUsageLedgerEntry {
                usage_report_id: uuid::Uuid::now_v7(),
                thread_id: thread.id,
                reporter: member,
                claim_lease_id: lease,
                model: "provider/model".into(),
                tokens: TokenUsage {
                    input,
                    ..Default::default()
                },
                usd_micros: input,
                price_snapshot: price(),
                turns: 1,
                evidence: UsageEvidence::default(),
            })
            .await
            .expect("report");
    }
}

fn query(ws: WorkspaceId) -> SeatCostQuery {
    SeatCostQuery {
        workspace_id: ws,
        since: None,
        until: None,
        limit: 100,
    }
}

async fn run_suite(store: &dyn Store) {
    let a = store
        .create_workspace(NewWorkspace {
            name: "seats-a".into(),
        })
        .await
        .expect("ws a")
        .id;
    let b = store
        .create_workspace(NewWorkspace {
            name: "seats-b".into(),
        })
        .await
        .expect("ws b")
        .id;
    let planner = seat(store, a, "planner").await;
    let coder = seat(store, a, "coder").await;
    let idle = seat(store, a, "idle").await;
    let other = seat(store, b, "other").await;
    // The coder works two threads, so its spend and thread count both add up.
    work(store, a, coder, &[300, 200]).await;
    work(store, a, coder, &[100]).await;
    work(store, a, planner, &[40]).await;
    work(store, b, other, &[9_999]).await;

    let costs = store.seat_costs(query(a)).await.expect("seat costs");
    let rows: Vec<_> = costs
        .seats
        .iter()
        .map(|s| {
            (
                s.member_id,
                s.handle.as_str(),
                s.reports,
                s.threads,
                s.usd_micros,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![(coder, "coder", 3, 2, 600), (planner, "planner", 1, 1, 40)],
        "most expensive seat first; a seat with no reports and another workspace's seat are absent"
    );
    assert!(costs.seats.iter().all(|s| s.member_id != idle));
    assert_eq!(costs.usd_micros, 640);
    assert!(!costs.truncated);
    assert!(costs
        .seats
        .iter()
        .all(|s| s.first_accepted_at <= s.last_accepted_at));

    let top = store
        .seat_costs(SeatCostQuery {
            limit: 1,
            ..query(a)
        })
        .await
        .expect("top seat");
    assert_eq!(top.seats.len(), 1);
    assert_eq!(top.seats[0].member_id, coder);
    assert_eq!(top.usd_micros, 600);
    assert!(top.truncated, "a second seat spent in the window");

    let now = Utc::now();
    let all_time = store
        .seat_costs(SeatCostQuery {
            since: Some(now - ChronoDuration::days(1)),
            until: Some(now + ChronoDuration::days(1)),
            ..query(a)
        })
        .await
        .expect("window around now");
    assert_eq!(all_time.usd_micros, 640);
    let future = store
        .seat_costs(SeatCostQuery {
            since: Some(now + ChronoDuration::days(1)),
            ..query(a)
        })
        .await
        .expect("future window");
    assert!(future.seats.is_empty());
    assert_eq!(future.usd_micros, 0);
    let past = store
        .seat_costs(SeatCostQuery {
            until: Some(now - ChronoDuration::days(1)),
            ..query(a)
        })
        .await
        .expect("past window");
    assert!(past.seats.is_empty());

    let bad = store
        .seat_costs(SeatCostQuery {
            limit: 0,
            ..query(a)
        })
        .await;
    assert!(matches!(bad, Err(StoreError::InvalidInput(_))), "{bad:?}");
}

#[tokio::test]
async fn seat_costs_sum_each_reporting_member_in_a_window_on_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn seat_costs_sum_each_reporting_member_in_a_window_on_postgres() {
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
