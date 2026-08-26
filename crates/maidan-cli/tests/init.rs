//! `maidan init` (Cluster 279): a one-time first-admin bootstrap that creates the
//! initial workspace + admin member + an all-capabilities token, and refuses on an
//! already-initialized database.

use std::process::Command;

fn temp_db_url() -> (std::path::PathBuf, String) {
    let db = std::env::temp_dir().join(format!("maidan-init-it-{}.db", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    let url = format!("sqlite://{}?mode=rwc", db.display());
    (db, url)
}

fn cleanup(db: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
}

#[test]
fn init_bootstraps_once_then_refuses() {
    let bin = env!("CARGO_BIN_EXE_maidan");
    let (db, url) = temp_db_url();

    // First init succeeds and prints a bearer token exactly once.
    let first = Command::new(bin)
        .args([
            "init",
            "--database-url",
            &url,
            "--workspace",
            "demo",
            "--admin-handle",
            "david",
        ])
        .output()
        .expect("run maidan init");
    let stdout = String::from_utf8_lossy(&first.stdout);
    assert!(
        first.status.success(),
        "first init failed: {}\n{}",
        String::from_utf8_lossy(&first.stderr),
        stdout
    );
    assert!(stdout.contains("Maidan initialized"), "stdout: {stdout}");
    let tokens: Vec<&str> = stdout
        .split_whitespace()
        .filter(|w| w.starts_with("maid_"))
        .collect();
    assert_eq!(
        tokens.len(),
        1,
        "expected exactly one printed token; stdout: {stdout}"
    );

    // Second init on the now-populated database is refused (non-zero exit).
    let second = Command::new(bin)
        .args(["init", "--database-url", &url])
        .output()
        .expect("run maidan init again");
    assert!(!second.status.success(), "second init should have refused");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        combined.to_lowercase().contains("refus"),
        "second init should explain the refusal; got: {combined}"
    );

    cleanup(&db);
}
