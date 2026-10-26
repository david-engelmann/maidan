//! A failed embedding is retried, and what retries cannot save is repaired
//! (414.2). Before, the live indexer counted a failed batch and dropped it:
//! the messages stayed searchable by text but never by meaning, until a full
//! reindex.

use std::sync::atomic::{AtomicU32, Ordering};
use std::{sync::Arc, time::Duration};

use maidan_bus::{EventBus, InMemoryBus};
use maidan_search::{
    BatchConfig, BatchingEmbeddingHandler, EmbeddingProvider, EmbeddingProviderError,
    HashV1Provider, IndexerMetrics, PostgresSearch, RetryPolicy, Search, SqliteSearch,
};
use maidan_store::usage_meters::UsageMeterBuffer;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{
    BusEnvelope, Event, MemberKind, Message, NewChannel, NewMember, NewMessage, NewThread,
    NewWorkspace, UsageMeter, WorkspaceId,
};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// The hash provider, failing its next `failures` batch calls
/// (`u32::MAX`: always).
struct Flaky {
    failures: AtomicU32,
}

impl EmbeddingProvider for Flaky {
    fn model_name(&self) -> &str {
        HashV1Provider.model_name()
    }
    fn dimension(&self) -> usize {
        HashV1Provider.dimension()
    }
    fn embed(&self, body: &str) -> Result<Vec<f32>, EmbeddingProviderError> {
        HashV1Provider.embed(body)
    }
    fn embed_batch(&self, bodies: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingProviderError> {
        let left = self.failures.load(Ordering::SeqCst);
        if left > 0 {
            if left != u32::MAX {
                self.failures.store(left - 1, Ordering::SeqCst);
            }
            return Err(EmbeddingProviderError::Remote(
                "provider unavailable".into(),
            ));
        }
        HashV1Provider.embed_batch(bodies)
    }
}

struct Fixture {
    store: Arc<dyn Store>,
    search: Arc<PostgresSearch>,
    pool: sqlx::PgPool,
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn fixture() -> Option<Fixture> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let search = Arc::new(PostgresSearch::new(pool.clone()));
    search.ensure_model(&HashV1Provider).await.unwrap();
    Some(Fixture {
        store: Arc::new(PostgresStore::for_tests(pool.clone())),
        search,
        pool,
        _container: container,
    })
}

/// Post `n` messages and return them with the event that announces each.
async fn post(store: &dyn Store, n: usize) -> Vec<(Message, Event)> {
    let ws = store
        .create_workspace(NewWorkspace { name: "e".into() })
        .await
        .unwrap();
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
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
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let mut out = Vec::new();
    for i in 0..n {
        let message = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: author.id,
                body: format!("body number {i}"),
                metadata: serde_json::json!({}),
                content: None,
            })
            .await
            .unwrap();
        let event = Event::MessagePosted {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            dm_conversation_id: None,
            message: message.clone(),
            sealed: None,
        };
        out.push((message, event));
    }
    out
}

async fn index_through(
    fx: &Fixture,
    provider: Arc<dyn EmbeddingProvider>,
    retry: RetryPolicy,
    events: Vec<Event>,
) -> Arc<IndexerMetrics> {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::with_capacity(64));
    let config = BatchConfig {
        queue_capacity: 64,
        batch_size: 8,
        retry,
    };
    let metrics = Arc::new(IndexerMetrics::new(config.queue_capacity));
    let handler = Arc::new(BatchingEmbeddingHandler::spawn(
        fx.store.clone(),
        fx.search.clone(),
        provider,
        config,
        metrics.clone(),
        None,
    ));
    let indexer = maidan_search::Indexer::new(bus.clone(), handler).spawn();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let n = events.len() as u64;
    for event in events {
        bus.publish(BusEnvelope::synthetic(event)).await.unwrap();
    }
    for _ in 0..200 {
        let done = metrics.embedded_total.load(Ordering::Relaxed)
            + metrics.failed_total.load(Ordering::Relaxed);
        if done >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    indexer.shutdown().await;
    metrics
}

#[tokio::test]
async fn a_transient_provider_failure_is_retried_not_dropped() {
    let Some(fx) = fixture().await else { return };
    let events = post(fx.store.as_ref(), 3).await;
    let provider = Arc::new(Flaky {
        failures: AtomicU32::new(2),
    });
    let retry = RetryPolicy {
        retries: 3,
        base: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
    };
    let metrics = index_through(
        &fx,
        provider,
        retry,
        events.into_iter().map(|(_, e)| e).collect(),
    )
    .await;
    assert_eq!(metrics.embedded_total.load(Ordering::Relaxed), 3);
    assert_eq!(metrics.failed_total.load(Ordering::Relaxed), 0);
    assert!(metrics.retries_total.load(Ordering::Relaxed) >= 2);
}

#[tokio::test]
async fn what_retries_cannot_save_the_repair_sweep_embeds_once() {
    let Some(fx) = fixture().await else { return };
    let events = post(fx.store.as_ref(), 3).await;
    let provider = Arc::new(Flaky {
        failures: AtomicU32::new(u32::MAX),
    });
    let metrics = index_through(
        &fx,
        provider,
        RetryPolicy::none(),
        events.into_iter().map(|(_, e)| e).collect(),
    )
    .await;
    assert_eq!(metrics.embedded_total.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.failed_total.load(Ordering::Relaxed), 3);

    let hash: Arc<dyn EmbeddingProvider> = Arc::new(HashV1Provider);
    // Another replica holds the repair lock: this one does nothing.
    let mut other = fx.pool.acquire().await.unwrap();
    let held: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(0x6d61_6964_656d_6272_i64)
        .fetch_one(&mut *other)
        .await
        .unwrap();
    assert!(held);
    let skipped = fx.search.embed_missing(&hash, 100, None).await.unwrap();
    assert_eq!(
        skipped.processed, 0,
        "a held lock means another replica is repairing"
    );
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(0x6d61_6964_656d_6272_i64)
        .execute(&mut *other)
        .await
        .unwrap();

    let repaired = fx.search.embed_missing(&hash, 100, None).await.unwrap();
    assert_eq!((repaired.processed, repaired.failed), (3, 0));
    let again = fx.search.embed_missing(&hash, 100, None).await.unwrap();
    assert_eq!(again.processed, 0, "nothing left missing");
}

fn workspace_of(events: &[(Message, Event)]) -> WorkspaceId {
    match &events[0].1 {
        Event::MessagePosted { workspace_id, .. } => *workspace_id,
        other => panic!("unexpected event {other:?}"),
    }
}

/// The embedding_calls counted per workspace in `meters`.
fn embedding_calls(meters: &UsageMeterBuffer, workspace: WorkspaceId) -> i64 {
    meters
        .take()
        .into_iter()
        .filter(|r| r.workspace_id == workspace && r.meter == UsageMeter::EmbeddingCalls)
        .map(|r| r.quantity)
        .sum()
}

#[tokio::test]
async fn the_repair_sweep_skips_a_cut_off_workspace_and_meters_the_other_postgres() {
    let Some(fx) = fixture().await else { return };
    let cut = post(fx.store.as_ref(), 3).await;
    let open = post(fx.store.as_ref(), 2).await;
    let (cut_ws, open_ws) = (workspace_of(&cut), workspace_of(&open));
    let meters = UsageMeterBuffer::new();
    meters.set_cut_off([(cut_ws, UsageMeter::EmbeddingCalls)]);
    let hash: Arc<dyn EmbeddingProvider> = Arc::new(HashV1Provider);

    let swept = fx
        .search
        .embed_missing(&hash, 100, Some(&meters))
        .await
        .unwrap();
    assert_eq!(
        (swept.processed, swept.failed),
        (2, 0),
        "only the open workspace's messages are embedded"
    );
    let counts = meters.take();
    let open_calls: i64 = counts
        .iter()
        .filter(|r| r.workspace_id == open_ws && r.meter == UsageMeter::EmbeddingCalls)
        .map(|r| r.quantity)
        .sum();
    assert_eq!(open_calls, 2, "each repaired message is metered");
    assert!(
        counts.iter().all(|r| r.workspace_id != cut_ws),
        "nothing is metered to the cut-off workspace"
    );

    // A second sweep under the cutoff still leaves the cut-off workspace alone.
    let again = fx
        .search
        .embed_missing(&hash, 100, Some(&meters))
        .await
        .unwrap();
    assert_eq!(again.processed, 0);
    assert_eq!(embedding_calls(&meters, cut_ws), 0);

    // Its three messages are still missing: lifting the cutoff repairs them.
    meters.set_cut_off([]);
    let lifted = fx
        .search
        .embed_missing(&hash, 100, Some(&meters))
        .await
        .unwrap();
    assert_eq!(
        lifted.processed, 3,
        "the cut-off workspace was never embedded"
    );
    assert_eq!(embedding_calls(&meters, cut_ws), 3);
}

#[tokio::test]
async fn the_repair_sweep_embeds_and_meters_nothing_on_sqlite_where_it_does_not_run() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search = SqliteSearch::new(pool);
    let cut = post(store.as_ref(), 3).await;
    let open = post(store.as_ref(), 2).await;
    let (cut_ws, open_ws) = (workspace_of(&cut), workspace_of(&open));
    let meters = UsageMeterBuffer::new();
    meters.set_cut_off([(cut_ws, UsageMeter::EmbeddingCalls)]);
    let hash: Arc<dyn EmbeddingProvider> = Arc::new(HashV1Provider);

    let swept = search.embed_missing(&hash, 100, Some(&meters)).await;
    assert!(
        matches!(swept, Err(maidan_search::SearchError::Unsupported(_))),
        "SQLite has no repair sweep, so it can embed nothing of either workspace"
    );
    let counts = meters.take();
    assert!(
        counts
            .iter()
            .all(|r| r.workspace_id != cut_ws && r.workspace_id != open_ws),
        "nothing is metered when nothing is embedded"
    );
}
