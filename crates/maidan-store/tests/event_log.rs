//! Persistent event log append + replay.

use maidan_store::{prelude::*, run_sqlite_migrations};
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
            confidence: None,
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

/// Cluster 209: thread assignments migrated to the transactional-outbox pattern.
/// assign/unassign always emit; claim/claim_next emit only when they claimed.
#[tokio::test]
async fn assignment_with_event_appends_atomically() {
    use maidan_types::{MemberId, NewThread};
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
            name: "assign-tx".to_string(),
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
    let actor_id = MemberId(actor.id.0);

    // assign → ThreadAssignmentChanged with previous None, note carried.
    let (assigned, assign_ev) = store
        .assign_thread_with_event(th.id, actor_id, actor_id, Some("take it".to_string()))
        .await
        .expect("assign");
    assert_eq!(assign_ev.kind, EventKind::ThreadAssignmentChanged);
    assert_eq!(assigned.assignee_id, Some(actor_id));

    // unassign → event, assignee cleared.
    let (unassigned, unassign_ev) = store
        .unassign_thread_with_event(th.id, actor_id)
        .await
        .expect("unassign");
    assert_eq!(unassign_ev.kind, EventKind::ThreadAssignmentChanged);
    assert_eq!(unassigned.assignee_id, None);

    // claim on the now-unassigned thread → event.
    let (claim_res, claim_ev) = store
        .claim_thread_with_event(th.id, actor_id)
        .await
        .expect("claim");
    assert!(claim_res.claimed && claim_ev.is_some());
    // claim again (already assigned) → no event.
    let (claim_res2, claim_ev2) = store
        .claim_thread_with_event(th.id, actor_id)
        .await
        .expect("claim2");
    assert!(!claim_res2.claimed && claim_ev2.is_none());

    // claim_next in a fresh channel with one unassigned thread → event, then null.
    let ch2 = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c2".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch2");
    let (th2, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch2.id,
            parent_thread_id: None,
            title: Some("t2".to_string()),
        })
        .await
        .expect("thread2");
    let (next, next_ev) = store
        .claim_next_thread_with_event(ch2.id, actor_id, None)
        .await
        .expect("claim_next");
    assert_eq!(next.map(|t| t.id), Some(th2.id));
    assert!(next_ev.is_some());
    let (none_next, none_ev) = store
        .claim_next_thread_with_event(ch2.id, actor_id, None)
        .await
        .expect("claim_next empty");
    assert!(none_next.is_none() && none_ev.is_none());

    // All emitted events are durably in the log.
    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    for id in [
        assign_ev.id,
        unassign_ev.id,
        claim_ev.unwrap().id,
        next_ev.unwrap().id,
    ] {
        assert!(events.iter().any(|e| e.id == id), "event {id} durable");
    }
}

/// Cluster 210: DM/group-DM posts migrated to the transactional-outbox pattern.
/// `post_message_with_event` inserts the message and appends `MessagePosted` in
/// one tx, threading `dm_conversation_id` (Some for a 1:1 DM, None for a group).
#[tokio::test]
async fn dm_post_with_event_appends_atomically() {
    use maidan_types::{DmConversationId, NewMessage, NewThread};
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
            name: "dm-tx".to_string(),
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
    let new = |body: &str| NewMessage {
        thread_id: th.id,
        author_id: author.id,
        body: body.to_string(),
        metadata: serde_json::json!({}),
        content: None,
    };

    // 1:1 DM → dm_conversation_id carried through to the event payload.
    let dm_id = DmConversationId(uuid::Uuid::from_u128(1));
    let (m1, dm_ev) = store
        .post_message_with_event(new("hi"), Some(dm_id))
        .await
        .expect("dm post");
    assert_eq!(dm_ev.kind, EventKind::MessagePosted);
    match serde_json::from_value::<Event>(dm_ev.payload.clone()).expect("event") {
        Event::MessagePosted {
            dm_conversation_id,
            message,
            ..
        } => {
            assert_eq!(dm_conversation_id, Some(dm_id));
            assert_eq!(message.id, m1.id);
        }
        other => panic!("expected MessagePosted, got {other:?}"),
    }

    // Group DM → dm_conversation_id None.
    let (_m2, grp_ev) = store
        .post_message_with_event(new("yo"), None)
        .await
        .expect("group post");
    assert_eq!(grp_ev.kind, EventKind::MessagePosted);

    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    for id in [dm_ev.id, grp_ev.id] {
        assert!(events.iter().any(|e| e.id == id), "event {id} durable");
    }
}

/// Cluster 211: the regular message-post path's slash finalization —
/// `edit_message_with_posted_event` commits the edit and a `MessagePosted` event
/// reflecting the **edited** message in one tx, and records edit history when the
/// body changes.
#[tokio::test]
async fn message_post_finalize_with_event_appends_atomically() {
    use maidan_types::{EditMessage, MemberId, NewMessage, NewThread};
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
            name: "post-finalize".to_string(),
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

    // Provisional insert (as the route does before slash dispatch).
    let m = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: author.id,
            body: "/deploy prod".to_string(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("provisional");

    // Metadata-only finalization (body unchanged — the slash path's shape).
    let (finalized, stored) = store
        .edit_message_with_posted_event(
            m.id,
            MemberId(author.id.0),
            EditMessage {
                body: m.body.clone(),
                metadata: serde_json::json!({"slash_command": "deploy"}),
                content: m.content.clone(),
            },
            None,
        )
        .await
        .expect("finalize");
    assert_eq!(stored.kind, EventKind::MessagePosted);
    // The event carries the post-edit message.
    match serde_json::from_value::<Event>(stored.payload.clone()).expect("event") {
        Event::MessagePosted { message, .. } => {
            assert_eq!(message.id, finalized.id);
            assert_eq!(message.metadata["slash_command"], "deploy");
        }
        other => panic!("expected MessagePosted, got {other:?}"),
    }
    // No body change → no edit-history row.
    assert!(store
        .list_message_edits(m.id, 10)
        .await
        .expect("edits")
        .is_empty());

    // A body-changing finalize records edit history (still one MessagePosted).
    let (_m2, stored2) = store
        .edit_message_with_posted_event(
            m.id,
            MemberId(author.id.0),
            EditMessage {
                body: "deployed".to_string(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .expect("finalize2");
    assert_eq!(stored2.kind, EventKind::MessagePosted);
    let edits = store.list_message_edits(m.id, 10).await.expect("edits2");
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].body_after, "deployed");

    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    for id in [stored.id, stored2.id] {
        assert!(events.iter().any(|e| e.id == id), "event {id} durable");
    }
}

/// Cluster 212: message edit + tombstone migrated to the transactional-outbox
/// pattern. Each appends its event in the same tx; a re-tombstone is `NotFound`.
#[tokio::test]
async fn edit_and_tombstone_with_event_append_atomically() {
    use maidan_types::{EditMessage, MemberId, NewMessage, NewThread};
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
            name: "edit-tomb-tx".to_string(),
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
    let m = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: author.id,
            body: "hi".to_string(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("msg");

    // Edit → MessageEdited, body changed, one history row.
    let (edited, edit_ev) = store
        .edit_message_with_event(
            m.id,
            MemberId(author.id.0),
            EditMessage {
                body: "hello".to_string(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .expect("edit");
    assert_eq!(edit_ev.kind, EventKind::MessageEdited);
    assert_eq!(edited.body, "hello");
    assert_eq!(
        store
            .list_message_edits(m.id, 10)
            .await
            .expect("edits")
            .len(),
        1
    );

    // Tombstone → MessageTombstoned; re-tombstone → NotFound (no event).
    let tomb_ev = store
        .tombstone_message_with_event(m.id, None)
        .await
        .expect("tombstone");
    assert_eq!(tomb_ev.kind, EventKind::MessageTombstoned);
    assert!(matches!(
        store.tombstone_message_with_event(m.id, None).await,
        Err(maidan_store::StoreError::NotFound)
    ));

    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    for id in [edit_ev.id, tomb_ev.id] {
        assert!(events.iter().any(|e| e.id == id), "event {id} durable");
    }
}

/// Cluster 213: workspace + member creation migrated to the transactional-outbox
/// pattern — `create_workspace_with_event` / `create_member_with_event` commit the
/// row and its `WorkspaceCreated` / `MemberJoined` event in one tx.
#[tokio::test]
async fn create_workspace_and_member_with_event_append_atomically() {
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

    let (ws, ws_ev) = store
        .create_workspace_with_event(NewWorkspace {
            name: "created-tx".to_string(),
        })
        .await
        .expect("workspace + event");
    assert_eq!(ws_ev.kind, EventKind::WorkspaceCreated);
    // The workspace row committed.
    assert_eq!(store.get_workspace(ws.id).await.expect("get").id, ws.id);

    let (member, member_ev) = store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "agent".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member + event");
    assert_eq!(member_ev.kind, EventKind::MemberJoined);
    assert_eq!(
        store.get_member(member.id).await.expect("get").id,
        member.id
    );

    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    for id in [ws_ev.id, member_ev.id] {
        assert!(events.iter().any(|e| e.id == id), "event {id} durable");
    }
}

/// Cluster 214: references + artifacts migrated to the transactional-outbox
/// pattern. `add_reference_with_event` appends `ReferenceAdded`;
/// `upsert_artifact_with_event` appends `ArtifactUpserted` and — for a non-bypass
/// caller (`ref_workspace = Some`) — records the Cluster-204 access ref in the
/// same tx.
#[tokio::test]
async fn reference_and_artifact_with_event_append_atomically() {
    use maidan_types::{
        ArtifactKind, MemberId, NewArtifact, NewMessage, NewReference, NewThread, RefSide,
    };
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
            name: "ref-art-tx".to_string(),
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

    // Reference → ReferenceAdded.
    let (reference, ref_ev) = store
        .add_reference_with_event(NewReference {
            src_kind: RefSide::Message,
            src_id: msg.id.0,
            dst_kind: RefSide::Thread,
            dst_id: th.id.0,
            relation: "derived_from".into(),
        })
        .await
        .expect("reference");
    assert_eq!(ref_ev.kind, EventKind::ReferenceAdded);
    assert_eq!(reference.src_id, msg.id.0);

    // Artifact with a ref (non-bypass) → ArtifactUpserted + a recorded ref.
    let sha_a = "a".repeat(64);
    let (_artifact, art_ev) = store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha_a.clone(),
                size_bytes: 3,
                mime_type: Some("text/plain".to_string()),
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(MemberId(author.id.0)),
            },
            Some(ws.id),
        )
        .await
        .expect("artifact + ref");
    assert_eq!(art_ev.kind, EventKind::ArtifactUpserted);
    assert!(store
        .artifact_ref_exists(ws.id, &sha_a)
        .await
        .expect("ref exists"));

    // Artifact without a ref (bypass) → event only, no ref for this workspace.
    let sha_b = "b".repeat(64);
    let (_artifact2, art_ev2) = store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha_b.clone(),
                size_bytes: 3,
                mime_type: None,
                kind: ArtifactKind::Attachment,
                uploaded_by: None,
            },
            None,
        )
        .await
        .expect("artifact no ref");
    assert_eq!(art_ev2.kind, EventKind::ArtifactUpserted);
    assert!(!store
        .artifact_ref_exists(ws.id, &sha_b)
        .await
        .expect("no ref"));

    // ReferenceAdded / ArtifactUpserted are workspace-less events, so verify
    // durability via the by-id read rather than a workspace-scoped list.
    for stored in [&ref_ev, &art_ev, &art_ev2] {
        assert!(
            store.get_stored_event(stored.id).await.is_ok(),
            "event {} durable",
            stored.id
        );
    }
}
