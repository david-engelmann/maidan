//! The operator CLI refuses a `MAIDAN_*` name the server does not know,
//! before clap, the way `maidan-server` does. The typo is assembled so this
//! file does not itself mention an unregistered name (`env_registry_contract`
//! scans every crate).

use std::process::Command;

fn maidan() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_maidan"));
    for (name, _) in std::env::vars() {
        if name.starts_with("MAIDAN_") {
            cmd.env_remove(name);
        }
    }
    cmd.arg("--help");
    cmd
}

fn typo() -> String {
    format!("MAIDAN_{}", "RATE_LIMT_MAX")
}

#[test]
fn a_misspelt_maidan_variable_is_refused_before_help() {
    let output = maidan().env(typo(), "1").output().expect("spawn maidan");
    assert!(!output.status.success(), "a typo must not reach --help");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Usage"),
        "help ran despite the typo:\n{stdout}"
    );
    assert!(
        stderr.contains(&typo()) && stderr.contains("did you mean MAIDAN_RATE_LIMIT_MAX?"),
        "the refusal should name the typo and the variable it is closest to, got: {stderr}"
    );
    assert!(
        stderr.contains("MAIDAN_ALLOW_UNKNOWN_ENV=1"),
        "the refusal should name the override, got: {stderr}"
    );
}

#[test]
fn the_override_lets_help_run() {
    let output = maidan()
        .env(typo(), "1")
        .env("MAIDAN_ALLOW_UNKNOWN_ENV", "1")
        .output()
        .expect("spawn maidan");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "allow=1 should reach --help, stderr: {stderr}"
    );
    assert!(stdout.contains("Usage"), "expected help, got: {stdout}");
}
