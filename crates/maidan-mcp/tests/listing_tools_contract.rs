//! The listing manifests under `listings/` are prepared, never published
//! from CI — but they must not drift from the server they describe. The
//! Docker catalog's `tools.json` is a projection of `tools/list`, so this
//! test fails the day a tool is added, renamed or re-described without
//! regenerating it, and the registry and ToolHive entries must name the
//! same published image.

use serde_json::{json, Value};

fn read_listing(path: &str) -> Value {
    let full = concat!(env!("CARGO_MANIFEST_DIR"), "/../../").to_string() + path;
    let raw = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{path} unreadable: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{path} is not valid JSON: {e}"))
}

#[test]
fn the_docker_catalog_tools_file_matches_tools_list() {
    let file = read_listing("listings/docker/tools.json");
    let expected: Vec<Value> = maidan_mcp::tools::catalog()
        .iter()
        .map(|tool| {
            json!({
                "name": tool["name"],
                "description": tool["description"],
            })
        })
        .collect();
    assert_eq!(
        file["tools"],
        Value::Array(expected),
        "listings/docker/tools.json drifted from the tools/list catalog; regenerate it from maidan_mcp::tools::catalog()"
    );
}

#[test]
fn the_registry_and_toolhive_entries_name_the_same_published_image() {
    let image = "ghcr.io/david-engelmann/maidan-server:v413.0.0";

    let registry = read_listing("listings/registry/server.json");
    let package = &registry["packages"][0];
    assert_eq!(package["registryType"], "oci");
    assert_eq!(package["registryBaseUrl"], "https://ghcr.io");
    assert_eq!(package["identifier"], "david-engelmann/maidan-server");
    assert_eq!(package["version"], "v413.0.0");
    // Together: ghcr.io/david-engelmann/maidan-server:v413.0.0 — the `image`
    // the ToolHive assertion below pins literally.

    let toolhive = read_listing("listings/toolhive/server.json");
    assert_eq!(toolhive["image"], image);
    let names: Vec<Value> = maidan_mcp::tools::catalog()
        .iter()
        .map(|tool| tool["name"].clone())
        .collect();
    assert_eq!(
        toolhive["tools"],
        Value::Array(names),
        "the ToolHive entry lists the catalog's tool names"
    );
}
