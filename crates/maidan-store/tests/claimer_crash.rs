//! A claimer that crashes mid-work (Wave 4 #45).
//!
//! An agent claims a thread with a lease and dies without releasing it. The
//! work must come back to the queue when the lease lapses, the next claimer
//! must be told the previous claim expired, and the dead claimer — should it
//! come back — must be fenced out: it can no longer renew, acknowledge or
//! release a claim it lost. Under a crowd of claimers, some crashing, every
//! thread is still claimed by exactly one live holder at a time. On both
//! backends.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    ChannelId, EventKind, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

/// Long enough to be clearly held, short enough to lapse in a test.
const LEASE_SECS: i64 = 1;

async fn setup(store: &dyn Store, threads: usize, claimers: usize) -> (ChannelId, Vec<MemberId>) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    for i in 0..threads {
        store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(format!("task {i}")),
            })
            .await
            .unwrap();
    }
    let mut members = Vec::new();
    for i in 0..claimers {
        let m = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: format!("agent-{i}"),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        members.push(m.id);
    }
    (channel.id, members)
}

async fn a_crashed_claim_lapses_and_the_dead_claimer_is_fenced(store: &dyn Store) {
    let (channel, members) = setup(store, 1, 2).await;
    let (dead, next) = (members[0], members[1]);

    let claimed = store
        .claim_next_thread(channel, dead, Some(LEASE_SECS))
        .await
        .unwrap()
        .expect("the thread is claimable");
    let stale_lease = claimed.claim_lease_id.expect("a leased claim has a token");
    // `dead` crashes here: no heartbeat, no release.

    assert!(
        store
            .claim_next_thread(channel, next, Some(LEASE_SECS))
            .await
            .unwrap()
            .is_none(),
        "a live lease is not reclaimable"
    );

    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (taken, events) = store
        .claim_next_thread_with_event(channel, next, Some(30))
        .await
        .unwrap();
    let taken = taken.expect("the lapsed claim returns to the queue");
    assert_eq!(taken.id, claimed.id);
    assert_eq!(taken.assignee_id, Some(next));
    assert_ne!(
        taken.claim_lease_id,
        Some(stale_lease),
        "a new claim gets a new token"
    );
    assert_eq!(
        events.first().map(|e| e.kind),
        Some(EventKind::ClaimExpired),
        "the takeover says the previous claim expired"
    );

    // The dead claimer comes back. Every write on its old claim is refused.
    let fenced =
        |r: Result<maidan_types::Thread, StoreError>| matches!(r, Err(StoreError::NotFound));
    assert!(fenced(
        store.renew_claim(claimed.id, dead, stale_lease, 30).await
    ));
    assert!(fenced(
        store.acknowledge_claim(claimed.id, dead, stale_lease).await
    ));
    assert!(fenced(
        store.release_claim(claimed.id, dead, stale_lease).await
    ));
    let still = store.get_thread(claimed.id).await.unwrap();
    assert_eq!(
        still.assignee_id,
        Some(next),
        "the live holder keeps the work"
    );
    assert_eq!(still.claim_lease_id, taken.claim_lease_id);

    let released = store
        .release_claim(claimed.id, next, taken.claim_lease_id.unwrap())
        .await
        .unwrap();
    assert_eq!(released.assignee_id, None);
}

/// Many claimers race for the queue; a third of them crash while holding
/// their claim. After the leases lapse, the survivors drain what the crashed
/// ones left, and no thread is ever held by two claimers at once.
async fn crashing_claimers_never_double_hold_a_thread(store: Arc<dyn Store>) {
    const THREADS: usize = 12;
    const CLAIMERS: usize = 6;
    let (channel, members) = setup(store.as_ref(), THREADS, CLAIMERS).await;

    let mut first_round = Vec::new();
    for (i, member) in members.iter().copied().enumerate() {
        let store = store.clone();
        first_round.push(tokio::spawn(async move {
            let mut held = Vec::new();
            if let Some(t) = store
                .claim_next_thread(channel, member, Some(LEASE_SECS))
                .await
                .unwrap()
            {
                held.push(t.id);
                if i % 3 == 0 {
                    // Crash while holding it: no release.
                    return (member, held, true);
                }
                store
                    .release_claim(t.id, member, t.claim_lease_id.unwrap())
                    .await
                    .unwrap();
            }
            (member, held, false)
        }));
    }
    let mut crashed_threads = HashSet::new();
    for handle in first_round {
        let (_, held, crashed) = handle.await.unwrap();
        if crashed {
            crashed_threads.extend(held);
        }
    }
    assert!(!crashed_threads.is_empty(), "the scenario crashed someone");

    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Survivors now take everything, crashed claims included; each thread is
    // claimed once, by one survivor.
    let survivors: Vec<MemberId> = members
        .iter()
        .copied()
        .enumerate()
        .filter(|(i, _)| i % 3 != 0)
        .map(|(_, m)| m)
        .collect();
    let mut second_round = Vec::new();
    for member in survivors {
        let store = store.clone();
        second_round.push(tokio::spawn(async move {
            let mut held = Vec::new();
            while let Some(t) = store
                .claim_next_thread(channel, member, Some(60))
                .await
                .unwrap()
            {
                held.push(t.id);
            }
            held
        }));
    }
    let mut seen = HashSet::new();
    for handle in second_round {
        for id in handle.await.unwrap() {
            assert!(seen.insert(id), "thread {id:?} claimed twice");
        }
    }
    assert_eq!(seen.len(), THREADS, "every thread was taken exactly once");
    assert!(crashed_threads.is_subset(&seen), "crashed claims came back");
}

#[tokio::test]
async fn a_crashed_claimer_is_recovered_and_fenced_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    a_crashed_claim_lapses_and_the_dead_claimer_is_fenced(store.as_ref()).await;
    crashing_claimers_never_double_hold_a_thread(store).await;
}

#[tokio::test]
async fn a_crashed_claimer_is_recovered_and_fenced_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool));
    a_crashed_claim_lapses_and_the_dead_claimer_is_fenced(store.as_ref()).await;
    crashing_claimers_never_double_hold_a_thread(store).await;
}
