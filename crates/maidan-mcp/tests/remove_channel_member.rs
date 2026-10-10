//! `remove_channel_member` answers an id that is no member of the channel's
//! workspace as not found: it claims no removal and writes no audit row naming
//! the id, whether the id is unknown or another workspace's member.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability::CHANNEL_ADMIN, AuthContext};
use maidan_mcp::{error::McpError, server::McpServer};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn removing_a_member_from_another_workspace_is_not_found_and_unaudited() {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let member = |ws: WorkspaceId, handle: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
        }
    };
    let ours = store
        .create_workspace(NewWorkspace {
            name: "ours".into(),
        })
        .await
        .unwrap();
    let theirs = store
        .create_workspace(NewWorkspace {
            name: "theirs".into(),
        })
        .await
        .unwrap();
    let admin = member(ours.id, "admin").await;
    let colleague = member(ours.id, "colleague").await;
    let outsider = member(theirs.id, "outsider").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ours.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let auth = AuthContext::from_session(admin.id, ours.id, vec![CHANNEL_ADMIN.to_string()]);
    let remove = |id: uuid::Uuid| {
        let (server, auth) = (&server, &auth);
        let args = json!({ "channel_id": channel.id.0, "member_id": id });
        async move { server.call_tool(auth, "remove_channel_member", &args).await }
    };

    for id in [outsider.id.0, uuid::Uuid::now_v7()] {
        let refused = remove(id).await;
        assert!(
            matches!(refused, Err(McpError::NotFound)),
            "removing {id} must be not found, got {refused:?}"
        );
        assert!(!store
            .list_audit(500)
            .await
            .unwrap()
            .iter()
            .any(|row| row.action == "channel_member.remove"
                && row.metadata["subject_member_id"] == json!(id)));
    }
    assert!(remove(colleague.id.0).await.is_ok());
}
