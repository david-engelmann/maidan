//! A result write is fenced by the writer's live claim, on both backends: a
//! seat whose lease lapsed and was taken over cannot land its result over
//! the new holder's, a lease that expired is lost before anyone takes the
//! thread over or the reaper frees it, and a lease from another workspace
//! fences nothing.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, ClaimLeaseId, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ThreadId,
};
use serde_json::json;

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
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

struct Tenant {
    channel: ChannelId,
    seats: Vec<MemberId>,
}

async fn tenant(store: &dyn Store, name: &str, seats: usize) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let mut members = Vec::new();
    for n in 0..seats {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: format!("seat-{n}"),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member")
                .id,
        );
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    Tenant {
        channel: channel.id,
        seats: members,
    }
}

async fn task(store: &dyn Store, channel: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id
}

async fn claim(
    store: &dyn Store,
    channel: ChannelId,
    seat: MemberId,
    secs: i64,
) -> (ThreadId, ClaimLeaseId) {
    let thread = store
        .claim_next_thread(channel, seat, Some(secs), None)
        .await
        .expect("claim")
        .expect("a thread to claim");
    (
        thread.id,
        thread.claim_lease_id.expect("a claim carries a lease"),
    )
}

fn is_claim_lost(result: Result<maidan_types::ThreadResult, StoreError>) -> bool {
    matches!(result, Err(StoreError::ClaimLost(_)))
}

async fn a_lapsed_seats_result_is_refused_and_the_new_holders_lands(store: &dyn Store) {
    let t = tenant(store, "ramp", 2).await;
    let (a, b) = (t.seats[0], t.seats[1]);
    let thread = task(store, t.channel).await;
    let (claimed, lease_a) = claim(store, t.channel, a, 1).await;
    assert_eq!(claimed, thread);

    // A is slow, not dead: its lease lapses and B takes the thread over.
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    let (taken, lease_b) = claim(store, t.channel, b, 300).await;
    assert_eq!(taken, thread);
    assert_ne!(lease_a, lease_b);

    assert!(
        is_claim_lost(
            store
                .set_thread_result(thread, a, lease_a, &json!({ "from": "a" }))
                .await
        ),
        "A no longer holds the thread"
    );
    assert!(
        store
            .get_thread_result(thread)
            .await
            .expect("get")
            .is_none(),
        "a refused write stores nothing"
    );
    // Not B's lease under A's name either: the fence is holder and lease.
    assert!(is_claim_lost(
        store
            .set_thread_result(thread, a, lease_b, &json!({ "from": "a" }))
            .await
    ));

    let landed = store
        .set_thread_result(thread, b, lease_b, &json!({ "from": "b" }))
        .await
        .expect("the holder's write lands");
    assert_eq!(landed.produced_by, b);
    assert_eq!(
        store
            .get_thread_result(thread)
            .await
            .expect("get")
            .map(|r| r.result),
        Some(json!({ "from": "b" }))
    );

    // Released, the lease fences nothing any more.
    store
        .release_claim(thread, b, lease_b)
        .await
        .expect("release");
    assert!(is_claim_lost(
        store
            .set_thread_result(thread, b, lease_b, &json!({ "from": "b again" }))
            .await
    ));
    assert_eq!(
        store
            .get_thread_result(thread)
            .await
            .expect("get")
            .map(|r| r.result),
        Some(json!({ "from": "b" })),
        "the refused re-set left the stored result alone"
    );
}

/// Between a lease's deadline and the reaper, `claim_next` already counts the
/// thread as free and may hand it on. A renewal or result in that window
/// used to answer success and, for a renewal, revive the claim, so two seats
/// could each believe they held the thread. Now the old holder is told its
/// claim is lost, however its calls and the takeover interleave.
async fn an_expired_lease_is_lost_before_anyone_takes_it_over(store: &dyn Store) {
    let t = tenant(store, "window", 2).await;
    let (a, b) = (t.seats[0], t.seats[1]);
    let thread = task(store, t.channel).await;
    let (claimed, lease_a) = claim(store, t.channel, a, 300).await;
    assert_eq!(claimed, thread);
    // A stops renewing: its deadline passes. Nothing has reaped it yet.
    store
        .renew_claim(thread, a, lease_a, -5)
        .await
        .expect("a live lease renews, here into the past");
    let held = store.get_thread(thread).await.expect("get");
    assert_eq!(held.assignee_id, Some(a), "not reaped yet");
    assert_eq!(held.claim_lease_id, Some(lease_a));

    let renewed = store.renew_claim(thread, a, lease_a, 300).await;
    assert!(
        matches!(renewed, Err(StoreError::ClaimLost(_))),
        "an expired lease does not renew: {renewed:?}"
    );
    assert!(
        is_claim_lost(
            store
                .set_thread_result(thread, a, lease_a, &json!({ "from": "a" }))
                .await
        ),
        "an expired lease writes no result"
    );
    assert!(store
        .get_thread_result(thread)
        .await
        .expect("get")
        .is_none());
    assert!(
        store
            .get_thread(thread)
            .await
            .expect("get")
            .assignment_expires_at
            .is_some_and(|deadline| deadline < chrono::Utc::now()),
        "the refused renewal did not move the deadline"
    );

    // A's calls race B's takeover; in either order A is refused and B holds.
    let (a_renew, a_result, taken) = tokio::join!(
        store.renew_claim(thread, a, lease_a, 300),
        store.set_thread_result(thread, a, lease_a, &json!({ "from": "a" })),
        store.claim_next_thread(t.channel, b, Some(300), None),
    );
    assert!(
        matches!(a_renew, Err(StoreError::ClaimLost(_))),
        "{a_renew:?}"
    );
    assert!(is_claim_lost(a_result));
    let taken = taken
        .expect("claim")
        .expect("the lapsed thread is claimable");
    assert_eq!(taken.id, thread);
    assert_eq!(taken.assignee_id, Some(b));
    let lease_b = taken.claim_lease_id.expect("a claim carries a lease");
    store
        .renew_claim(thread, b, lease_b, 300)
        .await
        .expect("the new holder renews");
    store
        .set_thread_result(thread, b, lease_b, &json!({ "from": "b" }))
        .await
        .expect("the new holder's result lands");
    assert_eq!(
        store
            .get_thread_result(thread)
            .await
            .expect("get")
            .map(|r| r.produced_by),
        Some(b)
    );

    // A claim taken without a lease has no deadline to pass.
    let durable = task(store, t.channel).await;
    let claim = store.claim_thread(durable, a).await.expect("claim");
    let lease = claim
        .thread
        .claim_lease_id
        .expect("a claim carries a lease");
    assert_eq!(claim.thread.assignment_expires_at, None);
    store
        .set_thread_result(durable, a, lease, &json!({ "from": "a" }))
        .await
        .expect("an unleased claim writes its result");
    // A thread that does not exist is not found, not lost.
    assert!(matches!(
        store
            .renew_claim(ThreadId(uuid::Uuid::now_v7()), a, lease, 60)
            .await,
        Err(StoreError::NotFound)
    ));
}

async fn a_lease_from_another_workspace_fences_nothing(store: &dyn Store) {
    let one = tenant(store, "one", 1).await;
    let two = tenant(store, "two", 1).await;
    let thread_one = task(store, one.channel).await;
    let thread_two = task(store, two.channel).await;
    let (_, lease_one) = claim(store, one.channel, one.seats[0], 300).await;
    let (_, lease_two) = claim(store, two.channel, two.seats[0], 300).await;

    // Two's seat, holding a live lease in its own workspace, cannot write
    // One's thread with it, nor with One's lease.
    for lease in [lease_two, lease_one] {
        assert!(is_claim_lost(
            store
                .set_thread_result(thread_one, two.seats[0], lease, &json!({ "from": "two" }))
                .await
        ));
    }
    assert!(store
        .get_thread_result(thread_one)
        .await
        .expect("get")
        .is_none());

    // Each holder writes its own thread.
    store
        .set_thread_result(
            thread_one,
            one.seats[0],
            lease_one,
            &json!({ "from": "one" }),
        )
        .await
        .expect("one");
    store
        .set_thread_result(
            thread_two,
            two.seats[0],
            lease_two,
            &json!({ "from": "two" }),
        )
        .await
        .expect("two");
}

async fn run_suite(store: &dyn Store) {
    a_lapsed_seats_result_is_refused_and_the_new_holders_lands(store).await;
    an_expired_lease_is_lost_before_anyone_takes_it_over(store).await;
    a_lease_from_another_workspace_fences_nothing(store).await;
}

#[tokio::test]
async fn a_result_write_is_fenced_by_the_live_claim_on_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_result_write_is_fenced_by_the_live_claim_on_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
