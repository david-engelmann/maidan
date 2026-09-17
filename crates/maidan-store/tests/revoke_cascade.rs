//! Revoking a token revokes everything derived from it.
//!
//! Holder-side attenuation came first, then a derived token inheriting the
//! parent's app installation and per-token quotas, because
//! re-issuing was otherwise a way to shed a bound. Revocation was the dimension
//! still leaking — the parent link lived only in audit metadata, so a child
//! outlived the credential it was minted from.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "revoke".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "r-agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    let new = |hash: &'static str| NewApiToken {
        workspace_id: ws.id,
        member_id: member.id,
        app_installation_id: None,
        token_hash: hash.into(),
        label: None,
        capabilities: vec!["workspace:read".into()],
        expires_at: None,
    };

    // root → child → grandchild, plus an unrelated token that must survive.
    let root = store.create_api_token(new("h-root")).await.expect("root");
    let child = store
        .create_attenuated_api_token(new("h-child"), root.id)
        .await
        .expect("child");
    let grandchild = store
        .create_attenuated_api_token(new("h-grandchild"), child.id)
        .await
        .expect("grandchild");
    let sibling = store
        .create_api_token(new("h-sibling"))
        .await
        .expect("sibling");

    let revoked_at = |id| async move { store.get_api_token(id).await.expect("token").revoked_at };
    assert!(revoked_at(child.id).await.is_none());
    assert!(revoked_at(grandchild.id).await.is_none());

    // Revoking the root reaches the whole subtree, not just its direct child —
    // a one-level cascade would leave the grandchild live, which is the same
    // leak one generation down.
    let returned = store.revoke_api_token(root.id).await.expect("revoke");
    assert_eq!(returned.id, root.id, "the caller still gets the root back");
    assert!(returned.revoked_at.is_some());
    assert!(
        revoked_at(child.id).await.is_some(),
        "a derived token must not outlive the credential it was minted from"
    );
    assert!(
        revoked_at(grandchild.id).await.is_some(),
        "the cascade must be transitive, not one level deep"
    );

    // An unrelated token is untouched — the cascade follows the link, not the
    // member or the workspace.
    assert!(
        revoked_at(sibling.id).await.is_none(),
        "revocation must not reach tokens that were never derived from the root"
    );

    // Revoking an already-revoked token is still NotFound, unchanged.
    assert!(store.revoke_api_token(root.id).await.is_err());

    // A child revoked earlier keeps its own timestamp when an ancestor is
    // revoked later — the cascade skips rows that are already revoked, so
    // "when was this killed" stays true.
    let a = store.create_api_token(new("h-a")).await.expect("a");
    let b = store
        .create_attenuated_api_token(new("h-b"), a.id)
        .await
        .expect("b");
    store.revoke_api_token(b.id).await.expect("revoke child");
    let b_first = revoked_at(b.id).await.expect("b revoked");
    store.revoke_api_token(a.id).await.expect("revoke parent");
    assert_eq!(
        revoked_at(b.id).await.expect("still revoked"),
        b_first,
        "an already-revoked child keeps its original revocation time"
    );
}

#[tokio::test]
async fn revoking_a_parent_revokes_its_derived_tokens_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn revoking_a_parent_revokes_its_derived_tokens_postgres() {
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
        Ok(c) => c,
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
    run_suite(&PostgresStore::new(pool)).await;
}
