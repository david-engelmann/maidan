//! `.cursor-plugin/plugin.json` is the Cursor package (Open Work Next 16). It
//! must stay a declared worker-only listing: one MCP server on `/mcp/worker`,
//! the instance and token read from install-time variables, every placeholder
//! declared, no host or secret of its own, and a logo path that exists.

use std::path::PathBuf;

use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest() -> Value {
    let text = std::fs::read_to_string(repo().join(".cursor-plugin/plugin.json"))
        .expect(".cursor-plugin/plugin.json at the repository root");
    serde_json::from_str(&text).expect("plugin.json is JSON")
}

#[test]
fn the_cursor_plugin_is_a_worker_only_listing_with_declared_variables() {
    let m = manifest();
    let name = m["name"].as_str().unwrap_or_default();
    assert_eq!(name, "maidan");

    let servers = m["mcpServers"].as_object().expect("inline mcpServers");
    assert_eq!(servers.len(), 1, "exactly one MCP server: the worker");
    let maidan = &servers["maidan"];
    assert_eq!(maidan["url"], "${MAIDAN_URL}/mcp/worker");
    assert_eq!(maidan["headers"]["Authorization"], "Bearer ${MAIDAN_TOKEN}");

    let vars = &m["variables"];
    assert_eq!(vars["type"], "object");
    let props = vars["properties"].as_object().expect("variable properties");
    let required: Vec<&str> = vars["required"]
        .as_array()
        .expect("required variables")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let text = serde_json::to_string(&m["mcpServers"]).expect("serialize");
    for placeholder in text.split("${").skip(1) {
        let var = placeholder.split('}').next().unwrap_or_default();
        assert!(props.contains_key(var), "{var} is not declared");
        assert!(required.contains(&var), "{var} is not required");
    }
    let token = props["MAIDAN_TOKEN"]["description"]
        .as_str()
        .unwrap_or_default();
    for word in ["admin mints", "expiry", "Never paste a token:admin"] {
        assert!(token.contains(word), "token description lacks {word:?}");
    }

    let logo = m["logo"].as_str().expect("logo");
    assert!(!logo.contains("..") && !logo.starts_with('/'));
    assert!(repo().join(logo).is_file(), "logo {logo} exists");
}

#[test]
fn the_listing_states_the_cursor_trust_model() {
    let doc = std::fs::read_to_string(repo().join("docs/Listing.md")).expect("Listing.md");
    for line in [
        "## The Cursor package",
        "Provider-rendered approval UI is untrusted display, the\nMaidan console is the trust root, and an approval whose evidence the human did\nnot see in the console is provisional.",
        "`token:admin`",
        "`cursor-`",
    ] {
        assert!(doc.contains(line), "docs/Listing.md lacks {line:?}");
    }
}
