//! Postgres deep workspace purge (Cluster 28).

use std::time::Duration;

use maidan_auth::{hash_secret, TokenSecret};
use maidan_search::{EmbeddingProvider, HashV1Provider, PostgresSearch, Search};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn postgres_deep_purge_removes_related_rows() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
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
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::new(pool.clone());
    let search = PostgresSearch::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "pg-deep-purge".into(),
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
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: "classified".into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let provider = HashV1Provider;
    let embedding = provider.embed("classified").unwrap();
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
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
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

    let result = store.purge_workspace_messages(ws.id).await.unwrap();
    assert_eq!(result.messages_purged, 1);
    assert_eq!(result.embeddings_removed, 1);
    assert_eq!(result.references_removed, 1);
    assert_eq!(result.api_tokens_revoked, 1);
    assert_eq!(result.events_removed, 1);
    assert!(store.list_messages(th.id, 10).await.unwrap().is_empty());
    assert!(store
        .list_events_after(ws.id, 0, 10)
        .await
        .unwrap()
        .is_empty());
}
