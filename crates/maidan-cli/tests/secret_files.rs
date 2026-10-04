//! `maidan init` reads `DATABASE_URL` and `MAIDAN_CONTENT_KEK` from mounted
//! secret files (`<NAME>_FILE`), so a container that runs it carries no secret
//! in its environment. The files are resolved before clap reads its `env`
//! defaults, and a conflicting or unreadable file refuses before anything is
//! written.

use std::path::Path;
use std::process::{Command, Output};

fn maidan(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_maidan"));
    for (name, _) in std::env::vars() {
        if name.starts_with("MAIDAN_") || name.starts_with("DATABASE_URL") {
            cmd.env_remove(name);
        }
    }
    cmd.current_dir(dir).args(["init", "--workspace", "demo"]);
    cmd
}

fn write(dir: &Path, name: &str, contents: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_string()
}

fn text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn init_reads_the_database_url_and_kek_from_files() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("maidan.db");
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let url_file = write(dir.path(), "database_url", &format!("{url}\n"));
    let kek_file = write(dir.path(), "kek", &format!("{}\n", "07".repeat(32)));

    let output = maidan(dir.path())
        .env("DATABASE_URL_FILE", &url_file)
        .env("MAIDAN_CONTENT_KEK_FILE", &kek_file)
        .output()
        .unwrap();
    let (stdout, stderr) = text(&output);
    assert!(output.status.success(), "{stderr}\n{stdout}");
    assert!(stdout.contains("Maidan initialized"), "{stdout}");
    assert!(
        db.exists(),
        "init wrote to the database the file named, not the default"
    );
}

#[test]
fn init_refuses_a_secret_set_both_ways_and_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("maidan.db");
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let url_file = write(dir.path(), "database_url", &url);

    let output = maidan(dir.path())
        .env("DATABASE_URL", &url)
        .env("DATABASE_URL_FILE", &url_file)
        .env("MAIDAN_CONTENT_KEK", "07".repeat(32))
        .output()
        .unwrap();
    let (stdout, stderr) = text(&output);
    assert!(!output.status.success(), "both forms must refuse");
    assert!(
        stderr.contains("DATABASE_URL and DATABASE_URL_FILE are both set"),
        "{stderr}"
    );
    assert!(
        !stderr.contains(&url) && !stdout.contains(&url),
        "the refusal must not print the secret: {stderr}"
    );
    assert!(!db.exists(), "nothing may be written before the refusal");
}

#[test]
fn init_refuses_an_unreadable_secret_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-kek");

    let output = maidan(dir.path())
        .env("DATABASE_URL", "sqlite::memory:")
        .env("MAIDAN_CONTENT_KEK_FILE", &missing)
        .output()
        .unwrap();
    let (_, stderr) = text(&output);
    assert!(!output.status.success(), "a missing file must refuse");
    assert!(
        stderr.contains("MAIDAN_CONTENT_KEK_FILE=") && stderr.contains("cannot read the file"),
        "{stderr}"
    );
}
