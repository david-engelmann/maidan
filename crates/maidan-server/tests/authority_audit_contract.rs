//! Authority changes are audited in their own transaction (D-A).
//!
//! The maintainer decided on 2026-09-23 that an action changing authority
//! writes its audit row inside the change's transaction, so a failed write
//! aborts the change instead of leaving it unrecorded. The store exposes an
//! `_audited` form of each such method. This fails if request-handling code
//! calls the unaudited form, which would quietly bring back the best-effort
//! write.
//!
//! The unaudited forms stay on the trait for tests, fixtures and the offline
//! `maidan init` bootstrap, none of which runs as a request.

use std::path::{Path, PathBuf};

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
    ".revoke_app_installation(",
    ".create_secret(",
    ".delete_secret(",
    ".add_channel_member(",
    ".remove_channel_member(",
    ".create_scim_user(",
    ".update_scim_user(",
    ".delete_scim_user(",
];

fn rust_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
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
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        for (line_no, line) in code.lines().enumerate() {
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
