//! Search token-aware read routing validated against REAL streaming replication
//! (Cluster 271). `#[ignore]`d — needs the primary+replica pair from
//! `scripts/replica-harness.sh` (MAIDAN_PRIMARY_URL / MAIDAN_REPLICA_URL); skips
//! when they're unset. The routing decision itself is unit-tested in CI
//! (maidan-store `route_decision`, shared via `replica_route`); this proves the
//! search-side end-to-end machinery (the shared read-consistency task-local +
//! `PostgresSearch::read_pool` + the background replay-LSN poller) against an actual
//! standby, and that search honors the same `Maidan-Consistency-Token` as the store.

use std::time::Duration;

use maidan_search::{PostgresSearch, Search, SearchFilters};
use maidan_store::postgres::{replication::replica_replay_lsn, with_read_consistency};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
#[ignore = "needs a live primary+replica pair (scripts/replica-harness.sh up); run with --ignored"]
async fn search_read_is_never_stale_and_replica_serves_reads() {
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
    let search = PostgresSearch::with_replica_reader(primary.clone(), replica.clone());

    // Seed a searchable message on the primary.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "search-route".into(),
        })
        .await
        .expect("workspace");
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
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("thread");
    store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "peregrine falcon dispatch".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("post");
    let token = store.write_lsn().await.expect("lsn").expect("some lsn");

    // Read-your-write: with the token in scope, the search returns the just-posted
    // message even before the replica has replayed — routed to the primary while the
    // replica is behind the token.
    let hits = with_read_consistency(
        Some(token),
        search.search_messages(ws.id, "peregrine", 10, &SearchFilters::default()),
    )
    .await
    .expect("read-your-write search");
    assert!(
        hits.iter().any(|h| h.body.contains("peregrine")),
        "search sees the just-posted message via the token"
    );

    // Wait for the standby to actually replay past the token, then a no-token search
    // is served from the replica and still finds the message.
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

    let from_replica = with_read_consistency(
        None,
        search.search_messages(ws.id, "peregrine", 10, &SearchFilters::default()),
    )
    .await
    .expect("replica search");
    assert!(
        from_replica.iter().any(|h| h.body.contains("peregrine")),
        "replica serves the replicated searchable message"
    );

    // The row genuinely exists on the replica (replication really happened).
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM maidan_messages WHERE body = 'peregrine falcon dispatch'",
    )
    .fetch_one(&replica)
    .await
    .expect("count on replica");
    assert_eq!(n, 1, "the posted message is present on the standby");

    // The routing counters (Cluster 272) register both outcomes deterministically: an
    // unreachably-high token can never be satisfied by the replica → forced to the
    // primary; a no-token search → the replica. (Assert on these two controlled reads
    // rather than the earlier ones, which race the poller on localhost.)
    let (p0, r0) = search.read_routing_metrics().snapshot();
    let _ = with_read_consistency(
        Some(maidan_types::Lsn(u64::MAX)),
        search.search_messages(ws.id, "peregrine", 10, &SearchFilters::default()),
    )
    .await
    .expect("forced-primary search");
    let _ = with_read_consistency(
        None,
        search.search_messages(ws.id, "peregrine", 10, &SearchFilters::default()),
    )
    .await
    .expect("forced-replica search");
    let (p1, r1) = search.read_routing_metrics().snapshot();
    assert!(p1 > p0, "the high-token search was routed to the primary");
    assert!(r1 > r0, "the no-token search was routed to the replica");
}
