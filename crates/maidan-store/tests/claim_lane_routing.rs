//! Claim routing by service lane (Context Economics C7) on both backends.
//! Lane orders the queue and never filters it. A claimer in a lane takes
//! threads in that lane first, then unlaned ones, then threads in another
//! lane. A claimer with no lane takes unlaned threads first and laned ones
//! after, so laned work never starves. Postgres runs under testcontainers
//! (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    agent: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
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
        .unwrap()
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        agent,
    }
}

/// A task in `lane` (or unlaned), titled and described distinctly so no two
/// share a fingerprint.
async fn task(store: &dyn Store, room: &Room, title: &str, lane: Option<TaskLane>) -> ThreadId {
    let attributes = TaskAttributes {
        kind: TaskKind::Change,
        lane,
        ..TaskAttributes::default()
    };
    let (thread, _event) = store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: None,
                title: Some(title.into()),
                description: Some(format!("{title} brief")),
                created_by: None,
            },
            None,
            Some(attributes),
            None,
        )
        .await
        .unwrap();
    thread.id
}

/// Claim in the channel as a claimer in `lane`, releasing nothing: each claim
/// takes the next thread the order gives.
async fn claim(store: &dyn Store, room: &Room, lane: Option<TaskLane>) -> Option<ThreadId> {
    store
        .claim_next_thread_with_event(room.channel, room.agent, Some(600), None, None, lane)
        .await
        .unwrap()
        .0
        .map(|t| t.id)
}

/// Created oldest first: flex, unlaned, batch. A batch claimer takes batch,
/// then unlaned, then flex, whatever their age.
async fn a_lane_claimer_takes_its_lane_then_unlaned_then_others(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "lane-claimer").await;
    let flex = task(store.as_ref(), &room, "flex", Some(TaskLane::Flex)).await;
    let plain = task(store.as_ref(), &room, "plain", None).await;
    let batch = task(store.as_ref(), &room, "batch", Some(TaskLane::Batch)).await;
    let s = store.as_ref();
    assert_eq!(claim(s, &room, Some(TaskLane::Batch)).await, Some(batch));
    assert_eq!(claim(s, &room, Some(TaskLane::Batch)).await, Some(plain));
    assert_eq!(claim(s, &room, Some(TaskLane::Batch)).await, Some(flex));
    assert_eq!(claim(s, &room, Some(TaskLane::Batch)).await, None);
}

/// A claimer with no lane takes the unlaned thread before an older laned
/// one, and then still takes the laned one: lane never reserves work.
async fn a_claimer_without_a_lane_takes_unlaned_first_and_laned_after(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "no-lane").await;
    let batch = task(store.as_ref(), &room, "batch", Some(TaskLane::Batch)).await;
    let plain = task(store.as_ref(), &room, "plain", None).await;
    let s = store.as_ref();
    assert_eq!(claim(s, &room, None).await, Some(plain));
    assert_eq!(claim(s, &room, None).await, Some(batch));
    assert_eq!(claim(s, &room, None).await, None);
}

/// The workspace queue orders by lane the same way.
async fn the_workspace_queue_orders_by_lane_too(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "ws-lane").await;
    let plain = task(store.as_ref(), &room, "plain", None).await;
    let flex = task(store.as_ref(), &room, "flex", Some(TaskLane::Flex)).await;
    let (claimed, _) = store
        .claim_next_workspace_thread_with_event(
            room.ws,
            room.agent,
            Some(600),
            None,
            None,
            Some(TaskLane::Flex),
        )
        .await
        .unwrap();
    assert_eq!(claimed.map(|t| t.id), Some(flex));
    let (claimed, _) = store
        .claim_next_workspace_thread_with_event(room.ws, room.agent, Some(600), None, None, None)
        .await
        .unwrap();
    assert_eq!(claimed.map(|t| t.id), Some(plain));
}

macro_rules! on_both_backends {
    ($($name:ident),* $(,)?) => {
        mod sqlite_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(super::sqlite().await).await;
                }
            )*
        }
        mod postgres_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    let Some((_container, store)) = super::postgres().await else {
                        return;
                    };
                    super::$name(store).await;
                }
            )*
        }
    };
}

async fn sqlite() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::for_tests(pool))
}

async fn postgres() -> Option<(testcontainers::ContainerAsync<Postgres>, Arc<dyn Store>)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, Arc::new(PostgresStore::for_tests(pool))))
}

on_both_backends!(
    a_lane_claimer_takes_its_lane_then_unlaned_then_others,
    a_claimer_without_a_lane_takes_unlaned_first_and_laned_after,
    the_workspace_queue_orders_by_lane_too,
);
