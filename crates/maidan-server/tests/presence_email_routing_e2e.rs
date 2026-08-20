//! Cluster 253: presence-aware email routing. With
//! `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` set, `deliver_notification_email` skips a
//! recipient who was seen within the window (they are active — the in-app
//! notification suffices) and still emails one who was not. In its own test
//! binary so the process-global env var can't race the other router tests.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{notification_router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{EventKind, MemberKind, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

struct RecordingMailer {
    sent: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl maidan_server::mail::MailTransport for RecordingMailer {
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        _body: &str,
    ) -> Result<(), maidan_server::mail::MailError> {
        self.sent.lock().unwrap().push(to.to_string());
        Ok(())
    }
}

#[tokio::test]
async fn presence_window_skips_email_for_recently_seen_member() {
    // Enable the guard for this test binary. A member seen within 300 s is
    // treated as active; unset/0 would send unconditionally (Cluster 249).
    std::env::set_var("MAIDAN_EMAIL_PRESENCE_WINDOW_SECS", "300");

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
    let present = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "present".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let away = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "away".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    // Both have an address on file; the difference is presence.
    store
        .set_member_email(present.id, "present@example.com")
        .await
        .unwrap();
    store
        .set_member_email(away.id, "away@example.com")
        .await
        .unwrap();

    // `present` was just seen -> inside the window -> email suppressed.
    store.touch_member_last_seen(present.id).await.unwrap();
    // `away` was never seen -> not active -> email sent.

    notification_router::deliver_notification_email(
        &state,
        present.id,
        EventKind::MentionRecorded,
        1,
    )
    .await;
    notification_router::deliver_notification_email(&state, away.id, EventKind::MentionRecorded, 2)
        .await;

    let sent = mailer.sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        1,
        "only the away member is emailed (present one is skipped)"
    );
    assert_eq!(sent[0], "away@example.com");
}
