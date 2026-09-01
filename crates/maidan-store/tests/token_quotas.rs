//! Token capability quotas persistence (Cluster 54).

use maidan_auth::hash_secret;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, TokenQuota};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn replace_and_list_token_quotas() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace { name: "q".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret("tok"),
            label: None,
            capabilities: vec!["workspace:read".into(), "message:post".into()],
            expires_at: None,
        })
        .await
        .unwrap();

    store
        .replace_token_quotas(
            token.id,
            &[
                TokenQuota {
                    capability: "workspace:read".into(),
                    max_per_window: 10,
                    window_secs: 60,
                },
                TokenQuota {
                    capability: "message:post".into(),
                    max_per_window: 3,
                    window_secs: 30,
                },
            ],
        )
        .await
        .unwrap();

    let listed = store.list_token_quotas(token.id).await.unwrap();
    assert_eq!(listed.len(), 2);

    store
        .replace_token_quotas(
            token.id,
            &[TokenQuota {
                capability: "message:post".into(),
                max_per_window: 1,
                window_secs: 10,
            }],
        )
        .await
        .unwrap();
    let listed = store.list_token_quotas(token.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].capability, "message:post");

    store.revoke_api_token(token.id).await.unwrap();
    let listed = store.list_token_quotas(token.id).await.unwrap();
    assert_eq!(listed.len(), 1);
}
