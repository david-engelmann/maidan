//! Named secrets + secret-references.
//!
//! A workspace stores a named secret; the value is AEAD-encrypted at rest and
//! **never enters the event log**. Instead the log — a message, a webhook
//! payload, a tool argument — carries a `secret://<name>` *reference*, and the
//! value is resolved only at the moment it's needed: a consumer fetches it at
//! exec, or a `SecretBroker` substitutes it on egress to an allowlisted host.
//!
//! [`Secret`] is metadata only — it never carries the value. The ref-parsing
//! helpers here are pure so the broker's substitution logic is unit-tested
//! without a store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{MemberId, SecretId, WorkspaceId};

/// A stored secret's **metadata** — never its value. Listing secrets returns
/// these; the value is a separate, capability-gated resolve.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Secret {
    pub id: SecretId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A new secret to persist. `value_ciphertext` is already AEAD-encrypted by the
/// route layer (which holds the key) — the store never sees the plaintext.
#[derive(Debug, Clone)]
pub struct NewSecret {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub value_ciphertext: String,
    pub created_by: MemberId,
}

/// A host a workspace trusts to receive its secret values: on egress to this
/// host the broker substitutes `secret://<name>` refs from this workspace's
/// secrets. Listing it is granting that host every secret the payload names,
/// so the list is per workspace and starts empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretEgressHost {
    pub workspace_id: WorkspaceId,
    /// Lowercase hostname or IPv4 address, no scheme or port.
    pub host: String,
    pub created_at: DateTime<Utc>,
}

/// A host to add to a workspace's secret-egress allowlist. `host` is checked
/// and lowercased by [`normalize_secret_egress_host`] on every write path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSecretEgressHost {
    pub workspace_id: WorkspaceId,
    pub host: String,
}

/// The canonical form of an allowlisted host, or why it is not one. Pure.
///
/// The broker compares this with the host the egress URL parses to, which is
/// lowercase ASCII (a punycode `xn--` label for an internationalised name) with
/// no port. So the rule is a DNS name or IPv4 address in that form: a scheme,
/// port, path, `*` wildcard or trailing dot would never match, and refusing it
/// here says so instead of leaving an entry that silently does nothing.
pub fn normalize_secret_egress_host(raw: &str) -> Result<String, &'static str> {
    if raw.trim() != raw || raw.is_empty() {
        return Err("host must be non-empty and free of surrounding whitespace");
    }
    if raw.len() > 253 {
        return Err("host is longer than 253 characters");
    }
    if raw.contains("://") || raw.contains('/') {
        return Err("host is a hostname, not a URL: no scheme or path");
    }
    if raw.contains(':') {
        return Err("host takes no port, and IPv6 addresses are not supported");
    }
    let host = raw.to_ascii_lowercase();
    for label in host.split('.') {
        if label.is_empty() {
            return Err("host has an empty label (a leading, trailing or doubled dot)");
        }
        if label.len() > 63 {
            return Err("host has a label longer than 63 characters");
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err("host may use only letters, digits, '-' and '.' (punycode for non-ASCII names; no wildcards)");
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err("host has a label that starts or ends with '-'");
        }
    }
    Ok(host)
}

/// Whether `host` is inside the instance ceiling on secret-egress hosts
/// (`MAIDAN_SECRET_EGRESS_ALLOWLIST`). `None` is no ceiling; an empty list
/// admits nothing. Pure.
pub fn within_secret_egress_ceiling(host: &str, ceiling: Option<&[String]>) -> bool {
    ceiling.is_none_or(|hosts| hosts.iter().any(|h| h == host))
}

/// The `secret://` scheme that marks a reference in otherwise-plain text.
pub const SECRET_REF_SCHEME: &str = "secret://";

/// Whether `name` is a valid secret name (the chars allowed after `secret://`):
/// letters, digits, `_`, `-`, `.`. Keeping the set tight means a ref ends at the
/// first other character, so `secret://key,` or `secret://key"` parse cleanly.
pub fn is_valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Every distinct secret name referenced (`secret://<name>`) in `text`, in first
/// appearance order. Pure — the broker uses it to decide which secrets a payload
/// needs before deciding whether to resolve them.
pub fn secret_refs_in(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(SECRET_REF_SCHEME) {
        let after = &rest[pos + SECRET_REF_SCHEME.len()..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
            .unwrap_or(after.len());
        let name = &after[..end];
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        rest = &after[end..];
    }
    names
}

/// Substitute each `secret://<name>` in `text` with `resolve(name)`. A name the
/// resolver returns `None` for is **left as the literal ref** (never blanked or
/// leaked) and reported in the returned `unresolved` list, so a caller can refuse
/// to send a payload that still carries an unresolved secret. Pure.
pub fn substitute_secret_refs<F>(text: &str, mut resolve: F) -> (String, Vec<String>)
where
    F: FnMut(&str) -> Option<String>,
{
    let mut out = String::with_capacity(text.len());
    let mut unresolved = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(SECRET_REF_SCHEME) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + SECRET_REF_SCHEME.len()..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
            .unwrap_or(after.len());
        let name = &after[..end];
        if name.is_empty() {
            // A bare `secret://` with no name — emit it verbatim and move on.
            out.push_str(SECRET_REF_SCHEME);
        } else if let Some(value) = resolve(name) {
            out.push_str(&value);
        } else {
            out.push_str(SECRET_REF_SCHEME);
            out.push_str(name);
            if !unresolved.iter().any(|n| n == name) {
                unresolved.push(name.to_string());
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    (out, unresolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_distinct_refs_in_order() {
        let text =
            "Authorization: Bearer secret://api-key and secret://api-key plus secret://db.pass";
        assert_eq!(secret_refs_in(text), vec!["api-key", "db.pass"]);
    }

    #[test]
    fn finds_none_when_absent() {
        assert!(secret_refs_in("nothing here, just http://example.com").is_empty());
    }

    #[test]
    fn substitutes_known_and_leaves_unknown_literal() {
        let (out, unresolved) =
            substitute_secret_refs("key=secret://known; other=secret://missing", |name| {
                (name == "known").then(|| "VALUE".to_string())
            });
        assert_eq!(out, "key=VALUE; other=secret://missing");
        assert_eq!(unresolved, vec!["missing"]);
    }

    #[test]
    fn a_ref_ends_at_a_delimiter() {
        // The comma is not a valid name char, so the ref is `secret://k` only.
        let (out, _) = substitute_secret_refs("v=secret://k,next", |_| Some("X".into()));
        assert_eq!(out, "v=X,next");
    }

    #[test]
    fn a_bare_scheme_is_passed_through() {
        let (out, unresolved) = substitute_secret_refs("just secret:// here", |_| Some("X".into()));
        assert_eq!(out, "just secret:// here");
        assert!(unresolved.is_empty());
    }

    #[test]
    fn an_egress_host_is_a_lowercase_name_without_scheme_port_or_wildcard() {
        assert_eq!(
            normalize_secret_egress_host("Hooks.Example.com"),
            Ok("hooks.example.com".to_string())
        );
        assert_eq!(
            normalize_secret_egress_host("203.0.113.9"),
            Ok("203.0.113.9".to_string())
        );
        for bad in [
            "",
            " hooks.example.com",
            "https://hooks.example.com",
            "hooks.example.com/x",
            "hooks.example.com:443",
            "*.example.com",
            "hooks.example.com.",
            "hooks..example.com",
            "-hooks.example.com",
            "bücher.example",
            "[::1]",
        ] {
            assert!(
                normalize_secret_egress_host(bad).is_err(),
                "{bad:?} accepted"
            );
        }
    }

    #[test]
    fn name_validation() {
        assert!(is_valid_secret_name("prod-api.key_1"));
        assert!(!is_valid_secret_name(""));
        assert!(!is_valid_secret_name("has space"));
        assert!(!is_valid_secret_name("has/slash"));
    }
}
