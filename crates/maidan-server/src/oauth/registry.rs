//! P1 client registry (`docs/OAuth.md`).
//!
//! Three ways a client identifies itself, and no others:
//! 1. **Pre-registered**: rows in `oauth_clients`, inserted by the operator
//!    (via `MAIDAN_OAUTH_CLIENTS` at boot, or directly).
//! 2. **Client ID Metadata Documents (CIMD)**: when `client_id` is an HTTPS
//!    URL, the server fetches the document — but only if the URL's host is
//!    in the instance's `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`. This is the
//!    fetch guard: a client cannot make the server fetch an arbitrary URL.
//!    It guards against server-side request forgery, which is the
//!    operator's concern, so it is an instance setting and no workspace
//!    configures it: a workspace fresh from sign-up resolves the same
//!    clients as any other.
//! 3. **Dynamically registered** (RFC 7591, `oauth::register`): rows the
//!    client created itself through `POST /oauth/register`, marked with
//!    `registered_at`, only while `MAIDAN_OAUTH_DYNAMIC_REGISTRATION` is on.
//!    A marked row is refused once the instance turns registration off.
//!
//! A resolved CIMD client is persisted as a row in `oauth_clients`,
//! marked with its metadata URL (migration 0172): its codes and grants
//! need the row for their foreign keys, and deleting the row still
//! cascades to them. The row is never an authority — a marked row is
//! never served as pre-registered. Every resolution refreshes the row
//! from a document validated through the fetch guard within the last
//! five minutes: the one this process holds in
//! [`super::document_cache`], or a fresh fetch. So a client's redirect
//! URIs and scopes are checked against what its document said at most
//! five minutes ago, the way a pre-registered client's are against the
//! operator's record.

use std::collections::BTreeSet;

use maidan_types::{NewOAuthClient, OAuthClient};
use serde::Deserialize;
use url::Url;

use super::document_cache::ValidatedDocument;
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
        Ok(Self {
            hosts: parse_host_list(CLIENT_METADATA_HOSTS_VAR, raw)?,
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

/// Parse a comma-separated list of exact, lowercase host names for the
/// setting `var`. Blank entries are skipped. An entry with a scheme, port,
/// path, wildcard or any character a DNS name cannot carry is an error, so a
/// typo refuses boot instead of silently allowing nothing (or too much).
pub(crate) fn parse_host_list(var: &str, raw: &str) -> Result<BTreeSet<String>, String> {
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
                "{var}: {entry:?} is not an exact host name \
                 (no scheme, port, path or wildcard)"
            ));
        }
        hosts.insert(host);
    }
    Ok(hosts)
}

/// Whether `given`, the redirect URI an authorization request names, is one
/// of the client's `registered` redirect URIs. Exact string match, with one
/// exception from RFC 8252 section 7.3: for an `http` loopback redirect
/// (`127.0.0.1`, `[::1]` or `localhost`) the port may differ, because a
/// native app listens on whatever port the OS gives it. Scheme, host, path
/// and query must still match, and `localhost` never matches an IP literal.
pub fn redirect_uri_matches(registered: &[String], given: &str) -> bool {
    if registered.iter().any(|uri| uri == given) {
        return true;
    }
    let Some(given) = loopback_without_port(given) else {
        return false;
    };
    registered
        .iter()
        .filter_map(|uri| loopback_without_port(uri))
        .any(|uri| uri == given)
}

/// An `http` loopback URL with its port removed, as a string to compare;
/// `None` for anything else, or for a URL carrying credentials or a
/// fragment, which no redirect URI may.
fn loopback_without_port(uri: &str) -> Option<String> {
    let mut url = Url::parse(uri).ok()?;
    if url.scheme() != "http"
        || !is_loopback_host(&url)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    url.set_port(None).ok()?;
    Some(url.to_string())
}

/// `127.0.0.1`, `[::1]` or `localhost`, exactly: not the rest of `127/8`,
/// not a name that merely resolves to loopback.
pub(crate) fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Some(url::Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
        None => false,
    }
}

/// Whether the instance accepts this client now: a self-registered client
/// only while dynamic registration is on, so turning it off disconnects
/// every client that registered itself.
pub fn client_enabled(state: &AppState, client: &OAuthClient) -> bool {
    client.registered_at.is_none() || state.oauth_dynamic_registration.is_some()
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

/// The largest client metadata document read. The ones real clients serve
/// are under a kilobyte; past this a listed host could hold the server's
/// memory with one authorize request.
pub const MAX_METADATA_DOCUMENT_BYTES: usize = 16 * 1024;

/// A client metadata document (RFC 7591 / CIMD subset).
#[derive(Debug, Deserialize)]
struct ClientMetadataDocument {
    /// The document's own URL. A document whose `client_id` names any other
    /// is not this client's (draft-ietf-oauth-client-id-metadata-document
    /// section 4.1), so serving it would let one listed URL speak for a
    /// client at another.
    client_id: Option<String>,
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
///    `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`, take the document this process
///    validated in the last five minutes, or fetch and validate it. A
///    metadata-document client's row is refreshed from that document,
///    never served as it stands: a marked row in the DB takes this path too.
pub async fn resolve_client(state: &AppState, client_id: &str) -> ApiResult<OAuthClient> {
    // Pre-registered first. A row marked with a metadata URL is not
    // pre-registered, however fresh: it falls through to the fetch.
    if let Some(client) = state.store.get_oauth_client_by_client_id(client_id).await? {
        if client.revoked_at.is_some() {
            return Err(ApiError::BadRequest("client is revoked".into()));
        }
        if !client_enabled(state, &client) {
            return Err(ApiError::BadRequest("unknown client_id".into()));
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

    let doc = match state.oauth_document_cache.get(client_id) {
        Some(doc) => doc,
        None => {
            let doc = fetch_document(hosts, &url, client_id).await?;
            // Only now, with every check passed: a refusal is never held, so
            // the next request fetches again.
            state.oauth_document_cache.insert(client_id, doc.clone());
            doc
        }
    };

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
                name: doc.name,
                redirect_uris: doc.redirect_uris,
                client_secret_hash: None, // CIMD clients are public (PKCE only)
                allowed_scopes: doc.allowed_scopes,
            },
            client_id,
        )
        .await?
        .ok_or_else(|| ApiError::BadRequest("client is revoked".into()))?;
    Ok(client)
}

/// Fetch the document at `url` and hold it to every rule a client metadata
/// document must meet, after the host check has passed.
async fn fetch_document(
    hosts: &ClientMetadataHosts,
    url: &Url,
    client_id: &str,
) -> ApiResult<ValidatedDocument> {
    // Fetch the document. No redirects: a redirect would lead the fetch to
    // a host the guard above never checked, so any non-2xx is a refusal.
    let response = metadata_fetch_client(hosts, url)
        .await?
        .get(hosts.fetch_url(url))
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("fetching client metadata: {e}")))?;
    if !response.status().is_success() {
        return Err(ApiError::BadRequest(format!(
            "client metadata document answered {}; redirects are not followed",
            response.status()
        )));
    }
    let body = read_bounded(response, MAX_METADATA_DOCUMENT_BYTES).await?;
    let doc: ClientMetadataDocument = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("invalid client metadata document: {e}")))?;
    if doc.client_id.as_deref() != Some(client_id) {
        return Err(ApiError::BadRequest(
            "the client metadata document's client_id must be the URL it is served from".into(),
        ));
    }
    // Every listed host is trusted to say who its clients are, but the name
    // still reaches the consent page, so it is held to the rules a
    // self-registered client's is: no control or invisible formatting
    // characters that make one name render as another.
    let name = match doc.client_name.as_deref().map(str::trim) {
        None | Some("") => client_id.to_string(),
        Some(raw) => super::register::clean_client_name(Some(raw)).map_err(|e| {
            ApiError::BadRequest(format!("client metadata document: {}", e.description()))
        })?,
    };

    if doc.redirect_uris.is_empty() {
        return Err(ApiError::BadRequest(
            "client metadata document has no redirect_uris".into(),
        ));
    }
    validate_redirect_uris(&doc.redirect_uris)?;
    Ok(ValidatedDocument {
        name,
        allowed_scopes: document_scopes(doc.scope.as_deref()),
        redirect_uris: doc.redirect_uris,
    })
}

/// The scopes a metadata-document client may ask for. A document that names
/// none (claude.ai's connector document has no `scope`) may ask for any
/// delegatable capability; without this it could ask for nothing, and every
/// authorize from it was refused as "scope is empty". What it gets is still
/// cut to what the member holds and consents to.
fn document_scopes(scope: Option<&str>) -> Vec<String> {
    let named: Vec<String> = scope
        .unwrap_or("")
        .split(' ')
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();
    if named.is_empty() {
        maidan_auth::capability::delegatable()
    } else {
        named
    }
}

/// The HTTP client a client metadata document is fetched with: no redirects,
/// the egress timeouts, and a connection only to the public addresses the
/// host resolved to, checked before anything is sent and pinned so a second
/// lookup cannot answer differently. This is the egress guard's test
/// without its development escape hatch (`MAIDAN_ALLOW_PRIVATE_EGRESS`): a
/// document a listed host serves is public by definition, so no instance
/// needs to fetch one from a private network. Only a test that serves its
/// documents from loopback through the fetch origin override skips it.
async fn metadata_fetch_client(
    hosts: &ClientMetadataHosts,
    url: &Url,
) -> ApiResult<reqwest::Client> {
    let builder = crate::egress_http::bounded().redirect(reqwest::redirect::Policy::none());
    let build = |builder: reqwest::ClientBuilder| {
        builder
            .build()
            .map_err(|e| ApiError::Internal(format!("building HTTP client: {e}")))
    };
    if hosts.fetch_origin_override.is_some() {
        return build(builder);
    }
    let not_public = || {
        ApiError::Forbidden(
            "client metadata host does not resolve to a public network address".into(),
        )
    };
    let target = maidan_auth::parse_egress_target(url.as_str()).map_err(|_| not_public())?;
    let host = target.host_str().ok_or_else(not_public)?.to_string();
    let port = target.port_or_known_default().ok_or_else(not_public)?;
    let addresses: Vec<std::net::SocketAddr> = match tokio::time::timeout(
        maidan_auth::EGRESS_RESOLUTION_TIMEOUT,
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(found)) => found.collect(),
        Ok(Err(_)) | Err(_) => {
            return Err(ApiError::BadRequest(format!(
                "client metadata host {host} could not be resolved"
            )));
        }
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|addr| !maidan_auth::is_public_ip(addr.ip()))
    {
        return Err(not_public());
    }
    if matches!(target.host(), Some(url::Host::Domain(_))) {
        build(builder.resolve_to_addrs(&host, &addresses))
    } else {
        build(builder)
    }
}

/// Read a response body, refusing it once it passes `limit` bytes rather
/// than after it has all arrived.
async fn read_bounded(mut response: reqwest::Response, limit: usize) -> ApiResult<Vec<u8>> {
    let too_large = || {
        ApiError::BadRequest(format!(
            "client metadata document is larger than {limit} bytes"
        ))
    };
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ApiError::BadRequest(format!("reading client metadata document: {e}")))?
    {
        if body.len() + chunk.len() > limit {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn a_document_without_a_scope_may_ask_for_any_delegatable_capability() {
        let all = maidan_auth::capability::delegatable();
        assert_eq!(document_scopes(None), all);
        assert_eq!(document_scopes(Some("  ")), all);
        assert_eq!(
            document_scopes(Some("workspace:read  message:post")),
            vec!["workspace:read".to_string(), "message:post".to_string()]
        );
    }

    #[test]
    fn a_loopback_redirect_matches_on_any_port_and_nothing_else_does() {
        let registered = vec![
            "http://127.0.0.1/".to_string(),
            "http://localhost:8787/callback".to_string(),
            "http://[::1]/cb?x=1".to_string(),
            "https://vscode.dev/redirect".to_string(),
            "cursor://anysphere.cursor-mcp/oauth/callback".to_string(),
        ];
        for ok in [
            "http://127.0.0.1/",
            "http://127.0.0.1:33418/",
            "http://127.0.0.1:1",
            "http://localhost:8787/callback",
            "http://localhost:51000/callback",
            "http://[::1]:4000/cb?x=1",
            "https://vscode.dev/redirect",
            "cursor://anysphere.cursor-mcp/oauth/callback",
        ] {
            assert!(redirect_uri_matches(&registered, ok), "{ok}");
        }
        for bad in [
            // Another path, query or host on the same loopback.
            "http://127.0.0.1:33418/other",
            "http://localhost:8787/callback/",
            "http://localhost:8787/callback?x=1",
            "http://[::1]:4000/cb",
            // localhost never stands in for an IP literal, or the reverse.
            "http://localhost:33418/",
            "http://127.0.0.1:8787/callback",
            // The rest of 127/8 is not the registered host.
            "http://127.0.0.2:33418/",
            // https gets no port leeway, and no prefix games.
            "https://vscode.dev:8443/redirect",
            "https://vscode.dev/redirect/../evil",
            "https://vscode.dev/redirectx",
            "https://127.0.0.1/",
            // Credentials and fragments are never a match.
            "http://user@127.0.0.1:33418/",
            "http://127.0.0.1:33418/#frag",
            "cursor://anysphere.cursor-mcp/oauth/callback/x",
        ] {
            assert!(!redirect_uri_matches(&registered, bad), "{bad}");
        }
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
