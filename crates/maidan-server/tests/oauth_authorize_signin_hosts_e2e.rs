//! The two things a person connecting an MCP client to a hosted workspace
//! hits first (`docs/OAuth.md`):
//!
//! - Client metadata documents are fetched only from the hosts the instance
//!   lists in `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`. A workspace fresh from
//!   sign-up, with no egress rows at all, resolves a listed host's client; an
//!   unlisted host is refused before anything is fetched; a document that
//!   answers with a redirect is refused and the redirect is not followed.
//! - A signed-out browser at `GET /oauth/authorize` is sent to sign in with
//!   `return_to` set to the exact authorize request, comes back to it signed
//!   in, and reaches consent. A bad bearer still gets a 401, and a
//!   `return_to` that could leave the origin falls back to the console.
//!
//! Two tenants throughout: the host list is the instance's, so both resolve
//! the same client, and neither needed any row of its own for it.

// The client's document server is a plain axum server, not the API (see
// clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::sync::{
    atomic::{AtomicI64, AtomicUsize, Ordering},
    Arc,
};

use axum::{response::IntoResponse, routing::get, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{
    oauth::registry::ClientMetadataHosts,
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMember, NewOAuthClient, NewOidcIdentity, NewWorkspace,
    WorkspaceId,
};
use reqwest::{header, redirect::Policy, StatusCode};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;

const ORIGIN: &str = "https://maidan.example.com";
const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";
const REDIRECT: &str = "https://client.example/callback";
const SECRET: &[u8] = b"oauth-authorize-signin-hosts-e2e-secret!!";
const ISSUER: &str = "https://mock.idp.local";
/// The listed host, and a document on it.
const LISTED: &str = "https://docs.client.example/oauth/client.json";
/// A document on the listed host that answers with a redirect.
const REDIRECTING: &str = "https://docs.client.example/oauth/moved.json";
/// A host the instance does not list.
const UNLISTED: &str = "https://other.example/oauth/client.json";
/// A client the operator registered, for the sign-in round trip.
const PREREGISTERED: &str = "preregistered-app";

struct Harness {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    /// Requests the document server has answered, any path.
    fetches: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

/// The document server: `/oauth/client.json` is a valid document,
/// `/oauth/moved.json` redirects to it.
async fn document_server(fetches: Arc<AtomicUsize>) -> String {
    let doc_hits = fetches.clone();
    let moved_hits = fetches;
    let app = Router::new()
        .route(
            "/oauth/client.json",
            get(move || {
                doc_hits.fetch_add(1, Ordering::SeqCst);
                async {
                    axum::Json(serde_json::json!({
                        "client_name": "Document Client",
                        "redirect_uris": [REDIRECT],
                        "scope": "workspace:read",
                    }))
                }
            }),
        )
        .route(
            "/oauth/moved.json",
            get(move || {
                moved_hits.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        StatusCode::FOUND,
                        [(header::LOCATION, "/oauth/client.json")],
                    )
                        .into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn spawn() -> Harness {
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
    state.public_origin = Some(ORIGIN.to_string());
    let fetches = Arc::new(AtomicUsize::new(0));
    let docs = document_server(fetches.clone()).await;
    state.oauth_client_metadata_hosts = ClientMetadataHosts::parse("docs.client.example")
        .unwrap()
        .with_fetch_origin_override(docs);
    state.oidc = Some(Arc::new(OidcRuntime {
        settings: OidcSettings {
            enabled: true,
            mock: true,
            issuer: ISSUER.into(),
            redirect_uri: "http://127.0.0.1/auth/oidc/callback".into(),
            auto_provision: false,
            link_email: false,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
            post_logout_redirect_uri: None,
            first_admin_mint: true,
            auto_mint: false,
            signup: false,
            signup_email_domains: Vec::new(),
            signup_open: false,
            emails_verified: false,
            privacy_url: None,
            terms_url: None,
        },
        session_secret: Arc::from(SECRET),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .unwrap(),
        store,
        fetches,
        _dir: dir,
    }
}

struct Tenant {
    ws: WorkspaceId,
    member: MemberId,
    bearer: String,
}

/// A fresh workspace: one member and a token, and no egress rows.
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
            capabilities: vec!["workspace:read".into(), "workspace:write".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    assert!(
        h.store.list_egress_targets(ws.id).await.unwrap().is_empty(),
        "a fresh workspace has no egress rows"
    );
    Tenant {
        ws: ws.id,
        member: member.id,
        bearer: secret.as_str().to_string(),
    }
}

/// The authorize request's path and query, as a client sends the browser.
fn authorize_path(client_id: &str) -> String {
    format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&scope=workspace%3Aread&state=xyz&resource={}",
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier")),
        urlencoding::encode(RESOURCE),
    )
}

fn location(resp: &reqwest::Response) -> String {
    resp.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

fn session_cookie(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| s.split(';').next())
        .find(|p| p.starts_with("maidan_session=") && p.len() > "maidan_session=".len())
        .map(str::to_string)
}

impl Harness {
    async fn authorize_as(&self, t: &Tenant, client_id: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{}", self.base, authorize_path(client_id)))
            .bearer_auth(&t.bearer)
            .send()
            .await
            .unwrap()
    }

    /// Follow a sign-in redirect through the mock provider as `mock-user`:
    /// the callback's response.
    async fn sign_in_at(&self, login_location: &str) -> reqwest::Response {
        let login = self
            .client
            .get(format!("{}{login_location}", self.base))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
        self.client
            .get(format!("{}{}", self.base, location(&login)))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_listed_host_resolves_for_every_fresh_workspace_and_nothing_else_is_fetched() {
    let h = spawn().await;
    let a = tenant(&h, "alpha").await;
    let b = tenant(&h, "beta").await;

    // A listed host's document resolves for a workspace with no rows of its
    // own, and for a second one the same way: the list is the instance's.
    for t in [&a, &b] {
        let resp = h.authorize_as(t, LISTED).await;
        assert_eq!(
            resp.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "{:?}",
            resp.text().await
        );
        assert!(location(&resp).starts_with("/ui/oauth/consent?request="));
    }
    assert_eq!(h.fetches.load(Ordering::SeqCst), 2);

    // An unlisted host is refused before any fetch.
    let before = h.fetches.load(Ordering::SeqCst);
    let refused = h.authorize_as(&a, UNLISTED).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(refused
        .text()
        .await
        .unwrap()
        .contains("MAIDAN_OAUTH_CLIENT_METADATA_HOSTS"));
    // The same host on another port is not the listed host.
    let port = h
        .authorize_as(&a, "https://docs.client.example:8443/oauth/client.json")
        .await;
    assert_eq!(port.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        h.fetches.load(Ordering::SeqCst),
        before,
        "nothing was fetched"
    );

    // A document that redirects is refused, and the redirect is not
    // followed: the server answered once, at the redirecting path.
    let redirected = h.authorize_as(&b, REDIRECTING).await;
    assert_eq!(redirected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(h.fetches.load(Ordering::SeqCst), before + 1);
}

#[tokio::test]
async fn a_signed_out_browser_signs_in_and_comes_back_to_consent() {
    let h = spawn().await;
    let a = tenant(&h, "alpha").await;
    let b = tenant(&h, "beta").await;
    h.store
        .create_oauth_client(NewOAuthClient {
            client_id: PREREGISTERED.into(),
            name: "Registered App".into(),
            redirect_uris: vec![REDIRECT.into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into()],
        })
        .await
        .unwrap();
    // The mock provider signs in as `mock-user`, whose identity is in
    // alpha only.
    h.store
        .upsert_oidc_identity(NewOidcIdentity {
            workspace_id: a.ws,
            issuer: ISSUER.into(),
            subject: "mock-user".into(),
            member_id: a.member,
            email: None,
        })
        .await
        .unwrap();

    // Signed out: 302 to sign-in, `return_to` the exact authorize request.
    let path = authorize_path(PREREGISTERED);
    let signed_out = h
        .client
        .get(format!("{}{path}", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(signed_out.status(), StatusCode::FOUND);
    let login = location(&signed_out);
    assert_eq!(
        login,
        format!("/auth/oidc/login?return_to={}", urlencoding::encode(&path))
    );
    let return_to = login.split_once("return_to=").unwrap().1;
    assert_eq!(urlencoding::decode(return_to).unwrap(), path);

    // Sign in at the mock provider: the callback sends the browser back to
    // the same authorize request, now with a session.
    let callback = h.sign_in_at(&login).await;
    assert!(callback.status().is_redirection(), "{}", callback.status());
    assert_eq!(location(&callback), path);
    let cookie = session_cookie(&callback).expect("the sign-in sets a session");

    // Back at authorize, signed in: on to consent, for alpha's member.
    let back = h
        .client
        .get(format!("{}{path}", h.base))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(
        back.status(),
        StatusCode::TEMPORARY_REDIRECT,
        "{:?}",
        back.text().await
    );
    let consent_at = location(&back);
    assert!(consent_at.starts_with("/ui/oauth/consent?request="));
    let request_id = consent_at.split("request=").nth(1).unwrap();
    let pending = h
        .store
        .get_oauth_pending_request(maidan_types::OAuthPendingRequestId(
            request_id.parse().unwrap(),
        ))
        .await
        .unwrap()
        .expect("the pending request");
    assert_eq!(pending.workspace_id, a.ws);
    assert_eq!(pending.member_id, a.member);
    assert_ne!(pending.workspace_id, b.ws);
    let page = h
        .client
        .get(format!("{}{consent_at}", h.base))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK, "{:?}", page.text().await);
}

#[tokio::test]
async fn a_request_with_a_credential_is_never_redirected() {
    let h = spawn().await;
    let path = authorize_path(PREREGISTERED);
    for value in ["Bearer not-a-real-token", "Basic Zm9vOmJhcg=="] {
        let resp = h
            .client
            .get(format!("{}{path}", h.base))
            .header(header::AUTHORIZATION, value)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{value}");
        assert!(resp.headers().get(header::LOCATION).is_none());
    }
    // A signed-out GET elsewhere keeps its 401: only authorize redirects.
    let other = h
        .client
        .get(format!("{}/members/{}", h.base, uuid::Uuid::now_v7()))
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_return_to_that_could_leave_the_origin_lands_on_the_console() {
    let h = spawn().await;
    let a = tenant(&h, "alpha").await;
    h.store
        .upsert_oidc_identity(NewOidcIdentity {
            workspace_id: a.ws,
            issuer: ISSUER.into(),
            subject: "mock-user".into(),
            member_id: a.member,
            email: None,
        })
        .await
        .unwrap();
    for evil in [
        "//evil.example/oauth/authorize",
        "https://evil.example/",
        "/\\evil.example",
        "\\\\evil.example",
    ] {
        let login = format!("/auth/oidc/login?return_to={}", urlencoding::encode(evil));
        let callback = h.sign_in_at(&login).await;
        assert!(callback.status().is_redirection(), "{evil}");
        assert_eq!(location(&callback), "/ui/", "{evil}");
    }
}
