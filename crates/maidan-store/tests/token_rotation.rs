//! `rotate_api_token_audited` on both backends: the successor keeps the
//! authority and the quotas, derived tokens move under it, the old secret is
//! revoked, and the audit row is written with it.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewAuditEvent, NewMember, NewWorkspace, TokenQuota};
use sqlx::sqlite::SqlitePoolOptions;

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "rot".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let new = |hash: &str, caps: &[&str]| NewApiToken {
        workspace_id: ws.id,
        member_id: member.id,
        app_installation_id: None,
        token_hash: hash.into(),
        label: Some("worker".into()),
        capabilities: caps.iter().map(|c| c.to_string()).collect(),
        expires_at: None,
    };
    let old = store
        .create_api_token(new("h-old", &["workspace:read", "message:post"]))
        .await
        .expect("token");
    let quota = TokenQuota {
        capability: "message:post".into(),
        max_per_window: 3,
        window_secs: 60,
    };
    store
        .replace_token_quotas(old.id, std::slice::from_ref(&quota))
        .await
        .expect("quota");
    store
        .create_attenuated_api_token(new("h-child", &["workspace:read"]), old.id)
        .await
        .expect("child");

    let audit = |t: &maidan_types::ApiToken| NewAuditEvent {
        actor_id: None,
        action: "token.rotate".into(),
        target_kind: Some("api_token".into()),
        target_id: Some(t.id.0),
        metadata: serde_json::json!({}),
    };
    let successor = store
        .rotate_api_token_audited(old.id, "h-new", Box::new(audit))
        .await
        .expect("rotate");
    assert_ne!(successor.id, old.id);
    assert_eq!(successor.member_id, member.id);
    assert_eq!(successor.capabilities, old.capabilities);
    assert_eq!(successor.label.as_deref(), Some("worker"));
    assert_eq!(
        store.list_token_quotas(successor.id).await.unwrap(),
        vec![quota]
    );
    assert!(store.get_active_api_token_by_hash("h-old").await.is_err());
    assert_eq!(
        store
            .get_active_api_token_by_hash("h-new")
            .await
            .unwrap()
            .id,
        successor.id
    );
    assert!(
        store.get_active_api_token_by_hash("h-child").await.is_ok(),
        "the derived token survives the rotation"
    );
    assert!(
        matches!(
            store
                .rotate_api_token_audited(old.id, "h-again", Box::new(audit))
                .await,
            Err(StoreError::NotFound)
        ),
        "a revoked token cannot be rotated"
    );
    assert_eq!(
        store
            .list_audit(10)
            .await
            .unwrap()
            .iter()
            .filter(|a| a.action == "token.rotate")
            .count(),
        1
    );

    store.revoke_api_token(successor.id).await.expect("revoke");
    assert!(
        store.get_active_api_token_by_hash("h-child").await.is_err(),
        "the derived token now hangs off the successor"
    );
}

#[tokio::test]
async fn rotation_keeps_authority_quotas_and_derived_tokens_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn rotation_keeps_authority_quotas_and_derived_tokens_postgres() {
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
