//! A request's trace is stored on the event it writes, and copied onto the
//! webhook and egress rows that fan that event out. A write with no trace
//! stores none. The content hash ignores the trace.

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressKind, EgressTarget, NewChannel, NewEgressOutbox, NewThread, NewWebhookSubscription,
    NewWorkspace, TraceContext, Workspace, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

fn parent() -> TraceContext {
    TraceContext::parse(
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        Some("vendor=one"),
    )
    .expect("traceparent")
}

fn workspace_created(name: &str) -> maidan_types::Event {
    maidan_types::Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: name.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    }
}

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

async fn run_suite(store: &impl Store) {
    let trace = parent();
    let event = workspace_created("traced");
    let stored = maidan_store::trace::scope(trace.clone(), store.append_event(&event))
        .await
        .expect("append");
    assert_eq!(
        stored.trace.as_ref().map(TraceContext::traceparent),
        Some(trace.traceparent())
    );
    assert_eq!(
        stored.trace.as_ref().and_then(TraceContext::tracestate),
        Some("vendor=one")
    );
    let read = store.get_stored_event(stored.id).await.expect("read");
    assert_eq!(read.trace, stored.trace);

    let other = TraceContext::root();
    let again = maidan_store::trace::scope(other.clone(), store.append_event(&event))
        .await
        .expect("append other");
    assert_eq!(again.content_hash, stored.content_hash);
    assert_ne!(again.trace, stored.trace);

    let bare = store
        .append_event(&workspace_created("bare"))
        .await
        .expect("bare");
    assert!(bare.trace.is_none());

    let ws = store
        .create_workspace(NewWorkspace {
            name: "trace".into(),
        })
        .await
        .expect("ws");
    let sub = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws.id,
            url: "https://example.test/hook".into(),
            label: None,
            event_kinds: vec![],
            secret_ciphertext: "sealed".into(),
        })
        .await
        .expect("sub");
    store
        .enqueue_webhook_delivery(sub.id, stored.id, "{}")
        .await
        .expect("enqueue hook");
    let deliveries = store
        .list_pending_webhook_deliveries(20)
        .await
        .expect("pending hooks");
    let delivery = deliveries
        .iter()
        .find(|row| row.log_id == stored.id)
        .expect("delivery");
    assert_eq!(delivery.trace, stored.trace);

    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("trace".into()),
        })
        .await
        .expect("thread");
    store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ws.id,
            thread_id: thread.id,
            source_log_id: stored.id,
            target: EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
            },
            body: "hello".into(),
            kind: EgressKind::Projector,
        })
        .await
        .expect("enqueue egress")
        .expect("inserted");
    let claimed = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("row");
    assert_eq!(claimed.thread_id, thread.id);
    assert_eq!(claimed.trace, stored.trace);
}

#[tokio::test]
async fn a_request_trace_is_stored_and_copied_onto_fanout_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_request_trace_is_stored_and_copied_onto_fanout_postgres() {
    use maidan_store::run_postgres_migrations;
    use std::time::Duration;

    use sqlx::postgres::PgPoolOptions;
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
