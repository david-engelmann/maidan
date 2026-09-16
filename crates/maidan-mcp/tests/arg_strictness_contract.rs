//! Cluster 398.6: every MCP tool argument struct rejects unknown fields.
//!
//! A static check over `src/tools/*.rs`, in the shape of the repo's other
//! grep-style contract guards (`ui_js_contract`). The property is worth pinning
//! rather than re-deriving: it only holds if it holds *everywhere*, and the
//! failure mode of a gap is silence, not a test failure — an absorbed field
//! produces a successful call that quietly did something else.
//!
//! Two of these were real. `SetBudgetArgs` (398.4): omission means "remove that
//! limit", so a typo'd dimension disarmed a safety control and returned `200`
//! with the budget echoed back. `RequestApprovalArgs` (398.5): a typo'd
//! `thread_id` left the approval gate unattached, so `claim_next` handed the
//! thread to an agent without waiting for the human.

use std::{fs, path::PathBuf};

/// Every `#[derive(…Deserialize…)] struct …Args` must also be
/// `#[serde(deny_unknown_fields)]`.
#[test]
fn every_tool_argument_struct_rejects_unknown_fields() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/tools");
    let mut checked = 0usize;
    let mut offenders: Vec<String> = Vec::new();

    for entry in fs::read_dir(&dir).expect("read tools dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let file = path.file_name().and_then(|f| f.to_str()).unwrap_or("?");
        let src = fs::read_to_string(&path).expect("read tool source");

        // Walk the attribute block immediately above each `struct …Args {`.
        for (idx, _) in src.match_indices("struct ") {
            let decl = &src[idx..];
            let name = decl
                .trim_start_matches("struct ")
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            if !name.ends_with("Args") {
                continue;
            }
            // The contiguous run of `#[...]` lines directly above the struct.
            let before = &src[..idx];
            let attrs: Vec<&str> = before
                .lines()
                .rev()
                .take_while(|l| {
                    let t = l.trim();
                    t.starts_with("#[") || t.starts_with("///") || t.starts_with("//")
                })
                .collect();
            let attrs = attrs.join("\n");
            if !attrs.contains("Deserialize") {
                continue; // not a wire type
            }
            checked += 1;
            if !attrs.contains("deny_unknown_fields") {
                offenders.push(format!("{file}::{name}"));
            }
        }
    }

    assert!(
        checked > 100,
        "expected to find the full tool-argument surface, only saw {checked} — \
         the scan is probably broken, not the code"
    );
    assert!(
        offenders.is_empty(),
        "these tool argument structs would silently absorb an unknown field \
         (add #[serde(deny_unknown_fields)]): {offenders:#?}"
    );
}
