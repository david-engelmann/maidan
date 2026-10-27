//! A display-name change commits with its event and its audit row, scoped to
//! the member's own workspace, on both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, EventKind, MemberKind, NewAuditEvent, NewMember, NewWorkspace, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

fn audit(scope: WorkspaceId, member: maidan_types::MemberId) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(scope),
        actor_id: Some(member),
        action: "member.display_name.set".into(),
        target_kind: Some("member".into()),
        target_id: Some(member.0),
        metadata: serde_json::json!({}),
    }
}

async fn renames(store: &dyn Store, ws: WorkspaceId) -> (usize, usize) {
    let events = store
        .list_events_after(ws, 0, 500)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == EventKind::MemberDisplayNameChanged)
        .count();
    let audits = store
        .list_audit_for_workspace(ws, 500)
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "member.display_name.set")
        .count();
    (events, audits)
}

async fn run_suite(store: &dyn Store) {
    let seed = |name: &'static str| async move {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "me".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        (ws.id, member.id)
    };
    let (home, me) = seed("home").await;
    let (away, _) = seed("away").await;

    let (member, stored) = store
        .set_member_display_name(me, Some("Ada Lovelace".into()), audit(home, me))
        .await
        .unwrap();
    assert_eq!(member.display_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(stored.kind, EventKind::MemberDisplayNameChanged);
    assert_eq!(stored.workspace_id, Some(home));
    assert_eq!(renames(store, home).await, (1, 1));
    assert_eq!(renames(store, away).await, (0, 0));

    // An audit row aimed at another workspace is refused, and the rename
    // rolls back with it.
    let refused = store
        .set_member_display_name(me, Some("Mallory".into()), audit(away, me))
        .await;
    assert!(refused.is_err(), "a cross-workspace audit row was accepted");
    let kept = store.get_member(me).await.unwrap();
    assert_eq!(kept.display_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(renames(store, home).await, (1, 1));
    assert_eq!(renames(store, away).await, (0, 0));

    let (cleared, _) = store
        .set_member_display_name(me, None, audit(home, me))
        .await
        .unwrap();
    assert_eq!(cleared.display_name, None);
    assert_eq!(renames(store, home).await, (2, 2));
}

#[tokio::test]
async fn a_rename_commits_with_its_event_and_audit_in_its_own_workspace_sqlite() {
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
async fn a_rename_commits_with_its_event_and_audit_in_its_own_workspace_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
