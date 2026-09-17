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
