//! Golden file for agent-facing event taxonomy (Cluster 59).

use std::path::PathBuf;

use maidan_types::EventKind;

#[test]
fn event_kinds_match_contract_file() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../../contracts/event-kinds.json");
    let expected: Vec<String> = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("contract json");
    let mut actual: Vec<String> = [
        EventKind::WorkspaceCreated,
        EventKind::MemberJoined,
        EventKind::ChannelCreated,
        EventKind::ThreadCreated,
        EventKind::ThreadStateChanged,
        EventKind::MessagePosted,
        EventKind::MessageEdited,
        EventKind::MessageTombstoned,
        EventKind::MentionRecorded,
        EventKind::VoteCast,
        EventKind::ReactionAdded,
        EventKind::ReactionRemoved,
        EventKind::MessagePinned,
        EventKind::MessageUnpinned,
        EventKind::ReferenceAdded,
        EventKind::ArtifactUpserted,
    ]
    .into_iter()
    .map(|k| k.as_str().to_string())
    .collect();
    actual.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(
        actual, expected,
        "update contracts/event-kinds.json if intentional"
    );
}
