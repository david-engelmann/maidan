//! Pure flatten / remap over a workspace export graph (Cluster 270 / 391).
//! Shared by REST and MCP so both import paths rewrite ids the same way.

use std::collections::HashMap;

use crate::*;

/// Flatten an exported bundle into the store's flat import shape: the export
/// nests channel members under each channel; the import wants two collections.
pub fn flatten_export(export: WorkspaceExport) -> WorkspaceImport {
    let mut channels = Vec::with_capacity(export.channels.len());
    let mut channel_members = Vec::new();
    for ec in export.channels {
        channel_members.extend(ec.members);
        channels.push(ec.channel);
    }
    WorkspaceImport {
        workspace: export.workspace,
        members: export.members,
        channels,
        channel_members,
        threads: export.threads,
        messages: export.messages,
        message_edits: export.message_edits,
        pins: export.pins,
        references: export.references,
    }
}

/// Remap every id in the bundle to a fresh one, rewriting all foreign keys.
/// Timestamps and content are preserved. Used by `mode=new`.
pub fn remap_import(
    bundle: WorkspaceImport,
    mut new_id: impl FnMut() -> uuid::Uuid,
) -> WorkspaceImport {
    let new_ws = WorkspaceId(new_id());

    let members: HashMap<MemberId, MemberId> = bundle
        .members
        .iter()
        .map(|m| (m.id, MemberId(new_id())))
        .collect();
    let channels: HashMap<ChannelId, ChannelId> = bundle
        .channels
        .iter()
        .map(|c| (c.id, ChannelId(new_id())))
        .collect();
    let threads: HashMap<ThreadId, ThreadId> = bundle
        .threads
        .iter()
        .map(|t| (t.id, ThreadId(new_id())))
        .collect();
    let messages: HashMap<MessageId, MessageId> = bundle
        .messages
        .iter()
        .map(|m| (m.id, MessageId(new_id())))
        .collect();

    let remap_ref = |kind: RefSide, id: uuid::Uuid| -> uuid::Uuid {
        match kind {
            RefSide::Thread => threads.get(&ThreadId(id)).map(|t| t.0).unwrap_or(id),
            RefSide::Message => messages.get(&MessageId(id)).map(|m| m.0).unwrap_or(id),
        }
    };

    WorkspaceImport {
        workspace: Workspace {
            id: new_ws,
            ..bundle.workspace
        },
        members: bundle
            .members
            .into_iter()
            .map(|m| Member {
                id: members[&m.id],
                workspace_id: new_ws,
                ..m
            })
            .collect(),
        channels: bundle
            .channels
            .into_iter()
            .map(|c| Channel {
                id: channels[&c.id],
                workspace_id: new_ws,
                ..c
            })
            .collect(),
        channel_members: bundle
            .channel_members
            .into_iter()
            .map(|cm| ChannelMember {
                channel_id: channels[&cm.channel_id],
                member_id: members[&cm.member_id],
                ..cm
            })
            .collect(),
        threads: bundle
            .threads
            .into_iter()
            .map(|t| Thread {
                id: threads[&t.id],
                channel_id: channels[&t.channel_id],
                parent_thread_id: t.parent_thread_id.map(|p| threads[&p]),
                assignee_id: t.assignee_id.map(|a| members[&a]),
                owner_id: t.owner_id.map(|o| members[&o]),
                ..t
            })
            .collect(),
        messages: bundle
            .messages
            .into_iter()
            .map(|m| Message {
                id: messages[&m.id],
                thread_id: threads[&m.thread_id],
                author_id: members[&m.author_id],
                ..m
            })
            .collect(),
        message_edits: bundle
            .message_edits
            .into_iter()
            .map(|e| MessageEdit {
                message_id: messages[&e.message_id],
                editor_id: members[&e.editor_id],
                ..e
            })
            .collect(),
        pins: bundle
            .pins
            .into_iter()
            .map(|p| Pin {
                thread_id: threads[&p.thread_id],
                message_id: messages[&p.message_id],
                member_id: members[&p.member_id],
                ..p
            })
            .collect(),
        references: bundle
            .references
            .into_iter()
            .map(|r| Reference {
                id: new_id(),
                src_id: remap_ref(r.src_kind, r.src_id),
                dst_id: remap_ref(r.dst_kind, r.dst_id),
                ..r
            })
            .collect(),
    }
}
