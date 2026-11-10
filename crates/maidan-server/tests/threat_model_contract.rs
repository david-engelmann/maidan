//! Each Primary-threats row in `docs/Threat-Model.md` names the test that
//! holds its mitigation. This contract is the connection: every row has a
//! Test cell, and every name resolves to a Rust test function or a `*.rs`
//! file stem in the repo. A name that drifts, or a new row with no test,
//! fails here rather than in a reader's head.
//!
//! T6 (sqlx parameterized queries) is a coding practice, not a named test.
//! The contract pins that single exception as `practice` so a later row
//! cannot silently reuse the word.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Threat IDs allowed to name `practice` instead of a resolvable test.
const PRACTICE_IDS: &[&str] = &["T6"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

struct Row {
    id: String,
    tests: Vec<String>,
}

/// File stems and `fn` names found under the repo (excluding build and VCS trees).
struct RepoIndex {
    stems: BTreeSet<String>,
    functions: BTreeSet<String>,
}

fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        "target" | ".git" | "node_modules" | ".claude" | "dist"
    )
}

fn index_repo(root: &Path) -> RepoIndex {
    let mut stems = BTreeSet::new();
    let mut functions = BTreeSet::new();
    fn walk(dir: &Path, stems: &mut BTreeSet<String>, functions: &mut BTreeSet<String>) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if skip_dir(&name) {
                    continue;
                }
                walk(&path, stems, functions);
                continue;
            }
            if path.extension().is_some_and(|e| e == "rs") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    stems.insert(stem.to_string());
                }
                if let Ok(src) = fs::read_to_string(&path) {
                    for line in src.lines() {
                        let line = line.trim_start();
                        let rest = line
                            .strip_prefix("async fn ")
                            .or_else(|| line.strip_prefix("fn "));
                        let Some(rest) = rest else { continue };
                        let name: String = rest
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if !name.is_empty() {
                            functions.insert(name);
                        }
                    }
                }
            }
        }
    }
    walk(root, &mut stems, &mut functions);
    RepoIndex { stems, functions }
}

fn parse_primary_threats(doc: &str) -> Vec<Row> {
    let section = doc
        .split("## Primary threats")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("docs/Threat-Model.md should have a `## Primary threats` section");

    let mut headers: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<String> = line
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        if cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-'))
        {
            continue;
        }
        if headers.is_none() {
            headers = Some(cells);
            continue;
        }
        let headers = headers.as_ref().expect("header");
        let get = |name: &str| -> String {
            headers
                .iter()
                .position(|h| h == name)
                .and_then(|i| cells.get(i).cloned())
                .unwrap_or_default()
        };
        let id = get("ID");
        if id.is_empty() {
            continue;
        }
        rows.push(Row {
            id,
            tests: parse_test_names(&get("Test")),
        });
    }

    let headers = headers.expect("Primary threats table should have a header row");
    assert!(
        headers.iter().any(|h| h == "Test"),
        "docs/Threat-Model.md Primary threats table should have a Test column; found {headers:?}"
    );
    assert!(
        headers.iter().any(|h| h == "ID"),
        "docs/Threat-Model.md Primary threats table should have an ID column; found {headers:?}"
    );
    rows
}

fn parse_test_names(cell: &str) -> Vec<String> {
    cell.split([',', ';'])
        .map(|part| part.trim().trim_matches('`').trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

#[test]
fn every_primary_threat_names_a_test_that_exists() {
    let rows = parse_primary_threats(&read("docs/Threat-Model.md"));
    assert!(
        !rows.is_empty(),
        "docs/Threat-Model.md should list at least one Primary threats row"
    );

    let index = index_repo(&repo_root());
    let mut missing = Vec::new();
    for row in &rows {
        assert!(
            row.id.starts_with('T')
                && row.id[1..].chars().all(|c| c.is_ascii_digit())
                && row.id.len() > 1,
            "Primary threats ID should look like T1; found {}",
            row.id
        );
        if row.tests.is_empty() {
            missing.push(format!("{}: empty Test cell", row.id));
            continue;
        }
        for name in &row.tests {
            if name == "practice" {
                assert!(
                    PRACTICE_IDS.contains(&row.id.as_str()),
                    "{} names `practice`, but only {} may",
                    row.id,
                    PRACTICE_IDS.join(", ")
                );
                continue;
            }
            if index.stems.contains(name) || index.functions.contains(name) {
                continue;
            }
            missing.push(format!(
                "{}: `{name}` is not a *.rs stem or fn in the repo",
                row.id
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "docs/Threat-Model.md Test names must exist in the repo:\n  {}",
        missing.join("\n  ")
    );
}
