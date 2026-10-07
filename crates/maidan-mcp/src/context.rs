//! MCP context export. The pack is the one `maidan_store::context_pack` builds,
//! so these bytes match REST.

use maidan_store::context_pack::{
    build_thread_context, build_workspace_context, ThreadContextLimits,
};
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;
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
    #[serde(default)]
    include_edits: bool,
    #[serde(default = "default_true")]
    include_glossary: bool,
    as_of: Option<i64>,
    token_budget: Option<i64>,
    #[serde(default = "default_true")]
    include_parent_grounding: bool,
    #[serde(default = "default_true")]
    include_accepted_decisions: bool,
    max_bytes: Option<i64>,
    #[serde(default)]
    delta: bool,
    since_prefix_sha: Option<String>,
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
    #[serde(default = "default_true")]
    include_glossary: bool,
    token_budget: Option<i64>,
    max_bytes: Option<i64>,
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

fn limits_from(a: &ThreadContextArgs) -> ThreadContextLimits {
    ThreadContextLimits {
        message_limit: a.message_limit,
        transition_limit: a.transition_limit,
        message_cursor: a.message_cursor.map(MessageId),
        include_edits: a.include_edits,
        include_glossary: a.include_glossary,
        as_of: a.as_of,
        token_budget: a.token_budget,
        max_bytes: a.max_bytes,
        include_parent_grounding: a.include_parent_grounding,
        include_accepted_decisions: a.include_accepted_decisions,
    }
}

fn parse_thread_args(args: &Value) -> Result<ThreadContextArgs, McpError> {
    crate::tools::parse_args(args).map_err(|e| McpError::InvalidParams(e.to_string()))
}

/// The pack as JSON for callers that navigate fields. Key order inside a
/// `Value` is alphabetical; the wire bytes are [`render_thread_context`].
pub async fn get_thread_context(store: &dyn Store, args: &Value) -> Result<Value, McpError> {
    let rendered = render_thread_context(store, args).await?;
    serde_json::from_slice(&rendered.canonical).map_err(McpError::from)
}

/// Canonical pack bytes, and the two content parts (stable prefix, volatile tail).
pub struct RenderedPack {
    pub canonical: Vec<u8>,
    pub stable: String,
    pub tail: String,
}

pub async fn render_thread_context(
    store: &dyn Store,
    args: &Value,
) -> Result<RenderedPack, McpError> {
    let a = parse_thread_args(args)?;
    let limits = limits_from(&a);
    let thread_id = ThreadId(a.thread_id);
    let packed = build_thread_context(store, thread_id, limits).await?;
    let want_delta = a.delta || a.since_prefix_sha.is_some();
    if want_delta {
        let full = if limits.message_cursor.is_some() {
            let mut unpaged = limits;
            unpaged.message_cursor = None;
            build_thread_context(store, thread_id, unpaged).await?
        } else {
            packed.clone()
        };
        let delta = frame_delta(
            &full,
            &packed,
            a.since_prefix_sha.as_deref(),
            limits.message_cursor,
        );
        let (stable, tail) = delta.parts()?;
        let canonical = delta.canonical_bytes()?;
        return Ok(RenderedPack {
            canonical,
            stable,
            tail,
        });
    }
    Ok(RenderedPack {
        canonical: packed.canonical_bytes()?,
        stable: packed.prefix_json()?,
        tail: packed.tail_json()?,
    })
}

pub async fn load_workspace_context(
    store: &dyn Store,
    args: &Value,
) -> Result<WorkspaceContext, McpError> {
    let a: WorkspaceContextArgs =
        crate::tools::parse_args(args).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let message_limit = if a.message_limit > 0 {
        a.message_limit
    } else {
        default_message_limit()
    };
    let transition_limit = if a.transition_limit > 0 {
        a.transition_limit
    } else {
        default_transition_limit()
    };
    build_workspace_context(
        store,
        WorkspaceId(a.workspace_id),
        a.thread_limit,
        a.thread_cursor.map(ThreadId),
        ThreadContextLimits {
            message_limit,
            transition_limit,
            message_cursor: None,
            include_edits: false,
            include_glossary: a.include_glossary,
            as_of: None,
            token_budget: a.token_budget,
            max_bytes: a.max_bytes,
            include_parent_grounding: false,
            include_accepted_decisions: false,
        },
    )
    .await
    .map_err(McpError::from)
}

pub async fn get_workspace_context(store: &dyn Store, args: &Value) -> Result<Value, McpError> {
    let packed = load_workspace_context(store, args).await?;
    Ok(serde_json::to_value(&packed)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use serde_json::json;
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));

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
                description: None,
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
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
                description: None,
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
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
                description: None,
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
        assert_eq!(at_post["state"], "open");

        store
            .transition_thread(thread.id, member.id, maidan_fsm::ThreadAction::StartReview)
            .await
            .unwrap();
        let after = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": thread.id.0, "as_of": posted.id }),
        )
        .await
        .unwrap();
        assert_eq!(
            after["state"], "open",
            "as_of rebuilds the row, not the live one"
        );
        assert_eq!(after["messages"][0]["body"], "v1");

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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
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
                description: None,
            })
            .await
            .unwrap();
        let sha = "ab".repeat(32);
        store
            .upsert_artifact(NewArtifact {
                sha256: sha.clone(),
                size_bytes: 5,
                mime_type: Some("text/plain".into()),
                filename: None,
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

        // The MCP context pack now surfaces referenced artifacts (previously
        // omitted — REST included them, MCP did not).
        let ctx = get_thread_context(store.as_ref(), &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap();
        let artifacts = ctx["artifacts"].as_array().expect("artifacts present");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0]["sha256"], json!(sha));
    }

    #[tokio::test]
    async fn token_budget_folds_the_pack_and_surfaces_elision() {
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
        let ws = store
            .create_workspace(NewWorkspace { name: "b".into() })
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
                description: None,
            })
            .await
            .unwrap();

        const N: usize = 6;
        for i in 0..N {
            store
                .post_message(NewMessage {
                    thread_id: thread.id,
                    author_id: member.id,
                    body: format!("m{i}-{}", (b'a' + i as u8) as char).repeat(120),
                    metadata: json!({}),
                    content: None,
                })
                .await
                .unwrap();
        }

        // Tight budget → opener + recent tail kept, middle elided, marker present.
        let folded = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": thread.id.0, "token_budget": 400 }),
        )
        .await
        .unwrap();
        let kept = folded["messages"].as_array().unwrap();
        assert!(kept.len() >= 2 && kept.len() < N);
        assert!(kept.first().unwrap()["body"]
            .as_str()
            .unwrap()
            .starts_with("m0-a"));
        assert!(kept.last().unwrap()["body"]
            .as_str()
            .unwrap()
            .starts_with("m5-f"));
        let elision = &folded["elision"];
        assert!(!elision.is_null());
        assert_eq!(
            elision["elided_message_count"].as_u64().unwrap() as usize,
            N - kept.len()
        );
        assert!(elision["summary"].as_str().unwrap().contains("elided"));

        // No budget → all messages, no elision.
        let full = get_thread_context(store.as_ref(), &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap();
        assert_eq!(full["messages"].as_array().unwrap().len(), N);
        assert!(full.get("elision").is_none());
    }

    #[tokio::test]
    async fn child_thread_pack_carries_parent_grounding() {
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
        let ws = store
            .create_workspace(NewWorkspace { name: "g".into() })
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
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let parent = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("parent task".into()),
                description: None,
            })
            .await
            .unwrap();
        store
            .post_message(NewMessage {
                thread_id: parent.id,
                author_id: member.id,
                body: "build the widget".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        store
            .set_thread_result(parent.id, member.id, &json!({"decision": "approved"}))
            .await
            .unwrap();
        let child = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: Some(parent.id),
                title: Some("child subtask".into()),
                description: None,
            })
            .await
            .unwrap();
        store
            .post_message(NewMessage {
                thread_id: child.id,
                author_id: member.id,
                body: "working on it".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();

        // Child pack grounds on the parent by default.
        let ctx = get_thread_context(store.as_ref(), &json!({ "thread_id": child.id.0 }))
            .await
            .unwrap();
        let g = &ctx["parent_grounding"];
        assert_eq!(g["thread_id"], json!(parent.id.0));
        assert_eq!(g["title"], "parent task");
        assert_eq!(g["opening_message"]["body"], "build the widget");
        assert_eq!(g["latest_result"], json!({"decision": "approved"}));

        // Opt-out drops it.
        let off = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": child.id.0, "include_parent_grounding": false }),
        )
        .await
        .unwrap();
        assert!(off.get("parent_grounding").is_none());

        // A root thread never grounds.
        let root = get_thread_context(store.as_ref(), &json!({ "thread_id": parent.id.0 }))
            .await
            .unwrap();
        assert!(root.get("parent_grounding").is_none());
    }

    #[tokio::test]
    async fn thread_pack_lists_in_channel_accepted_decisions() {
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
        let ws = store
            .create_workspace(NewWorkspace { name: "d".into() })
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
                name: "tasks".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();

        async fn close(store: &dyn Store, id: ThreadId, actor: MemberId) {
            store
                .transition_thread(id, actor, maidan_fsm::ThreadAction::StartReview)
                .await
                .unwrap();
            store
                .transition_thread(id, actor, maidan_fsm::ThreadAction::Close)
                .await
                .unwrap();
        }

        let opaque = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("opaque decision".into()),
                description: None,
            })
            .await
            .unwrap();
        store
            .set_thread_result(opaque.id, member.id, &json!({"decision": "use postgres"}))
            .await
            .unwrap();
        close(store.as_ref(), opaque.id, member.id).await;

        let failed = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("failed waiter".into()),
                description: None,
            })
            .await
            .unwrap();
        store
            .set_thread_result(
                failed.id,
                member.id,
                &json!({
                    "schema": WAITER_RESULT_SCHEMA,
                    "result_kind": "example.review.result/1",
                    "status": "failed",
                    "summary": "tests red",
                }),
            )
            .await
            .unwrap();
        close(store.as_ref(), failed.id, member.id).await;

        let reviewed = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("reviewed waiter".into()),
                description: None,
            })
            .await
            .unwrap();
        store
            .set_thread_result(
                reviewed.id,
                member.id,
                &json!({
                    "schema": WAITER_RESULT_SCHEMA,
                    "result_kind": "example.review.result/1",
                    "status": STATUS_REVIEWED,
                    "summary": "lgtm",
                    "rendered": "# must not inline",
                }),
            )
            .await
            .unwrap();
        close(store.as_ref(), reviewed.id, member.id).await;

        let claimer = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("claimer task".into()),
                description: None,
            })
            .await
            .unwrap();

        let ctx = get_thread_context(store.as_ref(), &json!({ "thread_id": claimer.id.0 }))
            .await
            .unwrap();
        let decisions = ctx["accepted_decisions"]
            .as_array()
            .expect("accepted_decisions present by default");
        let titles: Vec<&str> = decisions
            .iter()
            .filter_map(|d| d["title"].as_str())
            .collect();
        assert!(titles.contains(&"opaque decision"));
        assert!(titles.contains(&"reviewed waiter"));
        assert!(!titles.contains(&"failed waiter"));
        let reviewed_row = decisions
            .iter()
            .find(|d| d["title"] == "reviewed waiter")
            .unwrap();
        assert_eq!(reviewed_row["result_kind"], "example.review.result/1");
        assert_eq!(reviewed_row["status"], STATUS_REVIEWED);
        assert_eq!(reviewed_row["summary"], "lgtm");
        assert!(reviewed_row.get("result").is_none());
        assert!(reviewed_row.get("rendered").is_none());

        let off = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": claimer.id.0, "include_accepted_decisions": false }),
        )
        .await
        .unwrap();
        assert!(off.get("accepted_decisions").is_none());

        let wctx = get_workspace_context(store.as_ref(), &json!({ "workspace_id": ws.id.0 }))
            .await
            .unwrap();
        for t in wctx["threads"].as_array().unwrap() {
            assert!(t.get("accepted_decisions").is_none());
        }

        let bob = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "b".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let carol = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "c".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let dm_closed = store
            .open_dm_conversation(ws.id, member.id, bob.id)
            .await
            .unwrap();
        store
            .set_thread_result(
                dm_closed.thread_id,
                member.id,
                &json!({"decision": "secret"}),
            )
            .await
            .unwrap();
        close(store.as_ref(), dm_closed.thread_id, member.id).await;
        let dm_other = store
            .open_dm_conversation(ws.id, member.id, carol.id)
            .await
            .unwrap();
        let dm_pack = get_thread_context(
            store.as_ref(),
            &json!({ "thread_id": dm_other.thread_id.0 }),
        )
        .await
        .unwrap();
        assert!(dm_pack.get("accepted_decisions").is_none());
    }
}
