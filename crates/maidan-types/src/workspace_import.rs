//! Pure flatten / remap over a workspace export graph. Shared by REST and MCP
//! so both import paths rewrite ids the same way.

use std::collections::{HashMap, HashSet};

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

/// Check that the bundle describes one workspace and nothing outside it:
/// every member and channel belongs to the bundle's workspace, and every
/// reference (channel, thread, parent, holder, owner, creator, author,
/// editor, pin, cross-reference) names an entity defined in the bundle. A
/// bundle that fails names, or writes into, some other tenant, so both
/// import modes refuse it before writing anything. The error says which
/// reference failed.
pub fn validate_import(b: &WorkspaceImport) -> Result<(), String> {
    let ws = b.workspace.id;
    if let Some(m) = b.members.iter().find(|m| m.workspace_id != ws) {
        return Err(format!(
            "member {} belongs to workspace {}, not the imported workspace",
            m.id.0, m.workspace_id.0
        ));
    }
    if let Some(c) = b.channels.iter().find(|c| c.workspace_id != ws) {
        return Err(format!(
            "channel {} belongs to workspace {}, not the imported workspace",
            c.id.0, c.workspace_id.0
        ));
    }
    let members: HashSet<MemberId> = b.members.iter().map(|m| m.id).collect();
    let channels: HashSet<ChannelId> = b.channels.iter().map(|c| c.id).collect();
    let threads: HashSet<ThreadId> = b.threads.iter().map(|t| t.id).collect();
    let messages: HashSet<MessageId> = b.messages.iter().map(|m| m.id).collect();
    let member = |what: &str, id: MemberId| -> Result<(), String> {
        if members.contains(&id) {
            Ok(())
        } else {
            Err(format!(
                "{what} names member {}, which is not in the bundle",
                id.0
            ))
        }
    };
    let channel = |what: &str, id: ChannelId| -> Result<(), String> {
        if channels.contains(&id) {
            Ok(())
        } else {
            Err(format!(
                "{what} names channel {}, which is not in the bundle",
                id.0
            ))
        }
    };
    let thread = |what: &str, id: ThreadId| -> Result<(), String> {
        if threads.contains(&id) {
            Ok(())
        } else {
            Err(format!(
                "{what} names thread {}, which is not in the bundle",
                id.0
            ))
        }
    };
    let message = |what: &str, id: MessageId| -> Result<(), String> {
        if messages.contains(&id) {
            Ok(())
        } else {
            Err(format!(
                "{what} names message {}, which is not in the bundle",
                id.0
            ))
        }
    };
    for cm in &b.channel_members {
        channel("a channel membership", cm.channel_id)?;
        member("a channel membership", cm.member_id)?;
    }
    for t in &b.threads {
        let what = format!("thread {}", t.id.0);
        channel(&what, t.channel_id)?;
        if let Some(p) = t.parent_thread_id {
            thread(&format!("{what} as parent"), p)?;
        }
        if let Some(a) = t.assignee_id {
            member(&format!("{what} as holder"), a)?;
        }
        if let Some(o) = t.owner_id {
            member(&format!("{what} as owner"), o)?;
        }
        if let Some(c) = t.created_by {
            member(&format!("{what} as creator"), c)?;
        }
    }
    for m in &b.messages {
        let what = format!("message {}", m.id.0);
        thread(&what, m.thread_id)?;
        member(&format!("{what} as author"), m.author_id)?;
    }
    for e in &b.message_edits {
        message("a message edit", e.message_id)?;
        member("a message edit as editor", e.editor_id)?;
    }
    for p in &b.pins {
        thread("a pin", p.thread_id)?;
        message("a pin", p.message_id)?;
        member("a pin as pinner", p.member_id)?;
    }
    for r in &b.references {
        for (kind, id) in [(r.src_kind, r.src_id), (r.dst_kind, r.dst_id)] {
            let what = format!("reference {}", r.id);
            match kind {
                RefSide::Thread => thread(&what, ThreadId(id))?,
                RefSide::Message => message(&what, MessageId(id))?,
            }
        }
    }
    Ok(())
}

/// Remap every id in the bundle to a fresh one, rewriting all foreign keys.
/// Timestamps and content are preserved. Used by `mode=new`. The bundle is
/// checked with [`validate_import`] first, so every lookup below resolves;
/// a bundle that fails is refused with the check's message.
pub fn remap_import(
    bundle: WorkspaceImport,
    mut new_id: impl FnMut() -> uuid::Uuid,
) -> Result<WorkspaceImport, String> {
    validate_import(&bundle)?;
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

    Ok(WorkspaceImport {
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
                created_by: t.created_by.map(|c| members[&c]),
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
    })
}
