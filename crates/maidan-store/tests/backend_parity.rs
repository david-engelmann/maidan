//! Backend parity guard: keep the Postgres and SQLite implementations in
//! lockstep so a feature added to one backend can't silently ship without its
//! counterpart.
//!
//! Two static checks, run in the (always-available, Docker-free) `unit tests`
//! job: every migration *slug* exists for both backends, and every store
//! module exists for both backends — modulo an explicit, rationale-documented
//! allowlist of legitimate per-backend divergences. A new unmatched migration
//! or module fails this test until the author either adds the counterpart or
//! consciously extends the allowlist below (with a reason).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Migration slugs that legitimately exist for only one backend.
///
/// - `outbox_quarantine` (Postgres only): the transactional-outbox quarantine
///   columns are a *separate* migration on Postgres (`0014_outbox_quarantine`)
///   but are folded into the base `0013_outbox` migration on SQLite. Same
///   feature, different migration granularity — both backends have quarantine.
const POSTGRES_ONLY_MIGRATIONS: &[&str] = &["outbox_quarantine"];
const SQLITE_ONLY_MIGRATIONS: &[&str] = &[];

/// Store modules that legitimately exist for only one backend.
///
/// - `pragmas` (SQLite only): per-connection `PRAGMA` setup (foreign_keys,
///   busy_timeout). Postgres has no connection-pragma equivalent.
/// - `replication` (Postgres only): WAL-LSN helpers for streaming-replica routing
///   (`pg_current_wal_lsn`/`pg_last_wal_replay_lsn`, Cluster 261). SQLite has no
///   streaming replication, so there is no counterpart.
const POSTGRES_ONLY_MODULES: &[&str] = &["replication"];
const SQLITE_ONLY_MODULES: &[&str] = &["pragmas"];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/crates/maidan-store
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Reduce a migration filename to its feature slug: strip the `NNNN_` numeric
/// prefix and any trailing `_up` / `_down` (the only up/down-split migration is
/// `0001_core`, which collapses to `core`).
fn migration_slug(file_stem: &str) -> String {
    let after_num = match file_stem.find('_') {
        Some(i)
            if !file_stem[..i].is_empty() && file_stem[..i].bytes().all(|b| b.is_ascii_digit()) =>
        {
            &file_stem[i + 1..]
        }
        _ => file_stem,
    };
    after_num
        .strip_suffix("_up")
        .or_else(|| after_num.strip_suffix("_down"))
        .unwrap_or(after_num)
        .to_string()
}

fn migration_slugs(backend: &str) -> BTreeSet<String> {
    let dir = repo_root().join("migrations").join(backend);
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read migrations dir {}: {e}", dir.display()));
    let mut slugs = BTreeSet::new();
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("sql") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("utf8 stem");
            slugs.insert(migration_slug(stem));
        }
    }
    assert!(!slugs.is_empty(), "no migrations found for {backend}");
    slugs
}

fn module_stems(backend: &str) -> BTreeSet<String> {
    let dir = repo_root().join("crates/maidan-store/src").join(backend);
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read store src dir {}: {e}", dir.display()));
    let mut stems = BTreeSet::new();
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf8 stem");
        if stem == "mod" {
            continue;
        }
        stems.insert(stem.to_string());
    }
    assert!(!stems.is_empty(), "no store modules found for {backend}");
    stems
}

fn allowed(set: &[&str]) -> BTreeSet<String> {
    set.iter().map(|s| s.to_string()).collect()
}

#[test]
fn migrations_stay_in_lockstep() {
    let pg = migration_slugs("postgres");
    let sqlite = migration_slugs("sqlite");

    let pg_only: BTreeSet<String> = pg.difference(&sqlite).cloned().collect();
    let sqlite_only: BTreeSet<String> = sqlite.difference(&pg).cloned().collect();

    assert_eq!(
        pg_only,
        allowed(POSTGRES_ONLY_MIGRATIONS),
        "Postgres-only migrations changed. If this is intentional, add the SQLite \
         counterpart or extend POSTGRES_ONLY_MIGRATIONS with a rationale."
    );
    assert_eq!(
        sqlite_only,
        allowed(SQLITE_ONLY_MIGRATIONS),
        "SQLite-only migrations changed. If this is intentional, add the Postgres \
         counterpart or extend SQLITE_ONLY_MIGRATIONS with a rationale."
    );
}

#[test]
fn store_modules_stay_in_lockstep() {
    let pg = module_stems("postgres");
    let sqlite = module_stems("sqlite");

    let pg_only: BTreeSet<String> = pg.difference(&sqlite).cloned().collect();
    let sqlite_only: BTreeSet<String> = sqlite.difference(&pg).cloned().collect();

    assert_eq!(
        pg_only,
        allowed(POSTGRES_ONLY_MODULES),
        "Postgres-only store modules changed. Add the SQLite counterpart or extend \
         POSTGRES_ONLY_MODULES with a rationale."
    );
    assert_eq!(
        sqlite_only,
        allowed(SQLITE_ONLY_MODULES),
        "SQLite-only store modules changed. Add the Postgres counterpart or extend \
         SQLITE_ONLY_MODULES with a rationale."
    );
}

#[test]
fn migration_slug_strips_prefix_and_up_down_suffix() {
    assert_eq!(migration_slug("0001_core_up"), "core");
    assert_eq!(migration_slug("0001_core_down"), "core");
    assert_eq!(migration_slug("0020_slash_commands"), "slash_commands");
    assert_eq!(
        migration_slug("0014_outbox_quarantine"),
        "outbox_quarantine"
    );
    // No numeric prefix -> unchanged.
    assert_eq!(migration_slug("freeform"), "freeform");
}
