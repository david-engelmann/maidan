//! OIDC identity, pending auth, and session store coverage.

use chrono::{Duration, Utc};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    MemberKind, NewMaidanSession, NewMember, NewOidcIdentity, NewOidcPendingAuth, NewWorkspace,
    WorkspaceId,
};

async fn seed_workspace(store: &SqliteStore) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace {
            name: "oidc-ws".to_string(),
        })
        .await
        .expect("workspace")
        .id
}

async fn seed_human(store: &SqliteStore, workspace_id: WorkspaceId) -> maidan_types::MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: "human-1".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
        .id
}

#[tokio::test]
async fn oidc_identity_upsert_and_lookup_roundtrip() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("fk");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let workspace_id = seed_workspace(&store).await;
    let member_id = seed_human(&store, workspace_id).await;

    let identity = store
        .upsert_oidc_identity(NewOidcIdentity {
            workspace_id,
            issuer: "https://idp.example".to_string(),
            subject: "sub-1".to_string(),
            member_id,
            email: Some("human@example.com".to_string()),
        })
        .await
        .expect("upsert");

    let fetched = store
        .get_oidc_identity(workspace_id, "https://idp.example", "sub-1")
        .await
        .expect("get");
    assert_eq!(fetched.id, identity.id);
    assert_eq!(fetched.member_id, member_id);
}

#[tokio::test]
async fn oidc_pending_is_single_use() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("fk");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let workspace_id = seed_workspace(&store).await;

    store
        .insert_oidc_pending(NewOidcPendingAuth {
            state: "state-abc".to_string(),
            workspace_id,
            nonce: "nonce".to_string(),
            pkce_verifier: "verifier".to_string(),
            return_to: Some("/ui/".to_string()),
            expires_at: Utc::now() + Duration::minutes(10),
        })
        .await
        .expect("insert");

    let pending = store.take_oidc_pending("state-abc").await.expect("take");
    assert_eq!(pending.workspace_id, workspace_id);

    let err = store
        .take_oidc_pending("state-abc")
        .await
        .expect_err("second take");
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}

#[tokio::test]
async fn session_create_get_and_delete() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("fk");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let workspace_id = seed_workspace(&store).await;
    let member_id = seed_human(&store, workspace_id).await;

    let session = store
        .create_session(NewMaidanSession {
            workspace_id,
            member_id,
            csrf_secret: "csrf".to_string(),
            expires_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("create");

    let loaded = store.get_session(session.id).await.expect("get");
    assert_eq!(loaded.member_id, member_id);

    store.delete_session(session.id).await.expect("delete");
    let err = store.get_session(session.id).await.expect_err("deleted");
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}

#[tokio::test]
async fn get_member_by_handle_resolves_workspace_member() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("fk");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let workspace_id = seed_workspace(&store).await;
    let member_id = seed_human(&store, workspace_id).await;

    let member = store
        .get_member_by_handle(workspace_id, "human-1")
        .await
        .expect("by handle");
    assert_eq!(member.id, member_id);
}
