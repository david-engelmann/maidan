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
//! Test items are skipped one at a time. This used to stop reading a file at
//! its first `#[cfg(test)]`, which left 1,693 lines of real code in six files
//! unscanned, among them all of `openapi/mod.rs` (a `#[cfg(test)] use` on its
//! third line) and the MCP tool dispatch after a test-only constant in
//! `tools/mod.rs`.

mod source_scan;

use std::path::Path;

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

#[test]
fn authority_changes_are_audited_in_their_transaction() {
    let server = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Everything that serves a request: the server's source, and the MCP tools.
    let mut files = Vec::new();
    rust_files(&server.join("src"), &mut files);
    rust_files(&server.join("../maidan-mcp/src"), &mut files);
    assert!(files.len() > 50, "found only {} files", files.len());

    let mut offenders = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        for (line_no, line) in without_tests(&source).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for call in UNAUDITED {
                if line.contains(call) {
                    offenders.push(format!("{}:{}: {}", file.display(), line_no + 1, call));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "an authority change outside its audit transaction: {offenders:?}"
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
