//! Phase one of the OAuth authorization server (`docs/OAuth.md`): the MCP
//! endpoint describes itself as an OAuth protected resource (RFC 9728), and a
//! 401 from an MCP route says where that description is.
//!
//! Both are off until the operator names the instance's public origin in
//! `MAIDAN_PUBLIC_ORIGIN`. An identifier taken from the request's `Host` would
//! let a forged header make this server name another origin, so the origin is
//! configured, never derived.
//!
//! The authorization-server document (RFC 8414) is not served yet. That RFC
//! requires `response_types_supported` and an authorization endpoint, and
//! phase one serves no flow, so a document now would either be invalid or
//! advertise endpoints that do not exist. It lands with the token endpoint, and
//! `authorization_servers` is added to the resource document then.

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

/// The MCP endpoint clients connect to, and so the protected resource's path.
pub const MCP_RESOURCE_PATH: &str = "/mcp/streamable";

/// Where the resource's metadata is served, per RFC 9728 section 3.1: the
/// well-known prefix inserted before the resource's path.
pub const RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp/streamable";

/// Parse `MAIDAN_PUBLIC_ORIGIN`: `https://host[:port]`, or `http://` on a
/// loopback host for local work, with no path, query or fragment. Unset or
/// blank is `None`. Anything else refuses boot.
pub fn public_origin_from(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let origin = raw.trim_end_matches('/');
    let (scheme, rest) = origin
        .split_once("://")
        .ok_or_else(|| format!("MAIDAN_PUBLIC_ORIGIN must be an origin such as https://maidan.example.com, not {raw:?}"))?;
    if rest.is_empty() || rest.contains(['/', '?', '#', '@']) {
        return Err(format!(
            "MAIDAN_PUBLIC_ORIGIN must be a scheme and host with no path, query or credentials, not {raw:?}"
        ));
    }
    let host = rest.rsplit_once(':').map_or(rest, |(h, _)| h);
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    match scheme {
        "https" => Ok(Some(origin.to_string())),
        "http" if loopback => Ok(Some(origin.to_string())),
        _ => Err(format!(
            "MAIDAN_PUBLIC_ORIGIN must use https (http only on a loopback host), not {raw:?}"
        )),
    }
}

/// RFC 9728 protected-resource metadata for the MCP endpoint.
#[derive(Debug, Serialize)]
pub struct ProtectedResourceMetadata {
    resource: String,
    scopes_supported: Vec<String>,
    bearer_methods_supported: Vec<String>,
    resource_name: String,
}

fn document(origin: &str) -> ProtectedResourceMetadata {
    ProtectedResourceMetadata {
        resource: format!("{origin}{MCP_RESOURCE_PATH}"),
        // Capability names are the scope vocabulary (docs/Protocols.md).
        scopes_supported: vec!["workspace:read".into(), "workspace:write".into()],
        bearer_methods_supported: vec!["header".into()],
        resource_name: "Maidan".into(),
    }
}

/// `GET /.well-known/oauth-protected-resource[/mcp/streamable]`. 404 until
/// `MAIDAN_PUBLIC_ORIGIN` is set.
pub async fn oauth_protected_resource(
    State(state): State<AppState>,
) -> Result<Json<ProtectedResourceMetadata>, ApiError> {
    let origin = state.public_origin.as_deref().ok_or(ApiError::NotFound)?;
    Ok(Json(document(origin)))
}

/// The value of the challenge a 401 from an MCP route carries.
pub fn challenge_value(origin: &str) -> String {
    format!("Bearer resource_metadata=\"{origin}{RESOURCE_METADATA_PATH}\"")
}

/// Add the RFC 9728 challenge to a 401 from an MCP route, so a client learns
/// where the resource's metadata is. Other 401s, such as a webhook's failed
/// signature, are not about a bearer token and get none.
pub async fn challenge(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mcp = {
        let path = req.uri().path();
        path == "/mcp" || path.starts_with("/mcp/")
    };
    let mut response = next.run(req).await;
    if mcp && response.status() == StatusCode::UNAUTHORIZED {
        if let Some(origin) = state.public_origin.as_deref() {
            if let Ok(value) = HeaderValue::from_str(&challenge_value(origin)) {
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, value);
            }
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_public_origin_is_an_https_scheme_and_host_or_loopback_http() {
        assert_eq!(public_origin_from(None), Ok(None));
        assert_eq!(public_origin_from(Some("  ")), Ok(None));
        assert_eq!(
            public_origin_from(Some("https://maidan.example.com/")),
            Ok(Some("https://maidan.example.com".into()))
        );
        assert_eq!(
            public_origin_from(Some("http://127.0.0.1:8080")),
            Ok(Some("http://127.0.0.1:8080".into()))
        );
        for bad in [
            "maidan.example.com",
            "http://maidan.example.com",
            "https://maidan.example.com/mcp",
            "https://user@maidan.example.com",
            "ftp://maidan.example.com",
            "https://",
        ] {
            assert!(
                public_origin_from(Some(bad)).is_err(),
                "{bad} should refuse boot"
            );
        }
    }

    #[test]
    fn the_resource_document_names_the_mcp_endpoint_and_no_authorization_server_yet() {
        let json = serde_json::to_value(document("https://maidan.example.com")).expect("json");
        assert_eq!(
            json["resource"],
            "https://maidan.example.com/mcp/streamable"
        );
        assert!(
            json.get("authorization_servers").is_none(),
            "phase one names no authorization server, since none serves a flow yet"
        );
        assert_eq!(
            challenge_value("https://maidan.example.com"),
            "Bearer resource_metadata=\"https://maidan.example.com/.well-known/oauth-protected-resource/mcp/streamable\""
        );
    }
}
