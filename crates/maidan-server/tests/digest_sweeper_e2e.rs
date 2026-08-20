//! Cluster 255: the digest sweeper + the router honoring digest mode.
//! A digest-mode member gets no immediate email (the router skips them) and
//! instead receives a rollup from `digest::sweep_once`, whose watermark advance
//! makes a second sweep a no-op.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{digest, notification_router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    EmailDeliveryMode, EventKind, MemberId, MemberKind, NewMember, NewNotification, NewWorkspace,
    WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

struct RecordingMailer {
    sent: std::sync::Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl maidan_server::mail::MailTransport for RecordingMailer {
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        body: &str,
    ) -> Result<(), maidan_server::mail::MailError> {
        self.sent
            .lock()
            .unwrap()
            .push((to.to_string(), body.to_string()));
        Ok(())
    }
}

async fn state_with_mailer() -> (AppState, Arc<RecordingMailer>, Arc<dyn Store>) {
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
    (state, mailer, store)
}

async fn member_with_email(
    store: &dyn Store,
    ws: WorkspaceId,
    handle: &str,
    email: &str,
) -> MemberId {
    let m = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store.set_member_email(m.id, email).await.unwrap();
    m.id
}

async fn add_unread(store: &dyn Store, ws: WorkspaceId, member: MemberId, log_id: i64) {
    store
        .create_notification(NewNotification {
            workspace_id: ws,
            member_id: member,
            kind: EventKind::MentionRecorded,
            source_log_id: log_id,
            channel_id: None,
            thread_id: None,
            message_id: None,
            actor_id: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn router_skips_immediate_email_for_digest_mode_member() {
    let (state, mailer, store) = state_with_mailer().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let digester = member_with_email(store.as_ref(), ws.id, "digest", "digest@example.com").await;
    store
        .set_delivery_mode(digester, EmailDeliveryMode::Digest)
        .await
        .unwrap();
    let immediate = member_with_email(store.as_ref(), ws.id, "now", "now@example.com").await;
    // immediate stays default (Immediate).

    notification_router::deliver_notification_email(
        &state,
        digester,
        EventKind::MentionRecorded,
        1,
    )
    .await;
    notification_router::deliver_notification_email(
        &state,
        immediate,
        EventKind::MentionRecorded,
        2,
    )
    .await;

    let sent = mailer.sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        1,
        "only the immediate-mode member is emailed now"
    );
    assert_eq!(sent[0].0, "now@example.com");
}

#[tokio::test]
async fn digest_sweeper_sends_rollup_then_advances_watermark() {
    let (state, mailer, store) = state_with_mailer().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = member_with_email(store.as_ref(), ws.id, "d", "d@example.com").await;
    store
        .set_delivery_mode(member, EmailDeliveryMode::Digest)
        .await
        .unwrap();
    add_unread(store.as_ref(), ws.id, member, 1).await;
    add_unread(store.as_ref(), ws.id, member, 2).await;

    // First sweep: one digest, addressed to the member, mentioning the 2 unread.
    let n = digest::sweep_once(&state).await;
    assert_eq!(n, 1, "one digest sent");
    {
        let sent = mailer.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "d@example.com");
        assert!(sent[0].1.contains('2'), "body mentions the unread count");
    }

    // Second sweep: the watermark advanced past those notifications, so nothing.
    let n = digest::sweep_once(&state).await;
    assert_eq!(n, 0, "watermark advance makes the next sweep a no-op");
    assert_eq!(
        mailer.sent.lock().unwrap().len(),
        1,
        "still just the one digest"
    );
}
