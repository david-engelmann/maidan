//! Planner fan-out (#1421) on both backends: a plan's children are made in
//! one transaction or not at all, only by the member holding the parent's
//! live claim, only for repositories the workspace's `github_branch`
//! allowlist names, and a repeat answers with what the first call made.
//! Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_fsm::ThreadAction;
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

fn child(key: &str, deps: &[&str]) -> PlanChild {
    PlanChild {
        key: key.into(),
        title: format!("do {key}"),
        description: None,
        attributes: None,
        footprint: None,
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        test: None,
    }
}

fn in_repo(mut c: PlanChild, repo: &str, lane: &str) -> PlanChild {
    c.footprint = Some(Footprint {
        repo: repo.into(),
        mode: FootprintMode::Exclusive,
        items: vec![FootprintItem {
            kind: FootprintKind::Lane,
            value: lane.into(),
        }],
    });
    c
}

/// Validate as the routes do, then submit.
async fn submit(
    store: &dyn Store,
    parent: ThreadId,
    member: MemberId,
    lease: ClaimLeaseId,
    children: Vec<PlanChild>,
) -> Result<PlanOutcome, String> {
    let plan = SubmitPlan {
        lease_id: lease,
        children,
    }
    .validated()?;
    store
        .submit_plan(parent, member, plan)
        .await
        .map_err(|e| e.to_string())
}

fn created(outcome: PlanOutcome) -> Vec<PlannedChild> {
    match outcome {
        PlanOutcome::Created { children, events } => {
            assert_eq!(events.len(), children.len(), "one ThreadCreated per child");
            children
        }
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

async fn children_of(store: &dyn Store, room: &Room, parent: ThreadId) -> usize {
    store
        .list_threads(room.channel)
        .await
        .unwrap()
        .iter()
        .filter(|t| t.parent_thread_id == Some(parent))
        .count()
}

/// The issue's three children: `b` waits for `a`, `c` stands alone. Each is
/// made under the parent with its footprint, attributes and edges, and the
/// queue hands out `a` and `c` but not `b`.
async fn a_plan_makes_every_child_with_its_edges(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "plan").await;
    allow_repo(store.as_ref(), room.ws, REPO).await;
    let (parent, lease) = claimed_plan(store.as_ref(), &room).await;
    let mut a = in_repo(child("a", &[]), REPO, "db");
    a.attributes = Some(TaskAttributes {
        size: Some(TaskSize::S),
        acceptance: Some("the table exists".into()),
        ..TaskAttributes::default()
    });
    let made = created(
        submit(
            store.as_ref(),
            parent,
            room.planner,
            lease,
            vec![a, child("b", &["a"]), child("c", &[])],
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        made.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"],
        "children come back in submission order"
    );
    for planned in &made {
        let thread = store.get_thread(planned.thread_id).await.unwrap();
        assert_eq!(thread.parent_thread_id, Some(parent));
        assert_eq!(thread.channel_id, room.channel);
    }
    let a_id = id(&made, "a");
    let attributes = store.get_thread_attributes(a_id).await.unwrap();
    assert_eq!(attributes.size, Some(TaskSize::S));
    assert_eq!(attributes.acceptance.as_deref(), Some("the table exists"));
    let footprint = store.get_thread_footprint(a_id).await.unwrap().unwrap();
    assert_eq!(footprint.repo, REPO);
    let deps = store
        .list_thread_dependencies(id(&made, "b"))
        .await
        .unwrap();
    assert_eq!(
        deps.iter()
            .map(|d| d.depends_on_thread_id)
            .collect::<Vec<_>>(),
        vec![a_id]
    );
    assert_eq!(
        store.get_plan_state(parent).await.unwrap(),
        Some(PlanState {
            total: 3,
            closed: 0,
            open: 3,
            blocked: 0
        })
    );

    let mut handed = Vec::new();
    while let Some(t) = store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
    {
        handed.push(t.id);
    }
    handed.sort_by_key(|t| t.0);
    let mut ready = vec![a_id, id(&made, "c")];
    ready.sort_by_key(|t| t.0);
    assert_eq!(handed, ready, "b waits for a");
}

/// A child whose repository is off the allowlist, last in the plan, means
/// the children before it are not made either.
async fn a_bad_child_creates_nothing(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "atomic").await;
    allow_repo(store.as_ref(), room.ws, REPO).await;
    let (parent, lease) = claimed_plan(store.as_ref(), &room).await;
    let err = submit(
        store.as_ref(),
        parent,
        room.planner,
        lease,
        vec![
            in_repo(child("a", &[]), REPO, "db"),
            child("b", &["a"]),
            in_repo(child("c", &[]), "example/elsewhere", "api"),
        ],
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("child `c`") && err.contains("example/elsewhere"),
        "{err}"
    );
    assert_eq!(children_of(store.as_ref(), &room, parent).await, 0);
    assert_eq!(store.get_plan_state(parent).await.unwrap(), None);

    let cycle = submit(
        store.as_ref(),
        parent,
        room.planner,
        lease,
        vec![child("a", &["b"]), child("b", &["a"])],
    )
    .await
    .unwrap_err();
    assert!(cycle.contains("loop"), "{cycle}");
    let unknown = submit(
        store.as_ref(),
        parent,
        room.planner,
        lease,
        vec![child("a", &["ghost"])],
    )
    .await
    .unwrap_err();
    assert!(unknown.contains("ghost"), "{unknown}");
    assert_eq!(children_of(store.as_ref(), &room, parent).await, 0);

    // The same plan without the bad child goes through: nothing half-made
    // was left behind to conflict with.
    created(
        submit(
            store.as_ref(),
            parent,
            room.planner,
            lease,
            vec![in_repo(child("a", &[]), REPO, "db"), child("b", &["a"])],
        )
        .await
        .unwrap(),
    );
}

/// A retry from the same claim with the same keys gets the same children;
/// other keys, or another claim, get a conflict listing them.
async fn a_second_plan_replays_or_conflicts(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "retry").await;
    let (parent, lease) = claimed_plan(store.as_ref(), &room).await;
    let first = created(
        submit(
            store.as_ref(),
            parent,
            room.planner,
            lease,
            vec![child("a", &[]), child("b", &["a"])],
        )
        .await
        .unwrap(),
    );
    let again = submit(
        store.as_ref(),
        parent,
        room.planner,
        lease,
        vec![child("b", &["a"]), child("a", &[])],
    )
    .await
    .unwrap();
    assert_eq!(again, PlanOutcome::Replayed(first.clone()));
    let other_keys = submit(
        store.as_ref(),
        parent,
        room.planner,
        lease,
        vec![child("a", &[]), child("z", &[])],
    )
    .await
    .unwrap();
    assert_eq!(other_keys, PlanOutcome::Conflict(first.clone()));

    store
        .release_claim(parent, room.planner, lease)
        .await
        .unwrap();
    let reclaimed = store
        .claim_next_thread(room.channel, room.planner, Some(600), None)
        .await
        .unwrap()
        .filter(|t| t.id == parent)
        .and_then(|t| t.claim_lease_id);
    if let Some(new_lease) = reclaimed {
        let other_claim = submit(
            store.as_ref(),
            parent,
            room.planner,
            new_lease,
            vec![child("a", &[]), child("b", &["a"])],
        )
        .await
        .unwrap();
        assert_eq!(other_claim, PlanOutcome::Conflict(first));
    }
    assert_eq!(children_of(store.as_ref(), &room, parent).await, 2);
}

/// Only the planner holding the live claim, under that claim's lease, on a
/// thread that is a plan.
async fn only_the_claim_holder_plans(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "holder").await;
    let (parent, lease) = claimed_plan(store.as_ref(), &room).await;
    let plan = || vec![child("a", &[])];

    let not_holder = submit(store.as_ref(), parent, room.worker, lease, plan())
        .await
        .unwrap_err();
    assert!(not_holder.contains("live claim"), "{not_holder}");
    let stale = submit(
        store.as_ref(),
        parent,
        room.planner,
        ClaimLeaseId(uuid::Uuid::now_v7()),
        plan(),
    )
    .await
    .unwrap_err();
    assert!(stale.contains("live claim"), "{stale}");
    assert_eq!(children_of(store.as_ref(), &room, parent).await, 0);

    // A thread that is not a plan takes no children, even from its holder.
    let change = store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some("a change".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let held = store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.id, change.id);
    let not_plan = submit(
        store.as_ref(),
        change.id,
        room.worker,
        held.claim_lease_id.unwrap(),
        plan(),
    )
    .await
    .unwrap_err();
    assert!(not_plan.contains("not a plan"), "{not_plan}");
}

/// One workspace's allowlist entry does not let another workspace's plan
/// name that repository, and neither plan sees the other's children.
async fn another_workspace_allowlist_does_not_count(store: Arc<dyn Store>) {
    let ours = room(store.as_ref(), "ours").await;
    let theirs = room(store.as_ref(), "theirs").await;
    allow_repo(store.as_ref(), theirs.ws, REPO).await;
    let (parent, lease) = claimed_plan(store.as_ref(), &ours).await;
    let err = submit(
        store.as_ref(),
        parent,
        ours.planner,
        lease,
        vec![in_repo(child("a", &[]), REPO, "db")],
    )
    .await
    .unwrap_err();
    assert!(err.contains("allowlist"), "{err}");

    let (their_parent, their_lease) = claimed_plan(store.as_ref(), &theirs).await;
    created(
        submit(
            store.as_ref(),
            their_parent,
            theirs.planner,
            their_lease,
            vec![in_repo(child("a", &[]), REPO, "db")],
        )
        .await
        .unwrap(),
    );
    assert_eq!(store.get_plan_state(parent).await.unwrap(), None);
    // Their planner cannot plan our parent, whatever lease it names.
    let cross = submit(
        store.as_ref(),
        parent,
        theirs.planner,
        lease,
        vec![child("a", &[])],
    )
    .await
    .unwrap_err();
    assert!(cross.contains("live claim"), "{cross}");
}

/// The spawn budget counts the whole plan: one that would take the parent
/// past `max_children` makes nothing.
async fn the_spawn_budget_counts_the_whole_plan(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "budget").await;
    store
        .set_spawn_budget(room.ws, Some(2), None, None)
        .await
        .unwrap();
    let (parent, lease) = claimed_plan(store.as_ref(), &room).await;
    let err = submit(
        store.as_ref(),
        parent,
        room.planner,
        lease,
        vec![child("a", &[]), child("b", &[]), child("c", &[])],
    )
    .await
    .unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(children_of(store.as_ref(), &room, parent).await, 0);
    created(
        submit(
            store.as_ref(),
            parent,
            room.planner,
            lease,
            vec![child("a", &[]), child("b", &[])],
        )
        .await
        .unwrap(),
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

/// Land `thread` as the PR sweep does: a result, a delivered round, the pull
/// request merged, then the settle that closes it with `ThreadLanded`.
async fn land(store: &dyn Store, room: &Room, thread: ThreadId, pr: i64) -> Vec<StoredEvent> {
    store
        .set_thread_result(thread, room.worker, &serde_json::json!({"summary": "done"}))
        .await
        .unwrap();
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread,
            repo: REPO.into(),
            pull_number: pr,
            commit_sha: format!("{pr:040x}"),
            base_sha: "a".repeat(40),
            base_behind: None,
            diff_lines: 1,
            paths: Vec::new(),
            base_branch: None,
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .unwrap();
    let at = chrono::Utc::now();
    store
        .close_change_pull(REPO, pr, Some(at), Some("c0ffee"), at)
        .await
        .unwrap();
    match store.settle_change_pull(REPO, pr).await.unwrap() {
        ChangeSettlement::Landed(stored) => stored,
        other => panic!("a merged pull request lands its thread, got {other:?}"),
    }
}

/// Close `thread` by hand: through review, with no pull request merged.
async fn close_unlanded(store: &dyn Store, thread: ThreadId, by: MemberId) -> Vec<StoredEvent> {
    store
        .transition_thread_with_event(thread, by, ThreadAction::StartReview)
        .await
        .unwrap();
    store
        .transition_thread_with_event(thread, by, ThreadAction::Close)
        .await
        .unwrap()
        .1
}

/// The `notice` names of the system messages on `thread`.
async fn notices(store: &dyn Store, thread: ThreadId) -> Vec<String> {
    store
        .list_messages(thread, 50)
        .await
        .unwrap()
        .iter()
        .filter_map(|m| m.metadata.get("notice")?.as_str().map(str::to_string))
        .collect()
}

/// A child closed without landing blocks what depends on it, so the queue
/// never builds `b` on an `a` that never merged; the plan stays open with
/// one message, and `plan_state` counts the block.
async fn a_dependent_of_an_unlanded_child_is_never_claimed(store: Arc<dyn Store>) {
    let r = room(store.as_ref(), "plan-unlanded").await;
    let (parent, lease) = claimed_plan(store.as_ref(), &r).await;
    let kids = created(
        submit(
            store.as_ref(),
            parent,
            r.planner,
            lease,
            vec![child("a", &[]), child("b", &["a"]), child("c", &[])],
        )
        .await
        .unwrap(),
    );
    let (a, b, c) = (id(&kids, "a"), id(&kids, "b"), id(&kids, "c"));

    // The plan is still open under its planner's claim; its child moves on
    // regardless.
    let stored = close_unlanded(store.as_ref(), a, r.worker).await;
    assert!(
        stored.iter().any(|e| e.kind == EventKind::ThreadBlocked
            && e.payload["thread_id"] == serde_json::json!(b.0)),
        "the dependent's block is published with the close"
    );

    let block = store
        .get_thread_block(b)
        .await
        .unwrap()
        .expect("b is blocked");
    assert_eq!(block.reason, BlockedReason::Dag);
    assert!(
        block
            .note
            .as_deref()
            .is_some_and(|n| n.contains("closed without landing")),
        "the block names why: {:?}",
        block.note
    );
    assert_eq!(notices(store.as_ref(), b).await, ["dependency_unlanded"]);
    assert!(store.get_thread_block(c).await.unwrap().is_none());

    let first = store
        .claim_next_thread(r.channel, r.worker, Some(600), None)
        .await
        .unwrap()
        .expect("c is free");
    assert_eq!(first.id, c);
    assert_eq!(
        store
            .claim_next_thread(r.channel, r.worker, Some(600), None)
            .await
            .unwrap()
            .map(|t| t.id),
        None,
        "b depends on work that never merged"
    );

    assert_eq!(
        store.get_thread(parent).await.unwrap().state,
        ThreadState::Open
    );
    assert_eq!(
        notices(store.as_ref(), parent).await,
        ["plan_child_unlanded"]
    );
    assert_eq!(
        store.get_plan_state(parent).await.unwrap(),
        Some(PlanState {
            total: 3,
            closed: 1,
            open: 2,
            blocked: 1,
        })
    );
}

/// The plan closes with its last landed child, in that child's transaction,
/// with a `ThreadLanded` naming the pull request that finished it.
async fn the_last_landed_child_closes_the_plan(store: Arc<dyn Store>) {
    let r = room(store.as_ref(), "plan-lands").await;
    let (parent, lease) = claimed_plan(store.as_ref(), &r).await;
    let kids = created(
        submit(
            store.as_ref(),
            parent,
            r.planner,
            lease,
            vec![child("a", &[]), child("b", &["a"])],
        )
        .await
        .unwrap(),
    );

    let first = land(store.as_ref(), &r, id(&kids, "a"), 11).await;
    assert!(first.iter().all(|e| e.thread_id != Some(parent)));
    assert_eq!(
        store.get_thread(parent).await.unwrap().state,
        ThreadState::Open
    );
    assert!(store
        .get_thread_block(id(&kids, "b"))
        .await
        .unwrap()
        .is_none());

    let last = land(store.as_ref(), &r, id(&kids, "b"), 12).await;
    let landed = last
        .iter()
        .find(|e| e.kind == EventKind::ThreadLanded && e.thread_id == Some(parent))
        .expect("the plan lands with its last child");
    assert_eq!(landed.payload["pr_number"], 12);
    assert_eq!(
        store.get_thread(parent).await.unwrap().state,
        ThreadState::Closed
    );
    let messages = store.list_messages(parent, 50).await.unwrap();
    let [summary] = messages.as_slice() else {
        panic!("one message on the plan, got {}", messages.len());
    };
    assert_eq!(summary.metadata["notice"], "plan_landed");
    assert!(
        summary.body.contains("do a: example/app#11"),
        "{}",
        summary.body
    );
    assert!(
        summary.body.contains("do b: example/app#12"),
        "{}",
        summary.body
    );
    assert_eq!(
        store.get_plan_state(parent).await.unwrap(),
        Some(PlanState {
            total: 2,
            closed: 2,
            open: 0,
            blocked: 0,
        })
    );
}

/// An unlanded child keeps the plan open even after every other child
/// lands: nothing merged stands in for it.
async fn an_unlanded_child_keeps_the_plan_open(store: Arc<dyn Store>) {
    let r = room(store.as_ref(), "plan-partial").await;
    let (parent, lease) = claimed_plan(store.as_ref(), &r).await;
    let kids = created(
        submit(
            store.as_ref(),
            parent,
            r.planner,
            lease,
            vec![child("a", &[]), child("c", &[])],
        )
        .await
        .unwrap(),
    );
    close_unlanded(store.as_ref(), id(&kids, "a"), r.worker).await;
    let stored = land(store.as_ref(), &r, id(&kids, "c"), 21).await;
    assert!(stored.iter().all(|e| e.thread_id != Some(parent)));
    assert_eq!(
        store.get_thread(parent).await.unwrap().state,
        ThreadState::Open
    );
    assert_eq!(
        notices(store.as_ref(), parent).await,
        ["plan_child_unlanded"]
    );
}

/// The plan's own close goes through every gate. When one refuses, the
/// child's landing stands, the plan stays open, and it says why once.
async fn a_gate_refusal_leaves_the_plan_open(store: Arc<dyn Store>) {
    let r = room(store.as_ref(), "plan-gated").await;
    let (parent, lease) = claimed_plan(store.as_ref(), &r).await;
    store.set_review_requirement(parent, 1).await.unwrap();
    let kids = created(
        submit(
            store.as_ref(),
            parent,
            r.planner,
            lease,
            vec![child("only", &[])],
        )
        .await
        .unwrap(),
    );
    let only = id(&kids, "only");
    let stored = land(store.as_ref(), &r, only, 31).await;
    assert!(stored
        .iter()
        .any(|e| e.kind == EventKind::ThreadLanded && e.thread_id == Some(only)));
    assert!(!stored
        .iter()
        .any(|e| e.kind == EventKind::ThreadLanded && e.thread_id == Some(parent)));
    assert_eq!(
        store.get_thread(only).await.unwrap().state,
        ThreadState::Closed
    );
    let plan = store.get_thread(parent).await.unwrap();
    assert_eq!(plan.state, ThreadState::Open);
    assert_eq!(
        plan.assignee_id,
        Some(r.planner),
        "the planner's claim stands"
    );
    assert_eq!(
        notices(store.as_ref(), parent).await,
        ["plan_close_refused"]
    );
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
    a_plan_makes_every_child_with_its_edges,
    a_bad_child_creates_nothing,
    a_second_plan_replays_or_conflicts,
    only_the_claim_holder_plans,
    another_workspace_allowlist_does_not_count,
    the_spawn_budget_counts_the_whole_plan,
    a_dependent_of_an_unlanded_child_is_never_claimed,
    the_last_landed_child_closes_the_plan,
    an_unlanded_child_keeps_the_plan_open,
    a_gate_refusal_leaves_the_plan_open,
);
