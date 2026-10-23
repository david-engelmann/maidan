//! Amending a submitted plan (#1465) on both backends: cancel, add, replace
//! and rewire in one transaction, only under the planner's live claim, only
//! for unclaimed children, at most three times, and never across
//! workspaces. Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const REPO: &str = "example/app";

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    planner: MemberId,
    worker: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut members = Vec::new();
    for handle in ["planner", "worker"] {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
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
        planner: members[0],
        worker: members[1],
    }
}

async fn allow_repo(store: &dyn Store, ws: WorkspaceId, repo: &str) {
    store
        .allow_egress_target(NewEgressTarget {
            workspace_id: ws,
            surface: EgressSurface::GithubBranch,
            selector: format!("{repo}@main"),
        })
        .await
        .unwrap();
}

/// A plan parent, claimed by the planner; returns it and the claim's lease.
async fn claimed_plan(store: &dyn Store, room: &Room) -> (ThreadId, ClaimLeaseId) {
    let (parent, _) = store
        .create_thread_with_footprint(
            NewThread {
                channel_id: room.channel,
                parent_thread_id: None,
                title: Some("ship bookings".into()),
                description: None,
                created_by: Some(room.planner),
            },
            None,
            Some(TaskAttributes {
                kind: TaskKind::Plan,
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .unwrap();
    let claimed = store
        .claim_next_thread(room.channel, room.planner, Some(600), None)
        .await
        .unwrap()
        .expect("the plan is the only work");
    assert_eq!(claimed.id, parent.id);
    (
        parent.id,
        claimed.claim_lease_id.expect("a claim carries a lease"),
    )
}

/// A child that passes the route's lint: acceptance, an engine, a test and
/// a footprint in the allowlisted repository.
fn child(key: &str, deps: &[&str]) -> PlanChild {
    PlanChild {
        key: key.into(),
        title: format!("do {key}"),
        description: None,
        attributes: Some(TaskAttributes {
            engine: Some(TaskEngine::Pi),
            acceptance: Some(format!("{key} is done")),
            ..TaskAttributes::default()
        }),
        footprint: Some(Footprint {
            repo: REPO.into(),
            mode: FootprintMode::Exclusive,
            items: vec![FootprintItem {
                kind: FootprintKind::Lane,
                value: key.into(),
            }],
        }),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        test: Some("cargo test".into()),
    }
}

/// Submit `children` and answer them by key.
async fn submitted(
    store: &dyn Store,
    parent: ThreadId,
    member: MemberId,
    lease: ClaimLeaseId,
    children: Vec<PlanChild>,
) -> Vec<PlannedChild> {
    let plan = SubmitPlan {
        lease_id: lease,
        children,
    }
    .checked()
    .unwrap();
    match store.submit_plan(parent, member, plan).await.unwrap() {
        PlanOutcome::Created { children, .. } => children,
        other => panic!("expected the children to be created, got {other:?}"),
    }
}

fn id(children: &[PlannedChild], key: &str) -> ThreadId {
    children
        .iter()
        .find(|c| c.key == key)
        .map(|c| c.thread_id)
        .unwrap()
}

fn amendment(lease: ClaimLeaseId) -> AmendPlan {
    AmendPlan {
        lease_id: lease,
        cancel: Vec::new(),
        add: Vec::new(),
        rewire: Vec::new(),
        reason: Some("the plan missed a step".into()),
    }
}

/// Check as the routes do, then amend.
async fn amend(
    store: &dyn Store,
    parent: ThreadId,
    member: MemberId,
    amend: AmendPlan,
) -> Result<AmendOutcome, String> {
    let amend = amend.checked()?;
    store
        .amend_plan(parent, member, amend)
        .await
        .map_err(|e| e.to_string())
}

async fn deps(store: &dyn Store, thread: ThreadId) -> Vec<ThreadId> {
    let mut on: Vec<ThreadId> = store
        .list_thread_dependencies(thread)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.depends_on_thread_id)
        .collect();
    on.sort_by_key(|t| t.0);
    on
}

async fn state(store: &dyn Store, thread: ThreadId) -> ThreadState {
    store.get_thread(thread).await.unwrap().state
}

/// The plan `a`, `b` after `a`, `c` after `b`, claimed and submitted.
async fn three_step_plan(
    store: &dyn Store,
    name: &str,
) -> (Room, ThreadId, ClaimLeaseId, Vec<PlannedChild>) {
    let room = room(store, name).await;
    allow_repo(store, room.ws, REPO).await;
    let (parent, lease) = claimed_plan(store, &room).await;
    let made = submitted(
        store,
        parent,
        room.planner,
        lease,
        vec![child("a", &[]), child("b", &["a"]), child("c", &["b"])],
    )
    .await;
    (room, parent, lease, made)
}

/// Cancelling `b` closes it and frees `c` from waiting for it; adding `d`
/// after `a` makes it under the plan with its edge; rewiring `c` onto `d`
/// replaces its dependencies. One call, and the plan counts one amendment.
async fn an_amendment_cancels_adds_and_rewires_at_once(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let (room, parent, lease, made) = three_step_plan(s, "amend").await;
    let mut change = amendment(lease);
    change.cancel = vec!["b".into()];
    change.add = vec![child("d", &["a"])];
    change.rewire = vec![PlanRewire {
        key: "c".into(),
        depends_on: vec!["d".into()],
    }];
    let outcome = amend(s, parent, room.planner, change).await.unwrap();
    let a = outcome.amendment;
    assert_eq!(a.amendments, 1);
    assert_eq!(a.cancelled.len(), 1);
    assert_eq!(a.cancelled[0].key, "b");
    assert_eq!(a.added.len(), 1);
    let d = a.added[0].thread_id;
    assert!(a.rewired.contains(&"c".to_string()), "{:?}", a.rewired);
    assert!(!outcome.events.is_empty(), "the new child is announced");

    assert_eq!(state(s, id(&made, "b")).await, ThreadState::Closed);
    assert_eq!(deps(s, id(&made, "c")).await, vec![d]);
    assert_eq!(deps(s, d).await, vec![id(&made, "a")]);
    let new = s.get_thread(d).await.unwrap();
    assert_eq!(new.parent_thread_id, Some(parent));
    assert_eq!(new.channel_id, room.channel);
    let plan = s.get_plan_state(parent).await.unwrap().unwrap();
    assert_eq!(plan.total, 3, "b left the plan; d joined it");
}

/// Cancelling and adding the same key replaces the child: its dependents
/// wait for the new thread, not the closed one.
async fn a_replaced_child_keeps_its_dependents_waiting_for_the_new_one(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let (room, parent, lease, made) = three_step_plan(s, "replace").await;
    let mut change = amendment(lease);
    change.cancel = vec!["b".into()];
    change.add = vec![child("b", &["a"])];
    let a = amend(s, parent, room.planner, change)
        .await
        .unwrap()
        .amendment;
    let new_b = id(&a.added, "b");
    assert_ne!(new_b, id(&made, "b"));
    assert_eq!(state(s, id(&made, "b")).await, ThreadState::Closed);
    assert_eq!(deps(s, id(&made, "c")).await, vec![new_b]);
    assert_eq!(deps(s, new_b).await, vec![id(&made, "a")]);
}

/// A claimed child cannot be cancelled or rewired, and the refusal writes
/// nothing: the rest of the amendment is not half applied.
async fn a_claimed_child_is_refused_and_nothing_changes(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let (room, parent, lease, made) = three_step_plan(s, "claimed").await;
    let taken = s
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .expect("a is ready");
    assert_eq!(taken.id, id(&made, "a"));
    let mut change = amendment(lease);
    change.cancel = vec!["a".into()];
    change.add = vec![child("d", &[])];
    let refused = amend(s, parent, room.planner, change).await.unwrap_err();
    assert!(refused.contains("claimed or finished"), "{refused}");
    assert_eq!(s.get_plan_state(parent).await.unwrap().unwrap().total, 3);
    assert_eq!(state(s, id(&made, "a")).await, ThreadState::Open);

    // The refusal did not count: a good amendment is the first.
    let mut change = amendment(lease);
    change.cancel = vec!["c".into()];
    let a = amend(s, parent, room.planner, change).await.unwrap();
    assert_eq!(a.amendment.amendments, 1);
}

/// Three amendments are taken and a fourth is refused.
async fn a_fourth_amendment_is_refused(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let (room, parent, lease, _) = three_step_plan(s, "cap").await;
    for n in 1..=3 {
        let mut change = amendment(lease);
        change.add = vec![child(&format!("extra{n}"), &[])];
        let a = amend(s, parent, room.planner, change).await.unwrap();
        assert_eq!(a.amendment.amendments, n);
    }
    let mut change = amendment(lease);
    change.add = vec![child("extra4", &[])];
    let refused = amend(s, parent, room.planner, change).await.unwrap_err();
    assert!(refused.contains("the most it takes"), "{refused}");
    assert_eq!(s.get_plan_state(parent).await.unwrap().unwrap().total, 6);
}

/// Only the member holding the plan's live claim amends it, and an
/// amendment that loops the plan's dependencies is refused whole.
async fn only_the_claim_holder_amends_and_a_loop_is_refused(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let (room, parent, lease, _) = three_step_plan(s, "holder").await;
    let mut change = amendment(lease);
    change.add = vec![child("d", &[])];
    let not_holder = amend(s, parent, room.worker, change.clone())
        .await
        .unwrap_err();
    assert!(not_holder.contains("live claim"), "{not_holder}");
    change.lease_id = ClaimLeaseId(uuid::Uuid::now_v7());
    let stale = amend(s, parent, room.planner, change).await.unwrap_err();
    assert!(stale.contains("live claim"), "{stale}");

    let mut looped = amendment(lease);
    looped.rewire = vec![PlanRewire {
        key: "b".into(),
        depends_on: vec!["c".into()],
    }];
    let refused = amend(s, parent, room.planner, looped).await.unwrap_err();
    assert!(refused.contains("loop"), "{refused}");
    assert_eq!(s.get_plan_state(parent).await.unwrap().unwrap().total, 3);
}

/// A plan in one workspace cannot be amended by a member of another, even
/// with that plan's lease, and naming another workspace's child does
/// nothing to it.
async fn another_workspace_cannot_amend_the_plan(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let (_, parent, lease, made) = three_step_plan(s, "ours").await;
    let (theirs, their_parent, their_lease, _) = three_step_plan(s, "theirs").await;

    let mut change = amendment(lease);
    change.cancel = vec!["c".into()];
    let refused = amend(s, parent, theirs.planner, change).await.unwrap_err();
    assert!(refused.contains("live claim"), "{refused}");
    assert_eq!(state(s, id(&made, "c")).await, ThreadState::Open);

    // Their own amendment touches only their plan's `c`.
    let mut change = amendment(their_lease);
    change.cancel = vec!["c".into()];
    amend(s, their_parent, theirs.planner, change)
        .await
        .unwrap();
    assert_eq!(state(s, id(&made, "c")).await, ThreadState::Open);
    assert_eq!(s.get_plan_state(parent).await.unwrap().unwrap().total, 3);
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
    an_amendment_cancels_adds_and_rewires_at_once,
    a_replaced_child_keeps_its_dependents_waiting_for_the_new_one,
    a_claimed_child_is_refused_and_nothing_changes,
    a_fourth_amendment_is_refused,
    only_the_claim_holder_amends_and_a_loop_is_refused,
    another_workspace_cannot_amend_the_plan,
);
