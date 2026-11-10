//! Authority changes are audited in their own transaction (D-A).
//!
//! The maintainer decided on 2026-09-23 that an action changing authority
//! writes its audit row inside the change's transaction, so a failed write
//! aborts the change instead of leaving it unrecorded. The store exposes an
//! `_audited` form of each such method. This fails if request-handling code
//! calls the unaudited form, which would quietly bring back the best-effort
//! write.
//!
//! The unaudited forms are compiled only under maidan-store's `test-support`
//! feature, which only dev-dependencies enable. A non-test build of the
//! server, the MCP tools or the CLI cannot name them, so CI's
//! `cargo build --workspace --lib --bins` fails on a request-path call before
//! the scan below runs. `unaudited_forms_are_pinned_behind_test_support` keeps a new unaudited
//! method from skipping that gate.
//!
//! The scan covers every workspace crate and example, not only the two
//! crates that serve requests. Two carve-outs are named and pinned (Open
//! Work later-S / thread 14b), so a call elsewhere cannot slip past:
//! `maidan init` mints through the audited form, and the Playwright
//! harnesses seed fixtures through the store directly.
//!
//! Test items are skipped one at a time. This used to stop reading a file at
//! its first `#[cfg(test)]`, which left 1,693 lines of real code in six files
//! unscanned, among them all of `openapi/mod.rs` (a `#[cfg(test)] use` on its
//! third line) and the MCP tool dispatch after a test-only constant in
//! `tools/mod.rs`.

mod source_scan;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use source_scan::{rust_files, without_tests};

/// Store calls that change authority. Each has an `_audited` twin.
const UNAUDITED: &[&str] = &[
    ".create_api_token(",
    ".create_attenuated_api_token(",
    ".create_delegated_api_token(",
    ".revoke_api_token(",
    ".create_delegation_grant(",
    ".revoke_delegation_grant(",
    ".create_share_ticket(",
    ".revoke_share_ticket(",
    ".set_delegation_policy(",
    ".purge_workspace_messages(",
    ".erase_workspace(",
    ".import_workspace(",
    ".purge_message(",
    ".place_legal_hold(",
    ".lift_legal_hold(",
    ".freeze_member(",
    ".unfreeze_member(",
    ".set_review_requirement(",
    ".clear_review_requirement(",
    ".remove_reviewer(",
    ".clear_land_gate(",
    ".allow_egress_target(",
    ".revoke_egress_target(",
    ".create_app_installation(",
    ".revoke_app_installation(",
    ".create_secret(",
    ".delete_secret(",
    ".allow_secret_egress_host(",
    ".revoke_secret_egress_host(",
    ".add_channel_member(",
    ".remove_channel_member(",
    ".create_scim_user(",
    ".update_scim_user(",
    ".delete_scim_user(",
    ".create_session(",
    ".delete_session(",
];

/// Playwright seed-and-serve harnesses. They are test support, not shipped
/// binaries, and may call only the unaudited methods listed here. Each listed
/// call must still appear, so dropping one is a deliberate edit of this pin.
const PLAYWRIGHT_HARNESS: &[(&str, &[&str])] = &[
    (
        "crates/maidan-server/examples/ui_test_server.rs",
        &[
            ".create_api_token(",
            ".create_session(",
            ".set_review_requirement(",
        ],
    ),
    (
        "crates/maidan-server/examples/capture_server.rs",
        &[".create_api_token(", ".set_review_requirement("],
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn rel(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Workspace crate directory names from the root Cargo.toml `members` list.
fn workspace_crate_names(root: &Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let start = text
        .find("members = [")
        .expect("workspace Cargo.toml has a members list");
    let rest = &text[start..];
    let end = rest.find(']').expect("members list is closed");
    rest[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix('"')?;
            let path = line.split('"').next()?;
            path.strip_prefix("crates/").map(str::to_string)
        })
        .collect()
}

/// Production Rust in every workspace crate and in the repo examples, not tests.
fn production_rust(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let crates = root.join("crates");
    for entry in std::fs::read_dir(&crates).unwrap() {
        let crate_dir = entry.unwrap().path();
        if !crate_dir.is_dir() {
            continue;
        }
        for sub in ["src", "examples", "benches"] {
            let p = crate_dir.join(sub);
            if p.exists() {
                rust_files(&p, &mut files);
            }
        }
    }
    let examples = root.join("examples");
    if examples.exists() {
        rust_files(&examples, &mut files);
    }
    files.sort();
    files
}

fn unaudited_on_line(line: &str) -> impl Iterator<Item = &'static str> + '_ {
    let code = !line.trim_start().starts_with("//");
    UNAUDITED
        .iter()
        .copied()
        .filter(move |call| code && line.contains(call))
}

#[test]
fn authority_changes_are_audited_in_their_transaction() {
    let root = workspace_root();
    let files = production_rust(&root);
    assert!(files.len() > 50, "found only {} files", files.len());

    let crate_names: BTreeSet<String> = files
        .iter()
        .filter_map(|f| {
            let rel = rel(&root, f);
            let rest = rel.strip_prefix("crates/")?;
            Some(rest.split('/').next()?.to_string())
        })
        .collect();
    let members = workspace_crate_names(&root);
    assert_eq!(
        crate_names, members,
        "scanned crates must match workspace members"
    );

    let allowed: HashMap<&str, &[&str]> = PLAYWRIGHT_HARNESS.iter().copied().collect();
    let mut offenders = Vec::new();
    let mut seen: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for file in &files {
        let path = rel(&root, file);
        let source = std::fs::read_to_string(file).unwrap();
        for (line_no, line) in without_tests(&source).lines().enumerate() {
            for call in unaudited_on_line(line) {
                if let Some((pinned, pins)) = allowed.get_key_value(path.as_str())
                    && pins.contains(&call)
                {
                    seen.entry(*pinned).or_default().insert(call);
                    continue;
                }
                offenders.push(format!("{path}:{}: {call}", line_no + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "an authority change outside its audit transaction: {offenders:?}"
    );
    for (path, pins) in PLAYWRIGHT_HARNESS {
        let found = seen.get(path).cloned().unwrap_or_default();
        for call in *pins {
            assert!(
                found.contains(call),
                "{path} no longer calls {call}; update the Playwright harness pin"
            );
        }
    }
}

/// `maidan init` is the other named carve-out. It is not a hole: the
/// bootstrap mints the first admin token through the audited form, and the
/// unaudited form is unreachable in that binary.
#[test]
fn maidan_init_mints_through_the_audited_form() {
    let path = workspace_root().join("crates/maidan-cli/src/main.rs");
    let source = std::fs::read_to_string(&path).unwrap();
    let code = without_tests(&source);
    assert!(
        code.contains("create_api_token_audited("),
        "maidan init must mint through create_api_token_audited"
    );
    let mut offenders = Vec::new();
    for (line_no, line) in code.lines().enumerate() {
        for call in unaudited_on_line(line) {
            offenders.push(format!(
                "crates/maidan-cli/src/main.rs:{}: {call}",
                line_no + 1
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "maidan init called an unaudited authority method: {offenders:?}"
    );
}

/// Every unaudited authority method, on the trait and on the backend
/// delegation, is compiled only under `test-support`, so it is a compile error
/// to call one from non-test code.
#[test]
fn unaudited_forms_are_pinned_behind_test_support() {
    let store = Path::new(env!("CARGO_MANIFEST_DIR")).join("../maidan-store/src");
    for file in ["store.rs", "delegate.rs"] {
        let source = std::fs::read_to_string(store.join(file)).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for call in UNAUDITED {
            let name = call.trim_start_matches('.').trim_end_matches('(');
            let decl = format!("async fn {name}(");
            let at: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.trim_start().starts_with(&decl))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(
                at.len(),
                1,
                "{file}: expected one `{decl}`, found {}",
                at.len()
            );
            let above = lines[at[0] - 1].trim();
            assert_eq!(
                above, "#[cfg(feature = \"test-support\")]",
                "{file}: `{name}` is reachable outside tests; gate it behind test-support"
            );
        }
    }
}
