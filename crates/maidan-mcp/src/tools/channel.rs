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
struct ListChannelsArgs {
    workspace_id: uuid::Uuid,
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
    // Hide private channels the caller is not a member of (Cluster 162).
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
struct OpenDmArgs {
    workspace_id: uuid::Uuid,
    member_id: uuid::Uuid,
    other_member_id: uuid::Uuid,
}

pub(super) async fn open_dm_conversation(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: OpenDmArgs = serde_json::from_value(args.clone())?;
    let dm = store
        .open_dm_conversation(
            WorkspaceId(a.workspace_id),
            MemberId(a.member_id),
            MemberId(a.other_member_id),
        )
        .await?;
    Ok(content_json(&dm))
}

#[derive(Deserialize)]
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
