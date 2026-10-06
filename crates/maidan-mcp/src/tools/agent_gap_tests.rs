//! Authorized callers can use the agent-facing gap tools, and a caller
//! without the capability is refused.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{
    capability::{MESSAGE_POST, SEARCH_QUERY, THREAD_TRANSITION, WORKSPACE_READ},
    AuthContext,
};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

use crate::error::McpError;
use crate::server::McpServer;

async fn server() -> (
    McpServer,
    WorkspaceId,
    MemberId,
    MemberId,
    MemberId,
    ThreadId,
    ThreadId,
    MessageId,
) {
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
    let workspace = store
        .create_workspace(NewWorkspace { name: "gap".into() })
        .await
        .unwrap();
    let mut members = Vec::new();
    for handle in ["ada", "bea", "cy"] {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: workspace.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: workspace.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread_a = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("a".into()),
            description: None,
        })
        .await
        .unwrap();
    let thread_b = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("b".into()),
            description: None,
        })
        .await
        .unwrap();
    store
        .add_thread_dependency(thread_a.id, thread_b.id)
        .await
        .unwrap();
    let message = store
        .post_message(NewMessage {
            thread_id: thread_a.id,
            author_id: members[0],
            body: "hello".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    (
        server,
        workspace.id,
        members[0],
        members[1],
        members[2],
        thread_a.id,
        thread_b.id,
        message.id,
    )
}

fn bearer(member: MemberId, workspace: WorkspaceId, caps: Vec<String>) -> AuthContext {
    AuthContext::from_token(ApiTokenId::new(), member, workspace, caps)
}

fn assert_forbidden(err: McpError) {
    match err {
        McpError::Forbidden(_) => {}
        other => panic!("expected forbidden, got {other}"),
    }
}

#[tokio::test]
async fn authorized_gap_tools_succeed_and_a_weak_caller_is_refused() {
    let (server, workspace, ada, bea, cy, thread_a, thread_b, message) = server().await;
    let allowed = bearer(
        ada,
        workspace,
        vec![
            WORKSPACE_READ.to_string(),
            MESSAGE_POST.to_string(),
            THREAD_TRANSITION.to_string(),
        ],
    );
    let weak = bearer(ada, workspace, vec![SEARCH_QUERY.to_string()]);

    let members = server
        .call_tool(
            &allowed,
            "list_members",
            &json!({ "workspace_id": workspace.0 }),
        )
        .await
        .expect("list_members");
    let listed: Vec<Value> =
        serde_json::from_str(members["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(listed.len(), 3);

    server
        .call_tool(
            &allowed,
            "tombstone_message",
            &json!({ "message_id": message.0 }),
        )
        .await
        .expect("tombstone_message");
    let stored = server.store.get_message(message).await.unwrap();
    assert!(stored.tombstoned_at.is_some());

    server
        .call_tool(
            &allowed,
            "open_group_dm",
            &json!({
                "workspace_id": workspace.0,
                "member_ids": [ada.0, bea.0, cy.0],
            }),
        )
        .await
        .expect("open_group_dm");

    server
        .call_tool(
            &allowed,
            "remove_thread_dependency",
            &json!({
                "thread_id": thread_a.0,
                "depends_on_thread_id": thread_b.0,
            }),
        )
        .await
        .expect("remove_thread_dependency");
    let remaining = server
        .store
        .list_thread_dependencies(thread_a)
        .await
        .unwrap();
    assert!(remaining.is_empty());

    for (name, args) in [
        ("list_members", json!({ "workspace_id": workspace.0 })),
        ("tombstone_message", json!({ "message_id": message.0 })),
        (
            "open_group_dm",
            json!({
                "workspace_id": workspace.0,
                "member_ids": [ada.0, bea.0, cy.0],
            }),
        ),
        (
            "remove_thread_dependency",
            json!({
                "thread_id": thread_a.0,
                "depends_on_thread_id": thread_b.0,
            }),
        ),
    ] {
        let err = server.call_tool(&weak, name, &args).await.expect_err(name);
        assert_forbidden(err);
    }
}
