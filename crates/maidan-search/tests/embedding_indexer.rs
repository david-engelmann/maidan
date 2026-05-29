//! EmbeddingHandler writes vectors on MessagePosted (Postgres only).

use std::{sync::Arc, time::Duration};

use maidan_bus::{EventBus, InMemoryBus};
use maidan_search::{EmbeddingHandler, HashV1Provider, PostgresSearch, Search};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn embedding_handler_upserts_on_message_posted() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping embedding_handler_upserts: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");

    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn Search> = Arc::new(PostgresSearch::new(pool));
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::with_capacity(64));
    let handler = Arc::new(EmbeddingHandler::new(
        store.clone(),
        search.clone(),
        Arc::new(HashV1Provider),
    ));
    let indexer = maidan_search::Indexer::new(bus.clone(), handler).spawn();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws = store
        .create_workspace(NewWorkspace {
            name: "emb-ws".to_string(),
        })
        .await
        .expect("ws");
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "emb".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("thread");
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: author.id,
            body: "semantic body".to_string(),
            metadata: serde_json::json!({}),
        })
        .await
        .expect("message");

    bus.publish(maidan_types::BusEnvelope::synthetic(
        maidan_types::Event::MessagePosted {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel_id: ch.id,
            thread_id: thread.id,
            dm_conversation_id: None,
            message: msg.clone(),
        },
    ))
    .await
    .expect("publish");

    tokio::time::sleep(Duration::from_millis(300)).await;

    let query = maidan_search::hash_embedding("semantic body");
    let hits = search
        .semantic_search(
            ws.id,
            &query,
            5,
            &maidan_search::SearchFilters::default(),
            maidan_search::model_name(),
        )
        .await
        .expect("semantic search");
    assert!(
        hits.iter().any(|h| h.message_id == msg.id),
        "expected embedding-indexed message in semantic results"
    );

    indexer.shutdown().await;
}
