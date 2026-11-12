//! After a thread is sent back from review `max_rounds` times (3 when nobody
//! set a cap), the next send-back stays off the queue until a person accepts
//! another round. Declining parks it. A normal approval request still unblocks
//! when it is declined. Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGateKind, ApprovalGateState, ChannelId, Event, MemberId, MemberKind, NewApprovalGate,
    NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision, ThreadId, WorkspaceId,
    DEFAULT_MAX_REVIEW_ROUNDS,
};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
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

fn brief(n: u64) -> String {
    format!("round cap task {n} {}", uuid::Uuid::now_v7())
}

/// Claim, hand to review, release, and send back. The thread comes back `open`.
async fn send_back(
    store: &dyn Store,
    channel: ChannelId,
    thread: ThreadId,
    worker: MemberId,
    reviewer: MemberId,
    note: &str,
) -> maidan_types::ReviewSubmission {
    let claimed = store
        .claim_next_thread(channel, worker, Some(60), None)
        .await
        .expect("claim")
        .expect("the thread is claimable");
    assert_eq!(claimed.id, thread);
    store
        .set_thread_result_claiming(thread, worker, &serde_json::json!({"status": "done"}))
        .await
        .expect("result");
    store
        .transition_thread(thread, worker, ThreadAction::StartReview)
        .await
        .expect("start review");
    store
        .release_claim(thread, worker, claimed.claim_lease_id.expect("lease"))
        .await
        .expect("release");
    let submission = store
        .submit_review(
            thread,
            reviewer,
            ReviewDecision::RequestChanges,
            Some(note),
            None,
        )
        .await
        .expect("request changes");
    assert_eq!(
        store.get_thread(thread).await.expect("thread").state,
        maidan_types::ThreadState::Open
    );
    submission
}

async fn fresh(store: &dyn Store, ws: WorkspaceId, n: u64) -> (ThreadId, ChannelId) {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("cap-{n}-{}", uuid::Uuid::now_v7()),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(brief(n)),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    (thread, channel)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "rounds".into(),
        })
        .await
        .expect("ws")
        .id;
    let worker = member(store, ws, "worker").await;
    let reviewer = member(store, ws, "reviewer").await;
    let human = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "person".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("human")
        .id;

    let status = store
        .review_status({
            let (thread, _) = fresh(store, ws, 0).await;
            thread
        })
        .await
        .unwrap();
    assert_eq!(status.max_rounds, DEFAULT_MAX_REVIEW_ROUNDS);
    assert_eq!(status.send_backs, 0);

    // The default cap, with no requirement row at all.
    let (thread, channel) = fresh(store, ws, 1).await;
    for n in 1..=2 {
        let submission = send_back(store, channel, thread, worker, reviewer, "again").await;
        assert!(
            submission.approval_gate.is_none(),
            "send-back {n} opens no gate"
        );
        assert!(submission.escalated.is_none());
        assert!(store
            .claim_next_thread(channel, worker, Some(60), None)
            .await
            .unwrap()
            .is_some());
        store
            .release_claim(
                thread,
                worker,
                store
                    .get_thread(thread)
                    .await
                    .unwrap()
                    .claim_lease_id
                    .expect("still leased from the probe claim"),
            )
            .await
            .unwrap();
    }
    let third = send_back(store, channel, thread, worker, reviewer, "the empty case").await;
    let gate = third
        .approval_gate
        .clone()
        .expect("the third send-back opens a gate");
    assert_eq!(gate.kind, ApprovalGateKind::ReviewRoundCap);
    assert_eq!(gate.state, ApprovalGateState::Pending);
    assert_eq!(gate.requested_by, reviewer);
    assert_eq!(
        gate.prompt,
        "Sent back 3 times in review (latest note: the empty case). Accept to return this task to the queue for another round, or decline to keep it off the queue."
    );
    assert!(third.events().any(|event| {
        matches!(
            serde_json::from_value::<Event>(event.payload.clone()).unwrap(),
            Event::ApprovalRequested { gate_id, .. } if gate_id == gate.id
        )
    }));
    assert!(store
        .claim_next_thread(channel, worker, None, None)
        .await
        .unwrap()
        .is_none());
    let depth = store.channel_queue_depth(channel, None).await.unwrap();
    assert_eq!(depth.ready, 0);
    assert_eq!(depth.blocked, 1);
    let standing = store.review_status(thread).await.unwrap();
    assert_eq!(standing.send_backs, 3);
    assert_eq!(standing.max_rounds, 3);

    store
        .resolve_approval_gate(gate.id, human, ApprovalGateState::Accepted, None)
        .await
        .unwrap()
        .expect("pending");
    assert!(store
        .claim_next_thread(channel, worker, Some(60), None)
        .await
        .unwrap()
        .is_some());
    store
        .release_claim(
            thread,
            worker,
            store
                .get_thread(thread)
                .await
                .unwrap()
                .claim_lease_id
                .unwrap(),
        )
        .await
        .unwrap();
    for _ in 0..2 {
        let submission = send_back(store, channel, thread, worker, reviewer, "once more").await;
        assert!(submission.approval_gate.is_none());
        store
            .release_claim(
                thread,
                worker,
                store
                    .claim_next_thread(channel, worker, Some(60), None)
                    .await
                    .unwrap()
                    .expect("requeued")
                    .claim_lease_id
                    .unwrap(),
            )
            .await
            .unwrap();
    }
    let sixth = send_back(store, channel, thread, worker, reviewer, "cap again").await;
    assert_eq!(
        sixth.approval_gate.map(|g| g.kind),
        Some(ApprovalGateKind::ReviewRoundCap)
    );
    assert!(store
        .claim_next_thread(channel, worker, None, None)
        .await
        .unwrap()
        .is_none());

    // Declining parks the thread. A normal request gate still unblocks.
    let (parked, park_channel) = fresh(store, ws, 2).await;
    store.set_review_rounds(parked, 0, Some(1)).await.unwrap();
    let opened = send_back(store, park_channel, parked, worker, reviewer, "stop").await;
    let gate = opened
        .approval_gate
        .expect("a cap of 1 escalates immediately");
    store
        .resolve_approval_gate(gate.id, human, ApprovalGateState::Declined, None)
        .await
        .unwrap()
        .expect("declined");
    assert_eq!(
        store
            .get_approval_gate(gate.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        ApprovalGateState::Declined
    );
    assert!(store
        .claim_next_thread(park_channel, worker, None, None)
        .await
        .unwrap()
        .is_none());

    let (plain, plain_channel) = fresh(store, ws, 3).await;
    let request = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws,
            thread_id: Some(plain),
            requested_by: worker,
            prompt: "Ship it?".into(),
            schema: None,
            risk: Default::default(),
        })
        .await
        .unwrap();
    assert_eq!(request.kind, ApprovalGateKind::Request);
    store
        .resolve_approval_gate(request.id, human, ApprovalGateState::Declined, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .claim_next_thread(plain_channel, worker, None, None)
            .await
            .unwrap()
            .map(|t| t.id),
        Some(plain)
    );

    // A custom cap, an uncapped thread, and a write that omits the cap.
    let (one, one_channel) = fresh(store, ws, 4).await;
    let wrote = store.set_review_rounds(one, 1, Some(1)).await.unwrap();
    assert_eq!(wrote.max_rounds, 1);
    assert!(
        send_back(store, one_channel, one, worker, reviewer, "first")
            .await
            .approval_gate
            .is_some()
    );

    let (open_ended, open_channel) = fresh(store, ws, 5).await;
    store
        .set_review_rounds(open_ended, 1, Some(0))
        .await
        .unwrap();
    for _ in 0..4 {
        let submission = send_back(
            store,
            open_channel,
            open_ended,
            worker,
            reviewer,
            "keep going",
        )
        .await;
        assert!(submission.approval_gate.is_none());
        let claimed = store
            .claim_next_thread(open_channel, worker, Some(60), None)
            .await
            .unwrap()
            .expect("uncapped");
        store
            .release_claim(open_ended, worker, claimed.claim_lease_id.unwrap())
            .await
            .unwrap();
    }

    let (kept, _) = fresh(store, ws, 6).await;
    store.set_review_rounds(kept, 1, Some(5)).await.unwrap();
    let preserved = store.set_review_requirement(kept, 2).await.unwrap();
    assert_eq!(preserved.required_count, 2);
    assert_eq!(preserved.max_rounds, 5);
    let (born, _) = fresh(store, ws, 7).await;
    let created = store.set_review_requirement(born, 1).await.unwrap();
    assert_eq!(created.max_rounds, DEFAULT_MAX_REVIEW_ROUNDS);
}

async fn blank_note_prompt(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "note".into(),
        })
        .await
        .unwrap()
        .id;
    let worker = member(store, ws, "worker").await;
    let reviewer = member(store, ws, "reviewer").await;
    let (thread, channel) = fresh(store, ws, 8).await;
    store.set_review_rounds(thread, 0, Some(1)).await.unwrap();
    // A change request cannot carry an empty note, so the no-note sentence is
    // the one written when the stored note trims to nothing. The route refuses
    // an empty note; the store records whatever it is given, including none.
    let submission = {
        let claimed = store
            .claim_next_thread(channel, worker, Some(60), None)
            .await
            .unwrap()
            .unwrap();
        store
            .set_thread_result_claiming(thread, worker, &serde_json::json!({"status": "done"}))
            .await
            .unwrap();
        store
            .transition_thread(thread, worker, ThreadAction::StartReview)
            .await
            .unwrap();
        store
            .release_claim(thread, worker, claimed.claim_lease_id.unwrap())
            .await
            .unwrap();
        store
            .submit_review(thread, reviewer, ReviewDecision::RequestChanges, None, None)
            .await
            .unwrap()
    };
    assert_eq!(
        submission.approval_gate.unwrap().prompt,
        "Sent back 1 times in review. Accept to return this task to the queue for another round, or decline to keep it off the queue."
    );
}

/// A worker still holding its lease when the cap opens loses that lease, and
/// neither a named claim nor an assignment can take the thread. A direct
/// `request_changes` transition, not only `submit_review`, opens the gate.
async fn held_lease_and_direct_send_back(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap()
        .id;
    let worker = member(store, ws, "worker").await;
    let reviewer = member(store, ws, "reviewer").await;
    let human = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "person".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    let (thread, channel) = fresh(store, ws, 9).await;
    store.set_review_rounds(thread, 0, Some(1)).await.unwrap();
    let claimed = store
        .claim_next_thread(channel, worker, Some(60), None)
        .await
        .unwrap()
        .unwrap();
    store
        .set_thread_result_claiming(thread, worker, &serde_json::json!({"status": "done"}))
        .await
        .unwrap();
    store
        .transition_thread(thread, worker, ThreadAction::StartReview)
        .await
        .unwrap();
    let submission = store
        .submit_review(
            thread,
            reviewer,
            ReviewDecision::RequestChanges,
            Some("stop"),
            None,
        )
        .await
        .unwrap();
    assert!(submission.approval_gate.is_some());
    let held = store.get_thread(thread).await.unwrap();
    assert_eq!(held.assignee_id, None);
    assert_eq!(held.claim_lease_id, None);
    let refused = store.claim_thread(thread, worker).await.unwrap_err();
    assert!(
        refused.to_string().contains("review-round cap"),
        "{refused}"
    );
    let refused = store.assign_thread(thread, worker).await.unwrap_err();
    assert!(
        refused.to_string().contains("review-round cap"),
        "{refused}"
    );
    // The lease that was live at the send-back no longer renews.
    let renew = store
        .renew_claim(thread, worker, claimed.claim_lease_id.expect("lease"), 60)
        .await;
    assert!(renew.is_err(), "{renew:?}");

    let (direct, direct_channel) = fresh(store, ws, 10).await;
    store.set_review_rounds(direct, 0, Some(1)).await.unwrap();
    let claimed = store
        .claim_next_thread(direct_channel, worker, Some(60), None)
        .await
        .unwrap()
        .unwrap();
    store
        .set_thread_result_claiming(direct, worker, &serde_json::json!({"status": "done"}))
        .await
        .unwrap();
    store
        .transition_thread(direct, worker, ThreadAction::StartReview)
        .await
        .unwrap();
    store
        .release_claim(direct, worker, claimed.claim_lease_id.unwrap())
        .await
        .unwrap();
    store
        .transition_thread(direct, reviewer, ThreadAction::RequestChanges)
        .await
        .unwrap();
    let gates = store
        .list_pending_approval_gates(ws, None, 20)
        .await
        .unwrap();
    let gate = gates
        .iter()
        .find(|g| g.thread_id == Some(direct) && g.kind == ApprovalGateKind::ReviewRoundCap)
        .expect("direct request_changes opens the cap");
    store
        .resolve_approval_gate(gate.id, human, ApprovalGateState::Declined, None)
        .await
        .unwrap()
        .unwrap();
    let refused = store.claim_thread(direct, worker).await.unwrap_err();
    assert!(
        refused.to_string().contains("review-round cap"),
        "{refused}"
    );
}

#[tokio::test]
async fn review_round_cap_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    blank_note_prompt(&store).await;
    held_lease_and_direct_send_back(&store).await;
}

#[tokio::test]
async fn review_round_cap_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
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
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
    blank_note_prompt(&store).await;
    held_lease_and_direct_send_back(&store).await;
}
