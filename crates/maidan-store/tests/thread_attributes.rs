//! Task attributes on both backends: set at creation in the create
//! transaction, replaced later, read back as the defaults when a thread was
//! never described, carried on the live claims list, and respected by an
//! engine-filtered claim. The metrics list filters and splits on them.
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
    ws: WorkspaceId,
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
        ws: ws.id,
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

fn attrs(kind: TaskKind, engine: Option<TaskEngine>, threat_model: bool) -> TaskAttributes {
    TaskAttributes {
        kind,
        engine,
        threat_model,
    }
}

/// Attributes declared at intake are stored with the thread, and a thread
/// created without any reads back as the defaults.
async fn attributes_set_at_creation_are_read_back(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "intake", 1).await;
    let described = attrs(TaskKind::Review, Some(TaskEngine::Human), true);
    let (thread, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "described"), None, Some(described))
        .await
        .unwrap();
    assert_eq!(
        store.get_thread_attributes(thread.id).await.unwrap(),
        described
    );

    let (plain, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "plain"), None, None)
        .await
        .unwrap();
    assert_eq!(
        store.get_thread_attributes(plain.id).await.unwrap(),
        TaskAttributes::default(),
        "an undescribed thread reads as a change with no engine"
    );
}

/// Setting attributes later replaces them whole; a missing thread is a
/// NotFound, not a row for nothing.
async fn attributes_set_later_replace_the_whole_set(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "later", 1).await;
    let (thread, _event) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "task"),
            None,
            Some(attrs(TaskKind::Change, Some(TaskEngine::Pi), false)),
        )
        .await
        .unwrap();
    let replaced = attrs(TaskKind::Plan, Some(TaskEngine::Session), true);
    let stored = store
        .set_thread_attributes(thread.id, replaced)
        .await
        .unwrap();
    assert_eq!(stored, replaced);
    assert_eq!(
        store.get_thread_attributes(thread.id).await.unwrap(),
        replaced
    );

    let missing = store.set_thread_attributes(ThreadId::new(), replaced).await;
    assert!(
        matches!(missing, Err(maidan_store::StoreError::NotFound)),
        "setting attributes on a missing thread must be NotFound, got {missing:?}"
    );
}

/// An engine-filtered claim takes only work routed to that engine: other
/// engines and unrouted work are skipped, and an unfiltered claim still
/// takes anything.
async fn an_engine_claim_takes_only_its_own_work(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "engines", 2).await;
    let pi = room.agents[0];
    for (title, engine) in [
        ("for a session", Some(TaskEngine::Session)),
        ("for a person", Some(TaskEngine::Human)),
        ("unrouted", None),
        ("for pi", Some(TaskEngine::Pi)),
    ] {
        store
            .create_thread_with_footprint(
                new_thread(room.channel, title),
                None,
                Some(attrs(TaskKind::Change, engine, false)),
            )
            .await
            .unwrap();
    }
    let claimed = store
        .claim_next_thread(room.channel, pi, Some(600), Some(TaskEngine::Pi))
        .await
        .unwrap()
        .expect("the pi task is claimable");
    assert_eq!(claimed.title.as_deref(), Some("for pi"));
    // Nothing else in the channel is pi work.
    assert!(store
        .claim_next_thread(room.channel, pi, Some(600), Some(TaskEngine::Pi))
        .await
        .unwrap()
        .is_none());
    // The unfiltered queue still hands out the rest, oldest first.
    let next = store
        .claim_next_thread(room.channel, room.agents[1], Some(600), None)
        .await
        .unwrap()
        .expect("unfiltered work remains");
    assert_eq!(next.title.as_deref(), Some("for a session"));
}

/// A live claim's entry on the claims list carries the thread's attributes.
async fn a_live_claim_entry_carries_the_attributes(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "claims", 1).await;
    let described = attrs(TaskKind::Review, Some(TaskEngine::Pi), true);
    let (thread, _event) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "claimed"),
            Some(Footprint {
                repo: "example/app".into(),
                mode: FootprintMode::Exclusive,
                items: vec![FootprintItem {
                    kind: FootprintKind::Lane,
                    value: "db".into(),
                }],
            }),
            Some(described),
        )
        .await
        .unwrap();
    store
        .claim_next_thread(room.channel, room.agents[0], Some(600), None)
        .await
        .unwrap()
        .expect("the task claims");
    let claims = store.list_active_claims(room.ws, None, None).await.unwrap();
    let entry = claims
        .iter()
        .find(|claim| claim.thread_id == thread.id)
        .expect("the claim is listed");
    assert_eq!(entry.attributes, described);
}

/// The metrics list joins the attributes onto each pull, filters by engine
/// and by kind (an undescribed thread counts as a change), and the rollup
/// splits by engine.
async fn the_metrics_carry_filter_and_split_on_attributes(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "metrics", 1).await;
    let described = attrs(TaskKind::Review, Some(TaskEngine::Pi), false);
    let (review, _event) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "review task"),
            None,
            Some(described),
        )
        .await
        .unwrap();
    let (plain, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "plain task"), None, None)
        .await
        .unwrap();
    for (n, thread) in [(1, review.id), (2, plain.id)] {
        store
            .record_change_round(NewChangeRound {
                workspace_id: room.ws,
                thread_id: thread,
                repo: "example/app".into(),
                pull_number: n,
                commit_sha: format!("c{n}"),
                base_sha: "a".repeat(40),
                base_behind: None,
                diff_lines: 10,
                paths: Vec::new(),
                base_branch: None,
            })
            .await
            .unwrap();
    }

    let all = store
        .list_change_pulls(room.ws, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    let review_pull = all
        .iter()
        .find(|pull| pull.thread_id == review.id)
        .expect("the review pull is listed");
    assert_eq!(review_pull.attributes, described);
    let plain_pull = all
        .iter()
        .find(|pull| pull.thread_id == plain.id)
        .expect("the plain pull is listed");
    assert_eq!(plain_pull.attributes, TaskAttributes::default());

    let pi_only = store
        .list_change_pulls(room.ws, None, None, Some(TaskEngine::Pi), None)
        .await
        .unwrap();
    assert_eq!(pi_only.len(), 1);
    assert_eq!(pi_only[0].thread_id, review.id);
    let changes = store
        .list_change_pulls(room.ws, None, None, None, Some(TaskKind::Change))
        .await
        .unwrap();
    assert_eq!(changes.len(), 1, "the undescribed task counts as a change");
    assert_eq!(changes[0].thread_id, plain.id);

    let metrics = maidan_types::swarm_metrics(all, maidan_types::FlagLimits::default());
    let rollup = &metrics.repos[0];
    assert_eq!(rollup.by_engine.len(), 2);
    assert_eq!(rollup.by_engine[0].engine, Some(TaskEngine::Pi));
    assert_eq!(rollup.by_engine[0].tasks, 1);
    assert_eq!(rollup.by_engine[1].engine, None);
    assert_eq!(rollup.by_engine[1].tasks, 1);
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
    attributes_set_at_creation_are_read_back,
    attributes_set_later_replace_the_whole_set,
    an_engine_claim_takes_only_its_own_work,
    a_live_claim_entry_carries_the_attributes,
    the_metrics_carry_filter_and_split_on_attributes,
);
