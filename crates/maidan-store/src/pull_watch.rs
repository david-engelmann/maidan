//! Watching a pull request: track it so each change-sweep tick reads its
//! checks, on a thread so the notices that read owes have a home.
//!
//! REST (`POST /workspaces/{id}/pull-watches`) and MCP (`watch_pull`) share
//! this. Each caller checks first, with what only it knows: the caller's
//! capability, its access to a channel it names, and that the repository is
//! blessed for the workspace ([`repo_blessed`]), since the sweep reads the
//! pull request with the instance's GitHub credential.

use maidan_types::{
    ChannelId, MemberId, NewChannel, NewThread, StoredEvent, ThreadId, WatchedPull, WorkspaceId,
    DEFAULT_WATCH_CHANNEL,
};

use crate::error::StoreError;
use crate::store::Store;

/// Whether `repo` (lowercase `owner/name`) is in the workspace's egress
/// allowlist for GitHub comments or agent branches: the repositories the
/// workspace may have the instance read and write for it.
pub async fn repo_blessed(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    repo: &str,
) -> Result<bool, StoreError> {
    Ok(store
        .list_egress_targets(workspace_id)
        .await?
        .into_iter()
        .any(|target| match target.surface.as_str() {
            "github" => target.selector.eq_ignore_ascii_case(repo),
            "github_branch" => target
                .selector
                .split_once('@')
                .is_some_and(|(r, _)| r.eq_ignore_ascii_case(repo)),
            _ => false,
        }))
}

/// A watch, and the events its channel and thread were created with, for the
/// caller to publish.
#[derive(Debug)]
pub struct WatchOutcome {
    pub watched: WatchedPull,
    pub events: Vec<StoredEvent>,
}

/// What to watch. `repo` is already normalized and blessed.
#[derive(Debug, Clone)]
pub struct WatchRequest {
    pub workspace_id: WorkspaceId,
    pub repo: String,
    pub pull_number: i64,
    pub channel_id: Option<ChannelId>,
    pub title: Option<String>,
    /// The watcher: the new thread's creator and owner, so the notices are
    /// said as a message on it and its merge lands it.
    pub created_by: Option<MemberId>,
}

/// Watch a pull request. One this workspace already tracks answers with its
/// thread (`created: false`), whose access the caller must still check; one
/// another workspace tracks is a conflict, before anything is made.
/// Otherwise a thread is made for it
/// in the named channel (which must be in the workspace) or the workspace's
/// public `pull-requests` channel, made when missing.
pub async fn watch_pull(store: &dyn Store, req: WatchRequest) -> Result<WatchOutcome, StoreError> {
    if let Some(thread_id) = store
        .tracked_pull_thread(req.workspace_id, &req.repo, req.pull_number)
        .await?
    {
        return Ok(WatchOutcome {
            watched: watched(&req, thread_id, false),
            events: Vec::new(),
        });
    }
    let mut events = Vec::new();
    let channel_id = match req.channel_id {
        Some(channel_id) => {
            let channel = store.get_channel(channel_id).await?;
            if channel.workspace_id != req.workspace_id || channel.tombstoned_at.is_some() {
                return Err(StoreError::NotFound);
            }
            channel_id
        }
        None => default_channel(store, req.workspace_id, &mut events).await?,
    };
    let title = req
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| format!("{}#{}", req.repo, req.pull_number));
    let (thread, created) = store
        .create_thread_with_event(NewThread {
            channel_id,
            parent_thread_id: None,
            title: Some(title),
            description: Some(format!(
                "Watching https://github.com/{}/pull/{}: Maidan notes its checks here.",
                req.repo, req.pull_number
            )),
            created_by: req.created_by,
        })
        .await?;
    events.push(created);
    if req.created_by.is_some() {
        store.set_thread_owner(thread.id, req.created_by).await?;
    }
    store
        .watch_change_pull(req.workspace_id, thread.id, &req.repo, req.pull_number)
        .await?;
    Ok(WatchOutcome {
        watched: watched(&req, thread.id, true),
        events,
    })
}

fn watched(req: &WatchRequest, thread_id: ThreadId, created: bool) -> WatchedPull {
    WatchedPull {
        repo: req.repo.clone(),
        pull_number: req.pull_number,
        thread_id,
        created,
    }
}

/// The workspace's `pull-requests` channel, made public when missing. A
/// private one of that name is refused rather than used: the watcher might
/// not be able to read what lands in it.
async fn default_channel(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    events: &mut Vec<StoredEvent>,
) -> Result<ChannelId, StoreError> {
    let find = |channels: Vec<maidan_types::Channel>| {
        channels
            .into_iter()
            .find(|c| c.name == DEFAULT_WATCH_CHANNEL && c.tombstoned_at.is_none())
    };
    let existing = match find(store.list_channels(workspace_id).await?) {
        Some(channel) => channel,
        None => match store
            .create_channel_with_event(NewChannel {
                workspace_id,
                name: DEFAULT_WATCH_CHANNEL.to_string(),
                topic: Some("Pull requests Maidan watches.".to_string()),
                private: false,
            })
            .await
        {
            Ok((channel, event)) => {
                events.push(event);
                channel
            }
            // Another watch made it first.
            Err(StoreError::Conflict(_)) => find(store.list_channels(workspace_id).await?)
                .ok_or_else(|| {
                    StoreError::Conflict(format!("the `{DEFAULT_WATCH_CHANNEL}` channel is taken"))
                })?,
            Err(err) => return Err(err),
        },
    };
    if existing.private {
        return Err(StoreError::Conflict(format!(
            "the `{DEFAULT_WATCH_CHANNEL}` channel is private; name the channel to watch in"
        )));
    }
    Ok(existing.id)
}
