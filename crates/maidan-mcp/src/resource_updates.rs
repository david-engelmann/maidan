//! Map mutations to the `maidan://` resource URIs they touched, each scoped to
//! the workspace the mutation happened in, for subscription fan-out.

use std::collections::HashSet;

use maidan_bus::ResourceUpdate;
use maidan_router::resolve_thread_context;
use maidan_store::Store;
use maidan_types::*;
use serde_json::Value;

/// `caller_workspace` scopes what only the caller's workspace can say it
/// touched: an artifact is content-addressed and shared across workspaces, so
/// an upload is news only to the workspace it was uploaded into.
pub async fn uris_for_tool_mutation(
    store: &dyn Store,
    caller_workspace: WorkspaceId,
    tool_name: &str,
    args: &Value,
    result: &Value,
) -> Vec<ResourceUpdate> {
    let mut uris = HashSet::new();
    match tool_name {
        "post_message" => {
            if let Some(tid) = uuid_arg(args, "thread_id") {
                push_thread_chain(store, ThreadId(tid), &mut uris).await;
            }
            if let Some(body) = tool_result_json(result) {
                if let Ok(msg) = serde_json::from_value::<Message>(body) {
                    push_thread_chain(store, msg.thread_id, &mut uris).await;
                }
            }
        }
        "edit_message" => {
            if let Some(mid) = uuid_arg(args, "message_id") {
                if let Ok(msg) = store.get_message(MessageId(mid)).await {
                    push_thread_chain(store, msg.thread_id, &mut uris).await;
                }
            }
            if let Some(body) = tool_result_json(result) {
                if let Ok(msg) = serde_json::from_value::<Message>(body) {
                    push_thread_chain(store, msg.thread_id, &mut uris).await;
                }
            }
        }
        "upload_artifact" | "complete_artifact_multipart" => {
            if let Some(body) = tool_result_json(result) {
                if let Some(sha) = body.get("sha256").and_then(|v| v.as_str()) {
                    uris.insert(ResourceUpdate::new(
                        caller_workspace,
                        format!("maidan://artifacts/{sha}"),
                    ));
                }
            }
        }
        "record_mention" | "cast_vote" | "retract_vote" | "add_reaction" | "remove_reaction" => {
            if let Some(mid) = uuid_arg(args, "message_id") {
                if let Ok(msg) = store.get_message(MessageId(mid)).await {
                    push_thread_chain(store, msg.thread_id, &mut uris).await;
                }
            }
        }
        "pin_message" | "unpin_message" | "transition_thread" => {
            if let Some(tid) = uuid_arg(args, "thread_id") {
                push_thread_chain(store, ThreadId(tid), &mut uris).await;
            }
        }
        "add_reference" => {
            push_ref_side(store, args, "src_kind", "src_id", &mut uris).await;
            push_ref_side(store, args, "dst_kind", "dst_id", &mut uris).await;
        }
        "open_dm_conversation" | "post_dm_message" => {
            if let Some(body) = tool_result_json(result) {
                if let Ok(dm) = serde_json::from_value::<DmConversation>(body.clone()) {
                    push_thread_chain(store, dm.thread_id, &mut uris).await;
                } else if let Ok(msg) = serde_json::from_value::<Message>(body) {
                    push_thread_chain(store, msg.thread_id, &mut uris).await;
                }
            }
        }
        _ => {}
    }
    uris.into_iter().collect()
}

/// URIs to notify after HTTP message tombstone or other message-scoped mutations.
pub async fn uris_for_message(store: &dyn Store, message_id: MessageId) -> Vec<ResourceUpdate> {
    let mut uris = HashSet::new();
    if let Ok(msg) = store.get_message(message_id).await {
        push_thread_chain(store, msg.thread_id, &mut uris).await;
    }
    uris.into_iter().collect()
}

/// URIs to notify after HTTP message tombstone.
pub async fn uris_for_message_tombstone(
    store: &dyn Store,
    message_id: MessageId,
) -> Vec<ResourceUpdate> {
    uris_for_message(store, message_id).await
}

/// URIs to notify after workspace deep purge.
pub fn uris_for_workspace_purge(workspace_id: WorkspaceId) -> Vec<ResourceUpdate> {
    vec![ResourceUpdate::new(
        workspace_id,
        format!("maidan://workspaces/{}", workspace_id.0),
    )]
}

/// URIs to notify after thread FSM transition.
pub async fn uris_for_thread_transition(
    store: &dyn Store,
    thread_id: ThreadId,
) -> Vec<ResourceUpdate> {
    let mut uris = HashSet::new();
    push_thread_chain(store, thread_id, &mut uris).await;
    uris.into_iter().collect()
}

/// A thread whose workspace cannot be resolved notifies nobody: an update
/// with no workspace has no audience it may reach.
async fn push_thread_chain(
    store: &dyn Store,
    thread_id: ThreadId,
    uris: &mut HashSet<ResourceUpdate>,
) {
    let Ok(ctx) = resolve_thread_context(store, thread_id).await else {
        return;
    };
    let ws = ctx.workspace_id;
    uris.insert(ResourceUpdate::new(
        ws,
        format!("maidan://threads/{}", thread_id.0),
    ));
    uris.insert(ResourceUpdate::new(
        ws,
        format!("maidan://channels/{}", ctx.channel_id.0),
    ));
    uris.insert(ResourceUpdate::new(
        ws,
        format!("maidan://workspaces/{}", ws.0),
    ));
}

async fn push_ref_side(
    store: &dyn Store,
    args: &Value,
    kind_key: &str,
    id_key: &str,
    uris: &mut HashSet<ResourceUpdate>,
) {
    let Some(kind) = args.get(kind_key).and_then(|v| v.as_str()) else {
        return;
    };
    let Some(id) = uuid_arg(args, id_key) else {
        return;
    };
    match kind {
        "thread" => {
            push_thread_chain(store, ThreadId(id), uris).await;
        }
        "message" => {
            if let Ok(msg) = store.get_message(MessageId(id)).await {
                push_thread_chain(store, msg.thread_id, uris).await;
            }
        }
        _ => {}
    }
}

fn uuid_arg(args: &Value, key: &str) -> Option<uuid::Uuid> {
    let raw = args.get(key).and_then(|v| v.as_str())?;
    uuid::Uuid::parse_str(raw).ok()
}

fn tool_result_json(result: &Value) -> Option<Value> {
    let text = result
        .get("content")?
        .as_array()?
        .first()?
        .get("text")?
        .as_str()?;
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    async fn store() -> Arc<dyn Store> {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        Arc::new(SqliteStore::for_tests(pool))
    }

    #[tokio::test]
    async fn workspace_purge_uri_targets_workspace() {
        let ws_id = WorkspaceId(uuid::Uuid::new_v4());
        let uris = uris_for_workspace_purge(ws_id);
        assert_eq!(
            uris,
            vec![ResourceUpdate::new(
                ws_id,
                format!("maidan://workspaces/{}", ws_id.0)
            )]
        );
    }

    #[tokio::test]
    async fn post_message_includes_thread_channel_workspace_uris() {
        let store = store().await;
        let ws = store
            .create_workspace(NewWorkspace {
                name: "fanout".into(),
            })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let ch = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: ch.id,
                parent_thread_id: None,
                title: None,
                description: None,
            })
            .await
            .unwrap();
        let msg = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "hi".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        let result = json!({
            "content": [{ "type": "text", "text": serde_json::to_string(&msg).unwrap() }],
            "isError": false
        });
        let args = json!({
            "thread_id": thread.id.0,
            "author_id": member.id.0,
            "body": "hi"
        });
        let uris =
            uris_for_tool_mutation(store.as_ref(), ws.id, "post_message", &args, &result).await;
        for uri in [
            format!("maidan://threads/{}", thread.id.0),
            format!("maidan://channels/{}", ch.id.0),
            format!("maidan://workspaces/{}", ws.id.0),
        ] {
            assert!(uris.contains(&ResourceUpdate::new(ws.id, uri)));
        }
    }

    #[tokio::test]
    async fn an_artifact_upload_is_scoped_to_the_uploading_workspace() {
        let store = store().await;
        let uploader = WorkspaceId(uuid::Uuid::new_v4());
        let sha = "a".repeat(64);
        let result = json!({
            "content": [{ "type": "text", "text": json!({ "sha256": sha }).to_string() }],
            "isError": false
        });
        let uris = uris_for_tool_mutation(
            store.as_ref(),
            uploader,
            "upload_artifact",
            &json!({}),
            &result,
        )
        .await;
        assert_eq!(
            uris,
            vec![ResourceUpdate::new(
                uploader,
                format!("maidan://artifacts/{sha}")
            )]
        );
    }
}
