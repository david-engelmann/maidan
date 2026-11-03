//! Required-reviewers store: the requirement + named reviewer set + decisions,
//! and `review_status` — distinct qualifying approvals (decision=approve,
//! reviewer != owner/assignee, in the named set when one exists). Both
//! backends.

use maidan_store::attribution::with_attribution;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Attribution, DelegationGrantId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ReviewDecision,
};

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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let mk = |handle: &'static str| {
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect(handle)
        }
    };
    let owner = mk("owner").await;
    let assignee = mk("assignee").await;
    let r1 = mk("r1").await;
    let r2 = mk("r2").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    // owner + assignee are excluded from counting (separation of duties).
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .expect("owner");
    store
        .assign_thread(thread.id, assignee.id)
        .await
        .expect("assign");

    // No requirement → vacuously met.
    let s = store.review_status(thread.id).await.unwrap();
    assert_eq!(s.required_count, 0);
    assert_eq!(s.approvals, 0);
    assert!(s.approvals_met);

    // Require 2 approvals.
    let req = store.set_review_requirement(thread.id, 2).await.unwrap();
    assert_eq!(req.required_count, 2);
    let s = store.review_status(thread.id).await.unwrap();
    assert_eq!(s.required_count, 2);
    assert!(!s.approvals_met, "0 of 2");

    // Open review (no named set): r1 approves → 1; owner + assignee approvals do
    // NOT count (SoD); r2 approves → 2 → met.
    store
        .submit_review(
            thread.id,
            r1.id,
            ReviewDecision::Approve,
            Some("lgtm"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(store.review_status(thread.id).await.unwrap().approvals, 1);
    store
        .submit_review(thread.id, owner.id, ReviewDecision::Approve, None, None)
        .await
        .unwrap();
    store
        .submit_review(thread.id, assignee.id, ReviewDecision::Approve, None, None)
        .await
        .unwrap();
    assert_eq!(
        store.review_status(thread.id).await.unwrap().approvals,
        1,
        "owner + assignee self-approvals don't count"
    );
    store
        .submit_review(thread.id, r2.id, ReviewDecision::Approve, None, None)
        .await
        .unwrap();
    let s = store.review_status(thread.id).await.unwrap();
    assert_eq!(s.approvals, 2);
    assert!(s.approvals_met, "2 of 2 → met");

    // r1 changes their mind → request_changes → back to 1 → not met.
    store
        .submit_review(thread.id, r1.id, ReviewDecision::RequestChanges, None, None)
        .await
        .unwrap();
    assert!(!store.review_status(thread.id).await.unwrap().approvals_met);
    // Flip back.
    store
        .submit_review(thread.id, r1.id, ReviewDecision::Approve, None, None)
        .await
        .unwrap();
    assert!(store.review_status(thread.id).await.unwrap().approvals_met);

    // Named set: naming r1 restricts counting to the named set — r2's approve no
    // longer counts, so 1 of 2 → not met until r2 is named too.
    assert!(store.add_reviewer(thread.id, r1.id).await.unwrap());
    let s = store.review_status(thread.id).await.unwrap();
    assert_eq!(s.approvals, 1, "only the named r1 counts now");
    assert!(!s.approvals_met);
    store.add_reviewer(thread.id, r2.id).await.unwrap();
    assert!(store.review_status(thread.id).await.unwrap().approvals_met);

    // Reviewer list + reviews list + remove-reviewer + clear-requirement.
    assert_eq!(store.list_reviewers(thread.id).await.unwrap().len(), 2);
    assert_eq!(store.list_reviews(thread.id).await.unwrap().len(), 4);
    assert!(store.remove_reviewer(thread.id, r2.id).await.unwrap());
    assert_eq!(store.review_status(thread.id).await.unwrap().approvals, 1);
    assert!(store
        .get_review_requirement(thread.id)
        .await
        .unwrap()
        .is_some());
    assert!(store.clear_review_requirement(thread.id).await.unwrap());
    let s = store.review_status(thread.id).await.unwrap();
    assert_eq!(s.required_count, 0);
    assert!(s.approvals_met, "no requirement → met");
}

/// One workspace whose thread requires `required` approvals, with an
/// operator and `n` reviewers. Returns (thread, operator, reviewers).
async fn review_tenant(
    store: &dyn Store,
    name: &str,
    required: i64,
    n: usize,
) -> (
    maidan_types::ThreadId,
    maidan_types::MemberId,
    Vec<maidan_types::MemberId>,
) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let mut members = Vec::new();
    for i in 0..=n {
        let m = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: format!("{name}-m{i}"),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .expect("member");
        members.push(m.id);
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    store
        .set_review_requirement(thread.id, required)
        .await
        .expect("req");
    let operator = members.remove(0);
    (thread.id, operator, members)
}

/// Approvals one member submits for several reviewers (through delegation)
/// count once toward the requirement, and a second tenant is unaffected
/// (#1714).
async fn one_submitter_counts_once(store: &dyn Store) {
    let (a_thread, a_operator, a_reviewers) = review_tenant(store, "a", 2, 3).await;
    let (b_thread, _, b_reviewers) = review_tenant(store, "b", 2, 2).await;
    for reviewer in &a_reviewers {
        with_attribution(
            Some(Attribution {
                actor_id: a_operator,
                subject_id: *reviewer,
                grant_id: Some(DelegationGrantId(uuid::Uuid::now_v7())),
            }),
            store.submit_review(a_thread, *reviewer, ReviewDecision::Approve, None, None),
        )
        .await
        .expect("delegated approve");
    }
    let s = store.review_status(a_thread).await.unwrap();
    assert_eq!(
        s.approvals, 1,
        "three approvals from one submitter count once"
    );
    assert!(!s.approvals_met);
    // The operator's own approval is the same person again.
    store
        .submit_review(a_thread, a_operator, ReviewDecision::Approve, None, None)
        .await
        .unwrap();
    assert_eq!(store.review_status(a_thread).await.unwrap().approvals, 1);

    for reviewer in &b_reviewers {
        store
            .submit_review(b_thread, *reviewer, ReviewDecision::Approve, None, None)
            .await
            .unwrap();
    }
    let s = store.review_status(b_thread).await.unwrap();
    assert_eq!(s.approvals, 2, "tenant B's distinct reviewers each count");
    assert!(s.approvals_met);
    assert_eq!(store.review_status(a_thread).await.unwrap().approvals, 1);
}

#[tokio::test]
async fn reviews_requirement_named_set_and_sod_counting_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    one_submitter_counts_once(&store).await;
}

#[tokio::test]
async fn reviews_requirement_named_set_and_sod_counting_postgres() {
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
    one_submitter_counts_once(&store).await;
}
