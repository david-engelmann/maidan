//! `maidan reindex-embeddings` CLI smoke test.

use std::process::Command;

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use std::sync::Arc;

#[tokio::test]
async fn reindex_embeddings_cli_processes_live_messages() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reindex.db");
    let database_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect(&database_url)
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));

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
            description: None,
            created_by: None,
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

    let bin = &std::env::var_os("NEXTEST_BIN_EXE_maidan")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_maidan").into());
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

/// A remote embedder that answers 500 must show up as `failed` and a non-zero
/// exit. A silent 0 used to let a migration script believe the backfill finished.
#[tokio::test]
async fn reindex_embeddings_exits_nonzero_when_the_provider_fails() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;

    use maidan_store::{prelude::*, run_sqlite_migrations};
    use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            let mut header_end = None;
            loop {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            if let Some(header_end) = header_end {
                let headers = String::from_utf8_lossy(&buf[..header_end]);
                let length = headers.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                });
                if let Some(length) = length {
                    while buf.len() - header_end < length {
                        match stream.read(&mut tmp) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    }
                }
            }
            let body = br#"{"error":"stub"}"#;
            let resp = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(body);
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("reindex-fail.db").display()
    );
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect(&database_url)
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let ws = store
        .create_workspace(NewWorkspace {
            name: "reindex-fail".into(),
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
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "this embed will fail".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let bin = &std::env::var_os("NEXTEST_BIN_EXE_maidan")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_maidan").into());
    let endpoint = format!("http://127.0.0.1:{port}/v1/embeddings");
    let output = Command::new(bin)
        .args(["reindex-embeddings", "--database-url", &database_url])
        .env("MAIDAN_EMBEDDING_PROVIDER", "openai-compatible")
        .env("MAIDAN_EMBEDDING_ENDPOINT", &endpoint)
        .env("MAIDAN_EMBEDDING_MODEL", "stub-fail")
        .env("MAIDAN_EMBEDDING_DIM", "4")
        .env("MAIDAN_EMBEDDING_TIMEOUT_SECS", "2")
        .env("MAIDAN_LOG", "error")
        .output()
        .expect("run cli");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("processed=0") && stdout.contains("failed=1"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !output.status.success(),
        "a failed embed must not exit 0: {stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("reindex incomplete"),
        "stderr: {stderr}\nstdout: {stdout}"
    );
}
