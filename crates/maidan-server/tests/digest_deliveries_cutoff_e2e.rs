//! The email digest sweeper under the hard deliveries cutoff, on both
//! backends, with two open sign-up tenants. A member of a workspace past its
//! deliveries allowance gets no digest, nothing is counted, and the watermark
//! stays so the digest goes out after the cutoff lifts. The other tenant's
//! member gets theirs, counted as one delivery to that tenant only. Postgres
//! skips when Docker is unavailable.

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
use sqlx::sqlite::SqlitePoolOptions;

struct Backend {
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    _dir: tempfile::TempDir,
}

async fn sqlite() -> Backend {
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
    Backend {
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

async fn cut_off(state: &AppState, ws: WorkspaceId) {
    let store = state.store.as_ref();
    store
        .mark_over_allowance(
            &OverAllowance {
                workspace_id: ws,
                meter: UsageMeter::Deliveries,
                month: UsageMonth::of(Utc::now().date_naive()).label(),
                quantity: 11,
                allowance: 10,
                marked_at: Utc::now(),
            },
            &[],
        )
        .await
        .unwrap();
    maidan_server::usage_meters::refresh_cut_off(state).await;
    assert!(state.usage_meters.is_cut_off(ws, UsageMeter::Deliveries));
}

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
    });
    state.attach_mail(mailer.clone());
    let store = backend.store.as_ref();
    let a = signed_up(store, "ada").await;
    let b = signed_up(store, "bea").await;
    digest_member(store, a, 1).await;
    digest_member(store, b, 2).await;
    cut_off(&state, a).await;
    let _ = state.usage_meters.take();

    assert_eq!(
        digest::sweep_once(&state).await,
        1,
        "only the other tenant's digest is sent"
    );
    assert_eq!(
        *mailer.sent.lock().unwrap(),
        vec!["bea@example.com".to_string()]
    );
    let taken = state.usage_meters.take();
    assert_eq!(counted(a, &taken), 0, "a skipped digest is not counted");
    assert_eq!(
        counted(b, &taken),
        1,
        "each digest sent counts one delivery"
    );

    assert_eq!(
        digest::sweep_once(&state).await,
        0,
        "the cut-off member is still skipped"
    );
    let due = store.members_due_for_digest(10, &[]).await.unwrap();
    assert_eq!(
        due.len(),
        1,
        "the cut-off member stays due: the watermark did not advance"
    );
    assert_eq!(due[0].email, "ada@example.com");
}

/// One more digest-due member in `ws`, beyond its signed-up owner.
async fn held_member(store: &dyn Store, ws: WorkspaceId, handle: &str, log_id: i64) {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .set_member_email(member.id, &format!("{handle}@example.com"))
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

/// A cut-off workspace with more due members than one sweep page, all
/// ordered ahead of the other tenant's member, does not starve it: the held
/// members are left out of the page, so the other tenant gets its digest
/// on the first tick.
async fn starvation_suite(backend: &Backend) {
    let mut state = app_state(backend);
    let mailer = Arc::new(RecordingMailer {
        sent: std::sync::Mutex::new(Vec::new()),
    });
    state.attach_mail(mailer.clone());
    let store = backend.store.as_ref();
    // Member ids are time-ordered, so every member of `a` sorts before `b`'s.
    let a = signed_up(store, "ada").await;
    digest_member(store, a, 1).await;
    for (i, handle) in ["ann", "amy", "abe"].iter().enumerate() {
        held_member(store, a, handle, 10 + i as i64).await;
    }
    let b = signed_up(store, "bea").await;
    digest_member(store, b, 2).await;
    cut_off(&state, a).await;
    let _ = state.usage_meters.take();

    assert_eq!(
        digest::sweep_once_with_batch(&state, 2).await,
        1,
        "the other tenant's digest is sent despite four held members ahead of it"
    );
    assert_eq!(
        *mailer.sent.lock().unwrap(),
        vec!["bea@example.com".to_string()]
    );
    let taken = state.usage_meters.take();
    assert_eq!(counted(a, &taken), 0);
    assert_eq!(counted(b, &taken), 1);
    assert_eq!(
        store.members_due_for_digest(10, &[]).await.unwrap().len(),
        4,
        "the held members all stay due"
    );
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_gets_no_digest_email_while_another_does_on_sqlite(
) {
    suite(&sqlite().await).await;
}

#[tokio::test]
async fn held_members_past_a_page_do_not_starve_another_tenants_digest_on_sqlite() {
    starvation_suite(&sqlite().await).await;
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_gets_no_digest_email_while_another_does_on_postgres(
) {
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
    let backend = Backend {
        store: Arc::new(PostgresStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::PostgresSearch::new(pool.clone())),
        _dir: tempfile::tempdir().unwrap(),
    };
    suite(&backend).await;
    // A second database on the same server: the first suite's tenants
    // would collide with the starvation suite's sign-ups.
    sqlx::query("CREATE DATABASE starvation")
        .execute(&pool)
        .await
        .unwrap();
    let pool2 = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/starvation"
        ))
        .await
        .unwrap();
    run_postgres_migrations(&pool2).await.unwrap();
    starvation_suite(&Backend {
        store: Arc::new(PostgresStore::for_tests(pool2.clone())),
        search: Arc::new(maidan_search::PostgresSearch::new(pool2)),
        _dir: tempfile::tempdir().unwrap(),
    })
    .await;
}
