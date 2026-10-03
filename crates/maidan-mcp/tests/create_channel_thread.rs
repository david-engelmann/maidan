//! An agent with `workspace:write` creates a channel and a thread over MCP.
//! A caller without that capability is refused. The store path is the same one
//! REST uses (`create_channel_with_event` / `create_thread_with_event`).

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_mcp::{error::McpError, server::McpServer};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn server() -> (McpServer, Arc<dyn Store>, WorkspaceId, MemberId) {
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
        .create_workspace(NewWorkspace {
            name: "mcp-create".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    (server, store, workspace.id, member.id)
}

fn body(value: &Value) -> Value {
    serde_json::from_str(value["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn authorized_agent_creates_a_channel_and_a_thread() {
    let (server, store, workspace_id, member_id) = server().await;
    let writer =
        AuthContext::from_session(member_id, workspace_id, vec![WORKSPACE_WRITE.to_string()]);
    let reader =
        AuthContext::from_session(member_id, workspace_id, vec![WORKSPACE_READ.to_string()]);

    let refused_channel = server
        .call_tool(
            &reader,
            "create_channel",
            &json!({ "workspace_id": workspace_id.0, "name": "nope" }),
        )
        .await;
    assert!(
        matches!(&refused_channel, Err(McpError::Forbidden(m)) if m.contains("workspace:write")),
        "a caller without workspace:write must be refused, got {refused_channel:?}"
    );

    let channel = body(
        &server
            .call_tool(
                &writer,
                "create_channel",
                &json!({
                    "workspace_id": workspace_id.0,
                    "name": "work",
                    "topic": "agent room",
                    "private": true
                }),
            )
            .await
            .expect("workspace:write creates a channel"),
    );
    let channel_id = channel["id"]
        .as_str()
        .unwrap()
        .parse::<uuid::Uuid>()
        .unwrap();
    assert_eq!(channel["name"], "work");
    assert_eq!(channel["private"], true);

    let members = store
        .list_channel_members(ChannelId(channel_id))
        .await
        .unwrap();
    assert_eq!(
        members.len(),
        1,
        "the creator is seated on a private channel"
    );
    assert_eq!(members[0].member_id, member_id);
    assert_eq!(members[0].role, ChannelMemberRole::Admin);

    let refused_thread = server
        .call_tool(
            &reader,
            "create_thread",
            &json!({ "channel_id": channel_id, "title": "nope" }),
        )
        .await;
    assert!(
        matches!(&refused_thread, Err(McpError::Forbidden(m)) if m.contains("workspace:write")),
        "a caller without workspace:write must be refused, got {refused_thread:?}"
    );

    let thread = body(
        &server
            .call_tool(
                &writer,
                "create_thread",
                &json!({ "channel_id": channel_id, "title": "do the work" }),
            )
            .await
            .expect("workspace:write creates a thread"),
    );
    assert_eq!(thread["title"], "do the work");
    assert_eq!(thread["channel_id"], channel_id.to_string());
    let thread_id = thread["id"].as_str().unwrap();
    let stored = store
        .get_thread(ThreadId(thread_id.parse().unwrap()))
        .await
        .unwrap();
    assert_eq!(stored.channel_id, ChannelId(channel_id));

    let events = store.list_events_after(workspace_id, 0, 50).await.unwrap();
    assert!(
        events.iter().any(|e| e.kind == EventKind::ChannelCreated),
        "create_channel appends ChannelCreated"
    );
    assert!(
        events.iter().any(|e| e.kind == EventKind::ThreadCreated),
        "create_thread appends ThreadCreated"
    );

    let other = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap();
    let foreign = server
        .call_tool(
            &writer,
            "create_channel",
            &json!({ "workspace_id": other.id.0, "name": "stolen" }),
        )
        .await;
    assert!(
        matches!(&foreign, Err(McpError::Forbidden(_))),
        "a token cannot create a channel in another workspace, got {foreign:?}"
    );
    assert!(store.list_channels(other.id).await.unwrap().is_empty());
}
