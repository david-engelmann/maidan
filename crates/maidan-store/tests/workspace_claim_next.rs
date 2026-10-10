//! Workspace-wide `claim_next`: one call hands out the next ready thread across
//! every channel the claimer may read, with the channel route's filters, order,
//! lease and events. A private channel's threads go only to its members, a DM's
//! only to its participants, and nothing crosses a workspace. The DM rule holds
//! on the channel route too: claiming "next in `__dm__`" once handed any member
//! a DM between two others. Both backends.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Duration;

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BlockedReason, BudgetLimits, BudgetPatch, ChannelId, ChannelMemberRole, EventKind, MemberId,
    MemberKind, NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
    WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

const LEASE_SECS: i64 = 600;

async fn sqlite() -> SqliteStore {
    // One connection: every connection to `sqlite::memory:` is its own
    // database, and the concurrency suite would otherwise open a second.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::for_tests(pool)
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace")
        .id
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
        .id
}

async fn channel(store: &dyn Store, ws: WorkspaceId, name: &str, private: bool) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .expect("channel")
        .id
}

async fn thread(store: &dyn Store, channel_id: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id: None,
            title: None,
            description: None,
        })
        .await
        .expect("thread")
        .id
}

async fn claim(store: &dyn Store, ws: WorkspaceId, member_id: MemberId) -> Option<ThreadId> {
    store
        .claim_next_workspace_thread_with_event(ws, member_id, Some(LEASE_SECS))
        .await
        .expect("claim next in workspace")
        .0
        .map(|t| t.id)
}

/// The oldest ready thread wins wherever it is, and the claim is the channel
/// route's: leased, fenced, reported.
async fn takes_the_oldest_ready_thread_across_channels(store: &dyn Store) {
    let ws = workspace(store, "oldest").await;
    let agent = member(store, ws, "agent").await;
    let alpha = channel(store, ws, "alpha", false).await;
    let beta = channel(store, ws, "beta", false).await;
    let first = thread(store, beta).await;
    let second = thread(store, alpha).await;
    let third = thread(store, beta).await;

    let (claimed, events) = store
        .claim_next_workspace_thread_with_event(ws, agent, Some(LEASE_SECS))
        .await
        .expect("claim");
    let claimed = claimed.expect("a thread");
    assert_eq!(
        claimed.id, first,
        "the oldest thread, though in another channel"
    );
    assert_eq!(claimed.assignee_id, Some(agent));
    assert!(claimed.claim_lease_id.is_some(), "a fencing token");
    assert!(claimed.assignment_expires_at.is_some(), "a leased claim");
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ThreadAssignmentChanged]
    );

    assert_eq!(claim(store, ws, agent).await, Some(second));
    assert_eq!(claim(store, ws, agent).await, Some(third));
    assert_eq!(claim(store, ws, agent).await, None, "nothing left");
}

/// A lapsed lease anywhere in the workspace is taken over, and the dead holder
/// is reported first, as on the channel route.
async fn takes_over_a_lapsed_lease_and_reports_it(store: &dyn Store) {
    let ws = workspace(store, "lapsed").await;
    let dead = member(store, ws, "dead").await;
    let next = member(store, ws, "next").await;
    let ch = channel(store, ws, "work", false).await;
    let t = thread(store, ch).await;
    let held = store
        .claim_next_workspace_thread_with_event(ws, dead, Some(-1))
        .await
        .expect("claim")
        .0
        .expect("claimed");

    let (claimed, events) = store
        .claim_next_workspace_thread_with_event(ws, next, Some(LEASE_SECS))
        .await
        .expect("reclaim");
    let claimed = claimed.expect("the lapsed thread");
    assert_eq!(claimed.id, t);
    assert_eq!(claimed.assignee_id, Some(next));
    assert_ne!(
        claimed.claim_lease_id, held.claim_lease_id,
        "a takeover mints a new token"
    );
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ClaimExpired, EventKind::ThreadAssignmentChanged]
    );
}

/// A lapsed claim already over its wall budget is not handed out by the
/// workspace claim. The reaper charges it and stops it; raising the budget
/// puts the thread back in the queue.
async fn a_takeover_charges_the_lapsed_claims_wall_time(store: &dyn Store) {
    const LEASE_SECS: i64 = 2;
    let ws = workspace(store, "wall").await;
    let dead = member(store, ws, "dead").await;
    let next = member(store, ws, "next").await;
    let ch = channel(store, ws, "work", false).await;
    let t = thread(store, ch).await;
    store
        .set_thread_budget(
            t,
            BudgetLimits {
                max_wall_secs: Some(1),
                ..BudgetLimits::default()
            },
        )
        .await
        .expect("budget");
    let held = store
        .claim_next_workspace_thread_with_event(ws, dead, Some(LEASE_SECS))
        .await
        .expect("claim")
        .0
        .expect("claimed");
    let held = store
        .acknowledge_claim(held.id, dead, held.claim_lease_id.expect("token"))
        .await
        .expect("acknowledge");
    tokio::time::sleep(std::time::Duration::from_millis(
        (LEASE_SECS as u64) * 1000 + 300,
    ))
    .await;

    let (claimed, events) = store
        .claim_next_workspace_thread_with_event(ws, next, Some(LEASE_SECS))
        .await
        .expect("takeover");
    assert!(claimed.is_none(), "a thread over budget is not handed out");
    assert!(events.is_empty());
    let worked = (held.assignment_expires_at.expect("deadline")
        - held.work_started_at.expect("started"))
    .num_seconds();
    let now = held.assignment_expires_at.expect("deadline") + Duration::seconds(1);
    loop {
        let batch = store.reap_expired_claims(now, 100).await.expect("reap");
        let freed = store
            .get_thread(t)
            .await
            .expect("thread")
            .assignee_id
            .is_none();
        if freed || batch.len() < 100 {
            break;
        }
    }
    assert_eq!(store.get_thread(t).await.expect("thread").assignee_id, None);
    let budget = store
        .get_thread_budget(t)
        .await
        .expect("budget read")
        .expect("budget");
    assert_eq!(
        budget.used_wall_secs, worked,
        "charged acknowledgement to deadline"
    );
    assert_eq!(store.list_channel_dlq(ch, 10).await.expect("dlq").len(), 1);
    let (still, _) = store
        .claim_next_workspace_thread_with_event(ws, next, Some(LEASE_SECS))
        .await
        .expect("still over");
    assert!(still.is_none());
    store
        .patch_thread_budget(
            t,
            BudgetPatch {
                max_wall_secs: Some(Some(worked + 1)),
                ..BudgetPatch::default()
            },
        )
        .await
        .expect("raise");
    let (raised, _) = store
        .claim_next_workspace_thread_with_event(ws, next, Some(LEASE_SECS))
        .await
        .expect("raised");
    assert_eq!(
        raised.map(|c| c.id),
        Some(t),
        "raising the budget requeues it"
    );
}

/// A private channel's thread goes to its members only; the non-member gets
/// the next thread it may read, or nothing.
async fn a_private_channel_thread_goes_only_to_its_members(store: &dyn Store) {
    let ws = workspace(store, "private").await;
    let insider = member(store, ws, "insider").await;
    let outsider = member(store, ws, "outsider").await;
    let secret = channel(store, ws, "secret", true).await;
    let open = channel(store, ws, "open", false).await;
    store
        .add_channel_member(secret, insider, ChannelMemberRole::Member)
        .await
        .expect("add member");
    let hidden = thread(store, secret).await;
    let public = thread(store, open).await;

    assert_eq!(
        claim(store, ws, outsider).await,
        Some(public),
        "the older private thread is skipped for a non-member"
    );
    assert_eq!(claim(store, ws, outsider).await, None);
    assert_eq!(claim(store, ws, insider).await, Some(hidden));
}

/// DM and group-DM threads follow the thread-read rule: a participant may take
/// one, nobody else is handed it, workspace-wide or through the `__dm__`
/// channel.
async fn a_dm_thread_goes_only_to_its_participants(store: &dyn Store) {
    let ws = workspace(store, "dm").await;
    let alice = member(store, ws, "alice").await;
    let bob = member(store, ws, "bob").await;
    let carol = member(store, ws, "carol").await;
    let dm = store
        .open_dm_conversation(ws, alice, bob)
        .await
        .expect("dm");
    let group = store
        .open_group_dm_conversation(ws, &[alice, bob, carol], None)
        .await
        .expect("group dm");
    let dm_channel = store
        .get_thread(dm.thread_id)
        .await
        .expect("dm thread")
        .channel_id;

    let (through_channel, _) = store
        .claim_next_thread_with_event(dm_channel, carol, Some(LEASE_SECS))
        .await
        .expect("channel claim");
    assert_eq!(
        through_channel.map(|t| t.id),
        Some(group.thread_id),
        "the __dm__ channel hands carol her group DM, never alice and bob's DM"
    );
    assert!(store
        .claim_next_thread(dm_channel, carol, None)
        .await
        .expect("channel claim")
        .is_none());
    assert_eq!(claim(store, ws, carol).await, None);
    assert_eq!(claim(store, ws, alice).await, Some(dm.thread_id));
}

/// Every skip the channel route makes, the workspace route makes: a pending
/// dependency, a missing skill, a pending approval gate, an explicit block, an
/// unclaimable park. Only the newest thread, which has none, is handed out.
async fn skips_threads_that_are_not_ready(store: &dyn Store) {
    let ws = workspace(store, "skips").await;
    let agent = member(store, ws, "agent").await;
    let a = channel(store, ws, "a", false).await;
    let b = channel(store, ws, "b", false).await;
    let c = channel(store, ws, "c", false).await;

    let dependency = thread(store, a).await;
    let waits_on_dependency = thread(store, b).await;
    store
        .add_thread_dependency(waits_on_dependency, dependency)
        .await
        .expect("dependency");
    store
        .set_thread_block(dependency, BlockedReason::Human, agent, None)
        .await
        .expect("block");
    let needs_skill = thread(store, c).await;
    store
        .add_thread_required_skill(needs_skill, "rust")
        .await
        .expect("skill");
    let gated = thread(store, a).await;
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws,
            thread_id: Some(gated),
            requested_by: agent,
            prompt: "ship it?".into(),
            schema: None,
            risk: Default::default(),
        })
        .await
        .expect("gate");
    let parked = thread(store, b).await;
    store
        .mark_thread_unclaimable(parked, "waiting on a person", agent)
        .await
        .expect("park");
    let ready = thread(store, c).await;

    assert_eq!(claim(store, ws, agent).await, Some(ready));
    assert_eq!(claim(store, ws, agent).await, None, "the rest stay put");

    store.add_member_skill(agent, "rust").await.expect("skill");
    assert_eq!(
        claim(store, ws, agent).await,
        Some(needs_skill),
        "the skill was the only thing holding it back"
    );
}

async fn a_frozen_member_is_refused(store: &dyn Store) {
    let ws = workspace(store, "frozen").await;
    let agent = member(store, ws, "agent").await;
    let ch = channel(store, ws, "work", false).await;
    let t = thread(store, ch).await;
    store
        .freeze_member(agent, agent, Some("kill-switch"))
        .await
        .expect("freeze");
    assert_eq!(claim(store, ws, agent).await, None);
    store
        .unfreeze_member(agent, agent)
        .await
        .expect("unfreeze")
        .expect("was frozen");
    assert_eq!(claim(store, ws, agent).await, Some(t));
}

/// A member never takes another workspace's thread, even when the call names
/// that workspace: the store reads the claimer's workspace from its member row.
async fn two_tenants_never_cross(store: &dyn Store) {
    let ws_a = workspace(store, "tenant-a").await;
    let ws_b = workspace(store, "tenant-b").await;
    let agent_a = member(store, ws_a, "agent").await;
    let agent_b = member(store, ws_b, "agent").await;
    let b_thread = thread(store, channel(store, ws_b, "work", false).await).await;
    let a_thread = thread(store, channel(store, ws_a, "work", false).await).await;

    assert_eq!(
        claim(store, ws_b, agent_a).await,
        None,
        "A's member naming B's workspace takes nothing"
    );
    let b_after = store.get_thread(b_thread).await.expect("b thread");
    assert_eq!(b_after.assignee_id, None, "B's thread is untouched");

    assert_eq!(claim(store, ws_a, agent_a).await, Some(a_thread));
    assert_eq!(claim(store, ws_a, agent_a).await, None, "only A's work");
    assert_eq!(claim(store, ws_b, agent_b).await, Some(b_thread));
}

async fn run_suite(store: &dyn Store) {
    takes_the_oldest_ready_thread_across_channels(store).await;
    takes_over_a_lapsed_lease_and_reports_it(store).await;
    a_takeover_charges_the_lapsed_claims_wall_time(store).await;
    a_private_channel_thread_goes_only_to_its_members(store).await;
    a_dm_thread_goes_only_to_its_participants(store).await;
    skips_threads_that_are_not_ready(store).await;
    a_frozen_member_is_refused(store).await;
    two_tenants_never_cross(store).await;
}

/// Claimers racing over one workspace each get distinct threads, and together
/// they drain it.
async fn concurrent_claimers_never_share_a_thread(store: Arc<dyn Store>) {
    const THREADS: usize = 24;
    const CLAIMERS: usize = 6;
    let ws = workspace(store.as_ref(), "race").await;
    let mut channels = Vec::new();
    for name in ["one", "two", "three"] {
        channels.push(channel(store.as_ref(), ws, name, false).await);
    }
    for i in 0..THREADS {
        thread(store.as_ref(), channels[i % channels.len()]).await;
    }
    let mut claimers = Vec::new();
    for i in 0..CLAIMERS {
        let id = member(store.as_ref(), ws, &format!("claimer-{i}")).await;
        let store = store.clone();
        claimers.push(tokio::spawn(async move {
            let mut mine = Vec::new();
            while let Some(t) = claim(store.as_ref(), ws, id).await {
                mine.push(t);
            }
            mine
        }));
    }
    let mut all = Vec::new();
    for claimer in claimers {
        all.extend(claimer.await.expect("claimer"));
    }
    let distinct: HashSet<_> = all.iter().collect();
    assert_eq!(distinct.len(), all.len(), "a thread was handed out twice");
    assert_eq!(all.len(), THREADS, "every thread was claimed once");
}

#[tokio::test]
async fn workspace_claim_next_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    concurrent_claimers_never_share_a_thread(Arc::new(store)).await;
}

#[tokio::test]
async fn workspace_claim_next_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
    concurrent_claimers_never_share_a_thread(Arc::new(store)).await;
}
