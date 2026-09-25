//! Entity ids are UUIDv7 (Wave 4 #44). A v7 id sorts by creation time, so
//! primary-key indexes stay append-mostly and an id alone says roughly when a
//! row was made. A random v4 belongs only where an id must be unguessable.
//!
//! This fails on a `Uuid::new_v4()` in production code outside the allowlist.
//! Tests may use v4 freely. Postgres 16 has no `uuidv7()`, so every id is minted
//! app-side.

use std::path::{Path, PathBuf};

/// Where v4 is required, and why. The count is exact, so adding a v4 call to
/// one of these files still needs a reason here.
const ALLOWED: &[(&str, usize, &str)] = &[
    (
        "maidan-auth/src/token.rs",
        4,
        "token and share-ticket secrets: 256 random bits",
    ),
    (
        "maidan-server/src/app_oauth.rs",
        1,
        "OAuth authorization code: a bearer value",
    ),
    (
        "maidan-mcp/src/server.rs",
        1,
        "MCP session id: presented as the session credential",
    ),
    (
        "maidan-store/src/postgres/sessions.rs",
        1,
        "browser session id, carried in the (signed) session cookie",
    ),
    (
        "maidan-store/src/sqlite/sessions.rs",
        1,
        "browser session id, carried in the (signed) session cookie",
    ),
];

fn rust_files(path: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn entity_ids_are_v7() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found: Vec<(String, usize)> = Vec::new();
    for krate in std::fs::read_dir(&crates).unwrap() {
        let src = krate.unwrap().path().join("src");
        if !src.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            let code = source.split("#[cfg(test)]").next().unwrap_or_default();
            let count = code
                .lines()
                .filter(|l| !l.trim_start().starts_with("//") && l.contains("new_v4()"))
                .count();
            if count > 0 {
                let rel = file
                    .strip_prefix(&crates)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                found.push((rel, count));
            }
        }
    }
    assert!(
        !found.is_empty(),
        "the scan found no v4 calls at all; it is broken"
    );

    let mut problems = Vec::new();
    for (file, count) in &found {
        match ALLOWED.iter().find(|(f, _, _)| f == file) {
            Some((_, allowed, _)) if count == allowed => {}
            Some((_, allowed, why)) => problems.push(format!(
                "{file}: {count} v4 calls, {allowed} allowed ({why})"
            )),
            None => problems.push(format!(
                "{file}: {count} v4 calls; entity ids use Uuid::now_v7() or the id type's new()"
            )),
        }
    }
    for (file, _, _) in ALLOWED {
        if !found.iter().any(|(f, _)| f == file) {
            problems.push(format!(
                "{file} is allowlisted but has no v4 call; remove it"
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
