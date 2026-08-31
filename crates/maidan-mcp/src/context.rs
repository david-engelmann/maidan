//! MCP context export (Cluster 74) — mirrors HTTP context packs; pagination Cluster 82.

use std::collections::{HashMap, HashSet};

use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::McpError;

/// Thread + message references for a context pack, batched (Cluster 335): one
/// thread read + one `src_id = ANY` read across all messages, replacing the
/// per-message N+1. Ordered by `created_at`, deduped — mirrors the REST assembler.
async fn collect_references(
    store: &dyn Store,
    thread_id: ThreadId,
    messages: &[Message],
) -> Result<Vec<Reference>, McpError> {
    let msg_ids: Vec<Uuid> = messages.iter().map(|m| m.id.0).collect();
    let mut refs = store
        .list_references_from(RefSide::Thread, thread_id.0)
        .await?;
    let mut from_msgs = store
        .list_references_from_many(RefSide::Message, &msg_ids)
        .await?;
    refs.append(&mut from_msgs);
    refs.sort_by_key(|r| r.created_at);
    refs.dedup_by_key(|r| r.id);
    Ok(refs)
}

/// Edit records for a context pack, batched across all messages then re-ordered by
/// (message position, edited_at, id) (Cluster 335). Lean by default (id/editor/
/// timestamp); `include_edits` adds the heavy before/after bodies.
async fn collect_edit_views(
    store: &dyn Store,
    messages: &[Message],
    include_edits: bool,
    cutoff: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<Value>, McpError> {
    let msg_ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
    let pos: HashMap<MessageId, usize> =
        msg_ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut edits = store.list_message_edits_for_messages(&msg_ids, 20).await?;
    if let Some(c) = cutoff {
        edits.retain(|e| e.edited_at <= c);
    }
    edits.sort_by_key(|e| {
        (
            pos.get(&e.message_id).copied().unwrap_or(usize::MAX),
            e.edited_at,
            e.id,
        )
    });
    let mut out = Vec::with_capacity(edits.len());
    for edit in edits {
        if include_edits {
            out.push(serde_json::to_value(&edit)?);
        } else {
            out.push(json!({
                "id": edit.id,
                "message_id": edit.message_id,
                "editor_id": edit.editor_id,
                "edited_at": edit.edited_at,
            }));
        }
    }
    Ok(out)
}

/// Non-tombstoned artifacts referenced by a page's messages' metadata (Cluster
/// 335 — MCP context packs previously omitted artifacts entirely). Ordered by
/// `created_at`; a missing/tombstoned blob is skipped.
async fn collect_artifacts(store: &dyn Store, messages: &[Message]) -> Vec<Artifact> {
    let mut shas = HashSet::new();
    for m in messages {
        for sha in artifact_shas_from_metadata(&m.metadata) {
            shas.insert(sha);
        }
    }
    let mut artifacts = Vec::new();
    for sha in shas {
        if let Ok(a) = store.get_artifact_by_sha(&sha).await {
            if a.tombstoned_at.is_none() {
                artifacts.push(a);
            }
        }
    }
    artifacts.sort_by_key(|a| a.created_at);
    artifacts
}

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
    /// Include the workspace glossary (canonical term definitions) so the pack is
    /// grounded in shared vocabulary (Cluster 323). Default `true`; omitted from
    /// the response when the glossary is empty. `false` drops it.
    #[serde(default = "default_true")]
    include_glossary: bool,
    /// As-of context replay (Cluster 326): reconstruct the thread as it stood at
    /// this event-log id, deterministic over the immutable log. Omit for the live
    /// pack.
    as_of: Option<i64>,
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
    /// Include the workspace glossary once at the top level (grounding, Cluster
    /// 323). Default `true`; omitted when empty. `false` drops it.
    #[serde(default = "default_true")]
    include_glossary: bool,
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
fn default_true() -> bool {
    true
}

pub async fn get_thread_context(store: &dyn Store, args: &Value) -> Result<Value, McpError> {
    let a: ThreadContextArgs =
        serde_json::from_value(args.clone()).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    if let Some(as_of) = a.as_of {
        return get_thread_context_as_of(store, &a, as_of).await;
    }
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
    // Cluster 335: batched refs/edits (no per-message N+1) + artifacts (previously
    // omitted from MCP packs) — shared with the REST assembler's behavior.
    let references = collect_references(store, thread_id, &messages).await?;
    let message_edits = collect_edit_views(store, &messages, a.include_edits, None).await?;
    let artifacts = collect_artifacts(store, &messages).await;

    let mut out = json!({
        "workspace_id": channel.workspace_id.0,
        "channel_id": thread.channel_id.0,
        "thread": thread,
        "messages": messages,
        "message_edits": message_edits,
        "references": references,
        "artifacts": artifacts,
        "fsm": {
            "state": thread.state,
            "transitions": transitions,
        },
        "next_message_cursor": next_message_cursor,
    });
    // The glossary grounds the pack in the workspace's shared vocabulary (Cluster
    // 323). Attached only when present + requested, so an empty glossary costs no
    // tokens and a workspace-context pack (which carries it once at the top) can
    // suppress it per nested thread.
    if a.include_glossary {
        let glossary = store.list_glossary_terms(channel.workspace_id).await?;
        if !glossary.is_empty() {
            out["glossary"] = serde_json::to_value(&glossary)?;
        }
    }
    Ok(out)
}

/// As-of context replay (Cluster 326): the MCP twin of the REST assembler's
/// `build_thread_context_as_of`. The message set is folded from the immutable
/// event log via `maidan_types::reconstruct_messages_through`; the additive
/// components are cut by the anchor event's time. Deterministic; no fresh search;
/// glossary omitted (current vocabulary, not thread history).
async fn get_thread_context_as_of(
    store: &dyn Store,
    a: &ThreadContextArgs,
    as_of: i64,
) -> Result<Value, McpError> {
    let anchor = store.get_stored_event(as_of).await?;
    let cutoff = anchor.occurred_at;
    let thread_id = ThreadId(a.thread_id);
    let thread = store.get_thread(thread_id).await?;
    let channel = store.get_channel(thread.channel_id).await?;

    let events = store.list_thread_events_through(thread_id, as_of).await?;
    let mut all = reconstruct_messages_through(&events);
    if let Some(cursor) = a.message_cursor {
        match all.iter().position(|m| m.id.0 == cursor) {
            Some(pos) => all = all.split_off(pos + 1),
            None => all.clear(),
        }
    }
    let page_limit = a.message_limit.clamp(1, 500);
    let next_message_cursor = if all.len() as i64 > page_limit {
        all.get(page_limit as usize - 1).map(|m| m.id.0.to_string())
    } else {
        None
    };
    let messages: Vec<Message> = all.into_iter().take(page_limit as usize).collect();

    // Cluster 335: batched refs/edits + artifacts (shared with the live path), then
    // cut to the anchor's time — the additive components as they stood at `as_of`.
    let message_edits = collect_edit_views(store, &messages, a.include_edits, Some(cutoff)).await?;
    let mut references = collect_references(store, thread_id, &messages).await?;
    references.retain(|r| r.created_at <= cutoff);
    let mut artifacts = collect_artifacts(store, &messages).await;
    artifacts.retain(|art| art.created_at <= cutoff);

    let mut transitions = store
        .list_thread_transitions(thread_id, a.transition_limit.clamp(1, 200))
        .await?;
    transitions.retain(|t| t.occurred_at <= cutoff);
    let mut chrono = transitions.clone();
    chrono.sort_by_key(|t| t.occurred_at);
    let state = chrono
        .last()
        .map(|t| t.to_state)
        .unwrap_or(ThreadState::Open);

    Ok(json!({
        "workspace_id": channel.workspace_id.0,
        "channel_id": thread.channel_id.0,
        "as_of": as_of,
        "thread": thread,
        "messages": messages,
        "message_edits": message_edits,
        "references": references,
        "artifacts": artifacts,
        "fsm": { "state": state, "transitions": transitions },
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
                // The glossary rides the workspace level once (below); suppress it
                // per nested thread so it is not repeated N times.
                "include_glossary": false,
            }),
        )
        .await?;
        threads.push(packed);
    }
    let mut out = json!({
        "workspace": workspace,
        "channels": channels,
        "threads": threads,
        "next_thread_cursor": next_thread_cursor,
    });
    if a.include_glossary {
        let glossary = store.list_glossary_terms(workspace_id).await?;
        if !glossary.is_empty() {
            out["glossary"] = serde_json::to_value(&glossary)?;
        }
    }
    Ok(out)
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

    #[tokio::test]
    async fn context_carries_the_glossary_and_dedups_in_workspace_pack() {
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
            .create_workspace(NewWorkspace { name: "gl".into() })
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
        store
            .set_glossary_term(NewGlossaryTerm {
                workspace_id: ws.id,
                term: "LSN".into(),
                definition: "log sequence number".into(),
                aliases: vec![],
                created_by: member.id,
            })
            .await
            .unwrap();

        // Thread context includes the glossary by default…
        let ctx = get_thread_context(store.as_ref(), &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap();
        assert_eq!(ctx["glossary"].as_array().unwrap().len(), 1);
        assert_eq!(ctx["glossary"][0]["term"], "LSN");

        // …and is dropped when opted out.
        let off = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": thread.id.0, "include_glossary": false }),
        )
        .await
        .unwrap();
        assert!(off.get("glossary").is_none());

        // Workspace context carries it once at the top; nested threads don't repeat it.
        let wctx = get_workspace_context(store.as_ref(), &json!({ "workspace_id": ws.id.0 }))
            .await
            .unwrap();
        assert_eq!(wctx["glossary"].as_array().unwrap().len(), 1);
        for t in wctx["threads"].as_array().unwrap() {
            assert!(t.get("glossary").is_none());
        }
    }

    #[tokio::test]
    async fn as_of_replay_shows_the_message_body_at_that_point() {
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
            .create_workspace(NewWorkspace { name: "r".into() })
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
            })
            .await
            .unwrap();
        let (msg, posted) = store
            .post_message_with_event(
                NewMessage {
                    thread_id: thread.id,
                    author_id: member.id,
                    body: "v1".into(),
                    metadata: json!({}),
                    content: None,
                },
                None,
            )
            .await
            .unwrap();
        store
            .edit_message_with_event(
                msg.id,
                member.id,
                EditMessage {
                    body: "v2".into(),
                    metadata: json!({}),
                    content: None,
                },
                None,
            )
            .await
            .unwrap();

        // As-of the posting event: the body is the original.
        let at_post = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": thread.id.0, "as_of": posted.id }),
        )
        .await
        .unwrap();
        assert_eq!(at_post["messages"][0]["body"], "v1");
        assert_eq!(at_post["as_of"], json!(posted.id));
        assert!(at_post.get("glossary").is_none());

        // Live: the edited body.
        let live = get_thread_context(store.as_ref(), &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap();
        assert_eq!(live["messages"][0]["body"], "v2");
    }

    #[tokio::test]
    async fn context_pack_includes_artifacts() {
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
            .create_workspace(NewWorkspace { name: "a".into() })
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
            })
            .await
            .unwrap();
        let sha = "ab".repeat(32);
        store
            .upsert_artifact(NewArtifact {
                sha256: sha.clone(),
                size_bytes: 5,
                mime_type: Some("text/plain".into()),
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(member.id),
            })
            .await
            .unwrap();
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "see attachment".into(),
                metadata: json!({ "artifact_sha256": sha }),
                content: None,
            })
            .await
            .unwrap();

        // Cluster 335: the MCP context pack now surfaces referenced artifacts
        // (previously omitted — REST included them, MCP did not).
        let ctx = get_thread_context(store.as_ref(), &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap();
        let artifacts = ctx["artifacts"].as_array().expect("artifacts present");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0]["sha256"], json!(sha));
    }
}
