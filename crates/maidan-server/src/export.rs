//! Workspace export / portability (Cluster 187).
//!
//! Assembles a workspace's collaboration graph into one JSON bundle so an
//! operator can migrate or archive a tenant — the read-side counterpart to the
//! erase/purge paths, which were the only whole-workspace operations. Flat,
//! id-linked collections (not deep nesting) keep it easy to diff and re-import.
//!
//! **Excludes secrets** (API tokens, webhook/slash secrets, OIDC/OAuth) and
//! operational tables (events, audit, deliveries) — this is user content, not
//! credentials or ops state. Reactions/votes are deferred (per-message N+1 over
//! a large workspace); tracked in Open Work.

use std::sync::Arc;

use maidan_store::{Store, StoreError};
use maidan_types::*;
use serde::Serialize;

/// Bump when the bundle shape changes in a way an importer must notice.
const FORMAT_VERSION: u32 = 1;

/// Cap on messages fetched per page while paginating a thread to completeness.
const PAGE: i64 = 500;

#[derive(Debug, Serialize)]
pub struct WorkspaceExport {
    pub format_version: u32,
    pub exported_at: chrono::DateTime<chrono::Utc>,
    pub workspace: Workspace,
    pub members: Vec<Member>,
    pub channels: Vec<ExportChannel>,
    pub threads: Vec<Thread>,
    pub messages: Vec<Message>,
    pub message_edits: Vec<MessageEdit>,
    pub pins: Vec<Pin>,
    pub references: Vec<Reference>,
}

#[derive(Debug, Serialize)]
pub struct ExportChannel {
    pub channel: Channel,
    pub members: Vec<ChannelMember>,
}

/// Read the whole workspace content graph. Fans out per-channel (members) and
/// per-thread (messages, pins); messages are paginated so a thread with more
/// than one page is captured in full.
pub async fn build(
    store: &Arc<dyn Store>,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceExport, StoreError> {
    // Resolve the workspace first so an unknown id is a clean NotFound.
    let workspace = store.get_workspace(workspace_id).await?;
    let members = store.list_members(workspace_id).await?;

    let mut channels = Vec::new();
    for channel in store.list_channels(workspace_id).await? {
        let channel_members = store.list_channel_members(channel.id).await?;
        channels.push(ExportChannel {
            channel,
            members: channel_members,
        });
    }

    let threads = store.list_threads_for_workspace(workspace_id).await?;

    let mut messages = Vec::new();
    let mut pins = Vec::new();
    for thread in &threads {
        let mut after: Option<MessageId> = None;
        loop {
            let page = store.list_messages_after(thread.id, after, PAGE).await?;
            let got = page.len();
            if let Some(last) = page.last() {
                after = Some(last.id);
            }
            messages.extend(page);
            if (got as i64) < PAGE {
                break;
            }
        }
        pins.extend(store.list_pins_for_thread(thread.id).await?);
    }

    let message_ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
    let message_edits = store
        .list_message_edits_for_messages(&message_ids, PAGE)
        .await?;

    // References from both thread and message sources.
    let mut references = store
        .list_references_from_many(
            RefSide::Thread,
            &threads.iter().map(|t| t.id.0).collect::<Vec<_>>(),
        )
        .await?;
    references.extend(
        store
            .list_references_from_many(
                RefSide::Message,
                &message_ids.iter().map(|m| m.0).collect::<Vec<_>>(),
            )
            .await?,
    );

    Ok(WorkspaceExport {
        format_version: FORMAT_VERSION,
        exported_at: chrono::Utc::now(),
        workspace,
        members,
        channels,
        threads,
        messages,
        message_edits,
        pins,
        references,
    })
}
