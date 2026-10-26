//! The waitlist and the invited sign-up transaction, on both backends:
//! the list holds one row per address, an invite is minted and consumed
//! exactly once, and a sign-up always lands in a workspace born in its
//! own transaction — never in one that existed before it.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

fn signup(email: &str, invite: Option<WaitlistEntryId>) -> NewOidcSignup {
    NewOidcSignup {
        workspace_name: "ada's workspace".into(),
        handle: "ada".into(),
        issuer: "https://idp.example".into(),
        subject: "sub-ada".into(),
        email: email.into(),
        invite_entry_id: invite,
        session_expires_at: Utc::now() + Duration::from_secs(3600),
        open: false,
    }
}

fn no_audit() -> (
    maidan_store::AuditFor<OidcSignupCore>,
    maidan_store::AuditFor<MaidanSession>,
) {
    (
        Box::new(|core| NewAuditEvent {
            scope: AuditScope::Workspace(core.workspace.id),
            actor_id: Some(core.member.id),
            action: "workspace.signup".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(core.workspace.id.0),
            metadata: serde_json::json!({}),
        }),
        Box::new(|session| NewAuditEvent {
            scope: AuditScope::Workspace(session.workspace_id),
            actor_id: Some(session.member_id),
            action: "session.create".into(),
            target_kind: Some("member".into()),
            target_id: Some(session.member_id.0),
            metadata: serde_json::json!({}),
        }),
    )
}

async fn assert_signup(store: &dyn Store) {
    // The list: one row per address, matched case-insensitively.
    let entry = store
        .insert_waitlist_entry(NewWaitlistEntry {
            email: "Ada@Example.com".into(),
            use_case: "running a support swarm".into(),
            consented_at: Utc::now(),
        })
        .await
        .unwrap();
    assert_eq!(entry.status, WaitlistStatus::Waiting);
    let dupe = store
        .insert_waitlist_entry(NewWaitlistEntry {
            email: "ada@example.com".into(),
            use_case: "again".into(),
            consented_at: Utc::now(),
        })
        .await;
    assert!(
        matches!(dupe, Err(StoreError::Conflict(_))),
        "a second row for the same address must conflict, got {dupe:?}"
    );

    // An invite is minted over the entry and found by its hash.
    let invited = store
        .set_waitlist_invite(
            entry.id,
            "hash-ada".into(),
            Utc::now() + Duration::from_secs(3600),
        )
        .await
        .unwrap();
    assert_eq!(invited.status, WaitlistStatus::Invited);
    let by_hash = store
        .get_waitlist_entry_by_invite_hash("hash-ada")
        .await
        .unwrap();
    assert_eq!(by_hash.id, entry.id);

    // The two-tenant core: a workspace already exists whose admin uses
    // this very handle, and the person is even a member there by email
    // handle. The sign-up must still create its own workspace and touch
    // nothing in the existing one.
    let existing = store
        .create_workspace(NewWorkspace {
            name: "existing".into(),
        })
        .await
        .unwrap();
    let existing_admin = store
        .create_member(NewMember {
            workspace_id: existing.id,
            handle: "ada".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let members_before = store.list_members(existing.id).await.unwrap().len();

    let (signup_audit, session_audit) = no_audit();
    let outcome = store
        .complete_oidc_signup(
            signup("ada@example.com", Some(entry.id)),
            signup_audit,
            session_audit,
        )
        .await
        .unwrap();
    assert_ne!(outcome.workspace.id, existing.id);
    assert_eq!(outcome.member.workspace_id, outcome.workspace.id);
    assert_eq!(outcome.member.id, outcome.session.member_id);
    assert_ne!(outcome.member.id, existing_admin.id);
    assert_eq!(outcome.identity.member_id, outcome.member.id);
    assert_eq!(outcome.identity.workspace_id, outcome.workspace.id);
    assert_eq!(outcome.session.oidc_identity_id, Some(outcome.identity.id));
    // The existing workspace gained nobody and no identity.
    assert_eq!(
        store.list_members(existing.id).await.unwrap().len(),
        members_before
    );
    assert!(store
        .get_oidc_identity(existing.id, "https://idp.example", "sub-ada")
        .await
        .is_err());
    // The audit rows landed with the transaction.
    let audit = store
        .list_audit_for_workspace(outcome.workspace.id, 10)
        .await
        .unwrap();
    let actions: Vec<&str> = audit.iter().map(|a| a.action.as_str()).collect();
    assert!(actions.contains(&"workspace.signup"), "audit: {actions:?}");
    assert!(actions.contains(&"session.create"), "audit: {actions:?}");

    // The invite is consumed: the entry is signed up, bound to the new
    // workspace, and a second sign-up on it is refused.
    let after = store.get_waitlist_entry(entry.id).await.unwrap();
    assert_eq!(after.status, WaitlistStatus::SignedUp);
    assert_eq!(after.workspace_id, Some(outcome.workspace.id));
    // (A fresh subject, so the refusal is the invite's and not the identity's.)
    let (a1, a2) = no_audit();
    let mut again = signup("ada@example.com", Some(entry.id));
    again.subject = "sub-ada-2".into();
    let again = store.complete_oidc_signup(again, a1, a2).await;
    assert!(
        matches!(&again, Err(StoreError::Conflict(m)) if m != maidan_store::SIGNUP_IDENTITY_TAKEN),
        "a consumed invite must not sign up twice, got {again:?}"
    );
    // And no new invite can be minted over a completed sign-up.
    assert!(matches!(
        store
            .set_waitlist_invite(
                entry.id,
                "hash-new".into(),
                Utc::now() + Duration::from_secs(60)
            )
            .await,
        Err(StoreError::Conflict(_))
    ));

    // An invite addressed to someone else does not sign this person up.
    let other = store
        .insert_waitlist_entry(NewWaitlistEntry {
            email: "grace@example.com".into(),
            use_case: "compilers".into(),
            consented_at: Utc::now(),
        })
        .await
        .unwrap();
    store
        .set_waitlist_invite(
            other.id,
            "hash-grace".into(),
            Utc::now() + Duration::from_secs(3600),
        )
        .await
        .unwrap();
    let (a1, a2) = no_audit();
    let mut wrong_email = signup("ada@example.com", Some(other.id));
    wrong_email.subject = "sub-ada-3".into();
    let wrong_email = store.complete_oidc_signup(wrong_email, a1, a2).await;
    assert!(
        matches!(&wrong_email, Err(StoreError::Conflict(m)) if m != maidan_store::SIGNUP_IDENTITY_TAKEN),
        "the invite's refusal, not the identity's: {wrong_email:?}"
    );
    // An expired invite is refused the same way.
    store
        .set_waitlist_invite(
            other.id,
            "hash-grace-2".into(),
            Utc::now() - Duration::from_secs(1),
        )
        .await
        .unwrap();
    let (a1, a2) = no_audit();
    let mut expired = signup("grace@example.com", Some(other.id));
    expired.subject = "sub-grace".into();
    let expired = store.complete_oidc_signup(expired, a1, a2).await;
    assert!(
        matches!(&expired, Err(StoreError::Conflict(m)) if m != maidan_store::SIGNUP_IDENTITY_TAKEN),
        "the invite's refusal, not the identity's: {expired:?}"
    );

    // The domain path: no invite. A waiting entry with the same address is
    // marked signed up and bound to the new workspace.
    let waiting = store
        .insert_waitlist_entry(NewWaitlistEntry {
            email: "hopper@example.com".into(),
            use_case: "navy work".into(),
            consented_at: Utc::now(),
        })
        .await
        .unwrap();
    let (a1, a2) = no_audit();
    let mut domain_signup = signup("hopper@example.com", None);
    domain_signup.subject = "sub-hopper".into();
    domain_signup.handle = "hopper".into();
    let outcome = store
        .complete_oidc_signup(domain_signup, a1, a2)
        .await
        .unwrap();
    let marked = store.get_waitlist_entry(waiting.id).await.unwrap();
    assert_eq!(marked.status, WaitlistStatus::SignedUp);
    assert_eq!(marked.workspace_id, Some(outcome.workspace.id));

    // The pending row: a sign-up kind round-trips with its invite bound,
    // and kinds stay distinct.
    store
        .insert_oidc_pending(NewOidcPendingAuth {
            state: "state-signup".into(),
            target: OidcPendingTarget::SignUp,
            nonce: "n".into(),
            pkce_verifier: "v".into(),
            return_to: None,
            expires_at: Utc::now() + Duration::from_secs(600),
            invite_entry_id: Some(other.id),
        })
        .await
        .unwrap();
    let taken = store.take_oidc_pending("state-signup").await.unwrap();
    assert_eq!(taken.target, OidcPendingTarget::SignUp);
    assert_eq!(taken.invite_entry_id, Some(other.id));
}

#[tokio::test]
async fn waitlist_and_signup_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    assert_signup(&store).await;
}

#[tokio::test]
async fn waitlist_and_signup_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    assert_signup(&store).await;
}

/// Two callbacks for one identity at once, beside a third for another
/// identity: exactly one of the pair creates a workspace and the other is
/// refused as an identity that already has one, while the other identity
/// signs up untouched by their lock.
async fn assert_concurrent_signups_for_one_identity_make_one_workspace(store: &dyn Store) {
    let issuer = "https://idp.example";
    let attempt = |subject: &str, email: &str| {
        let mut new = signup(email, None);
        new.subject = subject.into();
        new.handle = subject.into();
        let (a1, a2) = no_audit();
        store.complete_oidc_signup(new, a1, a2)
    };
    let (first, second, other) = futures::join!(
        attempt("sub-twice", "twice@example.com"),
        attempt("sub-twice", "twice@example.com"),
        attempt("sub-other", "other@example.com"),
    );
    let won: Vec<_> = [&first, &second]
        .into_iter()
        .filter(|r| r.is_ok())
        .collect();
    assert_eq!(
        won.len(),
        1,
        "exactly one must land: {first:?} / {second:?}"
    );
    let lost = if first.is_ok() { &second } else { &first };
    assert!(
        matches!(lost, Err(StoreError::Conflict(m)) if m == maidan_store::SIGNUP_IDENTITY_TAKEN),
        "the loser is refused as an identity with a workspace, got {lost:?}"
    );
    assert_eq!(
        store
            .list_subject_workspaces(issuer, "sub-twice", 10)
            .await
            .unwrap()
            .len(),
        1
    );
    let other = other.unwrap();
    let theirs = store
        .list_subject_workspaces(issuer, "sub-other", 10)
        .await
        .unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].workspace_id, other.workspace.id);
    assert_ne!(
        Some(other.workspace.id),
        won.first()
            .and_then(|r| r.as_ref().ok())
            .map(|o| o.workspace.id)
    );
}

#[tokio::test]
async fn concurrent_signups_for_one_identity_make_one_workspace_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("signup.db").display()
    );
    let pool = SqlitePoolOptions::new()
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
    let store = SqliteStore::for_tests(pool);
    assert_concurrent_signups_for_one_identity_make_one_workspace(&store).await;
}

#[tokio::test]
async fn concurrent_signups_for_one_identity_make_one_workspace_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    assert_concurrent_signups_for_one_identity_make_one_workspace(&store).await;
}
