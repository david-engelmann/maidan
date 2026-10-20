//! `return_human_timeouts` on both backends: a holder that declared it needs
//! a person and got no answer before the cutoff loses its claim, its stale
//! declaration goes, and one `WaitTimedOut` (policy `return_to_queue`)
//! records it; the thread stays open and claimable. Only the holder's own
//! `needs_input` or `needs_review` on a live, acknowledged lease counts, an
//! answer stops the clock, and a wait in another workspace that is not yet
//! due is left alone.

use std::sync::Arc;

use maidan_store::prelude::*;
use maidan_types::*;

use super::thread_progress::{postgres, room, sqlite, task, tick, Room};

const POLICY: &str = "return_to_queue";

/// Claim `thread` (the channel's only open one) for the worker, acknowledge
/// it, and have the worker declare `status`.
async fn waiting_on(store: &dyn Store, room: &Room, thread: ThreadId, status: DeclaredStatus) {
    let claimed = store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, thread);
    store
        .acknowledge_claim(thread, room.worker, claimed.claim_lease_id.unwrap())
        .await
        .unwrap();
    store
        .declare_thread_status(thread, status, "which schema?".into(), room.worker)
        .await
        .unwrap();
}

async fn sweep(store: &dyn Store) -> Vec<StoredEvent> {
    let now = chrono::Utc::now();
    store
        .return_human_timeouts(now, now, POLICY, 100)
        .await
        .unwrap()
}

async fn an_unanswered_wait_returns_the_thread_to_the_queue(store: Arc<dyn Store>) {
    let ours = room(store.as_ref(), "ours").await;
    let theirs = room(store.as_ref(), "theirs").await;
    let thread = task(store.as_ref(), &ours, "migrate").await;
    waiting_on(store.as_ref(), &ours, thread, DeclaredStatus::NeedsInput).await;

    let early = chrono::Utc::now() - chrono::Duration::hours(1);
    let none = store
        .return_human_timeouts(chrono::Utc::now(), early, POLICY, 100)
        .await
        .unwrap();
    assert!(none.is_empty(), "not due before the cutoff");

    tick().await;
    // Declared after the cutoff the sweep below uses: not yet due.
    let cutoff = chrono::Utc::now();
    tick().await;
    let other = task(store.as_ref(), &theirs, "theirs").await;
    waiting_on(store.as_ref(), &theirs, other, DeclaredStatus::NeedsReview).await;

    let returned = store
        .return_human_timeouts(chrono::Utc::now(), cutoff, POLICY, 100)
        .await
        .unwrap();
    assert_eq!(returned.len(), 1);
    let event = &returned[0];
    assert_eq!(event.kind, EventKind::WaitTimedOut);
    assert_eq!(event.thread_id, Some(thread));
    assert_eq!(event.channel_id, Some(ours.channel));
    assert_eq!(event.payload["policy"], POLICY);
    assert_eq!(event.payload["reason"], "which schema?");

    let back = store.get_thread(thread).await.unwrap();
    assert_eq!(
        back.state,
        ThreadState::Open,
        "nothing is decided for the person"
    );
    assert_eq!(back.assignee_id, None);
    assert_eq!(back.assignment_expires_at, None);
    assert_eq!(back.claim_lease_id, None);
    assert_eq!(back.work_started_at, None);
    assert_eq!(
        store.get_thread_status(thread).await.unwrap(),
        None,
        "the declaration spoke for a holder that is gone"
    );
    let reclaimed = store
        .claim_next_thread(ours.channel, ours.worker, Some(600), None)
        .await
        .unwrap()
        .expect("claimable again");
    assert_eq!(reclaimed.id, thread);

    let still = store.get_thread(other).await.unwrap();
    assert_eq!(still.assignee_id, Some(theirs.worker), "not due yet");
    assert!(store.get_thread_status(other).await.unwrap().is_some());

    let due_now = sweep(store.as_ref()).await;
    assert_eq!(due_now.len(), 1, "each wait is returned once");
    assert_eq!(due_now[0].thread_id, Some(other));
    assert!(sweep(store.as_ref()).await.is_empty());
}

async fn only_a_holders_unanswered_wait_on_a_person_counts(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "kinds").await;
    let mut held = Vec::new();
    for status in [
        DeclaredStatus::Working,
        DeclaredStatus::Blocked,
        DeclaredStatus::Done,
    ] {
        let thread = task(store.as_ref(), &room, status.as_str()).await;
        waiting_on(store.as_ref(), &room, thread, status).await;
        held.push(thread);
    }

    let answered = task(store.as_ref(), &room, "answered").await;
    waiting_on(store.as_ref(), &room, answered, DeclaredStatus::NeedsInput).await;
    store.clear_thread_status(answered).await.unwrap();
    held.push(answered);

    // The owner's declaration replaces the holder's and does not speak for it.
    let by_owner = task(store.as_ref(), &room, "owner").await;
    waiting_on(store.as_ref(), &room, by_owner, DeclaredStatus::Working).await;
    store
        .set_thread_owner(by_owner, Some(room.lead))
        .await
        .unwrap();
    store
        .declare_thread_status(
            by_owner,
            DeclaredStatus::NeedsInput,
            "mine".into(),
            room.lead,
        )
        .await
        .unwrap();
    held.push(by_owner);

    let unacknowledged = task(store.as_ref(), &room, "unacknowledged").await;
    let claimed = store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, unacknowledged);
    store
        .declare_thread_status(
            unacknowledged,
            DeclaredStatus::NeedsInput,
            "?".into(),
            room.worker,
        )
        .await
        .unwrap();
    held.push(unacknowledged);

    let by_hand = task(store.as_ref(), &room, "by hand").await;
    let assigned = store.assign_thread(by_hand, room.worker).await.unwrap();
    store
        .acknowledge_claim(by_hand, room.worker, assigned.claim_lease_id.unwrap())
        .await
        .unwrap();
    store
        .declare_thread_status(by_hand, DeclaredStatus::NeedsInput, "?".into(), room.worker)
        .await
        .unwrap();
    held.push(by_hand);

    tick().await;
    assert!(sweep(store.as_ref()).await.is_empty());
    for thread in held {
        let read = store.get_thread(thread).await.unwrap();
        assert!(
            read.assignee_id.is_some(),
            "{:?} kept its holder",
            read.title
        );
    }
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
    an_unanswered_wait_returns_the_thread_to_the_queue,
    only_a_holders_unanswered_wait_on_a_person_counts,
);
