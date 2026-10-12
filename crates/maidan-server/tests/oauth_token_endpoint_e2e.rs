//! The OAuth 2.1 token and revocation endpoints over HTTP (`docs/OAuth.md`,
//! phase three): RFC 6749 error bodies, the three client authentication
//! methods, refresh-token rotation, lineage revocation when a refresh token
//! or a code is reused, the scope a refresh may narrow but never widen,
//! RFC 7009 revocation scoped to the client that holds the token, and the
//! RFC 8707 binding that makes an OAuth access token good on the MCP
//! endpoint and nowhere else. Auth is enabled throughout.

use std::sync::{atomic::AtomicI64, Arc};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMaidanSession, NewMember, NewOAuthClient, NewWorkspace,
    WorkspaceId,
};
use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

const ORIGIN: &str = "https://maidan.example.com";
const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";
const REDIRECT: &str = "https://client.example/callback";
const SESSION_SECRET: &[u8] = b"oauth-token-endpoint-e2e-secret-32";
/// The confidential client's secret.
const SECRET: &str = "confidential-client-secret";

struct Harness {
    base: String,
    client: reqwest::Client,
    no_redirect: reqwest::Client,
    store: Arc<dyn Store>,
    pool: SqlitePool,
}

async fn spawn(public_origin: Option<&str>) -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    state.public_origin = public_origin.map(str::to_string);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        no_redirect: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        store,
        pool,
    }
}

/// A workspace with a human member and that member's own bearer token.
struct Tenant {
    ws: WorkspaceId,
    member: MemberId,
    bearer: String,
}

async fn tenant(h: &Harness, name: &str) -> Tenant {
    let ws = h
        .store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-owner"),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                "workspace:read".into(),
                "workspace:write".into(),
                "message:post".into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    Tenant {
        ws: ws.id,
        member: member.id,
        bearer: secret.as_str().to_string(),
    }
}

/// Register a client allowed `workspace:read` and `workspace:write`; with
/// `secret`, a confidential one.
async fn register(h: &Harness, client_id: &str, secret: Option<&str>) {
    h.store
        .create_oauth_client(NewOAuthClient {
            client_id: client_id.into(),
            name: client_id.into(),
            redirect_uris: vec![REDIRECT.into()],
            client_secret_hash: secret.map(hash_secret),
            allowed_scopes: vec!["workspace:read".into(), "workspace:write".into()],
        })
        .await
        .unwrap();
}

/// A signed-in browser session cookie for `member`, as the OIDC callback
/// writes it.
async fn signed_in(h: &Harness, t: &Tenant) -> String {
    let session = h
        .store
        .create_session(NewMaidanSession {
            workspace_id: t.ws,
            member_id: t.member,
            api_token_id: None,
            oidc_identity_id: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    maidan_server::session::set_session_cookie(
        &mut headers,
        session.id,
        3600,
        false,
        SESSION_SECRET,
    )
    .unwrap();
    headers
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// An authorization code for `client_id` from `t`'s member, through
/// authorize and consent, and the PKCE verifier that goes with it.
async fn code_for(h: &Harness, t: &Tenant, client_id: &str) -> (String, String) {
    let verifier = URL_SAFE_NO_PAD.encode(uuid::Uuid::now_v7().as_bytes()) + "-pkce-verifier";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let url = format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&scope=workspace%3Aread%20workspace%3Awrite&state=xyz&resource={}",
        h.base,
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        urlencoding::encode(RESOURCE),
    );
    let resp = h
        .no_redirect
        .get(&url)
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp.headers()["location"].to_str().unwrap().to_string();
    let request_id = location.split("request=").nth(1).unwrap().to_string();
    let cookie = signed_in(h, t).await;
    let consent = h
        .no_redirect
        .post(format!("{}/ui/api/oauth/consent", h.base))
        .header("Cookie", cookie)
        .header("Sec-Fetch-Site", "same-origin")
        .form(&[("request_id", request_id.as_str()), ("approved", "true")])
        .send()
        .await
        .unwrap();
    assert_eq!(consent.status(), StatusCode::SEE_OTHER);
    let to = consent.headers()["location"].to_str().unwrap().to_string();
    let code = to
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    (urlencoding::decode(&code).unwrap().into_owned(), verifier)
}

/// How a request authenticates its client.
#[derive(Clone, Copy)]
enum Auth<'a> {
    /// `client_id` in the form and nothing else: a public client.
    Public(&'a str),
    /// `client_id` and `client_secret` in the form.
    Post(&'a str, &'a str),
    /// HTTP Basic.
    Basic(&'a str, &'a str),
}

async fn post_form(
    h: &Harness,
    path: &str,
    auth: Auth<'_>,
    params: &[(&str, &str)],
) -> reqwest::Response {
    let mut form: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut request = h.client.post(format!("{}{path}", h.base));
    match auth {
        Auth::Public(id) => form.push(("client_id".into(), id.into())),
        Auth::Post(id, secret) => {
            form.push(("client_id".into(), id.into()));
            form.push(("client_secret".into(), secret.into()));
        }
        Auth::Basic(id, secret) => request = request.basic_auth(id, Some(secret)),
    }
    request.form(&form).send().await.unwrap()
}

async fn exchange(h: &Harness, auth: Auth<'_>, code: &str, verifier: &str) -> reqwest::Response {
    post_form(
        h,
        "/oauth/token",
        auth,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
            ("resource", RESOURCE),
        ],
    )
    .await
}

async fn refresh(
    h: &Harness,
    auth: Auth<'_>,
    refresh_token: &str,
    extra: &[(&str, &str)],
) -> reqwest::Response {
    let mut params = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    if !extra.iter().any(|(k, _)| *k == "resource") {
        params.push(("resource", RESOURCE));
    }
    params.extend_from_slice(extra);
    params.retain(|(_, v)| !v.is_empty());
    post_form(h, "/oauth/token", auth, &params).await
}

/// The access token, refresh token and scope of a 200 token response.
struct Pair {
    access: String,
    refresh: String,
    scope: String,
}

async fn pair(resp: reqwest::Response) -> Pair {
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    Pair {
        access: body["access_token"].as_str().unwrap().to_string(),
        refresh: body["refresh_token"].as_str().unwrap().to_string(),
        scope: body["scope"].as_str().unwrap().to_string(),
    }
}

/// Run authorize, consent and the exchange for a public client.
async fn issue(h: &Harness, t: &Tenant, client_id: &str) -> Pair {
    let (code, verifier) = code_for(h, t, client_id).await;
    pair(exchange(h, Auth::Public(client_id), &code, &verifier).await).await
}

/// Assert an RFC 6749 error with this status and code, in its envelope.
async fn assert_oauth_error(resp: reqwest::Response, status: StatusCode, code: &str) -> Value {
    let url = resp.url().clone();
    assert_eq!(resp.status(), status, "{url}");
    assert_eq!(
        resp.headers()[header::CONTENT_TYPE],
        "application/json",
        "{url}"
    );
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], code, "{url}: {body}");
    assert!(
        body["error_description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "{body}"
    );
    body
}

/// `initialize` on the MCP endpoint with `token`: the route OAuth tokens are
/// bound to.
async fn mcp_initialize(h: &Harness, token: &str) -> StatusCode {
    h.client
        .post(format!("{}/mcp/streamable", h.base))
        .bearer_auth(token)
        .header("accept", "application/json")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .send()
        .await
        .unwrap()
        .status()
}

async fn refresh_used(h: &Harness, refresh_token: &str) -> bool {
    let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(refresh_token.as_bytes()));
    h.store
        .get_oauth_refresh_token(&hash)
        .await
        .unwrap()
        .unwrap()
        .used_at
        .is_some()
}

async fn lineage_revocations(h: &Harness, ws: WorkspaceId, reason: &str) -> usize {
    h.store
        .list_audit_for_workspace(ws, 200)
        .await
        .unwrap()
        .iter()
        .filter(|e| e.action == "oauth_lineage.revoke" && e.metadata["reason"] == reason)
        .count()
}

#[tokio::test]
async fn the_authorization_server_endpoints_answer_404_without_a_public_origin() {
    let h = spawn(None).await;
    let t = tenant(&h, "no-origin").await;
    register(&h, "public-client", None).await;
    for path in ["/oauth/token", "/oauth/revoke"] {
        let resp = post_form(&h, path, Auth::Public("public-client"), &[("token", "x")]).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{path}");
    }
    // Even a malformed body: the server is off before the body is read.
    let raw = h
        .client
        .post(format!("{}/oauth/token", h.base))
        .body("not a form")
        .send()
        .await
        .unwrap();
    assert_eq!(raw.status(), StatusCode::NOT_FOUND);
    let authorize = h
        .no_redirect
        .get(format!(
            "{}/oauth/authorize?response_type=code&client_id=public-client&redirect_uri={}&code_challenge=abc&code_challenge_method=S256&state=xyz&resource={}",
            h.base,
            urlencoding::encode(REDIRECT),
            urlencoding::encode(RESOURCE),
        ))
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(authorize.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_oauth_access_token_is_accepted_on_the_mcp_resource_and_nowhere_else() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "binding").await;
    register(&h, "public-client", None).await;
    let tokens = issue(&h, &t, "public-client").await;
    assert_eq!(tokens.scope, "workspace:read workspace:write");

    assert_eq!(mcp_initialize(&h, &tokens.access).await, StatusCode::OK);
    // The member's own token is unaffected by the binding.
    assert_eq!(mcp_initialize(&h, &t.bearer).await, StatusCode::OK);
    let me = h
        .client
        .get(format!("{}/me", h.base))
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);

    // Off the resource the token is no credential at all.
    let me = h
        .client
        .get(format!("{}/me", h.base))
        .bearer_auth(&tokens.access)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        me.headers()[header::WWW_AUTHENTICATE],
        "Bearer error=\"invalid_token\""
    );
    let legacy = h
        .client
        .post(format!("{}/mcp", h.base))
        .bearer_auth(&tokens.access)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::UNAUTHORIZED);
    // Nor can it be traded for a browser session that REST routes accept.
    let session = h
        .client
        .post(format!("{}/auth/session/from-token", h.base))
        .bearer_auth(&tokens.access)
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::UNAUTHORIZED);
    let channels = h
        .client
        .get(format!("{}/ui/api/workspaces/{}/channels", h.base, t.ws))
        .bearer_auth(&tokens.access)
        .send()
        .await
        .unwrap();
    assert_eq!(channels.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_confidential_client_authenticates_with_basic_or_post_and_never_both() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "confidential").await;
    register(&h, "conf-client", Some(SECRET)).await;

    let (code, verifier) = code_for(&h, &t, "conf-client").await;
    let basic =
        pair(exchange(&h, Auth::Basic("conf-client", SECRET), &code, &verifier).await).await;
    assert_eq!(mcp_initialize(&h, &basic.access).await, StatusCode::OK);
    let (code, verifier) = code_for(&h, &t, "conf-client").await;
    let posted =
        pair(exchange(&h, Auth::Post("conf-client", SECRET), &code, &verifier).await).await;
    assert_eq!(mcp_initialize(&h, &posted.access).await, StatusCode::OK);
    // A refresh authenticates the same ways.
    pair(refresh(&h, Auth::Basic("conf-client", SECRET), &basic.refresh, &[]).await).await;
    pair(refresh(&h, Auth::Post("conf-client", SECRET), &posted.refresh, &[]).await).await;

    let (code, verifier) = code_for(&h, &t, "conf-client").await;
    // No secret: a confidential client must authenticate.
    let missing = exchange(&h, Auth::Public("conf-client"), &code, &verifier).await;
    assert!(missing.headers().get(header::WWW_AUTHENTICATE).is_none());
    assert_oauth_error(missing, StatusCode::UNAUTHORIZED, "invalid_client").await;
    // A wrong secret, by Basic: the 401 names the scheme to retry with.
    let wrong = exchange(&h, Auth::Basic("conf-client", "wrong"), &code, &verifier).await;
    assert_eq!(
        wrong.headers()[header::WWW_AUTHENTICATE],
        "Basic realm=\"maidan\""
    );
    assert_oauth_error(wrong, StatusCode::UNAUTHORIZED, "invalid_client").await;
    let wrong = exchange(&h, Auth::Post("conf-client", "wrong"), &code, &verifier).await;
    assert_oauth_error(wrong, StatusCode::UNAUTHORIZED, "invalid_client").await;
    // Both methods in one request, or a body client_id beside a different
    // Basic one, is malformed.
    let both = h
        .client
        .post(format!("{}/oauth/token", h.base))
        .basic_auth("conf-client", Some(SECRET))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier.as_str()),
            ("resource", RESOURCE),
            ("client_secret", SECRET),
        ])
        .send()
        .await
        .unwrap();
    assert_oauth_error(both, StatusCode::BAD_REQUEST, "invalid_request").await;
    let mismatched = h
        .client
        .post(format!("{}/oauth/token", h.base))
        .basic_auth("conf-client", Some(SECRET))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier.as_str()),
            ("resource", RESOURCE),
            ("client_id", "someone-else"),
        ])
        .send()
        .await
        .unwrap();
    assert_oauth_error(mismatched, StatusCode::BAD_REQUEST, "invalid_request").await;
    // None of the refusals burned the code.
    pair(exchange(&h, Auth::Basic("conf-client", SECRET), &code, &verifier).await).await;
}

#[tokio::test]
async fn each_refused_token_request_answers_its_rfc_6749_error() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "errors").await;
    register(&h, "public-client", None).await;
    register(&h, "other-client", None).await;
    let client = Auth::Public("public-client");

    let no_grant_type = post_form(&h, "/oauth/token", client, &[("code", "x")]).await;
    assert_oauth_error(no_grant_type, StatusCode::BAD_REQUEST, "invalid_request").await;
    let password = post_form(&h, "/oauth/token", client, &[("grant_type", "password")]).await;
    assert_oauth_error(password, StatusCode::BAD_REQUEST, "unsupported_grant_type").await;
    let not_a_form = h
        .client
        .post(format!("{}/oauth/token", h.base))
        .json(&json!({"grant_type": "authorization_code"}))
        .send()
        .await
        .unwrap();
    assert_oauth_error(not_a_form, StatusCode::BAD_REQUEST, "invalid_request").await;
    let repeated = h
        .client
        .post(format!("{}/oauth/token", h.base))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("grant_type=authorization_code&grant_type=refresh_token&client_id=public-client")
        .send()
        .await
        .unwrap();
    assert_oauth_error(repeated, StatusCode::BAD_REQUEST, "invalid_request").await;
    let no_code = post_form(
        &h,
        "/oauth/token",
        client,
        &[("grant_type", "authorization_code")],
    )
    .await;
    assert_oauth_error(no_code, StatusCode::BAD_REQUEST, "invalid_request").await;
    let no_client = h
        .client
        .post(format!("{}/oauth/token", h.base))
        .form(&[("grant_type", "authorization_code"), ("code", "x")])
        .send()
        .await
        .unwrap();
    assert_oauth_error(no_client, StatusCode::UNAUTHORIZED, "invalid_client").await;
    let unknown_client = exchange(&h, Auth::Public("nobody"), "code", "verifier").await;
    assert_oauth_error(unknown_client, StatusCode::UNAUTHORIZED, "invalid_client").await;

    let (code, verifier) = code_for(&h, &t, "public-client").await;
    let unknown = exchange(&h, client, "not-a-code", &verifier).await;
    assert_oauth_error(unknown, StatusCode::BAD_REQUEST, "invalid_grant").await;
    let wrong_verifier = exchange(&h, client, &code, "a-wrong-verifier").await;
    assert_oauth_error(wrong_verifier, StatusCode::BAD_REQUEST, "invalid_grant").await;
    let other_client = exchange(&h, Auth::Public("other-client"), &code, &verifier).await;
    assert_oauth_error(other_client, StatusCode::BAD_REQUEST, "invalid_grant").await;
    let redirect = post_form(
        &h,
        "/oauth/token",
        client,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", "https://client.example/elsewhere"),
            ("code_verifier", verifier.as_str()),
            ("resource", RESOURCE),
        ],
    )
    .await;
    assert_oauth_error(redirect, StatusCode::BAD_REQUEST, "invalid_grant").await;
    for resource in [
        "",
        "https://maidan.example.com/mcp",
        "https://evil.example/mcp/streamable",
    ] {
        let mut params = vec![
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier.as_str()),
        ];
        if !resource.is_empty() {
            params.push(("resource", resource));
        }
        let refused = post_form(&h, "/oauth/token", client, &params).await;
        assert_oauth_error(refused, StatusCode::BAD_REQUEST, "invalid_target").await;
    }
    // A wrong guess at any parameter left the code usable.
    let tokens = pair(exchange(&h, client, &code, &verifier).await).await;

    // The authorize step requires the resource too.
    let authorize = h
        .no_redirect
        .get(format!(
            "{}/oauth/authorize?response_type=code&client_id=public-client&redirect_uri={}&code_challenge=abc&code_challenge_method=S256&state=xyz",
            h.base,
            urlencoding::encode(REDIRECT),
        ))
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(authorize.status(), StatusCode::BAD_REQUEST);

    // An expired code is refused.
    let (code, verifier) = code_for(&h, &t, "public-client").await;
    sqlx::query("UPDATE oauth_authorization_codes SET expires_at = ? WHERE code_hash = ?")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(5))
        .bind(URL_SAFE_NO_PAD.encode(Sha256::digest(code.as_bytes())))
        .execute(&h.pool)
        .await
        .unwrap();
    let expired = exchange(&h, client, &code, &verifier).await;
    assert_oauth_error(expired, StatusCode::BAD_REQUEST, "invalid_grant").await;

    // A refresh token that is not one is a bad grant; one without its
    // parameter is malformed.
    let bogus = refresh(&h, client, "not-a-refresh-token", &[]).await;
    assert_oauth_error(bogus, StatusCode::BAD_REQUEST, "invalid_grant").await;
    let missing = post_form(
        &h,
        "/oauth/token",
        client,
        &[("grant_type", "refresh_token")],
    )
    .await;
    assert_oauth_error(missing, StatusCode::BAD_REQUEST, "invalid_request").await;

    // An operator who narrows a client stops it refreshing past the new
    // limit: the client is no longer authorized for what it holds.
    sqlx::query("UPDATE oauth_clients SET allowed_scopes = ? WHERE client_id = ?")
        .bind(r#"["workspace:read"]"#)
        .bind("public-client")
        .execute(&h.pool)
        .await
        .unwrap();
    let narrowed = refresh(&h, client, &tokens.refresh, &[]).await;
    assert_oauth_error(narrowed, StatusCode::BAD_REQUEST, "unauthorized_client").await;
    let within =
        pair(refresh(&h, client, &tokens.refresh, &[("scope", "workspace:read")]).await).await;
    assert_eq!(within.scope, "workspace:read");
}

#[tokio::test]
async fn a_refresh_rotates_the_pair_and_the_new_pair_works() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "rotate").await;
    register(&h, "public-client", None).await;
    let client = Auth::Public("public-client");
    let first = issue(&h, &t, "public-client").await;
    assert!(!refresh_used(&h, &first.refresh).await);

    let second = pair(refresh(&h, client, &first.refresh, &[]).await).await;
    assert_ne!(second.access, first.access);
    assert_ne!(second.refresh, first.refresh);
    assert_eq!(second.scope, "workspace:read workspace:write");
    assert!(refresh_used(&h, &first.refresh).await);
    assert!(!refresh_used(&h, &second.refresh).await);
    assert_eq!(mcp_initialize(&h, &second.access).await, StatusCode::OK);
    // Rotation leaves the earlier access token to its own expiry.
    assert_eq!(mcp_initialize(&h, &first.access).await, StatusCode::OK);

    let third = pair(refresh(&h, client, &second.refresh, &[]).await).await;
    assert_eq!(mcp_initialize(&h, &third.access).await, StatusCode::OK);
    // Every token of the rotation is under one grant.
    let mut grant_ids = Vec::new();
    for access in [&first.access, &second.access, &third.access] {
        let token = h
            .store
            .get_active_api_token_by_hash(&hash_secret(access))
            .await
            .unwrap();
        grant_ids.push(token.oauth_grant_id.unwrap());
    }
    grant_ids.dedup();
    assert_eq!(grant_ids.len(), 1, "one grant under every rotation");
}

#[tokio::test]
async fn reusing_a_refresh_token_revokes_the_whole_lineage() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "reuse").await;
    register(&h, "public-client", None).await;
    let client = Auth::Public("public-client");
    let first = issue(&h, &t, "public-client").await;
    let second = pair(refresh(&h, client, &first.refresh, &[]).await).await;
    assert_eq!(mcp_initialize(&h, &second.access).await, StatusCode::OK);

    // The old refresh token, presented again, is the sign of a stolen one.
    let reused = refresh(&h, client, &first.refresh, &[]).await;
    assert_oauth_error(reused, StatusCode::BAD_REQUEST, "invalid_grant").await;

    // The newest access token stops on its next request, and so does every
    // earlier one; the newest refresh token is refused.
    assert_eq!(
        mcp_initialize(&h, &second.access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        mcp_initialize(&h, &first.access).await,
        StatusCode::UNAUTHORIZED
    );
    let newest = refresh(&h, client, &second.refresh, &[]).await;
    assert_oauth_error(newest, StatusCode::BAD_REQUEST, "invalid_grant").await;
    assert_eq!(
        lineage_revocations(&h, t.ws, "refresh_token_reuse").await,
        1
    );

    // A new consent starts a new lineage, which works.
    let fresh = issue(&h, &t, "public-client").await;
    assert_eq!(mcp_initialize(&h, &fresh.access).await, StatusCode::OK);
}

#[tokio::test]
async fn reusing_an_authorization_code_revokes_its_lineage() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "code-reuse").await;
    register(&h, "public-client", None).await;
    let client = Auth::Public("public-client");
    let (code, verifier) = code_for(&h, &t, "public-client").await;
    let tokens = pair(exchange(&h, client, &code, &verifier).await).await;
    assert_eq!(mcp_initialize(&h, &tokens.access).await, StatusCode::OK);

    // The code alone, without its verifier, is refused but revokes nothing:
    // under PKCE it proves nothing, and anyone who saw the redirect has it.
    let guessed = exchange(&h, client, &code, "not-the-verifier").await;
    assert_oauth_error(guessed, StatusCode::BAD_REQUEST, "invalid_grant").await;
    assert_eq!(mcp_initialize(&h, &tokens.access).await, StatusCode::OK);
    assert_eq!(lineage_revocations(&h, t.ws, "code_reuse").await, 0);

    let again = exchange(&h, client, &code, &verifier).await;
    assert_oauth_error(again, StatusCode::BAD_REQUEST, "invalid_grant").await;
    assert_eq!(
        mcp_initialize(&h, &tokens.access).await,
        StatusCode::UNAUTHORIZED
    );
    let refreshed = refresh(&h, client, &tokens.refresh, &[]).await;
    assert_oauth_error(refreshed, StatusCode::BAD_REQUEST, "invalid_grant").await;
    assert_eq!(lineage_revocations(&h, t.ws, "code_reuse").await, 1);

    // A code whose grant is revoked before the exchange dies with the grant,
    // rather than starting a lineage the revocation never saw.
    let (code, verifier) = code_for(&h, &t, "public-client").await;
    let grant = h
        .store
        .find_oauth_grant(
            "public-client",
            t.member,
            t.ws,
            &["workspace:read".to_string(), "workspace:write".to_string()],
        )
        .await
        .unwrap()
        .expect("consent recorded a new grant");
    h.store.revoke_oauth_grant(grant.id).await.unwrap();
    let orphaned = exchange(&h, client, &code, &verifier).await;
    assert_oauth_error(orphaned, StatusCode::BAD_REQUEST, "invalid_grant").await;
}

#[tokio::test]
async fn a_refresh_can_narrow_the_scope_but_never_widen_it() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "scope").await;
    register(&h, "public-client", None).await;
    let client = Auth::Public("public-client");
    let first = issue(&h, &t, "public-client").await;

    let narrow =
        pair(refresh(&h, client, &first.refresh, &[("scope", "workspace:read")]).await).await;
    assert_eq!(narrow.scope, "workspace:read");
    let minted = h
        .store
        .get_active_api_token_by_hash(&hash_secret(&narrow.access))
        .await
        .unwrap();
    assert_eq!(minted.capabilities, vec!["workspace:read".to_string()]);

    // A scope the member holds but never granted is still a widening.
    for wider in ["workspace:read message:post", "token:mint"] {
        let refused = refresh(&h, client, &narrow.refresh, &[("scope", wider)]).await;
        assert_oauth_error(refused, StatusCode::BAD_REQUEST, "invalid_scope").await;
    }
    // A refused widening does not use up the refresh token, and with no scope
    // the refresh is for what the member granted (RFC 6749 section 6).
    assert!(!refresh_used(&h, &narrow.refresh).await);
    let full = pair(refresh(&h, client, &narrow.refresh, &[]).await).await;
    assert_eq!(full.scope, "workspace:read workspace:write");
}

#[tokio::test]
async fn a_refresh_must_name_the_resource_its_token_is_bound_to() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "refresh-target").await;
    register(&h, "public-client", None).await;
    let client = Auth::Public("public-client");
    let first = issue(&h, &t, "public-client").await;
    for resource in ["", "https://maidan.example.com/mcp"] {
        let refused = refresh(&h, client, &first.refresh, &[("resource", resource)]).await;
        assert_oauth_error(refused, StatusCode::BAD_REQUEST, "invalid_target").await;
    }
    assert!(!refresh_used(&h, &first.refresh).await);
    pair(refresh(&h, client, &first.refresh, &[]).await).await;
}

#[tokio::test]
async fn revocation_ends_an_access_token_or_a_refresh_lineage_and_ignores_other_clients_tokens() {
    let h = spawn(Some(ORIGIN)).await;
    let t = tenant(&h, "revoke").await;
    register(&h, "client-x", None).await;
    register(&h, "client-y", Some(SECRET)).await;
    let x = Auth::Public("client-x");
    let y = Auth::Basic("client-y", SECRET);
    let tokens = issue(&h, &t, "client-x").await;

    // Another client gets the same 200 and changes nothing.
    for (token, hint) in [
        (&tokens.access, "access_token"),
        (&tokens.refresh, "refresh_token"),
    ] {
        let resp = post_form(
            &h,
            "/oauth/revoke",
            y,
            &[("token", token), ("token_type_hint", hint)],
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.text().await.unwrap().is_empty());
    }
    assert_eq!(mcp_initialize(&h, &tokens.access).await, StatusCode::OK);
    assert!(!refresh_used(&h, &tokens.refresh).await);

    // The owner revokes its access token: that token only.
    let resp = post_form(
        &h,
        "/oauth/revoke",
        x,
        &[
            ("token", &tokens.access),
            ("token_type_hint", "access_token"),
        ],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        mcp_initialize(&h, &tokens.access).await,
        StatusCode::UNAUTHORIZED
    );
    let next = pair(refresh(&h, x, &tokens.refresh, &[]).await).await;
    assert_eq!(mcp_initialize(&h, &next.access).await, StatusCode::OK);
    assert!(h
        .store
        .list_audit_for_workspace(t.ws, 100)
        .await
        .unwrap()
        .iter()
        .any(|e| e.action == "oauth_token.revoke"));

    // A refresh token, with no hint, takes its lineage with it.
    let resp = post_form(&h, "/oauth/revoke", x, &[("token", &next.refresh)]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        mcp_initialize(&h, &next.access).await,
        StatusCode::UNAUTHORIZED
    );
    let refused = refresh(&h, x, &next.refresh, &[]).await;
    assert_oauth_error(refused, StatusCode::BAD_REQUEST, "invalid_grant").await;
    assert_eq!(lineage_revocations(&h, t.ws, "client_revocation").await, 1);

    // An unknown token is a 200; a missing one or a bad secret is an error.
    let unknown = post_form(&h, "/oauth/revoke", x, &[("token", "never-issued")]).await;
    assert_eq!(unknown.status(), StatusCode::OK);
    let missing = post_form(&h, "/oauth/revoke", x, &[]).await;
    assert_oauth_error(missing, StatusCode::BAD_REQUEST, "invalid_request").await;
    let bad_secret = post_form(
        &h,
        "/oauth/revoke",
        Auth::Basic("client-y", "wrong"),
        &[("token", "never-issued")],
    )
    .await;
    assert_eq!(
        bad_secret.headers()[header::WWW_AUTHENTICATE],
        "Basic realm=\"maidan\""
    );
    assert_oauth_error(bad_secret, StatusCode::UNAUTHORIZED, "invalid_client").await;
    let nobody = post_form(
        &h,
        "/oauth/revoke",
        Auth::Public("nobody"),
        &[("token", "x")],
    )
    .await;
    assert_oauth_error(nobody, StatusCode::UNAUTHORIZED, "invalid_client").await;
}

#[tokio::test]
async fn a_refresh_token_stays_with_the_tenant_and_client_it_was_issued_to() {
    let h = spawn(Some(ORIGIN)).await;
    let a = tenant(&h, "tenant-a").await;
    let b = tenant(&h, "tenant-b").await;
    register(&h, "client-x", None).await;
    register(&h, "client-z", None).await;
    let x = Auth::Public("client-x");
    let a_tokens = issue(&h, &a, "client-x").await;
    let b_tokens = issue(&h, &b, "client-x").await;

    // Another client cannot spend workspace A's refresh token.
    let stolen = refresh(&h, Auth::Public("client-z"), &a_tokens.refresh, &[]).await;
    assert_oauth_error(stolen, StatusCode::BAD_REQUEST, "invalid_grant").await;
    assert!(!refresh_used(&h, &a_tokens.refresh).await);

    // Its own client gets a token for A's member in A, never B's.
    let a_next = pair(refresh(&h, x, &a_tokens.refresh, &[]).await).await;
    let minted = h
        .store
        .get_active_api_token_by_hash(&hash_secret(&a_next.access))
        .await
        .unwrap();
    assert_eq!((minted.workspace_id, minted.member_id), (a.ws, a.member));
    let b_minted = h
        .store
        .get_active_api_token_by_hash(&hash_secret(&b_tokens.access))
        .await
        .unwrap();
    assert_eq!(
        (b_minted.workspace_id, b_minted.member_id),
        (b.ws, b.member)
    );
    assert_ne!(minted.oauth_grant_id, b_minted.oauth_grant_id);

    // A reuse in A revokes A's lineage and leaves B's, under the same client.
    let reused = refresh(&h, x, &a_tokens.refresh, &[]).await;
    assert_oauth_error(reused, StatusCode::BAD_REQUEST, "invalid_grant").await;
    assert_eq!(
        mcp_initialize(&h, &a_next.access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(mcp_initialize(&h, &b_tokens.access).await, StatusCode::OK);
    pair(refresh(&h, x, &b_tokens.refresh, &[]).await).await;
    assert_eq!(
        lineage_revocations(&h, b.ws, "refresh_token_reuse").await,
        0
    );
}
