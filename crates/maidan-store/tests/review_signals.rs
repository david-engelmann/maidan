//! Every review verdict appends a `ReviewSubmitted` event in the verdict's own
//! transaction, naming the verdict, the reviewer, the delegate that gave it and
//! the thread's last worker: the member who most recently took hold of it,
//! even after they let go. A review agent's critical finding, applied again for
//! the same result by the router, is not a second verdict. Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::attribution::with_attribution;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Attribution, ChannelId, DelegationGrantId, Event, EventKind, MemberId, MemberKind, NewChannel,
    NewMember, NewThread, NewWorkspace, ReviewDecision, StoredEvent, ThreadId, WorkspaceId,
    EXAMPLE_REVIEW_RESULT_KIND, REVIEW_SKILL, WAITER_RESULT_SCHEMA,
};
use serde_json::json;
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
    SqliteStore::for_tests(pool)
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

async fn channel(store: &dyn Store, ws: WorkspaceId) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("rs-{}", uuid::Uuid::now_v7()),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id
}

async fn thread(store: &dyn Store, channel: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
        })
        .await
        .expect("thread")
        .id
}

/// `worker` takes the channel's next thread and lets go of it, leaving it
/// `in_review` when `hand_in`, else `open`.
async fn work(store: &dyn Store, channel: ChannelId, worker: MemberId, hand_in: bool) -> ThreadId {
    let claimed = store
        .claim_next_thread(channel, worker, Some(60))
        .await
        .expect("claim")
        .expect("a claimable thread");
    if hand_in {
        store
            .transition_thread(claimed.id, worker, ThreadAction::StartReview)
            .await
            .expect("start review");
    }
    store
        .release_claim(claimed.id, worker, claimed.claim_lease_id.expect("leased"))
        .await
        .expect("release");
    claimed.id
}

struct Submitted {
    reviewer_id: MemberId,
    actor_id: Option<MemberId>,
    decision: ReviewDecision,
    sent_back: bool,
    worker_id: Option<MemberId>,
}

fn submitted(stored: &StoredEvent, ws: WorkspaceId, channel: ChannelId, t: ThreadId) -> Submitted {
    assert_eq!(stored.kind, EventKind::ReviewSubmitted);
    assert_eq!(stored.workspace_id, Some(ws));
    assert_eq!(stored.channel_id, Some(channel));
    assert_eq!(stored.thread_id, Some(t));
    match stored.opened_event().expect("event payload") {
        Event::ReviewSubmitted {
            workspace_id,
            channel_id,
            thread_id,
            reviewer_id,
            actor_id,
            decision,
            sent_back,
            worker_id,
            ..
        } => {
            assert_eq!((workspace_id, channel_id, thread_id), (ws, channel, t));
            Submitted {
                reviewer_id,
                actor_id,
                decision,
                sent_back,
                worker_id,
            }
        }
        other => panic!("expected ReviewSubmitted, got {other:?}"),
    }
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "rs".into() })
        .await
        .expect("ws")
        .id;
    let worker = member(store, ws, "worker").await;
    let second = member(store, ws, "second").await;
    let reviewer = member(store, ws, "reviewer").await;
    let orchestrator = member(store, ws, "orchestrator").await;

    // A change request that sends the work back: the verdict event first, then
    // the reopen, both in the log.
    let ch = channel(store, ws).await;
    thread(store, ch).await;
    let t = work(store, ch, worker, true).await;
    let sent = store
        .submit_review(
            t,
            reviewer,
            ReviewDecision::RequestChanges,
            Some("tests"),
            None,
        )
        .await
        .expect("request changes");
    let event = submitted(&sent.submitted, ws, ch, t);
    assert_eq!(event.reviewer_id, reviewer);
    assert_eq!(event.actor_id, None);
    assert_eq!(event.decision, ReviewDecision::RequestChanges);
    assert!(event.sent_back);
    assert_eq!(
        event.worker_id,
        Some(worker),
        "the worker let go at review, and is still the one whose work this was"
    );
    let reopened = sent.reopened.as_ref().expect("the thread went back");
    assert_eq!(reopened.kind, EventKind::ThreadStateChanged);
    assert!(sent.submitted.id < reopened.id, "the verdict comes first");
    let logged: Vec<EventKind> = store
        .list_thread_events_through(t, i64::MAX)
        .await
        .expect("thread events")
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(
        logged.ends_with(&[EventKind::ReviewSubmitted, EventKind::ThreadStateChanged]),
        "{logged:?}"
    );

    // An approval is a verdict too.
    let t = {
        thread(store, ch).await;
        work(store, ch, worker, true).await
    };
    let approved = store
        .submit_review(t, reviewer, ReviewDecision::Approve, None, None)
        .await
        .expect("approve");
    let event = submitted(&approved.submitted, ws, ch, t);
    assert_eq!(event.decision, ReviewDecision::Approve);
    assert!(!event.sent_back);
    assert!(approved.reopened.is_none());
    assert_eq!(event.worker_id, Some(worker));

    // A change request that sends nothing back is still announced.
    let recorded = store
        .submit_review(t, worker, ReviewDecision::RequestChanges, None, None)
        .await
        .expect("implementer's own change request");
    let event = submitted(&recorded.submitted, ws, ch, t);
    assert!(!event.sent_back);
    assert!(recorded.reopened.is_none());

    // The last worker is whoever took hold of it most recently, not the first.
    let ch = channel(store, ws).await;
    thread(store, ch).await;
    let t = work(store, ch, worker, false).await;
    assert_eq!(work(store, ch, second, true).await, t);
    let sent = store
        .submit_review(t, reviewer, ReviewDecision::RequestChanges, None, None)
        .await
        .unwrap();
    assert_eq!(
        submitted(&sent.submitted, ws, ch, t).worker_id,
        Some(second)
    );
    assert_eq!(work(store, ch, worker, true).await, t);
    let sent = store
        .submit_review(t, reviewer, ReviewDecision::RequestChanges, None, None)
        .await
        .unwrap();
    assert_eq!(
        submitted(&sent.submitted, ws, ch, t).worker_id,
        Some(worker),
        "taking it back puts the first worker last again"
    );

    // A delegate claiming for a member did the holding for it: the member is
    // the last worker. A delegate submitting a review is named beside the
    // reviewer.
    let ch = channel(store, ws).await;
    thread(store, ch).await;
    let claimed = with_attribution(
        Some(Attribution {
            actor_id: orchestrator,
            subject_id: second,
            grant_id: Some(DelegationGrantId(uuid::Uuid::now_v7())),
        }),
        store.claim_next_thread(ch, second, Some(60)),
    )
    .await
    .unwrap()
    .unwrap();
    let t = claimed.id;
    let sent = with_attribution(
        Some(Attribution {
            actor_id: orchestrator,
            subject_id: reviewer,
            grant_id: Some(DelegationGrantId(uuid::Uuid::now_v7())),
        }),
        store.submit_review(t, reviewer, ReviewDecision::RequestChanges, None, None),
    )
    .await
    .unwrap();
    let event = submitted(&sent.submitted, ws, ch, t);
    assert_eq!(event.worker_id, Some(second));
    assert_eq!(event.reviewer_id, reviewer);
    assert_eq!(event.actor_id, Some(orchestrator));

    // Nobody has held it: no worker to name.
    let ch = channel(store, ws).await;
    let t = thread(store, ch).await;
    let lone = store
        .submit_review(t, reviewer, ReviewDecision::Approve, None, None)
        .await
        .unwrap();
    assert_eq!(submitted(&lone.submitted, ws, ch, t).worker_id, None);

    // A review agent's critical finding is one verdict per result, however
    // often the router applies it; a new result is a new verdict.
    let bot = member(store, ws, "review-bot").await;
    store.add_member_skill(bot, REVIEW_SKILL).await.unwrap();
    let ch = channel(store, ws).await;
    thread(store, ch).await;
    let t = work(store, ch, worker, true).await;
    let critical = json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
        "status": "reviewed",
        "findings": [{ "severity": "critical" }],
    });
    store.set_thread_result(t, bot, &critical).await.unwrap();
    let first = store
        .apply_critical_review_decision(t, bot, &critical)
        .await
        .unwrap()
        .expect("the first application is a verdict");
    assert_eq!(
        submitted(&first.submitted, ws, ch, t).decision,
        ReviewDecision::RequestChanges
    );
    // The close-gate arms in the same commit as the verdict.
    assert_eq!(store.review_status(t).await.unwrap().required_count, 1);
    assert!(store
        .apply_critical_review_decision(t, bot, &critical)
        .await
        .unwrap()
        .is_none());
    assert_eq!(store.list_review_history(t).await.unwrap().len(), 1);
    assert_eq!(
        store.review_status(t).await.unwrap().required_count,
        1,
        "a replay that writes nothing does not clear the gate"
    );
    store.set_thread_result(t, bot, &critical).await.unwrap();
    assert!(
        store
            .apply_critical_review_decision(t, bot, &critical)
            .await
            .unwrap()
            .is_some(),
        "a re-review is a new verdict"
    );
    assert_eq!(store.list_review_history(t).await.unwrap().len(), 2);
}

#[tokio::test]
async fn every_review_verdict_appends_review_submitted_with_the_last_worker_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn every_review_verdict_appends_review_submitted_with_the_last_worker_postgres() {
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
