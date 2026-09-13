//! Cluster 383.1: a reviewed `pi.review.result/1` with any `critical`
//! finding, submitted by a review-skilled member, upserts Cluster 375
//! `request_changes`. Wrong shape / no critical / no skill → no-op.
//! Does not set a requirement (383.2 arms `k`). Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision,
    CRITICAL_REVIEW_NOTE, PI_REVIEW_RESULT_KIND, REVIEW_SKILL, WAITER_RESULT_SCHEMA,
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
    let reviewer = mk("reviewer").await;
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
        .add_member_skill(reviewer.id, REVIEW_SKILL)
        .await
        .expect("review skill");

    let critical = envelope(PI_REVIEW_RESULT_KIND, "reviewed", &["warning", "critical"]);
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
        envelope(PI_REVIEW_RESULT_KIND, "reviewed", &["warning"]),
        envelope("pi.plan.result/1", "reviewed", &["critical"]),
        envelope(PI_REVIEW_RESULT_KIND, "failed", &["critical"]),
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

    // No requirement was armed — 383.2's job. approvals_met stays vacuously true.
    let status = store.review_status(thread.id).await.unwrap();
    assert_eq!(status.required_count, 0);
    assert!(
        status.approvals_met,
        "383.1 writes the decision the gate understands; it does not set k"
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
