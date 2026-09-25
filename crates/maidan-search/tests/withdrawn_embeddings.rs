//! A withdrawn message leaves no embeddings. Its vectors are derived from the
//! words it withdrew: withdrawing it deletes them, and the indexer, which may
//! reach the message's posted event afterwards, embeds only a live message.
//! Both backends.

use std::sync::Arc;
use std::time::Duration;

use maidan_search::{
    sqlite_pool_options, EmbeddingProvider, HashV1Provider, PostgresSearch, Search, SqliteSearch,
};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{
    MemberKind, MessageId, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn post(store: &dyn Store) -> (MessageId, MessageId) {
    let ws = store
        .create_workspace(NewWorkspace { name: "emb".into() })
        .await
        .unwrap();
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for body in ["kept words", "withdrawn words"] {
        ids.push(
            store
                .post_message(NewMessage {
                    thread_id: thread.id,
                    author_id: author.id,
                    body: body.into(),
                    metadata: serde_json::json!({}),
                    content: None,
                })
                .await
                .unwrap()
                .id,
        );
    }
    (ids[0], ids[1])
}

/// Embed both messages, withdraw one, then embed it again as a late indexer
/// would; return the message ids still embedded.
async fn run_suite<F, Fut>(store: &dyn Store, search: &dyn Search, embedded: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Vec<uuid::Uuid>>,
{
    let (kept, withdrawn) = post(store).await;
    let provider = HashV1Provider;
    for id in [kept, withdrawn] {
        let vector = provider.embed("words").unwrap();
        search
            .upsert_embedding(id, provider.model_name(), &vector)
            .await
            .unwrap();
    }
    assert_eq!(embedded().await.len(), 2);

    store.tombstone_message(withdrawn).await.unwrap();
    assert_eq!(
        embedded().await,
        vec![kept.0],
        "withdrawing deletes its embeddings"
    );

    let late = provider.embed("withdrawn words").unwrap();
    search
        .upsert_embedding(withdrawn, provider.model_name(), &late)
        .await
        .unwrap();
    assert_eq!(
        embedded().await,
        vec![kept.0],
        "a late indexer does not bring them back"
    );
}

#[tokio::test]
async fn a_withdrawn_message_leaves_no_embeddings_sqlite() {
    let pool = sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search = SqliteSearch::new(pool.clone());
    run_suite(store.as_ref(), &search, || {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, uuid::Uuid>("SELECT message_id FROM maidan_emb_hash_v1")
                .fetch_all(&pool)
                .await
                .unwrap()
        }
    })
    .await;
}

#[tokio::test]
async fn a_withdrawn_message_leaves_no_embeddings_postgres() {
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
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search = PostgresSearch::new(pool.clone());
    run_suite(store.as_ref(), &search, || {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, uuid::Uuid>("SELECT message_id FROM maidan_emb_hash_v1")
                .fetch_all(&pool)
                .await
                .unwrap()
        }
    })
    .await;
}
