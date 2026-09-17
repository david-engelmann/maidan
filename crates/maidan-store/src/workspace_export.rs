//! Assemble a workspace content graph for export. Shared by REST and MCP so
//! both sign the same payload shape.

use crate::{Store, StoreError};
use maidan_types::*;

const PAGE: i64 = 500;

/// Read the whole workspace content graph. Fans out per-channel (members)
/// and per-thread (messages, pins); messages are paginated so a thread
/// with more than one page is captured in full.
pub async fn build_workspace_export(
    store: &dyn Store,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceExport, StoreError> {
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
        format_version: WORKSPACE_EXPORT_FORMAT_VERSION,
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
