//! Browser sessions on both backends: a session made from a token keeps the
//! token's id and goes when the token row does, and creating one is recorded
//! in its own transaction (D-A) or not at all.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, MaidanSession, MemberKind, NewApiToken, NewAuditEvent, NewMaidanSession, NewMember,
    NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

enum Pool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl Pool {
    async fn execute(&self, sql: &str) {
        match self {
            Pool::Sqlite(p) => {
                sqlx::raw_sql(sql).execute(p).await.unwrap();
            }
            Pool::Postgres(p) => {
                sqlx::raw_sql(sql).execute(p).await.unwrap();
            }
        }
    }

    async fn sessions(&self) -> i64 {
        let sql = "SELECT COUNT(*) FROM maidan_sessions";
        match self {
            Pool::Sqlite(p) => sqlx::query_scalar(sql).fetch_one(p).await.unwrap(),
            Pool::Postgres(p) => sqlx::query_scalar(sql).fetch_one(p).await.unwrap(),
        }
    }
}

fn audit(action: &'static str) -> maidan_store::AuditFor<MaidanSession> {
    Box::new(move |session| NewAuditEvent {
        actor_id: Some(session.member_id),
        scope: AuditScope::Workspace(session.workspace_id),
        action: action.into(),
        target_kind: Some("api_token".into()),
        target_id: session.api_token_id.map(|t| t.0),
        metadata: serde_json::json!({}),
    })
}

async fn run_suite(store: &dyn Store, pool: Pool, break_audit: &str) {
    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "human".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: "h-session".into(),
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let new = |api_token_id| NewMaidanSession {
        workspace_id: ws.id,
        member_id: member.id,
        api_token_id,
        oidc_identity_id: None,
        expires_at: Utc::now() + Duration::hours(1),
    };

    let from_token = store
        .create_session_audited(new(Some(token.id)), audit("session.from_token"))
        .await
        .unwrap();
    let oidc = store.create_session(new(None)).await.unwrap();
    assert_eq!(
        store.get_session(from_token.id).await.unwrap().api_token_id,
        Some(token.id)
    );
    assert_eq!(store.get_session(oidc.id).await.unwrap().api_token_id, None);
    let recorded = store.list_audit(10).await.unwrap();
    assert!(recorded
        .iter()
        .any(|row| row.action == "session.from_token" && row.target_id == Some(token.id.0)));

    // The token's row going (a workspace erase, a member delete) takes its
    // sessions with it; the OIDC session is not the token's.
    pool.execute("DELETE FROM maidan_api_tokens WHERE token_hash = 'h-session'")
        .await;
    assert!(store.get_session(from_token.id).await.is_err());
    assert!(store.get_session(oidc.id).await.is_ok());

    pool.execute(break_audit).await;
    let before = pool.sessions().await;
    assert!(
        store
            .create_session_audited(new(None), audit("session.oidc_login"))
            .await
            .is_err(),
        "a session that cannot be recorded must not be made"
    );
    assert_eq!(pool.sessions().await, before, "and none is left behind");
}

#[tokio::test]
async fn a_tokens_session_keeps_its_id_and_is_recorded_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool.clone());
    run_suite(
        &store,
        Pool::Sqlite(pool),
        "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
         BEGIN SELECT RAISE(ABORT, 'audit down'); END",
    )
    .await;
}

#[tokio::test]
async fn a_tokens_session_keeps_its_id_and_is_recorded_postgres() {
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
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    run_suite(
        &store,
        Pool::Postgres(pool),
        "CREATE FUNCTION audit_down() RETURNS trigger AS $$
         BEGIN RAISE EXCEPTION 'audit down'; END $$ LANGUAGE plpgsql;
         CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
         FOR EACH ROW EXECUTE FUNCTION audit_down();",
    )
    .await;
}
