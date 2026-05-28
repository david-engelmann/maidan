//! Map MCP tool mutations to `maidan://` resource URIs for subscription fan-out.

use std::collections::HashSet;

use maidan_router::resolve_thread_context;
use maidan_store::Store;
use maidan_types::*;
use serde_json::Value;

pub async fn uris_for_tool_mutation(
    store: &dyn Store,
    tool_name: &str,
    args: &Value,
    result: &Value,
) -> Vec<String> {
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
        "upload_artifact" | "complete_artifact_multipart" => {
            if let Some(body) = tool_result_json(result) {
                if let Some(sha) = body.get("sha256").and_then(|v| v.as_str()) {
                    uris.insert(format!("maidan://artifacts/{sha}"));
                }
            }
        }
        "record_mention" | "cast_vote" => {
            if let Some(mid) = uuid_arg(args, "message_id") {
                if let Ok(msg) = store.get_message(MessageId(mid)).await {
                    push_thread_chain(store, msg.thread_id, &mut uris).await;
                }
            }
        }
        "add_reference" => {
            push_ref_side(store, args, "src_kind", "src_id", &mut uris).await;
            push_ref_side(store, args, "dst_kind", "dst_id", &mut uris).await;
        }
        _ => {}
    }
    uris.into_iter().collect()
}

async fn push_thread_chain(store: &dyn Store, thread_id: ThreadId, uris: &mut HashSet<String>) {
    uris.insert(format!("maidan://threads/{}", thread_id.0));
    let Ok(ctx) = resolve_thread_context(store, thread_id).await else {
        return;
    };
    uris.insert(format!("maidan://channels/{}", ctx.channel_id.0));
    uris.insert(format!("maidan://workspaces/{}", ctx.workspace_id.0));
}

async fn push_ref_side(
    store: &dyn Store,
    args: &Value,
    kind_key: &str,
    id_key: &str,
    uris: &mut HashSet<String>,
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
        Arc::new(SqliteStore::new(pool))
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
            })
            .await
            .unwrap();
        let msg = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "hi".into(),
                metadata: json!({}),
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
        let uris = uris_for_tool_mutation(store.as_ref(), "post_message", &args, &result).await;
        assert!(uris.contains(&format!("maidan://threads/{}", thread.id.0)));
        assert!(uris.contains(&format!("maidan://channels/{}", ch.id.0)));
        assert!(uris.contains(&format!("maidan://workspaces/{}", ws.id.0)));
    }
}
