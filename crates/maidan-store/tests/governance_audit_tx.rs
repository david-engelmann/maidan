//! Governance and membership changes commit with their record (D-A, 413.4), on
//! both backends.
//!
//! - With audit inserts broken, no freeze, unfreeze, requirement change,
//!   reviewer removal, land-gate clear, governance grant, egress change or app
//!   revoke happens.
//! - A call that removes nothing records nothing.
//! - A review requirement is not lowered by a caller that may not lower it,
//!   decided inside the write.
//! - An app installation and its tokens are revoked together.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    AppInstallation, ChannelMemberRole, EgressSurface, MemberKind, NewApiToken, NewApp,
    NewAppInstallation, NewAuditEvent, NewChannel, NewEgressTarget, NewMember, NewSecret,
    NewThread, NewWorkspace, REVIEW_SKILL,
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

async fn recorded(store: &dyn Store, action: &str) -> bool {
    store
        .list_audit(200)
        .await
        .unwrap()
        .iter()
        .any(|row| row.action == action)
}

async fn run_suite<F, Fut>(store: &dyn Store, break_audit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let ws = store
        .create_workspace(NewWorkspace { name: "g".into() })
        .await
        .unwrap();
    let member = |handle: &'static str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    let admin = store.create_member(member("admin")).await.unwrap();
    let worker = store.create_member(member("worker")).await.unwrap();
    let bot = store.create_member(member("bot")).await.unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let thread = |title: &'static str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    let t = store.create_thread(thread("t")).await.unwrap().id;
    let app = store
        .create_app(NewApp {
            workspace_id: ws.id,
            slug: "bot".into(),
            name: "Bot".into(),
            description: None,
            created_by: admin.id,
        })
        .await
        .unwrap();
    let secret = |name: &str| NewSecret {
        workspace_id: ws.id,
        name: name.into(),
        value_ciphertext: "ciphertext".into(),
        created_by: admin.id,
    };
    let install = || NewAppInstallation {
        app_id: app.id,
        workspace_id: ws.id,
        bot_member_id: bot.id,
        granted_capabilities: vec!["workspace:read".into()],
    };

    // Misses record nothing.
    assert!(!store
        .unfreeze_member_audited(worker.id, event("miss.unfreeze"))
        .await
        .unwrap());
    assert!(!store
        .clear_land_gate_audited(t, event("miss.land"))
        .await
        .unwrap());
    assert!(!store
        .remove_reviewer_audited(t, worker.id, event("miss.reviewer"))
        .await
        .unwrap());
    for action in ["miss.unfreeze", "miss.land", "miss.reviewer"] {
        assert!(!recorded(store, action).await, "{action} recorded a miss");
    }

    // The requirement is not lowered by a caller that may not lower it.
    store
        .set_review_requirement_audited(t, 2, false, audit_for("req.raise"))
        .await
        .unwrap();
    let refused = store
        .set_review_requirement_audited(t, 1, false, audit_for("req.lower"))
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "{refused:?}"
    );
    assert_eq!(
        store
            .get_review_requirement(t)
            .await
            .unwrap()
            .unwrap()
            .required_count,
        2
    );
    let (from, lowered) = store
        .set_review_requirement_audited(t, 1, true, audit_for("req.lower"))
        .await
        .unwrap();
    assert_eq!((from, lowered.required_count), (2, 1));

    // Revoking an installation revokes its tokens in the same transaction.
    let installed = store.create_app_installation(install()).await.unwrap();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: bot.id,
            app_installation_id: Some(installed.id),
            token_hash: maidan_auth::hash_secret("bot-token"),
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let revoked: AppInstallation = store
        .revoke_app_installation_audited(installed.id, audit_for("app.revoke"))
        .await
        .unwrap();
    assert!(revoked.revoked_at.is_some());
    assert!(store
        .get_active_api_token_by_hash(&maidan_auth::hash_secret("bot-token"))
        .await
        .is_err());

    // Arm everything the broken-audit phase will try to undo.
    store
        .freeze_member_audited(worker.id, admin.id, None, audit_for("freeze"))
        .await
        .unwrap();
    store.add_reviewer(t, worker.id).await.unwrap();
    store.require_land_gate(t).await.unwrap();
    let target = store
        .allow_egress_target_audited(
            NewEgressTarget {
                workspace_id: ws.id,
                surface: EgressSurface::Slack,
                selector: "C0123ABCDEF".into(),
            },
            audit_for("egress.allow"),
        )
        .await
        .unwrap();
    let live_install = store.create_app_installation(install()).await.unwrap();
    store
        .create_secret_audited(secret("kept"), audit_for("secret.create"))
        .await
        .unwrap();
    for action in [
        "req.raise",
        "req.lower",
        "app.revoke",
        "freeze",
        "egress.allow",
        "secret.create",
    ] {
        assert!(recorded(store, action).await, "{action} unrecorded");
    }

    break_audit().await;

    let other = store.create_thread(thread("other")).await.unwrap().id;
    let fails = |r: Result<(), StoreError>, what: &str| assert!(r.is_err(), "{what} happened");
    fails(
        store
            .freeze_member_audited(admin.id, admin.id, None, audit_for("x"))
            .await
            .map(drop),
        "freeze",
    );
    assert!(!store.is_member_frozen(admin.id).await.unwrap());
    fails(
        store
            .unfreeze_member_audited(worker.id, event("x"))
            .await
            .map(drop),
        "unfreeze",
    );
    assert!(store.is_member_frozen(worker.id).await.unwrap());
    fails(
        store
            .add_channel_member_audited(
                channel.id,
                worker.id,
                ChannelMemberRole::Member,
                audit_for("x"),
            )
            .await
            .map(drop),
        "channel add",
    );
    assert!(!store
        .list_channel_members(channel.id)
        .await
        .unwrap()
        .iter()
        .any(|m| m.member_id == worker.id));
    fails(
        store
            .set_review_requirement_audited(other, 3, true, audit_for("x"))
            .await
            .map(drop),
        "requirement set",
    );
    assert!(store.get_review_requirement(other).await.unwrap().is_none());
    fails(
        store
            .clear_review_requirement_audited(t, event("x"))
            .await
            .map(drop),
        "requirement clear",
    );
    assert!(store.get_review_requirement(t).await.unwrap().is_some());
    fails(
        store
            .remove_reviewer_audited(t, worker.id, event("x"))
            .await
            .map(drop),
        "reviewer removal",
    );
    assert_eq!(store.list_reviewers(t).await.unwrap(), vec![worker.id]);
    fails(
        store.clear_land_gate_audited(t, event("x")).await.map(drop),
        "land-gate clear",
    );
    assert!(store.get_land_gate_standing(t).await.unwrap().required);
    fails(
        store
            .grant_governance_skill_audited(worker.id, REVIEW_SKILL, event("x"))
            .await,
        "governance grant",
    );
    assert!(store
        .list_member_skills(worker.id)
        .await
        .unwrap()
        .is_empty());
    fails(
        store
            .revoke_egress_target_audited(ws.id, target.id, event("x"))
            .await
            .map(drop),
        "egress revoke",
    );
    assert_eq!(store.list_egress_targets(ws.id).await.unwrap().len(), 1);
    fails(
        store
            .revoke_app_installation_audited(live_install.id, audit_for("x"))
            .await
            .map(drop),
        "app revoke",
    );
    assert!(store
        .get_app_installation(live_install.id)
        .await
        .unwrap()
        .revoked_at
        .is_none());
    fails(
        store
            .create_secret_audited(secret("unrecorded"), audit_for("x"))
            .await
            .map(drop),
        "secret create",
    );
    fails(
        store
            .delete_secret_audited(ws.id, "kept", event("x"))
            .await
            .map(drop),
        "secret delete",
    );
    let names: Vec<String> = store
        .list_secrets(ws.id)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, vec!["kept".to_string()]);
}

#[tokio::test]
async fn governance_changes_need_their_record_sqlite() {
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
async fn governance_changes_need_their_record_postgres() {
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
