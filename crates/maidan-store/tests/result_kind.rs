//! `result_kind` search facet: exact match on the namespaced string extracted
//! from a thread result. Both backends. Not a closed enum —
//! `example.review.result/1` is just a string, and a different producer kind is
//! equally first-class. Does not touch the in-channel closed list.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    result_kind_from_payload, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

const REVIEW: &str = "example.review.result/1";
const PLAN: &str = "example.plan.result/1";

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
        .create_workspace(NewWorkspace {
            name: "kinds".into(),
        })
        .await
        .expect("ws");
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .expect("other ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let other_member = store
        .create_member(NewMember {
            workspace_id: other_ws.id,
            handle: "other".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("other member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let other_ch = store
        .create_channel(NewChannel {
            workspace_id: other_ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("other ch");

    let review = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("review".into()),
        })
        .await
        .expect("review thread");
    // The authoritative fixture shape — schema is *not* required for the facet.
    let review_payload = json!({
        "schema": "maidan.waiter.result/1",
        "result_kind": REVIEW,
        "status": "reviewed",
        "summary": "two findings",
    });
    assert_eq!(result_kind_from_payload(&review_payload), Some(REVIEW));
    store
        .set_thread_result(review.id, member.id, &review_payload)
        .await
        .expect("set review");

    let plan = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("plan".into()),
        })
        .await
        .expect("plan thread");
    store
        .set_thread_result(
            plan.id,
            member.id,
            &json!({ "result_kind": PLAN, "status": "reviewed" }),
        )
        .await
        .expect("set plan");

    // A result with no result_kind is stored but not facetable under a kind.
    let bare = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("bare".into()),
        })
        .await
        .expect("bare thread");
    store
        .set_thread_result(bare.id, member.id, &json!({ "decision": "approved" }))
        .await
        .expect("set bare");

    // Same kind in a different workspace must not leak.
    let foreign = store
        .create_thread(NewThread {
            channel_id: other_ch.id,
            parent_thread_id: None,
            title: Some("foreign review".into()),
        })
        .await
        .expect("foreign thread");
    store
        .set_thread_result(
            foreign.id,
            other_member.id,
            &json!({ "result_kind": REVIEW }),
        )
        .await
        .expect("set foreign");

    let reviews = store
        .list_thread_results(ws.id, Some(REVIEW), 50)
        .await
        .expect("filter review");
    assert_eq!(reviews.len(), 1, "exact match, not every result");
    assert_eq!(reviews[0].thread_id, review.id);
    assert_eq!(reviews[0].result["result_kind"], REVIEW);

    let plans = store
        .list_thread_results(ws.id, Some(PLAN), 50)
        .await
        .expect("filter plan");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].thread_id, plan.id);

    // Prefix / sibling string is not a match — this is exact, not LIKE.
    assert!(store
        .list_thread_results(ws.id, Some("example.review.result"), 50)
        .await
        .expect("prefix")
        .is_empty());
    assert!(store
        .list_thread_results(ws.id, Some("example.review.result/10"), 50)
        .await
        .expect("sibling")
        .is_empty());
    assert!(store
        .list_thread_results(ws.id, Some("decision"), 50)
        .await
        .expect("old enum word")
        .is_empty());

    let all = store
        .list_thread_results(ws.id, None, 50)
        .await
        .expect("unfiltered");
    let all_ids: Vec<_> = all.iter().map(|r| r.thread_id).collect();
    assert!(all_ids.contains(&review.id));
    assert!(all_ids.contains(&plan.id));
    assert!(all_ids.contains(&bare.id), "unkinded results stay listable");
    assert!(
        !all_ids.contains(&foreign.id),
        "another workspace's result does not leak"
    );
    assert_eq!(all.len(), 3);

    // Empty / whitespace kind is the same as no filter.
    let empty_kind = store
        .list_thread_results(ws.id, Some("   "), 50)
        .await
        .expect("whitespace kind");
    assert_eq!(empty_kind.len(), 3);

    // Newest first: plan was written after review.
    assert_eq!(all[0].thread_id, bare.id);
    assert!(all[0].produced_at >= all[1].produced_at);

    // Limit is honored (and clamped — 0 still returns at least the newest).
    let one = store
        .list_thread_results(ws.id, None, 1)
        .await
        .expect("limit 1");
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].thread_id, bare.id);

    // Re-set updates the facet: the old kind disappears, the new one appears.
    store
        .set_thread_result(
            review.id,
            member.id,
            &json!({ "result_kind": PLAN, "status": "revised" }),
        )
        .await
        .expect("re-set kind");
    let reviews_after = store
        .list_thread_results(ws.id, Some(REVIEW), 50)
        .await
        .expect("review after rewrite");
    assert!(
        reviews_after.is_empty(),
        "rewriting the kind must drop the old facet"
    );
    let plans_after = store
        .list_thread_results(ws.id, Some(PLAN), 50)
        .await
        .expect("plan after rewrite");
    assert_eq!(plans_after.len(), 2, "review now facets as a plan");

    // Re-set without a kind clears the facet.
    store
        .set_thread_result(review.id, member.id, &json!({ "status": "cleared" }))
        .await
        .expect("clear kind");
    assert!(store
        .list_thread_results(ws.id, Some(PLAN), 50)
        .await
        .expect("plan after clear")
        .iter()
        .all(|r| r.thread_id != review.id));
}

#[tokio::test]
async fn list_thread_results_filters_by_namespaced_kind_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn list_thread_results_filters_by_namespaced_kind_postgres() {
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
