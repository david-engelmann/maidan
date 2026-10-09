//! SCIM provisioning commits with its record, and a deprovision that cannot
//! finish leaves nothing half-done (D-A, 413.4b), on both backends. So do a
//! rename and every group write.
//!
//! Before, deprovisioning revoked tokens one by one, logged any failure, and
//! reported success: a deprovisioned user could keep a live token while the
//! identity provider believed it gone.

use maidan_auth::hash_secret;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewAuditEvent, NewMember, NewScimGroup, NewWorkspace, ScimGroupChange,
    ScimMembersOp,
};
use sqlx::sqlite::SqlitePoolOptions;

fn event(action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: maidan_types::AuditScope::Instance,
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

async fn live(store: &dyn Store, secret: &str) -> bool {
    store
        .get_active_api_token_by_hash(&hash_secret(secret))
        .await
        .is_ok()
}

async fn run_suite<F, Fut>(store: &dyn Store, break_audit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let user = |handle: &str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Human,
    };
    let provision = |handle: &str| {
        store.scim_provision_audited(
            user(handle),
            Some("ext"),
            true,
            Box::new(|_| event("scim.user.create")),
        )
    };
    let mint = |member_id, secret: &'static str| NewApiToken {
        workspace_id: ws.id,
        member_id,
        app_installation_id: None,
        token_hash: hash_secret(secret),
        label: None,
        capabilities: vec!["workspace:read".into()],
        expires_at: None,
    };

    // Deactivation revokes every live token, one record each.
    let (alice, _) = provision("alice").await.unwrap();
    for secret in ["a1", "a2"] {
        store
            .create_api_token(mint(alice.id, secret))
            .await
            .unwrap();
    }
    store
        .scim_update_user_audited(
            ws.id,
            alice.id,
            None,
            Some("ext"),
            false,
            event("deactivate"),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!live(store, "a1").await && !live(store, "a2").await);
    let revokes = store
        .list_audit(50)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.action == "token.revoke" && row.metadata["reason"] == "scim_deprovision")
        .count();
    assert_eq!(revokes, 2);

    // Set up what the broken-audit phase must leave intact.
    let (bob, _) = provision("bob").await.unwrap();
    let (carol, _) = provision("carol").await.unwrap();
    store.create_api_token(mint(bob.id, "b1")).await.unwrap();
    store.create_api_token(mint(carol.id, "c1")).await.unwrap();
    let group = store
        .scim_create_group_audited(
            NewScimGroup {
                workspace_id: ws.id,
                display_name: "Eng".into(),
                external_id: None,
                members: vec![bob.id],
            },
            Box::new(|_| event("scim.group.create")),
        )
        .await
        .unwrap();

    break_audit().await;

    // Group writes and a rename commit with their record or not at all.
    assert!(store
        .scim_create_group_audited(
            NewScimGroup {
                workspace_id: ws.id,
                display_name: "Ops".into(),
                external_id: None,
                members: vec![carol.id],
            },
            Box::new(|_| event("scim.group.create")),
        )
        .await
        .is_err());
    assert_eq!(store.list_scim_groups(ws.id).await.unwrap().len(), 1);
    assert!(store
        .scim_update_group_audited(
            ws.id,
            group.id,
            ScimGroupChange {
                display_name: Some("Renamed".into()),
                external_id: None,
                members: vec![ScimMembersOp::Replace(vec![carol.id])],
            },
            Box::new(|_| event("scim.group.update")),
        )
        .await
        .is_err());
    let kept = store
        .get_scim_group(ws.id, group.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.display_name, "Eng");
    assert_eq!(kept.members.len(), 1);
    assert_eq!(kept.members[0].member_id, bob.id);
    assert!(store
        .scim_delete_group_audited(ws.id, group.id, event("scim.group.delete"))
        .await
        .is_err());
    assert!(store
        .get_scim_group(ws.id, group.id)
        .await
        .unwrap()
        .is_some());
    assert!(store
        .scim_update_user_audited(ws.id, bob.id, Some("robert"), Some("ext"), true, event("u"))
        .await
        .is_err());
    assert_eq!(store.get_member(bob.id).await.unwrap().handle, "bob");

    assert!(provision("dave").await.is_err());
    assert!(
        store.get_member_by_handle(ws.id, "dave").await.is_err(),
        "no member without its link and record"
    );
    assert!(store
        .scim_update_user_audited(ws.id, bob.id, None, Some("ext"), false, event("deactivate"))
        .await
        .is_err());
    assert!(
        live(store, "b1").await,
        "a failed deactivation revokes nothing"
    );
    assert!(store.get_scim_user(bob.id).await.unwrap().unwrap().active);
    assert!(store
        .scim_deprovision_audited(ws.id, carol.id, event("delete"))
        .await
        .is_err());
    assert!(
        live(store, "c1").await,
        "a failed deprovision revokes nothing"
    );
    assert!(store.get_scim_user(carol.id).await.unwrap().is_some());
}

#[tokio::test]
async fn scim_provisioning_needs_its_record_sqlite() {
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
async fn scim_provisioning_needs_its_record_postgres() {
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

/// Deactivating or deprovisioning a member ends the delegation grants it holds
/// as delegate, which no token revoke reaches (a delegated token belongs to the
/// grant's subject), and deprovisioning also deletes its sessions, since no
/// SCIM link is left behind to say the person is gone. Each with its record.
async fn delegate_suite(store: &dyn Store) {
    use chrono::{Duration, Utc};
    let ws = store
        .create_workspace(NewWorkspace { name: "d".into() })
        .await
        .unwrap();
    let human = |handle: &str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Human,
    };
    let subject = store.create_member(human("subject")).await.unwrap();
    let mut grants = Vec::new();
    let mut delegates = Vec::new();
    for handle in ["deactivated", "deprovisioned"] {
        let (delegate, _) = store
            .scim_provision_audited(
                human(handle),
                None,
                true,
                Box::new(|_| event("scim.user.create")),
            )
            .await
            .unwrap();
        let grant = store
            .create_delegation_grant(maidan_types::NewDelegationGrant {
                workspace_id: ws.id,
                subject_id: subject.id,
                delegate_id: delegate.id,
                capabilities: vec!["workspace:read".into()],
                purpose: "cover".into(),
                authorized_by: subject.id,
                expires_at: Utc::now() + Duration::hours(2),
            })
            .await
            .unwrap();
        grants.push(grant.id);
        delegates.push(delegate.id);
    }
    let session = store
        .create_session(maidan_types::NewMaidanSession {
            workspace_id: ws.id,
            member_id: delegates[1],
            api_token_id: None,
            expires_at: Utc::now() + Duration::hours(1),
        })
        .await
        .unwrap();

    store
        .scim_update_user_audited(ws.id, delegates[0], None, None, false, event("deactivate"))
        .await
        .unwrap();
    assert!(store
        .get_delegation_grant(grants[0])
        .await
        .unwrap()
        .revoked_at
        .is_some());
    assert!(
        store
            .get_delegation_grant(grants[1])
            .await
            .unwrap()
            .revoked_at
            .is_none(),
        "another delegate's grant is untouched"
    );

    assert!(store
        .scim_deprovision_audited(ws.id, delegates[1], event("delete"))
        .await
        .unwrap());
    assert!(store
        .get_delegation_grant(grants[1])
        .await
        .unwrap()
        .revoked_at
        .is_some());
    assert!(store.get_session(session.id).await.is_err());

    let audit = store.list_audit(500).await.unwrap();
    for delegate in &delegates {
        assert!(audit.iter().any(|r| r.action == "delegation_grant.revoke"
            && r.metadata["delegate_id"] == serde_json::json!(delegate.0)));
    }
    assert!(audit
        .iter()
        .any(|r| r.action == "session.delete" && r.target_id == Some(delegates[1].0)));
}

#[tokio::test]
async fn ending_a_member_ends_its_delegate_grants_and_sessions_sqlite() {
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
    delegate_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn ending_a_member_ends_its_delegate_grants_and_sessions_postgres() {
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
    delegate_suite(&PostgresStore::for_tests(pool)).await;
}
