//! `report_unacknowledged_claims` on both backends.
//!
//! A leased claim its holder has not acknowledged by the cutoff gets one
//! `ClaimUnacknowledged` naming the holder and when it claimed, and the claim
//! itself is untouched. An acknowledged claim, a claim with no lease, a lapsed
//! lease (the reaper's), a thread in review and a claim taken after the cutoff
//! are not reported. A new claim of the same thread is a new claim and is
//! reported again. A batch is bounded, oldest claim first, and concurrent
//! callers report each claim once.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, Event, EventKind, MemberId, MemberKind, NewChannel, NewMember, NewThread,
    NewWorkspace, StoredEvent, Thread,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn setup(store: &dyn Store, threads: usize) -> (ChannelId, MemberId) {
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
    let holder = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "holder".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    (channel.id, holder.id)
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

/// Report with a cutoff just ahead of now, so every claim taken so far is
/// past its acknowledgement window.
async fn report(store: &dyn Store, limit: i64) -> Vec<StoredEvent> {
    let now = Utc::now();
    store
        .report_unacknowledged_claims(now, now + Duration::seconds(1), limit)
        .await
        .unwrap()
}

async fn an_unacknowledged_claim_is_reported_once_and_left_alone(store: &dyn Store) {
    let (channel, holder) = setup(store, 1).await;
    let before = Utc::now();
    let held = claim(store, channel, holder, Some(3600)).await;
    let after = Utc::now();

    let events = report(store, 100).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, EventKind::ClaimUnacknowledged);
    assert_eq!(events[0].thread_id, Some(held.id));
    let event: Event = serde_json::from_value(events[0].payload.clone()).unwrap();
    let Event::ClaimUnacknowledged {
        member_id,
        claimed_at,
        thread,
        ..
    } = event
    else {
        panic!("not a ClaimUnacknowledged: {event:?}");
    };
    assert_eq!(member_id, holder, "names the holder");
    assert!(
        before - Duration::milliseconds(1) <= claimed_at
            && claimed_at <= after + Duration::milliseconds(1),
        "claimed_at {claimed_at} is when the claim was taken ({before}..{after})"
    );
    assert_eq!(thread.assignee_id, Some(holder));

    // The claim is not changed: same holder, token and deadline.
    let now = store.get_thread(held.id).await.unwrap();
    assert_eq!(now.assignee_id, Some(holder));
    assert_eq!(now.claim_lease_id, held.claim_lease_id);
    assert_eq!(now.assignment_expires_at, held.assignment_expires_at);

    assert!(report(store, 100).await.is_empty(), "reported once");

    // Acknowledging late still works, and the holder can carry on.
    store
        .acknowledge_claim(held.id, holder, held.claim_lease_id.unwrap())
        .await
        .unwrap();
    assert!(report(store, 100).await.is_empty());
}

async fn claims_that_are_not_stuck_are_not_reported(store: &dyn Store) {
    let (channel, holder) = setup(store, 5).await;
    // Every claim is taken on a live lease first: a lapsed thread is
    // claimable, and the next claim would take it again.
    let acknowledged = claim(store, channel, holder, Some(3600)).await;
    let _unleased = claim(store, channel, holder, None).await;
    let lapsed = claim(store, channel, holder, Some(3600)).await;
    let in_review = claim(store, channel, holder, Some(3600)).await;
    // Taken after the cutoff: still inside its window.
    let cutoff = Utc::now();
    let fresh = claim(store, channel, holder, Some(3600)).await;

    store
        .acknowledge_claim(
            acknowledged.id,
            holder,
            acknowledged.claim_lease_id.unwrap(),
        )
        .await
        .unwrap();
    store
        .renew_claim(lapsed.id, holder, lapsed.claim_lease_id.unwrap(), -5)
        .await
        .unwrap();
    store
        .transition_thread(in_review.id, holder, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();

    let events = store
        .report_unacknowledged_claims(Utc::now(), cutoff, 100)
        .await
        .unwrap();
    assert!(events.is_empty(), "nothing is stuck: {events:?}");
    // The suite shares one store: the fresh claim starts work so a later
    // scenario's cutoff does not report it.
    store
        .acknowledge_claim(fresh.id, holder, fresh.claim_lease_id.unwrap())
        .await
        .unwrap();
}

async fn a_new_claim_of_the_same_thread_is_reported_again(store: &dyn Store) {
    let (channel, holder) = setup(store, 1).await;
    let first = claim(store, channel, holder, Some(3600)).await;
    assert_eq!(report(store, 100).await.len(), 1);
    store
        .release_claim(first.id, holder, first.claim_lease_id.unwrap())
        .await
        .unwrap();
    let second = claim(store, channel, holder, Some(3600)).await;
    assert_eq!(second.id, first.id);
    assert_ne!(second.claim_lease_id, first.claim_lease_id);
    let events = report(store, 100).await;
    assert_eq!(events.len(), 1, "a new claim has its own window");
    assert_eq!(events[0].thread_id, Some(second.id));
}

async fn a_batch_is_bounded_and_takes_the_oldest_claim_first(store: &dyn Store) {
    let (channel, holder) = setup(store, 3).await;
    let a = claim(store, channel, holder, Some(3600)).await;
    let b = claim(store, channel, holder, Some(3600)).await;
    let c = claim(store, channel, holder, Some(3600)).await;
    let first = report(store, 2).await;
    assert_eq!(
        first.iter().map(|e| e.thread_id).collect::<Vec<_>>(),
        vec![Some(a.id), Some(b.id)]
    );
    let rest = report(store, 2).await;
    assert_eq!(
        rest.iter().map(|e| e.thread_id).collect::<Vec<_>>(),
        vec![Some(c.id)]
    );
}

async fn concurrent_reporters_report_each_claim_once(store: Arc<dyn Store>) {
    const THREADS: usize = 40;
    let (channel, holder) = setup(store.as_ref(), THREADS).await;
    let mut held = HashSet::new();
    for _ in 0..THREADS {
        held.insert(claim(store.as_ref(), channel, holder, Some(3600)).await.id);
    }
    let reporters = (0..4).map(|_| {
        let store = store.clone();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            loop {
                let events = report(store.as_ref(), 3).await;
                if events.is_empty() {
                    return seen;
                }
                seen.extend(events.into_iter().filter_map(|e| e.thread_id));
            }
        })
    });
    let mut reported = HashSet::new();
    for reporter in reporters {
        for id in reporter.await.unwrap() {
            assert!(reported.insert(id), "thread {id:?} reported twice");
        }
    }
    assert_eq!(reported, held, "every claim reported");
}

async fn run_suite(store: Arc<dyn Store>) {
    an_unacknowledged_claim_is_reported_once_and_left_alone(store.as_ref()).await;
    claims_that_are_not_stuck_are_not_reported(store.as_ref()).await;
    a_new_claim_of_the_same_thread_is_reported_again(store.as_ref()).await;
    a_batch_is_bounded_and_takes_the_oldest_claim_first(store.as_ref()).await;
    concurrent_reporters_report_each_claim_once(store).await;
}

#[tokio::test]
async fn report_unacknowledged_claims_sqlite() {
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
async fn report_unacknowledged_claims_postgres() {
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
