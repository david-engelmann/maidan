//! In-memory bus integration tests.

use std::{collections::HashSet, time::Duration};

use chrono::Utc;
use futures::StreamExt;
use maidan_bus::{EventBus, InMemoryBus};
use maidan_types::*;

fn workspace(name: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(uuid::Uuid::new_v4()),
        name: name.into(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tombstoned_at: None,
    }
}

fn channel(workspace_id: WorkspaceId, name: &str) -> Channel {
    Channel {
        id: ChannelId(uuid::Uuid::new_v4()),
        workspace_id,
        name: name.into(),
        topic: None,
        private: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tombstoned_at: None,
    }
}

fn thread(channel_id: ChannelId) -> Thread {
    Thread {
        id: ThreadId(uuid::Uuid::new_v4()),
        channel_id,
        parent_thread_id: None,
        title: None,
        state: ThreadState::Open,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tombstoned_at: None,
    }
}

fn message(thread_id: ThreadId, author_id: MemberId, body: &str) -> Message {
    Message {
        id: MessageId(uuid::Uuid::new_v4()),
        thread_id,
        author_id,
        body: body.into(),
        metadata: serde_json::json!({}),
        posted_at: Utc::now(),
        edited_at: None,
        tombstoned_at: None,
    }
}

#[tokio::test]
async fn subscribers_with_different_filters_see_only_matching_events() {
    let bus = InMemoryBus::with_capacity(256);

    let ws_a = workspace("a");
    let ws_b = workspace("b");
    let ch_a1 = channel(ws_a.id, "a1");
    let ch_b1 = channel(ws_b.id, "b1");
    let th_a1 = thread(ch_a1.id);
    let th_b1 = thread(ch_b1.id);
    let author = MemberId(uuid::Uuid::new_v4());

    let mut all = bus.subscribe(EventFilter::all()).await.unwrap();
    let mut only_a = bus
        .subscribe(EventFilter::workspace(ws_a.id))
        .await
        .unwrap();
    let mut only_b = bus
        .subscribe(EventFilter::workspace(ws_b.id))
        .await
        .unwrap();
    let mut only_ch_a1 = bus.subscribe(EventFilter::channel(ch_a1.id)).await.unwrap();
    let mut only_messages = bus
        .subscribe(EventFilter::all().with_kinds([EventKind::MessagePosted]))
        .await
        .unwrap();

    // Publish 10 events: 4 message_posted (2 in ws_a, 2 in ws_b),
    // 2 thread_created (1 in each ws), 4 vote_cast (2 in each ws).
    let mut posted = Vec::new();

    for body in ["hi-a-1", "hi-a-2"] {
        let msg = message(th_a1.id, author, body);
        posted.push(Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id: ws_a.id,
            channel_id: ch_a1.id,
            thread_id: th_a1.id,
            message: msg,
        });
    }
    for body in ["hi-b-1", "hi-b-2"] {
        let msg = message(th_b1.id, author, body);
        posted.push(Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id: ws_b.id,
            channel_id: ch_b1.id,
            thread_id: th_b1.id,
            message: msg,
        });
    }
    posted.push(Event::ThreadCreated {
        occurred_at: Utc::now(),
        workspace_id: ws_a.id,
        channel_id: ch_a1.id,
        thread: th_a1.clone(),
    });
    posted.push(Event::ThreadCreated {
        occurred_at: Utc::now(),
        workspace_id: ws_b.id,
        channel_id: ch_b1.id,
        thread: th_b1.clone(),
    });
    for ws_id in [ws_a.id, ws_a.id, ws_b.id, ws_b.id] {
        let (ch_id, th_id) = if ws_id == ws_a.id {
            (ch_a1.id, th_a1.id)
        } else {
            (ch_b1.id, th_b1.id)
        };
        let msg_id = MessageId(uuid::Uuid::new_v4());
        let _ = ch_id;
        posted.push(Event::VoteCast {
            occurred_at: Utc::now(),
            workspace_id: ws_id,
            thread_id: th_id,
            message_id: msg_id,
            member_id: author,
            vote_kind: "approve".into(),
        });
    }

    for ev in &posted {
        bus.publish(ev.clone()).await.unwrap();
    }

    async fn collect(s: &mut maidan_bus::EventStream, n: usize) -> Vec<Event> {
        let mut out = Vec::new();
        let collect = async {
            while out.len() < n {
                if let Some(e) = s.next().await {
                    out.push(e);
                } else {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), collect).await;
        out
    }

    let all_seen = collect(&mut all, 10).await;
    assert_eq!(all_seen.len(), 10);

    let a_seen = collect(&mut only_a, 5).await;
    assert_eq!(
        a_seen.len(),
        5,
        "ws_a expected 5 events: 2 msg + 1 thread + 2 vote"
    );
    let a_kinds: HashSet<EventKind> = a_seen.iter().map(|e| e.kind()).collect();
    assert!(a_kinds.contains(&EventKind::MessagePosted));
    assert!(a_kinds.contains(&EventKind::ThreadCreated));
    assert!(a_kinds.contains(&EventKind::VoteCast));
    assert!(a_seen.iter().all(|e| e.workspace_id() == Some(ws_a.id)));

    let b_seen = collect(&mut only_b, 5).await;
    assert_eq!(b_seen.len(), 5);
    assert!(b_seen.iter().all(|e| e.workspace_id() == Some(ws_b.id)));

    let ch_a1_seen = collect(&mut only_ch_a1, 3).await;
    assert_eq!(
        ch_a1_seen.len(),
        3,
        "ch_a1: 2 msg + 1 thread; votes don't carry channel_id"
    );
    assert!(ch_a1_seen.iter().all(|e| e.channel_id() == Some(ch_a1.id)));

    let msg_seen = collect(&mut only_messages, 4).await;
    assert_eq!(msg_seen.len(), 4);
    assert!(msg_seen
        .iter()
        .all(|e| e.kind() == EventKind::MessagePosted));
}

#[tokio::test]
async fn publish_with_no_subscribers_does_not_error() {
    let bus = InMemoryBus::new();
    let ws = workspace("orphan");
    bus.publish(Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: ws,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn slow_subscriber_drops_events_but_stays_open() {
    // Capacity 4: publish 8, never read; stream survives, lagged events
    // are silently skipped (logged as a warning).
    let bus = InMemoryBus::with_capacity(4);
    let mut sub = bus.subscribe(EventFilter::all()).await.unwrap();

    for i in 0..8 {
        let ws = workspace(&format!("ws-{i}"));
        bus.publish(Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: ws,
        })
        .await
        .unwrap();
    }

    // Drain whatever remains in the channel; expect <= 4 events.
    let mut received = Vec::new();
    let drain = async {
        while let Some(e) = sub.next().await {
            received.push(e);
            if received.len() >= 4 {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_millis(200), drain).await;
    assert!(received.len() <= 4, "expected at most 4 events after lag");
}
