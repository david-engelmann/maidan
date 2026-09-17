//! Numbers stated in prose, checked against the thing they describe.
//!
//! A count written into a sentence is a copy, and copies drift. The MCP tool
//! count has been wrong three times — documented as 78, corrected to 85,
//! corrected to 155, actually 178 — each time found by a reader rather than by
//! CI, because nothing connected the sentence to the catalog it was counting.
//!
//! These tests are that connection. They do not check style or phrasing; they
//! check that a number a newcomer will believe is the number the repo can
//! prove. When one fails, the doc is wrong — update the prose, not the test.

use std::{fs, path::PathBuf};

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

/// The single number a caller uses to decide whether to filter the catalog.
#[test]
fn the_documented_mcp_tool_count_matches_the_catalog() {
    let catalog: Vec<String> =
        serde_json::from_str(&read("contracts/mcp-tool-names.json")).expect("parse tool names");
    let protocols = read("docs/Protocols.md");

    let claimed = protocols
        .lines()
        .find_map(|line| {
            line.strip_prefix("MCP tool count is **")
                .and_then(|rest| rest.split("**").next())
                .and_then(|n| n.parse::<usize>().ok())
        })
        .expect("docs/Protocols.md should state `MCP tool count is **N**.`");

    assert_eq!(
        claimed,
        catalog.len(),
        "docs/Protocols.md says {claimed} MCP tools; contracts/mcp-tool-names.json has {}",
        catalog.len()
    );
}

/// The first concrete fact `CLAUDE.md` tells an agent about the workspace.
#[test]
fn the_documented_crate_count_matches_the_workspace() {
    let crates = fs::read_dir(repo_root().join("crates"))
        .expect("read crates/")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("Cargo.toml").exists())
        .count();

    let guide = read("CLAUDE.md");
    let claimed = guide
        .split("Workspace with ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse::<usize>().ok())
        .expect("CLAUDE.md should state `Workspace with N member crates.`");

    assert_eq!(
        claimed, crates,
        "CLAUDE.md says {claimed} member crates; crates/ holds {crates}"
    );
}

/// The README tells a reader to pin a tag and then shows one. A tag that has
/// aged out is worse than no example: it reads as the current release and is
/// sixty releases behind. The newest CHANGELOG entry is the in-repo stand-in
/// for "latest release" — a retro writes it before the tag is cut.
#[test]
fn the_readme_image_pin_is_the_current_release() {
    let changelog = read("CHANGELOG.md");
    let newest = changelog
        .lines()
        .find_map(|line| {
            line.strip_prefix("## [")
                .and_then(|rest| rest.split(']').next())
                .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .map(str::to_string)
        })
        .expect("CHANGELOG.md should have a released `## [X.Y.Z]` heading");

    let readme = read("README.md");
    let pinned = readme
        .split("ghcr.io/david-engelmann/maidan-server:v")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("README.md should show a pinned `maidan-server:vX.Y.Z` image");

    assert_eq!(
        pinned, newest,
        "README pins maidan-server:v{pinned}; newest release in CHANGELOG.md is {newest}"
    );
}
