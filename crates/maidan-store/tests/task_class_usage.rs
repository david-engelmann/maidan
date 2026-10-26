//! `task_class_usage`: a workspace's completed usage reports grouped by
//! thread, model and effort, with each thread's kind, size and completion,
//! and never another workspace's. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewThread, NewUsageLedgerEntry, NewWorkspace, PriceSnapshot,
    TaskAttributes, TaskSize, ThreadId, TokenUsage, UsageEvidence, WorkspaceId,
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

struct Ws {
    id: WorkspaceId,
    member: maidan_types::MemberId,
    channel: maidan_types::ChannelId,
}

async fn workspace(store: &dyn Store, name: &str) -> Ws {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-agent"),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: format!("{name}-work"),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    Ws {
        id: ws.id,
        member: member.id,
        channel: channel.id,
    }
}

/// A sized task that reports one call per `(model, effort, usd)` and is
/// closed when `close` is set.
async fn task(
    store: &dyn Store,
    ws: &Ws,
    size: TaskSize,
    calls: &[(&str, Option<&str>, i64)],
    close: bool,
) -> ThreadId {
    let thread = store
        .create_thread(NewThread {
            channel_id: ws.channel,
            parent_thread_id: None,
            title: Some(format!("classed task {}", uuid::Uuid::now_v7())),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    store
        .set_thread_attributes(
            thread.id,
            TaskAttributes {
                size: Some(size),
                ..Default::default()
            },
        )
        .await
        .expect("attributes");
    let claimed = store
        .claim_thread(thread.id, ws.member)
        .await
        .expect("claim");
    let lease = claimed.thread.claim_lease_id.expect("claim lease");
    for (model, effort, usd) in calls {
        store
            .report_accounted_usage(&NewUsageLedgerEntry {
                usage_report_id: uuid::Uuid::now_v7(),
                thread_id: thread.id,
                reporter: ws.member,
                claim_lease_id: lease,
                model: (*model).into(),
                tokens: TokenUsage {
                    input: 1,
                    ..Default::default()
                },
                usd_micros: *usd,
                price_snapshot: PriceSnapshot {
                    input_usd_micros_per_million: *usd * 1_000_000,
                    ..Default::default()
                },
                turns: 1,
                evidence: UsageEvidence {
                    effort: effort.map(str::to_owned),
                    ..Default::default()
                },
            })
            .await
            .expect("report");
    }
    if close {
        store
            .transition_thread(thread.id, ws.member, maidan_fsm::ThreadAction::StartReview)
            .await
            .expect("review");
        store
            .transition_thread(thread.id, ws.member, maidan_fsm::ThreadAction::Close)
            .await
            .expect("close");
    }
    thread.id
}

async fn run_suite(store: &dyn Store) {
    let a = workspace(store, "class-a").await;
    let b = workspace(store, "class-b").await;
    let done = task(
        store,
        &a,
        TaskSize::S,
        &[("small", Some("low"), 40), ("small", Some("low"), 2)],
        true,
    )
    .await;
    let open = task(store, &a, TaskSize::L, &[("big", None, 90)], false).await;
    let switched = task(
        store,
        &a,
        TaskSize::S,
        &[("big", Some("high"), 5), ("big", Some("low"), 7)],
        true,
    )
    .await;
    task(
        store,
        &b,
        TaskSize::S,
        &[("other", Some("low"), 1000)],
        true,
    )
    .await;

    let rows = store.task_class_usage(a.id).await.expect("rows a");
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(
        rows.iter().all(|r| r.model != "other"),
        "B's reports stay out"
    );

    let d: Vec<_> = rows.iter().filter(|r| r.thread_id == done).collect();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, "change");
    assert_eq!(d[0].size.as_deref(), Some("S"));
    assert!(d[0].completed);
    assert_eq!(d[0].effort.as_deref(), Some("low"));
    assert_eq!(d[0].reports, 2);
    assert_eq!(d[0].usd_micros, 42);

    let o: Vec<_> = rows.iter().filter(|r| r.thread_id == open).collect();
    assert_eq!(o.len(), 1);
    assert!(!o[0].completed);
    assert_eq!(o[0].effort, None);
    assert_eq!(o[0].size.as_deref(), Some("L"));

    assert_eq!(rows.iter().filter(|r| r.thread_id == switched).count(), 2);

    let routing = maidan_types::TaskClassRouting::from_rows(a.id, 1, &rows).expect("routing");
    let small = routing
        .classes
        .iter()
        .find(|c| c.size.as_deref() == Some("S"))
        .expect("class S");
    assert_eq!(small.switched_tasks, 1);
    let suggested = small.suggested.as_ref().expect("suggested");
    assert_eq!(suggested.model, "small");
    assert_eq!(suggested.usd_micros_per_completed_task, 42);

    let other = store.task_class_usage(b.id).await.expect("rows b");
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].model, "other");
}

#[tokio::test]
async fn a_workspace_reads_its_own_usage_by_task_class_model_and_effort_on_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_workspace_reads_its_own_usage_by_task_class_model_and_effort_on_postgres() {
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
