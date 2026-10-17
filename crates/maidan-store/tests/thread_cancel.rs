//! Cancelling a task on both backends: its creator may, a member that did not
//! create it may not unless it holds `thread:transition`, a cancel ends the
//! live claim so the holder's renew is refused, a repeat changes nothing, and
//! nothing crosses a workspace. Postgres runs under testcontainers (skipped
//! without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Room {
    channel: ChannelId,
    producer: MemberId,
    worker: MemberId,
    lead: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["producer", "worker", "lead"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        channel: channel.id,
        producer: ids[0],
        worker: ids[1],
        lead: ids[2],
    }
}

async fn task(store: &dyn Store, room: &Room, title: &str) -> ThreadId {
    store
        .create_thread_with_event(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: Some(room.producer),
        })
        .await
        .unwrap()
        .0
        .id
}

fn cancelled(outcome: CancelOutcome) -> (CancelledThread, Vec<StoredEvent>) {
    match outcome {
        CancelOutcome::Cancelled { cancelled, events } => (cancelled, events),
        other => panic!("expected a cancel, got {other:?}"),
    }
}

/// Every read that maps a thread row carries its creator.
async fn the_creator_is_recorded_and_read_back(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "recorded").await;
    let thread = task(store.as_ref(), &room, "made").await;
    assert_eq!(
        store.get_thread(thread).await.unwrap().created_by,
        Some(room.producer)
    );
    let listed = store.list_threads(room.channel).await.unwrap();
    assert_eq!(listed[0].created_by, Some(room.producer));
    let claimed = store
        .claim_next_thread(room.channel, room.worker, Some(600))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.created_by, Some(room.producer));
}

/// The creator cancels a task nobody holds: it closes, leaves the queue, and
/// says why on the thread.
async fn the_creator_cancels_an_unclaimed_task(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "unclaimed").await;
    let thread = task(store.as_ref(), &room, "not needed").await;

    let (done, events) = cancelled(
        store
            .cancel_thread(
                thread,
                room.producer,
                "superseded",
                CancelAuthority::Creator,
            )
            .await
            .unwrap(),
    );
    assert_eq!(done.thread.state, ThreadState::Closed);
    assert_eq!(done.cancellation.reason, "superseded");
    assert_eq!(done.cancellation.cancelled_by, room.producer);
    let kinds: Vec<EventKind> = events.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            EventKind::ThreadStateChanged,
            EventKind::ThreadCancelled,
            EventKind::MessagePosted
        ]
    );

    let parked = store.get_thread_unclaimable(thread).await.unwrap().unwrap();
    assert_eq!(parked.reason, "superseded");
    let messages = store.list_messages(thread, 10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "Cancelled: superseded");
    assert_eq!(messages[0].metadata["notice"], "cancelled");
    let transitions = store.list_thread_transitions(thread, 10).await.unwrap();
    assert_eq!(transitions.last().unwrap().to_state, ThreadState::Closed);
    assert!(store
        .claim_next_thread(room.channel, room.worker, Some(600))
        .await
        .unwrap()
        .is_none());
}

/// A cancel ends the live claim: the worker's next renew is refused and the
/// thread has no holder.
async fn a_cancel_ends_the_claim_so_the_holders_renew_is_refused(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "claimed").await;
    let thread = task(store.as_ref(), &room, "in progress").await;
    let held = store
        .claim_next_thread(room.channel, room.worker, Some(600))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.id, thread);
    let lease = held.claim_lease_id.unwrap();
    store
        .renew_claim(thread, room.worker, lease, 600)
        .await
        .expect("the holder renews before the cancel");

    let (done, _) = cancelled(
        store
            .cancel_thread(
                thread,
                room.producer,
                "plan changed",
                CancelAuthority::Creator,
            )
            .await
            .unwrap(),
    );
    assert_eq!(done.thread.assignee_id, None);
    assert_eq!(done.thread.claim_lease_id, None);
    assert!(matches!(
        store.renew_claim(thread, room.worker, lease, 600).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(store.get_thread(thread).await.unwrap().assignee_id, None);
}

/// Under the creator rule a member that did not create the task is refused
/// and the task is untouched; so is a task with no recorded creator.
async fn a_member_that_did_not_create_it_is_refused(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "refused").await;
    let thread = task(store.as_ref(), &room, "someone else's").await;
    assert!(matches!(
        store
            .cancel_thread(thread, room.worker, "mine now", CancelAuthority::Creator)
            .await
            .unwrap(),
        CancelOutcome::NotCreator
    ));
    let untouched = store.get_thread(thread).await.unwrap();
    assert_eq!(untouched.state, ThreadState::Open);
    assert!(store
        .get_thread_unclaimable(thread)
        .await
        .unwrap()
        .is_none());
    assert!(store.list_messages(thread, 10).await.unwrap().is_empty());

    let anonymous = store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some("system made".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        store
            .cancel_thread(
                anonymous.id,
                room.producer,
                "not yours",
                CancelAuthority::Creator
            )
            .await
            .unwrap(),
        CancelOutcome::NotCreator
    ));
}

/// A `thread:transition` holder may cancel a task it did not create, even one
/// whose close is gated on reviews: a cancel lands nothing.
async fn a_transition_holder_cancels_any_task(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "lead").await;
    let thread = task(store.as_ref(), &room, "gated").await;
    store.set_review_requirement(thread, 2).await.unwrap();
    let (done, _) = cancelled(
        store
            .cancel_thread(thread, room.lead, "dropped", CancelAuthority::Transition)
            .await
            .unwrap(),
    );
    assert_eq!(done.thread.state, ThreadState::Closed);
    assert_eq!(done.cancellation.cancelled_by, room.lead);
}

/// A repeat answers with the first cancel and writes nothing; a task that
/// closed some other way is not cancelled after the fact.
async fn a_repeat_changes_nothing_and_a_finished_task_stays_finished(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "repeat").await;
    let thread = task(store.as_ref(), &room, "twice").await;
    store
        .cancel_thread(thread, room.producer, "first", CancelAuthority::Creator)
        .await
        .unwrap();
    match store
        .cancel_thread(thread, room.producer, "second", CancelAuthority::Creator)
        .await
        .unwrap()
    {
        CancelOutcome::AlreadyCancelled(again) => {
            assert_eq!(again.cancellation.reason, "first");
            assert_eq!(again.thread.state, ThreadState::Closed);
        }
        other => panic!("expected the first cancel back, got {other:?}"),
    }
    assert_eq!(store.list_messages(thread, 10).await.unwrap().len(), 1);
    assert_eq!(
        store
            .get_thread_unclaimable(thread)
            .await
            .unwrap()
            .unwrap()
            .reason,
        "first"
    );

    let finished = task(store.as_ref(), &room, "done").await;
    for action in [ThreadAction::StartReview, ThreadAction::Close] {
        store
            .transition_thread(finished, room.lead, action)
            .await
            .unwrap();
    }
    assert!(matches!(
        store
            .cancel_thread(finished, room.producer, "late", CancelAuthority::Creator)
            .await
            .unwrap(),
        CancelOutcome::AlreadyFinished(ThreadState::Closed)
    ));
    assert!(store
        .get_thread_unclaimable(finished)
        .await
        .unwrap()
        .is_none());
}

/// Two tenants: another workspace's member is no creator of this task, and a
/// cancel in one workspace leaves the other's queue as it was.
async fn a_cancel_stays_in_its_workspace(store: Arc<dyn Store>) {
    let ours = room(store.as_ref(), "ours").await;
    let theirs = room(store.as_ref(), "theirs").await;
    let our_task = task(store.as_ref(), &ours, "ours").await;
    let their_task = task(store.as_ref(), &theirs, "theirs").await;

    assert!(matches!(
        store
            .cancel_thread(
                our_task,
                theirs.producer,
                "crossing",
                CancelAuthority::Creator
            )
            .await
            .unwrap(),
        CancelOutcome::NotCreator
    ));
    cancelled(
        store
            .cancel_thread(
                their_task,
                theirs.producer,
                "theirs",
                CancelAuthority::Creator,
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        store.get_thread(our_task).await.unwrap().state,
        ThreadState::Open
    );
    let claimed = store
        .claim_next_thread(ours.channel, ours.worker, Some(600))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, our_task);
}

/// A system notice (a cancel or rebase note) is posted through the plain
/// `post_message`, not the evented path; it must still float its thread in the
/// recently-active view, or the thread that just got news sinks below quieter
/// ones. A post moves only its own thread: another workspace's thread with the
/// same history keeps its clock.
async fn a_plain_post_bumps_only_its_own_threads_activity(store: Arc<dyn Store>) {
    let mine = room(store.as_ref(), "mine").await;
    let theirs = room(store.as_ref(), "theirs").await;
    let older = task(store.as_ref(), &mine, "older").await;
    let newer = task(store.as_ref(), &mine, "newer").await;
    let their_thread = task(store.as_ref(), &theirs, "theirs").await;
    let older_before = store.get_thread(older).await.unwrap().updated_at;
    let theirs_before = store.get_thread(their_thread).await.unwrap().updated_at;
    tokio::time::sleep(Duration::from_millis(20)).await;

    store
        .post_message(NewMessage {
            thread_id: older,
            author_id: mine.lead,
            body: "cancelled".into(),
            metadata: serde_json::json!({ "system": "cancelled" }),
            content: None,
        })
        .await
        .unwrap();

    let older_after = store.get_thread(older).await.unwrap().updated_at;
    assert!(
        older_after > older_before,
        "the post must bump its thread: {older_before} -> {older_after}"
    );
    let order: Vec<ThreadId> = store
        .list_recently_active_threads(mine.channel, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(
        order,
        vec![older, newer],
        "the posted-to thread floats first"
    );
    assert_eq!(
        store.get_thread(their_thread).await.unwrap().updated_at,
        theirs_before,
        "another workspace's thread keeps its clock"
    );
}

async fn sqlite() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::for_tests(pool))
}

async fn postgres() -> Option<(testcontainers::ContainerAsync<Postgres>, Arc<dyn Store>)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, Arc::new(PostgresStore::for_tests(pool))))
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
    the_creator_is_recorded_and_read_back,
    the_creator_cancels_an_unclaimed_task,
    a_cancel_ends_the_claim_so_the_holders_renew_is_refused,
    a_member_that_did_not_create_it_is_refused,
    a_transition_holder_cancels_any_task,
    a_repeat_changes_nothing_and_a_finished_task_stays_finished,
    a_cancel_stays_in_its_workspace,
    a_plain_post_bumps_only_its_own_threads_activity,
);
