//! The inline approval card: an MCP Apps View (SEP-1865, stable `2026-01-26`)
//! that renders an approval gate in the host's chat and answers it through
//! `approval_decide`.
//!
//! The card is one static `ui://` resource. It holds no gate data and no
//! secret: the host fetches it once, renders it in a sandboxed iframe, and
//! hands it the tool result (`structuredContent`) over the host bridge. Its
//! buttons go back through the bridge's `tools/call`, so every decision is
//! the same `approval_decide` call a model makes, under the same credential,
//! and meets the same rules: a click on Accept is never a confirmation, and
//! a confirmation-required answer only shows the person the console link.
//!
//! The HTML pins its own inline script and style by hash in a meta CSP, loads
//! nothing remote, and declares no CSP domains in `_meta.ui.csp`, so a host
//! that applies the spec default (`connect-src 'none'`, no external origins)
//! gives it nothing more.
//!
//! Editing the script or the style changes its hash: update the two
//! `'sha256-…'` sources in the HTML's meta CSP to the values
//! `the_card_pins_its_own_script_and_style_by_hash` prints, or the browser
//! refuses to run the card.

use serde_json::{json, Value};

use crate::call_context::{UiSupport, UI_MIME_TYPE};
use crate::error::McpError;

/// The card's resource URI. Exactly this; a query or fragment names nothing.
pub const APPROVAL_CARD_URI: &str = "ui://maidan/approval-card.html";

/// The tools whose results the card renders. Both stay visible to the model
/// and the app (`visibility` `["model", "app"]`, the spec default spelled
/// out): the model uses them as before, and the card calls them for Refresh,
/// Accept and Decline.
pub const LINKED_TOOLS: [&str; 2] = ["get_approval_gate", "approval_decide"];

/// The card itself, checked in as served.
pub const APPROVAL_CARD_HTML: &str = include_str!("apps_card/approval-card.html");

/// What `_meta.ui` says about the resource: no external origins of any kind,
/// and a border, since the card is a bordered panel in the host's layout.
fn resource_ui_meta() -> Value {
    json!({
        "csp": {
            "connectDomains": [],
            "resourceDomains": [],
            "frameDomains": [],
            "baseUriDomains": []
        },
        "prefersBorder": true
    })
}

/// The tool-side link (`_meta.ui`), for [`LINKED_TOOLS`].
pub fn tool_ui_meta() -> Value {
    json!({
        "resourceUri": APPROVAL_CARD_URI,
        "visibility": ["model", "app"]
    })
}

/// The card's `resources/list` entry, unless the client said it has no MCP
/// Apps.
pub fn listed(ui: UiSupport) -> Option<Value> {
    ui.offers_card().then(|| {
        json!({
            "uri": APPROVAL_CARD_URI,
            "name": "approval-card",
            "title": "Approval card",
            "description": "An MCP Apps View that shows an approval gate inline and answers it \
                            through approval_decide. Static: it holds no gate data.",
            "mimeType": UI_MIME_TYPE,
            "_meta": { "ui": resource_ui_meta() }
        })
    })
}

/// `resources/read` for a `ui://` URI: the card for exactly its URI, not
/// found for anything else.
pub fn read(uri: &str) -> Result<Value, McpError> {
    if uri != APPROVAL_CARD_URI {
        return Err(McpError::NotFound);
    }
    Ok(json!({
        "contents": [{
            "uri": APPROVAL_CARD_URI,
            "mimeType": UI_MIME_TYPE,
            "text": APPROVAL_CARD_HTML,
            "_meta": { "ui": resource_ui_meta() }
        }]
    }))
}

/// Whether `uri` is in the card's scheme, so `resources/read` routes it here
/// rather than to the store-backed `maidan://` resources.
pub fn is_ui_uri(uri: &str) -> bool {
    uri.starts_with("ui://")
}

/// Drop the card's link from a `tools/list` for a client that declared its
/// capabilities without MCP Apps, so it sees the catalog it saw before.
pub fn strip_tool_links(tools: &mut [Value]) {
    for tool in tools {
        let Some(object) = tool.as_object_mut() else {
            continue;
        };
        let emptied = match object.get_mut("_meta").and_then(Value::as_object_mut) {
            Some(meta) => {
                meta.remove("ui");
                meta.is_empty()
            }
            None => false,
        };
        if emptied {
            object.remove("_meta");
        }
    }
}

/// Add `structuredContent` to a tool result when the card is offered. The
/// text content is untouched, so a client that reads only text sees the
/// result it always did.
pub fn with_structured(mut result: Value, ui: UiSupport, structured: Value) -> Value {
    if ui.offers_card() {
        if let Some(object) = result.as_object_mut() {
            object.insert("structuredContent".into(), structured);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::{Digest, Sha256};

    fn between<'a>(html: &'a str, open: &str, close: &str) -> &'a str {
        let start = html.find(open).expect("open tag") + open.len();
        let end = html[start..].find(close).expect("close tag") + start;
        &html[start..end]
    }

    fn csp_hash(body: &str) -> String {
        let digest = Sha256::digest(body.as_bytes());
        format!(
            "'sha256-{}'",
            base64::engine::general_purpose::STANDARD.encode(digest)
        )
    }

    fn meta_csp() -> &'static str {
        let tag = between(
            APPROVAL_CARD_HTML,
            "<meta http-equiv=\"Content-Security-Policy\" content=\"",
            "\">",
        );
        tag
    }

    /// The meta CSP admits exactly the card's own script and style, by hash,
    /// and nothing else: no `'unsafe-inline'`, no origin, no network.
    #[test]
    fn the_card_pins_its_own_script_and_style_by_hash() {
        let csp = meta_csp();
        let script = between(APPROVAL_CARD_HTML, "<script>", "</script>");
        let style = between(APPROVAL_CARD_HTML, "<style>", "</style>");
        let script_src = format!("script-src {}", csp_hash(script));
        let style_src = format!("style-src {}", csp_hash(style));
        assert!(
            csp.contains(&script_src),
            "the meta CSP must pin the inline script: expected `{script_src}` in `{csp}`"
        );
        assert!(
            csp.contains(&style_src),
            "the meta CSP must pin the inline style: expected `{style_src}` in `{csp}`"
        );
        for directive in [
            "default-src 'none'",
            "connect-src 'none'",
            "frame-src 'none'",
            "object-src 'none'",
            "base-uri 'none'",
            "form-action 'none'",
            "img-src 'none'",
        ] {
            assert!(csp.contains(directive), "missing `{directive}` in `{csp}`");
        }
        assert!(!csp.contains("unsafe"), "no unsafe-* source: {csp}");
        assert_eq!(
            APPROVAL_CARD_HTML.matches("<script").count(),
            1,
            "one inline script"
        );
    }

    /// Nothing in the card reaches off the page: no URL, no external script
    /// or stylesheet, no network API, no HTML parsing of data.
    #[test]
    fn the_card_names_no_remote_url_and_parses_no_data_as_html() {
        let lower = APPROVAL_CARD_HTML.to_ascii_lowercase();
        for banned in [
            "http://",
            "https://",
            "//cdn",
            "src=",
            "href=",
            "@import",
            "url(",
            "fetch(",
            "xmlhttprequest",
            "websocket",
            "eventsource",
            "sendbeacon",
            "innerhtml",
            "outerhtml",
            "insertadjacenthtml",
            "document.write",
            "eval(",
            "new function",
            "localstorage",
            "sessionstorage",
            "document.cookie",
        ] {
            assert!(
                !lower.contains(banned),
                "the card must not contain `{banned}`"
            );
        }
    }

    /// The card never claims an outcome the server did not report: the only
    /// accepted wording is the server's `accepted` state label, and nothing
    /// says "approved".
    #[test]
    fn the_card_has_no_approved_wording() {
        assert!(!APPROVAL_CARD_HTML.to_ascii_lowercase().contains("approved"));
    }

    #[test]
    fn read_serves_exactly_the_card_uri() {
        let read = read(APPROVAL_CARD_URI).expect("card");
        let content = &read["contents"][0];
        assert_eq!(content["mimeType"], UI_MIME_TYPE);
        assert_eq!(content["uri"], APPROVAL_CARD_URI);
        assert_eq!(content["text"], APPROVAL_CARD_HTML);
        assert_eq!(content["_meta"]["ui"]["csp"]["connectDomains"], json!([]));
        for other in [
            "ui://maidan/approval-card.html?gate=1",
            "ui://maidan/approval-card.html#x",
            "ui://maidan/other.html",
            "ui://evil/approval-card.html",
        ] {
            assert!(
                matches!(super::read(other), Err(McpError::NotFound)),
                "{other}"
            );
        }
    }

    #[test]
    fn listing_and_structured_content_follow_ui_support() {
        assert!(listed(UiSupport::Declared).is_some());
        assert!(listed(UiSupport::Unknown).is_some());
        assert!(listed(UiSupport::NotDeclared).is_none());
        let base = json!({ "content": [], "isError": false });
        let with = with_structured(base.clone(), UiSupport::Unknown, json!({ "a": 1 }));
        assert_eq!(with["structuredContent"], json!({ "a": 1 }));
        let without = with_structured(base.clone(), UiSupport::NotDeclared, json!({ "a": 1 }));
        assert_eq!(without, base);
    }

    #[test]
    fn strip_tool_links_removes_only_the_ui_link() {
        let mut tools = vec![
            json!({ "name": "a", "_meta": { "ui": tool_ui_meta() } }),
            json!({ "name": "b", "_meta": { "ui": tool_ui_meta(), "other": 1 } }),
            json!({ "name": "c" }),
        ];
        strip_tool_links(&mut tools);
        assert_eq!(tools[0], json!({ "name": "a" }));
        assert_eq!(tools[1], json!({ "name": "b", "_meta": { "other": 1 } }));
        assert_eq!(tools[2], json!({ "name": "c" }));
    }
}
