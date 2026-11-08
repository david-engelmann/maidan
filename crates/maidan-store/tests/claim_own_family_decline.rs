//! `claim_next` remembers an own-family decline, on both backends: a seat
//! with no usage evidence that releases a review with
//! `decline: own_family` is not handed that review again, while every other
//! member still is, and its other work is unaffected. Only the holder of the
//! live lease can record a decline, and a decline in one workspace touches
//! nothing in another.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const LEASE_SECS: i64 = 600;

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace")
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "reviews".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    Room { ws, channel }
}

async fn seat(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
        .id
}

/// A thread titled `title` (titles differ, or the fingerprint withholds the
/// later one as a duplicate), a review of an Anthropic-written change when
/// `review` is set.
async fn task(store: &dyn Store, channel: ChannelId, title: &str, review: bool) -> ThreadId {
    let id = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    if review {
        store
            .set_thread_attributes(
                id,
                TaskAttributes {
                    kind: TaskKind::Review,
                    author_family: Some("anthropic".into()),
                    ..TaskAttributes::default()
                },
            )
            .await
            .expect("review attributes");
    }
    id
}

async fn next(store: &dyn Store, channel: ChannelId, member: MemberId) -> Option<Thread> {
    store
        .claim_next_thread(channel, member, Some(LEASE_SECS), None)
        .await
        .expect("claim next")
}

fn lease(thread: &Thread) -> ClaimLeaseId {
    thread.claim_lease_id.expect("a leased claim")
}

async fn run_suite(store: &dyn Store) {
    let w = room(store, "reviews").await;
    // `fresh` has reported no usage, so the usage-evidence guard cannot tell
    // its family: only its decline keeps the review away from it.
    let fresh = seat(store, w.ws, "fresh").await;
    let other = seat(store, w.ws, "other").await;
    let older = task(store, w.channel, "older review", true).await;
    let newer = task(store, w.channel, "newer review", true).await;

    // The head-of-line block this fixes: a plain release hands the same
    // oldest review straight back.
    let held = next(store, w.channel, fresh).await.expect("a review");
    assert_eq!(held.id, older);
    store
        .release_claim_with_event(older, fresh, lease(&held), None)
        .await
        .expect("plain release");
    let held = next(store, w.channel, fresh).await.expect("a review");
    assert_eq!(held.id, older, "a plain release does not stick");

    let (released, event) = store
        .release_claim_with_event(older, fresh, lease(&held), Some(ClaimDecline::OwnFamily))
        .await
        .expect("release with a decline");
    assert_eq!(released.assignee_id, None, "a decline still releases");
    assert_eq!(released.claim_lease_id, None);
    match event.opened_event().expect("event") {
        Event::ThreadAssignmentChanged { note, .. } => assert_eq!(
            note.as_deref(),
            Some(ClaimDecline::OwnFamily.note()),
            "the event log says why the claimant let go"
        ),
        unexpected => panic!("expected an assignment change, got {unexpected:?}"),
    }

    // The decline holds for that review only: the next eligible one comes.
    let held = next(store, w.channel, fresh)
        .await
        .expect("the next review");
    assert_eq!(
        held.id, newer,
        "the declined review is skipped, not the queue"
    );
    store
        .release_claim_with_event(newer, fresh, lease(&held), Some(ClaimDecline::OwnFamily))
        .await
        .expect("decline the second review");
    assert!(
        next(store, w.channel, fresh).await.is_none(),
        "every review it declined is withheld from it"
    );
    let (across, _) = store
        .claim_next_workspace_thread_with_event(w.ws, fresh, Some(LEASE_SECS), None, None, None)
        .await
        .expect("claim next across the workspace");
    assert!(across.is_none(), "the workspace scope withholds them too");

    // Every other member is still handed the review.
    let taken = next(store, w.channel, other)
        .await
        .expect("another member takes it");
    assert_eq!(taken.id, older);

    // A second workspace: its seat cannot record a decline on a review it
    // does not hold, even presenting the holder's live lease, and the first
    // workspace's declines do not reach its own queue.
    let w2 = room(store, "elsewhere").await;
    let stranger = seat(store, w2.ws, "fresh").await;
    let theirs = task(store, w2.channel, "their review", true).await;
    let refused = store
        .release_claim_with_event(
            older,
            stranger,
            lease(&taken),
            Some(ClaimDecline::OwnFamily),
        )
        .await;
    assert!(
        matches!(refused, Err(StoreError::NotFound)),
        "only the holder records a decline, got {refused:?}"
    );
    let still = store.get_thread(older).await.expect("thread");
    assert_eq!(still.assignee_id, Some(other), "the holder keeps it");
    assert_eq!(still.claim_lease_id, taken.claim_lease_id);
    let got = next(store, w2.channel, stranger)
        .await
        .expect("its own review");
    assert_eq!(got.id, theirs);
    store
        .release_claim_with_event(older, other, lease(&taken), None)
        .await
        .expect("the holder releases");
    let again = next(store, w.channel, other)
        .await
        .expect("a stranger's refused decline recorded nothing");
    assert_eq!(again.id, older);
    store
        .release_claim_with_event(older, other, lease(&again), None)
        .await
        .expect("release");

    // A thread that names no author family cannot be declined as one: the
    // refusal releases nothing.
    let plain = task(store, w.channel, "plain work", false).await;
    let held = next(store, w.channel, fresh)
        .await
        .expect("its other work is unaffected");
    assert_eq!(held.id, plain);
    // A token that does not hold the claim learns only that, as a plain
    // release does, before anything about the thread's attributes.
    let stale = store
        .release_claim_with_event(
            plain,
            fresh,
            ClaimLeaseId::new(),
            Some(ClaimDecline::OwnFamily),
        )
        .await;
    assert!(
        matches!(stale, Err(StoreError::NotFound)),
        "a stale token is refused as not found, got {stale:?}"
    );
    let refused = store
        .release_claim_with_event(plain, fresh, lease(&held), Some(ClaimDecline::OwnFamily))
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "no author family, no own-family decline, got {refused:?}"
    );
    let kept = store.get_thread(plain).await.expect("thread");
    assert_eq!(kept.assignee_id, Some(fresh), "nothing was released");
    assert_eq!(kept.claim_lease_id, held.claim_lease_id);
    store
        .release_claim_with_event(plain, fresh, lease(&held), None)
        .await
        .expect("plain release");

    // A change that names an author family is still not a review.
    let change = task(store, w.channel, "a change", false).await;
    store
        .set_thread_attributes(
            change,
            TaskAttributes {
                kind: TaskKind::Change,
                author_family: Some("anthropic".into()),
                ..TaskAttributes::default()
            },
        )
        .await
        .expect("change attributes");
    let direct = store
        .claim_thread(change, fresh)
        .await
        .expect("direct claim");
    let refused = store
        .release_claim_with_event(
            change,
            fresh,
            lease(&direct.thread),
            Some(ClaimDecline::OwnFamily),
        )
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "only a review takes an own-family decline, got {refused:?}"
    );
    store
        .release_claim_with_event(change, fresh, lease(&direct.thread), None)
        .await
        .expect("plain release");

    // A direct claim is the caller's explicit choice and is not refused; a
    // second decline of the same review keeps one row.
    let direct = store
        .claim_thread(older, fresh)
        .await
        .expect("direct claim");
    assert!(direct.claimed);
    store
        .release_claim_with_event(
            older,
            fresh,
            lease(&direct.thread),
            Some(ClaimDecline::OwnFamily),
        )
        .await
        .expect("a repeat decline");
    let held = next(store, w.channel, fresh)
        .await
        .expect("its undeclined work is left for it");
    assert_eq!(held.id, plain, "the oldest thread it did not decline");
}

#[tokio::test]
async fn claim_next_remembers_an_own_family_decline_on_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn claim_next_remembers_an_own_family_decline_on_postgres() {
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}
