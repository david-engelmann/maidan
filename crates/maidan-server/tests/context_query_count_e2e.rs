//! Query-count regression for context assembly (Cluster 106.0.3).
//!
//! Asserts that `build_thread_context` issues the same number of store queries
//! whether the thread has a few messages or many — i.e. the per-message N+1s
//! (references, edits) eliminated in 106.0.2 stay gone. Query count is observed
//! by counting `sqlx::query` `tracing` events while the context is built.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use maidan_server::thread_context::{build_thread_context, ThreadContextLimits};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// Counts every `sqlx::query` tracing event (one per executed statement).
#[derive(Clone, Default)]
struct QueryCounter(Arc<AtomicUsize>);

impl QueryCounter {
    fn reset(&self) {
        self.0.store(0, Ordering::SeqCst);
    }
    fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl<S: Subscriber> Layer<S> for QueryCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target().starts_with("sqlx::query") {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

async fn seed_thread_with_messages(
    store: &dyn Store,
    thread_id: ThreadId,
    author: MemberId,
    n: usize,
) {
    for i in 0..n {
        let msg = store
            .post_message(NewMessage {
                thread_id,
                author_id: author,
                body: format!("message {i}"),
                metadata: json!({}),
            })
            .await
            .unwrap();
        // A reference and an edit per message — exactly the per-row reads
        // 106.0.2 batched. If the N+1 returns, these multiply the query count.
        store
            .add_reference(NewReference {
                src_kind: RefSide::Message,
                src_id: msg.id.0,
                dst_kind: RefSide::Thread,
                dst_id: thread_id.0,
                relation: "rel".into(),
            })
            .await
            .unwrap();
        store
            .edit_message(
                msg.id,
                author,
                EditMessage {
                    body: format!("message {i} edited"),
                    metadata: json!({}),
                },
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn thread_context_query_count_is_independent_of_message_count() {
    let counter = QueryCounter::default();
    tracing_subscriber::registry().with(counter.clone()).init();

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

    let ws = store
        .create_workspace(NewWorkspace { name: "qc".into() })
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

    // A small thread and a large one, each its own thread in the same channel.
    let small = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    seed_thread_with_messages(&store, small.id, author.id, 3).await;

    let large = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    seed_thread_with_messages(&store, large.id, author.id, 40).await;

    let limits = ThreadContextLimits::default();

    counter.reset();
    let small_ctx = build_thread_context(&store, small.id, limits)
        .await
        .unwrap();
    let small_queries = counter.get();

    counter.reset();
    let large_ctx = build_thread_context(&store, large.id, limits)
        .await
        .unwrap();
    let large_queries = counter.get();

    // Sanity: the build really did execute (and was observed) — guards against
    // the instrumentation silently counting nothing.
    assert!(
        small_queries >= 5,
        "expected several queries per context build, got {small_queries}"
    );
    assert_eq!(small_ctx.messages.len(), 3);
    assert_eq!(large_ctx.messages.len(), 40);
    assert_eq!(large_ctx.references.len(), 40);

    // The regression guard: a 13× larger thread must not issue more queries.
    assert_eq!(
        large_queries, small_queries,
        "context query count must be independent of message count \
         (small={small_queries}, large={large_queries}) — a per-message N+1 was reintroduced"
    );
}
