//! Every migration file on disk is actually registered.
//!
//! Migrations are applied from a hand-maintained list of `include_str!` entries,
//! not discovered from the directory. A file that exists but is not in that list
//! **silently does nothing**: the binary starts, the migration never runs, and
//! the first query against the missing table fails at runtime — far from the
//! edit that caused it.
//!
//! This is not hypothetical. A migration was very nearly shipped unregistered:
//! the edit that was supposed to add it matched against a `const` declaration
//! whose formatting had since changed, the match failed silently, and the gap
//! was only caught by a store test hitting "no column named ...". This test is
//! the cheaper version of that discovery.

use std::collections::HashSet;
use std::path::Path;

fn sql_files(dir: &str) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir);
    let mut out: Vec<String> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read {}: {e}", root.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        // `*_down.sql` are teardown scripts, deliberately never applied at
        // startup — they exist for manual rollback only.
        .filter(|n| n.ends_with(".sql") && !n.ends_with("_down.sql"))
        .collect();
    out.sort();
    out
}

fn register_source() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/migrate.rs"))
        .expect("read migrate.rs")
}

/// A file present on disk but absent from the register is the silent failure.
#[test]
fn every_migration_file_is_registered() {
    let src = register_source();
    let mut unregistered = Vec::new();
    for dir in ["migrations/postgres", "migrations/sqlite"] {
        for file in sql_files(dir) {
            // The register refers to each file by path in an `include_str!`.
            if !src.contains(&format!("{dir}/{file}")) {
                unregistered.push(format!("{dir}/{file}"));
            }
        }
    }
    assert!(
        unregistered.is_empty(),
        "these migration files exist but are never applied — add a const and an \
         apply_* call in migrate.rs:\n  {}",
        unregistered.join("\n  ")
    );
}

/// The mirror failure: a register entry pointing at a file that no longer
/// exists. `include_str!` catches this at compile time, so this test exists to
/// name it rather than to catch it.
#[test]
fn every_registered_migration_exists() {
    let src = register_source();
    let mut missing = Vec::new();
    for dir in ["migrations/postgres", "migrations/sqlite"] {
        let on_disk: HashSet<String> = sql_files(dir).into_iter().collect();
        for line in src.lines() {
            let Some(idx) = line.find(&format!("{dir}/")) else {
                continue;
            };
            let rest = &line[idx + dir.len() + 1..];
            let Some(end) = rest.find(".sql") else {
                continue;
            };
            let name = format!("{}.sql", &rest[..end]);
            if !on_disk.contains(&name) {
                missing.push(format!("{dir}/{name}"));
            }
        }
    }
    assert!(missing.is_empty(), "registered but absent: {missing:?}");
}

/// A registered file must also be *applied*. A `const` with no matching
/// `apply_*` call compiles, reads as done, and runs nothing.
#[test]
fn every_registered_constant_is_applied() {
    let src = register_source();
    let mut declared = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("const ") {
            if let Some(name) = rest.split(':').next() {
                if name.starts_with("POSTGRES_UP_V") || name.starts_with("SQLITE_UP_V") {
                    declared.push(name.to_string());
                }
            }
        }
    }
    assert!(
        declared.len() > 100,
        "scan found only {} constants — it is no longer matching the file",
        declared.len()
    );
    let unapplied: Vec<_> = declared
        .iter()
        .filter(|name| !src.contains(&format!(", {name})")))
        .cloned()
        .collect();
    assert!(
        unapplied.is_empty(),
        "declared but never applied: {unapplied:?}"
    );
}
