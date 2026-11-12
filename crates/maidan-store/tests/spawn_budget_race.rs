//! Two child creates racing for a parent's last `max_children` slot: exactly
//! one lands and the other is refused with `SpawnRejected`, on both backends.
//! The count runs in the same transaction as the insert, so a sibling created
//! between a pool-side count and the insert can no longer slip past the cap
//! (the race found while building #2249).
//!
//! Two workspaces race at once, each with its own budget and parent, so one
//! tenant's creates neither use nor block the other's slot. One racer goes
//! through `create_thread` and the other through `create_thread_with_event`,
//! which covers both insert paths.

use std::sync::Arc;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{
    ChannelId, NewChannel, NewThread, NewWorkspace, SpawnAxis, ThreadId, WorkspaceId,
};
use tokio::sync::Barrier;

/// Rounds per run. With the count back on the pool, two racers both see one
/// free slot in most rounds, so a few rounds catch the regression reliably.
const ROUNDS: usize = 6;

/// The cap each workspace sets; every parent gets `LIMIT - 1` children first.
const LIMIT: i64 = 2;

async fn tenant(store: &dyn Store, name: &str) -> (WorkspaceId, ChannelId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("{name}-work"),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id;
    store
        .set_spawn_budget(ws, Some(LIMIT), None, None)
        .await
        .unwrap();
    (ws, ch)
}

fn thread(channel: ChannelId, parent: Option<ThreadId>) -> NewThread {
    NewThread {
        channel_id: channel,
        parent_thread_id: parent,
        title: Some("t".into()),
        description: None,
        created_by: None,
    }
}

async fn create(
    store: &dyn Store,
    new: NewThread,
    with_event: bool,
) -> Result<maidan_types::Thread, StoreError> {
    if with_event {
        store.create_thread_with_event(new).await.map(|(t, _)| t)
    } else {
        store.create_thread(new).await
    }
}

async fn two_creates_for_the_last_slot_land_one(store: Arc<dyn Store>) {
    let tenants = [
        tenant(store.as_ref(), "a").await,
        tenant(store.as_ref(), "b").await,
    ];
    for round in 0..ROUNDS {
        let mut parents = Vec::new();
        for &(ws, ch) in &tenants {
            let parent = create(store.as_ref(), thread(ch, None), false)
                .await
                .unwrap();
            for _ in 0..LIMIT - 1 {
                create(store.as_ref(), thread(ch, Some(parent.id)), false)
                    .await
                    .unwrap();
            }
            parents.push((ws, ch, parent.id));
        }
        let barrier = Arc::new(Barrier::new(parents.len() * 2));
        let mut racers = Vec::new();
        for &(ws, ch, parent) in &parents {
            for with_event in [false, true] {
                let store = store.clone();
                let barrier = barrier.clone();
                racers.push((
                    ws,
                    parent,
                    tokio::spawn(async move {
                        barrier.wait().await;
                        create(store.as_ref(), thread(ch, Some(parent)), with_event).await
                    }),
                ));
            }
        }
        let mut outcomes = Vec::new();
        for (ws, parent, racer) in racers {
            outcomes.push((ws, parent, racer.await.unwrap()));
        }
        for &(ws, ch, parent) in &parents {
            let mine: Vec<_> = outcomes.iter().filter(|o| o.1 == parent).collect();
            let landed = mine.iter().filter(|o| o.2.is_ok()).count();
            let refused = mine
                .iter()
                .filter(|o| {
                    matches!(&o.2, Err(StoreError::SpawnRejected(d))
                        if d.axis == SpawnAxis::Children
                            && d.limit == LIMIT
                            && d.observed == LIMIT
                            && d.workspace_id == ws
                            && d.thread_id == parent)
                })
                .count();
            assert_eq!(
                (landed, refused),
                (1, 1),
                "round {round}: two creates raced for one slot under {parent:?}: {mine:?}"
            );
            let children = store
                .list_threads(ch)
                .await
                .unwrap()
                .into_iter()
                .filter(|t| t.parent_thread_id == Some(parent))
                .count();
            assert_eq!(
                children, LIMIT as usize,
                "round {round}: the parent holds more than its cap"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_creates_for_the_last_slot_land_one_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("s.db").display());
    let pool = maidan_search::sqlite_pool_options_with(10_000)
        .max_connections(6)
        .connect(&url)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    two_creates_for_the_last_slot_land_one(Arc::new(SqliteStore::for_tests(pool))).await;
}

async fn postgres() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    sqlx::PgPool,
)> {
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_creates_for_the_last_slot_land_one_postgres() {
    let Some((_c, pool)) = postgres().await else {
        return;
    };
    two_creates_for_the_last_slot_land_one(Arc::new(PostgresStore::for_tests(pool))).await;
}
