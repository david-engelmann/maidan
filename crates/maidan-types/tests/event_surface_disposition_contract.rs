//! Every event kind has an explicit producer-surface disposition and points to
//! an executable test. This records intentional asymmetry without inventing
//! public writers for locally derived events.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use maidan_types::EventKind;

#[derive(Debug, serde::Deserialize)]
struct Entry {
    kind: String,
    disposition: String,
    rest_evidence: Option<String>,
    mcp_evidence: Option<String>,
    internal_evidence: Option<String>,
    note: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load() -> Vec<Entry> {
    let path = repo_root().join("contracts/event-surface-disposition.json");
    serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .expect("event surface disposition JSON")
}

fn assert_test_exists(root: &Path, kind: &str, evidence: &str) {
    let (relative, test_name) = evidence
        .split_once('#')
        .unwrap_or_else(|| panic!("{kind}: evidence must be path#test_name: {evidence}"));
    let path = root.join(relative);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{kind}: read {}: {error}", path.display()));
    assert!(
        source.contains(&format!("fn {test_name}(")),
        "{kind}: {relative} has no test function {test_name}"
    );
    let function_at = source.find(&format!("fn {test_name}(")).unwrap();
    let declaration_prefix = source[..function_at]
        .rsplit_once('\n')
        .map_or("", |(before, _)| before);
    let immediate_attribute = declaration_prefix
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim);
    assert!(
        immediate_attribute
            .is_some_and(|attribute| { attribute.starts_with("#[") && attribute.contains("test") }),
        "{kind}: {relative}#{test_name} is not directly marked as a test"
    );
}

#[test]
fn every_event_kind_has_an_evidenced_surface_disposition() {
    let root = repo_root();
    let entries = load();
    let expected = EventKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let actual = entries
        .iter()
        .map(|entry| entry.kind.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        entries.len(),
        "duplicate event kind disposition"
    );
    assert_eq!(actual, expected, "EventKind and surface matrix drifted");

    for entry in entries {
        assert!(
            !entry.note.trim().is_empty(),
            "{}: rationale is empty",
            entry.kind
        );
        let expected_evidence = match entry.disposition.as_str() {
            "both" => (true, true, false),
            "rest_only" => (true, false, false),
            "mcp_only" => (false, true, false),
            "internal_only" => (false, false, true),
            other => panic!("{}: unknown disposition {other}", entry.kind),
        };
        let actual_evidence = (
            entry.rest_evidence.is_some(),
            entry.mcp_evidence.is_some(),
            entry.internal_evidence.is_some(),
        );
        assert_eq!(
            actual_evidence, expected_evidence,
            "{}: evidence does not match {}",
            entry.kind, entry.disposition
        );
        for evidence in [
            entry.rest_evidence.as_deref(),
            entry.mcp_evidence.as_deref(),
            entry.internal_evidence.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            assert_test_exists(&root, &entry.kind, evidence);
        }
    }
}
