//! An authority change does not happen without its record (D-A), on both
//! backends.
//!
//! A trigger makes every audit insert fail. Each audited change must then fail
//! and leave the state as it was: no token, grant or ticket created, none
//! revoked, the grant ceiling unchanged. The audit row is inside the change's
//! transaction, so they commit or roll back together.

use chrono::{Duration, Utc};
use maidan_auth::hash_secret;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewAuditEvent, NewChannel, NewDelegationGrant, NewMember,
    NewShareTicket, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

fn event(action: &str) -> NewAuditEvent {
    NewAuditEvent {
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

fn audit_for<T>(action: &'static str) -> maidan_store::AuditFor<T> {
    Box::new(move |_| event(action))
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

    let delegate = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "d".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let grant = |purpose: &str| NewDelegationGrant {
        workspace_id: ws.id,
        subject_id: member.id,
        delegate_id: delegate.id,
        capabilities: vec!["workspace:read".into()],
        purpose: purpose.into(),
        authorized_by: member.id,
        expires_at: Utc::now() + Duration::days(1),
    };
    let ticket = |hash: &str| NewShareTicket {
        workspace_id: ws.id,
        channel_id: channel.id,
        owner_id: member.id,
        created_by: member.id,
        token_hash: hash_secret(hash),
        expires_at: Utc::now() + Duration::hours(1),
        artifact_shas: Vec::new(),
    };

    // While audit works, the audited forms record their change.
    let live = store
        .create_api_token_audited(token("live"), audit_for("token.mint"))
        .await
        .unwrap();
    let live_grant = store
        .create_delegation_grant_audited(grant("live"), audit_for("grant.create"))
        .await
        .unwrap();
    let live_ticket = store
        .create_share_ticket_audited(ticket("live"), audit_for("ticket.create"))
        .await
        .unwrap();
    store
        .set_delegation_policy_audited(ws.id, Some(30), audit_for("policy.set"))
        .await
        .unwrap();
    let recorded: Vec<String> = store
        .list_audit(10)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.action)
        .collect();
    for action in ["token.mint", "grant.create", "ticket.create", "policy.set"] {
        assert!(recorded.iter().any(|a| a == action), "{action} unrecorded");
    }
    // A revoke that finds no live ticket changes nothing and records nothing.
    assert!(!store
        .revoke_share_ticket_audited(
            ws.id,
            maidan_types::ShareTicketId::new(),
            event("ticket.revoke.missing"),
        )
        .await
        .unwrap());
    assert!(!store
        .list_audit(10)
        .await
        .unwrap()
        .iter()
        .any(|row| row.action == "ticket.revoke.missing"));

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

    assert!(store
        .create_delegation_grant_audited(grant("unrecorded"), audit_for("grant.create"))
        .await
        .is_err());
    assert_eq!(
        store.list_delegation_grants(ws.id).await.unwrap().len(),
        1,
        "an unrecorded grant must not exist"
    );
    assert!(store
        .revoke_delegation_grant_audited(ws.id, live_grant.id, event("grant.revoke"))
        .await
        .is_err());
    assert!(
        store
            .get_delegation_grant(live_grant.id)
            .await
            .unwrap()
            .revoked_at
            .is_none(),
        "an unrecorded grant revoke must leave the grant live"
    );

    assert!(store
        .create_share_ticket_audited(ticket("unrecorded"), audit_for("ticket.create"))
        .await
        .is_err());
    assert_eq!(
        store.list_share_tickets(ws.id).await.unwrap().len(),
        1,
        "an unrecorded ticket must not exist"
    );
    assert!(store
        .revoke_share_ticket_audited(ws.id, live_ticket.id, event("ticket.revoke"))
        .await
        .is_err());
    assert!(
        store
            .get_share_ticket(live_ticket.id)
            .await
            .unwrap()
            .revoked_at
            .is_none(),
        "an unrecorded ticket revoke must leave the ticket live"
    );

    assert!(store
        .set_delegation_policy_audited(ws.id, Some(7), audit_for("policy.set"))
        .await
        .is_err());
    assert_eq!(
        store
            .get_delegation_policy(ws.id)
            .await
            .unwrap()
            .max_grant_days,
        30,
        "an unrecorded ceiling change must not take effect"
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
