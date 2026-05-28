//! Markdown reference generated from MCP catalogs (Track W.3).

use serde_json::Value;

use crate::{prompts, resources, tools};

/// Full MCP reference page for mdBook / docs site.
pub fn markdown() -> String {
    let mut out = String::from(
        "# MCP reference\n\n\
         Auto-generated from `maidan-mcp` `tools/list`, `resources/list`, and \
         `prompts/list` catalogs. Regenerate with \
         `cargo run -p maidan-mcp --bin gen-mcp-reference`.\n\n\
         ## Transport\n\n\
         - **HTTP:** `POST /mcp` (JSON-RPC 2.0, MCP 2024-11-05 subset)\n\
         - **HTTP notifications:** `GET /mcp/notifications` (SSE JSON-RPC notifications)\n\
         - **SSE:** `GET /mcp/stream` for workspace event stream replay/live\n\
         - **stdio:** `maidan mcp-stdio` for desktop clients (`resources/subscribe` notifications)\n\n\
         Bearer token required unless `AUTH_DISABLED=1`.\n\n",
    );

    out.push_str(
        "## JSON-RPC methods\n\n\
         - `initialize`\n\
         - `tools/list`, `tools/call`\n\
         - `resources/list`, `resources/read`, `resources/subscribe`, `resources/unsubscribe`\n\
         - `prompts/list`, `prompts/get`\n\n\
         **Notification:** `notifications/resources/updated` with `{ \"uri\": \"maidan://...\" }` \
         (stdio after each response; HTTP via `GET /mcp/notifications` SSE).\n\n",
    );

    out.push_str("## Tools\n\n");
    for tool in tools::catalog() {
        out.push_str(&render_tool(&tool));
        out.push('\n');
    }

    out.push_str("## Resources\n\n");
    for resource in resources::catalog() {
        out.push_str(&render_resource(&resource));
        out.push('\n');
    }

    out.push_str("## Prompts\n\n");
    for prompt in prompts::catalog() {
        out.push_str(&render_prompt(&prompt));
        out.push('\n');
    }

    out
}

fn render_tool(tool: &Value) -> String {
    let name = field_str(tool, "name");
    let description = field_str(tool, "description");
    let cap = tools::required_capability(&name)
        .map(|c| format!("`{c}`"))
        .unwrap_or_else(|_| "—".to_string());
    let schema = tool.get("inputSchema").map(pretty_json).unwrap_or_default();

    format!("### `{name}`\n\n{description}\n\n**Capability:** {cap}\n\n```json\n{schema}\n```\n")
}

fn render_resource(resource: &Value) -> String {
    let uri = field_str(resource, "uri");
    let name = field_str(resource, "name");
    let description = field_str(resource, "description");
    format!("### `{name}` — `{uri}`\n\n{description}\n")
}

fn render_prompt(prompt: &Value) -> String {
    let name = field_str(prompt, "name");
    let description = field_str(prompt, "description");
    let args = prompt
        .get("arguments")
        .map(pretty_json)
        .unwrap_or_else(|| "[]".to_string());
    format!("### `{name}`\n\n{description}\n\n**Arguments:**\n\n```json\n{args}\n```\n")
}

fn field_str(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_includes_every_catalog_entry() {
        let md = markdown();
        for tool in tools::catalog() {
            let name = tool["name"].as_str().expect("tool name");
            assert!(md.contains(name), "missing tool {name}");
        }
        for resource in resources::catalog() {
            let uri = resource["uri"].as_str().expect("resource uri");
            assert!(md.contains(uri), "missing resource {uri}");
        }
        for prompt in prompts::catalog() {
            let name = prompt["name"].as_str().expect("prompt name");
            assert!(md.contains(name), "missing prompt {name}");
        }
    }
}
