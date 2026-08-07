//! MCP context export (Cluster 74) — mirrors HTTP context packs; pagination Cluster 82.

use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::McpError;

#[derive(Debug, Deserialize)]
struct ThreadContextArgs {
    thread_id: Uuid,
    #[serde(default = "default_message_limit")]
    message_limit: i64,
    #[serde(default = "default_transition_limit")]
    transition_limit: i64,
    message_cursor: Option<Uuid>,
    /// Include full `body_before`/`body_after` on each edit. Default `false`:
    /// edits carry only `{id, message_id, editor_id, edited_at}` — the
    /// was-edited/when/by-whom signal without the two heavy body copies. The
    /// bodies are the single largest token cost of a context pack.
    #[serde(default)]
    include_edits: bool,
}

#[derive(Debug, Deserialize)]
struct WorkspaceContextArgs {
    workspace_id: Uuid,
    #[serde(default = "default_thread_limit")]
    thread_limit: i64,
    #[serde(default = "default_message_limit")]
    message_limit: i64,
    #[serde(default = "default_transition_limit")]
    transition_limit: i64,
    thread_cursor: Option<Uuid>,
}

fn default_message_limit() -> i64 {
    100
}
fn default_transition_limit() -> i64 {
    50
}
fn default_thread_limit() -> i64 {
    10
}

pub async fn get_thread_context(store: &dyn Store, args: &Value) -> Result<Value, McpError> {
    let a: ThreadContextArgs =
        serde_json::from_value(args.clone()).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let thread_id = ThreadId(a.thread_id);
    let thread = store.get_thread(thread_id).await?;
    if thread.tombstoned_at.is_some() {
        return Err(McpError::InvalidParams("thread is tombstoned".into()));
    }
    let channel = store.get_channel(thread.channel_id).await?;
    let page_limit = a.message_limit.clamp(1, 500);
    let messages = store
        .list_messages_after(thread_id, a.message_cursor.map(MessageId), page_limit + 1)
        .await?;
    let next_message_cursor = if messages.len() as i64 > page_limit {
        messages
            .get(page_limit as usize - 1)
            .map(|m| m.id.0.to_string())
    } else {
        None
    };
    let messages: Vec<Message> = messages.into_iter().take(page_limit as usize).collect();
    let transitions = store
        .list_thread_transitions(thread_id, a.transition_limit.clamp(1, 200))
        .await?;
    let mut references = store
        .list_references_from(RefSide::Thread, thread_id.0)
        .await?;
    for message in &messages {
        let mut from_message = store
            .list_references_from(RefSide::Message, message.id.0)
            .await?;
        references.append(&mut from_message);
    }
    references.sort_by_key(|r| r.created_at);
    references.dedup_by_key(|r| r.id);

    let mut message_edits = Vec::new();
    for message in &messages {
        let edits = store.list_message_edits(message.id, 20).await?;
        for edit in edits {
            if a.include_edits {
                message_edits.push(serde_json::to_value(&edit)?);
            } else {
                message_edits.push(json!({
                    "id": edit.id,
                    "message_id": edit.message_id,
                    "editor_id": edit.editor_id,
                    "edited_at": edit.edited_at,
                }));
            }
        }
    }

    Ok(json!({
        "workspace_id": channel.workspace_id.0,
        "channel_id": thread.channel_id.0,
        "thread": thread,
        "messages": messages,
        "message_edits": message_edits,
        "references": references,
        "fsm": {
            "state": thread.state,
            "transitions": transitions,
        },
        "next_message_cursor": next_message_cursor,
    }))
}

pub async fn get_workspace_context(store: &dyn Store, args: &Value) -> Result<Value, McpError> {
    let a: WorkspaceContextArgs =
        serde_json::from_value(args.clone()).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let workspace_id = WorkspaceId(a.workspace_id);
    let workspace = store.get_workspace(workspace_id).await?;
    let channels = store.list_channels(workspace_id).await?;
    let page_limit = a.thread_limit.clamp(1, 50);
    let mut ordered = Vec::new();
    for channel in &channels {
        for thread in store.list_threads(channel.id).await? {
            if thread.tombstoned_at.is_none() {
                ordered.push(thread);
            }
        }
    }
    ordered.sort_by(|x, y| {
        x.created_at
            .cmp(&y.created_at)
            .then_with(|| x.id.0.cmp(&y.id.0))
    });
    let start = a
        .thread_cursor
        .map(|cursor| {
            ordered
                .iter()
                .position(|t| t.id.0 == cursor)
                .map(|i| i + 1)
                .unwrap_or(ordered.len())
        })
        .unwrap_or(0);
    let slice: Vec<Thread> = ordered
        .into_iter()
        .skip(start)
        .take(page_limit as usize + 1)
        .collect();
    let next_thread_cursor = if slice.len() > page_limit as usize {
        slice
            .get(page_limit as usize - 1)
            .map(|t| t.id.0.to_string())
    } else {
        None
    };
    let mut threads = Vec::new();
    for thread in slice.into_iter().take(page_limit as usize) {
        let packed = get_thread_context(
            store,
            &json!({
                "thread_id": thread.id.0,
                "message_limit": a.message_limit,
                "transition_limit": a.transition_limit,
            }),
        )
        .await?;
        threads.push(packed);
    }
    Ok(json!({
        "workspace": workspace,
        "channels": channels,
        "threads": threads,
        "next_thread_cursor": next_thread_cursor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    /// Store with a single message that has been edited once, so
    /// `get_thread_context` has exactly one `MessageEdit` to render.
    async fn store_with_one_edit() -> (Arc<dyn Store>, ThreadId) {
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));

        let ws = store
            .create_workspace(NewWorkspace {
                name: "edits-ws".into(),
            })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "alice".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let channel = store
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
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        let msg = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "original body".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        store
            .edit_message(
                msg.id,
                member.id,
                EditMessage {
                    body: "edited body".into(),
                    metadata: json!({}),
                    content: None,
                },
            )
            .await
            .unwrap();
        (store, thread.id)
    }

    #[tokio::test]
    async fn thread_context_omits_edit_bodies_by_default() {
        let (store, thread_id) = store_with_one_edit().await;
        let ctx = get_thread_context(store.as_ref(), &json!({ "thread_id": thread_id.0 }))
            .await
            .unwrap();
        let edits = ctx["message_edits"].as_array().unwrap();
        assert_eq!(edits.len(), 1);
        // The signal survives…
        assert!(edits[0].get("editor_id").is_some());
        assert!(edits[0].get("edited_at").is_some());
        // …but the two heavy body copies are elided by default.
        assert!(edits[0].get("body_before").is_none());
        assert!(edits[0].get("body_after").is_none());
    }

    #[tokio::test]
    async fn thread_context_include_edits_returns_full_bodies() {
        let (store, thread_id) = store_with_one_edit().await;
        let ctx = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": thread_id.0, "include_edits": true }),
        )
        .await
        .unwrap();
        let edits = ctx["message_edits"].as_array().unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["body_before"], "original body");
        assert_eq!(edits[0]["body_after"], "edited body");
    }
}
