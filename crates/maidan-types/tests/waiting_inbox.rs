//! The pure waiting-on-you-inbox aggregate: excludes terminal/tombstoned
//! assigned threads, merges gates + mentions, sorts oldest-waiting first, and
//! flags SLA breaches.

use chrono::{Duration, Utc};
use maidan_types::*;
use uuid::Uuid;

fn thread(state: ThreadState, tombstoned: bool, age_secs: i64) -> Thread {
    let now = Utc::now();
    Thread {
        id: ThreadId(Uuid::new_v4()),
        channel_id: ChannelId(Uuid::new_v4()),
        parent_thread_id: None,
        title: Some("task".into()),
        description: None,
        state,
        assignee_id: None,
        assignment_expires_at: None,
        claim_lease_id: None,
        work_started_at: None,
        owner_id: None,
        created_at: now - Duration::seconds(age_secs),
        updated_at: now,
        tombstoned_at: if tombstoned { Some(now) } else { None },
        status: None,
        block: None,
        closed_without_review: false,
    }
}

fn gate(age_secs: i64) -> ApprovalGate {
    ApprovalGate {
        id: ApprovalGateId(Uuid::new_v4()),
        workspace_id: WorkspaceId(Uuid::new_v4()),
        thread_id: Some(ThreadId(Uuid::new_v4())),
        requested_by: MemberId(Uuid::new_v4()),
        prompt: "approve the deploy".into(),
        schema: None,
        state: ApprovalGateState::Pending,
        content: None,
        resolved_by: None,
        requested_actor_id: None,
        resolved_actor_id: None,
        created_at: Utc::now() - Duration::seconds(age_secs),
        resolved_at: None,
    }
}

fn mention(age_secs: i64) -> Mention {
    Mention {
        message_id: MessageId(Uuid::new_v4()),
        member_id: MemberId(Uuid::new_v4()),
        created_at: Utc::now() - Duration::seconds(age_secs),
    }
}

#[test]
fn assembles_excludes_terminal_sorts_and_flags_overdue() {
    let now = Utc::now();
    let sla = 3600; // 1h
    let assigned = vec![
        thread(ThreadState::Open, false, 100),      // recent, in play
        thread(ThreadState::Closed, false, 500),    // terminal → excluded
        thread(ThreadState::Archived, false, 500),  // terminal → excluded
        thread(ThreadState::Open, true, 500),       // tombstoned → excluded
        thread(ThreadState::InReview, false, 7200), // 2h old → overdue
    ];
    let gates = vec![gate(200)];
    let mentions = vec![mention(50)];

    let inbox = assemble_waiting_inbox(&assigned, &[], &[], &gates, &mentions, &[], now, sla);

    // 2 live threads + 1 gate + 1 mention = 4 items (the 3 excluded threads drop).
    assert_eq!(inbox.total, 4);
    assert_eq!(inbox.items.len(), 4);
    // Exactly the 2h-old thread is overdue (> 1h SLA).
    assert_eq!(inbox.overdue, 1);
    assert!(inbox
        .items
        .iter()
        .any(|i| i.overdue && i.kind == WaitingKind::AssignedThread));
    // Oldest-waiting first: the ~7200s-old thread leads (allow clock slack).
    assert!(inbox.items[0].age_secs >= 7000);
    assert!(inbox.items[0].age_secs >= inbox.items[1].age_secs);
    // Kinds are represented.
    assert!(inbox.items.iter().any(|i| i.kind == WaitingKind::OpenGate));
    assert!(inbox.items.iter().any(|i| i.kind == WaitingKind::Mention));
}

#[test]
fn empty_sources_yield_an_empty_inbox() {
    let inbox = assemble_waiting_inbox(&[], &[], &[], &[], &[], &[], Utc::now(), 3600);
    assert_eq!(inbox.total, 0);
    assert_eq!(inbox.overdue, 0);
    assert!(inbox.items.is_empty());
    assert_eq!(inbox.sla_secs, 3600);
}

#[test]
fn a_requested_review_waits_since_the_thread_last_changed() {
    let now = Utc::now();
    let mut review = thread(ThreadState::InReview, false, 9000);
    review.title = Some("Fix the flaky login test".into());
    review.updated_at = now - Duration::seconds(600);
    let closed = thread(ThreadState::Closed, false, 9000);

    let inbox = assemble_waiting_inbox(
        &[],
        &[review.clone(), closed],
        &[],
        &[],
        &[],
        &[],
        now,
        3600,
    );

    assert_eq!(inbox.total, 1, "a closed thread's review waits on nobody");
    let item = &inbox.items[0];
    assert_eq!(item.kind, WaitingKind::ReviewRequest);
    assert_eq!(item.thread_id, Some(review.id));
    assert_eq!(item.summary, "Fix the flaky login test");
    assert!(
        (590..=610).contains(&item.age_secs),
        "aged from when it went to review, not from creation: {}",
        item.age_secs
    );
    assert!(!item.overdue);
    assert_eq!(
        serde_json::to_value(item.kind).unwrap(),
        serde_json::json!("review_request")
    );
}

#[test]
fn a_human_blocked_thread_waits_as_blocked_with_its_note() {
    let now = Utc::now();
    let tid = ThreadId::new();
    let owner = MemberId::new();
    let block = ThreadBlock {
        thread_id: tid,
        reason: BlockedReason::Human,
        set_by: MemberId::new(),
        set_at: now - Duration::seconds(300),
        note: Some("waiting on security review".into()),
    };
    let blocked = vec![(tid, Some("Deploy to prod".into()), Some(owner), block)];

    let inbox = assemble_waiting_inbox(&[], &[], &[], &[], &[], &blocked, now, 3600);

    assert_eq!(inbox.total, 1);
    let item = &inbox.items[0];
    assert_eq!(item.kind, WaitingKind::Blocked);
    assert_eq!(item.thread_id, Some(tid));
    assert!(item.summary.contains("Deploy to prod"));
    assert!(item.summary.contains("human"));
    assert!(item.summary.contains("waiting on security review"));
    assert_eq!(
        serde_json::to_value(item.kind).unwrap(),
        serde_json::json!("blocked")
    );
}

#[test]
fn a_review_with_no_reviewer_waits_as_an_unassigned_review() {
    let now = Utc::now();
    let mut review = thread(ThreadState::InReview, false, 9000);
    review.title = Some("Rate-limit /api/upload".into());
    review.updated_at = now - Duration::seconds(120);
    let closed = thread(ThreadState::Closed, false, 9000);

    let inbox = assemble_waiting_inbox(&[], &[], &[review.clone(), closed], &[], &[], &[], now, 60);

    assert_eq!(inbox.total, 1, "a closed thread waits on nobody");
    let item = &inbox.items[0];
    assert_eq!(item.kind, WaitingKind::UnassignedReview);
    assert_eq!(item.thread_id, Some(review.id));
    assert_eq!(item.summary, "Rate-limit /api/upload");
    assert!(item.overdue, "aged from when review began, against the SLA");
    assert_eq!(
        serde_json::to_value(item.kind).unwrap(),
        serde_json::json!("unassigned_review")
    );
}
