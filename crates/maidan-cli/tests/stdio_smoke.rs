//! Stdio transport smoke test via subprocess.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn mcp_stdio_initialize_roundtrip() {
    let bin = env!("CARGO_BIN_EXE_maidan");
    let mut child = Command::new(bin)
        .arg("mcp-stdio")
        // No token here, so the unrestricted context has to be asked for.
        .arg("--allow-insecure-no-auth")
        .env("DATABASE_URL", "sqlite::memory:")
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
    let bin = env!("CARGO_BIN_EXE_maidan");
    let mut child = Command::new(bin)
        .arg("mcp-stdio")
        .arg("--allow-insecure-no-auth")
        .env("DATABASE_URL", "sqlite::memory:")
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
    let bin = env!("CARGO_BIN_EXE_maidan");
    let output = Command::new(bin)
        .arg("mcp-stdio")
        .env("DATABASE_URL", "sqlite::memory:")
        .env_remove("MAIDAN_MCP_TOKEN")
        .env_remove("MAIDAN_ALLOW_INSECURE_NO_AUTH")
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
