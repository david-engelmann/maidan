//! Catalog of every tool the MCP server exposes. The JSON-RPC client
//! receives this verbatim in the `tools/list` response.
//!
//! Every entry's `annotations` (a title and the four MCP behaviour hints) must
//! match its reviewed row in `tests/fixtures/tool-annotations.json`, which
//! gives each value its reason; `tool_annotations_contract` enforces it, so a
//! new tool needs both.

use serde_json::Value;

/// The catalog, checked in as `catalog.json` next to this file and parsed
/// once. `make regen` and `scripts/regen_derived.py` read the same file.
const CATALOG_JSON: &str = include_str!("catalog.json");

fn parsed() -> &'static [Value] {
    static CATALOG: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| match serde_json::from_str(CATALOG_JSON) {
        Ok(tools) => tools,
        Err(err) => {
            // `catalog_json_parses` fails the build's tests on this path.
            tracing::error!(%err, "tools/catalog.json is not a JSON array of tools");
            Vec::new()
        }
    })
}

pub fn catalog() -> Vec<Value> {
    parsed().to_vec()
}

/// The property names a tool's `inputSchema` declares.
///
/// The slash→MCP bridge uses this to inject only the context a tool actually
/// accepts. It used to inject `workspace_id`, `channel_id`, `thread_id` and
/// `author_id` into *every* call and rely on each tool silently discarding the
/// ones it does not declare — which stopped being true once the argument
/// structs began rejecting unknown fields, and was never a property worth
/// depending on.
///
/// Read from the catalog once: the MCP request log asks on every call.
pub fn declared_arguments(tool: &str) -> Option<&'static std::collections::HashSet<String>> {
    type Declared = std::collections::HashMap<String, std::collections::HashSet<String>>;
    static DECLARED: std::sync::LazyLock<Declared> = std::sync::LazyLock::new(|| {
        catalog()
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                let args = t
                    .get("inputSchema")
                    .and_then(|s| s.get("properties"))
                    .and_then(|p| p.as_object())
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                Some((name, args))
            })
            .collect()
    });
    DECLARED.get(tool)
}

#[cfg(test)]
mod portability_tests {
    use super::*;

    #[test]
    fn catalog_json_parses() {
        let tools: Vec<Value> = serde_json::from_str(CATALOG_JSON).expect("catalog.json parses");
        assert!(!tools.is_empty());
        assert_eq!(catalog().len(), tools.len());
    }

    fn array_types(path: &str, schema: &Value, found: &mut Vec<String>) {
        match schema {
            Value::Object(map) => {
                if map.get("type").is_some_and(Value::is_array) {
                    found.push(path.to_owned());
                }
                for (key, value) in map {
                    array_types(&format!("{path}.{key}"), value, found);
                }
            }
            Value::Array(items) => {
                for (i, value) in items.iter().enumerate() {
                    array_types(&format!("{path}[{i}]"), value, found);
                }
            }
            _ => {}
        }
    }

    /// `"type": ["integer", "null"]` is legal JSON Schema, but several MCP
    /// clients read `type` as a single string and reject the tool or drop the
    /// constraint — for a budget cap, silently. Nullable fields use `anyOf`.
    #[test]
    fn no_tool_schema_uses_an_array_type() {
        let mut found = Vec::new();
        for tool in catalog() {
            let name = tool["name"].as_str().unwrap_or("?").to_owned();
            array_types(&name, &tool["inputSchema"], &mut found);
        }
        assert!(found.is_empty(), "array-valued `type` at: {found:?}");
    }

    /// `body` is required only when `content` is absent, and an edit's content
    /// blocks name the same types a post does.
    #[test]
    fn message_tool_schemas_match_their_prose() {
        let catalog = catalog();
        let tool = |name: &str| {
            catalog
                .iter()
                .find(|entry| entry["name"] == name)
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        let post = tool("post_message");
        let edit = tool("edit_message");
        let dm = tool("post_dm_message");
        for (name, schema) in [
            ("post_message", post),
            ("edit_message", edit),
            ("post_dm_message", dm),
        ] {
            let required = schema["inputSchema"]["required"].as_array().unwrap();
            assert!(
                !required.iter().any(|value| value == "body"),
                "{name} still requires body unconditionally"
            );
            let any_of = schema["inputSchema"]["anyOf"]
                .as_array()
                .unwrap_or_else(|| panic!("{name} has no anyOf"));
            let needs = |key: &str| {
                any_of.iter().any(|branch| {
                    branch["required"]
                        .as_array()
                        .is_some_and(|req| req.iter().any(|value| value == key))
                })
            };
            assert!(
                needs("body") && needs("content"),
                "{name} must require body or content"
            );
            let description = schema["inputSchema"]["properties"]["body"]["description"]
                .as_str()
                .unwrap();
            assert!(
                description.contains("omit when sending typed content"),
                "{name} body prose drifted: {description}"
            );
        }
        let block_enum =
            serde_json::json!(["text", "code", "tool_use", "tool_result", "resource_link"]);
        assert_eq!(
            post["inputSchema"]["properties"]["content"]["items"]["properties"]["type"]["enum"],
            block_enum
        );
        assert_eq!(
            edit["inputSchema"]["properties"]["content"]["items"]["properties"]["type"]["enum"],
            block_enum
        );
        assert_eq!(
            dm["inputSchema"]["properties"]["content"]["items"]["properties"]["type"]["enum"],
            block_enum
        );
        // cast_vote.kind is a closed enum. The server rejects every other
        // string; the handler test is mcp_write_tools_emit_domain_events and
        // the HTTP test is vote_confidence_round_trips_and_validates.
        let kind = &tool("cast_vote")["inputSchema"]["properties"]["kind"];
        assert_eq!(kind["type"], "string");
        let mut values: Vec<&str> = kind["enum"]
            .as_array()
            .expect("cast_vote.kind must be a closed enum")
            .iter()
            .map(|value| value.as_str().expect("non-string enum value"))
            .collect();
        values.sort_unstable();
        assert_eq!(values, ["ack", "approve", "request_changes"]);
        let description = kind["description"].as_str().unwrap_or("");
        assert!(
            description.contains("rejected"),
            "cast_vote.kind must say an unknown kind is rejected: {description}"
        );
    }
}
