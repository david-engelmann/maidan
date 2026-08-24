//! Token-aware read routing validated against REAL streaming replication
//! (Cluster 264). `#[ignore]`d — needs the primary+replica pair from
//! `scripts/replica-harness.sh` (MAIDAN_PRIMARY_URL / MAIDAN_REPLICA_URL); skips
//! when they're unset. The pure routing decision is unit-tested in CI
//! (`route_decision`); this proves the end-to-end machinery (scope task-local +
//! read_pool + background replay-LSN poller) against an actual standby.

use std::time::Duration;

use maidan_store::postgres::{replication::replica_replay_lsn, with_read_consistency};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::NewWorkspace;
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
#[ignore = "needs a live primary+replica pair (scripts/replica-harness.sh up); run with --ignored"]
async fn token_read_is_never_stale_and_replica_serves_reads() {
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
    run_postgres_migrations(&primary).await.expect("migrate");

    let store = PostgresStore::with_replica_reader(primary.clone(), replica.clone());

    // Write, and capture the causality token for it.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "route".into(),
        })
        .await
        .expect("create");
    let token = store.write_lsn().await.expect("lsn").expect("some lsn");

    // Read-your-write: with the token in scope, the read must return the just-written
    // row no matter how far the replica has (or hasn't) replayed — routed to the
    // primary while the replica is behind the token.
    let got = with_read_consistency(Some(token), store.get_workspace(ws.id))
        .await
        .expect("read-your-write");
    assert_eq!(got.id, ws.id);
    assert_eq!(got.name, "route");

    // Wait for the standby to actually replay past the token, then a no-token read
    // is served from the replica and still returns the row (replica serves reads).
    let mut caught_up = false;
    for _ in 0..50 {
        if replica_replay_lsn(&replica)
            .await
            .expect("replay")
            .is_some_and(|r| r >= token)
        {
            caught_up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(caught_up, "replica replayed past the token {token}");

    let from_replica = with_read_consistency(None, store.get_workspace(ws.id))
        .await
        .expect("replica read");
    assert_eq!(from_replica.id, ws.id, "replica serves the replicated row");

    // And the row genuinely exists on the replica (replication really happened).
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM maidan_workspaces WHERE id = $1")
        .bind(ws.id.0)
        .fetch_one(&replica)
        .await
        .expect("count on replica");
    assert_eq!(n, 1, "the write is present on the standby");

    // The routing counters saw both outcomes: the token read went to the primary
    // (replica behind), the no-token read went to the replica (Cluster 265 metric).
    let (primary, replica_reads) = store.read_routing_metrics().snapshot();
    assert!(
        primary >= 1,
        "a read was routed to the primary (got {primary})"
    );
    assert!(
        replica_reads >= 1,
        "a read was routed to the replica (got {replica_reads})"
    );
}
