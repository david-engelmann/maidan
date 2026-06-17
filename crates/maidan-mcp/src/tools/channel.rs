//! Channel and DM-conversation listing/opening tool handlers.

use std::sync::Arc;

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

pub(super) async fn list_channels(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListChannelsArgs = serde_json::from_value(args.clone())?;
    let channels = store.list_channels(WorkspaceId(a.workspace_id)).await?;
    Ok(content_json(&channels))
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
