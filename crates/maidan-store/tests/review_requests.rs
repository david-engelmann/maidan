//! The reviews waiting on a member: threads under review that name the member
//! as a reviewer and lack their approval. Approving, a change request that
//! reopens the thread, closing it, or not being named all take it off the
//! list. Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision, ThreadId,
    WorkspaceId,
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

async fn requested(store: &dyn Store, ws: WorkspaceId, m: MemberId) -> Vec<ThreadId> {
    store
        .list_review_requests(ws, m)
        .await
        .expect("review requests")
        .into_iter()
        .map(|t| t.id)
        .collect()
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
        requested(store, ws, human).await,
        vec![a, b],
        "oldest first"
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
        .submit_review(a, human, ReviewDecision::Approve, None)
        .await
        .expect("approve a");
    assert_eq!(
        requested(store, ws, human).await,
        vec![b],
        "an approved review is no longer waiting"
    );

    store
        .submit_review(b, human, ReviewDecision::RequestChanges, Some("add a test"))
        .await
        .expect("request changes on b");
    assert!(
        requested(store, ws, human).await.is_empty(),
        "a change request sends b back to open, so it is not in review"
    );

    let c = handed_to_review(store, ws, worker).await;
    store.add_reviewer(c, human).await.expect("name human on c");
    assert_eq!(requested(store, ws, human).await, vec![c]);
    store
        .transition_thread(c, human, ThreadAction::Close)
        .await
        .expect("close c");
    assert!(
        requested(store, ws, human).await.is_empty(),
        "a closed thread waits on nobody"
    );
}

#[tokio::test]
async fn review_requests_list_what_waits_on_a_named_reviewer_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
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
}
