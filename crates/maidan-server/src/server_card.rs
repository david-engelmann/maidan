//! The MCP server card at `/.well-known/mcp/server-card.json`.
//!
//! A static discovery document for catalogs that index MCP servers without
//! connecting first — Smithery's URL path reads it before it holds any
//! credential, so the route is unauthenticated like the other well-known
//! documents. Everything in it is a projection of what the server already
//! serves: `serverInfo` mirrors the MCP initialize result's, and `tools` is
//! the unfiltered `tools/list` catalog itself, so the card cannot drift
//! from the endpoint it describes. The shared source is the contract; this
//! module keeps no second copy.

use axum::Json;
use serde_json::{json, Value};

/// The card document, built fresh per request from the live catalog. Pure,
/// so the shape is unit-testable without a server.
pub fn server_card() -> Value {
    json!({
        "serverInfo": {
            "name": "maidan",
            // The release this binary was built as (`MAIDAN_VERSION` at build
            // time), not the crate version, which is 0.0.0 in this workspace.
            "version": crate::version(),
        },
        "authentication": {
            "required": true,
            "schemes": ["bearer"],
        },
        "tools": maidan_mcp::tools::catalog(),
        "resources": [],
        "prompts": [],
    })
}

/// `GET /.well-known/mcp/server-card.json`.
pub async fn server_card_json() -> Json<Value> {
    Json(server_card())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_names_the_mcp_server_and_its_auth() {
        let card = server_card();
        assert_eq!(card["serverInfo"]["name"], "maidan");
        assert_eq!(
            card["serverInfo"]["version"],
            crate::version(),
            "the card carries the release the binary was built as"
        );
        assert_eq!(card["authentication"]["required"], true);
        assert_eq!(card["authentication"]["schemes"], json!(["bearer"]));
    }

    #[test]
    fn the_cards_tools_are_the_live_catalog() {
        let card = server_card();
        assert_eq!(
            card["tools"],
            Value::Array(maidan_mcp::tools::catalog()),
            "a card that restates the tools drifts; this one projects them"
        );
        assert!(
            !maidan_mcp::tools::catalog().is_empty(),
            "the catalog is never empty, so an empty card tools array is a bug"
        );
    }
}
