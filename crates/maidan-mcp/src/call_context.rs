//! What a `tools/call` says about its client, beside the tool's arguments.
//!
//! On `2026-07-28` every request carries the client's capabilities in
//! `params._meta`, and may carry its `clientInfo`; a retry after an
//! `input_required` result carries `inputResponses`. Earlier revisions said
//! this once, in `initialize`, which on Maidan's stateless transports is a
//! separate request with nothing tying it to the call, so for them this is
//! empty and nothing is guessed.

use serde_json::Value;

const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";

/// The one revision whose clients answer `input_required` (MRTR). An earlier
/// client that put capabilities in `_meta` still could not answer one.
const MRTR_REVISION: &str = "2026-07-28";

/// The longest client name or version kept. It is a label the client chose,
/// shown on a card and kept in the audit log, so it is bounded.
const MAX_CLIENT_FIELD: usize = 128;

/// The client as it named itself. A label for the record, not a credential:
/// any caller can send any name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    pub name: Option<String>,
    pub version: Option<String>,
}

/// See the module docs.
#[derive(Debug, Clone, Default)]
pub struct CallContext {
    pub client: ClientInfo,
    /// The client declared URL-mode elicitation on a revision that delivers
    /// it as an `input_required` result.
    pub url_elicitation: bool,
    /// This call is a retry answering an `input_required` result.
    pub answers_input: bool,
}

impl CallContext {
    pub fn from_params(params: &Value) -> Self {
        let meta = params.get("_meta").and_then(Value::as_object);
        let Some(meta) = meta else {
            return Self {
                answers_input: params.get("inputResponses").is_some(),
                ..Self::default()
            };
        };
        let mrtr = meta.get(META_PROTOCOL_VERSION).and_then(Value::as_str) == Some(MRTR_REVISION);
        // URL mode is declared by its own key; a bare `elicitation: {}` means
        // form mode only, which this server never asks for.
        let url_elicitation = mrtr
            && meta
                .get(META_CLIENT_CAPABILITIES)
                .and_then(|caps| caps.get("elicitation"))
                .and_then(|e| e.get("url"))
                .is_some_and(Value::is_object);
        let info = meta.get(META_CLIENT_INFO);
        Self {
            client: ClientInfo {
                name: info.and_then(|i| label(i.get("name"))),
                version: info.and_then(|i| label(i.get("version"))),
            },
            url_elicitation,
            answers_input: params.get("inputResponses").is_some(),
        }
    }
}

/// A client-chosen label, trimmed, without control characters, and bounded.
fn label(value: Option<&Value>) -> Option<String> {
    let cleaned: String = value?
        .as_str()?
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_CLIENT_FIELD)
        .collect();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_2026_request_declaring_url_mode_gets_url_elicitation_and_its_client() {
        let call = CallContext::from_params(&json!({
            "name": "approval_decide",
            "_meta": {
                META_PROTOCOL_VERSION: "2026-07-28",
                META_CLIENT_CAPABILITIES: { "elicitation": { "url": {} } },
                META_CLIENT_INFO: { "name": " Claude Code\n", "version": "2.1" }
            }
        }));
        assert!(call.url_elicitation);
        assert_eq!(call.client.name.as_deref(), Some("Claude Code"));
        assert_eq!(call.client.version.as_deref(), Some("2.1"));
    }

    #[test]
    fn form_only_elicitation_or_an_earlier_revision_is_not_url_elicitation() {
        for meta in [
            json!({ META_PROTOCOL_VERSION: "2026-07-28", META_CLIENT_CAPABILITIES: { "elicitation": {} } }),
            json!({ META_PROTOCOL_VERSION: "2026-07-28", META_CLIENT_CAPABILITIES: { "elicitation": { "form": {} } } }),
            json!({ META_PROTOCOL_VERSION: "2025-11-25", META_CLIENT_CAPABILITIES: { "elicitation": { "url": {} } } }),
            json!({ META_CLIENT_CAPABILITIES: { "elicitation": { "url": {} } } }),
        ] {
            assert!(!CallContext::from_params(&json!({ "_meta": meta })).url_elicitation);
        }
    }

    #[test]
    fn a_call_with_no_meta_names_no_client() {
        let call = CallContext::from_params(&json!({ "name": "x", "arguments": {} }));
        assert_eq!(call.client, ClientInfo::default());
        assert!(!call.url_elicitation && !call.answers_input);
    }

    #[test]
    fn a_long_client_name_is_bounded() {
        let long = "x".repeat(1000);
        let call =
            CallContext::from_params(&json!({ "_meta": { META_CLIENT_INFO: { "name": long } } }));
        assert_eq!(call.client.name.map(|n| n.len()), Some(MAX_CLIENT_FIELD));
    }
}
