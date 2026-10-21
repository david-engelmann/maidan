//! A workspace's default engine on both backends: every store path that
//! creates a thread routes one created with no engine named to it, inside
//! the create transaction; an engine named at creation wins; without a
//! default a thread stays unrouted; and one tenant's default never reaches
//! another's threads. Postgres runs under testcontainers (skipped without
//! Docker).

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

async fn set_default(store: &dyn Store, ws: WorkspaceId, engine: Option<TaskEngine>) {
    store
        .set_workspace_default_engine(
            ws,
            engine,
            NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "workspace.default_engine.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: serde_json::json!({ "engine": engine.map(TaskEngine::as_str) }),
            },
        )
        .await
        .unwrap();
}

async fn engine_of(store: &dyn Store, thread: ThreadId) -> Option<TaskEngine> {
    store.get_thread_attributes(thread).await.unwrap().engine
}

/// The default reads back as set, clears with `None`, starts unset, and a
/// missing workspace is NotFound.
async fn the_default_engine_reads_back_as_set_and_clears(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "setting", 0).await;
    assert_eq!(store.workspace_default_engine(room.ws).await.unwrap(), None);
    set_default(store.as_ref(), room.ws, Some(TaskEngine::Pi)).await;
    assert_eq!(
        store.workspace_default_engine(room.ws).await.unwrap(),
        Some(TaskEngine::Pi)
    );
    set_default(store.as_ref(), room.ws, None).await;
    assert_eq!(store.workspace_default_engine(room.ws).await.unwrap(), None);
    assert!(matches!(
        store.workspace_default_engine(WorkspaceId::new()).await,
        Err(maidan_store::StoreError::NotFound)
    ));
}

/// Every plain create path routes an untagged thread to the default, and
/// a thread given attributes without an engine keeps its other fields.
async fn an_untagged_thread_gets_the_default_on_every_create_path(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "untagged", 0).await;
    set_default(store.as_ref(), room.ws, Some(TaskEngine::Pi)).await;

    let plain = store
        .create_thread(new_thread(room.channel, "plain"))
        .await
        .unwrap();
    assert_eq!(
        engine_of(store.as_ref(), plain.id).await,
        Some(TaskEngine::Pi)
    );

    let (with_event, _) = store
        .create_thread_with_event(new_thread(room.channel, "with event"))
        .await
        .unwrap();
    assert_eq!(
        engine_of(store.as_ref(), with_event.id).await,
        Some(TaskEngine::Pi)
    );

    let described = TaskAttributes {
        kind: TaskKind::Review,
        engine: None,
        threat_model: true,
        size: Some(TaskSize::S),
        acceptance: Some("It passes.".into()),
        linear_issue: Some("ENG-1".into()),
    };
    let (thread, _) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "described"),
            None,
            Some(described.clone()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store.get_thread_attributes(thread.id).await.unwrap(),
        TaskAttributes {
            engine: Some(TaskEngine::Pi),
            ..described
        },
        "only the engine is filled in"
    );
}

/// An engine named at creation wins over the default.
async fn an_explicit_engine_wins_over_the_default(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "explicit", 0).await;
    set_default(store.as_ref(), room.ws, Some(TaskEngine::Pi)).await;
    let (thread, _) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "for a session"),
            None,
            Some(TaskAttributes {
                engine: Some(TaskEngine::Session),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        engine_of(store.as_ref(), thread.id).await,
        Some(TaskEngine::Session)
    );
}

/// Without a default an untagged thread stays unrouted, with no attribute
/// row written for it.
async fn no_default_leaves_a_thread_untagged(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "none", 0).await;
    let (thread, _) = store
        .create_thread_with_event(new_thread(room.channel, "unrouted"))
        .await
        .unwrap();
    assert_eq!(
        store.get_thread_attributes(thread.id).await.unwrap(),
        TaskAttributes::default()
    );
}

/// A plan child submitted without attributes gets the default; one naming
/// its engine keeps it.
async fn a_plan_child_without_attributes_gets_the_default(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "plans", 1).await;
    let planner = room.agents[0];
    let (parent, _) = store
        .create_thread_with_footprint(
            NewThread {
                created_by: Some(planner),
                ..new_thread(room.channel, "plan it")
            },
            None,
            Some(TaskAttributes {
                kind: TaskKind::Plan,
                engine: Some(TaskEngine::Session),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .unwrap();
    set_default(store.as_ref(), room.ws, Some(TaskEngine::Pi)).await;
    let lease = store
        .claim_thread(parent.id, planner)
        .await
        .unwrap()
        .thread
        .claim_lease_id
        .expect("a claim carries a lease");
    let child = |key: &str, attributes: Option<TaskAttributes>| PlanChild {
        key: key.into(),
        title: format!("do {key}"),
        description: None,
        attributes,
        footprint: None,
        depends_on: Vec::new(),
        test: None,
    };
    let plan = SubmitPlan {
        lease_id: lease,
        children: vec![
            child("bare", None),
            child(
                "human",
                Some(TaskAttributes {
                    engine: Some(TaskEngine::Human),
                    ..TaskAttributes::default()
                }),
            ),
        ],
    }
    .validated()
    .unwrap();
    let PlanOutcome::Created { children, .. } =
        store.submit_plan(parent.id, planner, plan).await.unwrap()
    else {
        panic!("the children are created");
    };
    let id = |key: &str| {
        children
            .iter()
            .find(|c| c.key == key)
            .map(|c| c.thread_id)
            .unwrap()
    };
    assert_eq!(
        engine_of(store.as_ref(), id("bare")).await,
        Some(TaskEngine::Pi)
    );
    assert_eq!(
        engine_of(store.as_ref(), id("human")).await,
        Some(TaskEngine::Human)
    );
    assert_eq!(
        engine_of(store.as_ref(), parent.id).await,
        Some(TaskEngine::Session),
        "setting the default does not reroute a thread made before"
    );
}

/// A recipe's threads, which carry no attributes, get the default.
async fn a_recipe_thread_gets_the_default(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "recipes", 1).await;
    set_default(store.as_ref(), room.ws, Some(TaskEngine::Human)).await;
    let recipe = store
        .create_recipe(NewRecipe {
            workspace_id: room.ws,
            channel_id: room.channel,
            name: "weekly".into(),
            spec: RecipeSpec::default(),
            created_by: room.agents[0],
        })
        .await
        .unwrap();
    let (run, _) = store
        .instantiate_recipe(recipe.id, serde_json::json!({}), room.agents[0])
        .await
        .unwrap();
    assert_eq!(
        engine_of(store.as_ref(), run.root_thread_id).await,
        Some(TaskEngine::Human)
    );
}

/// With default `pi`, a Pi claimer takes a thread created with no
/// attributes, and a session claimer never sees it.
async fn a_pi_claimer_takes_an_untagged_thread_under_a_pi_default(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "claims", 2).await;
    let (pi, session) = (room.agents[0], room.agents[1]);
    set_default(store.as_ref(), room.ws, Some(TaskEngine::Pi)).await;
    let (thread, _) = store
        .create_thread_with_event(new_thread(room.channel, "untagged"))
        .await
        .unwrap();
    assert!(
        store
            .claim_next_thread(room.channel, session, Some(600), Some(TaskEngine::Session))
            .await
            .unwrap()
            .is_none(),
        "a session claimer must not take pi work"
    );
    let claimed = store
        .claim_next_thread(room.channel, pi, Some(600), Some(TaskEngine::Pi))
        .await
        .unwrap()
        .expect("the untagged thread is pi work");
    assert_eq!(claimed.id, thread.id);
}

/// Tenant A's default routes A's threads only: B's stay as B set them.
async fn one_tenants_default_never_reaches_another(store: Arc<dyn Store>) {
    let a = room(store.as_ref(), "tenant-a", 0).await;
    let b = room(store.as_ref(), "tenant-b", 0).await;
    set_default(store.as_ref(), b.ws, Some(TaskEngine::Human)).await;
    set_default(store.as_ref(), a.ws, Some(TaskEngine::Pi)).await;
    assert_eq!(
        store.workspace_default_engine(b.ws).await.unwrap(),
        Some(TaskEngine::Human)
    );
    let (in_b, _) = store
        .create_thread_with_event(new_thread(b.channel, "b's work"))
        .await
        .unwrap();
    assert_eq!(
        engine_of(store.as_ref(), in_b.id).await,
        Some(TaskEngine::Human)
    );
    let (in_a, _) = store
        .create_thread_with_event(new_thread(a.channel, "a's work"))
        .await
        .unwrap();
    assert_eq!(
        engine_of(store.as_ref(), in_a.id).await,
        Some(TaskEngine::Pi)
    );
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
    the_default_engine_reads_back_as_set_and_clears,
    an_untagged_thread_gets_the_default_on_every_create_path,
    an_explicit_engine_wins_over_the_default,
    no_default_leaves_a_thread_untagged,
    a_plan_child_without_attributes_gets_the_default,
    a_recipe_thread_gets_the_default,
    a_pi_claimer_takes_an_untagged_thread_under_a_pi_default,
    one_tenants_default_never_reaches_another,
);
