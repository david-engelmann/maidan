//! Deep workspace purge: references, tokens, events, embeddings (Cluster 28).

use maidan_auth::{hash_secret, TokenSecret};
use maidan_search::{EmbeddingProvider, HashV1Provider, Search, SqliteSearch};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;
async fn seed_workspace(
    store: &SqliteStore,
    search: &SqliteSearch,
) -> (Workspace, Member, Thread, Message) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "deep-purge".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("topic".into()),
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: "secret content".into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let provider = HashV1Provider;
    let embedding = provider.embed("secret content").unwrap();
    search
        .upsert_embedding(msg.id, provider.model_name(), &embedding)
        .await
        .unwrap();
    store
        .add_reference(NewReference {
            src_kind: RefSide::Message,
            src_id: msg.id.0,
            dst_kind: RefSide::Thread,
            dst_id: th.id.0,
            relation: "relates_to".into(),
        })
        .await
        .unwrap();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: alice.id,
            token_hash: hash_secret(TokenSecret::generate().as_str()),
            label: Some("purge-me".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    store
        .upsert_artifact(NewArtifact {
            sha256: "aa".repeat(32),
            size_bytes: 4,
            mime_type: Some("text/plain".into()),
            kind: ArtifactKind::Attachment,
            uploaded_by: Some(alice.id),
        })
        .await
        .unwrap();
    store
        .append_event(&Event::MessagePosted {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel_id: ch.id,
            thread_id: th.id,
            dm_conversation_id: None,
            message: msg.clone(),
        })
        .await
        .unwrap();
    (ws, alice, th, msg)
}

#[tokio::test]
async fn deep_purge_removes_messages_embeddings_references_tokens_and_events() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool.clone());
    let search = SqliteSearch::new(pool);
    let (ws, _alice, th, msg) = seed_workspace(&store, &search).await;

    let result = store.purge_workspace_messages(ws.id).await.unwrap();
    assert_eq!(result.messages_tombstoned, 1);
    assert_eq!(result.messages_purged, 1);
    assert_eq!(result.embeddings_removed, 1);
    assert_eq!(result.references_removed, 1);
    assert_eq!(result.api_tokens_revoked, 1);
    assert_eq!(result.events_removed, 1);
    assert_eq!(result.artifacts_removed, 1);

    assert!(store.list_messages(th.id, 10).await.unwrap().is_empty());
    assert!(store.get_artifact_by_sha(&"aa".repeat(32)).await.is_err());
    let hits = search
        .search_messages(ws.id, "secret", 10, &Default::default())
        .await
        .unwrap();
    assert!(hits.is_empty());
    assert!(store
        .list_events_after(ws.id, 0, 10)
        .await
        .unwrap()
        .is_empty());
    let _ = store.get_message(msg.id).await.expect_err("message gone");
}

#[tokio::test]
async fn list_audit_for_workspace_scopes_to_workspace_actors_and_targets() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);
    let ws_a = store
        .create_workspace(NewWorkspace {
            name: "ws-a".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws_a.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ws_b = store
        .create_workspace(NewWorkspace {
            name: "ws-b".into(),
        })
        .await
        .unwrap();
    store
        .append_audit(NewAuditEvent {
            actor_id: Some(alice.id),
            action: "test.a".into(),
            target_kind: None,
            target_id: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    store
        .append_audit(NewAuditEvent {
            actor_id: None,
            action: "workspace.purge".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(ws_a.id.0),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    store
        .append_audit(NewAuditEvent {
            actor_id: None,
            action: "other".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(ws_b.id.0),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let scoped = store.list_audit_for_workspace(ws_a.id, 10).await.unwrap();
    assert_eq!(scoped.len(), 2);
    assert!(scoped.iter().all(|e| e.action != "other"));
}
