//! A `DATABASE_URL` the server cannot use stops boot, and the refusal reaches
//! the terminal and the logs. It must say what was wrong without printing the
//! password or token the URL carries, checked against the real binary.

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const PASSWORD: &str = "S3cr3tPw";
const TOKEN: &str = "tok_S3cr3t";

fn boot_with(database_url: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_maidan-server"));
    for (name, _) in std::env::vars() {
        if name.starts_with("MAIDAN_") || name.starts_with("DATABASE_URL") {
            cmd.env_remove(name);
        }
    }
    let mut child = cmd
        .env_remove("AUTH_DISABLED")
        .current_dir(dir.path())
        .env("DATABASE_URL", database_url)
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .env(
            "MAIDAN_SESSION_SECRET",
            "dev-session-secret-change-me-0123456789",
        )
        .env("MAIDAN_BIND", "127.0.0.1:0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A server that somehow boots would hang the test instead of failing it.
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the server booted with an unusable DATABASE_URL");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn an_unknown_prefix_stops_boot_without_printing_the_password_or_token() {
    for url in [
        format!("postgress://maidan:{PASSWORD}@db.example:5432/maidan"),
        format!("libsql://maidan-demo.turso.io?authToken={TOKEN}"),
    ] {
        let output = boot_with(&url);
        assert!(!output.status.success(), "{url} must not boot");
        let shown = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            shown.contains("cannot detect dialect"),
            "the refusal says what is wrong: {shown}"
        );
        assert!(
            !shown.contains(PASSWORD) && !shown.contains(TOKEN),
            "a credential leaked into the boot error: {shown}"
        );
    }
}
