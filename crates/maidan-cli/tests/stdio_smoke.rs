//! Stdio transport smoke test via subprocess.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn mcp_stdio_initialize_roundtrip() {
    let bin = env!("CARGO_BIN_EXE_maidan");
    let mut child = Command::new(bin)
        .arg("mcp-stdio")
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
