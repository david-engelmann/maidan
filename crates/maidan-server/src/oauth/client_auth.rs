//! Client authentication at the token and revocation endpoints (RFC 6749
//! section 2.3): `client_secret_basic`, `client_secret_post`, or `none` for a
//! public client, whose only proof is PKCE at the token endpoint.
//!
//! Authentication runs in two steps because a client identified by a client
//! metadata document resolves only against a workspace's egress allowlist,
//! and the workspace is known only once the code or token is read. A
//! pre-registered client, which is what a secret can belong to, is checked
//! before anything else is read.

use axum::http::{header, HeaderMap};
use base64::{engine::general_purpose::STANDARD, Engine};
use maidan_auth::hash_secret;
use maidan_auth::token::hashes_equal;
use maidan_types::{OAuthClient, WorkspaceId};

use super::error::OAuthError;
use crate::state::AppState;

/// The method a request used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Basic,
    Post,
    None,
}

/// What the request claims: a client id and, for a confidential client, a
/// secret, read from exactly one place.
#[derive(Debug)]
pub struct ClientCredentials {
    pub client_id: String,
    secret: Option<String>,
    pub method: AuthMethod,
}

impl ClientCredentials {
    /// An error answering this request, with the Basic challenge when the
    /// client tried Basic.
    pub fn invalid_client(&self, description: &str) -> OAuthError {
        OAuthError::invalid_client(description)
            .with_basic_challenge(self.method == AuthMethod::Basic)
    }
}

/// Read the credentials from the `Authorization` header or the form.
/// Both at once is `invalid_request` (RFC 6749 section 2.3: one method per
/// request). A `client_id` in the form beside Basic must name the same
/// client.
pub fn credentials(
    headers: &HeaderMap,
    form_client_id: Option<&str>,
    form_secret: Option<&str>,
) -> Result<ClientCredentials, OAuthError> {
    let form_client_id = form_client_id.filter(|s| !s.is_empty());
    let form_secret = form_secret.filter(|s| !s.is_empty());
    let Some(authorization) = headers.get(header::AUTHORIZATION) else {
        let client_id = form_client_id
            .ok_or_else(|| OAuthError::invalid_client("no client authentication included"))?;
        return Ok(ClientCredentials {
            client_id: client_id.to_owned(),
            secret: form_secret.map(str::to_owned),
            method: if form_secret.is_some() {
                AuthMethod::Post
            } else {
                AuthMethod::None
            },
        });
    };
    let basic = || {
        OAuthError::invalid_client("the Authorization header is not HTTP Basic client credentials")
            .with_basic_challenge(true)
    };
    let value = authorization.to_str().map_err(|_| basic())?;
    let (scheme, encoded) = value.split_once(' ').ok_or_else(basic)?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return Err(basic());
    }
    let decoded = STANDARD.decode(encoded.trim()).map_err(|_| basic())?;
    let decoded = String::from_utf8(decoded).map_err(|_| basic())?;
    let (id, secret) = decoded.split_once(':').ok_or_else(basic)?;
    // RFC 6749 section 2.3.1: each half is form-urlencoded before encoding.
    let id = form_decode(id).ok_or_else(basic)?;
    let secret = form_decode(secret).ok_or_else(basic)?;
    if id.is_empty() {
        return Err(basic());
    }
    if form_secret.is_some() {
        return Err(OAuthError::invalid_request(
            "the client authenticated with both HTTP Basic and client_secret in the body",
        ));
    }
    if form_client_id.is_some_and(|form_id| form_id != id) {
        return Err(OAuthError::invalid_request(
            "client_id in the body names a different client than the Authorization header",
        ));
    }
    Ok(ClientCredentials {
        client_id: id,
        secret: Some(secret),
        method: AuthMethod::Basic,
    })
}

/// `application/x-www-form-urlencoded` decoding of one value: `+` is a space
/// and `%XX` an octet.
fn form_decode(raw: &str) -> Option<String> {
    let spaced = raw.replace('+', " ");
    urlencoding::decode(&spaced).ok().map(|s| s.into_owned())
}

/// A pre-registered client, authenticated; `None` when the id names no
/// pre-registered client (it may name a client metadata document).
///
/// A confidential client must send its secret, compared in constant time. A
/// public client must not send one: a secret it does not have proves
/// nothing, and accepting it would hide a misconfigured client.
pub async fn authenticate_preregistered(
    state: &AppState,
    creds: &ClientCredentials,
) -> Result<Option<OAuthClient>, OAuthError> {
    let Some(client) = state
        .store
        .get_oauth_client_by_client_id(&creds.client_id)
        .await?
    else {
        return Ok(None);
    };
    check_secret(&client, creds)?;
    Ok(Some(client))
}

fn check_secret(client: &OAuthClient, creds: &ClientCredentials) -> Result<(), OAuthError> {
    match (
        client.client_secret_hash.as_deref(),
        creds.secret.as_deref(),
    ) {
        (Some(expected), Some(provided)) => {
            if hashes_equal(expected, &hash_secret(provided)) {
                Ok(())
            } else {
                Err(creds.invalid_client("client authentication failed"))
            }
        }
        (Some(_), None) => Err(creds.invalid_client("this client must authenticate")),
        (None, Some(_)) => {
            Err(creds.invalid_client("this is a public client and has no secret to send"))
        }
        (None, None) => Ok(()),
    }
}

/// Whether a client that is not pre-registered can be one at all: only a
/// client identified by a client metadata document can, and that is an
/// `https` URL and a public client, so it sends no secret. Anything else is
/// an unknown client, refused before a code or token is read.
pub fn require_known_shape(creds: &ClientCredentials) -> Result<(), OAuthError> {
    let metadata_document = creds.method == AuthMethod::None
        && url::Url::parse(&creds.client_id).is_ok_and(|url| url.scheme() == "https");
    if metadata_document {
        Ok(())
    } else {
        Err(creds.invalid_client("unknown client"))
    }
}

/// Finish authentication once the workspace is known: a client that is not
/// pre-registered must resolve from its client metadata document through the
/// workspace's egress guard. Such a client is public, so it sends no secret.
pub async fn authenticate_in(
    state: &AppState,
    creds: &ClientCredentials,
    preregistered: Option<OAuthClient>,
    workspace_id: WorkspaceId,
) -> Result<OAuthClient, OAuthError> {
    if let Some(client) = preregistered {
        return Ok(client);
    }
    if creds.secret.is_some() {
        return Err(creds.invalid_client("unknown client"));
    }
    super::registry::resolve_client(state, workspace_id, &creds.client_id)
        .await
        .map_err(|_| creds.invalid_client("unknown client"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(id: &str, secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let value = format!("Basic {}", STANDARD.encode(format!("{id}:{secret}")));
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        headers
    }

    #[test]
    fn basic_credentials_are_form_decoded() {
        let creds = credentials(&basic("my%20client", "s%3Ac+r"), None, None).unwrap();
        assert_eq!(creds.client_id, "my client");
        assert_eq!(creds.secret.as_deref(), Some("s:c r"));
        assert_eq!(creds.method, AuthMethod::Basic);
    }

    #[test]
    fn two_methods_or_two_client_ids_are_invalid_requests() {
        let both = credentials(&basic("c", "s"), None, Some("s")).unwrap_err();
        assert_eq!(both.code.as_str(), "invalid_request");
        let other = credentials(&basic("c", "s"), Some("d"), None).unwrap_err();
        assert_eq!(other.code.as_str(), "invalid_request");
        assert!(credentials(&basic("c", "s"), Some("c"), None).is_ok());
    }

    #[test]
    fn a_malformed_or_foreign_authorization_header_is_an_invalid_client_with_a_challenge() {
        for value in ["Basic !!!", "Bearer abc", "Basic", "Basic Og=="] {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            let err = credentials(&headers, None, None).unwrap_err();
            assert_eq!(err.code.as_str(), "invalid_client", "{value}");
            assert!(err.basic_challenge, "{value}");
        }
    }

    #[test]
    fn the_form_names_a_post_or_public_client() {
        let headers = HeaderMap::new();
        let post = credentials(&headers, Some("c"), Some("s")).unwrap();
        assert_eq!(post.method, AuthMethod::Post);
        let public = credentials(&headers, Some("c"), None).unwrap();
        assert_eq!(public.method, AuthMethod::None);
        let none = credentials(&headers, None, None).unwrap_err();
        assert_eq!(none.code.as_str(), "invalid_client");
        assert!(!none.basic_challenge);
    }
}
