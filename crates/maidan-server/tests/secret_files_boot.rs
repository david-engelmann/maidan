//! The server reads its secrets from mounted files (`<NAME>_FILE`) at boot, so
//! a container's config can carry paths instead of values. Run against the
//! real binary: the files are resolved in `main` before the runtime starts,
//! which no in-process test reaches.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn server(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_maidan-server"));
    for (name, _) in std::env::vars() {
        if name.starts_with("MAIDAN_") || name.starts_with("DATABASE_URL") {
            cmd.env_remove(name);
        }
    }
    cmd.env_remove("AUTH_DISABLED").current_dir(dir);
    cmd
}

fn write(dir: &Path, name: &str, contents: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_string()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_secret_set_both_ways_refuses_boot_without_printing_it() {
    let dir = tempfile::tempdir().unwrap();
    let token_file = write(dir.path(), "github_token", "ghp_from_file\n");
    let output = server(dir.path())
        .env("DATABASE_URL", "sqlite::memory:")
        .env("MAIDAN_GITHUB_TOKEN", "ghp_from_env")
        .env("MAIDAN_GITHUB_TOKEN_FILE", &token_file)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "both forms must refuse boot");
    assert!(
        stderr.contains("MAIDAN_GITHUB_TOKEN and MAIDAN_GITHUB_TOKEN_FILE are both set"),
        "{stderr}"
    );
    for secret in ["ghp_from_env", "ghp_from_file"] {
        assert!(!stderr.contains(secret) && !stdout.contains(secret));
    }
}

#[test]
fn an_unreadable_secret_file_refuses_boot() {
    let dir = tempfile::tempdir().unwrap();
    let output = server(dir.path())
        .env("DATABASE_URL_FILE", dir.path().join("absent"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "a missing file must refuse boot");
    assert!(
        stderr.contains("DATABASE_URL_FILE=") && stderr.contains("cannot read the file"),
        "{stderr}"
    );
}

#[tokio::test]
async fn the_server_boots_on_secrets_read_from_files_and_never_logs_them() {
    let dir = tempfile::tempdir().unwrap();
    let kek = "5a".repeat(32);
    let session_secret = "session-secret-from-a-file-0123456789abcdef";
    let url_file = write(dir.path(), "database_url", "sqlite::memory:\n");
    let kek_file = write(dir.path(), "kek", &format!("{kek}\n"));
    let session_file = write(dir.path(), "session", &format!("{session_secret}\n"));
    let port = free_port();

    let mut child = server(dir.path())
        .env("DATABASE_URL_FILE", &url_file)
        .env("MAIDAN_CONTENT_KEK_FILE", &kek_file)
        .env("MAIDAN_SESSION_SECRET_FILE", &session_file)
        .env("MAIDAN_BIND", format!("127.0.0.1:{port}"))
        .env("MAIDAN_LOG", "debug,sqlx=warn")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/health/ready");
    let deadline = Instant::now() + Duration::from_secs(60);
    let ready = loop {
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                break true;
            }
        }
        if Instant::now() > deadline || child.try_wait().unwrap().is_some() {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    let logs = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(ready, "server never became ready:\n{logs}");
    assert!(
        logs.contains("secrets read from *_FILE paths"),
        "boot should say which variables came from files:\n{logs}"
    );
    assert!(!logs.contains(&kek), "the KEK was logged");
    assert!(
        !logs.contains(session_secret),
        "the session secret was logged"
    );
}
