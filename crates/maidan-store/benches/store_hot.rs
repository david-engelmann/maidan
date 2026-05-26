//! Criterion benches for store hot paths (Track U.1).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use maidan_store::{configure_sqlite_pool, SqliteStore, Store};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

struct BenchCtx {
    store: SqliteStore,
    workspace_id: maidan_types::WorkspaceId,
}

fn sqlite_ctx() -> BenchCtx {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        configure_sqlite_pool(&pool).await.expect("pragmas");
        maidan_store::run_sqlite_migrations(&pool)
            .await
            .expect("migrate");
        let store = SqliteStore::new(pool);
        let ws = store
            .create_workspace(NewWorkspace {
                name: "bench".into(),
            })
            .await
            .expect("ws");
        for i in 0..32 {
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: format!("m{i}"),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member");
        }
        BenchCtx {
            store,
            workspace_id: ws.id,
        }
    })
}

fn bench_list_members(c: &mut Criterion) {
    let ctx = sqlite_ctx();
    let rt = tokio::runtime::Runtime::new().expect("runtime");

    c.bench_function("sqlite_list_members_32", |b| {
        b.iter(|| {
            let n = rt
                .block_on(ctx.store.list_members(ctx.workspace_id))
                .unwrap()
                .len();
            black_box(n);
        });
    });
}

criterion_group!(benches, bench_list_members);
criterion_main!(benches);
