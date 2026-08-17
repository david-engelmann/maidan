//! Scheduled / recurring task foundation (Cluster 226): schedule CRUD + the
//! sweeper's due-scan. Exercised on both backends. No worker/routes yet.

use chrono::{Duration, Utc};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewTaskSchedule, NewWorkspace};
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
        .create_workspace(NewWorkspace {
            name: "sched".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "scheduler".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");

    let now = Utc::now();
    let past = now - Duration::hours(1);
    let future = now + Duration::hours(1);

    // A recurring schedule already due, and a one-shot scheduled for the future.
    let due_sched = store
        .create_task_schedule(NewTaskSchedule {
            workspace_id: ws.id,
            channel_id: channel.id,
            title: "hourly standup".into(),
            interval_secs: Some(3600),
            next_run_at: past,
            created_by: member.id,
        })
        .await
        .expect("create due");
    assert_eq!(due_sched.interval_secs, Some(3600));
    assert!(due_sched.active, "new schedule starts active");
    assert!(due_sched.last_run_at.is_none(), "not fired yet");

    let future_sched = store
        .create_task_schedule(NewTaskSchedule {
            workspace_id: ws.id,
            channel_id: channel.id,
            title: "one-shot".into(),
            interval_secs: None,
            next_run_at: future,
            created_by: member.id,
        })
        .await
        .expect("create future");

    // get round-trips the stored row.
    let fetched = store.get_task_schedule(due_sched.id).await.expect("get");
    assert_eq!(fetched, due_sched);

    // list is workspace-scoped, ordered by next_run_at ascending.
    let all = store.list_task_schedules(ws.id).await.expect("list");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, due_sched.id, "past schedule sorts first");
    assert_eq!(all[1].id, future_sched.id);

    // due(now) returns only the active schedule whose next_run_at has arrived.
    let due = store.due_task_schedules(now, 10).await.expect("due");
    assert_eq!(due.len(), 1, "only the past-due schedule");
    assert_eq!(due[0].id, due_sched.id);

    // delete is conditional.
    assert!(store
        .delete_task_schedule(future_sched.id)
        .await
        .expect("delete"));
    assert!(!store
        .delete_task_schedule(future_sched.id)
        .await
        .expect("delete again"));
    assert_eq!(
        store
            .list_task_schedules(ws.id)
            .await
            .expect("list after delete")
            .len(),
        1
    );
    // Getting a deleted schedule is a clean NotFound.
    assert!(matches!(
        store.get_task_schedule(future_sched.id).await,
        Err(maidan_store::StoreError::NotFound)
    ));
}

#[tokio::test]
async fn task_schedule_crud_and_due_scan_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn task_schedule_crud_and_due_scan_postgres() {
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
