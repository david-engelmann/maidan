//! Second-family review routing on both backends (S4): a change entering
//! review spawns a review child routed away from its author's provider
//! family, and `claim_next` refuses that child to a seat whose usage
//! evidence names the author's family.

use maidan_store::test_support::SetResultClaiming;
use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

/// One usage record: `reporter` used `model` on `thread`.
struct Usage {
    ws: Uuid,
    thread: Uuid,
    reporter: Uuid,
    model: &'static str,
}

/// The scenario, with `record` writing a usage record on the backend.
async fn run_suite<F, Fut>(store: &dyn Store, record: F)
where
    F: Fn(Usage) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let ws = store
        .create_workspace(NewWorkspace {
            name: "families".into(),
        })
        .await
        .expect("workspace")
        .id;
    let mut ids = Vec::new();
    for handle in ["author", "same", "other", "roamer"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member")
                .id,
        );
    }
    let (author, same, other, roamer) = (ids[0], ids[1], ids[2], ids[3]);
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let new = |title: &str| NewThread {
        channel_id: channel,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };

    // A change with no usage evidence spawns no review child.
    let unrouted = store
        .create_thread(new("unrouted"))
        .await
        .expect("thread")
        .id;
    store.claim_thread(unrouted, author).await.expect("claim");
    store
        .set_thread_result_claiming(unrouted, author, &json!({"status": "done"}))
        .await
        .expect("result");
    store
        .transition_thread(unrouted, author, ThreadAction::StartReview)
        .await
        .expect("start review");
    assert!(
        store
            .claim_next_thread(channel, other, None, None)
            .await
            .expect("claim next")
            .is_none(),
        "no family named, nothing routed"
    );

    // A second tenant, where `roamer` once reported under the author's
    // family: evidence filed in another workspace refuses nothing here.
    let elsewhere = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .expect("workspace")
        .id;
    let elsewhere_channel = store
        .create_channel(NewChannel {
            workspace_id: elsewhere,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let elsewhere_thread = store
        .create_thread(NewThread {
            channel_id: elsewhere_channel,
            parent_thread_id: None,
            title: Some("elsewhere".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    record(Usage {
        ws: elsewhere.0,
        thread: elsewhere_thread.0,
        reporter: roamer.0,
        model: "claude-opus-4",
    })
    .await;

    // A change written by an Anthropic seat. Other seats report on it too,
    // a different family last: only the claimant's own records name the
    // author's family.
    let change = store.create_thread(new("change")).await.expect("thread").id;
    store.claim_thread(change, author).await.expect("claim");
    for (reporter, model) in [
        (author, "claude-sonnet-4"),
        (same, "claude-opus-4"),
        (other, "gpt-5-codex"),
        (other, "o4-mini"),
    ] {
        record(Usage {
            ws: ws.0,
            thread: change.0,
            reporter: reporter.0,
            model,
        })
        .await;
    }
    store
        .set_thread_result_claiming(change, author, &json!({"status": "done"}))
        .await
        .expect("result");
    let (_, events) = store
        .transition_thread_with_event(change, author, ThreadAction::StartReview)
        .await
        .expect("start review");
    // The spawned review child's routing rides out with the transition so a
    // live subscriber sees it (#1572).
    let routed = events
        .iter()
        .map(|e| e.opened_event().expect("event"))
        .find_map(|e| match e {
            Event::ThreadAttributesSet {
                attributes,
                author_family,
                ..
            } => Some((attributes, author_family)),
            _ => None,
        })
        .expect("the review child's routing is published");
    assert_eq!(routed.0.kind, TaskKind::Review);
    assert_eq!(routed.0.review_family.as_deref(), Some("openai"));
    assert_eq!(routed.1.as_deref(), Some("anthropic"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.opened_event(), Ok(Event::ThreadCreated { .. }))),
        "the review child's creation is published"
    );

    for seat in [author, same] {
        assert!(
            store
                .claim_next_thread(channel, seat, None, None)
                .await
                .expect("claim next")
                .is_none(),
            "a seat of the author's family never gets the review"
        );
    }
    let review = store
        .claim_next_thread(channel, roamer, None, None)
        .await
        .expect("claim next")
        .expect("another tenant's evidence does not refuse the review");
    assert_eq!(review.parent_thread_id, Some(change));
    let attributes = store.get_thread_attributes(review.id).await.expect("attrs");
    assert_eq!(attributes.kind, TaskKind::Review);
    assert_eq!(attributes.review_family.as_deref(), Some("openai"));

    // Setting attributes leaves the server-set routing as it was.
    let restated = store
        .set_thread_attributes(
            review.id,
            TaskAttributes {
                kind: TaskKind::Review,
                ..TaskAttributes::default()
            },
        )
        .await
        .expect("set");
    assert_eq!(restated.review_family.as_deref(), Some("openai"));

    // With review children off, a change entering review spawns none, and
    // the other tenant keeps the default.
    assert_eq!(
        store.workspace_review_children(ws).await.expect("read"),
        ReviewChildren::Spawn
    );
    store
        .set_workspace_review_children(
            ws,
            ReviewChildren::Off,
            NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "workspace.review_children.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: json!({ "review_children": "off" }),
            },
        )
        .await
        .expect("set off");
    assert_eq!(
        store.workspace_review_children(ws).await.expect("read"),
        ReviewChildren::Off
    );
    assert_eq!(
        store
            .workspace_review_children(elsewhere)
            .await
            .expect("read"),
        ReviewChildren::Spawn,
        "one workspace's setting never reaches another"
    );
    let quiet = store.create_thread(new("quiet")).await.expect("thread").id;
    store.claim_thread(quiet, author).await.expect("claim");
    record(Usage {
        ws: ws.0,
        thread: quiet.0,
        reporter: author.0,
        model: "claude-sonnet-4",
    })
    .await;
    store
        .set_thread_result_claiming(quiet, author, &json!({"status": "done"}))
        .await
        .expect("result");
    store
        .transition_thread(quiet, author, ThreadAction::StartReview)
        .await
        .expect("start review");
    for seat in [other, roamer] {
        assert!(
            store
                .claim_next_thread(channel, seat, None, None)
                .await
                .expect("claim next")
                .is_none(),
            "with review children off no review task is spawned"
        );
    }
}

const INSERT_SQLITE: &str = "INSERT INTO maidan_usage_ledger
    (usage_report_id, workspace_id, thread_id, reporter_id, claim_lease_id, model,
     input_tokens, output_tokens, cache_read_tokens, cache_write_5m_tokens,
     input_price, output_price, cache_read_price, cache_write_5m_price, usd_micros, turns)
    VALUES (?, ?, ?, ?, ?, ?, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)";

const INSERT_POSTGRES: &str = "INSERT INTO maidan_usage_ledger
    (usage_report_id, workspace_id, thread_id, reporter_id, claim_lease_id, model,
     input_tokens, output_tokens, cache_read_tokens, cache_write_5m_tokens,
     input_price, output_price, cache_read_price, cache_write_5m_price, usd_micros, turns)
    VALUES ($1, $2, $3, $4, $5, $6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)";

#[tokio::test]
async fn a_review_is_routed_away_from_its_authors_family_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    run_suite(&store, |u| {
        let pool = pool.clone();
        async move {
            sqlx::query(INSERT_SQLITE)
                .bind(Uuid::now_v7())
                .bind(u.ws)
                .bind(u.thread)
                .bind(u.reporter)
                .bind(Uuid::now_v7())
                .bind(u.model)
                .execute(&pool)
                .await
                .expect("usage");
        }
    })
    .await;
}

#[tokio::test]
async fn a_review_is_routed_away_from_its_authors_family_on_postgres() {
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    run_suite(&store, |u| {
        let pool = pool.clone();
        async move {
            sqlx::query(INSERT_POSTGRES)
                .bind(Uuid::now_v7())
                .bind(u.ws)
                .bind(u.thread)
                .bind(u.reporter)
                .bind(Uuid::now_v7())
                .bind(u.model)
                .execute(&pool)
                .await
                .expect("usage");
        }
    })
    .await;
}
