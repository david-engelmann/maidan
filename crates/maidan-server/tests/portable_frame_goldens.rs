//! Normalized review fixtures for the portable signed-export and event
//! snapshot/catch-up wire contracts. Generated identifiers, timestamps, and
//! cryptographic values are replaced; field names, value types, relationships,
//! enum strings, array structure, and protocol type ids remain load-bearing.

use std::{collections::BTreeMap, path::PathBuf};

use chrono::{DateTime, Utc};
use maidan_auth::{sign_export, ExportSigningKey};
use maidan_types::{
    link_for, normalize, CatchUpPage, Channel, ChannelId, ChannelMember, ChannelMemberRole, Event,
    EventKind, ExportChannel, LogSnapshot, Member, MemberId, MemberKind, Message, MessageId,
    SnapshotGraph, StoredEvent, Thread, ThreadId, ThreadState, Workspace, WorkspaceExport,
    WorkspaceId, WORKSPACE_EXPORT_FORMAT_VERSION,
};
use serde_json::{json, Map, Value};
use uuid::Uuid;

const WRITE_ENV: &str = "MAIDAN_WRITE_PORTABLE_GOLDENS";

fn timestamp() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp")
}

fn uuid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn sample_export() -> WorkspaceExport {
    let at = timestamp();
    let workspace_id = WorkspaceId(uuid(1));
    let member_id = MemberId(uuid(2));
    let channel_id = ChannelId(uuid(3));
    let thread_id = ThreadId(uuid(4));
    WorkspaceExport {
        format_version: WORKSPACE_EXPORT_FORMAT_VERSION,
        exported_at: at,
        workspace: Workspace {
            id: workspace_id,
            name: "portable-room".into(),
            created_at: at,
            updated_at: at,
            tombstoned_at: None,
        },
        members: vec![Member {
            id: member_id,
            workspace_id,
            handle: "alice".into(),
            display_name: Some("Alice".into()),
            kind: MemberKind::Human,
            created_at: at,
            updated_at: at,
            tombstoned_at: None,
        }],
        channels: vec![ExportChannel {
            channel: Channel {
                id: channel_id,
                workspace_id,
                name: "general".into(),
                topic: Some("portable contracts".into()),
                private: true,
                created_at: at,
                updated_at: at,
                tombstoned_at: None,
            },
            members: vec![ChannelMember {
                channel_id,
                member_id,
                role: ChannelMemberRole::Admin,
                created_at: at,
            }],
        }],
        threads: vec![Thread {
            id: thread_id,
            channel_id,
            parent_thread_id: None,
            title: Some("portable task".into()),
            state: ThreadState::Open,
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
            owner_id: Some(member_id),
            created_at: at,
            updated_at: at,
            tombstoned_at: None,
        }],
        messages: vec![Message {
            id: MessageId(uuid(5)),
            thread_id,
            author_id: member_id,
            body: "hello portable world".into(),
            metadata: json!({"source": "golden"}),
            content: None,
            posted_at: at,
            edited_at: None,
            tombstoned_at: None,
        }],
        message_edits: vec![],
        pins: vec![],
        references: vec![],
    }
}

fn normalize_volatile(value: Value) -> Value {
    let ids = BTreeMap::from([
        (uuid(1).to_string(), "<workspace-id>"),
        (uuid(2).to_string(), "<member-id>"),
        (uuid(3).to_string(), "<channel-id>"),
        (uuid(4).to_string(), "<thread-id>"),
        (uuid(5).to_string(), "<message-id>"),
    ]);

    fn walk(value: Value, key: Option<&str>, ids: &BTreeMap<String, &str>) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(name, value)| {
                        let value = walk(value, Some(&name), ids);
                        (name, value)
                    })
                    .collect::<Map<_, _>>(),
            ),
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| walk(value, key, ids))
                    .collect(),
            ),
            Value::String(raw) => {
                if let Some(replacement) = ids.get(&raw) {
                    return Value::String((*replacement).into());
                }
                match key {
                    Some(name) if name.ends_with("_at") => Value::String("<timestamp>".into()),
                    Some("public_key") => Value::String("<ed25519-public-key>".into()),
                    Some("signature") => Value::String("<ed25519-signature>".into()),
                    Some("content_sha256") => Value::String("<sha256>".into()),
                    Some("graph_hash" | "prev_hash" | "content_hash" | "genesis") => {
                        Value::String("sha256:<hash>".into())
                    }
                    _ => Value::String(raw),
                }
            }
            other => other,
        }
    }

    normalize(walk(value, None, &ids))
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn assert_golden(name: &str, value: Value) {
    let path = fixtures_dir().join(name);
    let got = serde_json::to_string_pretty(&normalize_volatile(value)).expect("pretty JSON") + "\n";
    if std::env::var(WRITE_ENV).ok().as_deref() == Some("1") {
        std::fs::create_dir_all(fixtures_dir()).expect("create fixtures directory");
        std::fs::write(&path, &got).expect("write golden fixture");
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got,
        expected,
        "portable wire fixture drift in {}; rerun with {WRITE_ENV}=1 only after reviewing compatibility",
        path.display()
    );
}

#[test]
fn signed_workspace_export_matches_normalized_golden() {
    let payload = serde_json::to_value(sample_export()).expect("export payload");
    let envelope =
        sign_export(&ExportSigningKey::from_seed([0x41; 32]), payload).expect("signed export");
    assert_golden(
        "normalized-workspace-export.json",
        serde_json::to_value(envelope).expect("export wire"),
    );
}

#[test]
fn snapshot_and_catch_up_match_normalized_golden() {
    let export = sample_export();
    let graph = SnapshotGraph::from(export.clone());
    let message_event = Event::MessagePosted {
        occurred_at: timestamp(),
        workspace_id: export.workspace.id,
        channel_id: export.channels[0].channel.id,
        thread_id: export.threads[0].id,
        dm_conversation_id: None,
        message: export.messages[0].clone(),
    };
    let payload = serde_json::to_value(&message_event).expect("event payload");
    let first = link_for(10, &json!({"kind": "checkpoint"}), None).expect("first link");
    let next = link_for(11, &payload, Some(&first)).expect("next link");
    let snapshot = LogSnapshot::new(
        export.workspace.id,
        Some(first.clone()),
        Some(first.clone()),
        graph,
        true,
    )
    .expect("snapshot");
    let stored = StoredEvent {
        id: next.id,
        lsn: next.lsn,
        kind: EventKind::MessagePosted,
        workspace_id: Some(export.workspace.id),
        channel_id: Some(export.channels[0].channel.id),
        thread_id: Some(export.threads[0].id),
        payload,
        occurred_at: timestamp(),
        prev_hash: next.prev_hash,
        content_hash: next.content_hash,
    };
    let catch_up = CatchUpPage::new(
        export.workspace.id,
        first.lsn,
        stored.lsn,
        stored.lsn,
        vec![stored],
        Some(&first),
        false,
    );
    assert_golden(
        "normalized-event-frames.json",
        json!({"snapshot": snapshot, "catch_up": catch_up}),
    );
}
