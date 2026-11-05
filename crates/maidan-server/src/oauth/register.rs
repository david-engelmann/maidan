//! `POST /oauth/register`: RFC 7591 dynamic client registration
//! (`docs/OAuth.md`, decided 2026-11-05).
//!
//! An MCP client that has no pre-registration and no client metadata
//! document the instance fetches (Cursor, the MCP Inspector, Gemini CLI, the
//! SDK examples) registers itself here and then runs the ordinary
//! authorization-code flow with PKCE. Off unless the instance sets
//! `MAIDAN_OAUTH_DYNAMIC_REGISTRATION=1`, and then held to what a public,
//! multi-tenant service can afford to let anyone create:
//!
//! - **Public clients only.** `token_endpoint_auth_method` is `none` (or
//!   omitted, which this server answers as `none`); no secret is ever issued.
//!   PKCE S256 is required at authorize for every client already.
//! - **Redirects that land on the person's own machine.** `http` only on a
//!   loopback host (`127.0.0.1`, `[::1]`, `localhost`, any port, matched at
//!   authorize per RFC 8252 section 7.3), a private-use scheme in
//!   reverse-DNS form or a known app's (`cursor`, `vscode`), and `https` only
//!   on a host the operator lists in `MAIDAN_OAUTH_DCR_REDIRECT_HOSTS`. A
//!   phishing link built on a self-registered client therefore sends the
//!   code to the victim's own machine, never to the attacker's server.
//!   Script-bearing and local-file schemes refuse the registration outright;
//!   an `https` URI on an unlisted host is left out of it, so a client that
//!   also offers a loopback redirect (Cursor) still registers.
//! - **Bounded rows.** The name, the URI list and every URI are capped; the
//!   rate limiter charges registrations per address
//!   (`MAIDAN_OAUTH_REGISTER_RATE_LIMIT_*`); at most
//!   [`MAX_UNAUTHORIZED_CLIENTS`] registrations may wait on a first
//!   authorization at once; and [`sweep`] deletes a client never authorized
//!   within a day or unused for 90 days.
//! - **No authority of its own.** A registered client may ask for any
//!   delegatable capability (or fewer, if it names a scope), and what a token
//!   carries is still cut at authorize to what the consenting member holds,
//!   through the same grant path as every other client. Its self-declared
//!   name is shown on the consent page as unverified.

use std::collections::BTreeSet;
use std::time::Duration;

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use maidan_auth::capability;
use maidan_types::NewOAuthClient;
use serde::{Deserialize, Serialize};
use url::Url;
use utoipa::ToSchema;
use uuid::Uuid;

use super::error::OAuthErrorBody;
use super::registry::{is_loopback_host, parse_host_list};
use crate::error::ApiError;
use crate::state::AppState;

/// The setting that turns registration on: `1` or `true`.
pub const DYNAMIC_REGISTRATION_VAR: &str = "MAIDAN_OAUTH_DYNAMIC_REGISTRATION";

/// The hosts an `https` redirect URI of a self-registered client may name.
pub const DCR_REDIRECT_HOSTS_VAR: &str = "MAIDAN_OAUTH_DCR_REDIRECT_HOSTS";

/// The registration endpoint's path, on the issuer origin.
pub const REGISTER_PATH: &str = "/oauth/register";

/// The largest registration body read. A real one is a few hundred bytes.
pub const MAX_BODY_BYTES: usize = 16 * 1024;

/// How many redirect URIs one client may register.
const MAX_REDIRECT_URIS: usize = 10;

/// How long one redirect URI may be.
const MAX_REDIRECT_URI_LEN: usize = 512;

/// How long a client name may be, in characters.
const MAX_CLIENT_NAME_CHARS: usize = 100;

/// How long a requested `scope` may be.
const MAX_SCOPE_LEN: usize = 1024;

/// Registrations that have not yet completed an authorization, across the
/// instance. Past this, registration answers 429 until the sweep or a first
/// authorization makes room: a flood spread over many addresses fills this
/// bound, not the table.
pub const MAX_UNAUTHORIZED_CLIENTS: i64 = 10_000;

/// A client that never completes an authorization is deleted this long after
/// registering.
pub const UNAUTHORIZED_TTL: chrono::Duration = chrono::Duration::hours(24);

/// A client that completes no token exchange or refresh for this long is
/// deleted, with its grants and tokens. Longer than a refresh token lives
/// (`token::REFRESH_TOKEN_TTL_DAYS`), so a client in use always outlives it.
pub const UNUSED_TTL: chrono::Duration = chrono::Duration::days(90);

/// How often the sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// Private-use schemes accepted without reverse-DNS form: the schemes of
/// MCP clients known to redirect through one. Any other private-use scheme
/// must be reverse-DNS (RFC 8252 section 7.1), such as `com.example.app`.
const KNOWN_APP_SCHEMES: &[&str] = &["cursor", "vscode", "vscode-insiders", "windsurf", "zed"];

/// Schemes no redirect may use: they run script, read local files, or hand
/// the code to the browser or another protocol handler rather than an app.
const FORBIDDEN_SCHEMES: &[&str] = &[
    "javascript",
    "vbscript",
    "data",
    "file",
    "blob",
    "filesystem",
    "about",
    "view-source",
    "ftp",
    "ws",
    "wss",
    "mailto",
    "tel",
    "sms",
    "intent",
    "content",
    "jar",
    "resource",
    "chrome",
    "chrome-extension",
    "moz-extension",
    "ms-browser-extension",
    "safari-extension",
];

/// Dynamic registration as this instance configures it. `None` in
/// [`AppState`] is off.
#[derive(Debug, Clone, Default)]
pub struct DynamicRegistration {
    redirect_hosts: BTreeSet<String>,
}

impl DynamicRegistration {
    /// Registration on, allowing `https` redirects to `hosts`
    /// (comma-separated exact host names).
    pub fn with_redirect_hosts(hosts: &str) -> Result<Self, String> {
        Ok(Self {
            redirect_hosts: parse_host_list(DCR_REDIRECT_HOSTS_VAR, hosts)?,
        })
    }

    /// Read [`DYNAMIC_REGISTRATION_VAR`] and [`DCR_REDIRECT_HOSTS_VAR`].
    /// Unset, empty, `0` or `false` is off. Any other value, or a host list
    /// that does not parse, refuses boot.
    pub fn from_env() -> Result<Option<Self>, String> {
        let on = match std::env::var(DYNAMIC_REGISTRATION_VAR) {
            Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "" | "0" | "false" => false,
                "1" | "true" => true,
                other => {
                    return Err(format!(
                        "{DYNAMIC_REGISTRATION_VAR} must be 1 or 0, not {other:?}"
                    ));
                }
            },
            Err(std::env::VarError::NotPresent) => false,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(format!("{DYNAMIC_REGISTRATION_VAR} is not valid UTF-8"));
            }
        };
        let hosts = match std::env::var(DCR_REDIRECT_HOSTS_VAR) {
            Ok(raw) => raw,
            Err(std::env::VarError::NotPresent) => String::new(),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(format!("{DCR_REDIRECT_HOSTS_VAR} is not valid UTF-8"));
            }
        };
        let config = Self::with_redirect_hosts(&hosts)?;
        Ok(on.then_some(config))
    }

    /// The `https` redirect hosts, in order.
    pub fn redirect_hosts(&self) -> impl Iterator<Item = &str> {
        self.redirect_hosts.iter().map(String::as_str)
    }
}

/// An RFC 7591 registration request. Only the fields this server reads;
/// any other metadata (`client_uri`, `logo_uri`, `software_id`) is ignored
/// and never shown, so a client cannot dress the consent page.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClientRegistrationRequest {
    /// Where the authorization response may go. Required, one to ten.
    pub redirect_uris: Option<Vec<String>>,
    /// The client's own name, shown on the consent page as unverified.
    pub client_name: Option<String>,
    /// `none`, or omitted. A confidential method is refused.
    pub token_endpoint_auth_method: Option<String>,
    /// A subset of `authorization_code` and `refresh_token`, or omitted.
    pub grant_types: Option<Vec<String>>,
    /// `["code"]`, or omitted.
    pub response_types: Option<Vec<String>>,
    /// Space-delimited capabilities the client may ask for. Omitted or
    /// empty, any delegatable capability.
    pub scope: Option<String>,
}

/// The registered client, as RFC 7591 section 3.2.1 returns it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClientRegistrationResponse {
    pub client_id: String,
    /// Seconds since the epoch.
    pub client_id_issued_at: i64,
    pub client_name: String,
    /// The redirect URIs registered: the request's, less any `https` URI
    /// on a host the instance does not list.
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    /// Always `none`: a self-registered client is public.
    pub token_endpoint_auth_method: String,
    pub scope: String,
}

/// An RFC 7591 section 3.2.2 error.
#[derive(Debug)]
pub struct RegistrationError {
    code: &'static str,
    description: String,
}

impl RegistrationError {
    fn metadata(description: impl Into<String>) -> Self {
        Self {
            code: "invalid_client_metadata",
            description: description.into(),
        }
    }

    fn redirect(description: impl Into<String>) -> Self {
        Self {
            code: "invalid_redirect_uri",
            description: description.into(),
        }
    }

    /// The RFC 7591 error code.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl IntoResponse for RegistrationError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            [(header::CACHE_CONTROL, "no-store")],
            Json(OAuthErrorBody {
                error: self.code.to_string(),
                error_description: self.description,
            }),
        )
            .into_response()
    }
}

/// What to do with one requested redirect URI.
#[derive(Debug, PartialEq, Eq)]
pub enum RedirectVerdict {
    /// Register it as given.
    Accept,
    /// Leave it out: an `https` URI on a host the instance does not list.
    Skip,
}

/// Judge one redirect URI a self-registered client asks for. An error
/// refuses the whole registration.
pub fn judge_redirect_uri(
    uri: &str,
    https_hosts: &BTreeSet<String>,
) -> Result<RedirectVerdict, RegistrationError> {
    if uri.len() > MAX_REDIRECT_URI_LEN {
        return Err(RegistrationError::redirect(format!(
            "a redirect URI may be at most {MAX_REDIRECT_URI_LEN} bytes"
        )));
    }
    if uri.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(RegistrationError::redirect(
            "a redirect URI may not contain whitespace or control characters",
        ));
    }
    let url = Url::parse(uri)
        .map_err(|_| RegistrationError::redirect(format!("{uri:?} is not an absolute URI")))?;
    if url.fragment().is_some() {
        return Err(RegistrationError::redirect(
            "a redirect URI may not carry a fragment (RFC 6749 section 3.1.2)",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RegistrationError::redirect(
            "a redirect URI may not carry credentials",
        ));
    }
    let scheme = url.scheme();
    match scheme {
        "http" if is_loopback_host(&url) => Ok(RedirectVerdict::Accept),
        "http" => Err(RegistrationError::redirect(
            "an http redirect must be a loopback address: 127.0.0.1, [::1] or localhost",
        )),
        "https" => {
            let listed = url.port().is_none()
                && matches!(url.host(), Some(url::Host::Domain(host)) if https_hosts.contains(host));
            Ok(if listed {
                RedirectVerdict::Accept
            } else {
                RedirectVerdict::Skip
            })
        }
        _ if FORBIDDEN_SCHEMES.contains(&scheme) => Err(RegistrationError::redirect(format!(
            "the {scheme}: scheme is never a redirect"
        ))),
        _ if KNOWN_APP_SCHEMES.contains(&scheme) || is_reverse_dns(scheme) => {
            Ok(RedirectVerdict::Accept)
        }
        _ => Err(RegistrationError::redirect(format!(
            "a private-use scheme must be in reverse-DNS form, such as com.example.app, \
             not {scheme}:"
        ))),
    }
}

/// A private-use scheme in reverse-DNS form (RFC 8252 section 7.1): two or
/// more dot-separated labels of letters, digits and hyphens.
fn is_reverse_dns(scheme: &str) -> bool {
    let labels: Vec<&str> = scheme.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// A client name a person can read and trust to be what it looks like:
/// trimmed, non-empty, at most [`MAX_CLIENT_NAME_CHARS`] characters, and free
/// of control and invisible formatting characters (bidirectional overrides,
/// zero-width joiners), which could make one name render as another.
pub fn clean_client_name(raw: Option<&str>) -> Result<String, RegistrationError> {
    let name = raw.map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Ok("Unnamed app".to_string());
    }
    if name.chars().count() > MAX_CLIENT_NAME_CHARS {
        return Err(RegistrationError::metadata(format!(
            "client_name may be at most {MAX_CLIENT_NAME_CHARS} characters"
        )));
    }
    let invisible = |c: char| {
        c.is_control()
            || matches!(c,
                '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}')
    };
    if name.chars().any(invisible) {
        return Err(RegistrationError::metadata(
            "client_name may not contain control or invisible formatting characters",
        ));
    }
    Ok(name.to_string())
}

/// The capabilities a registered client may ask for. Omitted or blank, any
/// delegatable capability, as for a metadata document that names none; what
/// a token carries is still cut to what the member holds at authorize.
fn requested_scopes(scope: Option<&str>) -> Result<Vec<String>, RegistrationError> {
    let raw = scope.unwrap_or("");
    if raw.len() > MAX_SCOPE_LEN {
        return Err(RegistrationError::metadata(format!(
            "scope may be at most {MAX_SCOPE_LEN} bytes"
        )));
    }
    let mut named: Vec<String> = raw
        .split(' ')
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();
    if named.is_empty() {
        return Ok(capability::delegatable());
    }
    if let Some(bad) = named.iter().find(|c| !capability::is_delegatable(c)) {
        return Err(RegistrationError::metadata(format!(
            "scope {bad:?} is not a delegatable capability"
        )));
    }
    named.sort();
    named.dedup();
    Ok(named)
}

/// Check a registration request against `config` and return the client row
/// to insert, less its id. Pure, so every refusal is testable without a
/// server.
pub fn validate(
    request: ClientRegistrationRequest,
    config: &DynamicRegistration,
) -> Result<NewOAuthClient, RegistrationError> {
    match request.token_endpoint_auth_method.as_deref() {
        None | Some("none") => {}
        Some(other) => {
            return Err(RegistrationError::metadata(format!(
                "token_endpoint_auth_method must be none: this server registers public \
                 clients only, which authenticate with PKCE, not {other:?}"
            )));
        }
    }
    if let Some(grants) = &request.grant_types {
        if let Some(bad) = grants
            .iter()
            .find(|g| !matches!(g.as_str(), "authorization_code" | "refresh_token"))
        {
            return Err(RegistrationError::metadata(format!(
                "grant_type {bad:?} is not supported; use authorization_code and refresh_token"
            )));
        }
        if !grants.iter().any(|g| g == "authorization_code") {
            return Err(RegistrationError::metadata(
                "grant_types must include authorization_code",
            ));
        }
    }
    if let Some(responses) = &request.response_types
        && responses.iter().any(|r| r != "code")
    {
        return Err(RegistrationError::metadata(
            "response_types must be [\"code\"]",
        ));
    }
    let client_name = clean_client_name(request.client_name.as_deref())?;
    let allowed_scopes = requested_scopes(request.scope.as_deref())?;

    let requested = request.redirect_uris.unwrap_or_default();
    if requested.is_empty() {
        return Err(RegistrationError::redirect("redirect_uris is required"));
    }
    if requested.len() > MAX_REDIRECT_URIS {
        return Err(RegistrationError::redirect(format!(
            "at most {MAX_REDIRECT_URIS} redirect URIs may be registered"
        )));
    }
    let mut redirect_uris = Vec::new();
    for uri in requested {
        if judge_redirect_uri(&uri, &config.redirect_hosts)? == RedirectVerdict::Accept
            && !redirect_uris.contains(&uri)
        {
            redirect_uris.push(uri);
        }
    }
    if redirect_uris.is_empty() {
        return Err(RegistrationError::redirect(
            "no redirect URI is allowed: use a loopback http URI (127.0.0.1, [::1] or \
             localhost), an app's private-use scheme, or https on a host this server lists",
        ));
    }

    Ok(NewOAuthClient {
        client_id: format!("dyn-{}", Uuid::now_v7().simple()),
        name: client_name,
        redirect_uris,
        client_secret_hash: None,
        allowed_scopes,
    })
}

/// `POST /oauth/register`. 404 unless the instance names its public origin
/// and turns registration on.
pub async fn register(State(state): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let (Some(_), Some(config)) = (
        state.public_origin.as_deref(),
        state.oauth_dynamic_registration.as_ref(),
    ) else {
        return ApiError::NotFound.into_response();
    };
    // RFC 7591 registration is a JSON document. A missing type is read as
    // JSON; a declared other one is refused rather than guessed at.
    if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        let json = content_type.to_str().is_ok_and(|v| {
            v.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/json")
        });
        if !json {
            return ApiError::UnsupportedMediaType(
                "a registration request is application/json".into(),
            )
            .into_response();
        }
    }
    let body = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => {
            return ApiError::PayloadTooLarge(format!(
                "a registration request may be at most {MAX_BODY_BYTES} bytes"
            ))
            .into_response();
        }
    };
    let request: ClientRegistrationRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => {
            return RegistrationError::metadata(format!("the body is not a JSON object: {e}"))
                .into_response();
        }
    };
    let new = match validate(request, config) {
        Ok(new) => new,
        Err(e) => return e.into_response(),
    };

    match state
        .store
        .count_unauthorized_registered_oauth_clients()
        .await
    {
        Ok(waiting) if waiting >= MAX_UNAUTHORIZED_CLIENTS => {
            tracing::warn!(
                waiting,
                "OAuth dynamic registration refused: too many registrations await a first authorization"
            );
            let mut response = ApiError::TooManyRequests(
                "too many registrations are waiting on a first sign-in; try again later".into(),
            )
            .into_response();
            response.headers_mut().insert(
                header::RETRY_AFTER,
                header::HeaderValue::from_static("3600"),
            );
            return response;
        }
        Ok(_) => {}
        Err(e) => return ApiError::from(e).into_response(),
    }

    let now = Utc::now();
    let client = match state.store.register_oauth_client(new, now).await {
        Ok(client) => client,
        Err(e) => return ApiError::from(e).into_response(),
    };
    tracing::info!(
        client_id = %client.client_id,
        client_name = %client.name,
        redirect_uris = ?client.redirect_uris,
        "OAuth client registered itself"
    );
    let body = ClientRegistrationResponse {
        client_id: client.client_id,
        client_id_issued_at: now.timestamp(),
        client_name: client.name,
        redirect_uris: client.redirect_uris,
        grant_types: vec!["authorization_code".into(), "refresh_token".into()],
        response_types: vec!["code".into()],
        token_endpoint_auth_method: "none".into(),
        scope: client.allowed_scopes.join(" "),
    };
    (
        StatusCode::CREATED,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(body),
    )
        .into_response()
}

/// Delete self-registered clients that never completed an authorization
/// within [`UNAUTHORIZED_TTL`] or went unused for [`UNUSED_TTL`], as of
/// `now`. Returns how many went.
pub async fn sweep(
    store: &dyn maidan_store::Store,
    now: chrono::DateTime<Utc>,
) -> Result<u64, maidan_store::StoreError> {
    store
        .sweep_registered_oauth_clients(now - UNAUTHORIZED_TTL, now - UNUSED_TTL)
        .await
}

/// Run [`sweep`] at boot and every hour after. Runs whether or not
/// registration is on, so clients left from when it was are still swept.
pub async fn run_sweeper(store: std::sync::Arc<dyn maidan_store::Store>) {
    let mut tick = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        tick.tick().await;
        match sweep(store.as_ref(), Utc::now()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(swept = n, "swept self-registered OAuth clients"),
            Err(e) => tracing::warn!(error = %e, "sweeping self-registered OAuth clients failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(hosts: &str) -> DynamicRegistration {
        DynamicRegistration::with_redirect_hosts(hosts).expect("hosts")
    }

    fn request(uris: &[&str]) -> ClientRegistrationRequest {
        ClientRegistrationRequest {
            redirect_uris: Some(uris.iter().map(|u| u.to_string()).collect()),
            client_name: Some("Cursor".into()),
            token_endpoint_auth_method: Some("none".into()),
            grant_types: Some(vec!["authorization_code".into(), "refresh_token".into()]),
            response_types: Some(vec!["code".into()]),
            scope: None,
        }
    }

    #[test]
    fn loopback_app_schemes_and_listed_https_hosts_are_accepted() {
        let hosts = config("vscode.dev,insiders.vscode.dev").redirect_hosts;
        for uri in [
            "http://127.0.0.1/",
            "http://127.0.0.1:33418/",
            "http://localhost:8787/callback",
            "http://[::1]:5000/cb",
            "cursor://anysphere.cursor-mcp/oauth/callback",
            "vscode://vscode.github-authentication/did-authenticate",
            "com.example.app:/oauth2redirect",
            "https://vscode.dev/redirect",
            "https://insiders.vscode.dev/redirect",
        ] {
            assert_eq!(
                judge_redirect_uri(uri, &hosts).map_err(|e| e.description),
                Ok(RedirectVerdict::Accept),
                "{uri}"
            );
        }
    }

    #[test]
    fn https_on_an_unlisted_host_is_left_out_not_registered() {
        let hosts = config("vscode.dev").redirect_hosts;
        for uri in [
            "https://www.cursor.com/agents/mcp/oauth/callback",
            "https://evil.example/cb",
            "https://vscode.dev.evil.example/redirect",
            "https://evil.vscode.dev/redirect",
            "https://vscode.dev:8443/redirect",
            "https://127.0.0.1/cb",
        ] {
            assert_eq!(
                judge_redirect_uri(uri, &hosts).map_err(|e| e.description),
                Ok(RedirectVerdict::Skip),
                "{uri}"
            );
        }
    }

    #[test]
    fn script_files_remote_http_and_odd_schemes_refuse_the_registration() {
        let hosts = config("vscode.dev").redirect_hosts;
        for uri in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "vbscript:msgbox",
            "about:blank",
            "blob:https://vscode.dev/x",
            "http://evil.example/cb",
            "http://127.0.0.2/cb",
            "http://localhost.evil.example/cb",
            "http://0.0.0.0/cb",
            "myapp://callback",
            "http://127.0.0.1/cb#frag",
            "http://user:pw@127.0.0.1/cb",
            "http://127.0.0.1/ cb",
            "/relative/path",
            "",
        ] {
            assert!(judge_redirect_uri(uri, &hosts).is_err(), "{uri}");
        }
        let long = format!("http://127.0.0.1/{}", "a".repeat(600));
        assert!(judge_redirect_uri(&long, &hosts).is_err());
    }

    #[test]
    fn a_cursor_registration_keeps_its_loopback_and_drops_its_web_callback() {
        let new = validate(
            request(&[
                "https://www.cursor.com/agents/mcp/oauth/callback",
                "http://localhost:8787/callback",
            ]),
            &config("vscode.dev,insiders.vscode.dev"),
        )
        .map_err(|e| e.description)
        .expect("registers");
        assert_eq!(new.redirect_uris, vec!["http://localhost:8787/callback"]);
        assert_eq!(new.name, "Cursor");
        assert!(new.client_secret_hash.is_none());
        assert!(new.client_id.starts_with("dyn-"));
        assert_eq!(new.allowed_scopes, capability::delegatable());
    }

    #[test]
    fn a_registration_with_no_usable_redirect_is_refused() {
        let err = validate(request(&["https://evil.example/cb"]), &config("vscode.dev"))
            .expect_err("refused");
        assert_eq!(err.code(), "invalid_redirect_uri");
        let err = validate(request(&[]), &config("")).expect_err("refused");
        assert_eq!(err.code(), "invalid_redirect_uri");
        let many: Vec<String> = (0..11).map(|n| format!("http://127.0.0.1:{n}/")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(validate(request(&many), &config("")).is_err());
    }

    #[test]
    fn only_public_clients_register() {
        let mut confidential = request(&["http://127.0.0.1/"]);
        confidential.token_endpoint_auth_method = Some("client_secret_post".into());
        assert_eq!(
            validate(confidential, &config(""))
                .expect_err("refused")
                .code(),
            "invalid_client_metadata"
        );
        let mut omitted = request(&["http://127.0.0.1/"]);
        omitted.token_endpoint_auth_method = None;
        assert!(validate(omitted, &config("")).is_ok());
        let mut implicit = request(&["http://127.0.0.1/"]);
        implicit.response_types = Some(vec!["token".into()]);
        assert!(validate(implicit, &config("")).is_err());
        let mut client_credentials = request(&["http://127.0.0.1/"]);
        client_credentials.grant_types = Some(vec!["client_credentials".into()]);
        assert!(validate(client_credentials, &config("")).is_err());
    }

    #[test]
    fn a_requested_scope_names_delegatable_capabilities_only() {
        let mut scoped = request(&["http://127.0.0.1/"]);
        scoped.scope = Some("workspace:read workspace:read message:post".into());
        assert_eq!(
            validate(scoped, &config("")).expect("ok").allowed_scopes,
            vec!["message:post".to_string(), "workspace:read".to_string()]
        );
        let mut empty = request(&["http://127.0.0.1/"]);
        empty.scope = Some(String::new());
        assert_eq!(
            validate(empty, &config("")).expect("ok").allowed_scopes,
            capability::delegatable()
        );
        let mut grabby = request(&["http://127.0.0.1/"]);
        grabby.scope = Some("workspace:read approval:grant".into());
        assert!(validate(grabby, &config("")).is_err());
    }

    #[test]
    fn a_client_name_is_bounded_and_cannot_hide_characters() {
        assert_eq!(
            clean_client_name(None).map_err(|e| e.description),
            Ok("Unnamed app".into())
        );
        assert_eq!(
            clean_client_name(Some("  Cursor  ")).map_err(|e| e.description),
            Ok("Cursor".into())
        );
        assert!(clean_client_name(Some(&"x".repeat(101))).is_err());
        for spoof in [
            "Claude\u{202E}edoC",
            "Clau\u{200B}de",
            "Claude\n",
            "a\u{FEFF}b",
        ] {
            assert!(clean_client_name(Some(spoof)).is_err(), "{spoof:?}");
        }
    }

    #[test]
    fn the_redirect_host_list_takes_exact_host_names() {
        assert!(DynamicRegistration::with_redirect_hosts("https://vscode.dev").is_err());
        assert_eq!(
            config(" vscode.dev , insiders.vscode.dev ")
                .redirect_hosts()
                .collect::<Vec<_>>(),
            ["insiders.vscode.dev", "vscode.dev"]
        );
    }
}
