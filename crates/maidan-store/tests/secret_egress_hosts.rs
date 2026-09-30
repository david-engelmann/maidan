//! The hosts a workspace trusts with its secret values: allow (idempotent,
//! host-validated), list, revoke (workspace-scoped) and the check the egress
//! broker makes. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{AuditScope, NewAuditEvent, NewSecretEgressHost, NewWorkspace, WorkspaceId};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::for_tests(pool)
}

fn host(ws: WorkspaceId, host: &str) -> NewSecretEgressHost {
    NewSecretEgressHost {
        workspace_id: ws,
        host: host.into(),
    }
}

fn event(ws: WorkspaceId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: Some("secret_egress_host".into()),
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .expect("ws");
    let other = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .expect("other ws");

    assert!(store
        .list_secret_egress_hosts(ws.id)
        .await
        .unwrap()
        .is_empty());
    assert!(!store
        .is_secret_egress_host_allowed(ws.id, "hooks.example.com")
        .await
        .unwrap());

    let first = store
        .allow_secret_egress_host(host(ws.id, "Hooks.Example.com"))
        .await
        .expect("allow");
    assert_eq!(first.host, "hooks.example.com", "stored lowercase");
    let again = store
        .allow_secret_egress_host(host(ws.id, "hooks.example.com"))
        .await
        .expect("allow again");
    assert_eq!(again, first, "a second allow keeps the first entry");
    assert_eq!(
        store.list_secret_egress_hosts(ws.id).await.unwrap().len(),
        1
    );

    assert!(store
        .is_secret_egress_host_allowed(ws.id, "HOOKS.example.com")
        .await
        .unwrap());
    assert!(
        !store
            .is_secret_egress_host_allowed(other.id, "hooks.example.com")
            .await
            .unwrap(),
        "one workspace's trusted host is not another's"
    );
    assert!(!store
        .is_secret_egress_host_allowed(ws.id, "evil.hooks.example.com")
        .await
        .unwrap());

    for bad in [
        "https://hooks.example.com",
        "hooks.example.com:443",
        "*.example.com",
    ] {
        assert!(
            matches!(
                store.allow_secret_egress_host(host(ws.id, bad)).await,
                Err(StoreError::InvalidInput(_))
            ),
            "{bad} accepted"
        );
    }

    assert!(
        !store
            .revoke_secret_egress_host_audited(
                other.id,
                "hooks.example.com",
                event(other.id, "revoke")
            )
            .await
            .unwrap(),
        "another workspace cannot revoke this workspace's host"
    );
    assert!(store
        .is_secret_egress_host_allowed(ws.id, "hooks.example.com")
        .await
        .unwrap());
    let before = store.list_audit(100).await.unwrap().len();
    assert!(store
        .revoke_secret_egress_host_audited(ws.id, "hooks.example.com", event(ws.id, "revoke"))
        .await
        .unwrap());
    assert_eq!(
        store.list_audit(100).await.unwrap().len(),
        before + 1,
        "a revoke that removed a host is recorded"
    );
    assert!(!store
        .is_secret_egress_host_allowed(ws.id, "hooks.example.com")
        .await
        .unwrap());
    assert!(!store
        .revoke_secret_egress_host(ws.id, "hooks.example.com")
        .await
        .unwrap());
}

#[tokio::test]
async fn secret_egress_hosts_allow_list_revoke_and_check_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn secret_egress_hosts_allow_list_revoke_and_check_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
