//! `report_stalled_claims` on both backends: a live claim whose holder shows
//! no progress past the cutoff, or whose lease lapses before it, gets one
//! `ClaimStalled` per reason naming the holder, its progress clock and its
//! deadline, and the claim is untouched. A declared wait, an unacknowledged
//! claim (the `ClaimUnacknowledged` report's) and a lapsed lease (the
//! reaper's) are not stalled, and a new claim of the thread is reported again.
//!
//! Cutoffs sit ten minutes either side of now: Postgres stamps progress with
//! the container's clock, which can drift from the host's.

use std::sync::Arc;

use chrono::{Duration, Utc};
use maidan_store::prelude::*;
use maidan_types::*;

use super::thread_progress::{postgres, room, sqlite, task, Room};

async fn take(store: &dyn Store, room: &Room, acknowledge: bool) -> Thread {
    let claimed = store
        .claim_next_thread(room.channel, room.worker, Some(3600), None)
        .await
        .unwrap()
        .unwrap();
    if acknowledge {
        store
            .acknowledge_claim(claimed.id, room.worker, claimed.claim_lease_id.unwrap())
            .await
            .unwrap();
    }
    claimed
}

async fn quiet(store: &dyn Store) -> Vec<StoredEvent> {
    let now = Utc::now();
    store
        .report_stalled_claims(
            now,
            ClaimStallReason::NoProgress,
            now + Duration::minutes(10),
            100,
        )
        .await
        .unwrap()
}

async fn lapsing(store: &dyn Store, within: Duration) -> Vec<StoredEvent> {
    let now = Utc::now();
    store
        .report_stalled_claims(now, ClaimStallReason::LeaseLapsing, now + within, 100)
        .await
        .unwrap()
}

fn stalled(
    event: &StoredEvent,
) -> (
    MemberId,
    ClaimStallReason,
    Option<chrono::DateTime<Utc>>,
    chrono::DateTime<Utc>,
) {
    assert_eq!(event.kind, EventKind::ClaimStalled);
    let Event::ClaimStalled {
        member_id,
        reason,
        last_progress_at,
        lease_expires_at,
        ..
    } = serde_json::from_value(event.payload.clone()).unwrap()
    else {
        panic!("not a ClaimStalled: {event:?}");
    };
    (member_id, reason, last_progress_at, lease_expires_at)
}

async fn a_quiet_claim_is_reported_once_per_reason_and_left_alone(store: Arc<dyn Store>) {
    let store = store.as_ref();
    let room = room(store, "quiet").await;
    let thread = task(store, &room, "index").await;
    let held = take(store, &room, true).await;
    assert_eq!(held.id, thread);

    let early = Utc::now() - Duration::minutes(10);
    let none = store
        .report_stalled_claims(Utc::now(), ClaimStallReason::NoProgress, early, 100)
        .await
        .unwrap();
    assert!(none.is_empty(), "progress after the cutoff is not stalled");
    assert!(
        lapsing(store, Duration::minutes(10)).await.is_empty(),
        "an hour's lease is not lapsing within ten minutes"
    );

    let events = quiet(store).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].thread_id, Some(thread));
    let (member, reason, progress, deadline) = stalled(&events[0]);
    assert_eq!(member, room.worker, "names the holder");
    assert_eq!(reason, ClaimStallReason::NoProgress);
    assert!(
        progress.is_some(),
        "carries the progress clock it was judged on"
    );
    let read = store.get_thread(thread).await.unwrap();
    assert_eq!(Some(deadline), read.assignment_expires_at);
    assert_eq!(
        read.assignee_id,
        Some(room.worker),
        "the claim is untouched"
    );
    assert_eq!(read.claim_lease_id, held.claim_lease_id);
    assert!(quiet(store).await.is_empty(), "reported once");

    // The other reason has its own record.
    let events = lapsing(store, Duration::hours(2)).await;
    assert_eq!(events.len(), 1);
    assert_eq!(stalled(&events[0]).1, ClaimStallReason::LeaseLapsing);
    assert!(
        lapsing(store, Duration::hours(2)).await.is_empty(),
        "warned once"
    );

    // A new claim mints a new token and is reported again.
    store
        .release_claim(thread, room.worker, held.claim_lease_id.unwrap())
        .await
        .unwrap();
    take(store, &room, true).await;
    assert_eq!(quiet(store).await.len(), 1);
    assert_eq!(lapsing(store, Duration::hours(2)).await.len(), 1);
}

async fn claims_that_said_why_or_never_started_are_not_stalled(store: Arc<dyn Store>) {
    let store = store.as_ref();
    let room = room(store, "said").await;
    let waiting = task(store, &room, "waiting").await;
    take(store, &room, true).await;
    store
        .declare_thread_status(
            waiting,
            DeclaredStatus::NeedsInput,
            "which region?".into(),
            room.worker,
        )
        .await
        .unwrap();
    let idle = task(store, &room, "idle").await;
    let unacked = take(store, &room, false).await;
    assert_eq!(unacked.id, idle);

    assert!(
        quiet(store).await.is_empty(),
        "a declared wait and an unacknowledged claim"
    );
    // Both leases are still lapsing, though: the warning is about the lease.
    assert_eq!(lapsing(store, Duration::hours(2)).await.len(), 2);

    // A lease already lapsed at `now` is the reaper's, not a warning.
    let later = Utc::now() + Duration::hours(3);
    let fresh = task(store, &room, "fresh").await;
    take(store, &room, true).await;
    let none = store
        .report_stalled_claims(
            later,
            ClaimStallReason::LeaseLapsing,
            later + Duration::hours(1),
            100,
        )
        .await
        .unwrap();
    assert!(none.is_empty(), "{fresh:?} lapsed at `now`");
}

macro_rules! on_both_backends {
    ($($name:ident),* $(,)?) => {
        mod sqlite_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(super::sqlite().await).await;
                }
            )*
        }
        mod postgres_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    let Some((_container, store)) = super::postgres().await else {
                        return;
                    };
                    super::$name(store).await;
                }
            )*
        }
    };
}

on_both_backends!(
    a_quiet_claim_is_reported_once_per_reason_and_left_alone,
    claims_that_said_why_or_never_started_are_not_stalled,
);
