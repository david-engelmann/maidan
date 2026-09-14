//! Cluster 383: a reviewed `example.review.result/1` with any `critical`
//! finding, submitted by a review-skilled member, upserts Cluster 375
//! `request_changes` and arms `k = 1` when no requirement exists. Wrong
//! shape / no critical / no skill → no-op. A human approve unblocks close.
//! Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision,
    CRITICAL_REVIEW_NOTE, EXAMPLE_REVIEW_RESULT_KIND, REVIEW_SKILL, WAITER_RESULT_SCHEMA,
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
    SqliteStore::new(pool)
}

fn envelope(kind: &str, status: &str, severities: &[&str]) -> serde_json::Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": kind,
        "status": status,
        "findings": severities.iter().map(|s| json!({ "severity": s })).collect::<Vec<_>>(),
    })
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
    let reviewer = mk("reviewer").await;
    let human = mk("human").await;
    let unskilled = mk("unskilled").await;
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
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .expect("owner");
    store
        .assign_thread(thread.id, assignee.id)
        .await
        .expect("assign");

    store
        .add_member_skill(reviewer.id, REVIEW_SKILL)
        .await
        .expect("review skill");

    let critical = envelope(
        EXAMPLE_REVIEW_RESULT_KIND,
        "reviewed",
        &["warning", "critical"],
    );
    let written = store
        .apply_critical_review_decision(thread.id, reviewer.id, &critical)
        .await
        .expect("apply")
        .expect("critical + review-skilled writes a decision");
    assert_eq!(written.reviewer_id, reviewer.id);
    assert_eq!(written.decision, ReviewDecision::RequestChanges);
    assert_eq!(written.note.as_deref(), Some(CRITICAL_REVIEW_NOTE));

    let listed = store.list_reviews(thread.id).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].decision, ReviewDecision::RequestChanges);

    // No skill → no-op, existing row untouched.
    let skipped = store
        .apply_critical_review_decision(thread.id, unskilled.id, &critical)
        .await
        .expect("unskilled");
    assert!(
        skipped.is_none(),
        "a member without the review skill is inert"
    );
    assert_eq!(store.list_reviews(thread.id).await.unwrap().len(), 1);

    // Warning-only / wrong kind / not reviewed → no-op, do not flip the row.
    for payload in [
        envelope(EXAMPLE_REVIEW_RESULT_KIND, "reviewed", &["warning"]),
        envelope("example.plan.result/1", "reviewed", &["critical"]),
        envelope(EXAMPLE_REVIEW_RESULT_KIND, "failed", &["critical"]),
    ] {
        let none = store
            .apply_critical_review_decision(thread.id, reviewer.id, &payload)
            .await
            .expect("no-op");
        assert!(none.is_none(), "expected no write for {payload}");
    }
    let still = store.list_reviews(thread.id).await.unwrap();
    assert_eq!(still.len(), 1);
    assert_eq!(still[0].decision, ReviewDecision::RequestChanges);

    // Re-apply upserts (same reviewer, same decision) — every-replica safe.
    let again = store
        .apply_critical_review_decision(thread.id, reviewer.id, &critical)
        .await
        .expect("re-apply")
        .expect("still writes");
    assert_eq!(again.decision, ReviewDecision::RequestChanges);
    assert_eq!(store.list_reviews(thread.id).await.unwrap().len(), 1);

    // 383.2: writing the decision arms k=1 so the existing close-gate refuses.
    let status = store.review_status(thread.id).await.unwrap();
    assert_eq!(status.required_count, 1);
    assert!(
        !status.approvals_met,
        "request_changes from the review agent is not a qualifying approve"
    );

    store
        .transition_thread(thread.id, owner.id, ThreadAction::StartReview)
        .await
        .expect("start review");
    let blocked = store
        .transition_thread(thread.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(blocked, Err(StoreError::Conflict(ref m)) if m.contains("review requirement")),
        "critical finding must block close, got {blocked:?}"
    );

    // A human who is neither owner nor assignee resolves; the agent does not
    // auto-approve. Owner/assignee approvals still do not count (SoD).
    store
        .submit_review(thread.id, owner.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    assert!(
        !store.review_status(thread.id).await.unwrap().approvals_met,
        "owner self-approve must not unblock"
    );
    store
        .submit_review(thread.id, human.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    assert!(store.review_status(thread.id).await.unwrap().approvals_met);
    let closed = store
        .transition_thread(thread.id, owner.id, ThreadAction::Close)
        .await
        .expect("human approve unblocks close");
    assert_eq!(closed.to_state.as_str(), "closed");

    // An existing k is left alone — a second thread already requiring 2
    // does not get silently collapsed to 1.
    let t2 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t2".into()),
        })
        .await
        .expect("t2");
    store.set_review_requirement(t2.id, 2).await.unwrap();
    store
        .apply_critical_review_decision(t2.id, reviewer.id, &critical)
        .await
        .expect("apply t2")
        .expect("writes");
    assert_eq!(
        store.review_status(t2.id).await.unwrap().required_count,
        2,
        "an existing requirement is not overwritten"
    );
}

#[tokio::test]
async fn critical_review_result_writes_request_changes_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn critical_review_result_writes_request_changes_postgres() {
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
