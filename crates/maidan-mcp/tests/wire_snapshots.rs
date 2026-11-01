//! Wire snapshots for the `tools/list` results.
//!
//! A connected app reads its whole world from `tools/list`; a tool's
//! description or input schema changing silently is a behaviour change
//! for every client. The catalog contract files pin tool *names*; these
//! fixtures pin the full result bytes the handler builds — the tool
//! objects plus the cache hint fields — for the three lists that do not
//! depend on a caller: the full catalog (what every per-token list on
//! `/mcp` is filtered from) and the two profile lists, which are the same
//! bytes for every caller by design.
//!
//! Regenerate with `MAIDAN_WRITE_SNAPSHOTS=1` when a change is intended.

use maidan_mcp::caching::{with_hint, PROFILE_TOOLS_LIST, TOOLS_LIST};
use maidan_mcp::profiles::Profile;
use maidan_mcp::tools;
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wire")
}

fn pretty(value: &Value) -> String {
    let mut s = serde_json::to_string_pretty(value).expect("pretty");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn assert_snapshot(name: &str, value: &Value) {
    let got = pretty(value);
    let path = fixtures_dir().join(name);
    if std::env::var("MAIDAN_WRITE_SNAPSHOTS").ok().as_deref() == Some("1") {
        std::fs::create_dir_all(fixtures_dir()).expect("mkdir");
        std::fs::write(&path, &got).expect("write snapshot");
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got, expected,
        "wire snapshot drift in {name} — rerun with MAIDAN_WRITE_SNAPSHOTS=1 if intentional"
    );
}

#[test]
fn the_full_tools_list_result_is_pinned() {
    let result = with_hint(json!({ "tools": tools::catalog() }), TOOLS_LIST);
    assert_snapshot("tools_list_full.json", &result);
}

#[test]
fn the_worker_tools_list_result_is_pinned() {
    let result = with_hint(
        json!({ "tools": Profile::Worker.catalog() }),
        PROFILE_TOOLS_LIST,
    );
    assert_snapshot("tools_list_worker.json", &result);
}

#[test]
fn the_reviewer_tools_list_result_is_pinned() {
    let result = with_hint(
        json!({ "tools": Profile::Reviewer.catalog() }),
        PROFILE_TOOLS_LIST,
    );
    assert_snapshot("tools_list_reviewer.json", &result);
}

#[test]
fn the_coordinator_tools_list_result_is_pinned() {
    let result = with_hint(
        json!({ "tools": Profile::Coordinator.catalog() }),
        PROFILE_TOOLS_LIST,
    );
    assert_snapshot("tools_list_coordinator.json", &result);
}
