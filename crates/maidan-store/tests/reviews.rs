//! Required-reviewers store (Cluster 375, Wave 2 #22): the requirement + named
//! reviewer set + decisions, and `review_status` — distinct qualifying approvals
//! (decision=approve, reviewer != owner/assignee, in the named set when one
//! exists). Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision};
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
        .submit_review(thread.id, r1.id, ReviewDecision::Approve, Some("lgtm"))
        .await
        .unwrap();
    assert_eq!(store.review_status(thread.id).await.unwrap().approvals, 1);
    store
        .submit_review(thread.id, owner.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    store
        .submit_review(thread.id, assignee.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    assert_eq!(
        store.review_status(thread.id).await.unwrap().approvals,
        1,
        "owner + assignee self-approvals don't count"
    );
    store
        .submit_review(thread.id, r2.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    let s = store.review_status(thread.id).await.unwrap();
    assert_eq!(s.approvals, 2);
    assert!(s.approvals_met, "2 of 2 → met");

    // r1 changes their mind → request_changes → back to 1 → not met.
    store
        .submit_review(thread.id, r1.id, ReviewDecision::RequestChanges, None)
        .await
        .unwrap();
    assert!(!store.review_status(thread.id).await.unwrap().approvals_met);
    // Flip back.
    store
        .submit_review(thread.id, r1.id, ReviewDecision::Approve, None)
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

#[tokio::test]
async fn reviews_requirement_named_set_and_sod_counting_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
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
            eprintln!("skipping: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
