//! Stdio transport against Postgres via testcontainers.

use std::io::Write;
use std::process::{Command, Stdio};

use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn mcp_stdio_postgres_initialize_roundtrip() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };

    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let bin = &std::env::var_os("NEXTEST_BIN_EXE_maidan")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_maidan").into());
    let init = Command::new(bin)
        .args(["init", "--database-url", &database_url])
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .env_remove("DATABASE_URL")
        .output()
        .expect("spawn maidan init");
    let init_stdout = String::from_utf8_lossy(&init.stdout);
    assert!(
        init.status.success(),
        "init failed: {init_stdout}\n{}",
        String::from_utf8_lossy(&init.stderr)
    );
    // The bearer is the first non-empty line after init's heading for it.
    let token = init_stdout
        .lines()
        .skip_while(|line| !line.contains("Admin bearer token"))
        .skip(1)
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| panic!("init printed no bearer: {init_stdout}"))
        .to_string();

    let mut child = Command::new(bin)
        .arg("mcp-stdio")
        .env("DATABASE_URL", &database_url)
        .env("MAIDAN_MCP_TOKEN", &token)
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
