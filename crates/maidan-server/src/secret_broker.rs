//! The egress SecretBroker.
//!
//! On outbound delivery (a webhook POST, an automation HTTP call from a slash
//! command or FSM hook, an A2A push notification), Maidan substitutes
//! `secret://<name>` references in the payload with the sending workspace's
//! secret values, **but only when the target host is on that workspace's
//! secret-egress allowlist** (`maidan_secret_egress_hosts`, managed with
//! `secret:admin`). A ref bound for any other host is left as the literal
//! placeholder, so a secret is never sent to a host its workspace has not
//! trusted with it. The value is resolved and decrypted here at send time and
//! never persists in a delivery queue, the event log or an audit row.
//!
//! `MAIDAN_SECRET_EGRESS_ALLOWLIST`, when set, is an instance-wide ceiling: a
//! host outside it never receives a value, whatever a workspace lists, and a
//! workspace cannot add it. Unset, the workspace lists alone decide. Set to
//! the empty string, no host receives a value.
//!
//! Every payload the broker sees is a JSON document, and a ref's characters
//! need no escaping, so a ref only ever appears inside a JSON string. The value
//! is inserted JSON-escaped: a secret holding a quote or a newline (a PEM key)
//! leaves the document valid instead of breaking or rewriting it.

use maidan_auth::decrypt_peer_secret_rotating;
use maidan_types::{
    secret_refs_in, substitute_secret_refs, within_secret_egress_ceiling, WorkspaceId,
};

use crate::state::AppState;

/// Parse a comma/whitespace-separated host list (the instance ceiling). Pure.
pub fn parse_allowlist(raw: &str) -> Vec<String> {
    raw.split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// The lowercase host an egress URL would be sent to, when the egress guard
/// would send to it at all.
pub fn host_of(url: &str) -> Option<String> {
    maidan_auth::validate_egress_target(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// Whether `workspace_id` may send its secret values to `url`'s host: the
/// host is inside the instance ceiling and on the workspace's allowlist. Any
/// doubt (an unparseable URL, a failed lookup) is a no.
pub async fn may_receive_secrets(state: &AppState, workspace_id: WorkspaceId, url: &str) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    if !within_secret_egress_ceiling(&host, state.mcp.secret_egress_ceiling()) {
        return false;
    }
    match state
        .store
        .is_secret_egress_host_allowed(workspace_id, &host)
        .await
    {
        Ok(allowed) => allowed,
        Err(err) => {
            tracing::warn!(%workspace_id, %host, error = %err, "secret egress allowlist lookup failed; leaving refs literal");
            false
        }
    }
}

/// `value` escaped for the inside of a JSON string.
fn json_escaped(value: &str) -> Option<String> {
    let quoted = serde_json::to_string(value).ok()?;
    Some(quoted.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

/// Substitute `secret://<name>` refs in the JSON `body` of an egress to `url`,
/// from `workspace_id`'s secrets. Returns `body` unchanged when there are no
/// refs, the host may not receive this workspace's secrets, or no encryption
/// key is configured; a ref naming a secret the workspace does not hold stays
/// literal. The broker fails safe: a ref is substituted or left literal, never
/// blanked, and only ever from the sending workspace's own secrets.
pub async fn substitute_for_egress(
    state: &AppState,
    workspace_id: WorkspaceId,
    url: &str,
    body: &str,
) -> String {
    let names = secret_refs_in(body);
    if names.is_empty() || !may_receive_secrets(state, workspace_id, url).await {
        return body.to_string();
    }
    let Some(key) = state.federation.encryption_key.as_deref() else {
        tracing::warn!(
            %workspace_id,
            "egress secret refs present but no encryption key configured; leaving them literal"
        );
        return body.to_string();
    };

    // Pre-resolve each referenced secret (async) into a map the pure substitutor
    // can read synchronously.
    let mut resolved: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for name in &names {
        match state.store.get_secret_ciphertext(workspace_id, name).await {
            Ok(Some(ciphertext)) => match decrypt_peer_secret_rotating(&ciphertext, key) {
                Ok(value) => {
                    if let Some(escaped) = json_escaped(&value) {
                        resolved.insert(name.clone(), escaped);
                    }
                }
                Err(err) => {
                    tracing::warn!(secret = %name, error = %err, "egress secret decrypt failed")
                }
            },
            Ok(None) => { /* unknown secret — left literal by the substitutor */ }
            Err(err) => tracing::warn!(secret = %name, error = %err, "egress secret lookup failed"),
        }
    }

    let (out, unresolved) = substitute_secret_refs(body, |name| resolved.get(name).cloned());
    if !unresolved.is_empty() {
        tracing::warn!(
            %workspace_id,
            unresolved = unresolved.join(","),
            "egress left unresolved secret refs literal"
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_lowercases_the_allowlist() {
        assert_eq!(
            parse_allowlist("Example.com, api.internal\n hooks.slack.com"),
            vec!["example.com", "api.internal", "hooks.slack.com"]
        );
        assert!(parse_allowlist("  , ,").is_empty());
    }

    #[test]
    fn extracts_host_from_urls() {
        assert_eq!(
            host_of("https://API.example.com/hook?x=1"),
            Some("api.example.com".to_string())
        );
        assert_eq!(host_of("http://user:pw@host.internal:8443/x"), None);
        assert_eq!(
            host_of("https://plain.host"),
            Some("plain.host".to_string())
        );
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn the_ceiling_is_an_exact_host_match_and_unset_is_no_ceiling() {
        let ceiling = parse_allowlist("api.example.com");
        assert!(within_secret_egress_ceiling(
            "api.example.com",
            Some(&ceiling)
        ));
        assert!(!within_secret_egress_ceiling(
            "evil.example.com",
            Some(&ceiling)
        ));
        assert!(!within_secret_egress_ceiling(
            "api.example.com.evil.com",
            Some(&ceiling)
        ));
        assert!(!within_secret_egress_ceiling("api.example.com", Some(&[])));
        assert!(within_secret_egress_ceiling("api.example.com", None));
    }

    #[test]
    fn a_value_is_escaped_for_the_json_string_it_lands_in() {
        assert_eq!(
            json_escaped("line one\n\"quoted\"").as_deref(),
            Some("line one\\n\\\"quoted\\\"")
        );
    }
}
