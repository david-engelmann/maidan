//! Tombstone explorer, message backlinks, and EventKind census (Cluster 394.3).
//!
//! Twins of REST `GET /workspaces/:id/tombstones`, `GET /messages/:id/backlinks`,
//! and `GET /workspaces/:id/kind-census`. All three are `workspace:read`.
//! Optional `channel_id` / `thread_id` on the workspace-scoped tools are gated
//! pre-dispatch; the tombstone list still post-filters by `can_access_thread`
//! because the store cannot see DM participation. Census applies
//! `private_channel_deny_set` in the query.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{clamp_tombstone_limit, ChannelId, MessageId, ThreadId, WorkspaceId};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct ListTombstonesArgs {
    #[serde(default)]
    workspace_id: Option<Uuid>,
    #[serde(default)]
    channel_id: Option<Uuid>,
    #[serde(default)]
    thread_id: Option<Uuid>,
    #[serde(default)]
    include_purged: bool,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct ListMessageBacklinksArgs {
    message_id: Uuid,
}

#[derive(Deserialize)]
struct KindCensusArgs {
    #[serde(default)]
    workspace_id: Option<Uuid>,
    #[serde(default)]
    channel_id: Option<Uuid>,
    #[serde(default)]
    thread_id: Option<Uuid>,
}

fn workspace(auth: &AuthContext, id: Option<Uuid>) -> Result<WorkspaceId, McpError> {
    let workspace_id = WorkspaceId(id.unwrap_or(auth.workspace_id.0));
    auth.ensure_workspace(workspace_id)?;
    Ok(workspace_id)
}

pub(super) async fn list_tombstones(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListTombstonesArgs = serde_json::from_value(args.clone())?;
    let workspace_id = workspace(auth, a.workspace_id)?;
    store.get_workspace(workspace_id).await?;
    let limit = clamp_tombstone_limit(a.limit);
    let rows = store
        .list_tombstones(
            workspace_id,
            a.channel_id.map(ChannelId),
            a.thread_id.map(ThreadId),
            a.include_purged,
            limit,
        )
        .await?;
    if auth.bypass {
        return Ok(content_json(&rows));
    }
    let mut visible = Vec::with_capacity(rows.len());
    for row in rows {
        if maidan_auth::can_access_thread(store.as_ref(), auth, row.thread_id).await? {
            visible.push(row);
        }
    }
    Ok(content_json(&visible))
}

pub(super) async fn list_message_backlinks(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListMessageBacklinksArgs = serde_json::from_value(args.clone())?;
    let backlinks = store
        .list_message_backlinks(MessageId(a.message_id))
        .await?;
    Ok(content_json(&backlinks))
}

pub(super) async fn get_kind_census(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: KindCensusArgs = serde_json::from_value(args.clone())?;
    let workspace_id = workspace(auth, a.workspace_id)?;
    store.get_workspace(workspace_id).await?;
    let deny = maidan_auth::private_channel_deny_set(store.as_ref(), auth, workspace_id).await?;
    let census = store
        .event_kind_census(
            workspace_id,
            a.channel_id.map(ChannelId),
            a.thread_id.map(ThreadId),
            &deny,
        )
        .await?;
    Ok(content_json(&census))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::capability::WORKSPACE_READ;
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use maidan_types::{
        MemberKind, NewChannel, NewMember, NewMessage, NewPin, NewReaction, NewReference,
        NewThread, NewVote, NewWorkspace, RefSide, RelationKind,
    };
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::server::McpServer;

    fn content(v: &Value) -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn new_msg(
        thread_id: maidan_types::ThreadId,
        author: maidan_types::MemberId,
        body: &str,
    ) -> NewMessage {
        NewMessage {
            thread_id,
            author_id: author,
            body: body.into(),
            metadata: serde_json::json!({}),
            content: None,
        }
    }

    async fn blank_store() -> (Arc<dyn Store>, sqlx::SqlitePool) {
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
        (Arc::new(SqliteStore::new(pool.clone())), pool)
    }

    fn mcp(store: Arc<dyn Store>, pool: sqlx::SqlitePool) -> McpServer {
        McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
    }

    #[tokio::test]
    async fn explorer_tools_list_tombstones_backlinks_and_census() {
        let (store, pool) = blank_store().await;
        let (ws, _) = store
            .create_workspace_with_event(NewWorkspace { name: "ex".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "me".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let (public, _) = store
            .create_channel_with_event(NewChannel {
                workspace_id: ws.id,
                name: "pub".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let (private, _) = store
            .create_channel_with_event(NewChannel {
                workspace_id: ws.id,
                name: "priv".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        let (pub_th, _) = store
            .create_thread_with_event(NewThread {
                channel_id: public.id,
                parent_thread_id: None,
                title: Some("pub".into()),
            })
            .await
            .unwrap();
        let (priv_th, _) = store
            .create_thread_with_event(NewThread {
                channel_id: private.id,
                parent_thread_id: None,
                title: Some("secret".into()),
            })
            .await
            .unwrap();

        let (keep, _) = store
            .post_message_with_event(new_msg(pub_th.id, member.id, "keep"), None)
            .await
            .unwrap();
        let (dead, _) = store
            .post_message_with_event(new_msg(pub_th.id, member.id, "drop"), None)
            .await
            .unwrap();
        store
            .tombstone_message_with_event(dead.id, None)
            .await
            .unwrap();
        let (hidden, _) = store
            .post_message_with_event(new_msg(priv_th.id, member.id, "secret"), None)
            .await
            .unwrap();
        store
            .tombstone_message_with_event(hidden.id, None)
            .await
            .unwrap();

        let (src, _) = store
            .post_message_with_event(new_msg(pub_th.id, member.id, "src"), None)
            .await
            .unwrap();
        store
            .add_reference_with_event(NewReference {
                src_kind: RefSide::Message,
                src_id: src.id.0,
                dst_kind: RefSide::Message,
                dst_id: keep.id.0,
                relation: RelationKind::Supports,
            })
            .await
            .unwrap();
        store
            .pin_message_with_event(NewPin {
                thread_id: pub_th.id,
                message_id: keep.id,
                member_id: member.id,
            })
            .await
            .unwrap();
        store
            .add_reaction_with_event(NewReaction {
                message_id: keep.id,
                member_id: member.id,
                emoji: "👍".into(),
            })
            .await
            .unwrap();
        store
            .cast_vote_with_event(NewVote {
                message_id: keep.id,
                member_id: member.id,
                kind: "up".into(),
                confidence: None,
            })
            .await
            .unwrap();

        let server = mcp(store.clone(), pool);
        let reader = AuthContext::from_session(member.id, ws.id, vec![WORKSPACE_READ.to_string()]);

        let tombs = content(
            &server
                .call_tool(&reader, "list_tombstones", &json!({}))
                .await
                .unwrap(),
        );
        let ids: Vec<String> = tombs
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect();
        assert!(ids.contains(&dead.id.0.to_string()));
        assert!(
            !ids.contains(&hidden.id.0.to_string()),
            "private-channel tombstone must not leak"
        );
        assert!(tombs
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["retained"] == true));

        store.purge_message(dead.id).await.unwrap();
        let after = content(
            &server
                .call_tool(&reader, "list_tombstones", &json!({}))
                .await
                .unwrap(),
        );
        assert!(after
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["id"] != dead.id.0.to_string()));
        let purged = content(
            &server
                .call_tool(
                    &reader,
                    "list_tombstones",
                    &json!({ "include_purged": true }),
                )
                .await
                .unwrap(),
        );
        let purged_ids: Vec<String> = purged
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect();
        assert!(purged_ids.contains(&dead.id.0.to_string()));
        let dead_row = purged
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == dead.id.0.to_string())
            .unwrap();
        assert_eq!(dead_row["retained"], false);

        let back = content(
            &server
                .call_tool(
                    &reader,
                    "list_message_backlinks",
                    &json!({ "message_id": keep.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(back["message_id"], keep.id.0.to_string());
        assert_eq!(back["references"].as_array().unwrap().len(), 1);
        assert_eq!(back["references"][0]["src_id"], src.id.0.to_string());
        assert_eq!(back["pins"].as_array().unwrap().len(), 1);
        assert_eq!(back["reactions"].as_array().unwrap().len(), 1);
        assert_eq!(back["votes"].as_array().unwrap().len(), 1);

        let gone = server
            .call_tool(
                &reader,
                "list_message_backlinks",
                &json!({ "message_id": dead.id.0 }),
            )
            .await
            .unwrap_err();
        // Pre-dispatch `ensure_message_access` maps a missing row through
        // `AuthError::Store` → `McpError::Internal` (REST maps the same store
        // error to 404). Either way the pointers are not returned.
        assert!(gone.to_string().contains("not found"), "{gone}");

        let census = content(
            &server
                .call_tool(&reader, "get_kind_census", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(census["workspace_id"], ws.id.0.to_string());
        assert!(census["total"].as_i64().unwrap() > 0);
        let kinds: Vec<&str> = census["counts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"message_posted"));
        assert!(kinds.contains(&"workspace_created"));
    }
}
