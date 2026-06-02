//! WS/MCP subscribe channel grant enforcement (Cluster 81.0).

use std::collections::HashSet;

use maidan_router::resolve_thread_context;
use maidan_types::{ChannelId, EventFilter, WorkspaceId};

use crate::state::AppState;

pub async fn apply_subscribe_grants(
    state: &AppState,
    filter: &mut EventFilter,
) -> Result<(), String> {
    let Some(ws) = filter.workspace_id else {
        return Ok(());
    };
    let channels = state
        .store
        .list_channels(ws)
        .await
        .map_err(|e| format!("list channels: {e}"))?;
    let mut grants: HashSet<ChannelId> = filter
        .channel_grants
        .as_ref()
        .map(|v| v.iter().copied().collect())
        .unwrap_or_default();
    if filter.dm_conversation_id.is_some() {
        if let Some(th) = filter.thread_id {
            let ctx = resolve_thread_context(state.store.as_ref(), th)
                .await
                .map_err(|e| format!("{e:?}"))?;
            grants.insert(ctx.channel_id);
        }
    }
    if let Some(ch) = filter.channel_id {
        let channel = channels
            .iter()
            .find(|c| c.id == ch)
            .ok_or_else(|| "channel not found in workspace".to_string())?;
        if channel.private && !grants.contains(&ch) {
            return Err("private channel requires channel_grants entry".into());
        }
    }
    if let Some(th) = filter.thread_id {
        let ctx = resolve_thread_context(state.store.as_ref(), th)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let channel = channels
            .iter()
            .find(|c| c.id == ctx.channel_id)
            .ok_or_else(|| "thread channel not found".to_string())?;
        if channel.private && !grants.contains(&ctx.channel_id) {
            return Err("private channel requires channel_grants entry".into());
        }
    }
    for channel in channels.iter().filter(|c| c.private) {
        if !grants.contains(&channel.id) {
            filter.private_channel_deny.insert(channel.id);
        }
    }
    if !grants.is_empty() {
        filter.channel_event_allow = Some(grants);
    }
    Ok(())
}

pub fn workspace_filter(ws: WorkspaceId, channel_grants: &[ChannelId]) -> EventFilter {
    EventFilter {
        workspace_id: Some(ws),
        channel_grants: Some(channel_grants.to_vec()),
        ..Default::default()
    }
}
