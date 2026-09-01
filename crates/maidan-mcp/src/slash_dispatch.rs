//! Slash-command dispatch, dependency-inverted (Cluster 345).
//!
//! Slash commands run an HTTP callback or an MCP tool and edit the posted
//! message with the result — logic that lives in `maidan-server` (it needs the
//! webhook client, secret decryption, and re-entry into the MCP server). The MCP
//! `post_message` handler lives here in `maidan-mcp`, which `maidan-server`
//! depends on (not the reverse), so it can't call that logic directly.
//!
//! [`SlashDispatcher`] inverts the dependency: `maidan-server` implements it and
//! attaches an instance to the [`McpServer`](crate::server::McpServer) at startup
//! (`set_slash_dispatcher`), and the MCP post path invokes it when a slash command
//! is registered — giving MCP posts the same slash behaviour as REST posts.

use async_trait::async_trait;
use maidan_auth::AuthContext;
use maidan_router::ParsedSlashCommand;
use maidan_types::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};
use serde_json::Value;

/// Runs a registered slash command for a just-posted message and returns the
/// slash **metadata to merge** into it (`{slash_command, slash_response}`),
/// matching the REST post path. Called only when a command named `parsed.name`
/// is registered for `workspace_id`.
#[async_trait]
pub trait SlashDispatcher: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn dispatch(
        &self,
        auth: &AuthContext,
        parsed: &ParsedSlashCommand,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        author_id: MemberId,
        message_id: MessageId,
    ) -> Value;
}
