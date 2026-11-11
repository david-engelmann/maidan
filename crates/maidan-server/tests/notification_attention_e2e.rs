//! The events a coordinator wakes for reach the thread's owner and the
//! followers of the thread and its channel, never the member who caused them,
//! never a follower who can no longer read the thread, and never a member of
//! another workspace. A coordinator that follows its swarm's channels is
//! notified of questions, stalls, handoffs, failed checks, blocks, failed
//! deliveries, spawn refusals, change requests and approval gates without
//! following each agent.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{notification_router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGateId, BlockedReason, ChannelId, ChannelMemberRole, ClaimStallReason, DeclaredStatus,
    Event, EventKind, IntakeSource, Member, MemberId, MemberKind, NewChannel, NewMember, NewThread,
    NewWorkspace, ReviewDecision, TaskEngine, Thread, ThreadNoticeKind, WorkspaceId,
};

/// One workspace with a private channel the owner, the coordinator and the
/// agent belong to, and an outsider who follows the channel without being in
/// it (a follow outlives a removal from a private channel).
struct Room {
    store: Arc<dyn Store>,
    state: AppState,
    ws: WorkspaceId,
    channel: ChannelId,
    thread: Thread,
    owner: Member,
    coordinator: Member,
    agent: Member,
    outsider: Member,
    agent_watcher: Member,
}

async fn member(store: &Arc<dyn Store>, ws: WorkspaceId, handle: &str) -> Member {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
}

async fn room() -> Room {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
    );
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let owner = member(&store, ws, "owner").await;
    let coordinator = member(&store, ws, "coordinator").await;
    let agent = member(&store, ws, "agent").await;
    let outsider = member(&store, ws, "outsider").await;
    let agent_watcher = member(&store, ws, "agent-watcher").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "swarm".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap()
        .id;
    for m in [&owner, &coordinator, &agent, &agent_watcher] {
        store
            .add_channel_member(channel, m.id, ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    store.follow_channel(coordinator.id, channel).await.unwrap();
    store.follow_channel(outsider.id, channel).await.unwrap();
    store.follow_channel(agent.id, channel).await.unwrap();
    store
        .follow_member(agent_watcher.id, agent.id)
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("fix the flaky test".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();
    let (thread, _) = store
        .assign_thread_with_event(thread.id, agent.id, owner.id, None)
        .await
        .unwrap();
    Room {
        store,
        state,
        ws,
        channel,
        thread,
        owner,
        coordinator,
        agent,
        outsider,
        agent_watcher,
    }
}

impl Room {
    async fn route(&self, log_id: i64, event: &Event) {
        notification_router::route_event(&self.state, log_id, event)
            .await
            .unwrap();
    }

    /// The kinds `member` was notified of by the event at `log_id`.
    async fn kinds_at(&self, member: MemberId, log_id: i64) -> Vec<EventKind> {
        self.store
            .list_notifications(member, false, 100)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.source_log_id == log_id)
            .map(|n| n.kind)
            .collect()
    }

    /// The owner and the coordinator hear about the event at `log_id`; the
    /// actor and the outsider do not.
    async fn assert_attention(&self, log_id: i64, kind: EventKind, actor: Option<MemberId>) {
        for (who, m) in [("owner", &self.owner), ("coordinator", &self.coordinator)] {
            assert_eq!(
                self.kinds_at(m.id, log_id).await,
                vec![kind],
                "the {who} hears about {kind:?}"
            );
        }
        let notes = self
            .store
            .list_notifications(self.coordinator.id, false, 100)
            .await
            .unwrap();
        let note = notes.iter().find(|n| n.source_log_id == log_id).unwrap();
        assert_eq!(note.thread_id, Some(self.thread.id));
        assert_eq!(note.channel_id, Some(self.channel));
        assert_eq!(note.actor_id, actor);
        assert!(
            self.kinds_at(self.outsider.id, log_id).await.is_empty(),
            "a follower who cannot read the private channel is not told of {kind:?}"
        );
        if let Some(actor) = actor {
            assert!(
                self.kinds_at(actor, log_id).await.is_empty(),
                "{kind:?} is not news to the member who caused it"
            );
        }
    }

    fn notice(&self, notice: ThreadNoticeKind) -> Event {
        Event::ThreadNotice {
            occurred_at: Utc::now(),
            workspace_id: self.ws,
            channel_id: self.channel,
            thread_id: self.thread.id,
            notice,
            repo: "acme/app".into(),
            pr_number: 7,
            paths: vec![],
            base_branch: None,
            base_behind: None,
            head_sha: Some("abc123".into()),
            status: None,
            failing_jobs: vec!["unit tests".into()],
            reasons: vec!["BEHIND".into()],
            environment: None,
            deploy_url: None,
            reviewer: None,
            review_state: None,
        }
    }

    fn status(&self, status: DeclaredStatus) -> Event {
        Event::StatusDeclared {
            occurred_at: Utc::now(),
            workspace_id: self.ws,
            channel_id: self.channel,
            thread_id: self.thread.id,
            status,
            note: "which branch should this target?".into(),
            declared_by: self.agent.id,
        }
    }
}

#[tokio::test]
async fn an_agents_question_reaches_the_owner_and_channel_followers_but_not_the_agent() {
    let r = room().await;
    r.route(10, &r.status(DeclaredStatus::NeedsInput)).await;
    r.assert_attention(10, EventKind::StatusDeclared, Some(r.agent.id))
        .await;
    assert_eq!(
        r.kinds_at(r.agent_watcher.id, 10).await,
        vec![EventKind::StatusDeclared],
        "whoever follows the asking agent hears its question"
    );
}

#[tokio::test]
async fn a_declared_status_other_than_needs_input_notifies_nobody() {
    let r = room().await;
    let statuses = [
        DeclaredStatus::Working,
        DeclaredStatus::NeedsReview,
        DeclaredStatus::Blocked,
        DeclaredStatus::Done,
    ];
    for (offset, status) in statuses.into_iter().enumerate() {
        r.route(20 + offset as i64, &r.status(status)).await;
    }
    for m in [&r.owner, &r.coordinator, &r.agent_watcher] {
        assert!(
            r.store
                .list_notifications(m.id, false, 100)
                .await
                .unwrap()
                .is_empty(),
            "{} was told of a status that asks nothing",
            m.handle
        );
    }
}

#[tokio::test]
async fn a_failed_check_and_a_blocked_merge_notify_but_other_notices_do_not() {
    let r = room().await;
    r.route(30, &r.notice(ThreadNoticeKind::CiFailed)).await;
    r.assert_attention(30, EventKind::ThreadNotice, None).await;
    r.route(31, &r.notice(ThreadNoticeKind::GateBlocked)).await;
    r.assert_attention(31, EventKind::ThreadNotice, None).await;
    let quiet = [
        ThreadNoticeKind::RebaseNeeded,
        ThreadNoticeKind::StaleBase,
        ThreadNoticeKind::PrOpened,
        ThreadNoticeKind::ChecksDone,
        ThreadNoticeKind::GateClear,
        ThreadNoticeKind::MergeQueued,
        ThreadNoticeKind::MergeDequeued,
        ThreadNoticeKind::ReviewSubmitted,
        ThreadNoticeKind::Deployed,
    ];
    for (offset, notice) in quiet.into_iter().enumerate() {
        let log_id = 40 + offset as i64;
        r.route(log_id, &r.notice(notice)).await;
        assert!(
            r.kinds_at(r.coordinator.id, log_id).await.is_empty(),
            "{notice:?} is progress, not a wake-up"
        );
    }
}

#[tokio::test]
async fn a_stalled_claim_reaches_channel_followers_but_not_its_holder() {
    let r = room().await;
    let event = Event::ClaimStalled {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: r.channel,
        thread_id: r.thread.id,
        member_id: r.agent.id,
        reason: ClaimStallReason::NoProgress,
        last_progress_at: None,
        lease_expires_at: Utc::now(),
        thread: r.thread.clone(),
    };
    r.route(50, &event).await;
    r.assert_attention(50, EventKind::ClaimStalled, Some(r.agent.id))
        .await;
}

#[tokio::test]
async fn a_handoff_reaches_the_owner_and_channel_followers() {
    let r = room().await;
    let event = Event::ThreadHandoff {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: r.channel,
        thread_id: r.thread.id,
        rounds: 4,
        repo: "acme/app".into(),
        pr_number: 7,
        to_engine: TaskEngine::Session,
    };
    r.route(60, &event).await;
    r.assert_attention(60, EventKind::ThreadHandoff, None).await;
    assert_eq!(
        r.kinds_at(r.agent.id, 60).await,
        vec![EventKind::ThreadHandoff],
        "a channel follower that did not cause the handoff hears it"
    );
}

#[tokio::test]
async fn a_blocked_thread_reaches_followers_but_not_whoever_blocked_it() {
    let r = room().await;
    let event = Event::ThreadBlocked {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: r.channel,
        thread_id: r.thread.id,
        reason: BlockedReason::Human,
        set_by: r.agent.id,
        note: Some("waiting on credentials".into()),
    };
    r.route(70, &event).await;
    r.assert_attention(70, EventKind::ThreadBlocked, Some(r.agent.id))
        .await;
}

#[tokio::test]
async fn a_failed_delivery_reaches_the_owner_and_channel_followers() {
    let r = room().await;
    let event = Event::ResultDeliveryFailed {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: r.channel,
        thread_id: r.thread.id,
        surface: "github".into(),
        selector: "acme/app#7".into(),
        status: "failed".into(),
        last_error: Some("403".into()),
    };
    r.route(80, &event).await;
    r.assert_attention(80, EventKind::ResultDeliveryFailed, None)
        .await;
}

#[tokio::test]
async fn a_refused_spawn_reaches_followers_but_not_the_member_refused() {
    let r = room().await;
    let event = Event::ThreadSpawnDenied {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: r.channel,
        thread_id: r.thread.id,
        member_id: Some(r.agent.id),
        axis: "children".into(),
        limit: 8,
        observed: 8,
    };
    r.route(90, &event).await;
    r.assert_attention(90, EventKind::ThreadSpawnDenied, Some(r.agent.id))
        .await;
}

#[tokio::test]
async fn a_change_request_reaches_the_worker_owner_and_channel_followers_but_not_the_reviewer() {
    let r = room().await;
    let event = Event::ReviewSubmitted {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: r.channel,
        thread_id: r.thread.id,
        reviewer_id: r.agent_watcher.id,
        actor_id: None,
        decision: ReviewDecision::RequestChanges,
        sent_back: true,
        worker_id: Some(r.agent.id),
    };
    r.route(100, &event).await;
    r.assert_attention(100, EventKind::ReviewSubmitted, Some(r.agent_watcher.id))
        .await;
    assert_eq!(
        r.kinds_at(r.agent.id, 100).await,
        vec![EventKind::ReviewSubmitted],
        "the last worker is still told"
    );

    let approval = Event::ReviewSubmitted {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: r.channel,
        thread_id: r.thread.id,
        reviewer_id: r.agent_watcher.id,
        actor_id: None,
        decision: ReviewDecision::Approve,
        sent_back: false,
        worker_id: Some(r.agent.id),
    };
    r.route(101, &approval).await;
    assert!(
        r.kinds_at(r.coordinator.id, 101).await.is_empty(),
        "an approval asks nothing of anyone"
    );
}

#[tokio::test]
async fn an_approval_gate_on_a_thread_reaches_channel_followers_but_not_its_requester() {
    let r = room().await;
    let event = Event::ApprovalRequested {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        channel_id: Some(r.channel),
        thread_id: Some(r.thread.id),
        gate_id: ApprovalGateId::new(),
        requested_by: r.agent.id,
    };
    r.route(110, &event).await;
    r.assert_attention(110, EventKind::ApprovalRequested, Some(r.agent.id))
        .await;
}

#[tokio::test]
async fn an_intake_proposal_reaches_the_proposers_followers_in_its_workspace_only() {
    let r = room().await;
    let other_ws = r
        .store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let stranger = member(&r.store, other_ws, "stranger").await;
    r.store
        .follow_member(stranger.id, r.agent.id)
        .await
        .unwrap();
    r.store
        .follow_member(r.coordinator.id, r.agent.id)
        .await
        .unwrap();
    let event = Event::IntakeProposed {
        occurred_at: Utc::now(),
        workspace_id: r.ws,
        intake_id: uuid::Uuid::now_v7(),
        title: "add a retry to the uploader".into(),
        source: IntakeSource::Agent,
        proposed_by: r.agent.id,
    };
    r.route(120, &event).await;
    for m in [&r.coordinator, &r.agent_watcher] {
        assert_eq!(
            r.kinds_at(m.id, 120).await,
            vec![EventKind::IntakeProposed],
            "{} follows the proposer",
            m.handle
        );
    }
    let note = r
        .store
        .list_notifications(r.coordinator.id, false, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|n| n.source_log_id == 120)
        .unwrap();
    assert_eq!((note.channel_id, note.thread_id), (None, None));
    assert_eq!(note.actor_id, Some(r.agent.id));
    for m in [&stranger, &r.agent, &r.owner] {
        assert!(
            r.kinds_at(m.id, 120).await.is_empty(),
            "{} is told of a proposal it does not follow, or its own",
            m.handle
        );
    }
}

#[tokio::test]
async fn attention_events_stay_in_their_workspace() {
    let r = room().await;
    let other_ws = r
        .store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let stranger = member(&r.store, other_ws, "stranger").await;
    // Follows that cross the tenant line: the router must not honour them.
    r.store
        .follow_channel(stranger.id, r.channel)
        .await
        .unwrap();
    r.store
        .follow_thread(stranger.id, r.thread.id)
        .await
        .unwrap();
    r.store
        .follow_member(stranger.id, r.agent.id)
        .await
        .unwrap();
    r.route(130, &r.status(DeclaredStatus::NeedsInput)).await;
    r.route(131, &r.notice(ThreadNoticeKind::CiFailed)).await;
    assert_eq!(
        r.kinds_at(r.coordinator.id, 130).await,
        vec![EventKind::StatusDeclared]
    );
    assert!(
        r.store
            .list_notifications(stranger.id, false, 10)
            .await
            .unwrap()
            .is_empty(),
        "a member of another workspace is told nothing"
    );
}

#[tokio::test]
async fn a_routed_event_is_written_once_per_member_however_many_ways_they_follow_it() {
    let r = room().await;
    r.store
        .follow_thread(r.coordinator.id, r.thread.id)
        .await
        .unwrap();
    r.store
        .follow_member(r.coordinator.id, r.agent.id)
        .await
        .unwrap();
    let event = r.status(DeclaredStatus::NeedsInput);
    r.route(140, &event).await;
    r.route(140, &event).await;
    assert_eq!(
        r.kinds_at(r.coordinator.id, 140).await,
        vec![EventKind::StatusDeclared]
    );
    let kinds: HashSet<_> = r
        .store
        .list_notifications(r.owner.id, false, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.kind)
        .collect();
    assert_eq!(kinds, HashSet::from([EventKind::StatusDeclared]));
}
