//! P1 client registry (`docs/OAuth.md`).
//!
//! Two ways a client identifies itself, and no others:
//! 1. **Pre-registered**: rows in `oauth_clients`, inserted by the operator
//!    (via `MAIDAN_OAUTH_CLIENTS` at boot, or directly). No dynamic
//!    registration: the protocol never creates clients.
//! 2. **Client ID Metadata Documents (CIMD)**: when `client_id` is an HTTPS
//!    URL, the server fetches the document — but only if the URL's host is
//!    in the workspace's egress allowlist. This is the egress guard: a
//!    client cannot make the server fetch an arbitrary URL.
//!
//! A CIMD client is used ephemerally for the request; it is not persisted.
//! Its redirect URIs and scopes are validated the same way as a
//! pre-registered client's.

use maidan_types::{NewOAuthClient, OAuthClient, WorkspaceId};
use serde::Deserialize;
use url::Url;

use crate::error::ApiError;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// A pre-registered client from `MAIDAN_OAUTH_CLIENTS`.
#[derive(Debug, Deserialize)]
struct EnvClient {
    client_id: String,
    name: String,
    redirect_uris: Vec<String>,
    /// Hex-encoded SHA-256 of the secret, for confidential clients.
    client_secret_hash: Option<String>,
    allowed_scopes: Vec<String>,
}

/// Insert pre-registered clients from `MAIDAN_OAUTH_CLIENTS` (JSON array).
/// Idempotent: existing `client_id`s are skipped. Called at boot.
pub async fn load_preregistered_clients(state: &AppState) -> ApiResult<usize> {
    let raw = std::env::var("MAIDAN_OAUTH_CLIENTS").unwrap_or_default();
    if raw.trim().is_empty() {
        return Ok(0);
    }
    let clients: Vec<EnvClient> = serde_json::from_str(&raw)
        .map_err(|e| ApiError::Internal(format!("MAIDAN_OAUTH_CLIENTS is not valid JSON: {e}")))?;
    let mut inserted = 0;
    for c in clients {
        validate_redirect_uris(&c.redirect_uris).map_err(|e| {
            ApiError::Internal(format!(
                "MAIDAN_OAUTH_CLIENTS: client {}: {:?}",
                c.client_id, e
            ))
        })?;
        if state
            .store
            .get_oauth_client_by_client_id(&c.client_id)
            .await?
            .is_some()
        {
            continue;
        }
        state
            .store
            .create_oauth_client(NewOAuthClient {
                client_id: c.client_id,
                name: c.name,
                redirect_uris: c.redirect_uris,
                client_secret_hash: c.client_secret_hash,
                allowed_scopes: c.allowed_scopes,
            })
            .await?;
        inserted += 1;
    }
    Ok(inserted)
}

/// A client metadata document (RFC 7591 / CIMD subset).
#[derive(Debug, Deserialize)]
struct ClientMetadataDocument {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(default)]
    scope: Option<String>,
}

/// Every redirect URI must be HTTPS, or HTTP on loopback (development).
/// Rejects cleartext URIs that would leak the authorization code in transit.
fn validate_redirect_uris(uris: &[String]) -> ApiResult<()> {
    for uri in uris {
        let url = Url::parse(uri)
            .map_err(|_| ApiError::BadRequest(format!("redirect URI is not a valid URL: {uri}")))?;
        let is_loopback = url
            .host_str()
            .map(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]")
            .unwrap_or(false);
        match url.scheme() {
            "https" => {}
            "http" if is_loopback => {}
            other => {
                return Err(ApiError::BadRequest(format!(
                    "redirect URI scheme must be https (or http on loopback), got {other}: {uri}"
                )));
            }
        }
    }
    Ok(())
}

/// Resolve a `client_id` to a client.
/// 1. Pre-registered (DB) — exact match, not revoked.
/// 2. CIMD: if `client_id` is an HTTPS URL, fetch the document through the
///    egress guard and validate it.
pub async fn resolve_client(
    state: &AppState,
    workspace_id: WorkspaceId,
    client_id: &str,
) -> ApiResult<OAuthClient> {
    // Pre-registered first.
    if let Some(client) = state.store.get_oauth_client_by_client_id(client_id).await? {
        if client.revoked_at.is_none() {
            return Ok(client);
        }
        return Err(ApiError::BadRequest("client is revoked".into()));
    }

    // CIMD: client_id must be an HTTPS URL.
    let url =
        Url::parse(client_id).map_err(|_| ApiError::BadRequest("unknown client_id".into()))?;
    if url.scheme() != "https" {
        return Err(ApiError::BadRequest("unknown client_id".into()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::BadRequest("client_id URL has no host".into()))?;

    // Egress guard: the host must be in the workspace's allowlist.
    let targets = state.store.list_egress_targets(workspace_id).await?;
    let allowed = targets.iter().any(|t| {
        // The selector is a host or host:port. Match exactly.
        t.selector == host
            || t.selector == format!("{}:{}", host, url.port_or_known_default().unwrap_or(443))
    });
    if !allowed {
        return Err(ApiError::Forbidden(format!(
            "client metadata host {host} is not in the workspace egress allowlist"
        )));
    }

    // Fetch the document. No redirects: a redirect would bypass the egress
    // guard check above.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ApiError::Internal(format!("building HTTP client: {e}")))?;
    let doc: ClientMetadataDocument = client
        .get(url)
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("fetching client metadata: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::BadRequest(format!("invalid client metadata document: {e}")))?;

    if doc.redirect_uris.is_empty() {
        return Err(ApiError::BadRequest(
            "client metadata document has no redirect_uris".into(),
        ));
    }
    validate_redirect_uris(&doc.redirect_uris)?;

    // Ephemeral client: not persisted. The caller validates redirect URI
    // and scopes against these.
    Ok(OAuthClient {
        id: uuid::Uuid::now_v7(),
        client_id: client_id.to_string(),
        name: doc.client_name.unwrap_or_else(|| client_id.to_string()),
        redirect_uris: doc.redirect_uris,
        client_secret_hash: None, // CIMD clients are public (PKCE only)
        allowed_scopes: doc
            .scope
            .map(|s| {
                s.split(' ')
                    .filter(|c| !c.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        revoked_at: None,
    })
}
