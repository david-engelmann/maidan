//! The claim reaper charges wall time: a claim freed because its lease lapsed
//! pays for the time it worked, on both backends.
//!
//! A hung agent never calls `report_usage`, the only place `max_wall_secs` was
//! checked, so a thread whose claims kept hanging went round the queue
//! forever. The reaper (and the reclaim inside `claim_next`) now charges the
//! freed claim's worked time, from its acknowledgement to its lease deadline,
//! to the thread's `used_wall_secs` in the transaction that frees it, and a
//! claim that leaves the thread over budget ends with `ClaimFailed` and a DLQ
//! entry, as a report over budget would. An unacknowledged claim has no
//! working clock and is charged nothing.
//!
//! Every deadline here comes from the store: the reaper is handed a `now` just
//! past the claim's own `assignment_expires_at`, and the expected charge is
//! read off the stored deadline and `work_started_at`, so the host's clock
//! never meets the Postgres container's.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BudgetLimits, BudgetPatch, ChannelId, Event, EventKind, MemberId, MemberKind, NewChannel,
    NewMember, NewThread, NewWorkspace, StoredEvent, Thread, ThreadId, UsageDelta,
};
use sqlx::sqlite::SqlitePoolOptions;

struct Queue {
    channel: ChannelId,
    thread: ThreadId,
    holder: MemberId,
    next: MemberId,
}

/// A workspace with one channel, one thread and two agents. The thread gets a
/// wall budget of `max_wall_secs` when one is given.
async fn queue(store: &dyn Store, name: &str, max_wall_secs: Option<i64>) -> Queue {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap();
    let mut members = Vec::new();
    for handle in ["holder", "next"] {
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
    if let Some(max) = max_wall_secs {
        store
            .set_thread_budget(
                thread.id,
                BudgetLimits {
                    max_wall_secs: Some(max),
                    ..BudgetLimits::default()
                },
            )
            .await
            .unwrap();
    }
    Queue {
        channel: channel.id,
        thread: thread.id,
        holder: members[0],
        next: members[1],
    }
}

/// Claim the channel's next thread on a `lease_secs` lease and, when
/// `acknowledge`, start its working clock: an agent that then hangs.
async fn hang(
    store: &dyn Store,
    channel: ChannelId,
    member: MemberId,
    lease_secs: i64,
    acknowledge: bool,
) -> Thread {
    let held = store
        .claim_next_thread(channel, member, Some(lease_secs))
        .await
        .unwrap()
        .expect("a claimable thread");
    if !acknowledge {
        return held;
    }
    store
        .acknowledge_claim(held.id, member, held.claim_lease_id.unwrap())
        .await
        .unwrap()
}

/// What the claim should be charged: acknowledgement to deadline, on the
/// store's own timestamps.
fn worked(held: &Thread) -> i64 {
    held.work_started_at.map_or(0, |started| {
        (held.assignment_expires_at.unwrap() - started)
            .num_seconds()
            .max(0)
    })
}

/// Just past the claims' deadlines, so the reaper sees every one lapsed.
fn past(claims: &[&Thread]) -> DateTime<Utc> {
    claims
        .iter()
        .map(|t| t.assignment_expires_at.unwrap())
        .max()
        .unwrap()
        + Duration::seconds(1)
}

/// Reap everything lapsed by `now`, keeping the events for `thread`. The
/// reaper is global and a sweep ahead of real time also takes other tests'
/// live claims, so each test looks only at its own threads.
async fn reap_for(store: &dyn Store, now: DateTime<Utc>, thread: ThreadId) -> Vec<StoredEvent> {
    let mut mine = Vec::new();
    loop {
        let events = store.reap_expired_claims(now, 100).await.unwrap();
        let done = events.len() < 100;
        mine.extend(events.into_iter().filter(|e| e.thread_id == Some(thread)));
        if done {
            return mine;
        }
    }
}

async fn used_wall_secs(store: &dyn Store, thread: ThreadId) -> i64 {
    store
        .get_thread_budget(thread)
        .await
        .unwrap()
        .map_or(0, |b| b.used_wall_secs)
}

async fn a_hung_claim_past_its_wall_budget_fails_and_is_dead_lettered(store: &dyn Store) {
    let q = queue(store, "over", Some(60)).await;
    let held = hang(store, q.channel, q.holder, 3600, true).await;
    let charge = worked(&held);
    assert!(charge >= 60, "the claim worked past the budget: {charge}s");

    let events = reap_for(store, past(&[&held]), q.thread).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ClaimFailed],
        "one stop, and no ClaimExpired beside it"
    );
    let failed: Event = serde_json::from_value(events[0].payload.clone()).unwrap();
    let Event::ClaimFailed {
        member_id,
        reason,
        thread,
        ..
    } = failed
    else {
        panic!("not a ClaimFailed: {failed:?}");
    };
    assert_eq!(member_id, q.holder, "names the hung holder");
    assert_eq!(reason, "wall");
    assert_eq!(thread.assignee_id, None, "the snapshot is the freed thread");

    let dlq = store.list_channel_dlq(q.channel, 10).await.unwrap();
    assert_eq!(dlq.len(), 1, "the run is dead-lettered");
    assert_eq!(dlq[0].thread_id, q.thread);
    assert_eq!(dlq[0].member_id, q.holder);
    assert_eq!(dlq[0].reason, "wall");

    assert_eq!(used_wall_secs(store, q.thread).await, charge);
    let freed = store.get_thread(q.thread).await.unwrap();
    assert_eq!(freed.assignee_id, None);
    assert_eq!(freed.claim_lease_id, None);
    assert_eq!(freed.work_started_at, None);
}

async fn a_hung_claim_under_its_wall_budget_is_charged_and_requeued(store: &dyn Store) {
    let q = queue(store, "under", Some(7200)).await;
    let held = hang(store, q.channel, q.holder, 3600, true).await;
    let charge = worked(&held);
    assert!(charge > 0 && charge < 7200, "{charge}s");

    let events = reap_for(store, past(&[&held]), q.thread).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ClaimExpired]
    );
    assert_eq!(used_wall_secs(store, q.thread).await, charge, "charged");
    assert!(store
        .list_channel_dlq(q.channel, 10)
        .await
        .unwrap()
        .is_empty());

    // Back in the queue, and the next run starts with the time already spent.
    let retry = hang(store, q.channel, q.next, 3600, true).await;
    assert_eq!(retry.id, q.thread, "the work is requeued");
    store
        .patch_thread_budget(
            q.thread,
            BudgetPatch {
                max_wall_secs: Some(Some(charge)),
                ..BudgetPatch::default()
            },
        )
        .await
        .unwrap();
    let (report, stored) = store
        .report_thread_usage(q.thread, UsageDelta::default())
        .await
        .unwrap();
    assert!(
        report.stopped,
        "the charged time alone reaches the lowered cap"
    );
    assert_eq!(report.reason.as_deref(), Some("wall"));
    assert_eq!(stored.map(|e| e.kind), Some(EventKind::ClaimFailed));
}

async fn an_unacknowledged_claim_is_charged_nothing(store: &dyn Store) {
    let q = queue(store, "unacked", Some(60)).await;
    let held = hang(store, q.channel, q.holder, 3600, false).await;
    assert_eq!(held.work_started_at, None);

    let events = reap_for(store, past(&[&held]), q.thread).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ClaimExpired],
        "an hour held, but never started: not a budget stop"
    );
    assert_eq!(used_wall_secs(store, q.thread).await, 0);
    assert!(store
        .list_channel_dlq(q.channel, 10)
        .await
        .unwrap()
        .is_empty());
}

async fn a_lapsed_claim_is_charged_once(store: &dyn Store) {
    let q = queue(store, "once", Some(7200)).await;
    let held = hang(store, q.channel, q.holder, 3600, true).await;
    let now = past(&[&held]);

    assert_eq!(reap_for(store, now, q.thread).await.len(), 1);
    assert!(
        reap_for(store, now, q.thread).await.is_empty(),
        "a second sweep finds nothing"
    );
    let (taken, events) = store
        .claim_next_thread_with_event(q.channel, q.next, Some(60))
        .await
        .unwrap();
    assert_eq!(taken.map(|t| t.id), Some(q.thread));
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ThreadAssignmentChanged],
        "the next claim ends nothing"
    );
    assert_eq!(used_wall_secs(store, q.thread).await, worked(&held));
}

/// A lapsed claim already over its wall budget is not handed out. `claim_next`
/// leaves it for the reaper, which charges it and stops it; raising the budget
/// puts the thread back in the queue.
async fn a_reclaim_by_claim_next_charges_as_the_reaper_would(store: &dyn Store) {
    const LEASE_SECS: i64 = 3;
    let q = queue(store, "reclaim", Some(1)).await;
    let held = hang(store, q.channel, q.holder, LEASE_SECS, true).await;
    let charge = worked(&held);
    assert!(charge >= 1, "the claim worked past the budget: {charge}s");
    tokio::time::sleep(std::time::Duration::from_millis(
        (LEASE_SECS as u64) * 1000 + 300,
    ))
    .await;

    let (taken, events) = store
        .claim_next_thread_with_event(q.channel, q.next, Some(3600))
        .await
        .unwrap();
    assert!(taken.is_none(), "a thread over budget is not handed out");
    assert!(events.is_empty());
    assert_eq!(
        store.get_thread(q.thread).await.unwrap().assignee_id,
        Some(q.holder),
        "the dead holder keeps it until the reaper"
    );

    let events = reap_for(store, past(&[&held]), q.thread).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ClaimFailed]
    );
    let failed: Event = serde_json::from_value(events[0].payload.clone()).unwrap();
    assert_eq!(failed.member_id(), Some(q.holder), "names the dead holder");
    assert_eq!(used_wall_secs(store, q.thread).await, charge);
    assert_eq!(
        store.list_channel_dlq(q.channel, 10).await.unwrap().len(),
        1
    );

    let (still, _) = store
        .claim_next_thread_with_event(q.channel, q.next, Some(3600))
        .await
        .unwrap();
    assert!(still.is_none(), "the charge left it over budget");

    store
        .patch_thread_budget(
            q.thread,
            BudgetPatch {
                max_wall_secs: Some(Some(charge + 1)),
                ..BudgetPatch::default()
            },
        )
        .await
        .unwrap();
    let (raised, events) = store
        .claim_next_thread_with_event(q.channel, q.next, Some(3600))
        .await
        .unwrap();
    assert_eq!(
        raised.map(|t| t.id),
        Some(q.thread),
        "raising the budget requeues it"
    );
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ThreadAssignmentChanged]
    );
}

async fn two_tenants_budgets_never_mix(store: &dyn Store) {
    let a = queue(store, "tenant-a", Some(60)).await;
    let b = queue(store, "tenant-b", Some(7200)).await;
    let held_a = hang(store, a.channel, a.holder, 3600, true).await;
    let held_b = hang(store, b.channel, b.holder, 1800, true).await;
    let now = past(&[&held_a, &held_b]);

    let mut kinds = HashMap::new();
    loop {
        let events = store.reap_expired_claims(now, 100).await.unwrap();
        let done = events.len() < 100;
        for e in events {
            if let Some(t) = e.thread_id.filter(|t| *t == a.thread || *t == b.thread) {
                assert!(kinds.insert(t, e.kind).is_none(), "{t:?} reported twice");
            }
        }
        if done {
            break;
        }
    }
    assert_eq!(kinds.get(&a.thread), Some(&EventKind::ClaimFailed));
    assert_eq!(kinds.get(&b.thread), Some(&EventKind::ClaimExpired));
    assert_eq!(used_wall_secs(store, a.thread).await, worked(&held_a));
    assert_eq!(
        used_wall_secs(store, b.thread).await,
        worked(&held_b),
        "B pays for its own claim only"
    );
    assert_eq!(
        store.list_channel_dlq(a.channel, 10).await.unwrap().len(),
        1
    );
    assert!(
        store
            .list_channel_dlq(b.channel, 10)
            .await
            .unwrap()
            .is_empty(),
        "A's stop is not in B's dead letters"
    );
}

/// Reapers on several replicas sweep at once: every lapsed claim is charged
/// exactly once.
async fn concurrent_reapers_charge_each_claim_once(store: Arc<dyn Store>) {
    const THREADS: usize = 30;
    let q = queue(store.as_ref(), "replicas", None).await;
    for i in 1..THREADS {
        store
            .create_thread(NewThread {
                channel_id: q.channel,
                parent_thread_id: None,
                title: Some(format!("task {i}")),
            })
            .await
            .unwrap();
    }
    let mut held = Vec::new();
    for _ in 0..THREADS {
        let claim = hang(store.as_ref(), q.channel, q.holder, 3600, true).await;
        store
            .set_thread_budget(
                claim.id,
                BudgetLimits {
                    max_wall_secs: Some(7200),
                    ..BudgetLimits::default()
                },
            )
            .await
            .unwrap();
        held.push(claim);
    }
    let now = past(&held.iter().collect::<Vec<_>>());
    let mine: HashSet<ThreadId> = held.iter().map(|t| t.id).collect();

    let reapers = (0..4).map(|_| {
        let store = store.clone();
        let mine = mine.clone();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            loop {
                let events = store.reap_expired_claims(now, 3).await.unwrap();
                if events.is_empty() {
                    return seen;
                }
                seen.extend(
                    events
                        .into_iter()
                        .filter_map(|e| e.thread_id)
                        .filter(|t| mine.contains(t)),
                );
            }
        })
    });
    let mut reported = HashSet::new();
    for reaper in reapers {
        for id in reaper.await.unwrap() {
            assert!(reported.insert(id), "thread {id:?} reported twice");
        }
    }
    assert_eq!(reported, mine, "every lapse reported");
    for claim in &held {
        assert_eq!(
            used_wall_secs(store.as_ref(), claim.id).await,
            worked(claim),
            "thread {:?} charged once",
            claim.id
        );
    }
}

async fn run_suite(store: Arc<dyn Store>) {
    a_hung_claim_past_its_wall_budget_fails_and_is_dead_lettered(store.as_ref()).await;
    a_hung_claim_under_its_wall_budget_is_charged_and_requeued(store.as_ref()).await;
    an_unacknowledged_claim_is_charged_nothing(store.as_ref()).await;
    a_lapsed_claim_is_charged_once(store.as_ref()).await;
    a_reclaim_by_claim_next_charges_as_the_reaper_would(store.as_ref()).await;
    two_tenants_budgets_never_mix(store.as_ref()).await;
    concurrent_reapers_charge_each_claim_once(store).await;
}

#[tokio::test]
async fn reaper_charges_wall_budget_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(Arc::new(SqliteStore::for_tests(pool))).await;
}

#[tokio::test]
async fn reaper_charges_wall_budget_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
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
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(Arc::new(PostgresStore::for_tests(pool))).await;
}
