//! The front door's store side: a pending sign-in with no workspace, a pending
//! sign-in naming a workspace that need not exist, the CHECK that keeps the
//! two kinds apart, and the lookup by issuer and subject. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewMember, NewOidcIdentity, NewOidcPendingAuth, NewWorkspace,
    OidcPendingTarget, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

const ISSUER: &str = "https://idp.example";

fn pending(state: &str, target: OidcPendingTarget) -> NewOidcPendingAuth {
    NewOidcPendingAuth {
        state: state.into(),
        target,
        nonce: "nonce".into(),
        pkce_verifier: "verifier".into(),
        return_to: Some("/ui/".into()),
        expires_at: Utc::now() + Duration::minutes(10),
    }
}

async fn run_suite(store: &dyn Store) {
    // A front-door row names no workspace, and comes back as one.
    store
        .insert_oidc_pending(pending("front", OidcPendingTarget::FrontDoor))
        .await
        .expect("front door pending");
    let front = store.take_oidc_pending("front").await.expect("take front");
    assert_eq!(front.target, OidcPendingTarget::FrontDoor);

    // A workspace id nobody created is stored like a real one: login must not
    // be able to tell them apart by failing on the unknown one.
    let unknown = WorkspaceId(uuid::Uuid::new_v4());
    store
        .insert_oidc_pending(pending("unknown", OidcPendingTarget::Workspace(unknown)))
        .await
        .expect("an unknown workspace id is stored like a real one");
    assert_eq!(
        store
            .take_oidc_pending("unknown")
            .await
            .expect("take")
            .target,
        OidcPendingTarget::Workspace(unknown)
    );

    // Two tenants: X has members in A and B, Y in C. The lookup finds a
    // subject's workspaces and nothing else.
    let mut ws = Vec::new();
    for name in ["front-a", "front-b", "front-c", "front-d"] {
        ws.push(
            store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .expect("workspace"),
        );
    }
    let (a, b, c, d) = (ws[0].id, ws[1].id, ws[2].id, ws[3].id);
    let member = |w: WorkspaceId, handle: &'static str| async move {
        store
            .create_member(NewMember {
                workspace_id: w,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .expect("member")
            .id
    };
    let link = |w: WorkspaceId, issuer: &'static str, subject: &'static str, m: MemberId| async move {
        store
            .upsert_oidc_identity(NewOidcIdentity {
                workspace_id: w,
                issuer: issuer.into(),
                subject: subject.into(),
                member_id: m,
                email: Some("x@example.com".into()),
            })
            .await
            .expect("identity")
    };
    let xa = member(a, "x").await;
    let xb = member(b, "x").await;
    let yc = member(c, "y").await;
    // Same subject at another issuer: a different person.
    let other_issuer = member(d, "x").await;
    link(a, ISSUER, "x", xa).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    link(b, ISSUER, "x", xb).await;
    link(c, ISSUER, "y", yc).await;
    link(d, "https://other.example", "x", other_issuer).await;

    let ids = |rows: Vec<maidan_types::IdentityWorkspace>| -> Vec<WorkspaceId> {
        rows.iter().map(|w| w.workspace_id).collect()
    };
    let x = store
        .list_subject_workspaces(ISSUER, "x", 200)
        .await
        .expect("list x");
    assert_eq!(x[0].member_id, xb, "{x:?}");
    assert_eq!(ids(x), vec![b, a], "newest sign-in first, never C or D");
    assert_eq!(
        ids(store
            .list_subject_workspaces(ISSUER, "y", 200)
            .await
            .expect("list y")),
        vec![c]
    );
    assert!(store
        .list_subject_workspaces(ISSUER, "nobody", 200)
        .await
        .expect("list nobody")
        .is_empty());
    assert_eq!(
        store
            .list_subject_workspaces(ISSUER, "x", 1)
            .await
            .expect("bounded")
            .len(),
        1
    );

    // A SCIM-deactivated member's workspace is left out, and so is a frozen
    // member's: the front door never lands anyone where they can't sign in.
    store
        .create_scim_user(xb, b, Some("ext-xb"), false)
        .await
        .expect("scim");
    assert_eq!(
        ids(store
            .list_subject_workspaces(ISSUER, "x", 200)
            .await
            .expect("list")),
        vec![a]
    );
    let admin = member(a, "admin").await;
    store
        .freeze_member(xa, admin, Some("test"))
        .await
        .expect("freeze");
    assert!(store
        .list_subject_workspaces(ISSUER, "x", 200)
        .await
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn front_door_store_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    // The CHECK refuses a front-door row naming a workspace, and a sign-in row
    // naming none.
    for (kind, ws) in [
        ("front_door", Some(uuid::Uuid::new_v4().to_string())),
        ("sign_in", None),
        ("sign_up", None),
    ] {
        let res = sqlx::query(
            "INSERT INTO maidan_oidc_pending
                (state, kind, workspace_id, nonce, pkce_verifier, created_at, expires_at)
             VALUES (?, ?, ?, 'n', 'v', datetime('now'), datetime('now', '+10 minutes'))",
        )
        .bind(format!("bad-{kind}"))
        .bind(kind)
        .bind(ws)
        .execute(&pool)
        .await;
        assert!(res.is_err(), "{kind} must be refused");
    }
    let store = SqliteStore::for_tests(pool);
    run_suite(&store).await;
}

#[tokio::test]
async fn front_door_store_postgres() {
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
    for (kind, ws) in [
        ("front_door", Some(uuid::Uuid::new_v4())),
        ("sign_in", None),
        ("sign_up", None),
    ] {
        let res = sqlx::query(
            "INSERT INTO maidan_oidc_pending
                (state, kind, workspace_id, nonce, pkce_verifier, expires_at)
             VALUES ($1, $2, $3, 'n', 'v', NOW() + INTERVAL '10 minutes')",
        )
        .bind(format!("bad-{kind}"))
        .bind(kind)
        .bind(ws)
        .execute(&pool)
        .await;
        assert!(res.is_err(), "{kind} must be refused");
    }
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
