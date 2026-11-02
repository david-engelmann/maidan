//! The digest sweeper's watermark and metering, on both backends (#1721). A
//! failed watermark save sends nothing and meters nothing, and the member
//! stays due; a failed send puts the watermark back so the digest retries;
//! a sent digest is metered once and the member is no longer due.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{digest, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;

struct Backend {
    /// Makes every watermark write fail, or lets them through again.
    break_watermark: Box<dyn Fn(bool) -> futures::future::BoxFuture<'static, ()> + Send + Sync>,
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    _dir: tempfile::TempDir,
}

async fn sqlite() -> Backend {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let trigger_pool = pool.clone();
    Backend {
        break_watermark: Box::new(move |on| {
            let pool = trigger_pool.clone();
            Box::pin(async move {
                let sql = if on {
                    "CREATE TRIGGER fail_digest_watermark BEFORE INSERT ON maidan_member_digest_state
                     BEGIN SELECT RAISE(ABORT, 'watermark write refused'); END"
                } else {
                    "DROP TRIGGER fail_digest_watermark"
                };
                sqlx::query(sql).execute(&pool).await.unwrap();
            })
        }),
        store: Arc::new(SqliteStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::SqliteSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    }
}

fn app_state(backend: &Backend) -> AppState {
    let mut state = AppState::new(
        backend.store.clone(),
        Arc::new(LocalFsStore::new(backend._dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        backend.search.clone(),
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, Some(Arc::new([0x5e; 32]))),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.usage_allowance = Arc::new(UsageAllowance::parse("deliveries=10").unwrap());
    state
}

async fn signed_up(store: &dyn Store, who: &str) -> WorkspaceId {
    store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open: true,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
                metadata: json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap()
        .workspace
        .id
}

struct RecordingMailer {
    sent: std::sync::Mutex<Vec<String>>,
    fail: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl maidan_server::mail::MailTransport for RecordingMailer {
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        _body: &str,
    ) -> Result<(), maidan_server::mail::MailError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(maidan_server::mail::MailError::Send("down".into()));
        }
        self.sent.lock().unwrap().push(to.to_string());
        Ok(())
    }
}

fn counted(ws: WorkspaceId, taken: &[UsageMeterDay]) -> i64 {
    taken
        .iter()
        .filter(|d| d.workspace_id == ws && d.meter == UsageMeter::Deliveries)
        .map(|d| d.quantity)
        .sum()
}

async fn digest_member(store: &dyn Store, ws: WorkspaceId, log_id: i64) {
    let member = store
        .list_members(ws)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    store
        .set_member_email(member.id, &format!("{}@example.com", member.handle))
        .await
        .unwrap();
    store
        .set_delivery_mode(member.id, EmailDeliveryMode::Digest)
        .await
        .unwrap();
    store
        .create_notification(NewNotification {
            workspace_id: ws,
            member_id: member.id,
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

async fn suite(backend: &Backend) {
    let mut state = app_state(backend);
    let mailer = Arc::new(RecordingMailer {
        sent: std::sync::Mutex::new(Vec::new()),
        fail: std::sync::atomic::AtomicBool::new(false),
    });
    state.attach_mail(mailer.clone());
    let store = backend.store.as_ref();
    let a = signed_up(store, "ada").await;
    let b = signed_up(store, "bea").await;
    digest_member(store, a, 1).await;
    digest_member(store, b, 2).await;
    let _ = state.usage_meters.take();

    // The watermark can't be saved: nothing is sent or metered, both stay due.
    (backend.break_watermark)(true).await;
    assert_eq!(digest::sweep_once(&state).await, 0);
    assert!(
        mailer.sent.lock().unwrap().is_empty(),
        "no send without a saved watermark"
    );
    let taken = state.usage_meters.take();
    assert_eq!(counted(a, &taken) + counted(b, &taken), 0);
    (backend.break_watermark)(false).await;
    assert_eq!(
        store.members_due_for_digest(10, &[]).await.unwrap().len(),
        2
    );

    // The mail transport is down: the watermark is put back, both stay due.
    mailer.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(digest::sweep_once(&state).await, 0);
    assert_eq!(
        store.members_due_for_digest(10, &[]).await.unwrap().len(),
        2,
        "a failed send restores the watermark"
    );
    let taken = state.usage_meters.take();
    assert_eq!(counted(a, &taken) + counted(b, &taken), 0);

    // Sent: each tenant metered once, and nobody is due or re-sent.
    mailer
        .fail
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(digest::sweep_once(&state).await, 2);
    let taken = state.usage_meters.take();
    assert_eq!(counted(a, &taken), 1);
    assert_eq!(counted(b, &taken), 1);
    assert!(store
        .members_due_for_digest(10, &[])
        .await
        .unwrap()
        .is_empty());
    assert_eq!(digest::sweep_once(&state).await, 0, "no re-send");
    assert_eq!(mailer.sent.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn digest_watermark_and_metering_on_sqlite() {
    suite(&sqlite().await).await;
}

#[tokio::test]
async fn digest_watermark_and_metering_on_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    sqlx::query(
        "CREATE FUNCTION fail_digest_watermark() RETURNS trigger LANGUAGE plpgsql AS
         $$ BEGIN RAISE EXCEPTION 'watermark write refused'; END $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    let trigger_pool = pool.clone();
    let backend = Backend {
        break_watermark: Box::new(move |on| {
            let pool = trigger_pool.clone();
            Box::pin(async move {
                let sql = if on {
                    "CREATE TRIGGER fail_digest_watermark BEFORE INSERT OR UPDATE ON maidan_member_digest_state
                     FOR EACH ROW EXECUTE FUNCTION fail_digest_watermark()"
                } else {
                    "DROP TRIGGER fail_digest_watermark ON maidan_member_digest_state"
                };
                sqlx::query(sql).execute(&pool).await.unwrap();
            })
        }),
        store: Arc::new(PostgresStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::PostgresSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    };
    suite(&backend).await;
}
