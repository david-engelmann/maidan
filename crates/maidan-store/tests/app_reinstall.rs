//! Re-installing an app whose installation was revoked reuses its bot member,
//! on both backends.
//!
//! - The first install creates the `app:<slug>` member; a re-install after a
//!   revoke keeps that member's id and handle and takes the new grants.
//! - An install while one is active is refused and changes nothing.
//! - A re-install in one workspace never picks up another workspace's member
//!   for an app with the same slug.
//! - A hand-made member holding the handle is not taken over.
//! - Each install writes its audit row in its own transaction: with audit
//!   inserts broken, nothing is installed and no member is created.

use maidan_store::{prelude::*, run_sqlite_migrations, AuditFor, InstalledApp, StoreError};
use maidan_types::{
    AppId, MemberKind, NewApp, NewAuditEvent, NewMember, NewWorkspace, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

fn audit(action: &'static str) -> AuditFor<InstalledApp> {
    Box::new(move |installed| NewAuditEvent {
        scope: maidan_types::AuditScope::Workspace(installed.installation.workspace_id),
        actor_id: None,
        action: action.into(),
        target_kind: Some("app_installation".into()),
        target_id: Some(installed.installation.id.0),
        metadata: serde_json::json!({ "bot_member_reused": installed.bot_member_reused }),
    })
}

fn caps(list: &[&str]) -> Vec<String> {
    list.iter().map(|c| c.to_string()).collect()
}

async fn workspace_with_app(store: &dyn Store, name: &str, slug: &str) -> (WorkspaceId, AppId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let app = store
        .create_app(NewApp {
            workspace_id: ws.id,
            slug: slug.into(),
            name: "Bot".into(),
            description: None,
            created_by: owner.id,
        })
        .await
        .unwrap();
    (ws.id, app.id)
}

async fn audit_rows(store: &dyn Store, action: &str) -> usize {
    store
        .list_audit(500)
        .await
        .unwrap()
        .iter()
        .filter(|row| row.action == action)
        .count()
}

async fn run_suite<F, Fut>(store: &dyn Store, break_audit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (ws_a, app_a) = workspace_with_app(store, "a", "bot").await;
    let (ws_b, app_b) = workspace_with_app(store, "b", "bot").await;

    // First install creates the bot member.
    let first = store
        .install_app_audited(ws_a, app_a, caps(&["workspace:read"]), audit("a.install"))
        .await
        .unwrap();
    assert!(!first.bot_member_reused);
    let bot = store
        .get_member(first.installation.bot_member_id)
        .await
        .unwrap();
    assert_eq!(bot.handle, "app:bot");
    assert_eq!(bot.workspace_id, ws_a);
    assert_eq!(bot.kind, MemberKind::Agent);
    assert_eq!(audit_rows(store, "a.install").await, 1);

    // While it is active, a second install is refused and records nothing.
    let refused = store
        .install_app_audited(ws_a, app_a, caps(&["message:post"]), audit("a.again"))
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "{refused:?}"
    );
    assert_eq!(audit_rows(store, "a.again").await, 0);
    assert_eq!(store.list_app_installations(ws_a).await.unwrap().len(), 1);

    // Revoke, then re-install with other grants: same member, new grants.
    store
        .revoke_app_installation(first.installation.id)
        .await
        .unwrap();
    let second = store
        .install_app_audited(
            ws_a,
            app_a,
            caps(&["message:post", "workspace:read"]),
            audit("a.reinstall"),
        )
        .await
        .unwrap();
    assert!(second.bot_member_reused);
    assert_ne!(second.installation.id, first.installation.id);
    assert_eq!(second.installation.bot_member_id, bot.id);
    assert_eq!(
        second.installation.granted_capabilities,
        caps(&["message:post", "workspace:read"])
    );
    assert!(second.installation.revoked_at.is_none());
    assert!(store
        .get_app_installation(first.installation.id)
        .await
        .unwrap()
        .revoked_at
        .is_some());
    assert_eq!(audit_rows(store, "a.reinstall").await, 1);

    // Workspace B's app has the same slug; its install is its own member.
    let in_b = store
        .install_app_audited(ws_b, app_b, caps(&["workspace:read"]), audit("b.install"))
        .await
        .unwrap();
    assert!(!in_b.bot_member_reused);
    assert_ne!(in_b.installation.bot_member_id, bot.id);
    store
        .revoke_app_installation(in_b.installation.id)
        .await
        .unwrap();
    let b_again = store
        .install_app_audited(ws_b, app_b, caps(&["workspace:read"]), audit("b.reinstall"))
        .await
        .unwrap();
    assert!(b_again.bot_member_reused);
    assert_eq!(
        b_again.installation.bot_member_id,
        in_b.installation.bot_member_id
    );
    let b_bot = store
        .get_member(b_again.installation.bot_member_id)
        .await
        .unwrap();
    assert_eq!(b_bot.workspace_id, ws_b);

    // An app is installed only in its own workspace.
    let crossed = store
        .install_app_audited(ws_b, app_a, caps(&["workspace:read"]), audit("crossed"))
        .await;
    assert!(matches!(crossed, Err(StoreError::NotFound)), "{crossed:?}");

    // A member someone else made with the handle is not taken over.
    let (ws_c, app_c) = workspace_with_app(store, "c", "squat").await;
    store
        .create_member(NewMember {
            workspace_id: ws_c,
            handle: "app:squat".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let squatted = store
        .install_app_audited(ws_c, app_c, caps(&["workspace:read"]), audit("c.install"))
        .await;
    assert!(
        matches!(squatted, Err(StoreError::Conflict(_))),
        "{squatted:?}"
    );

    // With audit inserts broken, neither a re-install nor a first install
    // happens.
    store
        .revoke_app_installation(second.installation.id)
        .await
        .unwrap();
    let (ws_d, app_d) = workspace_with_app(store, "d", "fresh").await;
    let members_in_d = store.list_members(ws_d).await.unwrap().len();
    break_audit().await;
    assert!(store
        .install_app_audited(ws_a, app_a, caps(&["workspace:read"]), audit("x"))
        .await
        .is_err());
    assert!(store
        .list_app_installations(ws_a)
        .await
        .unwrap()
        .iter()
        .all(|row| row.revoked_at.is_some()));
    assert!(store
        .install_app_audited(ws_d, app_d, caps(&["workspace:read"]), audit("x"))
        .await
        .is_err());
    assert!(store.list_app_installations(ws_d).await.unwrap().is_empty());
    assert_eq!(store.list_members(ws_d).await.unwrap().len(), members_in_d);
}

#[tokio::test]
async fn a_revoked_app_reinstalls_onto_its_bot_member_sqlite() {
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
async fn a_revoked_app_reinstalls_onto_its_bot_member_postgres() {
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
    let store = PostgresStore::for_tests(pool.clone());
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
