//! Cluster 238: the notification router resolves a `MentionRecorded` event to a
//! per-recipient notification row, and dedups replays / multi-replica delivery.

use std::sync::Arc;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{notification_router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{Event, EventKind, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn router_writes_a_notification_per_mention_and_dedups() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);

    let ws = store
        .create_workspace(NewWorkspace { name: "n".into() })
        .await
        .unwrap();
    let mentioned = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "mentioned".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();

    let message_id = maidan_types::MessageId::new();
    let mention = Event::MentionRecorded {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        thread_id: thread.id,
        message_id,
        member_id: mentioned.id,
    };

    // No notifications before routing.
    assert!(store
        .list_notifications(mentioned.id, false, 10)
        .await
        .unwrap()
        .is_empty());

    // Routing the mention writes one notification with the resolved context.
    notification_router::route_event(&state, 1, &mention)
        .await
        .unwrap();
    let notes = store
        .list_notifications(mentioned.id, false, 10)
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    let n = &notes[0];
    assert_eq!(n.member_id, mentioned.id);
    assert_eq!(n.kind, EventKind::MentionRecorded);
    assert_eq!(n.source_log_id, 1);
    assert_eq!(
        n.channel_id,
        Some(channel.id),
        "channel resolved from the thread"
    );
    assert_eq!(n.thread_id, Some(thread.id));
    assert_eq!(n.message_id, Some(message_id));
    assert!(n.read_at.is_none());
    assert_eq!(
        store.unread_notification_count(mentioned.id).await.unwrap(),
        1
    );

    // Re-routing the SAME event (a replay or a second replica) does not
    // double-notify — dedup on (member_id, source_log_id).
    notification_router::route_event(&state, 1, &mention)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        1,
        "a replay of the same event is deduped"
    );

    // A distinct event (new log_id) is a distinct notification.
    notification_router::route_event(&state, 2, &mention)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        2
    );

    // A non-mention event routes to nothing.
    let ready = Event::ThreadReady {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        thread: thread.clone(),
    };
    notification_router::route_event(&state, 3, &ready)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        2,
        "a non-mention event produces no notification (yet)"
    );

    // Cluster 242: muting the kind makes the router skip the write.
    store
        .set_notification_pref(mentioned.id, EventKind::MentionRecorded, true)
        .await
        .unwrap();
    let muted_mention = Event::MentionRecorded {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        thread_id: thread.id,
        message_id: maidan_types::MessageId::new(),
        member_id: mentioned.id,
    };
    notification_router::route_event(&state, 4, &muted_mention)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        2,
        "a muted kind is suppressed — no new notification"
    );

    // Cluster 245: a channel follower gets a MessagePosted notification; the author
    // and non-followers don't.
    let author = store
        .create_member(maidan_types::NewMember {
            workspace_id: ws.id,
            handle: "author".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let follower = store
        .create_member(maidan_types::NewMember {
            workspace_id: ws.id,
            handle: "follower".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    store.follow_channel(follower.id, channel.id).await.unwrap();
    store.follow_channel(author.id, channel.id).await.unwrap(); // author follows too
    let msg = store
        .post_message(maidan_types::NewMessage {
            thread_id: thread.id,
            author_id: author.id,
            body: "hello followers".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let posted = Event::MessagePosted {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        dm_conversation_id: None,
        message: msg,
    };
    notification_router::route_event(&state, 5, &posted)
        .await
        .unwrap();
    // The follower is notified; the author (also a follower) is NOT (own message).
    assert_eq!(
        store
            .list_notifications(follower.id, false, 10)
            .await
            .unwrap()
            .len(),
        1,
        "a channel follower is notified of the new message"
    );
    assert!(
        store
            .list_notifications(author.id, false, 10)
            .await
            .unwrap()
            .is_empty(),
        "the author is not notified of their own message"
    );
    // The mentioned member (not following this channel) gets nothing new from the post.
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        2,
        "a non-follower gets no follow notification"
    );
}

/// Cluster 249: a recording transport that captures what would be emailed.
struct RecordingMailer {
    sent: std::sync::Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl maidan_server::mail::MailTransport for RecordingMailer {
    async fn send(
        &self,
        to: &str,
        subject: &str,
        _body: &str,
    ) -> Result<(), maidan_server::mail::MailError> {
        self.sent
            .lock()
            .unwrap()
            .push((to.to_string(), subject.to_string()));
        Ok(())
    }
}

#[tokio::test]
async fn email_delivery_when_configured_and_address_present() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    let mailer = Arc::new(RecordingMailer {
        sent: std::sync::Mutex::new(Vec::new()),
    });
    state.attach_mail(mailer.clone());

    let ws = store
        .create_workspace(NewWorkspace { name: "e".into() })
        .await
        .unwrap();
    let with_addr = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "has-email".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let no_addr = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "no-email".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .set_member_email(with_addr.id, "user@example.com")
        .await
        .unwrap();

    // A member with an address on file gets emailed.
    notification_router::deliver_notification_email(
        &state,
        with_addr.id,
        EventKind::MentionRecorded,
        1,
    )
    .await;
    // A member without one does not.
    notification_router::deliver_notification_email(
        &state,
        no_addr.id,
        EventKind::MentionRecorded,
        2,
    )
    .await;

    let sent = mailer.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "only the member with an address is emailed");
    assert_eq!(sent[0].0, "user@example.com");
}
