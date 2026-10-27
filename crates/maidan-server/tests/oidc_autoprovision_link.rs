//! Auto-provisioning never hands a login an existing member by handle.
//!
//! With `MAIDAN_OIDC_AUTO_PROVISION=1`, the handle is derived from the email's
//! local part. That derivation proves nothing about who owns a member that
//! already holds the handle, so a colliding login gets a new member of its own,
//! and a login in one workspace never reaches another workspace's member. Run
//! on SQLite and on Postgres.

mod common;

use std::sync::Arc;

use maidan_server::oidc::{resolve_member_for_login, LoginMember};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewMember, NewWorkspace, WorkspaceId};
use sqlx::sqlite::SqlitePoolOptions;

const ISSUER: &str = "https://idp.example";

async fn sqlite() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::for_tests(pool))
}

async fn workspace_with_admin(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    (ws.id, admin.id)
}

async fn login(
    store: &dyn Store,
    ws: WorkspaceId,
    subject: &str,
    email: &str,
    verified: bool,
) -> (MemberId, LoginMember) {
    resolve_member_for_login(
        store,
        ws,
        ISSUER,
        subject,
        Some(email),
        verified,
        true,
        None,
    )
    .await
    .unwrap_or_else(|_| panic!("login {subject} refused"))
}

async fn an_attackers_local_part_does_not_attach_to_an_existing_member(store: Arc<dyn Store>) {
    let (ws, admin) = workspace_with_admin(store.as_ref(), "a").await;
    for verified in [false, true] {
        let subject = format!("attacker-{verified}");
        let (member, how) =
            login(store.as_ref(), ws, &subject, "admin@attacker.com", verified).await;
        assert_ne!(member, admin, "verified={verified} attached to admin");
        assert_eq!(how, LoginMember::Provisioned);
        let got = store.get_member(member).await.unwrap();
        assert_ne!(got.handle, "admin");
    }
    // The admin member gained no identity.
    let again = login(
        store.as_ref(),
        ws,
        "attacker-false",
        "admin@attacker.com",
        false,
    )
    .await;
    assert_eq!(again.1, LoginMember::Existing);
    assert_ne!(again.0, admin);
}

async fn a_login_in_tenant_b_never_links_to_tenant_as_member_with_the_same_handle(
    store: Arc<dyn Store>,
) {
    let (ws_a, admin_a) = workspace_with_admin(store.as_ref(), "a").await;
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let (member, how) = login(store.as_ref(), ws_b, "sub-b", "admin@b.example", true).await;
    assert_ne!(member, admin_a);
    assert_eq!(how, LoginMember::Provisioned);
    let got = store.get_member(member).await.unwrap();
    assert_eq!(got.workspace_id, ws_b);
    assert_eq!(got.handle, "admin");
    assert!(store
        .get_oidc_identity(ws_a, ISSUER, "sub-b")
        .await
        .is_err());
}

async fn legitimate_auto_provision_creates_a_member_and_reuses_it_on_the_next_login(
    store: Arc<dyn Store>,
) {
    let (ws, _) = workspace_with_admin(store.as_ref(), "a").await;
    let (first, how) = login(store.as_ref(), ws, "sub-carol", "carol@corp.example", true).await;
    assert_eq!(how, LoginMember::Provisioned);
    assert_eq!(store.get_member(first).await.unwrap().handle, "carol");
    let (second, how) = login(store.as_ref(), ws, "sub-carol", "carol@corp.example", true).await;
    assert_eq!((second, how), (first, LoginMember::Existing));
}

async fn a_case_folded_local_part_does_not_attach_and_a_taken_handle_is_a_conflict(
    store: Arc<dyn Store>,
) {
    let (ws, admin) = workspace_with_admin(store.as_ref(), "a").await;
    let (got, how) = login(
        store.as_ref(),
        ws,
        "sub-upper",
        "ADMIN@attacker.example",
        true,
    )
    .await;
    assert_ne!(got, admin);
    assert_eq!(how, LoginMember::Provisioned);
    // A login that loses the race for a handle sees this conflict from the
    // insert and moves on to the fallback handle instead of failing.
    let raced = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "Admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await;
    assert!(matches!(raced, Err(maidan_store::StoreError::Conflict(_))));
}

#[tokio::test]
async fn a_case_folded_local_part_does_not_attach_and_a_taken_handle_is_a_conflict_sqlite() {
    a_case_folded_local_part_does_not_attach_and_a_taken_handle_is_a_conflict(sqlite().await).await;
}

#[tokio::test]
async fn a_case_folded_local_part_does_not_attach_and_a_taken_handle_is_a_conflict_postgres() {
    let Some((_c, pool)) = common::postgres_pool().await else {
        return;
    };
    a_case_folded_local_part_does_not_attach_and_a_taken_handle_is_a_conflict(Arc::new(
        PostgresStore::for_tests(pool),
    ))
    .await;
}

#[tokio::test]
async fn an_attackers_email_local_part_does_not_attach_to_an_existing_member_sqlite() {
    an_attackers_local_part_does_not_attach_to_an_existing_member(sqlite().await).await;
}

#[tokio::test]
async fn a_login_in_tenant_b_never_links_to_tenant_a_member_with_the_same_handle_sqlite() {
    a_login_in_tenant_b_never_links_to_tenant_as_member_with_the_same_handle(sqlite().await).await;
}

#[tokio::test]
async fn legitimate_auto_provision_still_creates_and_reuses_a_member_sqlite() {
    legitimate_auto_provision_creates_a_member_and_reuses_it_on_the_next_login(sqlite().await)
        .await;
}

#[tokio::test]
async fn an_attackers_email_local_part_does_not_attach_to_an_existing_member_postgres() {
    let Some((_c, pool)) = common::postgres_pool().await else {
        return;
    };
    an_attackers_local_part_does_not_attach_to_an_existing_member(Arc::new(
        PostgresStore::for_tests(pool),
    ))
    .await;
}

#[tokio::test]
async fn a_login_in_tenant_b_never_links_to_tenant_a_member_with_the_same_handle_postgres() {
    let Some((_c, pool)) = common::postgres_pool().await else {
        return;
    };
    a_login_in_tenant_b_never_links_to_tenant_as_member_with_the_same_handle(Arc::new(
        PostgresStore::for_tests(pool),
    ))
    .await;
}

#[tokio::test]
async fn legitimate_auto_provision_still_creates_and_reuses_a_member_postgres() {
    let Some((_c, pool)) = common::postgres_pool().await else {
        return;
    };
    legitimate_auto_provision_creates_a_member_and_reuses_it_on_the_next_login(Arc::new(
        PostgresStore::for_tests(pool),
    ))
    .await;
}

async fn an_email_verified_by_another_issuer_does_not_link_to_an_existing_member(
    store: Arc<dyn Store>,
) {
    let (ws_a, _) = workspace_with_admin(store.as_ref(), "a").await;
    let (ws_b, _) = workspace_with_admin(store.as_ref(), "b").await;
    let mut linked = Vec::new();
    for ws in [ws_a, ws_b] {
        let admin = store
            .create_member(NewMember {
                workspace_id: ws,
                handle: "admin@corp.example".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap()
            .id;
        linked.push(admin);
        let untrusted = resolve_member_for_login(
            store.as_ref(),
            ws,
            "https://weak-idp.example",
            "sub-weak",
            Some("admin@corp.example"),
            true,
            false,
            Some(ISSUER),
        )
        .await;
        assert!(untrusted.is_err());
        assert!(store
            .get_oidc_identity(ws, "https://weak-idp.example", "sub-weak")
            .await
            .is_err());
        let trusted = resolve_member_for_login(
            store.as_ref(),
            ws,
            ISSUER,
            "sub-admin",
            Some("admin@corp.example"),
            true,
            false,
            Some(ISSUER),
        )
        .await
        .unwrap_or_else(|_| panic!("the configured issuer's login was refused"));
        assert_eq!(trusted, (admin, LoginMember::Linked));
    }
    assert!(store
        .get_oidc_identity(ws_b, ISSUER, "sub-admin")
        .await
        .map(|identity| identity.member_id == linked[1])
        .unwrap_or(false));
}

#[tokio::test]
async fn an_email_verified_by_another_issuer_does_not_link_to_an_existing_member_sqlite() {
    an_email_verified_by_another_issuer_does_not_link_to_an_existing_member(sqlite().await).await;
}

#[tokio::test]
async fn an_email_verified_by_another_issuer_does_not_link_to_an_existing_member_postgres() {
    let Some((_c, pool)) = common::postgres_pool().await else {
        return;
    };
    an_email_verified_by_another_issuer_does_not_link_to_an_existing_member(Arc::new(
        PostgresStore::for_tests(pool),
    ))
    .await;
}

async fn a_login_linked_to_a_removed_member_is_refused_even_with_auto_provision(
    store: Arc<dyn Store>,
    remove: impl AsyncFnOnce(MemberId),
) {
    let (ws_a, _) = workspace_with_admin(store.as_ref(), "a").await;
    let (ws_b, _) = workspace_with_admin(store.as_ref(), "b").await;
    let (gone, how) = login(store.as_ref(), ws_a, "sub-dana", "dana@corp.example", true).await;
    assert_eq!(how, LoginMember::Provisioned);
    let (kept, _) = login(store.as_ref(), ws_b, "sub-dana", "dana@corp.example", true).await;
    remove(gone).await;
    let again = resolve_member_for_login(
        store.as_ref(),
        ws_a,
        ISSUER,
        "sub-dana",
        Some("dana@corp.example"),
        true,
        true,
        None,
    )
    .await;
    assert!(matches!(
        again,
        Err(maidan_server::error::ApiError::Forbidden(ref why)) if why == maidan_server::oidc::NOT_PROVISIONED
    ));
    let other = login(store.as_ref(), ws_b, "sub-dana", "dana@corp.example", true).await;
    assert_eq!(other, (kept, LoginMember::Existing));
}

#[tokio::test]
async fn a_login_linked_to_a_removed_member_is_refused_even_with_auto_provision_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    a_login_linked_to_a_removed_member_is_refused_even_with_auto_provision(store, async |id| {
        sqlx::query("UPDATE maidan_members SET tombstoned_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id.0)
            .execute(&pool)
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_login_linked_to_a_removed_member_is_refused_even_with_auto_provision_postgres() {
    let Some((_c, pool)) = common::postgres_pool().await else {
        return;
    };
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    a_login_linked_to_a_removed_member_is_refused_even_with_auto_provision(store, async |id| {
        sqlx::query("UPDATE maidan_members SET tombstoned_at = NOW() WHERE id = $1")
            .bind(id.0)
            .execute(&pool)
            .await
            .unwrap();
    })
    .await;
}

async fn link_once(
    store: &dyn Store,
    ws: WorkspaceId,
    issuer: &str,
    link_issuer: Option<&str>,
) -> Result<(MemberId, LoginMember), maidan_server::error::ApiError> {
    resolve_member_for_login(
        store,
        ws,
        issuer,
        "sub-erin",
        Some("erin@corp.example"),
        true,
        false,
        link_issuer,
    )
    .await
}

#[tokio::test]
async fn a_link_by_email_stays_in_its_tenant_skips_a_removed_member_and_needs_a_named_issuer() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let (ws_a, _) = workspace_with_admin(store.as_ref(), "a").await;
    let (ws_b, _) = workspace_with_admin(store.as_ref(), "b").await;
    let mut erin = Vec::new();
    for ws in [ws_a, ws_b] {
        erin.push(
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: "erin@corp.example".into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
                .id,
        );
    }

    // No named issuer, or an empty one, never links.
    for (issuer, link) in [(ISSUER, None), ("", Some(ISSUER)), (ISSUER, Some(""))] {
        assert!(
            link_once(store.as_ref(), ws_a, issuer, link).await.is_err(),
            "issuer={issuer:?} link={link:?} linked"
        );
    }

    // The same issuer, sub and email in tenant A links A's member only.
    let a = link_once(store.as_ref(), ws_a, ISSUER, Some(ISSUER))
        .await
        .unwrap_or_else(|_| panic!("tenant A's link was refused"));
    assert_eq!(a, (erin[0], LoginMember::Linked));

    // Tenant B's member is removed: the same login there neither links to
    // it nor reaches tenant A's member, from the named issuer or any other.
    sqlx::query("UPDATE maidan_members SET tombstoned_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(erin[1].0)
        .execute(&pool)
        .await
        .unwrap();
    for issuer in [ISSUER, "https://other-idp.example"] {
        assert!(
            link_once(store.as_ref(), ws_b, issuer, Some(issuer))
                .await
                .is_err(),
            "a removed member in tenant B was linked from {issuer}"
        );
        assert!(store
            .get_oidc_identity(ws_b, issuer, "sub-erin")
            .await
            .is_err());
    }
    let again = link_once(store.as_ref(), ws_a, ISSUER, Some(ISSUER))
        .await
        .unwrap_or_else(|_| panic!("tenant A's identity stopped resolving"));
    assert_eq!(again, (erin[0], LoginMember::Existing));
}
