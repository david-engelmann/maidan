//! A change request sends work back. A `request_changes` review on a thread
//! under review, from its owner or from a reviewer whose approval would count,
//! returns the thread to `open`: it is claimable again, the approvals given to
//! the version it replaces stop counting, and the reopen is in the event log.
//! From the implementer, from outside a named reviewer set, or on a thread not
//! under review, the review is only recorded. Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, Event, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ReviewDecision, Thread, ThreadState, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
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

/// A thread the worker claimed, worked, handed to review and let go of, alone
/// on its own channel.
async fn in_review(store: &dyn Store, ws: WorkspaceId, worker: MemberId) -> (Thread, ChannelId) {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("rc-{}", uuid::Uuid::now_v7()),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .expect("thread");
    let claimed = store
        .claim_next_thread(channel, worker, Some(60))
        .await
        .expect("claim")
        .expect("the new thread is claimable");
    assert_eq!(claimed.id, thread.id);
    store
        .transition_thread(thread.id, worker, ThreadAction::StartReview)
        .await
        .expect("start review");
    store
        .release_claim(thread.id, worker, claimed.claim_lease_id.expect("leased"))
        .await
        .expect("release");
    (thread, channel)
}

async fn state(store: &dyn Store, thread: &Thread) -> ThreadState {
    store.get_thread(thread.id).await.expect("thread").state
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "rc".into() })
        .await
        .expect("ws")
        .id;
    let owner = member(store, ws, "owner").await;
    let worker = member(store, ws, "worker").await;
    let reviewer = member(store, ws, "reviewer").await;
    let outsider = member(store, ws, "outsider").await;

    // A reviewer's change request sends the work back.
    let (t, channel) = in_review(store, ws, worker).await;
    store.set_review_requirement(t.id, 1).await.unwrap();
    store
        .submit_review(t.id, outsider, ReviewDecision::Approve, None)
        .await
        .unwrap();
    assert_eq!(store.review_status(t.id).await.unwrap().approvals, 1);
    let (review, reopened) = store
        .submit_review(
            t.id,
            reviewer,
            ReviewDecision::RequestChanges,
            Some("handle the empty input"),
        )
        .await
        .expect("request changes");
    assert_eq!(review.decision, ReviewDecision::RequestChanges);
    let reopened = reopened.expect("the thread went back, so the reopen is logged");
    match serde_json::from_value::<Event>(reopened.payload.clone()).expect("event payload") {
        Event::ThreadStateChanged {
            thread_id,
            actor_id,
            from_state,
            to_state,
            ..
        } => {
            assert_eq!(thread_id, t.id);
            assert_eq!(actor_id, reviewer);
            assert_eq!(from_state, ThreadState::InReview);
            assert_eq!(to_state, ThreadState::Open);
        }
        other => panic!("expected ThreadStateChanged, got {other:?}"),
    }
    assert_eq!(state(store, &t).await, ThreadState::Open);

    // The approval of the old version no longer counts, and says why.
    assert_eq!(store.review_status(t.id).await.unwrap().approvals, 0);
    let reviews = store.list_reviews(t.id).await.unwrap();
    let stale = reviews.iter().find(|r| r.reviewer_id == outsider).unwrap();
    assert!(
        stale.dismissed_at.is_some(),
        "the old approval is dismissed"
    );
    let request = reviews.iter().find(|r| r.reviewer_id == reviewer).unwrap();
    assert!(request.dismissed_at.is_none(), "the change request stands");

    // The work is back in the queue, and the rework round needs a fresh approval.
    let again = store
        .claim_next_thread(channel, worker, Some(60))
        .await
        .unwrap()
        .expect("the reopened thread is claimable again");
    assert_eq!(again.id, t.id);
    store
        .transition_thread(t.id, worker, ThreadAction::StartReview)
        .await
        .unwrap();
    store
        .release_claim(t.id, worker, again.claim_lease_id.unwrap())
        .await
        .unwrap();
    assert!(
        store
            .transition_thread(t.id, owner, ThreadAction::Close)
            .await
            .is_err(),
        "a dismissed approval does not close the reworked thread"
    );
    store
        .submit_review(t.id, outsider, ReviewDecision::Approve, None)
        .await
        .unwrap();
    let renewed = store.list_reviews(t.id).await.unwrap();
    assert!(renewed
        .iter()
        .find(|r| r.reviewer_id == outsider)
        .unwrap()
        .dismissed_at
        .is_none());
    store
        .transition_thread(t.id, owner, ThreadAction::Close)
        .await
        .expect("a fresh approval closes it");

    // The implementer's own change request is recorded, not acted on.
    let (t, _) = in_review(store, ws, worker).await;
    let (_, reopened) = store
        .submit_review(t.id, worker, ReviewDecision::RequestChanges, None)
        .await
        .unwrap();
    assert!(reopened.is_none());
    assert_eq!(state(store, &t).await, ThreadState::InReview);

    // The owner may send it back, even with no requirement set.
    store.set_thread_owner(t.id, Some(owner)).await.unwrap();
    let (_, reopened) = store
        .submit_review(t.id, owner, ReviewDecision::RequestChanges, None)
        .await
        .unwrap();
    assert!(reopened.is_some());
    assert_eq!(state(store, &t).await, ThreadState::Open);

    // Nothing to send back on a thread that is not under review.
    let (_, reopened) = store
        .submit_review(t.id, reviewer, ReviewDecision::RequestChanges, None)
        .await
        .unwrap();
    assert!(reopened.is_none());
    assert_eq!(state(store, &t).await, ThreadState::Open);

    // With a named reviewer set, only its members (or the owner) send work back.
    let (t, _) = in_review(store, ws, worker).await;
    store.add_reviewer(t.id, reviewer).await.unwrap();
    let (_, reopened) = store
        .submit_review(t.id, outsider, ReviewDecision::RequestChanges, None)
        .await
        .unwrap();
    assert!(reopened.is_none(), "not a named reviewer");
    assert_eq!(state(store, &t).await, ThreadState::InReview);
    let (_, reopened) = store
        .submit_review(t.id, reviewer, ReviewDecision::RequestChanges, None)
        .await
        .unwrap();
    assert!(reopened.is_some(), "a named reviewer");
    assert_eq!(state(store, &t).await, ThreadState::Open);

    // An approval does not reopen anything.
    let (t, _) = in_review(store, ws, worker).await;
    let (_, reopened) = store
        .submit_review(t.id, reviewer, ReviewDecision::Approve, None)
        .await
        .unwrap();
    assert!(reopened.is_none());
    assert_eq!(state(store, &t).await, ThreadState::InReview);
}

#[tokio::test]
async fn a_change_request_sends_work_back_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_change_request_sends_work_back_postgres() {
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
            eprintln!("skipping: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
