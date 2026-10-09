//! `MAIDAN_MARK_READY_BASES` is read in `main` at boot, so a malformed value
//! is checked against the real binary: it must refuse to start and name the
//! variable and the entry, never start with the pin silently dropped. A
//! designated mark-ready app with no pins boots, but says so.

use std::io::{BufRead, BufReader};
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

const NO_PINS: &str = "MAIDAN_MARK_READY_BASES pins no base";

/// Boot with JSON logs and collect every log line up to `listening`, then
/// stop the server. Fails after a minute without `listening`.
fn boot_log(env: &[(&str, &str)]) -> Vec<serde_json::Value> {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = server(dir.path());
    cmd.env("MAIDAN_LOG_FORMAT", "json");
    for (name, value) in env {
        cmd.env(name, value);
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut lines = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = rx.recv_timeout(left) else {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "the server did not log `listening` within a minute: {lines:?}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let listening = entry["fields"]["message"] == "listening";
        lines.push(entry);
        if listening {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    lines
}

/// The warnings among `lines`, for a failure message short enough to read.
fn warnings(lines: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    lines.iter().filter(|l| l["level"] == "WARN").collect()
}

fn no_pins_warnings(lines: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    lines
        .iter()
        .filter(|l| {
            l["fields"]["message"]
                .as_str()
                .is_some_and(|m| m.contains(NO_PINS))
        })
        .collect()
}

const APP_ID: &str = "0192f2a0-0000-7000-8000-000000000001";

#[test]
fn a_mark_ready_app_with_no_base_pins_boots_with_a_warning() {
    for bases in [None, Some(""), Some(" , ")] {
        let mut env = vec![("MAIDAN_MARK_READY_APP_ID", APP_ID)];
        if let Some(raw) = bases {
            env.push(("MAIDAN_MARK_READY_BASES", raw));
        }
        let lines = boot_log(&env);
        let warned = no_pins_warnings(&lines);
        assert_eq!(warned.len(), 1, "bases {bases:?}: {:?}", warnings(&lines));
        assert_eq!(warned[0]["level"], "WARN", "{:?}", warned[0]);
        assert_eq!(warned[0]["fields"]["app_id"], APP_ID, "{:?}", warned[0]);
    }
}

#[test]
fn base_pins_or_no_mark_ready_app_boot_without_the_warning() {
    for env in [
        vec![
            ("MAIDAN_MARK_READY_APP_ID", APP_ID),
            ("MAIDAN_MARK_READY_BASES", "example-org/example-repo=dev"),
        ],
        vec![],
        vec![("MAIDAN_MARK_READY_BASES", "example-org/example-repo=dev")],
        vec![("MAIDAN_MARK_READY_APP_ID", "  ")],
    ] {
        let lines = boot_log(&env);
        assert!(
            no_pins_warnings(&lines).is_empty(),
            "{env:?} warned: {:?}",
            warnings(&lines)
        );
    }
}
