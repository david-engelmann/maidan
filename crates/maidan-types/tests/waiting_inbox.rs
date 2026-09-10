//! The pure waiting-on-you-inbox aggregate (Cluster 368, Wave 2 #16): excludes
//! terminal/tombstoned assigned threads, merges gates + mentions, sorts
//! oldest-waiting first, and flags SLA breaches.

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
        state,
        assignee_id: None,
        assignment_expires_at: None,
        claim_lease_id: None,
        work_started_at: None,
        owner_id: None,
        created_at: now - Duration::seconds(age_secs),
        updated_at: now,
        tombstoned_at: if tombstoned { Some(now) } else { None },
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

    let inbox = assemble_waiting_inbox(&assigned, &gates, &mentions, now, sla);

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
    let inbox = assemble_waiting_inbox(&[], &[], &[], Utc::now(), 3600);
    assert_eq!(inbox.total, 0);
    assert_eq!(inbox.overdue, 0);
    assert!(inbox.items.is_empty());
    assert_eq!(inbox.sla_secs, 3600);
}
