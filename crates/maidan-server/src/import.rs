//! Workspace import — the write-side counterpart to the export. Two modes:
//!
//! - **new** (default): every id in the bundle is remapped to a fresh one, so the
//!   content lands as a brand-new workspace (a clone/fork). Never collides.
//! - **restore**: ids are preserved verbatim, so an exported bundle round-trips
//!   into the same identities — for disaster recovery into a fresh database. Guarded
//!   by an "already exists" check at the route (409 unless `force`).
//!
//! The heavy lifting is [`remap`], a pure function over the bundle: it is fully
//! unit-tested here (referential integrity after remap) with no database.

pub use maidan_types::{flatten_export as flatten, remap_import as remap};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use maidan_types::*;

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
                    work_started_at: None,
                    owner_id: None,
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
                    work_started_at: None,
                    owner_id: None,
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
        use maidan_types::{ExportChannel, WorkspaceExport};
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
