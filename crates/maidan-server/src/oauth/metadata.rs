//! Phase one of the OAuth authorization server (`docs/OAuth.md`): the MCP
//! endpoint describes itself as an OAuth protected resource (RFC 9728), and a
//! 401 from an MCP route says where that description is.
//!
//! Both are off until the operator names the instance's public origin in
//! `MAIDAN_PUBLIC_ORIGIN`. An identifier taken from the request's `Host` would
//! let a forged header make this server name another origin, so the origin is
//! configured, never derived.
//!
//! Phase three adds the authorization-server document (RFC 8414) at
//! `/.well-known/oauth-authorization-server`, and the resource document gains
//! `authorization_servers` naming it. The document lists only what the server
//! does today: the authorization-code grant, PKCE with S256, and the authorize
//! and token endpoints. Refresh tokens are not listed; they arrive with the
//! flow that mints them, so a client reading the document never plans around
//! a grant the token endpoint would refuse.

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

/// Where the authorization server's metadata is served, per RFC 8414
/// section 3: the well-known suffix appended to the issuer identifier, which
/// here is the origin itself, so the path is the suffix alone.
pub const AUTHORIZATION_SERVER_METADATA_PATH: &str = "/.well-known/oauth-authorization-server";

/// The authorization endpoint's path, on the issuer origin.
pub const AUTHORIZE_PATH: &str = "/oauth/authorize";

/// The token endpoint's path, on the issuer origin.
pub const TOKEN_PATH: &str = "/oauth/token";

/// Parse `MAIDAN_PUBLIC_ORIGIN`: `https://host[:port]`, or `http://` on a
/// loopback host for local work, with no path, query, fragment or
/// credentials. Unset or blank is `None`. Anything else refuses boot.
pub fn public_origin_from(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let url = url::Url::parse(raw).map_err(|e| {
        format!("MAIDAN_PUBLIC_ORIGIN must be an origin such as https://maidan.example.com, not {raw:?}: {e}")
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!(
            "MAIDAN_PUBLIC_ORIGIN must be a scheme and host with no path, query or credentials, not {raw:?}"
        ));
    }
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => {
            return Err(format!(
                "MAIDAN_PUBLIC_ORIGIN must use https (http only on a loopback host), not {raw:?}"
            ))
        }
    }
    Ok(Some(url.origin().ascii_serialization()))
}

/// RFC 9728 protected-resource metadata for the MCP endpoint.
///
/// No `scopes_supported`: a client treats it as the set it may request, and
/// the set a member can delegate is their own capabilities intersected with
/// the client's allowed scopes, so there is no server-wide set to name.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    bearer_methods_supported: Vec<String>,
    resource_name: String,
}

fn resource_document(origin: &str) -> ProtectedResourceMetadata {
    ProtectedResourceMetadata {
        resource: format!("{origin}{MCP_RESOURCE_PATH}"),
        authorization_servers: vec![origin.into()],
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
    Ok(Json(resource_document(origin)))
}

/// RFC 8414 authorization-server metadata.
///
/// Only the grant, response type and challenge method the server implements
/// are listed. `refresh_token` is absent until the token endpoint mints
/// refresh tokens, and there is no `registration_endpoint` because clients
/// are pre-registered or carry a client metadata document, never registered
/// dynamically.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    response_types_supported: Vec<String>,
    /// Named because an omitted list means `query` and `fragment`, and the
    /// authorization response is only ever a query string.
    response_modes_supported: Vec<String>,
    grant_types_supported: Vec<String>,
    /// Named because the RFC 8414 default is `client_secret_basic`, which
    /// the token endpoint does not read: a confidential client sends its
    /// secret in the form body, and a public client sends none.
    token_endpoint_auth_methods_supported: Vec<String>,
    code_challenge_methods_supported: Vec<String>,
}

fn authorization_server_document(origin: &str) -> AuthorizationServerMetadata {
    AuthorizationServerMetadata {
        issuer: origin.into(),
        authorization_endpoint: format!("{origin}{AUTHORIZE_PATH}"),
        token_endpoint: format!("{origin}{TOKEN_PATH}"),
        response_types_supported: vec!["code".into()],
        response_modes_supported: vec!["query".into()],
        grant_types_supported: vec!["authorization_code".into()],
        token_endpoint_auth_methods_supported: vec!["client_secret_post".into(), "none".into()],
        code_challenge_methods_supported: vec!["S256".into()],
    }
}

/// `GET /.well-known/oauth-authorization-server`. 404 until
/// `MAIDAN_PUBLIC_ORIGIN` is set, like the resource document: both are off
/// until the deployment names the origin its clients reach it at.
pub async fn oauth_authorization_server(
    State(state): State<AppState>,
) -> Result<Json<AuthorizationServerMetadata>, ApiError> {
    let origin = state.public_origin.as_deref().ok_or(ApiError::NotFound)?;
    Ok(Json(authorization_server_document(origin)))
}

/// The RFC 9207 `iss` parameter for an authorization response, as a query
/// fragment the response's redirect appends: `&iss=<issuer>` when the
/// deployment names its public origin, empty when it does not. The issuer is
/// the origin, the same value the authorization-server document carries, so
/// a client can match a response against the server it discovered. With no
/// origin configured there is no issuer identifier to assert and discovery
/// is off, so the parameter is omitted rather than invented.
pub fn issuer_param(origin: Option<&str>) -> String {
    match origin {
        Some(origin) => format!("&iss={}", urlencoding::encode(origin)),
        None => String::new(),
    }
}

/// The value of the challenge a 401 from an MCP route carries.
pub fn challenge_value(origin: &str) -> String {
    format!("Bearer resource_metadata=\"{origin}{RESOURCE_METADATA_PATH}\"")
}

/// Add the RFC 9728 challenge to a 401 from the MCP endpoint, so a client
/// learns where the resource's metadata is. Only the endpoint the metadata
/// names gets one: a client must reject metadata whose `resource` is not the
/// URL it asked, so pointing `/mcp` or another route at it would send that
/// client to a document it has to refuse. Other 401s, such as a webhook's
/// failed signature, are not about a bearer token and get none.
pub async fn challenge(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mcp = req.uri().path() == MCP_RESOURCE_PATH;
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
        assert_eq!(
            public_origin_from(Some("http://[::1]")),
            Ok(Some("http://[::1]".into()))
        );
        assert_eq!(
            public_origin_from(Some("http://[::1]:8080/")),
            Ok(Some("http://[::1]:8080".into()))
        );
        assert_eq!(
            public_origin_from(Some("https://maidan.example.com:443")),
            Ok(Some("https://maidan.example.com".into())),
            "the default port is not part of the origin"
        );
        for bad in [
            "maidan.example.com",
            "http://maidan.example.com",
            "https://maidan.example.com/mcp",
            "https://user@maidan.example.com",
            "https://maidan.example.com?x=1",
            "https://maidan.example.com#top",
            "https://maidan.example.com:abc",
            "https://maidan.example.com:99999",
            "http://[::2]",
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
    fn the_resource_document_names_the_mcp_endpoint_and_its_authorization_server() {
        let json =
            serde_json::to_value(resource_document("https://maidan.example.com")).expect("json");
        assert_eq!(
            json["resource"],
            "https://maidan.example.com/mcp/streamable"
        );
        assert_eq!(
            json["authorization_servers"],
            serde_json::json!(["https://maidan.example.com"]),
            "the issuer is the origin the discovery documents are served from"
        );
        assert!(
            json.get("scopes_supported").is_none(),
            "no scope set is advertised, since what a member can delegate is not a server-wide set"
        );
        assert_eq!(
            challenge_value("https://maidan.example.com"),
            "Bearer resource_metadata=\"https://maidan.example.com/.well-known/oauth-protected-resource/mcp/streamable\""
        );
    }

    #[test]
    fn the_authorization_server_document_lists_only_what_the_server_does() {
        let json =
            serde_json::to_value(authorization_server_document("https://maidan.example.com"))
                .expect("json");
        assert_eq!(json["issuer"], "https://maidan.example.com");
        assert_eq!(
            json["authorization_endpoint"],
            "https://maidan.example.com/oauth/authorize"
        );
        assert_eq!(
            json["token_endpoint"],
            "https://maidan.example.com/oauth/token"
        );
        assert_eq!(
            json["response_types_supported"],
            serde_json::json!(["code"])
        );
        assert_eq!(
            json["response_modes_supported"],
            serde_json::json!(["query"]),
            "the code and error arrive in the query string only"
        );
        assert_eq!(
            json["grant_types_supported"],
            serde_json::json!(["authorization_code"]),
            "refresh_token is listed only once the token endpoint mints refresh tokens"
        );
        assert_eq!(
            json["token_endpoint_auth_methods_supported"],
            serde_json::json!(["client_secret_post", "none"]),
            "the default, client_secret_basic, is not what the token endpoint reads"
        );
        assert_eq!(
            json["code_challenge_methods_supported"],
            serde_json::json!(["S256"])
        );
        assert!(
            json.get("registration_endpoint").is_none(),
            "there is no dynamic client registration to advertise"
        );
    }

    #[test]
    fn the_issuer_parameter_names_the_configured_origin_and_nothing_when_unset() {
        assert_eq!(
            issuer_param(Some("https://maidan.example.com")),
            "&iss=https%3A%2F%2Fmaidan.example.com"
        );
        assert_eq!(issuer_param(None), "");
    }
}
