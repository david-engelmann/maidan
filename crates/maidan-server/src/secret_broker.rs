//! The egress SecretBroker (Cluster 371.4, Wave 2 #19, G19/T3).
//!
//! On outbound delivery (a webhook POST), Maidan substitutes `secret://<name>`
//! references in the payload with the resolved value — **but only when the target
//! host is on an allowlist** (`MAIDAN_SECRET_EGRESS_ALLOWLIST`, comma-separated
//! hostnames). A ref bound for a non-allowlisted host is left as the literal
//! placeholder, so a secret is never leaked to an untrusted endpoint. The value
//! is resolved + decrypted here at send time and never persists in the delivery
//! queue or the event log.

use std::sync::OnceLock;

use maidan_auth::decrypt_peer_secret_rotating;
use maidan_types::{secret_refs_in, substitute_secret_refs, WorkspaceId};

use crate::state::AppState;

/// The parsed egress allowlist, cached from `MAIDAN_SECRET_EGRESS_ALLOWLIST` on
/// first use (comma/whitespace-separated hostnames). Empty ⇒ the broker never
/// substitutes (the safe default — no host is trusted with resolved secrets).
fn allowlist() -> &'static [String] {
    static ALLOWLIST: OnceLock<Vec<String>> = OnceLock::new();
    ALLOWLIST.get_or_init(|| {
        std::env::var("MAIDAN_SECRET_EGRESS_ALLOWLIST")
            .map(|raw| parse_allowlist(&raw))
            .unwrap_or_default()
    })
}

/// Parse a comma/whitespace-separated host allowlist. Pure.
pub fn parse_allowlist(raw: &str) -> Vec<String> {
    raw.split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// The host of a URL — everything after `://`, before the first `/?#`, minus any
/// `userinfo@` and `:port`. Pure; avoids a URL-parser dependency.
pub fn host_of(url: &str) -> Option<&str> {
    let after_scheme = url.split("://").nth(1)?;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .filter(|s| !s.is_empty())?;
    let host = authority.rsplit('@').next()?; // drop any userinfo@
    let host = host.split(':').next()?; // drop any :port
    (!host.is_empty()).then_some(host)
}

/// Whether `url`'s host is on `allowlist` (case-insensitive exact match). Pure.
pub fn host_allowed(url: &str, allowlist: &[String]) -> bool {
    match host_of(url) {
        Some(host) => allowlist.contains(&host.to_ascii_lowercase()),
        None => false,
    }
}

/// Substitute `secret://<name>` refs in `body` for an egress to `url`, if the
/// host is allowlisted. Returns `body` unchanged when there are no refs, the host
/// isn't allowlisted, or no encryption key is configured (the broker fails safe —
/// a ref is never blanked, only substituted or left literal). Resolves + decrypts
/// each referenced secret at send time; the plaintext never persists.
pub async fn substitute_for_egress(
    state: &AppState,
    workspace_id: WorkspaceId,
    url: &str,
    body: &str,
) -> String {
    substitute_with(state, workspace_id, url, body, allowlist()).await
}

/// The allowlist-parameterized core of [`substitute_for_egress`], so the resolve
/// path is testable without the process-global env cache.
pub async fn substitute_with(
    state: &AppState,
    workspace_id: WorkspaceId,
    url: &str,
    body: &str,
    allowlist: &[String],
) -> String {
    let names = secret_refs_in(body);
    if names.is_empty() || !host_allowed(url, allowlist) {
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
                    resolved.insert(name.clone(), value);
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
            host_of("https://api.example.com/hook?x=1"),
            Some("api.example.com")
        );
        assert_eq!(
            host_of("http://user:pw@host.internal:8443/x"),
            Some("host.internal")
        );
        assert_eq!(host_of("https://plain.host"), Some("plain.host"));
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn allowlist_is_exact_case_insensitive_host_match() {
        let allow = parse_allowlist("api.example.com");
        assert!(host_allowed("https://API.example.com/hook", &allow));
        assert!(!host_allowed("https://evil.example.com/hook", &allow));
        assert!(!host_allowed(
            "https://api.example.com.evil.com/hook",
            &allow
        ));
        assert!(!host_allowed("https://api.example.com/hook", &[]));
    }
}
