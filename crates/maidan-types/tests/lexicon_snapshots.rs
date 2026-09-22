//! Canon snapshot tests over normalized `$type` wire shapes.
//!
//! A new [`EventKind`] fails the exhaustive `sample_event` match until a
//! fixture is produced. The committed JSON is the review surface for SDK 0.2.

use chrono::{DateTime, Utc};
use maidan_types::{
    catalog, event_schema, event_wire, inject_type, normalize, normalized_event_wire, pack_files,
    schema_filename, waiter_result_schema, Artifact, ArtifactKind, BlockedReason, Channel, Event,
    EventKind, Member, MemberKind, Message, RefSide, Reference, RelationKind, Thread, ThreadState,
    Workspace, EXAMPLE_PLAN_RESULT_KIND, EXAMPLE_REVIEW_RESULT_KIND, WAITER_RESULT_SCHEMA,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

fn ts() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("fixed unix seconds")
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn workspace() -> Workspace {
    Workspace {
        id: maidan_types::WorkspaceId(id(1)),
        name: "ws".into(),
        created_at: ts(),
        updated_at: ts(),
        tombstoned_at: None,
    }
}

fn member() -> Member {
    Member {
        id: maidan_types::MemberId(id(2)),
        workspace_id: maidan_types::WorkspaceId(id(1)),
        handle: "agent".into(),
        display_name: Some("Agent".into()),
        kind: MemberKind::Agent,
        created_at: ts(),
        updated_at: ts(),
        tombstoned_at: None,
    }
}

fn channel() -> Channel {
    Channel {
        id: maidan_types::ChannelId(id(3)),
        workspace_id: maidan_types::WorkspaceId(id(1)),
        name: "general".into(),
        topic: Some("talk".into()),
        private: false,
        created_at: ts(),
        updated_at: ts(),
        tombstoned_at: None,
    }
}

fn thread() -> Thread {
    Thread {
        id: maidan_types::ThreadId(id(4)),
        channel_id: maidan_types::ChannelId(id(3)),
        parent_thread_id: None,
        title: Some("task".into()),
        state: ThreadState::Open,
        assignee_id: None,
        assignment_expires_at: None,
        claim_lease_id: None,
        work_started_at: None,
        owner_id: None,
        created_at: ts(),
        updated_at: ts(),
        tombstoned_at: None,
    }
}

fn message() -> Message {
    Message {
        id: maidan_types::MessageId(id(5)),
        thread_id: maidan_types::ThreadId(id(4)),
        author_id: maidan_types::MemberId(id(2)),
        body: "hello".into(),
        metadata: serde_json::json!({}),
        content: None,
        posted_at: ts(),
        edited_at: None,
        tombstoned_at: None,
    }
}

fn reference() -> Reference {
    Reference {
        id: id(6),
        src_kind: RefSide::Message,
        src_id: id(5),
        dst_kind: RefSide::Thread,
        dst_id: id(4),
        relation: RelationKind::Supports,
        created_at: ts(),
    }
}

fn artifact() -> Artifact {
    Artifact {
        id: maidan_types::ArtifactId(id(7)),
        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        size_bytes: 4,
        mime_type: Some("text/plain".into()),
        kind: ArtifactKind::Attachment,
        uploaded_by: Some(maidan_types::MemberId(id(2))),
        created_at: ts(),
        tombstoned_at: None,
    }
}

/// Deterministic sample for every kind. Compile-time tripwire: a new variant
/// without an arm here fails to build (no wildcard).
fn sample_event(kind: EventKind) -> Event {
    let occurred_at = ts();
    let workspace_id = maidan_types::WorkspaceId(id(1));
    let channel_id = maidan_types::ChannelId(id(3));
    let thread_id = maidan_types::ThreadId(id(4));
    let member_id = maidan_types::MemberId(id(2));
    let message_id = maidan_types::MessageId(id(5));
    match kind {
        EventKind::WorkspaceCreated => Event::WorkspaceCreated {
            occurred_at,
            workspace: workspace(),
        },
        EventKind::MemberJoined => Event::MemberJoined {
            occurred_at,
            workspace_id,
            member: member(),
        },
        EventKind::ChannelCreated => Event::ChannelCreated {
            occurred_at,
            workspace_id,
            channel: channel(),
        },
        EventKind::ThreadCreated => Event::ThreadCreated {
            occurred_at,
            workspace_id,
            channel_id,
            thread: thread(),
        },
        EventKind::ThreadStateChanged => Event::ThreadStateChanged {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            actor_id: member_id,
            from_state: ThreadState::Open,
            to_state: ThreadState::Closed,
            thread: thread(),
        },
        EventKind::ThreadAssignmentChanged => Event::ThreadAssignmentChanged {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            actor_id: member_id,
            previous_assignee_id: None,
            assignee_id: Some(member_id),
            note: Some("handoff".into()),
            thread: thread(),
        },
        EventKind::ThreadReady => Event::ThreadReady {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            thread: thread(),
        },
        EventKind::ThreadResultSet => Event::ThreadResultSet {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            produced_by: member_id,
        },
        EventKind::ApprovalRequested => Event::ApprovalRequested {
            occurred_at,
            workspace_id,
            channel_id: Some(channel_id),
            thread_id: Some(thread_id),
            gate_id: maidan_types::ApprovalGateId(id(10)),
            requested_by: member_id,
        },
        EventKind::BlockedResolved => Event::BlockedResolved {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            reason: BlockedReason::Human,
            resolved_by: member_id,
        },
        EventKind::ClaimExpired => Event::ClaimExpired {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            member_id,
            thread: thread(),
        },
        EventKind::ClaimFailed => Event::ClaimFailed {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            member_id,
            reason: "tokens".into(),
            thread: thread(),
        },
        EventKind::ThreadLanded => Event::ThreadLanded {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            repo: "example/repo".into(),
            pr_number: 1,
            merged_by: Some("octocat".into()),
            merge_commit_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            title: Some("land".into()),
        },
        EventKind::WaitTimedOut => Event::WaitTimedOut {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            policy: "notify".into(),
            reason: Some("waiting".into()),
        },
        EventKind::ScheduleSkipped => Event::ScheduleSkipped {
            occurred_at,
            workspace_id,
            channel_id,
            schedule_id: maidan_types::TaskScheduleId(id(8)),
            recipe_id: maidan_types::RecipeId(id(9)),
            reason: "prior run still in flight".into(),
        },
        EventKind::ThreadSpawnDenied => Event::ThreadSpawnDenied {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            member_id: Some(member_id),
            axis: "children".into(),
            limit: 3,
            observed: 3,
        },
        EventKind::ProjectorMisconfigured => Event::ProjectorMisconfigured {
            occurred_at,
            workspace_id,
            channel_id: Some(channel_id),
            thread_id,
            surface: "github".into(),
            selector: "example/repo#1".into(),
            error: "401".into(),
        },
        EventKind::MessagePosted => Event::MessagePosted {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            dm_conversation_id: None,
            message: message(),
        },
        EventKind::MessageEdited => Event::MessageEdited {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            dm_conversation_id: None,
            editor_id: member_id,
            message: message(),
        },
        EventKind::MessageTombstoned => Event::MessageTombstoned {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            dm_conversation_id: None,
            message_id,
        },
        EventKind::MentionRecorded => Event::MentionRecorded {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
        },
        EventKind::VoteCast => Event::VoteCast {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
            vote_kind: "up".into(),
        },
        EventKind::ReactionAdded => Event::ReactionAdded {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
            emoji: "👍".into(),
        },
        EventKind::ReactionRemoved => Event::ReactionRemoved {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
            emoji: "👍".into(),
        },
        EventKind::MessagePinned => Event::MessagePinned {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            message_id,
            member_id,
        },
        EventKind::MessageUnpinned => Event::MessageUnpinned {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            message_id,
            member_id,
        },
        EventKind::ReferenceAdded => Event::ReferenceAdded {
            occurred_at,
            reference: reference(),
        },
        EventKind::ArtifactUpserted => Event::ArtifactUpserted {
            occurred_at,
            artifact: artifact(),
        },
        EventKind::MemoryBlockUpdated => Event::MemoryBlockUpdated {
            occurred_at,
            workspace_id,
            block_id: maidan_types::MemoryBlockId(id(10)),
            label: "persona".into(),
            updated_by: member_id,
        },
    }
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lexicon")
}

fn contracts_lexicon_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/lexicon")
}

fn pretty(value: &Value) -> String {
    let mut s = serde_json::to_string_pretty(value).expect("pretty");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn read_or_empty(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn sample_event_kind_matches_and_is_exhaustive() {
    for &kind in EventKind::ALL {
        let event = sample_event(kind);
        assert_eq!(event.kind(), kind);
        match kind {
            EventKind::WorkspaceCreated
            | EventKind::MemberJoined
            | EventKind::ChannelCreated
            | EventKind::ThreadCreated
            | EventKind::ThreadStateChanged
            | EventKind::ThreadAssignmentChanged
            | EventKind::ThreadReady
            | EventKind::ThreadResultSet
            | EventKind::ApprovalRequested
            | EventKind::BlockedResolved
            | EventKind::ClaimExpired
            | EventKind::ClaimFailed
            | EventKind::ThreadLanded
            | EventKind::WaitTimedOut
            | EventKind::ScheduleSkipped
            | EventKind::ThreadSpawnDenied
            | EventKind::ProjectorMisconfigured
            | EventKind::MessagePosted
            | EventKind::MessageEdited
            | EventKind::MessageTombstoned
            | EventKind::MentionRecorded
            | EventKind::VoteCast
            | EventKind::ReactionAdded
            | EventKind::ReactionRemoved
            | EventKind::MessagePinned
            | EventKind::MessageUnpinned
            | EventKind::ReferenceAdded
            | EventKind::ArtifactUpserted
            | EventKind::MemoryBlockUpdated => {}
        }
    }
}

#[test]
fn normalized_event_snapshots_match_committed_fixture() {
    let mut map = BTreeMap::new();
    for &kind in EventKind::ALL {
        let type_id = kind.type_id();
        let wire = normalized_event_wire(&sample_event(kind)).expect("wire");
        assert_eq!(
            wire.get("$type").and_then(Value::as_str),
            Some(type_id.as_str())
        );
        assert_eq!(
            wire.get("kind").and_then(Value::as_str),
            Some(kind.as_str())
        );
        map.insert(type_id, wire);
    }
    let got = pretty(&Value::Object(map.into_iter().collect()));
    let path = fixtures_dir().join("normalized-events.json");
    if std::env::var("MAIDAN_WRITE_LEXICON").ok().as_deref() == Some("1") {
        std::fs::create_dir_all(fixtures_dir()).expect("mkdir");
        std::fs::write(&path, &got).expect("write snapshot");
    }
    let expected = read_or_empty(&path);
    assert_eq!(
        got, expected,
        "normalized event snapshot drift — rerun with MAIDAN_WRITE_LEXICON=1 if intentional"
    );
}

#[test]
fn lexicon_pack_files_match_generation() {
    let dir = contracts_lexicon_dir();
    if std::env::var("MAIDAN_WRITE_LEXICON").ok().as_deref() == Some("1") {
        std::fs::create_dir_all(&dir).expect("mkdir");
        for (name, body) in pack_files() {
            std::fs::write(dir.join(name), body).expect("write pack file");
        }
    }
    let generated = pack_files();
    assert!(!generated.is_empty());
    for (name, body) in &generated {
        let path = dir.join(name);
        let expected = read_or_empty(&path);
        assert_eq!(
            body,
            &expected,
            "{} drift — rerun with MAIDAN_WRITE_LEXICON=1 if intentional",
            path.display()
        );
    }
    // No stray files in the pack directory (except we allow only generated names).
    let mut expected_names: Vec<String> = generated.iter().map(|(n, _)| n.clone()).collect();
    expected_names.sort();
    let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    on_disk.sort();
    assert_eq!(
        on_disk, expected_names,
        "unexpected files in contracts/lexicon"
    );
}

#[test]
fn event_schema_const_matches_type_id() {
    for &kind in EventKind::ALL {
        let schema = event_schema(kind);
        assert_eq!(schema["$id"], kind.type_id());
        assert_eq!(schema["properties"]["$type"]["const"], kind.type_id());
        assert_eq!(schema["properties"]["kind"]["const"], kind.as_str());
        assert_eq!(schema["additionalProperties"], true);
        assert_eq!(
            schema_filename(&kind.type_id()),
            format!("maidan.event.{}.1.json", kind.as_str())
        );
    }
}

#[test]
fn catalog_filename_set_covers_waiter_examples() {
    let catalog = catalog();
    let types = catalog["types"].as_array().expect("types");
    let as_str: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
    assert!(as_str.contains(&WAITER_RESULT_SCHEMA));
    assert!(as_str.contains(&EXAMPLE_REVIEW_RESULT_KIND));
    assert!(as_str.contains(&EXAMPLE_PLAN_RESULT_KIND));
}

#[test]
fn waiter_fixture_normalizes_with_type_without_rewriting_the_lock() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/waiter_result_v1.json")).expect("fixture");
    assert_eq!(
        fixture.get("schema").and_then(Value::as_str),
        Some(WAITER_RESULT_SCHEMA)
    );
    assert!(fixture.get("$type").is_none());
    let mut stamped = fixture.clone();
    inject_type(&mut stamped, WAITER_RESULT_SCHEMA);
    let got = pretty(&normalize(stamped));
    let path = fixtures_dir().join("normalized-waiter-result.json");
    if std::env::var("MAIDAN_WRITE_LEXICON").ok().as_deref() == Some("1") {
        std::fs::create_dir_all(fixtures_dir()).expect("mkdir");
        std::fs::write(&path, &got).expect("write");
    }
    let expected = read_or_empty(&path);
    assert_eq!(
        got, expected,
        "normalized waiter snapshot drift — rerun with MAIDAN_WRITE_LEXICON=1 if intentional"
    );
    assert_eq!(waiter_result_schema()["$id"], WAITER_RESULT_SCHEMA);
}

#[test]
fn breaking_type_is_a_new_id() {
    let event = sample_event(EventKind::MessagePosted);
    let mut wire = event_wire(&event).expect("wire");
    wire["$type"] = Value::String("maidan.event.message_posted/2".into());
    // `/2` does not parse as `/1`.
    assert_eq!(
        EventKind::parse_type_id(wire["$type"].as_str().unwrap()),
        None
    );
    // The `/1` event still deserializes (unknown $type ignored).
    let parsed: Event = serde_json::from_value(wire).expect("still an Event");
    assert_eq!(parsed.kind(), EventKind::MessagePosted);
}

#[test]
fn new_optional_field_does_not_change_type() {
    let event = sample_event(EventKind::ThreadResultSet);
    let mut wire = event_wire(&event).expect("wire");
    let type_id = wire["$type"].clone();
    wire["future_field"] = Value::String("added".into());
    let parsed: Event = serde_json::from_value(wire.clone()).expect("unknown ignored");
    assert_eq!(parsed.kind().type_id(), type_id.as_str().unwrap());
    assert_eq!(parsed.kind(), EventKind::ThreadResultSet);
}
