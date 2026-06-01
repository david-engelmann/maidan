//! Golden map: MCP tool name → required capability (Cluster 69).

use std::collections::BTreeMap;
use std::path::PathBuf;

#[test]
fn mcp_capability_map_matches_tools_and_contract() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let names_path = root.join("contracts/mcp-tool-names.json");
    let map_path = root.join("contracts/mcp-capability-map.json");

    let names: Vec<String> = serde_json::from_slice(
        &std::fs::read(&names_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", names_path.display())),
    )
    .expect("tool names json");

    let map: BTreeMap<String, String> = serde_json::from_slice(
        &std::fs::read(&map_path).unwrap_or_else(|e| panic!("read {}: {e}", map_path.display())),
    )
    .expect("capability map json");

    for name in &names {
        let cap = map
            .get(name)
            .unwrap_or_else(|| panic!("missing capability map entry for tool {name}"));
        let code = maidan_mcp::tools::required_capability(name)
            .unwrap_or_else(|e| panic!("required_capability({name}): {e}"));
        assert_eq!(
            code,
            cap.as_str(),
            "map says {cap} but tools::required_capability returns {code} for {name}"
        );
    }

    for key in map.keys() {
        assert!(
            names.iter().any(|n| n == key),
            "capability map has unknown tool {key}; update mcp-tool-names.json"
        );
    }
}
