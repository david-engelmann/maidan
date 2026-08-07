//! `maidan reindex-embeddings` CLI smoke test.

use std::process::Command;

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;

#[tokio::test]
async fn reindex_embeddings_cli_processes_live_messages() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reindex.db");
    let database_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));

    let ws = store
        .create_workspace(NewWorkspace {
            name: "reindex".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
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
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: member.id,
            body: "reindex me".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let bin = env!("CARGO_BIN_EXE_maidan");
    let output = Command::new(bin)
        .args(["reindex-embeddings", "--database-url", &database_url])
        .output()
        .expect("run cli");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("processed=1"));

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM maidan_emb_hash_v1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    let body: String = sqlx::query_scalar(
        "SELECT m.body FROM maidan_emb_hash_v1 e
         JOIN maidan_messages m ON m.id = e.message_id
         WHERE e.message_id = ?",
    )
    .bind(msg.id.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(body, "reindex me");
}
