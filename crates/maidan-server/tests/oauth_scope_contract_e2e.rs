//! The grant-scope contract: no token the authorization server issues —
//! by code exchange or by refresh — ever carries a capability beyond
//! what the consenting member holds, what the client is allowed, and what
//! is delegatable at all. The bounds are applied at authorize time (the
//! member), at consent (the pending carries them), and again at the
//! token endpoint (the client, and the grant's own scope on refresh);
//! this suite pins the whole chain end to end so a change to any link
//! fails here rather than widening a grant in production.

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
use reqwest::StatusCode;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;

const ORIGIN: &str = "https://maidan.example.com";
const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";
const REDIRECT: &str = "https://client.example/callback";
const SESSION_SECRET: &[u8] = concat!(
    "0123456789abcdef0123456789abcdef",
    "fedcba9876543210fedcba9876543210"
)
.as_bytes();

struct Harness {
    base: String,
    client: reqwest::Client,
    no_redirect: reqwest::Client,
    store: Arc<dyn Store>,
}

async fn spawn() -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
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
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: SESSION_SECRET.into(),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    state.public_origin = Some(ORIGIN.into());
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

struct Tenant {
    ws: WorkspaceId,
    member: MemberId,
    bearer: String,
}

/// A workspace whose member's own token holds exactly
/// `capabilities` — the member's capability ceiling for this suite.
async fn tenant(h: &Harness, name: &str, capabilities: &[&str]) -> Tenant {
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
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
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

/// A public client allowed read and write, whatever the member holds.
async fn register(h: &Harness, client_id: &str) {
    h.store
        .create_oauth_client(NewOAuthClient {
            client_id: client_id.into(),
            name: client_id.into(),
            redirect_uris: vec![REDIRECT.into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into(), "workspace:write".into()],
        })
        .await
        .unwrap();
}

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

/// A fresh PKCE pair: the verifier and its S256 challenge.
fn pkce() -> (String, String) {
    let verifier = URL_SAFE_NO_PAD.encode(uuid::Uuid::now_v7().as_bytes()) + "-pkce-verifier";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The authorize step alone: the raw response, so a refusal is visible.
/// `scope` is the raw query value; `None` omits the parameter entirely.
/// The request is otherwise fully valid — client, redirect and PKCE all
/// check out — so a refusal here is the scope bound and nothing else.
async fn authorize(
    h: &Harness,
    t: &Tenant,
    client_id: &str,
    scope: Option<&str>,
) -> reqwest::Response {
    let (_, challenge) = pkce();
    let scope_part = scope
        .map(|s| format!("&scope={}", urlencoding::encode(s)))
        .unwrap_or_default();
    let url = format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=xyz&resource={}{scope_part}",
        h.base,
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        urlencoding::encode(RESOURCE),
    );
    h.no_redirect
        .get(&url)
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap()
}

/// An authorization code through authorize and consent, requesting
/// exactly `scope`, plus its PKCE verifier.
async fn code_for(h: &Harness, t: &Tenant, client_id: &str, scope: &str) -> (String, String) {
    let (verifier, challenge) = pkce();
    let url = format!(
        "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state=xyz&resource={}&scope={}",
        h.base,
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        urlencoding::encode(RESOURCE),
        urlencoding::encode(scope),
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

async fn token_form(h: &Harness, params: &[(&str, &str)]) -> reqwest::Response {
    let mut form: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    form.push(("client_id".into(), "scope-contract-client".into()));
    h.client
        .post(format!("{}/oauth/token", h.base))
        .form(&form)
        .send()
        .await
        .unwrap()
}

async fn exchange(h: &Harness, code: &str, verifier: &str) -> reqwest::Response {
    token_form(
        h,
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

/// The (access scope, refresh token) of a 200 token response.
async fn issued(resp: reqwest::Response) -> (String, String) {
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    (
        body["scope"].as_str().unwrap().to_string(),
        body["refresh_token"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn authorize_refuses_a_scope_beyond_the_member() {
    let h = spawn().await;
    // The member holds read only; the client allows read and write.
    let t = tenant(&h, "narrow", &["workspace:read"]).await;
    register(&h, "scope-contract-client").await;

    // Asking for more than the member holds is refused at the door: no
    // pending, no grant, nothing a later step could widen.
    let wide = authorize(
        &h,
        &t,
        "scope-contract-client",
        Some("workspace:read workspace:write"),
    )
    .await;
    assert_eq!(wide.status(), StatusCode::FORBIDDEN);
    // And the default — the client's whole list — is bounded the same way.
    let defaulted = authorize(&h, &t, "scope-contract-client", None).await;
    assert_eq!(defaulted.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn no_grant_carries_more_than_the_member_holds() {
    let h = spawn().await;
    let t = tenant(&h, "bounded", &["workspace:read"]).await;
    register(&h, "scope-contract-client").await;

    let (code, verifier) = code_for(&h, &t, "scope-contract-client", "workspace:read").await;
    let (scope, refresh_token) = issued(exchange(&h, &code, &verifier).await).await;
    assert_eq!(scope, "workspace:read");

    // A refresh may narrow or keep the grant's scope, never widen it past
    // what the member held when they consented.
    let widen = token_form(
        &h,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("resource", RESOURCE),
            ("scope", "workspace:write"),
        ],
    )
    .await;
    assert_eq!(widen.status(), StatusCode::BAD_REQUEST);
    let body: Value = widen.json().await.unwrap();
    assert_eq!(body["error"], "invalid_scope");

    let (scope, _) = issued(
        token_form(
            &h,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("resource", RESOURCE),
            ],
        )
        .await,
    )
    .await;
    assert_eq!(scope, "workspace:read");
}

#[tokio::test]
async fn a_non_delegatable_scope_is_refused_even_when_the_member_holds_it() {
    let h = spawn().await;
    // The member's own token holds token:admin; it is still not a scope a
    // grant may carry — authority capabilities are never delegatable.
    let t = tenant(&h, "admin", &["workspace:read", "token:admin"]).await;
    register(&h, "scope-contract-client").await;
    let resp = authorize(&h, &t, "scope-contract-client", Some("token:admin")).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
