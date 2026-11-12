//! No code outside `maidan-auth` makes an `AuthContext`, and no code anywhere
//! has a caller that skips the checks (SEC-003, SEC-004, R11).
//!
//! A caller is something a credential or a stored record proved, so the only
//! ways in are `maidan-auth`'s resolvers. Its constructors are crate-private in
//! a build that runs, so a call from another crate's library code already fails
//! the strict lint pass (`--lib --bins`, which turns no dev feature on). A test
//! build turns `maidan-auth/test-support` on and sees them, which is how the
//! same call would compile there. This reads the source so the rule holds in
//! both builds: every crate but `maidan-auth`, test items and test-only modules
//! left out.
//!
//! The server once had an unauthenticated mode (`AUTH_DISABLED`), served by a
//! context that passed every check, and 191 branches read its flag. It is
//! gone, and the second test keeps any trace of it from coming back.

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

fn library_files(include_auth: bool) -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap() {
        let dir = entry.unwrap().path();
        if !include_auth && dir.file_name().is_some_and(|n| n == "maidan-auth") {
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
    let files = library_files(false);
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
    let files = library_files(true);
    assert!(
        !files
            .iter()
            .any(|f| f.ends_with("maidan-mcp/src/tools/agent_gap_tests.rs")),
        "agent_gap_tests.rs is declared under #[cfg(test)] and should not be scanned"
    );
}

/// What the unauthenticated mode was made of. Each is a token of code, not a
/// word: "bypass" alone also names unrelated things (a rate-limit test, a
/// review comment's text).
const BYPASS_TRACES: &[&str] = &[
    "AuthContext::bypass",
    ".bypass",
    "bypass:",
    "auth_disabled",
    "AUTH_DISABLED",
    "ALLOW_INSECURE_NO_AUTH",
    "allow_insecure_no_auth",
    "test_identity_header",
    "maidan-test-member-id",
];

/// The one place a trace may stay, with why.
const ALLOWED: &[(&str, &str)] = &[(
    "maidan-server/src/config.rs",
    "the boot refusal names the removed variables, so setting one stops the server",
)];

#[test]
fn no_library_code_has_a_caller_that_skips_the_checks() {
    let files = library_files(true);
    let mut offenders = Vec::new();
    for file in files {
        let shown = file.to_string_lossy().replace('\\', "/");
        if ALLOWED.iter().any(|(path, _)| shown.ends_with(path)) {
            continue;
        }
        let source = std::fs::read_to_string(&file).unwrap();
        for (line_no, line) in without_tests(&source).lines().enumerate() {
            if let Some(trace) = BYPASS_TRACES.iter().find(|t| line.contains(**t)) {
                offenders.push(format!("{shown}:{}: {trace}", line_no + 1));
            }
        }
    }
    assert_eq!(
        offenders.len(),
        0,
        "every caller is authenticated; the unauthenticated mode is gone (R11):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_allowed_trace_is_still_where_it_is_said_to_be() {
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/config.rs");
    let source = std::fs::read_to_string(config).unwrap();
    assert!(
        without_tests(&source).contains("\"AUTH_DISABLED\""),
        "config.rs no longer refuses AUTH_DISABLED by name; drop it from ALLOWED"
    );
}
