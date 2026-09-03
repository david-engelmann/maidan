//! Workspace import (Cluster 270) — the write-side counterpart to the Cluster-187
//! export. Two modes:
//!
//! - **new** (default): every id in the bundle is remapped to a fresh one, so the
//!   content lands as a brand-new workspace (a clone/fork). Never collides.
//! - **restore**: ids are preserved verbatim, so an exported bundle round-trips
//!   into the same identities — for disaster recovery into a fresh database. Guarded
//!   by an "already exists" check at the route (409 unless `force`).
//!
//! The heavy lifting is [`remap`], a pure function over the bundle: it is fully
//! unit-tested here (referential integrity after remap) with no database.

use std::collections::HashMap;

use maidan_types::*;

use crate::export::WorkspaceExport;

/// Flatten an exported bundle into the store's flat import shape: the export nests
/// channel members under each channel; the import wants two flat collections.
pub fn flatten(export: WorkspaceExport) -> WorkspaceImport {
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

/// Remap every id in the bundle to a fresh one, rewriting all foreign keys so the
/// graph stays internally consistent. Timestamps and content are preserved — only
/// identities change. Used by `mode=new` so an import never collides with existing
/// rows. Pure: `new_id()` is the only source of freshness (injected for testing).
pub fn remap(bundle: WorkspaceImport, mut new_id: impl FnMut() -> uuid::Uuid) -> WorkspaceImport {
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

    // A reference endpoint is a raw uuid tagged by its kind; remap through the
    // matching table, falling back to the original if it points outside the bundle.
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample() -> WorkspaceImport {
        let now = Utc::now();
        let ws = WorkspaceId(uuid::Uuid::new_v4());
        let m = MemberId(uuid::Uuid::new_v4());
        let ch = ChannelId(uuid::Uuid::new_v4());
        let root = ThreadId(uuid::Uuid::new_v4());
        let child = ThreadId(uuid::Uuid::new_v4());
        let msg = MessageId(uuid::Uuid::new_v4());
        WorkspaceImport {
            workspace: Workspace {
                id: ws,
                name: "w".into(),
                created_at: now,
                updated_at: now,
                tombstoned_at: None,
            },
            members: vec![Member {
                id: m,
                workspace_id: ws,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Human,
                created_at: now,
                updated_at: now,
                tombstoned_at: None,
            }],
            channels: vec![Channel {
                id: ch,
                workspace_id: ws,
                name: "c".into(),
                topic: None,
                private: true,
                created_at: now,
                updated_at: now,
                tombstoned_at: None,
            }],
            channel_members: vec![ChannelMember {
                channel_id: ch,
                member_id: m,
                role: ChannelMemberRole::Admin,
                created_at: now,
            }],
            threads: vec![
                Thread {
                    id: root,
                    channel_id: ch,
                    parent_thread_id: None,
                    title: None,
                    state: ThreadState::Open,
                    assignee_id: Some(m),
                    assignment_expires_at: None,
                    claim_lease_id: None,
                    created_at: now,
                    updated_at: now,
                    tombstoned_at: None,
                },
                Thread {
                    id: child,
                    channel_id: ch,
                    parent_thread_id: Some(root),
                    title: None,
                    state: ThreadState::Open,
                    assignee_id: None,
                    assignment_expires_at: None,
                    claim_lease_id: None,
                    created_at: now,
                    updated_at: now,
                    tombstoned_at: None,
                },
            ],
            messages: vec![Message {
                id: msg,
                thread_id: root,
                author_id: m,
                body: "hi".into(),
                metadata: serde_json::json!({}),
                content: None,
                posted_at: now,
                edited_at: None,
                tombstoned_at: None,
            }],
            message_edits: vec![MessageEdit {
                id: 0,
                message_id: msg,
                editor_id: m,
                body_before: "h".into(),
                body_after: "hi".into(),
                edited_at: now,
            }],
            pins: vec![Pin {
                thread_id: root,
                message_id: msg,
                member_id: m,
                created_at: now,
            }],
            references: vec![Reference {
                id: uuid::Uuid::new_v4(),
                src_kind: RefSide::Thread,
                src_id: root.0,
                dst_kind: RefSide::Message,
                dst_id: msg.0,
                relation: "about".into(),
                created_at: now,
            }],
        }
    }

    #[test]
    fn remap_assigns_fresh_ids_and_preserves_referential_integrity() {
        let original = sample();
        let orig_ws = original.workspace.id;
        let remapped = remap(sample(), uuid::Uuid::new_v4);

        // Every top-level id changed.
        assert_ne!(remapped.workspace.id, orig_ws);
        assert_ne!(remapped.members[0].id, original.members[0].id);
        assert_ne!(remapped.channels[0].id, original.channels[0].id);
        assert_ne!(remapped.threads[0].id, original.threads[0].id);
        assert_ne!(remapped.messages[0].id, original.messages[0].id);

        let ws = remapped.workspace.id;
        let member = remapped.members[0].id;
        let channel = remapped.channels[0].id;
        let root = remapped.threads[0].id;
        let child = remapped.threads[1].id;
        let msg = remapped.messages[0].id;

        // Foreign keys point at the remapped ids, not stale originals.
        assert_eq!(remapped.members[0].workspace_id, ws);
        assert_eq!(remapped.channels[0].workspace_id, ws);
        assert_eq!(remapped.channel_members[0].channel_id, channel);
        assert_eq!(remapped.channel_members[0].member_id, member);
        assert_eq!(remapped.threads[0].channel_id, channel);
        assert_eq!(remapped.threads[0].assignee_id, Some(member));
        assert_eq!(remapped.threads[1].parent_thread_id, Some(root));
        assert_eq!(remapped.messages[0].thread_id, root);
        assert_eq!(remapped.messages[0].author_id, member);
        assert_eq!(remapped.message_edits[0].message_id, msg);
        assert_eq!(remapped.pins[0].thread_id, root);
        assert_eq!(remapped.pins[0].message_id, msg);
        assert_eq!(remapped.references[0].src_id, root.0);
        assert_eq!(remapped.references[0].dst_id, msg.0);
        // Content is preserved through the remap.
        assert_eq!(remapped.messages[0].body, "hi");
        assert!(remapped.channels[0].private);

        // `child` is distinct from `root` (not collapsed).
        assert_ne!(root, child);
    }

    #[test]
    fn flatten_splits_nested_channel_members() {
        use crate::export::{ExportChannel, WorkspaceExport};
        let b = sample();
        let export = WorkspaceExport {
            format_version: crate::export::FORMAT_VERSION,
            exported_at: Utc::now(),
            workspace: b.workspace.clone(),
            members: b.members.clone(),
            channels: vec![ExportChannel {
                channel: b.channels[0].clone(),
                members: b.channel_members.clone(),
            }],
            threads: b.threads.clone(),
            messages: b.messages.clone(),
            message_edits: b.message_edits.clone(),
            pins: b.pins.clone(),
            references: b.references.clone(),
        };
        let flat = flatten(export);
        assert_eq!(flat.channels.len(), 1);
        assert_eq!(flat.channel_members.len(), 1);
        assert_eq!(flat.channel_members[0].channel_id, b.channels[0].id);
    }
}
