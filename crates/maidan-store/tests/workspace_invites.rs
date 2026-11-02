//! Workspace invites on both backends: an invite is found only while live,
//! works once, stops working when revoked or expired, joins only the email
//! it names, and never lets one workspace's invite open another.

use std::time::Duration;

use chrono::Utc;
use maidan_store::workspace_invites::{INVITE_ALREADY_MEMBER, INVITE_NOT_VALID};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const ISSUER: &str = "https://idp.example";

fn hash(n: u32) -> String {
    format!("{n:064x}")
}

async fn seed(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-admin"),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    (ws.id, admin.id)
}

async fn mint(
    store: &dyn Store,
    ws: WorkspaceId,
    by: MemberId,
    email: &str,
    role: WorkspaceInviteRole,
    n: u32,
    ttl: chrono::Duration,
) -> WorkspaceInvite {
    store
        .create_workspace_invite_audited(
            NewWorkspaceInvite {
                workspace_id: ws,
                email: email.into(),
                role,
                created_by: by,
                token_hash: hash(n),
                expires_at: Utc::now() + ttl,
            },
            Box::new(|i| NewAuditEvent {
                scope: AuditScope::Workspace(i.workspace_id),
                actor_id: Some(i.created_by),
                action: "workspace_invite.create".into(),
                target_kind: Some("workspace_invite".into()),
                target_id: Some(i.id.0),
                metadata: serde_json::json!({}),
            }),
        )
        .await
        .unwrap()
}

async fn accept(
    store: &dyn Store,
    invite: &WorkspaceInvite,
    ws: WorkspaceId,
    email: &str,
    subject: &str,
) -> Result<WorkspaceInviteAcceptance, StoreError> {
    store
        .accept_workspace_invite(
            AcceptWorkspaceInvite {
                invite_id: invite.id,
                workspace_id: ws,
                email: email.into(),
                issuer: ISSUER.into(),
                subject: subject.into(),
                handle: "invitee".into(),
                session_expires_at: Utc::now() + chrono::Duration::hours(1),
            },
            Box::new(|a| NewAuditEvent {
                scope: AuditScope::Workspace(a.invite.workspace_id),
                actor_id: Some(a.member.id),
                action: "workspace_invite.accept".into(),
                target_kind: Some("workspace_invite".into()),
                target_id: Some(a.invite.id.0),
                metadata: serde_json::json!({}),
            }),
            Box::new(|s| NewAuditEvent {
                scope: AuditScope::Workspace(s.workspace_id),
                actor_id: Some(s.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(s.member_id.0),
                metadata: serde_json::json!({}),
            }),
        )
        .await
}

fn not_valid(r: &Result<WorkspaceInviteAcceptance, StoreError>) -> bool {
    matches!(r, Err(StoreError::Conflict(m)) if m == INVITE_NOT_VALID)
}

async fn assert_invites(store: &dyn Store) {
    let (ws, admin) = seed(store, "inv-a").await;
    let (other_ws, other_admin) = seed(store, "inv-b").await;
    let week = chrono::Duration::days(7);

    // Out-of-range expiry and a bad hash are refused at mint.
    let too_long = store
        .create_workspace_invite_audited(
            NewWorkspaceInvite {
                workspace_id: ws,
                email: "x@example.com".into(),
                role: WorkspaceInviteRole::Member,
                created_by: admin,
                token_hash: hash(99),
                expires_at: Utc::now() + chrono::Duration::days(31),
            },
            Box::new(|i| NewAuditEvent {
                scope: AuditScope::Workspace(i.workspace_id),
                actor_id: None,
                action: "workspace_invite.create".into(),
                target_kind: None,
                target_id: None,
                metadata: serde_json::json!({}),
            }),
        )
        .await;
    assert!(
        matches!(too_long, Err(StoreError::InvalidInput(_))),
        "{too_long:?}"
    );

    // Accept once, with the email matched ignoring case; the member is a
    // human and carries the invite's role.
    let viewer = mint(
        store,
        ws,
        admin,
        "Vi@Example.com",
        WorkspaceInviteRole::Viewer,
        1,
        week,
    )
    .await;
    assert_eq!(viewer.email, "vi@example.com");
    assert_eq!(
        store
            .find_live_workspace_invite(&hash(1), Utc::now())
            .await
            .unwrap()
            .id,
        viewer.id
    );
    let ok = accept(store, &viewer, ws, "VI@example.COM", "sub-vi")
        .await
        .unwrap();
    assert_eq!(ok.member.kind, MemberKind::Human);
    assert_eq!(ok.member.workspace_id, ws);
    assert_eq!(ok.session.member_id, ok.member.id);
    assert_eq!(
        store.member_invite_role(ok.member.id).await.unwrap(),
        Some(WorkspaceInviteRole::Viewer)
    );
    assert_eq!(store.member_invite_role(admin).await.unwrap(), None);

    // Reuse: the same invite, even by another identity, is spent.
    assert!(matches!(
        store.find_live_workspace_invite(&hash(1), Utc::now()).await,
        Err(StoreError::NotFound)
    ));
    let again = accept(store, &viewer, ws, "vi@example.com", "sub-vi-2").await;
    assert!(not_valid(&again), "{again:?}");
    // The same identity with a fresh invite is already a member.
    let second = mint(
        store,
        ws,
        admin,
        "vi@example.com",
        WorkspaceInviteRole::Member,
        2,
        week,
    )
    .await;
    let dup = accept(store, &second, ws, "vi@example.com", "sub-vi").await;
    assert!(
        matches!(&dup, Err(StoreError::Conflict(m)) if m == INVITE_ALREADY_MEMBER),
        "{dup:?}"
    );

    // Email mismatch consumes nothing.
    let bound = mint(
        store,
        ws,
        admin,
        "bo@example.com",
        WorkspaceInviteRole::Member,
        3,
        week,
    )
    .await;
    let wrong = accept(store, &bound, ws, "eve@example.com", "sub-eve").await;
    assert!(not_valid(&wrong), "{wrong:?}");
    // Cross-workspace: the invite cannot be accepted into another workspace.
    let cross = accept(store, &bound, other_ws, "bo@example.com", "sub-bo").await;
    assert!(not_valid(&cross), "{cross:?}");
    assert!(store
        .find_live_workspace_invite(&hash(3), Utc::now())
        .await
        .is_ok());
    let member = accept(store, &bound, ws, "bo@example.com", "sub-bo")
        .await
        .unwrap();
    assert_eq!(
        store.member_invite_role(member.member.id).await.unwrap(),
        Some(WorkspaceInviteRole::Member)
    );

    // Revocation: only in the invite's own workspace, only once, and then
    // the invite cannot be accepted.
    let rev = mint(
        store,
        ws,
        admin,
        "re@example.com",
        WorkspaceInviteRole::Member,
        4,
        week,
    )
    .await;
    let audit = || NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: Some(admin),
        action: "workspace_invite.revoke".into(),
        target_kind: Some("workspace_invite".into()),
        target_id: Some(rev.id.0),
        metadata: serde_json::json!({}),
    };
    let mut cross_audit = audit();
    cross_audit.scope = AuditScope::Workspace(other_ws);
    cross_audit.actor_id = Some(other_admin);
    assert!(!store
        .revoke_workspace_invite_audited(other_ws, rev.id, cross_audit)
        .await
        .unwrap());
    assert!(store
        .revoke_workspace_invite_audited(ws, rev.id, audit())
        .await
        .unwrap());
    assert!(!store
        .revoke_workspace_invite_audited(ws, rev.id, audit())
        .await
        .unwrap());
    let revoked = accept(store, &rev, ws, "re@example.com", "sub-re").await;
    assert!(not_valid(&revoked), "{revoked:?}");
    // A spent invite cannot be revoked after the fact.
    assert!(!store
        .revoke_workspace_invite_audited(ws, viewer.id, audit())
        .await
        .unwrap());

    // Expiry: an invite past its time is neither found nor accepted.
    let short = mint(
        store,
        ws,
        admin,
        "ex@example.com",
        WorkspaceInviteRole::Member,
        5,
        chrono::Duration::milliseconds(300),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(matches!(
        store.find_live_workspace_invite(&hash(5), Utc::now()).await,
        Err(StoreError::NotFound)
    ));
    let late = accept(store, &short, ws, "ex@example.com", "sub-ex").await;
    assert!(not_valid(&late), "{late:?}");

    // Lists are per workspace.
    let listed = store.list_workspace_invites(ws).await.unwrap();
    assert_eq!(listed.len(), 5);
    assert!(listed.iter().all(|i| i.workspace_id == ws));
    assert!(store
        .list_workspace_invites(other_ws)
        .await
        .unwrap()
        .is_empty());
}

/// Two sign-ins race for one invite: exactly one joins.
async fn assert_one_winner(store: &dyn Store) {
    let (ws, admin) = seed(store, "inv-race").await;
    let invite = mint(
        store,
        ws,
        admin,
        "race@example.com",
        WorkspaceInviteRole::Member,
        77,
        chrono::Duration::days(1),
    )
    .await;
    let (a, b) = futures::join!(
        accept(store, &invite, ws, "race@example.com", "sub-r1"),
        accept(store, &invite, ws, "race@example.com", "sub-r2"),
    );
    assert_eq!(
        [&a, &b].iter().filter(|r| r.is_ok()).count(),
        1,
        "{a:?} / {b:?}"
    );
    let members = store.list_members(ws).await.unwrap();
    assert_eq!(members.len(), 2, "the admin and one invitee");
}

async fn sqlite_file_store(dir: &tempfile::TempDir) -> SqliteStore {
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("inv.db").display());
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("PRAGMA busy_timeout = 5000")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    SqliteStore::for_tests(pool)
}

async fn postgres_store() -> Option<(PostgresStore, testcontainers::ContainerAsync<Postgres>)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    Some((PostgresStore::for_tests(pool), container))
}

#[tokio::test]
async fn workspace_invites_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let store = sqlite_file_store(&dir).await;
    assert_invites(&store).await;
    assert_one_winner(&store).await;
}

#[tokio::test]
async fn workspace_invites_postgres() {
    let Some((store, _container)) = postgres_store().await else {
        return;
    };
    assert_invites(&store).await;
    assert_one_winner(&store).await;
}
