//! Discovery documents for the OAuth authorization server.
//!
//! - `GET /.well-known/oauth-authorization-server` — RFC 8414 metadata.
//! - `GET /.well-known/oauth-protected-resource` — RFC 9728 metadata for the
//!   MCP endpoint, so clients discover the AS from a 401.
//!
//! Phase one serves metadata only. The documents list no `token_endpoint`,
//! `authorization_endpoint`, or `registration_endpoint` because those flows
//! are not built yet; advertising them would lie to clients. Later phases
//! add the endpoints and extend these documents.

use axum::Json;
use serde::Serialize;

/// RFC 8414 authorization-server metadata.
///
/// Only the fields that are true today are present. `issuer` names this
/// server; `scopes_supported` is the capability vocabulary (capabilities
/// are the OAuth scopes — no parallel ACL). Endpoints land in later phases.
#[derive(Debug, Serialize)]
pub struct AuthorizationServerMetadata {
    issuer: String,
    scopes_supported: Vec<String>,
    /// PKCE S256 is required for public clients (OAuth 2.1).
    code_challenge_methods_supported: Vec<String>,
}

/// RFC 9728 protected-resource metadata for the MCP endpoint.
#[derive(Debug, Serialize)]
pub struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
    /// The bearer scheme the resource accepts.
    bearer_methods_supported: Vec<String>,
}

/// `GET /.well-known/oauth-authorization-server`
pub async fn oauth_authorization_server() -> Json<AuthorizationServerMetadata> {
    // The issuer is this server's public base URL. In phase one there is no
    // configured public URL, so the document is served relative to the
    // request host by the route layer; here we use the well-known path form
    // that clients resolve against the request URL.
    Json(AuthorizationServerMetadata {
        issuer: "https://maidan.dev".to_string(),
        scopes_supported: vec![
            // Capability names are the scope vocabulary (see docs/Protocols.md J6).
            "workspace:read".to_string(),
            "workspace:write".to_string(),
        ],
        code_challenge_methods_supported: vec!["S256".to_string()],
    })
}

/// `GET /.well-known/oauth-protected-resource`
pub async fn oauth_protected_resource() -> Json<ProtectedResourceMetadata> {
    Json(ProtectedResourceMetadata {
        resource: "https://maidan.dev/mcp".to_string(),
        authorization_servers: vec!["https://maidan.dev".to_string()],
        scopes_supported: vec!["workspace:read".to_string(), "workspace:write".to_string()],
        bearer_methods_supported: vec!["header".to_string()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_metadata_advertises_no_unserved_endpoint() {
        // Phase-one invariant: the document must not name an endpoint that
        // does not exist yet. Serialized JSON must contain no
        // "authorization_endpoint", "token_endpoint", "revocation_endpoint",
        // or "registration_endpoint" keys.
        let meta = AuthorizationServerMetadata {
            issuer: "https://example.test".to_string(),
            scopes_supported: vec!["workspace:read".to_string()],
            code_challenge_methods_supported: vec!["S256".to_string()],
        };
        let json = serde_json::to_string(&meta).expect("serializes");
        for key in [
            "authorization_endpoint",
            "token_endpoint",
            "revocation_endpoint",
            "registration_endpoint",
            "grant_types_supported",
        ] {
            assert!(
                !json.contains(key),
                "phase-one metadata must not advertise {key}"
            );
        }
    }

    #[test]
    fn protected_resource_metadata_points_at_authorization_server() {
        let meta = ProtectedResourceMetadata {
            resource: "https://example.test/mcp".to_string(),
            authorization_servers: vec!["https://example.test".to_string()],
            scopes_supported: vec!["workspace:read".to_string()],
            bearer_methods_supported: vec!["header".to_string()],
        };
        let json = serde_json::to_string(&meta).expect("serializes");
        assert!(json.contains("authorization_servers"));
        assert!(json.contains("https://example.test"));
    }
}
