//! Shared backend-agnostic test scenarios for the `Store` trait.
//!
//! Both `postgres_roundtrip.rs` and `sqlite_roundtrip.rs` call into
//! these helpers with their respective concrete stores so the same
//! assertions run against both dialects.

use maidan_fsm::ThreadAction;
use maidan_store::{Store, StoreError};
use maidan_types::*;

#[allow(dead_code)]
pub async fn run_full_roundtrip(store: &dyn Store) {
    store.health_check().await.expect("health");

    let workspace = store
        .create_workspace(NewWorkspace {
            name: "acme".to_string(),
        })
        .await
        .expect("create workspace");
    assert_eq!(workspace.name, "acme");
    assert_eq!(
        store
            .get_workspace(workspace.id)
            .await
            .expect("get workspace")
            .id,
        workspace.id
    );

    let alice = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "alice".to_string(),
            display_name: Some("Alice".to_string()),
            kind: MemberKind::Human,
        })
        .await
        .expect("create alice");
    let bot = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "bot".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("create bot");
    let members = store
        .list_members(workspace.id)
        .await
        .expect("list members");
    assert_eq!(members.len(), 2);

    let dup = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "alice".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await;
    assert!(
        matches!(dup, Err(StoreError::Conflict(_))),
        "expected conflict, got {dup:?}"
    );

    let channel = store
        .create_channel(NewChannel {
            workspace_id: workspace.id,
            name: "general".to_string(),
            topic: Some("everything".to_string()),
            private: false,
        })
        .await
        .expect("create channel");
    assert_eq!(
        store
            .list_channels(workspace.id)
            .await
            .expect("list channels")
            .len(),
        1
    );

    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("first thread".to_string()),
        })
        .await
        .expect("create thread");
    assert_eq!(
        store
            .list_threads(channel.id)
            .await
            .expect("list threads")
            .len(),
        1
    );

    let msg1 = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: alice.id,
            body: "hello".to_string(),
            metadata: serde_json::json!({"client": "test"}),
            content: None,
        })
        .await
        .expect("post msg1");
    let msg2 = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: bot.id,
            body: "world".to_string(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("post msg2");

    let listed = store
        .list_messages(thread.id, 10)
        .await
        .expect("list messages");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, msg1.id);
    assert_eq!(listed[1].id, msg2.id);
    assert_eq!(listed[0].metadata["client"], "test");

    store
        .record_mention(msg1.id, bot.id)
        .await
        .expect("record mention");
    store
        .record_mention(msg1.id, bot.id)
        .await
        .expect("re-record mention");
    let mentions = store
        .list_mentions_for_member(bot.id, 10)
        .await
        .expect("list mentions");
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].message_id, msg1.id);

    store
        .cast_vote(NewVote {
            message_id: msg1.id,
            member_id: bot.id,
            kind: "approve".to_string(),
        })
        .await
        .expect("cast vote");
    store
        .cast_vote(NewVote {
            message_id: msg1.id,
            member_id: bot.id,
            kind: "approve".to_string(),
        })
        .await
        .expect("idempotent vote");
    let votes = store
        .list_votes_for_message(msg1.id)
        .await
        .expect("list votes");
    assert_eq!(votes.len(), 1);

    let reference = store
        .add_reference(NewReference {
            src_kind: RefSide::Message,
            src_id: msg2.id.0,
            dst_kind: RefSide::Message,
            dst_id: msg1.id.0,
            relation: "replies-to".to_string(),
        })
        .await
        .expect("add reference");
    let outgoing = store
        .list_references_from(RefSide::Message, msg2.id.0)
        .await
        .expect("list references");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].id, reference.id);

    let artifact = store
        .upsert_artifact(NewArtifact {
            sha256: "abcd1234".repeat(8),
            size_bytes: 42,
            mime_type: Some("image/png".to_string()),
            kind: ArtifactKind::Screenshot,
            uploaded_by: Some(alice.id),
        })
        .await
        .expect("upsert artifact");
    let same = store
        .upsert_artifact(NewArtifact {
            sha256: artifact.sha256.clone(),
            size_bytes: 42,
            mime_type: None,
            kind: ArtifactKind::Screenshot,
            uploaded_by: None,
        })
        .await
        .expect("upsert idempotent");
    assert_eq!(same.id, artifact.id);
    assert_eq!(same.mime_type.as_deref(), Some("image/png"));
    let fetched = store
        .get_artifact_by_sha(&artifact.sha256)
        .await
        .expect("get artifact");
    assert_eq!(fetched.id, artifact.id);

    let audit = store
        .append_audit(NewAuditEvent {
            actor_id: Some(alice.id),
            action: "post_message".to_string(),
            target_kind: Some("message".to_string()),
            target_id: Some(msg1.id.0),
            metadata: serde_json::json!({"len": 5}),
        })
        .await
        .expect("append audit");
    let audits = store.list_audit(10).await.expect("list audit");
    assert!(!audits.is_empty());
    assert_eq!(audits[0].id, audit.id);

    store.tombstone_message(msg2.id).await.expect("tombstone");
    let remaining = store
        .list_messages(thread.id, 10)
        .await
        .expect("list after tombstone");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, msg1.id);

    let direct = store.get_message(msg2.id).await.expect("get tombstoned");
    assert!(direct.tombstoned_at.is_some());
    assert_eq!(direct.body, "");
}

/// Smaller cross-dialect parity scenario: a few inserts + selects, used
/// by `dialect_parity.rs` to assert two stores return identical results
/// for the same input sequence.
#[allow(dead_code)]
pub async fn run_parity_scenario(store: &dyn Store) -> ParitySnapshot {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "parity-ws".to_string(),
        })
        .await
        .expect("ws");
    let m = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".to_string(),
            display_name: Some("Alice".to_string()),
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let c = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "ch".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let t = store
        .create_thread(NewThread {
            channel_id: c.id,
            parent_thread_id: None,
            title: Some("t".to_string()),
        })
        .await
        .expect("thread");
    let m1 = store
        .post_message(NewMessage {
            thread_id: t.id,
            author_id: m.id,
            body: "one".to_string(),
            metadata: serde_json::json!({"i": 1}),
            content: None,
        })
        .await
        .expect("m1");
    let m2 = store
        .post_message(NewMessage {
            thread_id: t.id,
            author_id: m.id,
            body: "two".to_string(),
            metadata: serde_json::json!({"i": 2}),
            content: None,
        })
        .await
        .expect("m2");
    let listed = store.list_messages(t.id, 10).await.expect("list");

    // FSM transition: Open -> InReview, recorded through the store.
    let transition = store
        .transition_thread(t.id, m.id, ThreadAction::StartReview)
        .await
        .expect("transition");

    // Edit m1's body and count the recorded edits.
    let edited = store
        .edit_message(
            m1.id,
            m.id,
            EditMessage {
                body: "one-edited".to_string(),
                metadata: serde_json::json!({"i": 1, "edited": true}),
                content: None,
            },
        )
        .await
        .expect("edit");
    let edit_count = store
        .list_message_edits(m1.id, 10)
        .await
        .expect("list edits")
        .len();

    // React to m1 and read the emoji back.
    store
        .add_reaction(NewReaction {
            message_id: m1.id,
            member_id: m.id,
            emoji: "👍".to_string(),
        })
        .await
        .expect("react");
    let mut reaction_emojis: Vec<String> = store
        .list_reactions_for_message(m1.id)
        .await
        .expect("list reactions")
        .into_iter()
        .map(|r| r.emoji)
        .collect();
    reaction_emojis.sort();

    ParitySnapshot {
        workspace_name: ws.name,
        member_handle: m.handle,
        member_kind: m.kind,
        channel_name: c.name,
        thread_title: t.title,
        thread_state: t.state,
        message_bodies: listed.iter().map(|m| m.body.clone()).collect(),
        message_metadata: listed.iter().map(|m| m.metadata.clone()).collect(),
        message_ids: vec![m1.id.0, m2.id.0],
        thread_state_after_review: transition.to_state,
        edited_body: edited.body,
        edit_count,
        reaction_emojis,
    }
}

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub struct ParitySnapshot {
    pub workspace_name: String,
    pub member_handle: String,
    pub member_kind: MemberKind,
    pub channel_name: String,
    pub thread_title: Option<String>,
    pub thread_state: ThreadState,
    pub message_bodies: Vec<String>,
    pub message_metadata: Vec<serde_json::Value>,
    pub message_ids: Vec<uuid::Uuid>,
    pub thread_state_after_review: ThreadState,
    pub edited_body: String,
    pub edit_count: usize,
    pub reaction_emojis: Vec<String>,
}

/// Cluster 159: `channel_members` round-trip — add (idempotent upsert), query,
/// list, remove — exercised against both backends from the roundtrip suites.
#[allow(dead_code)]
pub async fn run_channel_members_scenario(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "cm-ws".into(),
        })
        .await
        .expect("workspace");
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("alice");
    let bob = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("bob");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .expect("channel");

    // No rows yet.
    assert!(!store.channel_is_member(ch.id, alice.id).await.unwrap());
    assert!(store.list_channel_members(ch.id).await.unwrap().is_empty());

    // Add alice as admin, bob as member.
    let a = store
        .add_channel_member(ch.id, alice.id, ChannelMemberRole::Admin)
        .await
        .expect("add alice");
    assert_eq!(a.role, ChannelMemberRole::Admin);
    store
        .add_channel_member(ch.id, bob.id, ChannelMemberRole::Member)
        .await
        .expect("add bob");
    assert!(store.channel_is_member(ch.id, alice.id).await.unwrap());
    assert!(store.channel_is_member(ch.id, bob.id).await.unwrap());
    assert_eq!(store.list_channel_members(ch.id).await.unwrap().len(), 2);

    // Idempotent upsert: re-adding alice as plain member updates the role, not a dup.
    let a2 = store
        .add_channel_member(ch.id, alice.id, ChannelMemberRole::Member)
        .await
        .expect("re-add alice");
    assert_eq!(a2.role, ChannelMemberRole::Member);
    assert_eq!(
        a2.created_at, a.created_at,
        "created_at preserved on upsert"
    );
    assert_eq!(store.list_channel_members(ch.id).await.unwrap().len(), 2);

    // Remove bob.
    store
        .remove_channel_member(ch.id, bob.id)
        .await
        .expect("remove bob");
    assert!(!store.channel_is_member(ch.id, bob.id).await.unwrap());
    assert_eq!(store.list_channel_members(ch.id).await.unwrap().len(), 1);
}
