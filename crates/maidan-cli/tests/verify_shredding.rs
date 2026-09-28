//! `maidan verify-shredding`: passes on a database where every withdrawal
//! took its words with it, and names the table when a copy is left behind.

use std::process::{Command, Output};
use std::sync::Arc;

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

fn verify(url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_maidan"))
        .args(["verify-shredding", "--database-url", url])
        .output()
        .expect("run maidan verify-shredding")
}

#[tokio::test]
async fn verify_shredding_reports_words_a_withdrawal_left() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("verify.db").display()
    );
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(
        pool.clone(),
        Arc::new(ContentKeyring::new([7; 32], Vec::new())),
    );
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
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
        })
        .await
        .unwrap();
    let message = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: alice.id,
            body: "withdrawn words".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    store.tombstone_message(message.id).await.unwrap();

    let clean = verify(&url);
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    assert!(String::from_utf8_lossy(&clean.stdout).contains("no residue"));

    sqlx::query("UPDATE maidan_messages SET body = 'withdrawn words' WHERE id = ?")
        .bind(message.id.0)
        .execute(&pool)
        .await
        .unwrap();
    let dirty = verify(&url);
    assert!(!dirty.status.success(), "residue must fail the check");
    let stdout = String::from_utf8_lossy(&dirty.stdout);
    assert!(
        stdout.contains(&format!("subject={} table=maidan_messages", message.id.0)),
        "stdout: {stdout}"
    );
}
