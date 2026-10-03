//! The workspace views beside the workspace claim: whether one thread is one
//! `claim_next` would hand a member (what `wait_for_ready` wakes on), and the
//! queue depth and occupancy of a whole workspace. Both apply the claim's read
//! rule, so a private channel's threads are its members', a DM's are its
//! participants', and nothing crosses a workspace. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BlockedReason, ChannelId, ChannelMemberRole, ChannelOccupancy, MemberId, MemberKind,
    NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace, QueueDepth, ThreadId,
    WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

const LEASE_SECS: i64 = 600;

async fn sqlite() -> SqliteStore {
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
        })
        .await
        .expect("thread")
        .id
}

async fn claimable(store: &dyn Store, thread_id: ThreadId, member_id: MemberId) -> bool {
    store
        .thread_claimable_by(thread_id, member_id)
        .await
        .expect("claimable")
}

async fn close(store: &dyn Store, thread_id: ThreadId, actor: MemberId) {
    for action in [
        maidan_fsm::ThreadAction::StartReview,
        maidan_fsm::ThreadAction::Close,
    ] {
        store
            .transition_thread(thread_id, actor, action)
            .await
            .expect("transition");
    }
}

/// A thread is claimable by a member exactly when `claim_next` would hand it
/// to them: each of the claim's skips, and its read rule, says no.
async fn claimable_by_follows_every_rule_of_the_claim(store: &dyn Store) {
    let ws = workspace(store, "claimable").await;
    let agent = member(store, ws, "agent").await;
    let other = member(store, ws, "other").await;
    let insider = member(store, ws, "insider").await;
    let open = channel(store, ws, "open", false).await;
    let secret = channel(store, ws, "secret", true).await;
    store
        .add_channel_member(secret, insider, ChannelMemberRole::Member)
        .await
        .expect("join");

    let free = thread(store, open).await;
    assert!(claimable(store, free, agent).await, "a free, ready thread");

    let dependency = thread(store, open).await;
    let waits = thread(store, open).await;
    store
        .add_thread_dependency(waits, dependency)
        .await
        .expect("dependency");
    assert!(!claimable(store, waits, agent).await, "an unfinished dependency");
    close(store, dependency, agent).await;
    assert!(claimable(store, waits, agent).await, "the dependency finished");
    assert!(!claimable(store, dependency, agent).await, "a closed thread");

    let needs_skill = thread(store, open).await;
    store
        .add_thread_required_skill(needs_skill, "rust")
        .await
        .expect("skill");
    assert!(!claimable(store, needs_skill, agent).await, "a missing skill");
    store.add_member_skill(agent, "rust").await.expect("skill");
    assert!(claimable(store, needs_skill, agent).await, "the skill is held");

    let gated = thread(store, open).await;
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws,
            thread_id: Some(gated),
            requested_by: other,
            prompt: "ship it?".into(),
            schema: None,
        })
        .await
        .expect("gate");
    assert!(!claimable(store, gated, agent).await, "a pending approval gate");

    let blocked = thread(store, open).await;
    store
        .set_thread_block(blocked, BlockedReason::Human, other)
        .await
        .expect("block");
    assert!(!claimable(store, blocked, agent).await, "an explicit block");

    let parked = thread(store, open).await;
    store
        .mark_thread_unclaimable(parked, "waiting on a person", other)
        .await
        .expect("park");
    assert!(!claimable(store, parked, agent).await, "an unclaimable park");

    let held = thread(store, open).await;
    store.assign_thread(held, other).await.expect("assign");
    assert!(!claimable(store, held, agent).await, "held by someone else");

    let lapsed_channel = channel(store, ws, "lapsed", false).await;
    let lapsed = thread(store, lapsed_channel).await;
    store
        .claim_next_thread_with_event(lapsed_channel, other, Some(-1))
        .await
        .expect("claim")
        .0
        .expect("claimed");
    assert!(
        claimable(store, lapsed, agent).await,
        "a lapsed lease is the claim's to take over"
    );

    let private = thread(store, secret).await;
    assert!(!claimable(store, private, agent).await, "a private channel");
    assert!(claimable(store, private, insider).await, "its member may");

    let dm = store
        .open_dm_conversation(ws, insider, other)
        .await
        .expect("dm");
    assert!(!claimable(store, dm.thread_id, agent).await, "someone's DM");
    assert!(claimable(store, dm.thread_id, insider).await, "a participant");

    store
        .freeze_member(agent, other, Some("kill-switch"))
        .await
        .expect("freeze");
    assert!(!claimable(store, free, agent).await, "a frozen claimer");
    store
        .unfreeze_member(agent, other)
        .await
        .expect("unfreeze")
        .expect("was frozen");
    assert!(claimable(store, free, agent).await);

    let elsewhere = workspace(store, "claimable-elsewhere").await;
    let foreign = thread(store, channel(store, elsewhere, "open", false).await).await;
    assert!(
        !claimable(store, foreign, agent).await,
        "another workspace's free thread"
    );

    assert!(store
        .claim_next_thread_with_event(open, agent, Some(LEASE_SECS))
        .await
        .expect("claim")
        .0
        .is_some());
}

fn depth(open: i64, ready: i64, assigned: i64, blocked: i64, unclaimable: i64) -> QueueDepth {
    QueueDepth {
        open,
        ready,
        assigned,
        blocked,
        unclaimable,
    }
}

fn add(a: QueueDepth, b: QueueDepth) -> QueueDepth {
    depth(
        a.open + b.open,
        a.ready + b.ready,
        a.assigned + b.assigned,
        a.blocked + b.blocked,
        a.unclaimable + b.unclaimable,
    )
}

/// A workspace's counts are its channels' counts summed, for the same reader:
/// a private channel counts for its members, a DM for its participants, every
/// thread when there is no reader, and nothing of another workspace. The
/// shared `__dm__` channel's own counts hold only the reader's DMs.
async fn workspace_counts_sum_the_channels_the_reader_may_read(store: &dyn Store) {
    let ws = workspace(store, "counts").await;
    let agent = member(store, ws, "agent").await;
    let insider = member(store, ws, "insider").await;
    let alice = member(store, ws, "alice").await;
    let bob = member(store, ws, "bob").await;
    let open = channel(store, ws, "open", false).await;
    let secret = channel(store, ws, "secret", true).await;
    store
        .add_channel_member(secret, insider, ChannelMemberRole::Member)
        .await
        .expect("join");

    let dependency = thread(store, open).await;
    let waits = thread(store, open).await;
    store
        .add_thread_dependency(waits, dependency)
        .await
        .expect("dependency");
    let parked = thread(store, open).await;
    store
        .mark_thread_unclaimable(parked, "waiting on a person", alice)
        .await
        .expect("park");
    let claimed = thread(store, open).await;
    store.assign_thread(claimed, bob).await.expect("assign");
    let working_channel = channel(store, ws, "working", false).await;
    let working = thread(store, working_channel).await;
    let held = store
        .claim_next_thread_with_event(working_channel, bob, Some(LEASE_SECS))
        .await
        .expect("claim")
        .0
        .expect("claimed");
    assert_eq!(held.id, working);
    store
        .acknowledge_claim(working, bob, held.claim_lease_id.expect("token"))
        .await
        .expect("acknowledge");
    for _ in 0..2 {
        thread(store, secret).await;
    }
    let dm = store
        .open_dm_conversation(ws, alice, bob)
        .await
        .expect("dm");
    let dm_channel = store
        .get_thread(dm.thread_id)
        .await
        .expect("dm thread")
        .channel_id;

    let other = workspace(store, "counts-elsewhere").await;
    let foreign = channel(store, other, "open", false).await;
    for _ in 0..3 {
        thread(store, foreign).await;
    }

    // open: dependency ready, waits blocked, parked unclaimable, claimed held.
    // working: one held. secret: two ready. __dm__: one ready.
    let public = depth(5, 1, 2, 1, 1);
    let cases = [
        (Some(agent), public.clone()),
        (Some(insider), add(public.clone(), depth(2, 2, 0, 0, 0))),
        (Some(alice), add(public.clone(), depth(1, 1, 0, 0, 0))),
        (None, add(public.clone(), depth(3, 3, 0, 0, 0))),
    ];
    for (reader, expected) in cases {
        let got = store
            .workspace_queue_depth(ws, reader)
            .await
            .expect("workspace depth");
        assert_eq!(got, expected, "workspace depth for {reader:?}");
        let mut summed = depth(0, 0, 0, 0, 0);
        for ch in [open, working_channel, secret, dm_channel] {
            let per_channel = store
                .channel_queue_depth(ch, reader)
                .await
                .expect("channel depth");
            summed = add(summed, per_channel);
        }
        assert_eq!(got, summed, "the sum of its channels for {reader:?}");
    }

    assert_eq!(
        store
            .channel_queue_depth(dm_channel, Some(agent))
            .await
            .expect("dm depth"),
        depth(0, 0, 0, 0, 0),
        "the __dm__ channel holds none of the agent's DMs"
    );
    assert_eq!(
        store
            .channel_queue_depth(dm_channel, Some(bob))
            .await
            .expect("dm depth")
            .open,
        1,
        "and bob's own"
    );

    assert_eq!(
        store
            .workspace_queue_depth(other, Some(agent))
            .await
            .expect("foreign depth"),
        depth(0, 0, 0, 0, 0),
        "a reader from another workspace counts nothing there"
    );
    assert_eq!(
        store
            .workspace_queue_depth(other, None)
            .await
            .expect("foreign depth")
            .open,
        3,
        "and the other workspace's threads are its own"
    );

    let occupancy = store
        .workspace_occupancy(ws, Some(agent))
        .await
        .expect("occupancy");
    assert_eq!(
        occupancy,
        ChannelOccupancy {
            open: 5,
            queued: 2,
            claimed: 1,
            working: 1,
            blocked: 1,
        },
        "queued counts the parked thread, as the channel occupancy does"
    );
    let insider_occupancy = store
        .workspace_occupancy(ws, Some(insider))
        .await
        .expect("occupancy");
    assert_eq!(insider_occupancy.open, 7);
    assert_eq!(insider_occupancy.queued, 4);
    assert_eq!(
        store
            .workspace_occupancy(other, Some(agent))
            .await
            .expect("foreign occupancy")
            .open,
        0
    );
    let mut summed_open = 0;
    for ch in [open, working_channel, secret, dm_channel] {
        summed_open += store
            .channel_occupancy(ch, Some(alice))
            .await
            .expect("channel occupancy")
            .open;
    }
    assert_eq!(
        store
            .workspace_occupancy(ws, Some(alice))
            .await
            .expect("occupancy")
            .open,
        summed_open
    );
}

async fn run_suite(store: &dyn Store) {
    claimable_by_follows_every_rule_of_the_claim(store).await;
    workspace_counts_sum_the_channels_the_reader_may_read(store).await;
}

#[tokio::test]
async fn workspace_queue_counts_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn workspace_queue_counts_postgres() {
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
        .max_connections(4)
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
