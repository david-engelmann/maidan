//! The reviews waiting on a member: threads under review that name the member
//! as a reviewer and lack their approval. Approving, a change request that
//! reopens the thread, closing it, or not being named all take it off the
//! list. A review that names nobody falls to its owner, or, with no owner, to
//! whoever the caller says may take ownerless reviews. A gated thread cannot
//! go to review without a result, and a close with no approval reads as
//! closed without review. Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision, Thread,
    ThreadId, WorkspaceId,
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

/// A task the worker claimed and handed to review, alone on its own channel.
async fn handed_to_review(store: &dyn Store, ws: WorkspaceId, worker: MemberId) -> ThreadId {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("rr-{}", uuid::Uuid::now_v7()),
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
            description: None,
        })
        .await
        .expect("thread");
    let claimed = store
        .claim_next_thread(channel, worker, Some(60))
        .await
        .expect("claim")
        .expect("claimable");
    store
        .transition_thread(thread.id, worker, ThreadAction::StartReview)
        .await
        .expect("start review");
    store
        .release_claim(thread.id, worker, claimed.claim_lease_id.expect("leased"))
        .await
        .expect("release");
    thread.id
}

async fn requested(store: &dyn Store, ws: WorkspaceId, m: MemberId) -> Vec<Thread> {
    store
        .list_review_requests(ws, m)
        .await
        .expect("review requests")
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "rr".into() })
        .await
        .expect("ws")
        .id;
    let other_ws = store
        .create_workspace(NewWorkspace { name: "rr2".into() })
        .await
        .expect("ws2")
        .id;
    let worker = member(store, ws, "worker").await;
    let human = member(store, ws, "human").await;
    let bystander = member(store, ws, "bystander").await;

    let a = handed_to_review(store, ws, worker).await;
    let b = handed_to_review(store, ws, worker).await;
    store.set_review_requirement(a, 1).await.expect("req a");
    store.add_reviewer(a, human).await.expect("name human on a");
    store.add_reviewer(b, human).await.expect("name human on b");

    assert_eq!(
        requested(store, ws, human)
            .await
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![a, b],
        "oldest first"
    );
    // Touching an older request (here a claim on it while it waits) must not
    // move it behind a newer one: the queue is ordered by when review began.
    store
        .claim_thread(a, worker)
        .await
        .expect("claim a while it waits");
    let after_touch = requested(store, ws, human).await;
    assert_eq!(
        after_touch.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![a, b],
        "still oldest review first after a is touched"
    );
    // The same moment ages the request: a touch bumps the row, but the inbox
    // reads when review began.
    let touched = after_touch.iter().find(|t| t.id == a).expect("a");
    let row = store.get_thread(a).await.expect("row");
    assert!(
        touched.updated_at < row.updated_at,
        "the request ages from review entry ({}), not the touched row ({})",
        touched.updated_at,
        row.updated_at
    );
    assert!(
        requested(store, ws, bystander).await.is_empty(),
        "a member nobody named is not asked"
    );
    assert!(
        requested(store, other_ws, human).await.is_empty(),
        "scoped to the workspace"
    );

    store
        .submit_review(a, human, ReviewDecision::Approve, None, None)
        .await
        .expect("approve a");
    assert_eq!(
        requested(store, ws, human)
            .await
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![b],
        "an approved review is no longer waiting"
    );

    store
        .submit_review(
            b,
            human,
            ReviewDecision::RequestChanges,
            Some("add a test"),
            None,
        )
        .await
        .expect("request changes on b");
    assert!(
        requested(store, ws, human).await.is_empty(),
        "a change request sends b back to open, so it is not in review"
    );

    // A change request from another reviewer dismisses the human's approval.
    // Once the worker hands the task back, the human is asked again: close
    // needs a fresh approval, so the inbox must say so.
    let d = handed_to_review(store, ws, worker).await;
    store.add_reviewer(d, human).await.expect("name human on d");
    store
        .add_reviewer(d, bystander)
        .await
        .expect("name bystander on d");
    store
        .submit_review(d, human, ReviewDecision::Approve, None, None)
        .await
        .expect("human approves d");
    assert!(!requested(store, ws, human).await.iter().any(|t| t.id == d));
    store
        .submit_review(
            d,
            bystander,
            ReviewDecision::RequestChanges,
            Some("rework"),
            None,
        )
        .await
        .expect("bystander requests changes on d");
    store.claim_thread(d, worker).await.expect("reclaim d");
    store
        .transition_thread(d, worker, ThreadAction::StartReview)
        .await
        .expect("back to review");
    assert_eq!(
        requested(store, ws, human)
            .await
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![d],
        "a dismissed approval does not count: the human is asked again"
    );
    store
        .submit_review(d, human, ReviewDecision::Approve, None, None)
        .await
        .expect("human approves d again");
    assert!(
        requested(store, ws, human).await.is_empty(),
        "the fresh approval takes it off the list"
    );

    let c = handed_to_review(store, ws, worker).await;
    store.add_reviewer(c, human).await.expect("name human on c");
    assert_eq!(
        requested(store, ws, human)
            .await
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![c]
    );
    store
        .transition_thread(c, human, ThreadAction::Close)
        .await
        .expect("close c");
    assert!(
        requested(store, ws, human).await.is_empty(),
        "a closed thread waits on nobody"
    );
}

async fn unassigned(
    store: &dyn Store,
    ws: WorkspaceId,
    m: MemberId,
    include_ownerless: bool,
) -> Vec<ThreadId> {
    store
        .list_unassigned_reviews(ws, m, include_ownerless)
        .await
        .expect("unassigned reviews")
        .iter()
        .map(|t| t.id)
        .collect()
}

async fn run_unassigned_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "ur".into() })
        .await
        .expect("ws")
        .id;
    let other_ws = store
        .create_workspace(NewWorkspace { name: "ur2".into() })
        .await
        .expect("ws2")
        .id;
    let worker = member(store, ws, "worker").await;
    let owner = member(store, ws, "owner").await;
    let admin = member(store, ws, "admin").await;
    let reviewer = member(store, ws, "reviewer").await;
    let foreign = member(store, other_ws, "foreign").await;
    let foreign_worker = member(store, other_ws, "foreign-worker").await;

    let owned = handed_to_review(store, ws, worker).await;
    store
        .set_thread_owner(owned, Some(owner))
        .await
        .expect("owner");
    let ownerless = handed_to_review(store, ws, worker).await;
    let named = handed_to_review(store, ws, worker).await;
    store.add_reviewer(named, reviewer).await.expect("named");
    let elsewhere = handed_to_review(store, other_ws, foreign_worker).await;

    assert_eq!(
        unassigned(store, ws, owner, false).await,
        vec![owned],
        "the owner hears about its own review nobody was named for"
    );
    assert_eq!(
        unassigned(store, ws, admin, true).await,
        vec![ownerless],
        "an ownerless review falls to whoever takes ownerless reviews"
    );
    assert!(
        unassigned(store, ws, admin, false).await.is_empty(),
        "a member who takes no ownerless reviews and owns nothing hears nothing"
    );
    assert!(
        !unassigned(store, ws, owner, true).await.contains(&named),
        "a review with a named reviewer reaches that reviewer instead"
    );
    assert!(
        !unassigned(store, ws, admin, true)
            .await
            .contains(&elsewhere),
        "another workspace's ownerless review never reaches this one"
    );
    assert_eq!(
        unassigned(store, other_ws, foreign, true).await,
        vec![elsewhere],
        "and that workspace's own admin hears it"
    );

    store
        .submit_review(ownerless, admin, ReviewDecision::Approve, None, None)
        .await
        .expect("admin approves");
    assert!(
        unassigned(store, ws, admin, true).await.is_empty(),
        "an approval the member gave answers it for them"
    );

    store
        .add_reviewer(owned, reviewer)
        .await
        .expect("name a reviewer");
    assert!(
        unassigned(store, ws, owner, false).await.is_empty(),
        "naming a reviewer moves it to that reviewer's requests"
    );
    assert_eq!(requested(store, ws, reviewer).await.len(), 2);
}

/// `start_review` on a thread whose close needs approvals is refused until a
/// result is posted. A thread with no requirement may go to review without one.
async fn run_result_gate_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "rg".into() })
        .await
        .expect("ws")
        .id;
    let worker = member(store, ws, "worker").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "rg".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let new_thread = |title: &'static str| NewThread {
        channel_id: channel,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
    };
    let gated = store
        .create_thread(new_thread("gated"))
        .await
        .expect("t")
        .id;
    store.set_review_requirement(gated, 1).await.expect("gate");
    let err = store
        .transition_thread(gated, worker, ThreadAction::StartReview)
        .await
        .expect_err("a gated thread with no result is refused");
    assert!(
        matches!(&err, maidan_store::StoreError::Conflict(m) if m.contains("no result posted")),
        "{err:?}"
    );
    assert_eq!(
        store.get_thread(gated).await.expect("row").state,
        maidan_types::ThreadState::Open,
        "the refusal changes nothing"
    );
    store
        .set_thread_result(gated, worker, &serde_json::json!({"status": "done"}))
        .await
        .expect("result");
    store
        .transition_thread(gated, worker, ThreadAction::StartReview)
        .await
        .expect("with a result it goes to review");

    let ungated = store
        .create_thread(new_thread("ungated"))
        .await
        .expect("t")
        .id;
    store
        .transition_thread(ungated, worker, ThreadAction::StartReview)
        .await
        .expect("no requirement, no result needed");
}

/// A close with no approval reads as closed without review on the reads a
/// board makes; an approved close and an open thread do not.
async fn run_closed_without_review_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "cwr".into() })
        .await
        .expect("ws")
        .id;
    let worker = member(store, ws, "worker").await;
    let human = member(store, ws, "human").await;
    let bare = handed_to_review(store, ws, worker).await;
    store
        .transition_thread(bare, human, ThreadAction::Close)
        .await
        .expect("an ungated close needs no approval");
    let approved = handed_to_review(store, ws, worker).await;
    store
        .submit_review(approved, human, ReviewDecision::Approve, None, None)
        .await
        .expect("approve");
    store
        .transition_thread(approved, human, ThreadAction::Close)
        .await
        .expect("close");
    let waiting = handed_to_review(store, ws, worker).await;

    assert!(
        store
            .get_thread(bare)
            .await
            .expect("bare")
            .closed_without_review
    );
    assert!(
        !store
            .get_thread(approved)
            .await
            .expect("approved")
            .closed_without_review
    );
    assert!(
        !store
            .get_thread(waiting)
            .await
            .expect("waiting")
            .closed_without_review
    );
    let page = store
        .page_threads_for_channel(
            store.get_thread(bare).await.expect("bare").channel_id,
            None,
            10,
        )
        .await
        .expect("page");
    assert_eq!(page.len(), 1);
    assert!(
        page[0].closed_without_review,
        "the channel page carries it too"
    );
    let json =
        serde_json::to_value(store.get_thread(approved).await.expect("approved")).expect("json");
    assert!(
        json.get("closed_without_review").is_none(),
        "the flag is left off the wire when false"
    );
}

#[tokio::test]
async fn review_requests_list_what_waits_on_a_named_reviewer_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    run_unassigned_suite(&store).await;
    run_result_gate_suite(&store).await;
    run_closed_without_review_suite(&store).await;
}

#[tokio::test]
async fn review_requests_list_what_waits_on_a_named_reviewer_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
    run_unassigned_suite(&store).await;
    run_result_gate_suite(&store).await;
    run_closed_without_review_suite(&store).await;
}
