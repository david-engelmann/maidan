//! No code outside `maidan-auth` makes an `AuthContext` (SEC-003, R11).
//!
//! A caller is something a credential or a stored record proved, so the only
//! ways in are `maidan-auth`'s resolvers. Its constructors are crate-private in
//! a build that runs, so a call from another crate's library code already fails
//! the strict lint pass (`--lib --bins`, which turns no dev feature on). A test
//! build turns `maidan-auth/test-support` on and sees them, which is how the
//! same call would compile there. This reads the source so the rule holds in
//! both builds: every crate but `maidan-auth`, test items and test-only modules
//! left out.

mod source_scan;

use std::path::{Path, PathBuf};

use source_scan::{rust_files, without_tests};

const CONSTRUCTORS: &[&str] = &[
    "AuthContext::from_token",
    "AuthContext::from_app_token",
    "AuthContext::from_session",
    "AuthContext::from_delegated_token",
    "AuthContext::anonymous_reader",
];

/// Files a `#[cfg(test)] mod name;` declares, which only a test build compiles.
fn test_only_modules(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(file).unwrap();
        let mut lines = source.lines().map(str::trim);
        while let Some(line) = lines.next() {
            if line != "#[cfg(test)]" {
                continue;
            }
            let Some(next) = lines.next() else { break };
            let Some(name) = next
                .trim_start_matches("pub(crate) ")
                .trim_start_matches("pub ")
                .strip_prefix("mod ")
                .and_then(|rest| rest.strip_suffix(';'))
            else {
                continue;
            };
            let dir = file.parent().unwrap();
            let stem = file.file_stem().unwrap();
            let base = if stem == "mod" || stem == "lib" || stem == "main" {
                dir.to_path_buf()
            } else {
                dir.join(stem)
            };
            out.push(base.join(format!("{name}.rs")));
            out.push(dir.join(format!("{name}.rs")));
            out.push(base.join(name).join("mod.rs"));
        }
    }
    out
}

fn library_files() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap() {
        let dir = entry.unwrap().path();
        if dir.file_name().is_some_and(|n| n == "maidan-auth") {
            continue;
        }
        let src = dir.join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    let test_only = test_only_modules(&files);
    files.retain(|f| !test_only.contains(f));
    files
}

#[test]
fn no_library_code_outside_maidan_auth_constructs_an_auth_context() {
    let files = library_files();
    assert!(files.len() > 300, "found only {} source files", files.len());
    let mut offenders = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        for (line_no, line) in without_tests(&source).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(ctor) = CONSTRUCTORS.iter().find(|c| line.contains(**c)) {
                offenders.push(format!("{}:{}: {ctor}", file.display(), line_no + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "build an AuthContext only through a maidan-auth resolver \
         (resolve_bearer, resolve_token_id, resolve_session, \
         resolve_anonymous_reader, member_can_access_*):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn a_test_only_module_is_left_out_of_the_scan() {
    let files = library_files();
    assert!(
        !files
            .iter()
            .any(|f| f.ends_with("maidan-mcp/src/tools/agent_gap_tests.rs")),
        "agent_gap_tests.rs is declared under #[cfg(test)] and should not be scanned"
    );
}
