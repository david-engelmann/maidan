//! Workspace suspensions on both backends: suspend, read, re-suspend, lift,
//! each audited in its own transaction, and held to the workspace named.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

fn audit(ws: WorkspaceId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: Some("workspace".into()),
        target_id: Some(ws.0),
        metadata: serde_json::json!({}),
    }
}

async fn run_suite(store: &dyn Store) {
    let a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let actions = |ws: WorkspaceId| async move {
        store
            .list_audit_for_workspace(ws, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.action)
            .collect::<Vec<_>>()
    };

    let missing = WorkspaceId(uuid::Uuid::now_v7());
    assert!(matches!(
        store
            .suspend_workspace(missing, "x", None, audit(missing, "workspace.suspend"))
            .await,
        Err(StoreError::NotFound)
    ));

    let first = store
        .suspend_workspace(a, "spam", None, audit(a, "workspace.suspend"))
        .await
        .unwrap();
    assert_eq!((first.workspace_id, first.reason.as_str()), (a, "spam"));
    let again = store
        .suspend_workspace(a, "spam and abuse", None, audit(a, "workspace.suspend"))
        .await
        .unwrap();
    assert_eq!(again.reason, "spam and abuse");
    assert_eq!(
        again.suspended_at, first.suspended_at,
        "the first suspension stands"
    );
    assert_eq!(
        store
            .get_workspace_suspension(a)
            .await
            .unwrap()
            .map(|s| s.reason),
        Some("spam and abuse".to_string())
    );
    assert_eq!(
        store.get_workspace_suspension(b).await.unwrap(),
        None,
        "B untouched"
    );

    assert!(store
        .lift_workspace_suspension(a, audit(a, "workspace.suspension_lift"))
        .await
        .unwrap());
    assert!(!store
        .lift_workspace_suspension(a, audit(a, "workspace.suspension_lift"))
        .await
        .unwrap());
    assert_eq!(store.get_workspace_suspension(a).await.unwrap(), None);
    let a_actions = actions(a).await;
    assert_eq!(
        a_actions
            .iter()
            .filter(|x| *x == "workspace.suspend")
            .count(),
        2
    );
    assert_eq!(
        a_actions
            .iter()
            .filter(|x| *x == "workspace.suspension_lift")
            .count(),
        1,
        "a lift that lifted nothing is not audited"
    );
    assert!(actions(b).await.is_empty());
}

#[tokio::test]
async fn a_workspace_is_suspended_and_lifted_with_an_audit_row_each_sqlite() {
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
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn a_workspace_is_suspended_and_lifted_with_an_audit_row_each_postgres() {
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}
