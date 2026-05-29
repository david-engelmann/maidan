//! Per-model embedding tables and mixed dimensions.

use maidan_search::{embedding_tables, sqlite_pool_options, Search, SearchFilters, SqliteSearch};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use std::sync::Arc;

fn vec_dim(dim: usize, peak: usize) -> Vec<f32> {
    let mut v = vec![0.0; dim];
    if peak < dim {
        v[peak] = 1.0;
    }
    v
}

#[tokio::test]
async fn sqlite_mixed_dimension_models_coexist() {
    let pool = sqlite_pool_options()
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

    let ws = store
        .create_workspace(NewWorkspace { name: "mix".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
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
            title: None,
        })
        .await
        .unwrap();
    let m1 = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: member.id,
            body: "dim1024".into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let m2 = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: member.id,
            body: "dim512".into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    search
        .upsert_embedding(m1.id, "hash-v1", &vec_dim(1024, 0))
        .await
        .unwrap();
    search
        .upsert_embedding(m2.id, "compact-v1", &vec_dim(512, 1))
        .await
        .unwrap();

    let reg: Vec<(String, i32)> =
        sqlx::query_as("SELECT model, dimension FROM maidan_embedding_models ORDER BY model")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(reg.len(), 2);
    assert!(reg.iter().any(|(m, d)| m == "compact-v1" && *d == 512));
    assert!(reg.iter().any(|(m, d)| m == "hash-v1" && *d == 1024));

    let hits = search
        .semantic_search(
            ws.id,
            &vec_dim(512, 1),
            5,
            &SearchFilters::default(),
            "compact-v1",
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id, m2.id);

    let err = search
        .upsert_embedding(m1.id, "hash-v1", &vec_dim(512, 0))
        .await
        .expect_err("dimension mismatch");
    assert!(err.to_string().contains("dimension"));
}

#[test]
fn table_name_matches_migration_slug() {
    assert_eq!(
        embedding_tables::table_name_for_model("hash-v1").unwrap(),
        "maidan_emb_hash_v1"
    );
}
