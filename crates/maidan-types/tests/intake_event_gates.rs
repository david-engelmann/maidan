//! `intake_accepted` names a thread, so it must pass the same channel gates
//! as the thread's own events: a subscriber denied the thread's private
//! channel does not receive it.

use std::collections::HashSet;

use chrono::Utc;
use maidan_types::{ChannelId, Event, EventFilter, MemberId, ThreadId, WorkspaceId};
use uuid::Uuid;

fn accepted(workspace_id: WorkspaceId, channel_id: ChannelId) -> Event {
    Event::IntakeAccepted {
        occurred_at: Utc::now(),
        workspace_id,
        intake_id: Uuid::now_v7(),
        thread_id: ThreadId(Uuid::now_v7()),
        channel_id,
        triaged_by: MemberId(Uuid::now_v7()),
    }
}

#[test]
fn a_denied_private_channel_hides_the_acceptance() {
    let ws = WorkspaceId(Uuid::now_v7());
    let private = ChannelId(Uuid::now_v7());
    let mut filter = EventFilter::workspace(ws);
    filter.private_channel_deny = HashSet::from([private]);
    assert!(!filter.matches(&accepted(ws, private)));
}

#[test]
fn a_readable_channel_delivers_the_acceptance() {
    let ws = WorkspaceId(Uuid::now_v7());
    let open = ChannelId(Uuid::now_v7());
    let mut filter = EventFilter::workspace(ws);
    filter.private_channel_deny = HashSet::from([ChannelId(Uuid::now_v7())]);
    let event = accepted(ws, open);
    assert!(filter.matches(&event));
    assert_eq!(event.channel_id(), Some(open));
}
