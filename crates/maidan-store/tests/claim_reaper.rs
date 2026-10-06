//! The claim reaper's store half: `reap_expired_claims` on both backends.
//!
//! A lapsed lease on an open thread is returned to the queue with one
//! `ClaimExpired` naming the dead holder, whose token is fenced from then on.
//! A live lease, a claim with no lease and a lease on a thread that left
//! `open` are left alone. A batch is bounded and takes the oldest deadline
//! first, and concurrent reapers report each lapse once.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    ChannelId, Event, EventKind, MemberId, MemberKind, NewChannel, NewMember, NewThread,
    NewWorkspace, Thread,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn setup(store: &dyn Store, threads: usize) -> (ChannelId, MemberId, MemberId) {
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
                description: None,
            })
            .await
            .unwrap();
    }
    let mut members = Vec::new();
    for handle in ["dead", "next"] {
        let m = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        members.push(m.id);
    }
    (channel.id, members[0], members[1])
}

async fn claim(
    store: &dyn Store,
    channel: ChannelId,
    member: MemberId,
    lease: Option<i64>,
) -> Thread {
    store
        .claim_next_thread(channel, member, lease)
        .await
        .unwrap()
        .expect("a claimable thread")
}

/// Move a live claim's deadline `secs_ago` into the past: a holder that
/// stopped renewing. Claims are taken on a live lease first, because a thread
/// whose lease has lapsed is claimable and the next claim would take it again.
async fn lapse(store: &dyn Store, held: &Thread, secs_ago: i64) -> Thread {
    store
        .renew_claim(
            held.id,
            held.assignee_id.unwrap(),
            held.claim_lease_id.unwrap(),
            -secs_ago,
        )
        .await
        .unwrap()
}

/// Claim the next thread and let its lease lapse `secs_ago`.
async fn lapsed(store: &dyn Store, channel: ChannelId, member: MemberId, secs_ago: i64) -> Thread {
    let held = claim(store, channel, member, Some(3600)).await;
    lapse(store, &held, secs_ago).await
}

async fn a_lapsed_lease_is_reaped_once_and_the_holder_fenced(store: &dyn Store) {
    let (channel, dead, next) = setup(store, 1).await;
    let held = lapsed(store, channel, dead, 5).await;
    let stale = held.claim_lease_id.expect("a claim has a token");
    store
        .acknowledge_claim(held.id, dead, stale)
        .await
        .expect("the holder starts work before it dies");

    let events = store.reap_expired_claims(Utc::now(), 100).await.unwrap();
    assert_eq!(events.len(), 1, "one lapsed lease, one event");
    assert_eq!(events[0].kind, EventKind::ClaimExpired);
    assert_eq!(events[0].thread_id, Some(held.id));
    let expired: Event = serde_json::from_value(events[0].payload.clone()).unwrap();
    assert_eq!(expired.member_id(), Some(dead), "names the dead holder");
    let Event::ClaimExpired { thread, .. } = expired else {
        panic!("not a ClaimExpired");
    };
    assert_eq!(thread.assignee_id, None, "the snapshot is the freed thread");

    let freed = store.get_thread(held.id).await.unwrap();
    assert_eq!(freed.assignee_id, None);
    assert_eq!(freed.assignment_expires_at, None);
    assert_eq!(freed.claim_lease_id, None);
    assert_eq!(freed.work_started_at, None, "the working clock is cleared");

    // Reported once: a second sweep and the next claim say nothing more.
    assert!(store
        .reap_expired_claims(Utc::now(), 100)
        .await
        .unwrap()
        .is_empty());
    let (taken, claim_events) = store
        .claim_next_thread_with_event(channel, next, Some(60))
        .await
        .unwrap();
    assert_eq!(
        taken.map(|t| t.id),
        Some(held.id),
        "the work is back in the queue"
    );
    assert_eq!(
        claim_events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ThreadAssignmentChanged],
        "no second ClaimExpired"
    );

    // The dead holder comes back to a claim it no longer has.
    let fenced = |r: Result<Thread, StoreError>| matches!(r, Err(StoreError::NotFound));
    assert!(fenced(store.renew_claim(held.id, dead, stale, 60).await));
    assert!(fenced(store.acknowledge_claim(held.id, dead, stale).await));
    assert!(fenced(store.release_claim(held.id, dead, stale).await));
}

async fn live_leases_unleased_claims_and_threads_out_of_open_are_left_alone(store: &dyn Store) {
    let (channel, holder, _) = setup(store, 3).await;
    let live = claim(store, channel, holder, Some(3600)).await;
    let unleased = claim(store, channel, holder, None).await;
    let in_review = lapsed(store, channel, holder, 5).await;
    store
        .transition_thread(in_review.id, holder, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();

    assert!(store
        .reap_expired_claims(Utc::now(), 100)
        .await
        .unwrap()
        .is_empty());
    for (thread, what) in [
        (live, "a live lease"),
        (unleased, "a claim with no lease"),
        (in_review, "a thread in review"),
    ] {
        assert_eq!(
            store.get_thread(thread.id).await.unwrap().assignee_id,
            Some(holder),
            "{what} keeps its holder"
        );
    }
}

async fn a_batch_is_bounded_and_takes_the_oldest_deadline_first(store: &dyn Store) {
    let (channel, holder, _) = setup(store, 3).await;
    // The last claimed lapsed longest ago.
    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(claim(store, channel, holder, Some(3600)).await);
    }
    let a = lapse(store, &held[0], 10).await;
    let b = lapse(store, &held[1], 20).await;
    let c = lapse(store, &held[2], 30).await;

    let first = store.reap_expired_claims(Utc::now(), 2).await.unwrap();
    assert_eq!(
        first.iter().map(|e| e.thread_id).collect::<Vec<_>>(),
        vec![Some(c.id), Some(b.id)],
        "the two oldest deadlines, oldest first"
    );
    let rest = store.reap_expired_claims(Utc::now(), 2).await.unwrap();
    assert_eq!(
        rest.iter().map(|e| e.thread_id).collect::<Vec<_>>(),
        vec![Some(a.id)]
    );
}

async fn a_lease_lapsing_after_now_waits_for_the_next_sweep(store: &dyn Store) {
    let (channel, holder, _) = setup(store, 1).await;
    let held = lapsed(store, channel, holder, 5).await;
    let before_it_lapsed = held.assignment_expires_at.unwrap() - chrono::Duration::seconds(1);
    assert!(store
        .reap_expired_claims(before_it_lapsed, 100)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .reap_expired_claims(Utc::now(), 100)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Reapers on several replicas sweep at once: every lapse is reported by
/// exactly one of them.
async fn concurrent_reapers_report_each_lapse_once(store: Arc<dyn Store>) {
    const THREADS: usize = 40;
    let (channel, holder, _) = setup(store.as_ref(), THREADS).await;
    let mut claims = Vec::new();
    for _ in 0..THREADS {
        claims.push(claim(store.as_ref(), channel, holder, Some(3600)).await);
    }
    let mut held = HashSet::new();
    for claimed in &claims {
        held.insert(lapse(store.as_ref(), claimed, 5).await.id);
    }
    let reapers = (0..4).map(|_| {
        let store = store.clone();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            loop {
                let events = store.reap_expired_claims(Utc::now(), 3).await.unwrap();
                if events.is_empty() {
                    return seen;
                }
                seen.extend(events.into_iter().filter_map(|e| e.thread_id));
            }
        })
    });
    let mut reported = HashSet::new();
    for reaper in reapers {
        for id in reaper.await.unwrap() {
            assert!(reported.insert(id), "thread {id:?} reported twice");
        }
    }
    assert_eq!(reported, held, "every lapse reported");
}

async fn run_suite(store: Arc<dyn Store>) {
    a_lapsed_lease_is_reaped_once_and_the_holder_fenced(store.as_ref()).await;
    live_leases_unleased_claims_and_threads_out_of_open_are_left_alone(store.as_ref()).await;
    a_batch_is_bounded_and_takes_the_oldest_deadline_first(store.as_ref()).await;
    a_lease_lapsing_after_now_waits_for_the_next_sweep(store.as_ref()).await;
    concurrent_reapers_report_each_lapse_once(store).await;
}

#[tokio::test]
async fn reap_expired_claims_sqlite() {
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
    run_suite(Arc::new(SqliteStore::for_tests(pool))).await;
}

#[tokio::test]
async fn reap_expired_claims_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    run_suite(Arc::new(PostgresStore::for_tests(pool))).await;
}
