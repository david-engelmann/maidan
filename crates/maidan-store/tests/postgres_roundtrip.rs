//! Integration test for the Postgres backend.
//!
//! Spins up a real Postgres testcontainer, applies migrations, exercises
//! every CRUD path on `Store`, and verifies the data round-trips. Skips
//! gracefully if Docker is not available so the test suite still runs in
//! environments without containers.

use std::time::Duration;

use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{
    MemberKind, NewArtifact, NewAuditEvent, NewChannel, NewMember, NewMessage, NewReference,
    NewThread, NewVote, NewWorkspace, RefSide,
};
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

async fn spawn() -> Option<(PostgresStore, testcontainers::ContainerAsync<Postgres>)> {
    let container = match Postgres::default().start().await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping postgres_roundtrip: docker unavailable ({err})");
            return None;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect to testcontainer postgres");
    run_postgres_migrations(&pool)
        .await
        .expect("apply migrations");
    Some((PostgresStore::new(pool), container))
}

#[tokio::test]
async fn full_roundtrip() {
    let Some((store, _container)) = spawn().await else {
        return;
    };

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

    // duplicate handle conflicts
    let dup = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "alice".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await;
    assert!(
        matches!(dup, Err(maidan_store::StoreError::Conflict(_))),
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
        })
        .await
        .expect("post message 1");
    let msg2 = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: bot.id,
            body: "world".to_string(),
            metadata: serde_json::json!({}),
        })
        .await
        .expect("post message 2");

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
    // idempotent
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
            kind: "screenshot".to_string(),
            uploaded_by: Some(alice.id),
        })
        .await
        .expect("upsert artifact");
    let same = store
        .upsert_artifact(NewArtifact {
            sha256: artifact.sha256.clone(),
            size_bytes: 42,
            mime_type: None,
            kind: "screenshot".to_string(),
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

#[tokio::test]
async fn migrations_are_idempotent() {
    let Some((store, _container)) = spawn().await else {
        return;
    };
    // spawn() already ran migrations once; running again must not error.
    run_postgres_migrations(store.pool())
        .await
        .expect("re-apply migrations");
}
