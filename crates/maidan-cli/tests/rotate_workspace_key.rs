//! `maidan rotate-workspace-key`: retires a workspace's key, moves its message
//! keys under a new one, destroys the old key and audits the rotation, and the
//! messages still open afterwards. An unknown workspace is refused.

use std::process::Command;
use std::sync::Arc;

use maidan_store::prelude::*;
use maidan_types::{
    ContentKeyring, Event, MemberId, NewChannel, NewMessage, NewThread, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

fn temp_db_url() -> (std::path::PathBuf, String) {
    let db = std::env::temp_dir().join(format!("maidan-rotate-it-{}.db", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    let url = format!("sqlite://{}?mode=rwc", db.display());
    (db, url)
}

fn cleanup(db: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
}

fn maidan(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_maidan"))
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .args(args)
        .output()
        .expect("run maidan")
}

/// The id in parentheses on the `label:` line `maidan init` prints.
fn printed_id(stdout: &str, label: &str) -> uuid::Uuid {
    stdout
        .lines()
        .find(|line| line.trim_start().starts_with(label))
        .and_then(|line| line.rsplit_once('(')?.1.strip_suffix(')'))
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("no {label} id in: {stdout}"))
}

async fn open(url: &str) -> (sqlx::SqlitePool, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(
        pool.clone(),
        Arc::new(ContentKeyring::insecure_dev()),
    ));
    (pool, store)
}

fn body_of(event: &maidan_types::StoredEvent) -> String {
    match event.opened_event().unwrap() {
        Event::MessagePosted { message, .. } => message.body,
        other => panic!("not a post: {other:?}"),
    }
}

#[tokio::test]
async fn a_rotation_moves_the_keys_destroys_the_old_one_and_the_words_still_open() {
    let (db, url) = temp_db_url();
    let init = maidan(&["init", "--database-url", &url, "--workspace", "demo"]);
    let stdout = String::from_utf8_lossy(&init.stdout).to_string();
    assert!(init.status.success(), "{stdout}");
    let workspace = WorkspaceId(printed_id(&stdout, "workspace:"));
    let admin = MemberId(printed_id(&stdout, "admin:"));

    let (pool, store) = open(&url).await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: workspace,
            name: "general".into(),
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
        })
        .await
        .unwrap();
    let (_, posted) = store
        .post_message_with_event(
            NewMessage {
                thread_id: thread.id,
                author_id: admin,
                body: "before the rotation".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    let old_key: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM maidan_workspace_keys WHERE workspace_id = ? AND retired_at IS NULL",
    )
    .bind(workspace.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    pool.close().await;

    let rotate = maidan(&[
        "rotate-workspace-key",
        "--database-url",
        &url,
        "--workspace-id",
        &workspace.0.to_string(),
    ]);
    let out = String::from_utf8_lossy(&rotate.stdout).to_string();
    assert!(
        rotate.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&rotate.stderr)
    );
    assert!(out.contains("retired and destroyed"), "{out}");

    let (pool, store) = open(&url).await;
    let (key, destroyed): (uuid::Uuid, bool) = sqlx::query_as(
        "SELECT k.workspace_key_id, wk.destroyed_at IS NOT NULL
         FROM maidan_content_keys k JOIN maidan_workspace_keys wk ON wk.id = ?
         WHERE k.workspace_id = ?",
    )
    .bind(old_key)
    .bind(workspace.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_ne!(key, old_key, "the message key moved to the new key");
    assert!(destroyed, "the old key is destroyed");
    let read = store.get_stored_event(posted.id).await.unwrap();
    assert_eq!(body_of(&read), "before the rotation");
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maidan_audit WHERE action = 'workspace_key.rotate' AND workspace_id = ?",
    )
    .bind(workspace.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);
    pool.close().await;

    let unknown = maidan(&[
        "rotate-workspace-key",
        "--database-url",
        &url,
        "--workspace-id",
        &uuid::Uuid::now_v7().to_string(),
    ]);
    assert!(!unknown.status.success(), "an unknown workspace is refused");
    cleanup(&db);
}
