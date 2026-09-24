//! Channel and DM-conversation listing/opening tool handlers.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListChannelsArgs {
    workspace_id: uuid::Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddChannelMemberArgs {
    channel_id: uuid::Uuid,
    member_id: uuid::Uuid,
    #[serde(default)]
    role: Option<ChannelMemberRole>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelMemberRefArgs {
    channel_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelRefArgs {
    channel_id: uuid::Uuid,
}

/// The channel a membership tool names, if it is in the caller's workspace.
/// `channel:admin` is per-workspace, and these tools are not channel-scoped at
/// dispatch — an admin manages a private channel's membership without being in
/// it — so the workspace is checked here. A foreign channel is `NotFound`, the
/// same answer as a missing one.
async fn own_channel(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    channel_id: ChannelId,
) -> Result<Channel, McpError> {
    let channel = store.get_channel(channel_id).await?;
    if !auth.bypass && channel.workspace_id != auth.workspace_id {
        return Err(McpError::NotFound);
    }
    Ok(channel)
}

pub(super) async fn add_channel_member(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AddChannelMemberArgs = serde_json::from_value(args.clone())?;
    let channel = own_channel(store, auth, ChannelId(a.channel_id)).await?;
    let member = store.get_member(MemberId(a.member_id)).await?;
    if member.workspace_id != channel.workspace_id {
        return Err(McpError::InvalidParams(
            "member is not in the channel's workspace".into(),
        ));
    }
    let role = a.role.unwrap_or(ChannelMemberRole::Member);
    let (actor, workspace_id) = (auth.actor_id, channel.workspace_id);
    let m = store
        .add_channel_member_audited(
            channel.id,
            member.id,
            role,
            Box::new(move |m| NewAuditEvent {
                actor_id: Some(actor),
                action: "channel_member.add".into(),
                target_kind: Some("channel".into()),
                target_id: Some(m.channel_id.0),
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "subject_member_id": m.member_id.0,
                    "role": m.role.as_str(),
                    "surface": "mcp",
                }),
            }),
        )
        .await?;
    Ok(content_json(&m))
}

pub(super) async fn list_channel_members(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ChannelRefArgs = serde_json::from_value(args.clone())?;
    own_channel(store, auth, ChannelId(a.channel_id)).await?;
    let members = store.list_channel_members(ChannelId(a.channel_id)).await?;
    Ok(content_json(&members))
}

pub(super) async fn remove_channel_member(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ChannelMemberRefArgs = serde_json::from_value(args.clone())?;
    let channel = own_channel(store, auth, ChannelId(a.channel_id)).await?;
    store
        .remove_channel_member_audited(
            channel.id,
            MemberId(a.member_id),
            NewAuditEvent {
                actor_id: Some(auth.actor_id),
                action: "channel_member.remove".into(),
                target_kind: Some("channel".into()),
                target_id: Some(channel.id.0),
                metadata: serde_json::json!({
                    "workspace_id": channel.workspace_id.0,
                    "subject_member_id": a.member_id,
                    "surface": "mcp",
                }),
            },
        )
        .await?;
    Ok(content_json(&serde_json::json!({"ok": true})))
}

/// Mute a channel for the caller. The notification router then suppresses the
/// channel's firehose for the caller — a mention still breaks through. Channel
/// access is enforced pre-dispatch.
pub(super) async fn mute_channel(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ChannelRefArgs = serde_json::from_value(args.clone())?;
    store
        .mute_channel(auth.member_id, ChannelId(a.channel_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "muted": true })))
}

/// Unmute a channel for the caller. `{unmuted}` is `false` when it was not
/// muted.
pub(super) async fn unmute_channel(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ChannelRefArgs = serde_json::from_value(args.clone())?;
    let unmuted = store
        .unmute_channel(auth.member_id, ChannelId(a.channel_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "unmuted": unmuted })))
}

pub(super) async fn list_channels(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListChannelsArgs = serde_json::from_value(args.clone())?;
    let channels = store.list_channels(WorkspaceId(a.workspace_id)).await?;
    if auth.bypass {
        return Ok(content_json(&channels));
    }
    // Hide private channels the caller is not a member of.
    let mut visible = Vec::with_capacity(channels.len());
    for ch in channels {
        if !ch.private
            || ch.name == DM_CHANNEL_NAME
            || store.channel_is_member(ch.id, auth.member_id).await?
        {
            visible.push(ch);
        }
    }
    Ok(content_json(&visible))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenDmArgs {
    workspace_id: uuid::Uuid,
    other_member_id: uuid::Uuid,
}

pub(super) async fn open_dm_conversation(
    store: &Arc<dyn Store>,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: OpenDmArgs = serde_json::from_value(args.clone())?;
    let dm = store
        .open_dm_conversation(
            WorkspaceId(a.workspace_id),
            auth.member_id,
            MemberId(a.other_member_id),
        )
        .await?;
    Ok(content_json(&dm))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDmArgs {
    workspace_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

pub(super) async fn list_dm_conversations(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListDmArgs = serde_json::from_value(args.clone())?;
    let list = store
        .list_dm_conversations_for_member(WorkspaceId(a.workspace_id), MemberId(a.member_id))
        .await?;
    Ok(content_json(&list))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::{capability::CHANNEL_ADMIN, AuthContext};
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::server::McpServer;

    async fn member_of(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId, ChannelId) {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: format!("{name}-admin"),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "private".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        store
            .add_channel_member(channel.id, member.id, ChannelMemberRole::Member)
            .await
            .unwrap();
        (ws.id, member.id, channel.id)
    }

    /// `channel:admin` is per-workspace. Tenant A's admin must not read, join,
    /// or empty tenant B's private channel, nor seat B's members in A's.
    #[tokio::test]
    async fn channel_membership_tools_stay_in_the_callers_workspace() {
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
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let (ws_a, admin_a, channel_a) = member_of(store.as_ref(), "alpha").await;
        let (_, member_b, channel_b) = member_of(store.as_ref(), "bravo").await;
        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::from_session(admin_a, ws_a, vec![CHANNEL_ADMIN.to_string()]);
        let call = |name: &'static str, args: serde_json::Value| {
            let server = &server;
            let auth = &auth;
            async move { server.call_tool(auth, name, &args).await }
        };

        let foreign = json!({ "channel_id": channel_b.0 });
        assert!(call("list_channel_members", foreign).await.is_err());
        assert!(call(
            "add_channel_member",
            json!({ "channel_id": channel_b.0, "member_id": admin_a.0 }),
        )
        .await
        .is_err());
        assert!(call(
            "remove_channel_member",
            json!({ "channel_id": channel_b.0, "member_id": member_b.0 }),
        )
        .await
        .is_err());
        assert!(call(
            "add_channel_member",
            json!({ "channel_id": channel_a.0, "member_id": member_b.0 }),
        )
        .await
        .is_err());

        let b_members = store.list_channel_members(channel_b).await.unwrap();
        assert_eq!(b_members.len(), 1, "B's private channel is untouched");
        assert_eq!(b_members[0].member_id, member_b);
        assert_eq!(
            store.list_channel_members(channel_a).await.unwrap().len(),
            1
        );

        // Its own channel it still manages.
        let listed = call("list_channel_members", json!({ "channel_id": channel_a.0 }))
            .await
            .unwrap();
        assert!(listed["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(&admin_a.0.to_string()));
    }
}
