//! Persistent event log append + replay.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{Event, EventKind, MemberKind, NewChannel, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn append_and_replay_events_in_order() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "log-ws".to_string(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "log-ch".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");

    let e1 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("append1");
    let e2 = store
        .append_event(&Event::ChannelCreated {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel: ch,
        })
        .await
        .expect("append2");

    assert_eq!(e1.kind, EventKind::MemberJoined);
    assert_eq!(e2.kind, EventKind::ChannelCreated);
    assert!(e2.id > e1.id);

    let page = store.list_events_after(ws.id, 0, 10).await.expect("list");
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, e1.id);
    assert_eq!(page[1].id, e2.id);

    let tail = store
        .list_events_after(ws.id, e1.id, 10)
        .await
        .expect("tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].id, e2.id);
}

#[tokio::test]
async fn list_events_after_stable_gates_on_insert_time() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "stable-ws".to_string(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");

    let e1 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("append1");
    let e2 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member,
        })
        .await
        .expect("append2");

    // A horizon in the past treats the just-inserted rows as not-yet-stable.
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let none = store
        .list_events_after_stable(ws.id, 0, past, 10)
        .await
        .expect("stable past");
    assert!(
        none.is_empty(),
        "rows inserted now must be excluded by a past horizon, got {}",
        none.len()
    );

    // A horizon in the future treats them all as stable, in id order, honoring after_id.
    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    let all = store
        .list_events_after_stable(ws.id, 0, future, 10)
        .await
        .expect("stable future");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, e1.id);
    assert_eq!(all[1].id, e2.id);

    let tail = store
        .list_events_after_stable(ws.id, e1.id, future, 10)
        .await
        .expect("stable tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].id, e2.id);
}

#[tokio::test]
async fn get_stored_event_returns_row_and_missing_is_not_found() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "get-log".to_string(),
        })
        .await
        .expect("ws");
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: "u".to_string(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .expect("member"),
        })
        .await
        .expect("append");

    let fetched = store.get_stored_event(stored.id).await.expect("get");
    assert_eq!(fetched.id, stored.id);
    assert_eq!(fetched.kind, EventKind::MemberJoined);

    let err = store.get_stored_event(999_999).await.unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}

/// Cluster 205: `*_with_event` commits the domain row and its event atomically —
/// after the single call, both the row and the durable event exist.
#[tokio::test]
async fn create_with_event_commits_row_and_event() {
    use maidan_types::NewThread;
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = store
        .create_workspace(NewWorkspace {
            name: "tx-outbox".to_string(),
        })
        .await
        .expect("ws");

    let (ch, ch_event) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "atomic".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel + event");
    assert_eq!(ch_event.kind, EventKind::ChannelCreated);
    // The channel row committed.
    assert_eq!(
        store.get_channel(ch.id).await.expect("get channel").id,
        ch.id
    );

    let (th, th_event) = store
        .create_thread_with_event(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".to_string()),
        })
        .await
        .expect("thread + event");
    assert_eq!(th_event.kind, EventKind::ThreadCreated);
    assert_eq!(store.get_thread(th.id).await.expect("get thread").id, th.id);

    // Both events are durably in the log — the returned events are the logged ones.
    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    assert!(events
        .iter()
        .any(|e| e.id == ch_event.id && e.kind == EventKind::ChannelCreated));
    assert!(events
        .iter()
        .any(|e| e.id == th_event.id && e.kind == EventKind::ThreadCreated));
}

/// Cluster 206: the social `*_with_event` mutations append their event in the
/// same tx as the row — a cast vote / added reaction produces a durable event.
#[tokio::test]
async fn social_with_event_appends_atomically() {
    use maidan_types::{MemberId, NewReaction, NewThread, NewVote};
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = store
        .create_workspace(NewWorkspace {
            name: "social-tx".to_string(),
        })
        .await
        .expect("ws");
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let (th, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".to_string()),
        })
        .await
        .expect("thread");
    let msg = store
        .post_message(maidan_types::NewMessage {
            thread_id: th.id,
            author_id: author.id,
            body: "hi".to_string(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("msg");

    let vote_event = store
        .cast_vote_with_event(NewVote {
            message_id: msg.id,
            member_id: MemberId(author.id.0),
            kind: "up".to_string(),
        })
        .await
        .expect("vote");
    assert_eq!(vote_event.kind, EventKind::VoteCast);

    let react_event = store
        .add_reaction_with_event(NewReaction {
            message_id: msg.id,
            member_id: MemberId(author.id.0),
            emoji: "👍".to_string(),
        })
        .await
        .expect("reaction");
    assert_eq!(react_event.kind, EventKind::ReactionAdded);

    // A removal that hits nothing produces no event; a real one does.
    let (removed_none, none_event) = store
        .remove_reaction_with_event(msg.id, MemberId(author.id.0), "🚫")
        .await
        .expect("remove miss");
    assert!(!removed_none && none_event.is_none());
    let (removed, some_event) = store
        .remove_reaction_with_event(msg.id, MemberId(author.id.0), "👍")
        .await
        .expect("remove hit");
    assert!(removed && some_event.is_some());

    // All the produced events are durably in the log.
    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    for id in [vote_event.id, react_event.id, some_event.unwrap().id] {
        assert!(events.iter().any(|e| e.id == id), "event {id} durable");
    }
}

/// Cluster 207: pins + mentions migrated to the transactional-outbox pattern.
/// A pin/mention appends its event in the same tx; an unpin miss produces no
/// event, a real unpin does.
#[tokio::test]
async fn pins_and_mentions_with_event_append_atomically() {
    use maidan_types::{MemberId, NewMessage, NewPin, NewThread};
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = store
        .create_workspace(NewWorkspace {
            name: "pin-tx".to_string(),
        })
        .await
        .expect("ws");
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let (th, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".to_string()),
        })
        .await
        .expect("thread");
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: author.id,
            body: "hi".to_string(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("msg");

    let pin_event = store
        .pin_message_with_event(NewPin {
            thread_id: th.id,
            message_id: msg.id,
            member_id: MemberId(author.id.0),
        })
        .await
        .expect("pin");
    assert_eq!(pin_event.kind, EventKind::MessagePinned);

    let mention_event = store
        .record_mention_with_event(msg.id, MemberId(author.id.0))
        .await
        .expect("mention");
    assert_eq!(mention_event.kind, EventKind::MentionRecorded);

    // Unpin a message that was never pinned → no event; the real one → event.
    let other = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: author.id,
            body: "bye".to_string(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("other msg");
    let (removed_none, none_event) = store
        .unpin_message_with_event(th.id, other.id, MemberId(author.id.0))
        .await
        .expect("unpin miss");
    assert!(!removed_none && none_event.is_none());
    let (removed, some_event) = store
        .unpin_message_with_event(th.id, msg.id, MemberId(author.id.0))
        .await
        .expect("unpin hit");
    assert!(removed && some_event.is_some());
    assert_eq!(
        some_event.as_ref().unwrap().kind,
        EventKind::MessageUnpinned
    );

    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    for id in [pin_event.id, mention_event.id, some_event.unwrap().id] {
        assert!(events.iter().any(|e| e.id == id), "event {id} durable");
    }
}

/// Cluster 208: thread transitions migrated to the transactional-outbox pattern.
/// A transition appends its `ThreadStateChanged` event in the same tx as the
/// state change, over the new `thread_scope_in_tx` resolver.
#[tokio::test]
async fn transition_with_event_appends_atomically() {
    use maidan_fsm::ThreadAction;
    use maidan_types::{MemberId, NewThread, ThreadState};
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = store
        .create_workspace(NewWorkspace {
            name: "transition-tx".to_string(),
        })
        .await
        .expect("ws");
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let (th, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".to_string()),
        })
        .await
        .expect("thread");

    let (result, stored) = store
        .transition_thread_with_event(th.id, MemberId(actor.id.0), ThreadAction::StartReview)
        .await
        .expect("transition");
    assert_eq!(stored.kind, EventKind::ThreadStateChanged);
    assert_eq!(result.from_state, ThreadState::Open);
    assert_eq!(result.to_state, ThreadState::InReview);
    // The state change committed.
    assert_eq!(
        store.get_thread(th.id).await.expect("get").state,
        ThreadState::InReview
    );
    // The event is durably in the log.
    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    assert!(
        events.iter().any(|e| e.id == stored.id),
        "ThreadStateChanged durable"
    );
}
