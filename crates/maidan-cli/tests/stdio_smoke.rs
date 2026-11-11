//! Stdio transport smoke test via subprocess.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn mcp_stdio_initialize_roundtrip() {
    let bin = &std::env::var_os("NEXTEST_BIN_EXE_maidan")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_maidan").into());
    let mut child = Command::new(bin)
        .arg("mcp-stdio")
        // No token here, so the unrestricted context has to be asked for.
        .arg("--allow-insecure-no-auth")
        .env("DATABASE_URL", "sqlite::memory:")
        .env("MAIDAN_ENV", "development")
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn maidan mcp-stdio");

    let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{req}").expect("write");
    }

    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() || !stdout.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("maidan"),
        "expected serverInfo name in {stdout}"
    );
    assert!(
        stdout.contains("\"result\""),
        "expected json-rpc result in {stdout}"
    );
}

/// stdout is the protocol: a stdio client parses every line as a JSON-RPC
/// message, so a log line there breaks it. Logging is on at info, and a tool call
/// writes a request-log line, so a log that reaches stdout fails this.
#[test]
fn every_line_on_stdout_is_a_json_rpc_message() {
    let bin = &std::env::var_os("NEXTEST_BIN_EXE_maidan")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_maidan").into());
    let mut child = Command::new(bin)
        .arg("mcp-stdio")
        .arg("--allow-insecure-no-auth")
        .env("DATABASE_URL", "sqlite::memory:")
        .env("MAIDAN_ENV", "development")
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .env("MAIDAN_LOG", "info")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn maidan mcp-stdio");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for req in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"whoami","arguments":{}}}"#,
        ] {
            writeln!(stdin, "{req}").expect("write");
        }
    }
    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 3, "one response per request: {stdout}");
    for line in lines {
        let message: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|err| panic!("not a JSON-RPC message ({err}): {line}"));
        assert_eq!(message["jsonrpc"], "2.0", "{line}");
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mcp request"),
        "the request log goes to stderr: {stderr}"
    );
}

/// The interesting half of the flag: without it, and with no token, the process
/// refuses rather than serving every tool with unrestricted authority. The other
/// tests pass `--allow-insecure-no-auth`, so none of them would notice if the
/// requirement went away.
#[test]
fn mcp_stdio_refuses_to_serve_unauthenticated_unless_asked() {
    let bin = &std::env::var_os("NEXTEST_BIN_EXE_maidan")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_maidan").into());
    let output = Command::new(bin)
        .arg("mcp-stdio")
        .env("DATABASE_URL", "sqlite::memory:")
        .env_remove("MAIDAN_MCP_TOKEN")
        .env_remove("MAIDAN_ALLOW_INSECURE_NO_AUTH")
        .env("MAIDAN_ENV", "development")
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .stdin(Stdio::null())
        .output()
        .expect("spawn maidan mcp-stdio");

    assert!(!output.status.success(), "expected a non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("MAIDAN_MCP_TOKEN") && stderr.contains("--allow-insecure-no-auth"),
        "the refusal should name both ways forward, got: {stderr}"
    );
}

fn maidan_bin() -> std::ffi::OsString {
    std::env::var_os("NEXTEST_BIN_EXE_maidan")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_maidan").into())
}

fn wait_with_timeout(mut child: std::process::Child, secs: u64) -> std::process::Output {
    use std::io::Read;
    drop(child.stdin.take());
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let (out_tx, out_rx) = std::sync::mpsc::channel();
    let (err_tx, err_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = out_tx.send(buf);
    });
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        let _ = err_tx.send(buf);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return std::process::Output {
                    status,
                    stdout: out_rx.recv().unwrap_or_default(),
                    stderr: err_rx.recv().unwrap_or_default(),
                };
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("mcp-stdio did not exit within {secs}s");
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(err) => panic!("mcp-stdio wait failed: {err}"),
        }
    }
}

/// A message posted over a one-shot `mcp-stdio` pipe is embedded before the
/// process exits, and a later invocation can find it by semantic search.
#[tokio::test]
async fn mcp_stdio_embeds_a_sqlite_message_before_stdin_closes() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::Arc;

    use maidan_store::{prelude::*, run_sqlite_migrations};
    use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};

    const PHRASE: &str = "stdio embed lantern phrase";

    let dir = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("stdio_embed.db").display()
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = store
        .create_workspace(NewWorkspace {
            name: "stdio".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "stdio-bot".into(),
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
    let secret = maidan_auth::TokenSecret::generate();
    let actor = member.id;
    store
        .create_api_token_audited(
            maidan_types::NewApiToken {
                workspace_id: ws.id,
                member_id: member.id,
                app_installation_id: None,
                token_hash: maidan_auth::hash_secret(secret.as_str()),
                label: Some("stdio smoke".into()),
                capabilities: maidan_auth::capability::all(),
                expires_at: None,
            },
            Box::new(move |record| maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(record.workspace_id),
                actor_id: Some(actor),
                action: "token.mint".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({ "source": "stdio-smoke" }),
            }),
        )
        .await
        .unwrap();

    let post = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"post_message","arguments":{{"thread_id":"{}","body":"{PHRASE}"}}}}}}"#,
        thread.id.0
    );
    let mut child = Command::new(maidan_bin())
        .args([
            "mcp-stdio",
            "--database-url",
            &database_url,
            "--embedding-provider",
            "hash-v1",
        ])
        .env("MAIDAN_MCP_TOKEN", secret.as_str())
        .env("MAIDAN_ENV", "development")
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .env("MAIDAN_LOG", "error")
        .env_remove("MAIDAN_ALLOW_INSECURE_NO_AUTH")
        .env_remove("MAIDAN_EMBEDDING_PROVIDER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn maidan mcp-stdio");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"smoke","version":"0"}}}}}}"#
        )
        .unwrap();
        writeln!(stdin, "{post}").unwrap();
    }
    let posted = wait_with_timeout(child, 30);
    let post_stdout = String::from_utf8_lossy(&posted.stdout);
    let post_stderr = String::from_utf8_lossy(&posted.stderr);
    assert!(
        posted.status.success(),
        "stdout: {post_stdout}\nstderr: {post_stderr}"
    );
    assert!(
        post_stdout.contains(PHRASE),
        "post_message did not return the body: {post_stdout}\nstderr: {post_stderr}"
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM maidan_emb_hash_v1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "the message was not embedded before exit");

    let search = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"search_messages","arguments":{{"workspace_id":"{}","query":"{PHRASE}","mode":"semantic","limit":5}}}}}}"#,
        ws.id.0
    );
    let mut child = Command::new(maidan_bin())
        .args([
            "mcp-stdio",
            "--database-url",
            &database_url,
            "--embedding-provider",
            "hash-v1",
        ])
        .env("MAIDAN_MCP_TOKEN", secret.as_str())
        .env("MAIDAN_ENV", "development")
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .env("MAIDAN_LOG", "error")
        .env_remove("MAIDAN_ALLOW_INSECURE_NO_AUTH")
        .env_remove("MAIDAN_EMBEDDING_PROVIDER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn search");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"smoke","version":"0"}}}}}}"#
        )
        .unwrap();
        writeln!(stdin, "{search}").unwrap();
    }
    let found = wait_with_timeout(child, 30);
    let found_stdout = String::from_utf8_lossy(&found.stdout);
    let found_stderr = String::from_utf8_lossy(&found.stderr);
    assert!(
        found.status.success(),
        "stdout: {found_stdout}\nstderr: {found_stderr}"
    );
    assert!(
        found_stdout.contains(PHRASE),
        "semantic search did not return the message: {found_stdout}\nstderr: {found_stderr}"
    );
}

#[test]
fn mcp_stdio_rejects_an_unknown_embedding_provider_before_serving() {
    use std::process::{Command, Stdio};

    let output = Command::new(maidan_bin())
        .args([
            "mcp-stdio",
            "--embedding-provider",
            "bogus-provider",
            "--allow-insecure-no-auth",
        ])
        .env("DATABASE_URL", "sqlite::memory:")
        .env_remove("MAIDAN_EMBEDDING_PROVIDER")
        .stdin(Stdio::null())
        .output()
        .expect("spawn maidan mcp-stdio");
    assert!(!output.status.success(), "an unknown provider must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown embedding provider") && stderr.contains("bogus-provider"),
        "stderr: {stderr}"
    );
}
