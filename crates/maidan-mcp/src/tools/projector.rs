//! MCP projector link-management tools (Cluster 349) — the MCP twins of the
//! Cluster-346 REST link routes. A projector link maps a Maidan thread to an
//! external Slack channel or GitHub issue/PR so the projector can bridge messages
//! both ways. The link's `workspace_id`/`channel_id` are resolved from the thread
//! (via [`maidan_auth::authorize_thread`], which also authorizes access), so they
//! can't disagree with it; the caller supplies the external id, the thread, and the
//! member that relayed external messages are attributed to. Writes need
//! `workspace:write` + thread access; lists/unlinks are workspace-scoped.

use maidan_auth::AuthContext;
use maidan_types::{MemberId, NewGithubIssueLink, NewSlackChannelLink, ThreadId};
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;
use crate::server::McpServer;

#[derive(Deserialize)]
struct LinkSlackArgs {
    thread_id: uuid::Uuid,
    slack_channel_id: String,
    member_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct UnlinkSlackArgs {
    slack_channel_id: String,
}

#[derive(Deserialize)]
struct LinkGithubArgs {
    thread_id: uuid::Uuid,
    repo: String,
    issue_number: i64,
    member_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct UnlinkGithubArgs {
    repo: String,
    issue_number: i64,
}

pub(super) async fn link_slack_channel(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: LinkSlackArgs = serde_json::from_value(args.clone())?;
    let scope =
        maidan_auth::authorize_thread(server.store.as_ref(), auth, ThreadId(a.thread_id)).await?;
    let link = server
        .store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: a.slack_channel_id,
            workspace_id: scope.workspace_id,
            channel_id: scope.channel_id,
            thread_id: ThreadId(a.thread_id),
            member_id: MemberId(a.member_id),
        })
        .await?;
    Ok(content_json(&link))
}

pub(super) async fn list_slack_channel_links(
    server: &McpServer,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let links = server
        .store
        .list_slack_channel_links(auth.workspace_id)
        .await?;
    Ok(content_json(&links))
}

pub(super) async fn unlink_slack_channel(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UnlinkSlackArgs = serde_json::from_value(args.clone())?;
    // Workspace-scope the unlink: only remove a link that belongs to the caller's ws.
    let scoped = matches!(
        server.store.get_slack_channel_link(&a.slack_channel_id).await?,
        Some(link) if link.workspace_id == auth.workspace_id
    );
    let unlinked = scoped
        && server
            .store
            .unlink_slack_channel(&a.slack_channel_id)
            .await?;
    Ok(content_json(&json!({ "unlinked": unlinked })))
}

pub(super) async fn link_github_issue(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: LinkGithubArgs = serde_json::from_value(args.clone())?;
    let scope =
        maidan_auth::authorize_thread(server.store.as_ref(), auth, ThreadId(a.thread_id)).await?;
    let link = server
        .store
        .link_github_issue(NewGithubIssueLink {
            repo: a.repo,
            issue_number: a.issue_number,
            workspace_id: scope.workspace_id,
            channel_id: scope.channel_id,
            thread_id: ThreadId(a.thread_id),
            member_id: MemberId(a.member_id),
        })
        .await?;
    Ok(content_json(&link))
}

pub(super) async fn list_github_issue_links(
    server: &McpServer,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let links = server
        .store
        .list_github_issue_links(auth.workspace_id)
        .await?;
    Ok(content_json(&links))
}

pub(super) async fn unlink_github_issue(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UnlinkGithubArgs = serde_json::from_value(args.clone())?;
    let scoped = matches!(
        server.store.get_github_issue_link(&a.repo, a.issue_number).await?,
        Some(link) if link.workspace_id == auth.workspace_id
    );
    let unlinked = scoped
        && server
            .store
            .unlink_github_issue(&a.repo, a.issue_number)
            .await?;
    Ok(content_json(&json!({ "unlinked": unlinked })))
}
