//! An authority change does not happen without its record (D-A), on both
//! backends.
//!
//! A trigger makes every audit insert fail. An audited mint must then fail and
//! leave no token behind, and an audited revoke must fail and leave the token
//! live. The audit row is inside the change's transaction, so they commit or
//! roll back together.

use maidan_auth::hash_secret;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{ApiToken, MemberKind, NewApiToken, NewAuditEvent, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

fn audit_for(action: &'static str) -> maidan_store::AuditFor<ApiToken> {
    Box::new(move |token| NewAuditEvent {
        actor_id: None,
        action: action.into(),
        target_kind: Some("api_token".into()),
        target_id: Some(token.id.0),
        metadata: serde_json::json!({}),
    })
}

/// `break_audit` makes every later audit insert fail.
async fn run_suite<F, Fut>(store: &dyn Store, break_audit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let token = |hash: &str| NewApiToken {
        workspace_id: ws.id,
        member_id: member.id,
        app_installation_id: None,
        token_hash: hash_secret(hash),
        label: None,
        capabilities: vec!["workspace:read".into()],
        expires_at: None,
    };

    // While audit works, the audited forms record their change.
    let live = store
        .create_api_token_audited(token("live"), audit_for("token.mint"))
        .await
        .unwrap();
    assert!(store
        .list_audit(10)
        .await
        .unwrap()
        .iter()
        .any(|row| row.action == "token.mint" && row.target_id == Some(live.id.0)));

    break_audit().await;

    let refused = store
        .create_api_token_audited(token("unrecorded"), audit_for("token.mint"))
        .await;
    assert!(refused.is_err(), "a mint that cannot be recorded must fail");
    assert!(
        store
            .get_active_api_token_by_hash(&hash_secret("unrecorded"))
            .await
            .is_err(),
        "and must leave no token behind"
    );

    let refused = store
        .revoke_api_token_audited(live.id, audit_for("token.revoke"))
        .await;
    assert!(
        refused.is_err(),
        "a revoke that cannot be recorded must fail"
    );
    assert!(
        store
            .get_active_api_token_by_hash(&hash_secret("live"))
            .await
            .is_ok(),
        "and must leave the token live"
    );
}

#[tokio::test]
async fn an_authority_change_needs_its_record_sqlite() {
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
    let store = SqliteStore::new(pool.clone());
    run_suite(&store, || async {
        sqlx::query(
            "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             BEGIN SELECT RAISE(ABORT, 'audit down'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn an_authority_change_needs_its_record_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool.clone());
    run_suite(&store, || async {
        sqlx::query(
            "CREATE FUNCTION audit_down() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'audit down'; END $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             FOR EACH ROW EXECUTE FUNCTION audit_down()",
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}
