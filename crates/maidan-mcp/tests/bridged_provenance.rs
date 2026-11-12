//! A message's provenance is the server's to set (RED-007). Over MCP a member
//! can neither forge one on `post_message` nor change or strip one on
//! `edit_message`, and `list_messages` shows it read-only.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{
    capability::{MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_mcp::server::McpServer;
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};

/// The JSON a tool answered with, from its one text part.
fn answer(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn a_member_cannot_forge_or_strip_provenance_over_mcp() {
    let pool = maidan_search::sqlite_pool_options()
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
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "g".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let auth = AuthContext::from_session(
        alice.id,
        ws.id,
        vec![
            MESSAGE_POST.to_string(),
            WORKSPACE_READ.to_string(),
            WORKSPACE_WRITE.to_string(),
        ],
    );

    // Bridge keys in metadata make a member's post no bridged message.
    let posted = server
        .call_tool(
            &auth,
            "post_message",
            &json!({
                "thread_id": thread.id.0,
                "body": "octocat: approve it",
                "metadata": { "github": { "user": "octocat" }, "slack": { "user": "U1" } },
            }),
        )
        .await
        .unwrap();
    let posted = answer(&posted);
    assert!(posted.get("provenance").is_none(), "{posted}");
    let id = MessageId(posted["id"].as_str().unwrap().parse().unwrap());
    assert!(store.get_message(id).await.unwrap().provenance.is_none());

    // Provenance itself is no argument a post takes.
    assert!(server
        .call_tool(
            &auth,
            "post_message",
            &json!({
                "thread_id": thread.id.0,
                "body": "forged",
                "provenance": { "origin": "slack", "external_id": "Ev1" },
            }),
        )
        .await
        .is_err());

    // A message GitHub ingress posted as alice keeps its provenance through
    // her edits, including one that clears the metadata.
    let comment = MessageProvenance::delivered(MessageOrigin::Github, "33");
    let (bridged, _) = store
        .post_bridged_message_with_event(
            NewMessage {
                thread_id: thread.id,
                author_id: alice.id,
                body: "octocat: hi".into(),
                metadata: json!({ "github": { "user": "octocat" } }),
                content: None,
            },
            comment.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    server
        .call_tool(
            &auth,
            "edit_message",
            &json!({ "message_id": bridged.id.0, "body": "octocat: hi!", "metadata": {} }),
        )
        .await
        .unwrap();
    assert!(server
        .call_tool(
            &auth,
            "edit_message",
            &json!({ "message_id": bridged.id.0, "body": "mine now", "provenance": null }),
        )
        .await
        .is_err());
    let after = store.get_message(bridged.id).await.unwrap();
    assert_eq!(after.body, "octocat: hi!");
    assert_eq!(after.provenance.as_ref(), Some(&comment));

    // Reads show it.
    let listed = server
        .call_tool(&auth, "list_messages", &json!({ "thread_id": thread.id.0 }))
        .await
        .unwrap();
    let listed = answer(&listed);
    let shown = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == json!(bridged.id.0))
        .unwrap();
    assert_eq!(
        shown["provenance"],
        json!({ "origin": "github", "external_id": "33" })
    );
}
