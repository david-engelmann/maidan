//! P1 client registry (`docs/OAuth.md`).
//!
//! Two ways a client identifies itself, and no others:
//! 1. **Pre-registered**: rows in `oauth_clients`, inserted by the operator
//!    (via `MAIDAN_OAUTH_CLIENTS` at boot, or directly). No dynamic
//!    registration: the protocol never creates clients.
//! 2. **Client ID Metadata Documents (CIMD)**: when `client_id` is an HTTPS
//!    URL, the server fetches the document — but only if the URL's host is
//!    in the instance's `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`. This is the
//!    fetch guard: a client cannot make the server fetch an arbitrary URL.
//!    It guards against server-side request forgery, which is the
//!    operator's concern, so it is an instance setting and no workspace
//!    configures it: a workspace fresh from sign-up resolves the same
//!    clients as any other.
//!
//! A resolved CIMD client is persisted as a row in `oauth_clients`,
//! marked with its metadata URL (migration 0172): its codes and grants
//! need the row for their foreign keys, and deleting the row still
//! cascades to them. The row is a cache, never an authority — a marked
//! row is never served as pre-registered. Every resolution re-fetches
//! the document through the fetch guard and refreshes the row from it,
//! so its redirect URIs and scopes are validated against the live
//! document the same way as a pre-registered client's are against the
//! operator's record.

use std::collections::BTreeSet;

use maidan_types::{NewOAuthClient, OAuthClient};
use serde::Deserialize;
use url::Url;

use crate::error::ApiError;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// The setting that names the hosts client metadata documents are fetched
/// from.
pub const CLIENT_METADATA_HOSTS_VAR: &str = "MAIDAN_OAUTH_CLIENT_METADATA_HOSTS";

/// The hosts a client metadata document may be fetched from
/// (`MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`): exact, lowercase host names,
/// parsed once at boot. Empty, the default, fetches no document.
#[derive(Debug, Clone, Default)]
pub struct ClientMetadataHosts {
    hosts: BTreeSet<String>,
    /// Tests only: the `scheme://authority` a document is fetched from in
    /// place of its client id's, after the host check. The host check always
    /// runs on the client id itself.
    fetch_origin_override: Option<String>,
}

impl ClientMetadataHosts {
    /// Parse a comma-separated list of exact host names. Blank entries are
    /// skipped. An entry with a scheme, port, path, wildcard or any character
    /// a DNS name cannot carry is an error, so a typo refuses boot instead of
    /// silently allowing nothing (or too much).
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut hosts = BTreeSet::new();
        for entry in raw.split(',') {
            let host = entry.trim().to_ascii_lowercase();
            if host.is_empty() {
                continue;
            }
            let labels_ok = host.split('.').all(|label| {
                !label.is_empty()
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });
            if !labels_ok || host.len() > 253 {
                return Err(format!(
                    "{CLIENT_METADATA_HOSTS_VAR}: {entry:?} is not an exact host name \
                     (no scheme, port, path or wildcard)"
                ));
            }
            hosts.insert(host);
        }
        Ok(Self {
            hosts,
            fetch_origin_override: None,
        })
    }

    /// Read [`CLIENT_METADATA_HOSTS_VAR`]. Unset is empty; a value that is
    /// not UTF-8 or does not parse refuses boot.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(CLIENT_METADATA_HOSTS_VAR) {
            Ok(raw) => Self::parse(&raw),
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(format!("{CLIENT_METADATA_HOSTS_VAR} is not valid UTF-8"))
            }
        }
    }

    /// Whether a document at `url` may be fetched: its host is listed
    /// exactly and it names no port other than the scheme's default.
    pub fn allows(&self, url: &Url) -> bool {
        url.port().is_none()
            && url
                .host_str()
                .is_some_and(|host| self.hosts.contains(&host.to_ascii_lowercase()))
    }

    /// The listed hosts, in order.
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        self.hosts.iter().map(String::as_str)
    }

    /// Tests only: fetch documents from `origin` (`scheme://authority`)
    /// instead of their client id's host, which is still checked.
    #[doc(hidden)]
    pub fn with_fetch_origin_override(mut self, origin: impl Into<String>) -> Self {
        self.fetch_origin_override = Some(origin.into());
        self
    }

    fn fetch_url(&self, url: &Url) -> String {
        match &self.fetch_origin_override {
            Some(origin) => {
                let mut target = format!("{}{}", origin.trim_end_matches('/'), url.path());
                if let Some(query) = url.query() {
                    target.push('?');
                    target.push_str(query);
                }
                target
            }
            None => url.to_string(),
        }
    }
}

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
/// 1. Pre-registered (DB) — exact match, not revoked, unmarked.
/// 2. CIMD: if `client_id` is an HTTPS URL on a host the instance lists in
///    `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`, fetch the document and validate it. A metadata-document client's row is
///    refreshed from the document, never served from the cache: a marked
///    row in the DB takes this path too.
pub async fn resolve_client(state: &AppState, client_id: &str) -> ApiResult<OAuthClient> {
    // Pre-registered first. A row marked with a metadata URL is not
    // pre-registered, however fresh: it falls through to the fetch.
    if let Some(client) = state.store.get_oauth_client_by_client_id(client_id).await? {
        if client.revoked_at.is_some() {
            return Err(ApiError::BadRequest("client is revoked".into()));
        }
        if client.metadata_url.is_none() {
            return Ok(client);
        }
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

    // Fetch guard: the host must be one the instance lists, checked before
    // anything is fetched.
    let hosts = &state.oauth_client_metadata_hosts;
    if !hosts.allows(&url) {
        return Err(ApiError::Forbidden(format!(
            "client metadata host {host} is not in {CLIENT_METADATA_HOSTS_VAR}"
        )));
    }

    // Fetch the document. No redirects: a redirect would lead the fetch to
    // a host the guard above never checked, so any non-2xx is a refusal.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ApiError::Internal(format!("building HTTP client: {e}")))?;
    let response = client
        .get(hosts.fetch_url(&url))
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("fetching client metadata: {e}")))?;
    if !response.status().is_success() {
        return Err(ApiError::BadRequest(format!(
            "client metadata document answered {}; redirects are not followed",
            response.status()
        )));
    }
    let doc: ClientMetadataDocument = response
        .json()
        .await
        .map_err(|e| ApiError::BadRequest(format!("invalid client metadata document: {e}")))?;

    if doc.redirect_uris.is_empty() {
        return Err(ApiError::BadRequest(
            "client metadata document has no redirect_uris".into(),
        ));
    }
    validate_redirect_uris(&doc.redirect_uris)?;

    // Persist the document's client as a marked row (or refresh the live
    // one): the codes and grants this client goes on to mint reference
    // the row through their foreign keys. `None` means the row that
    // carries this client_id is revoked or pre-registered — neither may
    // be replaced by a fetched document.
    let client = state
        .store
        .upsert_oauth_metadata_client(
            NewOAuthClient {
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
            },
            client_id,
        )
        .await?
        .ok_or_else(|| ApiError::BadRequest("client is revoked".into()))?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn empty_allows_nothing() {
        let hosts = ClientMetadataHosts::parse("").unwrap();
        assert!(!hosts.allows(&url("https://claude.ai/oauth.json")));
        assert!(ClientMetadataHosts::default().hosts().next().is_none());
    }

    #[test]
    fn hosts_match_exactly_and_ignore_case() {
        let hosts = ClientMetadataHosts::parse(" Claude.AI , ,client.example").unwrap();
        assert_eq!(
            hosts.hosts().collect::<Vec<_>>(),
            ["claude.ai", "client.example"]
        );
        assert!(hosts.allows(&url("https://claude.ai/x")));
        assert!(hosts.allows(&url("https://CLAUDE.ai:443/x")));
        assert!(!hosts.allows(&url("https://evil.claude.ai/x")));
        assert!(!hosts.allows(&url("https://claude.ai.evil/x")));
        assert!(!hosts.allows(&url("https://claude.ai:8443/x")));
    }

    #[test]
    fn anything_but_a_host_name_refuses_boot() {
        for bad in [
            "https://claude.ai",
            "claude.ai:443",
            "claude.ai/path",
            "*.claude.ai",
            "claude..ai",
            "-claude.ai",
            "user@claude.ai",
        ] {
            assert!(ClientMetadataHosts::parse(bad).is_err(), "{bad}");
        }
    }
}
