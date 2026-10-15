//! The channel thread page's order and filters, on both backends: newest
//! first reverses the keyset walk (cursor included), `since` bounds by
//! creation time, and `result_kind` / `status` filter in the query — so a
//! filtered page of size N holds N matches, not the matches that fell in
//! an unfiltered page of size N.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn assert_thread_page_filters(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "pagefilters".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    // Five threads in creation order. The second carries a review result
    // and a working declaration; the fourth a review result and a done
    // declaration; the fifth a summary result and no declaration.
    let mut ids = Vec::new();
    for title in ["one", "two", "three", "four", "five"] {
        let t = store
            .create_thread(NewThread {
                channel_id: ch.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
        ids.push(t.id);
    }
    let review = json!({ "result_kind": "example.review.result/1", "verdict": "ok" });
    let summary = json!({ "result_kind": "example.summary.result/1", "text": "done" });
    store
        .set_thread_result(ids[1], agent.id, &review)
        .await
        .unwrap();
    store
        .set_thread_result(ids[3], agent.id, &review)
        .await
        .unwrap();
    store
        .set_thread_result(ids[4], agent.id, &summary)
        .await
        .unwrap();
    for (id, status) in [
        (ids[1], DeclaredStatus::Working),
        (ids[3], DeclaredStatus::Done),
    ] {
        store.claim_thread(id, agent.id).await.unwrap();
        store
            .declare_thread_status(id, status, "note".into(), agent.id)
            .await
            .unwrap();
    }

    let page = |query: ThreadPageQuery| async move {
        store
            .page_threads_for_channel(ch.id, query)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
    };

    // The default walk is the creation order, unchanged.
    assert_eq!(page(ThreadPageQuery::unfiltered(None, 100)).await, ids);

    // Newest first is the exact reverse, and its cursor continues the
    // reversed walk rather than restarting or flipping direction.
    let mut reversed = ids.clone();
    reversed.reverse();
    let newest = ThreadPageQuery {
        newest_first: true,
        ..ThreadPageQuery::unfiltered(None, 100)
    };
    assert_eq!(page(newest).await, reversed);
    let first_two = page(ThreadPageQuery {
        newest_first: true,
        ..ThreadPageQuery::unfiltered(None, 2)
    })
    .await;
    assert_eq!(first_two, reversed[..2].to_vec());
    let rest = page(ThreadPageQuery {
        newest_first: true,
        ..ThreadPageQuery::unfiltered(first_two.last().copied(), 100)
    })
    .await;
    assert_eq!(rest, reversed[2..].to_vec());

    // `since` the middle thread's creation keeps it and everything after.
    let third = store.get_thread(ids[2]).await.unwrap();
    let since = page(ThreadPageQuery {
        since: Some(third.created_at),
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await;
    assert_eq!(since, ids[2..].to_vec());

    // `result_kind` matches the stored kind exactly, in either order.
    let reviews = page(ThreadPageQuery {
        result_kind: Some("example.review.result/1".into()),
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await;
    assert_eq!(reviews, vec![ids[1], ids[3]]);
    let reviews_newest = page(ThreadPageQuery {
        result_kind: Some("example.review.result/1".into()),
        newest_first: true,
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await;
    assert_eq!(reviews_newest, vec![ids[3], ids[1]]);
    // A filtered page fills with matches: limit 1 returns the first match,
    // and its cursor returns the second — not an empty page.
    let first_match = page(ThreadPageQuery {
        result_kind: Some("example.review.result/1".into()),
        ..ThreadPageQuery::unfiltered(None, 1)
    })
    .await;
    assert_eq!(first_match, vec![ids[1]]);
    let second_match = page(ThreadPageQuery {
        result_kind: Some("example.review.result/1".into()),
        ..ThreadPageQuery::unfiltered(first_match.last().copied(), 1)
    })
    .await;
    assert_eq!(second_match, vec![ids[3]]);
    // A kind nobody produced matches nothing.
    assert!(page(ThreadPageQuery {
        result_kind: Some("example.absent.result/1".into()),
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await
    .is_empty());

    // `status` matches the current declaration only.
    let working = page(ThreadPageQuery {
        status: Some(DeclaredStatus::Working),
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await;
    assert_eq!(working, vec![ids[1]]);
    let done = page(ThreadPageQuery {
        status: Some(DeclaredStatus::Done),
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await;
    assert_eq!(done, vec![ids[3]]);
    assert!(page(ThreadPageQuery {
        status: Some(DeclaredStatus::Blocked),
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await
    .is_empty());

    // Filters compose: the review threads that are also done.
    let composed = page(ThreadPageQuery {
        result_kind: Some("example.review.result/1".into()),
        status: Some(DeclaredStatus::Done),
        ..ThreadPageQuery::unfiltered(None, 100)
    })
    .await;
    assert_eq!(composed, vec![ids[3]]);
}

#[tokio::test]
async fn thread_page_filters_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    assert_thread_page_filters(&store).await;
}

#[tokio::test]
async fn thread_page_filters_postgres() {
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::for_tests(pool);
    assert_thread_page_filters(&store).await;
}
