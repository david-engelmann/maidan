//! `MAIDAN_MARK_READY_BASES` is read in `main` at boot, so a malformed value
//! is checked against the real binary: it must refuse to start and name the
//! variable and the entry, never start with the pin silently dropped.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn server(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_maidan-server"));
    for (name, _) in std::env::vars() {
        if name.starts_with("MAIDAN_") || name.starts_with("DATABASE_URL") {
            cmd.env_remove(name);
        }
    }
    cmd.env_remove("AUTH_DISABLED")
        .current_dir(dir)
        .env("DATABASE_URL", "sqlite::memory:")
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .env(
            "MAIDAN_SESSION_SECRET",
            "dev-session-secret-change-me-0123456789",
        )
        .env("MAIDAN_BIND", "127.0.0.1:0");
    cmd
}

/// Run to exit, or kill and fail after a minute: a server that boots despite
/// the bad value would otherwise hang the test instead of failing it.
fn run_to_exit(mut cmd: Command) -> Output {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "the server booted instead of refusing:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn a_malformed_mark_ready_base_map_refuses_boot_and_names_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    for (raw, want) in [
        (
            "example-org/example-repo",
            "MAIDAN_MARK_READY_BASES: `example-org/example-repo` is not `owner/name=branch`",
        ),
        (
            "example-org/example-repo=dev,example-repo=main",
            "MAIDAN_MARK_READY_BASES: `example-repo` is not an `owner/name` repository",
        ),
        (
            "example-org/example-repo=prod",
            "MAIDAN_MARK_READY_BASES: `example-org/example-repo` names `prod`",
        ),
    ] {
        let mut cmd = server(dir.path());
        cmd.env("MAIDAN_MARK_READY_BASES", raw);
        let output = run_to_exit(cmd);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{raw:?} must refuse boot");
        assert!(stderr.contains(want), "{raw:?}: {stderr}");
    }
}
