//! Cluster 227: the scheduler sweeper materializes a task thread when a schedule
//! comes due, and advances/deactivates the schedule.

use std::sync::Arc;

use chrono::{Duration, Utc};
use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{scheduler, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EventFilter, EventKind, MemberKind, NewChannel, NewMember, NewRecipe, NewTaskSchedule,
    NewWorkspace, RecipeChild, RecipeSpec,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn sweep_fires_due_schedule_and_creates_thread() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "sched".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    // A one-shot schedule already due, and a recurring one not yet due.
    let once = store
        .create_task_schedule(NewTaskSchedule {
            workspace_id: ws.id,
            channel_id: channel.id,
            title: "standup".into(),
            interval_secs: None,
            next_run_at: Utc::now() - Duration::minutes(5),
            created_by: member.id,
            recipe_id: None,
        })
        .await
        .unwrap();
    let future = store
        .create_task_schedule(NewTaskSchedule {
            workspace_id: ws.id,
            channel_id: channel.id,
            title: "later".into(),
            interval_secs: Some(3600),
            next_run_at: Utc::now() + Duration::hours(1),
            created_by: member.id,
            recipe_id: None,
        })
        .await
        .unwrap();

    // Before the sweep: no threads.
    assert!(store.list_threads(channel.id).await.unwrap().is_empty());

    let fired = scheduler::sweep_once(&state).await;
    assert_eq!(fired, 1, "only the due one-shot fires");

    // A thread titled after the schedule now exists.
    let threads = store.list_threads(channel.id).await.unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].title.as_deref(), Some("standup"));

    // The one-shot deactivated; the future schedule is untouched.
    assert!(!store.get_task_schedule(once.id).await.unwrap().active);
    assert!(store.get_task_schedule(future.id).await.unwrap().active);

    // A second sweep fires nothing more (the due one is spent).
    assert_eq!(scheduler::sweep_once(&state).await, 0);
}

#[tokio::test]
async fn sweep_instantiates_a_recipe_and_skips_when_a_prior_run_is_in_flight() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    // Subscribe for ScheduleSkipped before the sweep.
    let mut sub = bus
        .subscribe(EventFilter::all().with_kinds([EventKind::ScheduleSkipped]))
        .await
        .unwrap();
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "sched".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    // A recipe with two children (no required params, so a param-less firing works).
    let recipe = store
        .create_recipe(NewRecipe {
            workspace_id: ws.id,
            channel_id: channel.id,
            name: "nightly-build".into(),
            spec: RecipeSpec {
                children: vec![
                    RecipeChild {
                        key: "build".into(),
                        title: "build".into(),
                        required_skills: vec![],
                        depends_on: vec![],
                    },
                    RecipeChild {
                        key: "review".into(),
                        title: "review".into(),
                        required_skills: vec![],
                        depends_on: vec!["build".into()],
                    },
                ],
                ..Default::default()
            },
            created_by: member.id,
        })
        .await
        .unwrap();

    // Two due one-shot schedules on the same recipe: the first fires, the second
    // finds the run still in flight and is skipped.
    for offset in [6, 5] {
        store
            .create_task_schedule(NewTaskSchedule {
                workspace_id: ws.id,
                channel_id: channel.id,
                title: "nightly".into(),
                interval_secs: None,
                next_run_at: Utc::now() - Duration::minutes(offset),
                created_by: member.id,
                recipe_id: Some(recipe.id),
            })
            .await
            .unwrap();
    }

    assert_eq!(
        scheduler::sweep_once(&state).await,
        2,
        "both schedules claimed"
    );

    // Exactly one run materialized (parent + 2 children = 3 threads); the second
    // schedule skipped rather than creating a second run.
    let threads = store.list_threads(channel.id).await.unwrap();
    assert_eq!(threads.len(), 3, "one run only (parent + 2 children)");
    assert!(store.latest_recipe_run(recipe.id).await.unwrap().is_some());

    // The skip emitted a ScheduleSkipped event.
    let skipped = tokio::time::timeout(std::time::Duration::from_secs(2), sub.next())
        .await
        .expect("a ScheduleSkipped event")
        .expect("bus item");
    match skipped {
        BusItem::Event(env) => {
            assert_eq!(env.event.kind(), EventKind::ScheduleSkipped);
        }
        other => panic!("expected a ScheduleSkipped event, got {other:?}"),
    }
}
