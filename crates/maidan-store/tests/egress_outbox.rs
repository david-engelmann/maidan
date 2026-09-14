//! Durable projector egress outbox (Cluster 377.1): enqueue (deduped on the source
//! event + target) / atomic-lease-claim / mark delivered / reschedule-or-dead-letter
//! / DLQ count. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressKind, EgressTarget, NewChannel, NewEgressOutbox, NewThread, NewWorkspace, ThreadId,
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
    SqliteStore::new(pool)
}

fn slack(ws: WorkspaceId, thread: ThreadId, log_id: i64, body: &str) -> NewEgressOutbox {
    NewEgressOutbox {
        workspace_id: ws,
        thread_id: thread,
        source_log_id: log_id,
        target: EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into(),
        },
        body: body.into(),
        kind: EgressKind::Projector,
    }
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "egress".into(),
        })
        .await
        .expect("ws");
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
            title: Some("egress".into()),
        })
        .await
        .expect("thread");

    // Enqueue, then claim: leased forward + attempts -> 1, destination intact.
    let id = store
        .enqueue_egress(slack(ws.id, thread.id, 1, "hello"))
        .await
        .expect("enqueue")
        .expect("inserted");
    let claimed = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("some");
    assert_eq!(claimed.id, id);
    assert_eq!(claimed.workspace_id, ws.id);
    assert_eq!(claimed.thread_id, thread.id);
    assert_eq!(claimed.body, "hello");
    assert_eq!(claimed.attempts, 1);
    assert_eq!(
        claimed.target(),
        Some(EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into()
        }),
        "the stored (surface, selector) pair decodes back to the enqueued target"
    );

    // Re-claiming immediately finds nothing — the row is leased 300s forward.
    assert!(store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim2")
        .is_none());

    // A failure with a past retry_at reschedules it -> claimable again, attempts -> 2.
    store
        .mark_egress_failed(id, "slack 502", Some(Utc::now() - Duration::seconds(1)))
        .await
        .expect("fail-retry");
    let again = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim3")
        .expect("some3");
    assert_eq!(again.id, id);
    assert_eq!(again.attempts, 2);

    // Dead-letter it (retry_at None) — no longer claimable, DLQ depth 1.
    assert_eq!(store.count_dead_egress().await.expect("count0"), 0);
    store
        .mark_egress_failed(id, "gave up", None)
        .await
        .expect("dead");
    assert!(store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim4")
        .is_none());
    assert_eq!(store.count_dead_egress().await.expect("count1"), 1);

    // A GitHub delivery from a different source event is queued and delivered
    // cleanly: not re-claimed afterwards, and it doesn't grow the DLQ.
    let gh = store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ws.id,
            thread_id: thread.id,
            source_log_id: 2,
            target: EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 3915,
            },
            body: "review posted".into(),
            kind: EgressKind::Projector,
        })
        .await
        .expect("enqueue gh")
        .expect("inserted gh");
    let claimed_gh = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim5")
        .expect("some5");
    assert_eq!(claimed_gh.id, gh);
    assert_eq!(
        claimed_gh.target(),
        Some(EgressTarget::Github {
            repo: "example/repo".into(),
            issue_number: 3915
        })
    );
    store.mark_egress_delivered(gh).await.expect("delivered");
    assert!(store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim6")
        .is_none());
    assert_eq!(store.count_dead_egress().await.expect("count2"), 1);

    // DLQ ops (Cluster 377.4): the dead entry is listed with its destination and
    // last error, then requeued -> pending + due, no longer dead + claimable.
    let dead = store.list_dead_egress(10).await.expect("list dead");
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].id, id);
    assert_eq!(dead[0].surface, "slack");
    assert_eq!(dead[0].selector, "C0123ABCDEF");
    assert_eq!(dead[0].thread_id, thread.id);
    assert_eq!(dead[0].last_error.as_deref(), Some("gave up"));
    assert!(store.requeue_dead_egress(id).await.expect("requeue"));
    assert_eq!(store.count_dead_egress().await.expect("count3"), 0);
    let reclaimed = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim7")
        .expect("requeued is claimable");
    assert_eq!(reclaimed.id, id);
    assert_eq!(reclaimed.attempts, 1, "requeue reset attempts (claim -> 1)");
    assert!(
        !store.requeue_dead_egress(id).await.expect("requeue2"),
        "requeue only affects a dead entry"
    );
    // Leave the queue empty for the dedup suite that follows.
    store.mark_egress_delivered(id).await.expect("drain");
}

/// Every replica runs the router that enqueues, so the same event reaches each of
/// them. `(source_log_id, surface, selector)` is unique: the second enqueue is a
/// no-op, and only one delivery is ever claimable.
async fn run_dedup_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "egress-dedup".into(),
        })
        .await
        .expect("ws");
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
            title: Some("dedup".into()),
        })
        .await
        .expect("thread");

    let first = store
        .enqueue_egress(slack(ws.id, thread.id, 77, "once"))
        .await
        .expect("enqueue")
        .expect("inserted");
    assert!(
        store
            .enqueue_egress(slack(ws.id, thread.id, 77, "once"))
            .await
            .expect("enqueue again")
            .is_none(),
        "a second replica's enqueue of the same (event, target) is deduped"
    );

    let claimed = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("some");
    assert_eq!(claimed.id, first);
    store.mark_egress_delivered(first).await.expect("delivered");
    assert!(
        store
            .claim_next_due_egress(Utc::now(), 300)
            .await
            .expect("claim2")
            .is_none(),
        "the duplicate never became a second delivery"
    );

    // A different target for the same event is a different delivery — fan-out to
    // both a Slack channel and a GitHub PR is two rows, not one.
    assert!(store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ws.id,
            thread_id: thread.id,
            source_log_id: 77,
            target: EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 1,
            },
            body: "once".into(),
            kind: EgressKind::Projector,
        })
        .await
        .expect("enqueue other surface")
        .is_some());
    assert!(store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim3")
        .is_some());

    // Kind round-trips through claim so the worker can tell a result row from
    // a projector row (Cluster 379.4).
    store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ws.id,
            thread_id: thread.id,
            source_log_id: 100,
            target: EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 2,
            },
            body: "the result".into(),
            kind: EgressKind::Result,
        })
        .await
        .expect("enqueue result kind")
        .expect("inserted result kind");
    let claimed = store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim result kind")
        .expect("claimed result kind");
    assert_eq!(claimed.kind, EgressKind::Result);
    assert_eq!(claimed.body, "the result");
}

#[tokio::test]
async fn egress_outbox_enqueue_claim_retry_deadletter_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    run_dedup_suite(&store).await;
}

#[tokio::test]
async fn egress_outbox_enqueue_claim_retry_deadletter_postgres() {
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
    run_dedup_suite(&store).await;
}
