//! Every SQLite transaction that can write takes the write lock when it
//! begins.
//!
//! A deferred `BEGIN` that reads before it writes fails at once with
//! "database is locked" when another connection commits in between: WAL
//! cannot upgrade a stale snapshot to a writer, and `busy_timeout` does not
//! apply (`src/sqlite/tx.rs` has the detail and the tests that show it). The
//! store opens those transactions through `tx::begin_write` and
//! `tx::begin_write_on`. This reads the source, so a new `pool.begin()`,
//! `Connection::begin` or hand-written `begin_with` under `src/sqlite/` fails
//! here unless it is listed below as a read-only transaction or a savepoint,
//! with the reason.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// `(module, enclosing function, sites, reason)`: the `begin` calls under
/// `src/sqlite/` that need not take the write lock.
const DEFERRED: &[(&str, &str, usize, &str)] = &[
    (
        "content_keys",
        "rewrap",
        2,
        "savepoints inside rewrap's own begin_write transaction, one per row",
    ),
    (
        "plans",
        "close_parent_in_tx",
        1,
        "a savepoint inside the caller's transaction, which took the lock",
    ),
    (
        "reviews",
        "evidence_changed",
        1,
        "reads only, then rolls back: a consistent snapshot, no write to upgrade to",
    ),
];

/// What opens a transaction or a savepoint in sqlx.
const BEGINS: [&str; 4] = [
    ".begin()",
    ".begin_with(",
    "Connection::begin(",
    "Transaction::begin(",
];

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every `.rs` file under `dir`, by its path relative to `src/sqlite`
/// without the extension.
fn modules(dir: &Path, prefix: &str, out: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(dir).expect("sqlite dir") {
        let path = entry.expect("entry").path();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let name = format!("{prefix}{stem}");
        if path.is_dir() {
            modules(&path, &format!("{name}/"), out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.insert(name, read(&path));
        }
    }
}

/// The name of the last `fn` declared before `line` in `lines`.
fn enclosing_fn(lines: &[&str], line: usize) -> String {
    lines[..=line]
        .iter()
        .rev()
        .find_map(|l| {
            let at = l.find("fn ")?;
            if at > 0 && l.as_bytes()[at - 1].is_ascii_alphanumeric() {
                return None;
            }
            let name: String = l[at + 3..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .unwrap_or_default()
}

/// Each `begin` site outside `tx.rs`: `(module, function, line)`.
fn begin_sites() -> Vec<(String, String, usize)> {
    let mut mods = BTreeMap::new();
    modules(&src().join("sqlite"), "", &mut mods);
    assert!(mods.len() > 50, "found {} sqlite modules", mods.len());
    let mut sites = Vec::new();
    for (module, source) in &mods {
        if module == "tx" {
            continue;
        }
        let lines: Vec<&str> = source.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            if BEGINS.iter().any(|b| code.contains(b)) {
                sites.push((module.clone(), enclosing_fn(&lines, i), i + 1));
            }
        }
    }
    sites
}

#[test]
fn every_sqlite_transaction_that_writes_begins_with_the_write_lock() {
    let sites = begin_sites();
    let unlisted: Vec<String> = sites
        .iter()
        .filter(|(module, function, _)| {
            !DEFERRED
                .iter()
                .any(|(m, f, _, _)| m == module && f == function)
        })
        .map(|(module, function, line)| format!("src/sqlite/{module}.rs:{line} in {function}"))
        .collect();
    assert!(
        unlisted.is_empty(),
        "these open a transaction without the write lock. A transaction that can \
         write begins with `super::tx::begin_write(pool)` (or `begin_write_on(conn)` \
         on a connection); one that only reads, or a savepoint, goes in DEFERRED \
         with its reason: {unlisted:#?}"
    );
}

#[test]
fn every_deferred_entry_names_the_sites_it_excuses() {
    let sites = begin_sites();
    for (module, function, expected, _) in DEFERRED {
        let found = sites
            .iter()
            .filter(|(m, f, _)| m == module && f == function)
            .count();
        assert_eq!(
            found, *expected,
            "DEFERRED lists {expected} begin site(s) in {module}::{function}, the source has \
             {found}: update the entry, or remove it if the function is gone"
        );
    }
}

#[test]
fn sqlite_migrations_take_the_write_lock() {
    let source = read(&src().join("migrate.rs"));
    let at = source.find("async fn apply_sqlite(").expect("apply_sqlite");
    let body = &source[at..];
    let body = &body[..body.find("\n}\n").expect("end of apply_sqlite")];
    assert!(
        body.contains("tx::begin_write(") && !body.contains(".begin()"),
        "apply_sqlite must open its transaction with sqlite::tx::begin_write"
    );
}
