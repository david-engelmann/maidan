//! Golden file for MCP tool names (Cluster 59).

use std::path::PathBuf;

#[test]
fn mcp_tool_names_match_contract_file() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../../contracts/mcp-tool-names.json");
    let expected: Vec<String> = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("contract json");
    let mut actual: Vec<String> = maidan_mcp::tools::catalog()
        .into_iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect();
    actual.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(
        actual, expected,
        "update contracts/mcp-tool-names.json if intentional"
    );
}
