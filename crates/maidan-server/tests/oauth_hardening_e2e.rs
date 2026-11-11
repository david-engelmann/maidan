//! The OAuth authorization server's security properties that the other
//! `oauth_*` suites did not pin (`docs/OAuth.md`, "Security properties"):
//! PKCE held to RFC 7636's shapes for every client, consent only by a person
//! on a same-origin request to a page that cannot be framed, client metadata
//! documents fetched only from public addresses, bounded, and naming their
//! own URL, a deactivated member's apps disconnected for good, and the
//! redirect that carries a code kept out of the trace. Two tenants wherever
//! a tenant could leak. Auth is enabled throughout.

// The client's document server is a plain axum server, not the API (see
// clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::{
    io::Write,
    sync::{atomic::AtomicI64, Arc, Mutex},
};

use axum::{routing::get, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{oauth::registry::ClientMetadataHosts, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, MemberId, MemberKind, NewApiToken, NewAuditEvent, NewMaidanSession, NewMember,
    NewOAuthClient, NewWorkspace, WorkspaceId,
};
use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

const ORIGIN: &str = "https://maidan.example.com";
const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";
const REDIRECT: &str = "https://client.example/callback";
const SESSION_SECRET: &[u8] = b"oauth-hardening-e2e-session-secret-32b";
/// A listed host's well-formed document, whose `client_id` is its own URL.
const DOC: &str = "https://docs.client.example/oauth/client.json";
/// A listed host's document whose `client_id` names another document.
const OTHER_ID: &str = "https://docs.client.example/oauth/other-id.json";
/// A listed host's document with no `client_id` at all.
const NO_ID: &str = "https://docs.client.example/oauth/no-id.json";
/// A listed host's document past the size limit.
const HUGE: &str = "https://docs.client.example/oauth/huge.json";
/// A listed host's document whose name hides a right-to-left override.
const SPOOFED_NAME: &str = "https://docs.client.example/oauth/spoofed-name.json";

struct Harness {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: SqlitePool,
    _dir: tempfile::TempDir,
}

/// Serves the documents above, from loopback, through the test-only fetch
/// origin override.
async fn document_server() -> String {
    let doc = |client_id: Option<&str>, name: &str| {
        let mut body = json!({
            "client_name": name,
            "redirect_uris": [REDIRECT],
            "scope": "workspace:read workspace:write",
        });
        if let Some(id) = client_id {
            body["client_id"] = json!(id);
        }
        body
    };
    let valid = doc(Some(DOC), "Document Client");
    let other = doc(Some(DOC), "Document Client");
    let missing = doc(None, "Document Client");
    let mut huge = doc(Some(HUGE), "Huge Client");
    huge["padding"] = json!("x".repeat(64 * 1024));
    let spoofed = doc(Some(SPOOFED_NAME), "Cl\u{202E}edua");
    let app = Router::new()
        .route(
            "/oauth/client.json",
            get(move || async move { axum::Json(valid) }),
        )
        .route(
            "/oauth/other-id.json",
            get(move || async move { axum::Json(other) }),
        )
        .route(
            "/oauth/no-id.json",
            get(move || async move { axum::Json(missing) }),
        )
        .route(
            "/oauth/huge.json",
            get(move || async move { axum::Json(huge) }),
        )
        .route(
            "/oauth/spoofed-name.json",
            get(move || async move { axum::Json(spoofed) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A server whose metadata hosts are `hosts`; documents come from the
/// loopback document server only when `serve_documents` is set, and are
/// otherwise fetched from the hosts themselves.
async fn spawn_with(hosts: &str, serve_documents: bool) -> Harness {
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
    state.public_origin = Some(ORIGIN.to_string());
    let listed = ClientMetadataHosts::parse(hosts).unwrap();
    state.oauth_client_metadata_hosts = if serve_documents {
        listed.with_fetch_origin_override(document_server().await)
    } else {
        listed
    };
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        store,
        pool,
        _dir: dir,
    }
}

async fn spawn() -> Harness {
    spawn_with("docs.client.example", true).await
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
    let external_id = format!("ext-{name}");
    let member = h
        .store
        .scim_provision_audited(
            NewMember {
                workspace_id: ws.id,
                handle: format!("{name}-person"),
                display_name: None,
                kind: MemberKind::Human,
            },
            Some(external_id.as_str()),
            true,
            Box::new(move |(m, _)| NewAuditEvent {
                scope: AuditScope::Workspace(m.workspace_id),
                actor_id: None,
                action: "scim.user.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(m.id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap()
        .0;
    let bearer = token_for(h, ws.id, member.id).await;
    Tenant {
        ws: ws.id,
        member: member.id,
        bearer,
    }
}

async fn token_for(h: &Harness, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec!["workspace:read".into(), "workspace:write".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

/// Register a client allowed `workspace:read` and `workspace:write`; with
/// `secret`, a confidential one.
async fn register(h: &Harness, client_id: &str, name: &str, secret: Option<&str>) {
    h.store
        .create_oauth_client(NewOAuthClient {
            client_id: client_id.into(),
            name: name.into(),
            redirect_uris: vec![REDIRECT.into()],
            client_secret_hash: secret.map(hash_secret),
            allowed_scopes: vec!["workspace:read".into(), "workspace:write".into()],
        })
        .await
        .unwrap();
}

/// The cookie of a browser session for `member`, as the OIDC callback makes
/// one: no token behind it.
async fn session_cookie(h: &Harness, ws: WorkspaceId, member: MemberId) -> String {
    let session = h
        .store
        .create_session(NewMaidanSession {
            workspace_id: ws,
            member_id: member,
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

/// A verifier of RFC 7636 length and its S256 challenge.
fn pkce() -> (String, String) {
    let verifier = URL_SAFE_NO_PAD.encode(uuid::Uuid::now_v7().as_bytes())
        + "-hardening-pkce-verifier-long-enough";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn authorize_url(h: &Harness, client_id: &str, challenge: &str, method: &str) -> String {
    format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method={method}&scope=workspace%3Aread&state=xyz&resource={}",
        h.base,
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        urlencoding::encode(challenge),
        urlencoding::encode(RESOURCE),
    )
}

async fn authorize(h: &Harness, t: &Tenant, client_id: &str, challenge: &str) -> reqwest::Response {
    h.client
        .get(authorize_url(h, client_id, challenge, "S256"))
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap()
}

fn location(resp: &reqwest::Response) -> String {
    resp.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

/// The pending request id an authorize redirect names.
fn request_id(resp: &reqwest::Response) -> String {
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    location(resp).split("request=").nth(1).unwrap().to_string()
}

async fn decide(h: &Harness, cookie: &str, request_id: &str) -> reqwest::Response {
    h.client
        .post(format!("{}/ui/api/oauth/consent", h.base))
        .header(header::COOKIE, cookie)
        .header("Sec-Fetch-Site", "same-origin")
        .form(&[("request_id", request_id), ("approved", "true")])
        .send()
        .await
        .unwrap()
}

/// The code a consent redirect hands the client.
fn code_in(resp: &reqwest::Response) -> String {
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let to = location(resp);
    let code = to.split("code=").nth(1).unwrap().split('&').next().unwrap();
    urlencoding::decode(code).unwrap().into_owned()
}

/// Authorize as `t`'s member with `challenge`, consent on their session, and
/// return the code.
async fn code_with(h: &Harness, t: &Tenant, client_id: &str, challenge: &str) -> String {
    let pending = authorize(h, t, client_id, challenge).await;
    let id = request_id(&pending);
    let cookie = session_cookie(h, t.ws, t.member).await;
    code_in(&decide(h, &cookie, &id).await)
}

async fn token_form(h: &Harness, params: &[(&str, &str)]) -> reqwest::Response {
    h.client
        .post(format!("{}/oauth/token", h.base))
        .form(params)
        .send()
        .await
        .unwrap()
}

async fn exchange(h: &Harness, client_id: &str, code: &str, verifier: &str) -> reqwest::Response {
    token_form(
        h,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
            ("resource", RESOURCE),
        ],
    )
    .await
}

async fn refresh(h: &Harness, client_id: &str, refresh_token: &str) -> reqwest::Response {
    token_form(
        h,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("resource", RESOURCE),
        ],
    )
    .await
}

/// The access and refresh token of a 200 token response.
async fn issued(resp: reqwest::Response) -> (String, String) {
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    (
        body["access_token"].as_str().unwrap().to_string(),
        body["refresh_token"].as_str().unwrap().to_string(),
    )
}

async fn oauth_error(resp: reqwest::Response) -> String {
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.unwrap();
    body["error"].as_str().unwrap().to_string()
}

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

#[tokio::test]
async fn authorize_refuses_plain_pkce_and_any_challenge_no_s256_verifier_can_meet() {
    let h = spawn().await;
    let t = tenant(&h, "pkce").await;
    register(&h, "web-client", "Web Client", None).await;
    let (_, challenge) = pkce();

    // `plain` and any other method, even with a well-formed challenge.
    for method in ["plain", "s256", "S512"] {
        let resp = h
            .client
            .get(authorize_url(&h, "web-client", &challenge, method))
            .bearer_auth(&t.bearer)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{method}");
    }
    // A challenge that is not an S256 digest: a plain verifier passed off as
    // one, too short, padded, or empty.
    let padded = format!("{}=", &challenge[..42]);
    for bad in [
        "short",
        "a-verifier-sent-as-its-own-challenge-of-plain-length",
        padded.as_str(),
        "",
    ] {
        let resp = authorize(&h, &t, "web-client", bad).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{bad:?}");
    }
    // No challenge at all.
    let missing = h
        .client
        .get(authorize_url(&h, "web-client", &challenge, "S256").replace(
            &format!("&code_challenge={}", urlencoding::encode(&challenge)),
            "",
        ))
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let ok = authorize(&h, &t, "web-client", &challenge).await;
    assert_eq!(ok.status(), StatusCode::TEMPORARY_REDIRECT);
}

#[tokio::test]
async fn a_verifier_too_short_to_be_unguessable_is_refused_even_when_its_challenge_matches() {
    let h = spawn().await;
    let t = tenant(&h, "short-verifier").await;
    register(&h, "web-client", "Web Client", None).await;
    // The client hashed a five-character verifier: its challenge is a real
    // S256 digest, and anyone who sees the authorize URL can find the
    // verifier from it by trying every short string.
    let weak = "short";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(weak.as_bytes()));
    let code = code_with(&h, &t, "web-client", &challenge).await;
    let refused = exchange(&h, "web-client", &code, weak).await;
    assert_eq!(oauth_error(refused).await, "invalid_grant");
    // Over the upper bound, or outside the unreserved alphabet, the same.
    for bad in ["v".repeat(129), format!("{}+", "v".repeat(50))] {
        let refused = exchange(&h, "web-client", &code, &bad).await;
        assert_eq!(oauth_error(refused).await, "invalid_grant", "{bad}");
    }
}

#[tokio::test]
async fn a_confidential_client_must_prove_pkce_as_well_as_its_secret() {
    let h = spawn().await;
    let t = tenant(&h, "confidential").await;
    register(&h, "server-client", "Server Client", Some("server-secret")).await;
    let (verifier, challenge) = pkce();
    let code = code_with(&h, &t, "server-client", &challenge).await;
    let with_secret = |params: Vec<(&'static str, String)>| {
        let mut form: Vec<(&str, String)> = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.clone()),
            ("redirect_uri", REDIRECT.to_string()),
            ("resource", RESOURCE.to_string()),
        ];
        form.extend(params);
        h.client
            .post(format!("{}/oauth/token", h.base))
            .basic_auth("server-client", Some("server-secret"))
            .form(&form)
            .send()
    };
    // The secret alone is not enough: no verifier, or a wrong one.
    let none = with_secret(vec![]).await.unwrap();
    assert_eq!(oauth_error(none).await, "invalid_request");
    let wrong = with_secret(vec![("code_verifier", "w".repeat(43))])
        .await
        .unwrap();
    assert_eq!(oauth_error(wrong).await, "invalid_grant");
    let right = with_secret(vec![("code_verifier", verifier)])
        .await
        .unwrap();
    assert_eq!(right.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_agent_members_session_never_consents_and_a_person_beside_it_does() {
    let h = spawn().await;
    let t = tenant(&h, "agent-seat").await;
    let other = tenant(&h, "other-workspace").await;
    register(&h, "web-client", "Web Client", None).await;
    let agent = h
        .store
        .create_member(NewMember {
            workspace_id: t.ws,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    // The agent holds a session no token made, the kind only a person's
    // sign-in should produce. Authorize goes through; consent does not.
    let agent_cookie = session_cookie(&h, t.ws, agent.id).await;
    let (_, challenge) = pkce();
    let pending = h
        .client
        .get(authorize_url(&h, "web-client", &challenge, "S256"))
        .header(header::COOKIE, &agent_cookie)
        .send()
        .await
        .unwrap();
    let id = request_id(&pending);
    let page = h
        .client
        .get(format!("{}/ui/oauth/consent?request={id}", h.base))
        .header(header::COOKIE, &agent_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::FORBIDDEN);
    let refused = decide(&h, &agent_cookie, &id).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(h
        .store
        .list_oauth_grants(t.ws, agent.id)
        .await
        .unwrap()
        .is_empty());

    // A session made from a token proves no more than the token, so the
    // person's own token cannot stand in for their sign-in.
    let token = h
        .store
        .create_api_token(NewApiToken {
            workspace_id: t.ws,
            member_id: t.member,
            app_installation_id: None,
            token_hash: hash_secret(TokenSecret::generate().as_str()),
            label: None,
            capabilities: vec!["workspace:read".into(), "workspace:write".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let token_session = h
        .store
        .create_session(NewMaidanSession {
            workspace_id: t.ws,
            member_id: t.member,
            api_token_id: Some(token.id),
            oidc_identity_id: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    maidan_server::session::set_session_cookie(
        &mut headers,
        token_session.id,
        3600,
        false,
        SESSION_SECRET,
    )
    .unwrap();
    let token_cookie = headers[axum::http::header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let person_request = request_id(&authorize(&h, &t, "web-client", &challenge).await);
    let by_token = decide(&h, &token_cookie, &person_request).await;
    assert_eq!(by_token.status(), StatusCode::FORBIDDEN);

    // A removed member's lingering session is refused the same way.
    let removed_cookie = session_cookie(&h, other.ws, other.member).await;
    let removed_request = request_id(&authorize(&h, &other, "web-client", &challenge).await);
    sqlx::query("UPDATE maidan_members SET tombstoned_at = ? WHERE id = ?")
        .bind(chrono::Utc::now())
        .bind(other.member.0)
        .execute(&h.pool)
        .await
        .unwrap();
    // A removed member's session is no session at all: the session layer
    // refuses it before consent is reached.
    let removed = decide(&h, &removed_cookie, &removed_request).await;
    assert_eq!(removed.status(), StatusCode::UNAUTHORIZED);

    // The person in the same workspace consents as before.
    let code = code_with(&h, &t, "web-client", &challenge).await;
    assert!(!code.is_empty());
}

#[tokio::test]
async fn consent_takes_a_same_origin_post_on_a_page_that_cannot_be_framed() {
    let h = spawn().await;
    let t = tenant(&h, "csrf").await;
    // A pre-registered client whose name is markup: shown, never run.
    register(&h, "web-client", "<img src=x onerror=alert(1)>", None).await;
    let (_, challenge) = pkce();
    let id = request_id(&authorize(&h, &t, "web-client", &challenge).await);
    let cookie = session_cookie(&h, t.ws, t.member).await;

    let page = h
        .client
        .get(format!("{}/ui/oauth/consent?request={id}", h.base))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let csp = page.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(
        csp.contains("form-action 'self' https://client.example"),
        "{csp}"
    );
    let html = page.text().await.unwrap();
    assert!(
        html.contains("&lt;img src=x onerror=alert(1)&gt;"),
        "{html}"
    );
    assert!(!html.contains("<img src=x"), "{html}");
    assert!(html.contains("client.example"), "the redirect host: {html}");

    // A post from another origin, or one that names no origin, is refused
    // and leaves the request for the real page.
    let cases: [&[(&str, &str)]; 4] = [
        &[("Sec-Fetch-Site", "cross-site")],
        &[("Sec-Fetch-Site", "same-site")],
        &[("Origin", "https://evil.example")],
        &[],
    ];
    for headers in cases {
        let mut post = h
            .client
            .post(format!("{}/ui/api/oauth/consent", h.base))
            .header(header::COOKIE, &cookie)
            .form(&[("request_id", id.as_str()), ("approved", "true")]);
        for (name, value) in headers {
            post = post.header(*name, *value);
        }
        let refused = post.send().await.unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{headers:?}");
    }
    let approved = decide(&h, &cookie, &id).await;
    let to = location(&approved);
    assert!(to.starts_with(REDIRECT), "{to}");
    assert!(to.contains("&state=xyz"), "state is passed through: {to}");
    // Single use: the same decision again finds nothing.
    let again = decide(&h, &cookie, &id).await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_metadata_document_is_this_clients_only_when_its_client_id_is_its_own_url() {
    let h = spawn().await;
    let t = tenant(&h, "cimd-id").await;
    let (_, challenge) = pkce();
    let ok = authorize(&h, &t, DOC, &challenge).await;
    assert_eq!(ok.status(), StatusCode::TEMPORARY_REDIRECT);
    for wrong in [OTHER_ID, NO_ID] {
        let refused = authorize(&h, &t, wrong, &challenge).await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{wrong}");
        let body = refused.text().await.unwrap();
        assert!(body.contains("client_id"), "{wrong}: {body}");
    }
    // Neither document left a row a later request could be served from.
    for wrong in [OTHER_ID, NO_ID] {
        assert!(h
            .store
            .get_oauth_client_by_client_id(wrong)
            .await
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn a_metadata_document_past_the_size_limit_or_with_a_disguised_name_is_refused() {
    let h = spawn().await;
    let t = tenant(&h, "cimd-shape").await;
    let (_, challenge) = pkce();
    let huge = authorize(&h, &t, HUGE, &challenge).await;
    assert_eq!(huge.status(), StatusCode::BAD_REQUEST);
    assert!(huge.text().await.unwrap().contains("larger than"));
    let spoofed = authorize(&h, &t, SPOOFED_NAME, &challenge).await;
    assert_eq!(spoofed.status(), StatusCode::BAD_REQUEST);
    assert!(spoofed.text().await.unwrap().contains("invisible"));
}

#[tokio::test]
async fn a_listed_metadata_host_on_a_private_address_is_never_fetched() {
    // The operator listed hosts that are loopback addresses. Even so, the
    // fetch would reach this machine's own services, so it is refused before
    // any connection: nothing here listens on 443, and without the guard the
    // refusal would be a failed fetch instead.
    let h = spawn_with("127.0.0.1,localhost", false).await;
    let t = tenant(&h, "ssrf").await;
    let (_, challenge) = pkce();
    for client_id in [
        "https://127.0.0.1/oauth/client.json",
        "https://localhost/oauth/client.json",
    ] {
        let refused = authorize(&h, &t, client_id, &challenge).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{client_id}");
        assert!(
            refused
                .text()
                .await
                .unwrap()
                .contains("public network address"),
            "{client_id}"
        );
    }
}

#[tokio::test]
async fn deactivating_a_member_disconnects_their_apps_for_good_and_leaves_another_tenants() {
    let h = spawn().await;
    let a = tenant(&h, "alpha").await;
    let b = tenant(&h, "bravo").await;
    register(&h, "web-client", "Web Client", None).await;
    let mut pairs = Vec::new();
    for t in [&a, &b] {
        let (verifier, challenge) = pkce();
        let code = code_with(&h, t, "web-client", &challenge).await;
        pairs.push(issued(exchange(&h, "web-client", &code, &verifier).await).await);
    }
    let (a_access, a_refresh) = pairs.remove(0);
    let (b_access, b_refresh) = pairs.remove(0);

    let event = |t: &Tenant| NewAuditEvent {
        scope: AuditScope::Workspace(t.ws),
        actor_id: None,
        action: "scim.user.update".into(),
        target_kind: Some("member".into()),
        target_id: Some(t.member.0),
        metadata: json!({}),
    };
    h.store
        .scim_update_user_audited(a.ws, a.member, None, Some("ext-alpha"), false, event(&a))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        mcp_initialize(&h, &a_access).await,
        StatusCode::UNAUTHORIZED
    );
    // The refresh token would have minted a new access token while the
    // member was out, and kept the app connected after they came back.
    let refused = refresh(&h, "web-client", &a_refresh).await;
    assert_eq!(oauth_error(refused).await, "invalid_grant");
    h.store
        .scim_update_user_audited(a.ws, a.member, None, Some("ext-alpha"), true, event(&a))
        .await
        .unwrap()
        .unwrap();
    let still_refused = refresh(&h, "web-client", &a_refresh).await;
    assert_eq!(oauth_error(still_refused).await, "invalid_grant");
    assert!(h
        .store
        .list_oauth_grants(a.ws, a.member)
        .await
        .unwrap()
        .is_empty());
    let revocations = h
        .store
        .list_audit_for_workspace(a.ws, 200)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.action == "oauth_lineage.revoke" && e.metadata["reason"] == "scim_deactivate")
        .count();
    assert_eq!(revocations, 1);

    // The other tenant's member, and their app, are untouched.
    assert_eq!(mcp_initialize(&h, &b_access).await, StatusCode::OK);
    let (_, rotated) = issued(refresh(&h, "web-client", &b_refresh).await).await;
    assert!(!rotated.is_empty());
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn the_redirect_that_hands_over_a_code_never_reaches_the_trace() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(captured.clone())
        .finish();
    // The server, the document server and the client all run on this
    // thread (a current-thread runtime), so a thread-local subscriber sees
    // every span and event the server makes.
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = spawn().await;
    let t = tenant(&h, "trace").await;
    register(&h, "web-client", "Web Client", None).await;
    let (verifier, challenge) = pkce();
    let code = code_with(&h, &t, "web-client", &challenge).await;
    let (access, refresh_token) = issued(exchange(&h, "web-client", &code, &verifier).await).await;

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("path=/ui/api/oauth/consent"),
        "no request span for the consent decision in: {log}"
    );
    assert!(log.contains("\"location\": Sensitive"), "{log}");
    for secret in [
        code.as_str(),
        verifier.as_str(),
        access.as_str(),
        refresh_token.as_str(),
    ] {
        assert!(!log.contains(secret), "{secret} leaked into the trace");
    }
}
