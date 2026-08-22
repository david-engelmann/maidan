//! Read-replica LSN helpers validated against REAL streaming replication
//! (Cluster 261, Program D).
//!
//! `#[ignore]`d — it needs a live primary + hot-standby pair, which
//! `scripts/replica-harness.sh` stands up (Docker; the pgvector image with
//! streaming replication). Run it:
//!
//!   eval "$(scripts/replica-harness.sh up)"
//!   cargo test -p maidan-store --test replication -- --ignored --nocapture
//!   scripts/replica-harness.sh down
//!
//! The test connects via MAIDAN_PRIMARY_URL / MAIDAN_REPLICA_URL and skips (does
//! not fail) when they are unset — so a normal `cargo test` run is unaffected.

use std::time::Duration;

use maidan_store::postgres::replication::{current_wal_lsn, replica_caught_up, replica_replay_lsn};
use maidan_store::run_postgres_migrations;
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
#[ignore = "needs a live primary+replica pair (scripts/replica-harness.sh up); run with --ignored"]
async fn replica_replays_up_to_the_primary_write_lsn() {
    let (Ok(primary_url), Ok(replica_url)) = (
        std::env::var("MAIDAN_PRIMARY_URL"),
        std::env::var("MAIDAN_REPLICA_URL"),
    ) else {
        eprintln!(
            "skipping: set MAIDAN_PRIMARY_URL + MAIDAN_REPLICA_URL (scripts/replica-harness.sh up)"
        );
        return;
    };

    let primary = PgPoolOptions::new()
        .max_connections(4)
        .connect(&primary_url)
        .await
        .expect("connect primary");
    let replica = PgPoolOptions::new()
        .max_connections(4)
        .connect(&replica_url)
        .await
        .expect("connect replica");

    // Migrations run on the PRIMARY and replicate to the standby.
    run_postgres_migrations(&primary)
        .await
        .expect("migrate primary");

    // The primary has a write LSN; the standby is in recovery and has a replay LSN.
    let write_lsn = current_wal_lsn(&primary).await.expect("primary write lsn");
    assert!(
        current_wal_lsn(&replica).await.is_err()
            || replica_replay_lsn(&replica)
                .await
                .expect("replay")
                .is_some(),
        "the replica pool must look like a standby (has a replay position)"
    );

    // Force some WAL past the current token, then wait for the standby to replay it.
    sqlx::query("CREATE TABLE IF NOT EXISTS repl_probe (id bigserial primary key, at timestamptz default now())")
        .execute(&primary)
        .await
        .expect("ddl");
    for _ in 0..20 {
        sqlx::query("INSERT INTO repl_probe DEFAULT VALUES")
            .execute(&primary)
            .await
            .expect("insert");
    }
    let token = current_wal_lsn(&primary).await.expect("token after writes");
    assert!(token >= write_lsn, "write LSN advances monotonically");

    // The standby should catch up to the token within a short window.
    let mut caught_up = false;
    for _ in 0..50 {
        if replica_caught_up(&replica, token)
            .await
            .expect("caught up check")
        {
            caught_up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        caught_up,
        "standby replayed up to the primary token {token}"
    );

    // And the replicated rows are visible on the standby (read-your-write once caught up).
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM repl_probe")
        .fetch_one(&replica)
        .await
        .expect("count on replica");
    assert!(
        n >= 20,
        "replicated rows are visible on the standby (got {n})"
    );
}
