//! The exact bytes of a thread context pack. A provider prompt cache matches a
//! byte-identical prefix, so the pack's layout and serialization are a wire
//! contract down to the byte: a reordered field, a new default field or a
//! changed separator moves every agent's cached prefix. The fixture uses fixed
//! ids and timestamps and feeds rows in a shuffled order, so the golden pins the
//! ordering rules as well as the shape.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use maidan_types::{
    assemble_thread_context, fold_messages_to_budget, Artifact, ArtifactId, ArtifactKind,
    ChannelClosedResult, ChannelId, ClaimLeaseId, ContentBlock, GlossaryTerm, MemberId, Message,
    MessageEdit, MessageId, PackParts, ParentGrounding, RefSide, Reference, RelationKind,
    ReviewDecision, Thread, ThreadContext, ThreadId, ThreadReview, ThreadState, ThreadTransition,
    WorkspaceId,
};
use serde_json::json;
use uuid::Uuid;

const WRITE_ENV: &str = "MAIDAN_WRITE_PORTABLE_GOLDENS";

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 + secs, 0).expect("fixed timestamp")
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

fn message(n: u128, thread_id: ThreadId, author: MemberId, body: &str) -> Message {
    Message {
        id: MessageId(id(n)),
        thread_id,
        author_id: author,
        body: body.into(),
        metadata: json!({}),
        content: None,
        posted_at: at(n as i64),
        edited_at: None,
        tombstoned_at: None,
    }
}

fn fixture() -> PackParts {
    let workspace_id = WorkspaceId(id(1));
    let channel_id = ChannelId(id(2));
    let thread_id = ThreadId(id(3));
    let parent_id = ThreadId(id(5));
    let owner = MemberId(id(6));
    let worker = MemberId(id(7));
    let thread = Thread {
        id: thread_id,
        channel_id,
        parent_thread_id: Some(parent_id),
        title: Some("Fix the OIDC cache".into()),
        state: ThreadState::InProgress,
        assignee_id: Some(worker),
        assignment_expires_at: Some(at(900)),
        claim_lease_id: Some(ClaimLeaseId(id(8))),
        work_started_at: Some(at(310)),
        owner_id: Some(owner),
        created_at: at(0),
        updated_at: at(420),
        tombstoned_at: None,
    };

    let mut page: Vec<Message> = (100..120)
        .map(|n| message(n, thread_id, worker, &format!("step {n}: checked the discovery TTL")))
        .collect();
    page[0].body = "The auth service returns 500 when the discovery document is stale.".into();
    page[17].metadata = json!({ "artifacts": [sha('b'), { "sha256": sha('a') }] });
    page[18].content = Some(vec![ContentBlock::Text {
        text: "re-fetch on a stale read".into(),
    }]);
    page[18].body = "re-fetch on a stale read".into();
    page[18].edited_at = Some(at(400));
    let (messages, elision) = fold_messages_to_budget(page, 220);

    let term = |n, term: &str, definition: &str| GlossaryTerm {
        id: id(n),
        workspace_id,
        term: term.into(),
        definition: definition.into(),
        aliases: vec![],
        created_by: owner,
        created_at: at(-100),
        updated_at: at(-100),
    };
    let closed = |n, secs, summary: &str| ChannelClosedResult {
        thread_id: ThreadId(id(n)),
        title: Some(format!("decision {n}")),
        state: ThreadState::Closed,
        result: json!({ "summary": summary }),
        produced_by: owner,
        produced_at: at(secs),
    };
    let reference = |n| Reference {
        id: id(n),
        src_kind: RefSide::Thread,
        src_id: thread_id.0,
        dst_kind: RefSide::Thread,
        dst_id: parent_id.0,
        relation: RelationKind::from_wire("relates_to"),
        created_at: at(5),
    };
    let edit = |n, before: &str, after: &str| MessageEdit {
        id: n,
        message_id: MessageId(id(118)),
        editor_id: worker,
        body_before: before.into(),
        body_after: after.into(),
        edited_at: at(400),
    };
    let transition = |n, from, to| ThreadTransition {
        id: id(n),
        thread_id,
        from_state: from,
        to_state: to,
        actor_id: worker,
        occurred_at: at(300),
    };
    let artifact = |n, c, name: &str| Artifact {
        id: ArtifactId(id(n)),
        sha256: sha(c),
        size_bytes: 128,
        mime_type: Some("text/plain".into()),
        filename: Some(name.into()),
        kind: ArtifactKind::Attachment,
        uploaded_by: Some(worker),
        created_at: at(200),
        tombstoned_at: None,
    };

    PackParts {
        workspace_id,
        thread,
        required_skills: vec!["rust".into(), "oidc".into()],
        glossary: vec![
            term(31, "lease", "how long a claim lasts"),
            term(30, "LSN", "log sequence number"),
        ],
        parent_grounding: Some(ParentGrounding {
            thread_id: parent_id,
            title: Some("Harden auth".into()),
            state: ThreadState::InProgress,
            opening_message: Some(message(50, parent_id, owner, "Make sign-in survive IdP blips")),
            latest_result: Some(json!({ "decision": "fan out" })),
        }),
        // Newest first, as the store lists them; two tie on `produced_at`.
        closed_results: vec![
            closed(42, 50, "keep the 600 s lease"),
            closed(41, 50, "use Postgres"),
            closed(40, 10, "adopt UUIDv7"),
        ],
        messages,
        elision,
        edits: vec![edit(7, "refetch", "re-fetch on a stale read"), edit(6, "", "refetch")],
        include_edit_bodies: false,
        references: vec![reference(61), reference(60)],
        artifacts: vec![artifact(91, 'b', "trace.txt"), artifact(90, 'a', "notes.txt")],
        transitions: vec![
            transition(71, ThreadState::InProgress, ThreadState::InReview),
            transition(70, ThreadState::Open, ThreadState::InProgress),
        ],
        change_requests: vec![ThreadReview {
            thread_id,
            reviewer_id: owner,
            decision: ReviewDecision::RequestChanges,
            note: Some("cover the 404 path".into()),
            actor_id: None,
            created_at: at(350),
            updated_at: at(350),
            dismissed_at: None,
        }],
        as_of: None,
        next_message_cursor: Some(id(119).to_string()),
    }
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/thread-context-pack.json")
}

#[test]
fn a_thread_context_pack_serializes_to_the_golden_bytes() {
    let pack = assemble_thread_context(fixture());
    let actual = String::from_utf8(pack.to_bytes().unwrap()).unwrap();
    let path = golden_path();
    if std::env::var(WRITE_ENV).ok().as_deref() == Some("1") {
        std::fs::write(&path, format!("{actual}\n")).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        actual,
        expected.trim_end_matches('\n'),
        "context pack bytes drifted from {}; every cached prefix moves with them. \
         Rerun with {WRITE_ENV}=1 only after reviewing the change",
        path.display()
    );
}

#[test]
fn the_golden_pack_reads_back_to_the_same_bytes() {
    let bytes = assemble_thread_context(fixture()).to_bytes().unwrap();
    let parsed: ThreadContext = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.to_bytes().unwrap(), bytes);
}
