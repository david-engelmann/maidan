//! Task attributes on both backends: set at creation in the create
//! transaction, replaced later, read back as the defaults when a thread was
//! never described, carried on the live claims list, and respected by an
//! engine-filtered claim. The metrics list filters and splits on them.
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
        ..TaskAttributes::default()
    }
}

/// Attributes declared at intake are stored with the thread, and a thread
/// created without any reads back as the defaults.
async fn attributes_set_at_creation_are_read_back(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "intake", 1).await;
    let described = attrs(TaskKind::Review, Some(TaskEngine::Human), true);
    let (thread, _event) = store
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
        described
    );

    let (plain, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "plain"), None, None, None)
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
            None,
        )
        .await
        .unwrap();
    let replaced = attrs(TaskKind::Plan, Some(TaskEngine::Session), true);
    let stored = store
        .set_thread_attributes(thread.id, replaced.clone())
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
                None,
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
            Some(described.clone()),
            None,
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
            Some(described.clone()),
            None,
        )
        .await
        .unwrap();
    let (plain, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "plain task"), None, None, None)
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
                rebase: false,
                previous_head: None,
                linear_issue: None,
            })
            .await
            .unwrap();
    }

    let all = store
        .list_change_pulls(room.ws, None, None, None, None, None)
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
        .list_change_pulls(room.ws, None, None, Some(TaskEngine::Pi), None, None)
        .await
        .unwrap();
    assert_eq!(pi_only.len(), 1);
    assert_eq!(pi_only[0].thread_id, review.id);
    let changes = store
        .list_change_pulls(room.ws, None, None, None, Some(TaskKind::Change), None)
        .await
        .unwrap();
    assert_eq!(changes.len(), 1, "the undescribed task counts as a change");
    assert_eq!(changes[0].thread_id, plain.id);

    let metrics =
        maidan_types::swarm_metrics(all, maidan_types::FlagLimits::default(), chrono::Utc::now());
    let rollup = &metrics.repos[0];
    assert_eq!(rollup.by_engine.len(), 2);
    assert_eq!(rollup.by_engine[0].engine, Some(TaskEngine::Pi));
    assert_eq!(rollup.by_engine[0].numbers.tasks, 1);
    assert_eq!(rollup.by_engine[1].engine, None);
    assert_eq!(rollup.by_engine[1].numbers.tasks, 1);
}

/// A task with model spend reads its cost back, with the provider and the
/// harness its usage named; usage that named none still counts toward the
/// cost. On Postgres a SUM over a BIGINT is NUMERIC: read as an i64 it
/// panicked, and took the change sweep down on its first tick.
async fn a_tasks_model_spend_reads_back_as_its_cost(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "spend", 1).await;
    let agent = room.agents[0];
    let (thread, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "spent"), None, None, None)
        .await
        .unwrap();
    let (claimed, _event) = store
        .claim_thread_with_event(thread.id, agent)
        .await
        .unwrap();
    let lease = claimed.thread.claim_lease_id.unwrap();
    store
        .report_accounted_usage(&NewUsageLedgerEntry {
            usage_report_id: uuid::Uuid::new_v4(),
            thread_id: thread.id,
            reporter: agent,
            claim_lease_id: lease,
            model: "provider/model".into(),
            tokens: TokenUsage {
                input: 1_000_000,
                ..Default::default()
            },
            usd_micros: 10,
            price_snapshot: PriceSnapshot {
                input_usd_micros_per_million: 10,
                ..Default::default()
            },
            turns: 1,
            evidence: maidan_types::UsageEvidence {
                provider: Some("openai".into()),
                harness: Some("codex".into()),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    store
        .report_accounted_usage(&NewUsageLedgerEntry {
            usage_report_id: uuid::Uuid::new_v4(),
            thread_id: thread.id,
            reporter: agent,
            claim_lease_id: lease,
            model: "provider/model".into(),
            tokens: TokenUsage {
                input: 500_000,
                ..Default::default()
            },
            usd_micros: 5,
            price_snapshot: PriceSnapshot {
                input_usd_micros_per_million: 10,
                ..Default::default()
            },
            turns: 1,
            evidence: Default::default(),
        })
        .await
        .unwrap();
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread.id,
            repo: "example/app".into(),
            pull_number: 9,
            commit_sha: "c".repeat(40),
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
    let pulls = store
        .list_change_pulls(room.ws, None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(pulls[0].cost_usd_micros, 15);
    assert_eq!(pulls[0].provider.as_deref(), Some("openai"));
    assert_eq!(pulls[0].harness.as_deref(), Some("codex"));

    // The claim is in the flow, and a skipped delivery counts. The event and
    // delivery reads run on both backends here.
    assert!(pulls[0].flow.claimed_at.is_some(), "{:?}", pulls[0].flow);
    assert_eq!(pulls[0].flow.deliveries_skipped, 0);
    let row = store
        .arm_unroutable_result_delivery(thread.id, "nowhere", "x", chrono::Utc::now())
        .await
        .unwrap()
        .expect("armed");
    store
        .mark_result_delivery_skipped(row.id, "unknown surface")
        .await
        .unwrap();
    let pulls = store
        .list_change_pulls(room.ws, None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(pulls[0].flow.deliveries_skipped, 1);
    assert_eq!(
        pulls[0].first_delivery_ci, None,
        "no CI notice names the head"
    );
    let metrics = maidan_types::swarm_metrics(
        pulls,
        maidan_types::FlagLimits::default(),
        chrono::Utc::now(),
    );
    assert!(metrics.tasks[0].claim_to_first_delivery_secs.is_some());
    assert_eq!(metrics.repos[0].totals.deliveries_skipped, 1);
}

/// A pull request keeps the first Linear issue a round names: a later round
/// without one, or naming another, changes nothing. A round another
/// workspace delivers to the same pull request never writes it.
async fn a_change_pull_keeps_the_first_linear_issue_a_round_names(store: Arc<dyn Store>) {
    let mine = room(store.as_ref(), "linear-mine", 0).await;
    let theirs = room(store.as_ref(), "linear-theirs", 0).await;
    let (thread, _event) = store
        .create_thread_with_footprint(new_thread(mine.channel, "linked"), None, None, None)
        .await
        .unwrap();
    let (other, _event) = store
        .create_thread_with_footprint(new_thread(theirs.channel, "elsewhere"), None, None, None)
        .await
        .unwrap();
    let round = |ws: WorkspaceId, thread_id: ThreadId, n: u8, key: Option<&str>| NewChangeRound {
        workspace_id: ws,
        thread_id,
        repo: "example/app".into(),
        pull_number: 41,
        commit_sha: format!("{n:x}").repeat(40),
        base_sha: "a".repeat(40),
        base_behind: None,
        diff_lines: 1,
        paths: Vec::new(),
        base_branch: None,
        rebase: false,
        previous_head: None,
        linear_issue: key.map(str::to_string),
    };
    let key = |store: Arc<dyn Store>| async move {
        store
            .list_change_pulls(mine.ws, None, None, None, None, None)
            .await
            .unwrap()
            .into_iter()
            .next()
            .and_then(|pull| pull.linear_issue)
    };
    store
        .record_change_round(round(mine.ws, thread.id, 1, None))
        .await
        .unwrap();
    assert_eq!(key(store.clone()).await, None);
    store
        .record_change_round(round(theirs.ws, other.id, 2, Some("OPS-9")))
        .await
        .unwrap();
    assert_eq!(key(store.clone()).await, None, "another workspace's round");
    store
        .record_change_round(round(mine.ws, thread.id, 3, Some("ENG-1234")))
        .await
        .unwrap();
    assert_eq!(key(store.clone()).await.as_deref(), Some("ENG-1234"));
    for (n, later) in [(4, None), (5, Some("ENG-9"))] {
        store
            .record_change_round(round(mine.ws, thread.id, n, later))
            .await
            .unwrap();
        assert_eq!(key(store.clone()).await.as_deref(), Some("ENG-1234"));
    }
}

/// Size and acceptance round-trip at creation and on replace; acceptance is
/// trimmed (blank reads as none) and refused past its limit, creating
/// nothing.
async fn size_and_acceptance_are_stored_and_checked(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "sizing", 1).await;
    let sized = TaskAttributes {
        size: Some(TaskSize::M),
        acceptance: Some("  The bridge retries twice, and its test proves it.  ".into()),
        ..attrs(TaskKind::Change, Some(TaskEngine::Pi), false)
    };
    let (thread, _) = store
        .create_thread_with_footprint(new_thread(room.channel, "sized"), None, Some(sized), None)
        .await
        .unwrap();
    let read = store.get_thread_attributes(thread.id).await.unwrap();
    assert_eq!(read.size, Some(TaskSize::M));
    assert_eq!(
        read.acceptance.as_deref(),
        Some("The bridge retries twice, and its test proves it.")
    );

    let blank = TaskAttributes {
        acceptance: Some("   ".into()),
        ..read.clone()
    };
    let stored = store
        .set_thread_attributes(thread.id, blank.validated().unwrap())
        .await
        .unwrap();
    assert_eq!(stored.acceptance, None, "blank acceptance reads as none");

    let before = store.list_threads(room.channel).await.unwrap().len();
    let too_long = TaskAttributes {
        acceptance: Some("x".repeat(maidan_types::ACCEPTANCE_MAX_CHARS + 1)),
        ..TaskAttributes::default()
    };
    assert!(store
        .create_thread_with_footprint(new_thread(room.channel, "long"), None, Some(too_long), None)
        .await
        .is_err());
    assert_eq!(
        store.list_threads(room.channel).await.unwrap().len(),
        before
    );
}

/// A priority set at creation is stored with the thread and the claim queue
/// hands it out before older unprioritised work; a priority needs an
/// attributed creator.
async fn a_priority_set_at_creation_is_claimed_first(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "priority", 2).await;
    let older = store
        .create_thread_with_footprint(new_thread(room.channel, "older"), None, None, None)
        .await
        .unwrap()
        .0;
    let urgent_new = NewThread {
        created_by: Some(room.agents[1]),
        ..new_thread(room.channel, "urgent")
    };
    let urgent = store
        .create_thread_with_footprint(urgent_new, None, None, Some(100))
        .await
        .unwrap()
        .0;
    assert_eq!(
        store
            .get_thread_priority(urgent.id)
            .await
            .unwrap()
            .map(|p| p.priority),
        Some(100)
    );
    let claimed = store
        .claim_next_thread(room.channel, room.agents[0], Some(600), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        claimed.id, urgent.id,
        "the urgent task is claimed before the older one"
    );
    assert_ne!(claimed.id, older.id);
    assert!(
        store
            .create_thread_with_footprint(new_thread(room.channel, "anon"), None, None, Some(5))
            .await
            .is_err(),
        "a priority needs an attributed creator"
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

/// A change's tier is stored at intake, carried on its live claim and its
/// pull's metrics row, restated later, and read back unset when a
/// restatement leaves it out: unset is never stored as B.
async fn a_tier_is_stored_restated_and_left_unset(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "tiers", 1).await;
    let tier_a = TaskAttributes {
        tier: Some(TaskTier::A),
        ..attrs(TaskKind::Change, Some(TaskEngine::Pi), false)
    };
    let (thread, _event) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "tiered"),
            Some(Footprint {
                repo: "example/app".into(),
                mode: FootprintMode::Exclusive,
                items: vec![FootprintItem {
                    kind: FootprintKind::Lane,
                    value: "db".into(),
                }],
            }),
            Some(tier_a.clone()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store.get_thread_attributes(thread.id).await.unwrap(),
        tier_a
    );
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
    assert_eq!(entry.attributes.tier, Some(TaskTier::A));
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread.id,
            repo: "example/app".into(),
            pull_number: 7,
            commit_sha: "c".repeat(40),
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
    let pulls = store
        .list_change_pulls(room.ws, None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(pulls[0].attributes.tier, Some(TaskTier::A));

    let tier_b = TaskAttributes {
        tier: Some(TaskTier::B),
        ..tier_a.clone()
    };
    assert_eq!(
        store
            .set_thread_attributes(thread.id, tier_b.clone())
            .await
            .unwrap(),
        tier_b
    );
    let unsaid = TaskAttributes {
        tier: None,
        ..tier_a
    };
    store
        .set_thread_attributes(thread.id, unsaid)
        .await
        .unwrap();
    assert_eq!(
        store.get_thread_attributes(thread.id).await.unwrap().tier,
        None
    );
}

/// A task's service lane (Context Economics C7) is stored at intake, carried
/// on its live claim and its pull's metrics row, restated later, and read back
/// unset when a restatement leaves it out: unset is never stored as a lane.
async fn a_lane_is_stored_restated_and_left_unset(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "lanes", 1).await;
    let batch = TaskAttributes {
        lane: Some(TaskLane::Batch),
        ..attrs(TaskKind::Change, Some(TaskEngine::Pi), false)
    };
    let (thread, _event) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "laned"),
            Some(Footprint {
                repo: "example/app".into(),
                mode: FootprintMode::Exclusive,
                items: vec![FootprintItem {
                    kind: FootprintKind::Lane,
                    value: "db".into(),
                }],
            }),
            Some(batch.clone()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(store.get_thread_attributes(thread.id).await.unwrap(), batch);
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
    assert_eq!(entry.attributes.lane, Some(TaskLane::Batch));
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread.id,
            repo: "example/app".into(),
            pull_number: 8,
            commit_sha: "c".repeat(40),
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
    let pulls = store
        .list_change_pulls(room.ws, None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(pulls[0].attributes.lane, Some(TaskLane::Batch));

    let flex = TaskAttributes {
        lane: Some(TaskLane::Flex),
        ..batch.clone()
    };
    assert_eq!(
        store
            .set_thread_attributes(thread.id, flex.clone())
            .await
            .unwrap(),
        flex
    );
    let unsaid = TaskAttributes {
        lane: None,
        ..batch
    };
    store
        .set_thread_attributes(thread.id, unsaid)
        .await
        .unwrap();
    assert_eq!(
        store.get_thread_attributes(thread.id).await.unwrap().lane,
        None
    );
}

/// A review of a change finishes before the change merges, so a review
/// child moves ahead of its open parent; any other child still may not.
async fn a_review_child_closes_ahead_of_its_open_change(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "review-child", 1).await;
    let actor = room.agents[0];
    let (change, _event) = store
        .create_thread_with_footprint(
            new_thread(room.channel, "change"),
            None,
            Some(attrs(TaskKind::Change, Some(TaskEngine::Pi), false)),
            None,
        )
        .await
        .unwrap();
    let child = |title: &str| NewThread {
        parent_thread_id: Some(change.id),
        ..new_thread(room.channel, title)
    };
    let (review, _event) = store
        .create_thread_with_footprint(
            child("review"),
            None,
            Some(attrs(TaskKind::Review, Some(TaskEngine::Pi), false)),
            None,
        )
        .await
        .unwrap();
    let (other, _event) = store
        .create_thread_with_footprint(child("other"), None, None, None)
        .await
        .unwrap();

    store
        .transition_thread(review.id, actor, ThreadAction::StartReview)
        .await
        .expect("a review child moves ahead of its open change");
    store
        .transition_thread(review.id, actor, ThreadAction::Close)
        .await
        .expect("a review child closes while its change is open");
    assert_eq!(
        store.get_thread(review.id).await.unwrap().state,
        ThreadState::Closed
    );
    assert_eq!(
        store.get_thread(change.id).await.unwrap().state,
        ThreadState::Open
    );

    let held = store
        .transition_thread(other.id, actor, ThreadAction::StartReview)
        .await;
    assert!(
        matches!(held, Err(maidan_store::StoreError::Conflict(_))),
        "a child that is not a review stays behind its parent, got {held:?}"
    );
}

/// An ordinary child cannot be turned into a review after creation, so it
/// stays behind its parent; a child created as a review keeps its kind.
async fn an_ordinary_child_turned_into_a_review_stays_held(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "relabel", 1).await;
    let actor = room.agents[0];
    let (change, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "change"), None, None, None)
        .await
        .unwrap();
    let child = |title: &str| NewThread {
        parent_thread_id: Some(change.id),
        ..new_thread(room.channel, title)
    };
    let (ordinary, _event) = store
        .create_thread_with_footprint(child("ordinary"), None, None, None)
        .await
        .unwrap();
    let relabel = store
        .set_thread_attributes(ordinary.id, attrs(TaskKind::Review, None, false))
        .await;
    assert!(
        matches!(relabel, Err(maidan_store::StoreError::Conflict(_))),
        "an ordinary child cannot become a review, got {relabel:?}"
    );
    assert_eq!(
        store.get_thread_attributes(ordinary.id).await.unwrap().kind,
        TaskKind::default()
    );
    let held = store
        .transition_thread(ordinary.id, actor, ThreadAction::StartReview)
        .await;
    assert!(
        matches!(held, Err(maidan_store::StoreError::Conflict(_))),
        "the relabel attempt leaves the child held, got {held:?}"
    );

    let (review, _event) = store
        .create_thread_with_footprint(
            child("review"),
            None,
            Some(attrs(TaskKind::Review, None, false)),
            None,
        )
        .await
        .unwrap();
    let restated = store
        .set_thread_attributes(
            review.id,
            attrs(TaskKind::Review, Some(TaskEngine::Pi), false),
        )
        .await
        .expect("a review child restates its attributes");
    assert_eq!(restated.kind, TaskKind::Review);

    let (top, _event) = store
        .create_thread_with_footprint(new_thread(room.channel, "top level"), None, None, None)
        .await
        .unwrap();
    let top_review = store
        .set_thread_attributes(top.id, attrs(TaskKind::Review, None, false))
        .await
        .expect("a thread without a parent may become a review");
    assert_eq!(top_review.kind, TaskKind::Review);
}

on_both_backends!(
    attributes_set_at_creation_are_read_back,
    attributes_set_later_replace_the_whole_set,
    an_engine_claim_takes_only_its_own_work,
    a_live_claim_entry_carries_the_attributes,
    the_metrics_carry_filter_and_split_on_attributes,
    size_and_acceptance_are_stored_and_checked,
    a_priority_set_at_creation_is_claimed_first,
    a_change_pull_keeps_the_first_linear_issue_a_round_names,
    a_tasks_model_spend_reads_back_as_its_cost,
    a_tier_is_stored_restated_and_left_unset,
    a_lane_is_stored_restated_and_left_unset,
    a_review_child_closes_ahead_of_its_open_change,
    an_ordinary_child_turned_into_a_review_stays_held,
);
