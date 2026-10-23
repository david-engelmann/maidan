//! Task fingerprints (Context Economics C8) on both backends: stored in the
//! create transaction, linked to the oldest open identical task in the same
//! workspace, never across workspaces, and never to a closed task.
//! Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Room {
    channel: ChannelId,
    agents: Vec<MemberId>,
}

async fn room(store: &dyn Store, name: &str, agents: usize) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for n in 0..agents {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: format!("agent{n}"),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
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
        channel: channel.id,
        agents: ids,
    }
}

fn new_thread(channel: ChannelId, title: &str) -> NewThread {
    NewThread {
        channel_id: channel,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    }
}

async fn create(
    store: &dyn Store,
    channel: ChannelId,
    title: &str,
    attributes: Option<TaskAttributes>,
) -> ThreadId {
    store
        .create_thread_with_footprint(new_thread(channel, title), None, attributes, None)
        .await
        .unwrap()
        .0
        .id
}

/// A second identical task is linked to the first; the first links to
/// nothing; the stored fingerprint is the one the types compute.
async fn an_identical_open_task_is_linked_to_the_first(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "dupes", 1).await;
    let acceptance = TaskAttributes {
        acceptance: Some("the sweep closes it".into()),
        ..TaskAttributes::default()
    };
    let first = create(
        store.as_ref(),
        room.channel,
        "Fix the sweep",
        Some(acceptance.clone()),
    )
    .await;
    let second = create(
        store.as_ref(),
        room.channel,
        "  Fix the sweep ",
        Some(acceptance.clone()),
    )
    .await;
    let third = create(
        store.as_ref(),
        room.channel,
        "Fix the sweep",
        Some(acceptance.clone()),
    )
    .await;
    let a = store.get_task_fingerprint(first).await.unwrap().unwrap();
    assert_eq!(
        Some(a.fingerprint.clone()),
        task_fingerprint(Some("Fix the sweep"), None, &acceptance)
    );
    assert_eq!(a.duplicate_of, None);
    let b = store.get_task_fingerprint(second).await.unwrap().unwrap();
    assert_eq!(b.fingerprint, a.fingerprint);
    assert_eq!(b.duplicate_of, Some(first));
    // Linked to the oldest, not the latest.
    let c = store.get_task_fingerprint(third).await.unwrap().unwrap();
    assert_eq!(c.duplicate_of, Some(first));
}

/// A different brief, or the same brief with different inputs, is not a
/// duplicate.
async fn a_different_brief_or_input_is_not_linked(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "near-misses", 1).await;
    create(store.as_ref(), room.channel, "Fix the sweep", None).await;
    let other_title = create(store.as_ref(), room.channel, "Fix the sweeps", None).await;
    let other_input = create(
        store.as_ref(),
        room.channel,
        "Fix the sweep",
        Some(TaskAttributes {
            linear_issue: Some("ENG-6004".into()),
            ..TaskAttributes::default()
        }),
    )
    .await;
    for id in [other_title, other_input] {
        assert_eq!(
            store
                .get_task_fingerprint(id)
                .await
                .unwrap()
                .unwrap()
                .duplicate_of,
            None
        );
    }
}

/// The same brief in another workspace is never linked, though the
/// fingerprint itself is the same.
async fn another_workspaces_identical_task_is_not_linked(store: Arc<dyn Store>) {
    let a = room(store.as_ref(), "tenant-a", 1).await;
    let b = room(store.as_ref(), "tenant-b", 1).await;
    let in_a = create(store.as_ref(), a.channel, "Rotate the keys", None).await;
    let in_b = create(store.as_ref(), b.channel, "Rotate the keys", None).await;
    let fa = store.get_task_fingerprint(in_a).await.unwrap().unwrap();
    let fb = store.get_task_fingerprint(in_b).await.unwrap().unwrap();
    assert_eq!(fa.fingerprint, fb.fingerprint);
    assert_eq!(fb.duplicate_of, None);
}

/// A closed task is not an open duplicate: a new identical task after it
/// links to nothing.
async fn a_closed_task_is_not_linked(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "closed", 1).await;
    let first = create(store.as_ref(), room.channel, "Ship the index", None).await;
    store
        .transition_thread(first, room.agents[0], maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    // In review still counts as open work.
    let during = create(store.as_ref(), room.channel, "Ship the index", None).await;
    assert_eq!(
        store
            .get_task_fingerprint(during)
            .await
            .unwrap()
            .unwrap()
            .duplicate_of,
        Some(first)
    );
    store
        .transition_thread(first, room.agents[0], maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    store
        .transition_thread(
            during,
            room.agents[0],
            maidan_fsm::ThreadAction::StartReview,
        )
        .await
        .unwrap();
    store
        .transition_thread(during, room.agents[0], maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    let after = create(store.as_ref(), room.channel, "Ship the index", None).await;
    assert_eq!(
        store
            .get_task_fingerprint(after)
            .await
            .unwrap()
            .unwrap()
            .duplicate_of,
        None
    );
}

/// A thread with no title or description has no fingerprint.
async fn a_thread_with_no_brief_has_no_fingerprint(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "blank", 1).await;
    let (thread, _) = store
        .create_thread_with_footprint(
            NewThread {
                title: None,
                ..new_thread(room.channel, "x")
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(store.get_task_fingerprint(thread.id).await.unwrap(), None);
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
    an_identical_open_task_is_linked_to_the_first,
    a_different_brief_or_input_is_not_linked,
    another_workspaces_identical_task_is_not_linked,
    a_closed_task_is_not_linked,
    a_thread_with_no_brief_has_no_fingerprint,
);
