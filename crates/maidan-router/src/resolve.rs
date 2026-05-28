//! Resolve workspace / channel / thread hierarchy for messages and threads.

use maidan_store::Store;
use maidan_types::*;

use crate::error::RouterError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelContext {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadContext {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageChain {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
}

pub async fn resolve_channel_context(
    store: &dyn Store,
    channel_id: ChannelId,
) -> Result<ChannelContext, RouterError> {
    let channel = store.get_channel(channel_id).await?;
    Ok(ChannelContext {
        workspace_id: channel.workspace_id,
        channel_id: channel.id,
    })
}

pub async fn resolve_thread_context(
    store: &dyn Store,
    thread_id: ThreadId,
) -> Result<ThreadContext, RouterError> {
    let thread = store.get_thread(thread_id).await?;
    let channel = store.get_channel(thread.channel_id).await?;
    Ok(ThreadContext {
        workspace_id: channel.workspace_id,
        channel_id: channel.id,
        thread_id,
    })
}

pub async fn resolve_message_chain(
    store: &dyn Store,
    message_id: MessageId,
) -> Result<MessageChain, RouterError> {
    let message = store.get_message(message_id).await?;
    let ctx = resolve_thread_context(store, message.thread_id).await?;
    Ok(MessageChain {
        workspace_id: ctx.workspace_id,
        channel_id: ctx.channel_id,
        thread_id: ctx.thread_id,
    })
}
