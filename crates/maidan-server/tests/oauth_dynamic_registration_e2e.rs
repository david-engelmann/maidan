//! RFC 7591 dynamic client registration over HTTP (`docs/OAuth.md`,
//! decided 2026-11-05): an MCP client registers itself, signs a person in
//! with PKCE on a loopback redirect whose port is its own, exchanges the code
//! and calls MCP, all through the grant path every other client uses. The
//! refusals: script, file and remote-web redirects, a confidential client,
//! a redirect at authorize the client never registered, and the whole
//! surface (404, no `registration_endpoint`, registered clients refused)
//! while the instance leaves registration off. The sweep, and two tenants:
//! a token authorized by a member of A reads nothing of B.

use std::sync::{atomic::AtomicI64, Arc};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::oauth::register::DynamicRegistration;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMaidanSession, NewMember, NewWorkspace,
    WorkspaceId,
};
use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ORIGIN: &str = "https://maidan.example.com";
const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";
const SESSION_SECRET: &[u8] = b"oauth-dynamic-registration-e2e-secret";
/// What the client registers: a loopback callback with no port, as VS Code
/// and Claude Code's documents do.
const REGISTERED: &str = "http://127.0.0.1/callback";
/// What it sends at authorize: the same callback on the port its OS gave it.
const ON_ITS_PORT: &str = "http://127.0.0.1:53123/callback";

struct Harness {
    base: String,
    client: reqwest::Client,
    no_redirect: reqwest::Client,
    store: Arc<dyn Store>,
}

async fn spawn(registration: Option<DynamicRegistration>) -> Harness {
    let pool = maidan_search::sqlite_pool_options()
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
    state.public_origin = Some(ORIGIN.into());
    state.oauth_dynamic_registration = registration;
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
    }
}

fn on() -> Option<DynamicRegistration> {
    Some(DynamicRegistration::with_redirect_hosts("vscode.dev,insiders.vscode.dev").unwrap())
}

/// A workspace with a human member, that member's bearer, and one channel
/// whose name only this tenant knows.
struct Tenant {
    ws: WorkspaceId,
    member: MemberId,
    bearer: String,
    channel_name: String,
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
    let channel_name = format!("{name}-secret-plans");
    h.store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: channel_name.clone(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Tenant {
        ws: ws.id,
        member: member.id,
        bearer: secret.as_str().to_string(),
        channel_name,
    }
}

async fn register(h: &Harness, body: Value) -> reqwest::Response {
    h.client
        .post(format!("{}/oauth/register", h.base))
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Register a client the way Cursor or VS Code does, and return its id.
async fn registered_client(h: &Harness, name: &str, redirect: &str) -> String {
    let resp = register(
        h,
        json!({
            "client_name": name,
            "redirect_uris": [redirect],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = resp.json().await.unwrap();
    body["client_id"].as_str().unwrap().to_string()
}

/// A signed-in browser session cookie for `t`'s member.
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

fn pkce() -> (String, String) {
    let verifier = URL_SAFE_NO_PAD.encode(uuid::Uuid::now_v7().as_bytes())
        + "-dcr-pkce-verifier-of-rfc-7636-length";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// `GET /oauth/authorize` as `t`'s member, naming no scope (as Cursor and
/// claude.ai do).
async fn authorize(
    h: &Harness,
    t: &Tenant,
    client_id: &str,
    redirect: &str,
    challenge: &str,
) -> reqwest::Response {
    let url = format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=st&resource={}",
        h.base,
        urlencoding::encode(client_id),
        urlencoding::encode(redirect),
        urlencoding::encode(RESOURCE),
    );
    h.no_redirect
        .get(&url)
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap()
}

/// Authorize, read the consent page, approve, and return the code, the
/// verifier and the consent page's HTML.
async fn sign_in(
    h: &Harness,
    t: &Tenant,
    client_id: &str,
    redirect: &str,
) -> (String, String, String) {
    let (verifier, challenge) = pkce();
    let resp = authorize(h, t, client_id, redirect, &challenge).await;
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp.headers()["location"].to_str().unwrap().to_string();
    let request_id = location.split("request=").nth(1).unwrap().to_string();
    let cookie = signed_in(h, t).await;
    let page = h
        .client
        .get(format!("{}{location}", h.base))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = page.text().await.unwrap();
    let consent = h
        .no_redirect
        .post(format!("{}/ui/api/oauth/consent", h.base))
        .header("Cookie", &cookie)
        .header("Sec-Fetch-Site", "same-origin")
        .form(&[("request_id", request_id.as_str()), ("approved", "true")])
        .send()
        .await
        .unwrap();
    assert_eq!(consent.status(), StatusCode::SEE_OTHER);
    let to = consent.headers()["location"].to_str().unwrap().to_string();
    assert!(
        to.starts_with(&format!("{redirect}?code=")),
        "the code goes to the redirect the client sent, port and all: {to}"
    );
    let code = to
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    (
        urlencoding::decode(&code).unwrap().into_owned(),
        verifier,
        html,
    )
}

async fn exchange(
    h: &Harness,
    client_id: &str,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> reqwest::Response {
    h.client
        .post(format!("{}/oauth/token", h.base))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
            ("resource", RESOURCE),
        ])
        .send()
        .await
        .unwrap()
}

/// One MCP `tools/call` over `/mcp/streamable`: the tool output, or the
/// error text.
async fn tool(h: &Harness, token: &str, name: &str, args: Value) -> Result<String, String> {
    let resp = h
        .client
        .post(format!("{}/mcp/streamable", h.base))
        .bearer_auth(token)
        .header(header::ACCEPT, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {body}"));
    }
    let reply: Value = serde_json::from_str(&body).unwrap();
    if !reply["error"].is_null() {
        return Err(reply["error"].to_string());
    }
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if reply["result"]["isError"] == true {
        return Err(text);
    }
    Ok(text)
}

/// A full sign-in for a fresh client of `t`'s, returning the access token.
async fn access_token(h: &Harness, t: &Tenant, client_id: &str) -> String {
    let (code, verifier, _) = sign_in(h, t, client_id, ON_ITS_PORT).await;
    let resp = exchange(h, client_id, &code, ON_ITS_PORT, &verifier).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    body["access_token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_self_registered_client_signs_in_with_pkce_on_its_own_loopback_port_and_calls_mcp() {
    let h = spawn(on()).await;
    let a = tenant(&h, "dcr-a").await;

    let discovery: Value = h
        .client
        .get(format!("{}/.well-known/oauth-authorization-server", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        discovery["registration_endpoint"],
        "https://maidan.example.com/oauth/register"
    );

    let resp = register(
        &h,
        json!({
            "client_name": "Cursor",
            "redirect_uris": [
                "https://www.cursor.com/agents/mcp/oauth/callback",
                REGISTERED,
            ],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "client_uri": "https://cursor.com",
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
    let registered: Value = resp.json().await.unwrap();
    let client_id = registered["client_id"].as_str().unwrap().to_string();
    assert_eq!(registered["token_endpoint_auth_method"], "none");
    assert!(registered.get("client_secret").is_none(), "no secret, ever");
    assert_eq!(
        registered["redirect_uris"],
        json!([REGISTERED]),
        "the web callback on an unlisted host is left out"
    );

    let (code, verifier, html) = sign_in(&h, &a, &client_id, ON_ITS_PORT).await;
    assert!(
        html.contains("Cursor (unverified app, registered by itself)"),
        "the self-declared name is marked unverified: {html}"
    );
    assert!(html.contains("Maidan has not verified this app"));
    assert!(html.contains("Allow only if you just started this sign-in yourself"));
    assert!(
        html.contains("an app on this computer (127.0.0.1:53123)"),
        "the page names where the code goes"
    );

    // The exchange names the redirect the code was issued for.
    let wrong_port = exchange(
        &h,
        &client_id,
        &code,
        "http://127.0.0.1:1/callback",
        &verifier,
    )
    .await;
    assert_eq!(wrong_port.status(), StatusCode::BAD_REQUEST);
    let resp = exchange(&h, &client_id, &code, ON_ITS_PORT, &verifier).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let tokens: Value = resp.json().await.unwrap();
    let access = tokens["access_token"].as_str().unwrap();
    // Capped at what the member holds, as for every other client.
    for cap in tokens["scope"].as_str().unwrap().split(' ') {
        assert!(
            ["workspace:read", "workspace:write", "message:post"].contains(&cap),
            "{cap} is beyond the member's capabilities"
        );
    }

    let channels = tool(
        &h,
        access,
        "list_channels",
        json!({"workspace_id": a.ws.to_string()}),
    )
    .await
    .expect("the token works on MCP");
    assert!(channels.contains(&a.channel_name));

    // A refresh works for it too, as a public client.
    let refreshed = h
        .client
        .post(format!("{}/oauth/token", h.base))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id.as_str()),
            ("refresh_token", tokens["refresh_token"].as_str().unwrap()),
            ("resource", RESOURCE),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(refreshed.status(), StatusCode::OK);
}

#[tokio::test]
async fn unsafe_redirects_and_confidential_clients_are_refused() {
    let h = spawn(on()).await;
    for redirect in [
        "javascript:alert(document.cookie)",
        "data:text/html,<script>alert(1)</script>",
        "file:///etc/passwd",
        "http://evil.example/callback",
        "http://127.0.0.2/callback",
        "myapp://callback",
        "http://127.0.0.1/callback#frag",
    ] {
        let resp = register(&h, json!({"client_name": "x", "redirect_uris": [redirect]})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{redirect}");
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["error"], "invalid_redirect_uri", "{redirect}");
    }
    // A script redirect beside a good one still refuses the registration.
    let resp = register(
        &h,
        json!({"redirect_uris": [REGISTERED, "javascript:alert(1)"]}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // Only an unlisted https host: nothing to register.
    let resp = register(&h, json!({"redirect_uris": ["https://evil.example/cb"]})).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // The listed host registers.
    let resp = register(
        &h,
        json!({"redirect_uris": ["https://vscode.dev/redirect", "http://127.0.0.1:33418/"]}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    // A confidential client is not registered.
    let resp = register(
        &h,
        json!({
            "redirect_uris": [REGISTERED],
            "token_endpoint_auth_method": "client_secret_post",
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_client_metadata");
    // Not JSON at all.
    let resp = h
        .client
        .post(format!("{}/oauth/register", h.base))
        .header(header::CONTENT_TYPE, "application/json")
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // At authorize, only what was registered: the loopback callback on any
    // port, nothing else on loopback, and no other host.
    let a = tenant(&h, "dcr-refused").await;
    let client_id = registered_client(&h, "Cursor", REGISTERED).await;
    let (_, challenge) = pkce();
    for redirect in [
        "http://127.0.0.1:53123/other",
        "http://localhost:53123/callback",
        "https://evil.example/callback",
        "http://127.0.0.1:53123/callback?next=https://evil.example",
    ] {
        let resp = authorize(&h, &a, &client_id, redirect, &challenge).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{redirect}");
    }
    // PKCE is required of a self-registered client like any other.
    let url = format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&state=st&resource={}&code_challenge=&code_challenge_method=plain",
        h.base,
        urlencoding::encode(&client_id),
        urlencoding::encode(ON_ITS_PORT),
        urlencoding::encode(RESOURCE),
    );
    let resp = h
        .no_redirect
        .get(&url)
        .bearer_auth(&a.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn with_registration_off_the_endpoint_answers_404_and_registered_clients_are_refused() {
    let h = spawn(None).await;
    let resp = register(&h, json!({"redirect_uris": [REGISTERED]})).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let discovery: Value = h
        .client
        .get(format!("{}/.well-known/oauth-authorization-server", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        discovery.get("registration_endpoint").is_none(),
        "nothing is advertised that is not served"
    );

    // A client that registered while it was on is refused once it is off.
    h.store
        .register_oauth_client(
            maidan_types::NewOAuthClient {
                client_id: "dyn-left-over".into(),
                name: "Left over".into(),
                redirect_uris: vec![REGISTERED.into()],
                client_secret_hash: None,
                allowed_scopes: vec!["workspace:read".into()],
            },
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let a = tenant(&h, "dcr-off").await;
    let (_, challenge) = pkce();
    let resp = authorize(&h, &a, "dyn-left-over", ON_ITS_PORT, &challenge).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_token_a_member_of_a_authorized_for_a_self_registered_client_reads_nothing_of_b() {
    let h = spawn(on()).await;
    let a = tenant(&h, "dcr-two-a").await;
    let b = tenant(&h, "dcr-two-b").await;
    // Registered by anyone: registration is anonymous.
    let client_id = registered_client(&h, "Claude", REGISTERED).await;
    let token = access_token(&h, &a, &client_id).await;

    let own = tool(
        &h,
        &token,
        "list_channels",
        json!({"workspace_id": a.ws.to_string()}),
    )
    .await
    .expect("its own workspace reads");
    assert!(own.contains(&a.channel_name));
    assert!(!own.contains(&b.channel_name));

    match tool(
        &h,
        &token,
        "list_channels",
        json!({"workspace_id": b.ws.to_string()}),
    )
    .await
    {
        Ok(text) => panic!("workspace B answered A's token: {text}"),
        Err(e) => assert!(
            !e.contains(&b.channel_name),
            "the refusal leaked B's channel: {e}"
        ),
    }
    // Nor does it work anywhere but MCP (RFC 8707), B's REST included.
    let rest = h
        .client
        .get(format!("{}/workspaces/{}/channels", h.base, b.ws))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(rest.status().is_client_error());
    let body = rest.text().await.unwrap();
    assert!(!body.contains(&b.channel_name));
}

#[tokio::test]
async fn a_client_that_never_signs_in_is_swept_after_a_day_and_one_in_use_is_kept() {
    let h = spawn(on()).await;
    let a = tenant(&h, "dcr-sweep").await;
    let idle = registered_client(&h, "Idle", REGISTERED).await;
    let used = registered_client(&h, "Used", REGISTERED).await;
    let token = access_token(&h, &a, &used).await;

    let now = chrono::Utc::now();
    let swept =
        maidan_server::oauth::register::sweep(h.store.as_ref(), now + chrono::Duration::hours(25))
            .await
            .unwrap();
    assert_eq!(swept, 1, "only the client that never signed in goes");
    assert!(h
        .store
        .get_oauth_client_by_client_id(&idle)
        .await
        .unwrap()
        .is_none());
    assert!(h
        .store
        .get_oauth_client_by_client_id(&used)
        .await
        .unwrap()
        .is_some());
    assert!(tool(
        &h,
        &token,
        "list_channels",
        json!({"workspace_id": a.ws.to_string()})
    )
    .await
    .is_ok());

    // Ninety days unused: it goes, and what it was issued goes with it.
    let swept =
        maidan_server::oauth::register::sweep(h.store.as_ref(), now + chrono::Duration::days(91))
            .await
            .unwrap();
    assert_eq!(swept, 1);
    assert!(h
        .store
        .get_oauth_client_by_client_id(&used)
        .await
        .unwrap()
        .is_none());
    assert!(
        tool(
            &h,
            &token,
            "list_channels",
            json!({"workspace_id": a.ws.to_string()})
        )
        .await
        .is_err(),
        "a swept client's token stops working"
    );
}
