//! Criterion benches for search hot paths — lexical (FTS5) and semantic
//! (brute-force cosine) latency on SQLite (Cluster 109.0.2, Track U).
//!
//! SQLite keeps the bench self-contained (no testcontainer), so it is a
//! reproducible baseline. Postgres `pgvector`/HNSW latency is tuned via the
//! Cluster 109.0.1 knobs and measured separately against a real instance.
//! Run with `cargo bench -p maidan-search`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use maidan_search::{hash_embedding, model_name, Search, SearchFilters, SqliteSearch};
use maidan_store::{configure_sqlite_pool, run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

const N_MESSAGES: usize = 200;

struct BenchCtx {
    search: SqliteSearch,
    workspace_id: WorkspaceId,
}

fn body_for(i: usize) -> String {
    // Mix shared and per-row terms so lexical queries have real candidates.
    format!("the quick brown fox message {i} jumps over lazy dog lorem ipsum dolor")
}

fn sqlite_ctx() -> BenchCtx {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        configure_sqlite_pool(&pool).await.expect("pragmas");
        run_sqlite_migrations(&pool).await.expect("migrate");
        let store = SqliteStore::new(pool.clone());
        let search = SqliteSearch::new(pool);

        let ws = store
            .create_workspace(NewWorkspace {
                name: "bench".into(),
            })
            .await
            .expect("ws");
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "bot".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .expect("member");
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: None,
            })
            .await
            .expect("thread");

        for i in 0..N_MESSAGES {
            let body = body_for(i);
            let msg = store
                .post_message(NewMessage {
                    thread_id: thread.id,
                    author_id: member.id,
                    body: body.clone(),
                    metadata: serde_json::json!({}),
                    content: None,
                })
                .await
                .expect("message");
            search
                .upsert_embedding(msg.id, model_name(), &hash_embedding(&body))
                .await
                .expect("embedding");
        }

        BenchCtx {
            search,
            workspace_id: ws.id,
        }
    })
}

fn bench_search(c: &mut Criterion) {
    let ctx = sqlite_ctx();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let query_vec = hash_embedding("brown fox jumps");

    let mut group = c.benchmark_group("search");
    group.sample_size(20);

    group.bench_function("sqlite_lexical_200", |b| {
        b.iter(|| {
            let hits = rt
                .block_on(ctx.search.search_messages(
                    ctx.workspace_id,
                    "brown fox",
                    25,
                    &SearchFilters::default(),
                ))
                .unwrap();
            black_box(hits.len());
        });
    });

    group.bench_function("sqlite_semantic_200", |b| {
        b.iter(|| {
            let hits = rt
                .block_on(ctx.search.semantic_search(
                    ctx.workspace_id,
                    &query_vec,
                    25,
                    &SearchFilters::default(),
                    model_name(),
                ))
                .unwrap();
            black_box(hits.len());
        });
    });

    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
