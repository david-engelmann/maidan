//! Web Push router integration: `deliver_notification_web_push` sends to a
//! member's subscriptions only when they have no live WS (not seen within the
//! live window), and prunes a subscription the push service reports `Gone`.
//! Uses a recording mock sender attached via `attach_web_push`.

use std::sync::{Arc, Mutex};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::web_push::{WebPushConfig, WebPushConfigError, WebPushError, WebPushSender};
use maidan_server::web_push_worker;
use maidan_server::{notification_router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EventKind, MemberKind, NewMember, NewPushSubscription, NewWorkspace, PushSubscription,
};

struct RecordingSender {
    sent: Mutex<Vec<String>>,
    gone: bool,
}

#[async_trait::async_trait]
impl WebPushSender for RecordingSender {
    async fn send(&self, sub: &PushSubscription, _payload: &[u8]) -> Result<(), WebPushError> {
        self.sent.lock().unwrap().push(sub.endpoint.clone());
        if self.gone {
            Err(WebPushError::Endpoint(410))
        } else {
            Ok(())
        }
    }
}

async fn setup() -> (Arc<dyn Store>, AppState) {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);
    (store, state)
}

use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn web_push_delivers_when_offline_skips_when_present_and_prunes_gone() {
    let (store, mut state) = setup().await;
    let sender = Arc::new(RecordingSender {
        sent: Mutex::new(Vec::new()),
        gone: false,
    });
    state.attach_web_push(sender.clone());

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .add_push_subscription(NewPushSubscription {
            member_id: member.id,
            endpoint: "https://push.example.com/a".into(),
            p256dh: "k".into(),
            auth: "s".into(),
        })
        .await
        .unwrap();

    // Offline (never seen) → delivered.
    notification_router::deliver_notification_web_push(
        &state,
        member.id,
        EventKind::MentionRecorded,
        7,
    )
    .await;
    assert_eq!(
        sender.sent.lock().unwrap().len(),
        1,
        "offline member is pushed"
    );

    // Present (seen just now) → skipped.
    store.touch_member_last_seen(member.id).await.unwrap();
    notification_router::deliver_notification_web_push(
        &state,
        member.id,
        EventKind::MentionRecorded,
        8,
    )
    .await;
    assert_eq!(
        sender.sent.lock().unwrap().len(),
        1,
        "a member with a live WS is not pushed"
    );
}

#[tokio::test]
async fn web_push_prunes_a_gone_subscription() {
    let (store, mut state) = setup().await;
    let sender = Arc::new(RecordingSender {
        sent: Mutex::new(Vec::new()),
        gone: true, // the push service reports 410 Gone
    });
    state.attach_web_push(sender.clone());

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .add_push_subscription(NewPushSubscription {
            member_id: member.id,
            endpoint: "https://push.example.com/dead".into(),
            p256dh: "k".into(),
            auth: "s".into(),
        })
        .await
        .unwrap();

    notification_router::deliver_notification_web_push(
        &state,
        member.id,
        EventKind::MentionRecorded,
        9,
    )
    .await;
    assert_eq!(sender.sent.lock().unwrap().len(), 1, "attempted once");
    assert!(
        store
            .list_push_subscriptions(member.id)
            .await
            .unwrap()
            .is_empty(),
        "a Gone subscription is pruned"
    );
}

struct FlakySender {
    fails_left: Mutex<u32>,
    sent: Mutex<u32>,
}

#[async_trait::async_trait]
impl WebPushSender for FlakySender {
    async fn send(&self, _sub: &PushSubscription, _payload: &[u8]) -> Result<(), WebPushError> {
        *self.sent.lock().unwrap() += 1;
        let mut left = self.fails_left.lock().unwrap();
        if *left > 0 {
            *left -= 1;
            return Err(WebPushError::Endpoint(503));
        }
        Ok(())
    }
}

#[test]
fn unset_vapid_is_a_named_skip_and_not_a_generated_key() {
    match WebPushConfig::from_values(None, None, None) {
        Err(err) => {
            assert_eq!(err, WebPushConfigError::Unset);
            assert_eq!(err.to_string(), "vapid_unset");
        }
        Ok(_) => panic!("unset VAPID must not become a key"),
    }
}

#[tokio::test]
async fn web_push_without_a_sender_does_not_queue() {
    let (store, state) = setup().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .add_push_subscription(NewPushSubscription {
            member_id: member.id,
            endpoint: "https://push.example.com/a".into(),
            p256dh: "k".into(),
            auth: "s".into(),
        })
        .await
        .unwrap();
    notification_router::deliver_notification_web_push(
        &state,
        member.id,
        EventKind::MentionRecorded,
        1,
    )
    .await;
    let queued = store
        .claim_next_due_web_push(chrono::Utc::now() + chrono::Duration::hours(2), 30)
        .await
        .unwrap();
    assert!(queued.is_none(), "no sender means nothing is queued");
    assert_eq!(
        state.web_push_disabled_reason.as_deref(),
        Some("vapid_unset")
    );
}

#[tokio::test]
async fn failed_web_push_is_retried_from_the_outbox() {
    let (store, mut state) = setup().await;
    let sender = Arc::new(FlakySender {
        fails_left: Mutex::new(1),
        sent: Mutex::new(0),
    });
    state.attach_web_push(sender.clone());
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .add_push_subscription(NewPushSubscription {
            member_id: member.id,
            endpoint: "https://push.example.com/retry".into(),
            p256dh: "k".into(),
            auth: "s".into(),
        })
        .await
        .unwrap();

    notification_router::deliver_notification_web_push(
        &state,
        member.id,
        EventKind::MentionRecorded,
        11,
    )
    .await;
    assert_eq!(
        *sender.sent.lock().unwrap(),
        1,
        "the first send was attempted"
    );

    // The failed send is due after backoff. Sweep as of later so the test does
    // not wait on the wall clock.
    let stats =
        web_push_worker::sweep_due(&state, chrono::Utc::now() + chrono::Duration::hours(2)).await;
    assert_eq!(stats.sent, 1, "the queued push is sent on the next sweep");
    assert_eq!(*sender.sent.lock().unwrap(), 2);
    let left = store
        .claim_next_due_web_push(chrono::Utc::now() + chrono::Duration::hours(2), 30)
        .await
        .unwrap();
    assert!(left.is_none(), "a successful retry leaves the queue");
}
