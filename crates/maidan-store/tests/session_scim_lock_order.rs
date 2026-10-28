//! A Postgres session write takes the SCIM row lock before the member row,
//! the same order a SCIM delete takes them (#1670). A sign-in for a member
//! whose SCIM delete is mid-transaction waits for it and is then refused,
//! and the delete commits instead of being chosen as a deadlock victim. A
//! sign-in in another tenant is not held up. Two tenants.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, PostgresStore, StoreError};
use maidan_types::{Member, MemberKind, NewMaidanSession, NewMember, NewWorkspace};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn scim_member(store: &dyn Store, name: &str) -> Member {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-human"),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .create_scim_user(member.id, ws.id, Some(name), true)
        .await
        .unwrap();
    member
}

fn session_for(member: &Member) -> NewMaidanSession {
    NewMaidanSession {
        workspace_id: member.workspace_id,
        member_id: member.id,
        api_token_id: None,
        oidc_identity_id: None,
        expires_at: Utc::now() + chrono::Duration::hours(1),
    }
}

#[tokio::test]
async fn a_sign_in_during_a_scim_delete_waits_and_is_refused_without_deadlocking_on_postgres() {
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
        .max_connections(6)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = std::sync::Arc::new(PostgresStore::for_tests(pool.clone()));
    let alpha = scim_member(store.as_ref(), "alpha").await;
    let beta = scim_member(store.as_ref(), "beta").await;

    // The SCIM delete's order: the SCIM row, then the member row.
    let mut delete = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM maidan_scim_users WHERE member_id = $1")
        .bind(alpha.id.0)
        .execute(&mut *delete)
        .await
        .unwrap();

    let racing = {
        let store = store.clone();
        let new = session_for(&alpha);
        tokio::spawn(async move { store.create_session(new).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !racing.is_finished(),
        "alpha's sign-in waits for the delete"
    );

    // Another tenant's sign-in goes straight through.
    let other = tokio::time::timeout(
        Duration::from_secs(5),
        store.create_session(session_for(&beta)),
    )
    .await
    .expect("beta's sign-in is not held up by alpha's delete")
    .expect("beta's session is created");
    assert_eq!(other.workspace_id, beta.workspace_id);

    sqlx::query("UPDATE maidan_members SET tombstoned_at = NOW() WHERE id = $1")
        .bind(alpha.id.0)
        .execute(&mut *delete)
        .await
        .expect("the delete is not a deadlock victim");
    delete.commit().await.expect("the delete commits");

    let refused = tokio::time::timeout(Duration::from_secs(10), racing)
        .await
        .expect("alpha's sign-in finishes")
        .unwrap()
        .expect_err("no session for a member deleted while signing in");
    assert!(matches!(refused, StoreError::Conflict(_)), "{refused:?}");
    assert!(store.get_session(other.id).await.is_ok());
}
