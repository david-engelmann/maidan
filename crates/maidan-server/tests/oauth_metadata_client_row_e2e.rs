//! Metadata-document OAuth clients persist as marked rows (migration
//! 0172): the row anchors the client's codes and grants, and it never
//! makes the client pre-registered — the token endpoint and authorize
//! resolve a marked client from its document, through the egress guard,
//! every time. The document host is never allowlisted here, so a
//! resolution attempt is refused by the guard; that refusal is what
//! proves the row was not served from the cache.

use std::sync::{atomic::AtomicI64, Arc};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMember, NewOAuthAuthorizationCode, NewOAuthClient,
    NewOAuthGrant, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;

const ORIGIN: &str = "https://maidan.example.com";
const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";
const REDIRECT: &str = "https://client.example/callback";
const SESSION_SECRET: &[u8] = b"metadata client row test secret";
/// The metadata-document client: its id is its document's URL.
const META: &str = "https://meta.example/oauth.json";
/// A URL-shaped client id the operator registered directly (unmarked row).
const PREREGISTERED_URL: &str = "https://pre.example/oauth.json";

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
        secret: SESSION_SECRET.into(),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    state.public_origin = Some(ORIGIN.to_string());
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
    Tenant {
        ws: ws.id,
        member: member.id,
        bearer: secret.as_str().to_string(),
    }
}

/// The row a first resolve leaves behind for a metadata-document client.
async fn marked_row(h: &Harness) {
    h.store
        .upsert_oauth_metadata_client(
            NewOAuthClient {
                client_id: META.into(),
                name: "Document App".into(),
                redirect_uris: vec![REDIRECT.into()],
                client_secret_hash: None,
                allowed_scopes: vec!["workspace:read".into()],
            },
            META,
        )
        .await
        .unwrap()
        .expect("the metadata client's row");
}

/// A code for `client_id`, as consent would mint it — grant first, the
/// code carrying the grant's lineage — and its PKCE verifier.
async fn seed_code(h: &Harness, t: &Tenant, client_id: &str) -> (String, String) {
    let code = format!("code-{}", uuid::Uuid::now_v7());
    let verifier = format!("verifier-{}", uuid::Uuid::now_v7());
    let lineage_id = uuid::Uuid::now_v7();
    h.store
        .create_oauth_grant(NewOAuthGrant {
            client_id: client_id.into(),
            member_id: t.member,
            workspace_id: t.ws,
            scope: vec!["workspace:read".into()],
            lineage_id,
        })
        .await
        .unwrap();
    h.store
        .create_oauth_authorization_code(NewOAuthAuthorizationCode {
            code_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(code.as_bytes())),
            client_id: client_id.into(),
            member_id: t.member,
            workspace_id: t.ws,
            redirect_uri: REDIRECT.into(),
            code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            scope: vec!["workspace:read".into()],
            resource: Some(RESOURCE.into()),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            lineage_id: Some(lineage_id),
        })
        .await
        .unwrap();
    (code, verifier)
}

async fn exchange(h: &Harness, client_id: &str, code: &str, verifier: &str) -> reqwest::Response {
    h.client
        .post(format!("{}/oauth/token", h.base))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
            ("resource", RESOURCE),
            ("client_id", client_id),
        ])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_marked_client_is_resolved_from_its_document_never_from_its_row() {
    let h = spawn().await;
    let t = tenant(&h, "marked").await;
    marked_row(&h).await;

    // The row is the foreign-key anchor: the grant and code seed_code
    // records against it would fail their foreign keys if the row were
    // not there — before 0172 there was no row, and they did.

    // At the token endpoint the row buys nothing: resolution goes to
    // the document, the egress guard refuses the unallowlisted host,
    // and the exchange is an invalid_client refusal.
    let (code, verifier) = seed_code(&h, &t, META).await;
    let resp = exchange(&h, META, &code, &verifier).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "invalid_client");
}

#[tokio::test]
async fn an_unmarked_client_with_a_url_id_exchanges_from_its_row() {
    let h = spawn().await;
    let t = tenant(&h, "unmarked").await;
    h.store
        .create_oauth_client(NewOAuthClient {
            client_id: PREREGISTERED_URL.into(),
            name: "Registered URL Client".into(),
            redirect_uris: vec![REDIRECT.into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into()],
        })
        .await
        .unwrap();
    let (code, verifier) = seed_code(&h, &t, PREREGISTERED_URL).await;
    let resp = exchange(&h, PREREGISTERED_URL, &code, &verifier).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "body: {:?}",
        resp.text().await.unwrap()
    );
    let body: Value = resp.json().await.unwrap();
    assert!(body["access_token"].is_string());
}

#[tokio::test]
async fn authorize_refuses_a_marked_client_at_the_egress_guard_but_not_an_unmarked_one() {
    let h = spawn().await;
    let t = tenant(&h, "authorize").await;
    marked_row(&h).await;
    h.store
        .create_oauth_client(NewOAuthClient {
            client_id: PREREGISTERED_URL.into(),
            name: "Registered URL Client".into(),
            redirect_uris: vec![REDIRECT.into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into()],
        })
        .await
        .unwrap();

    let authorize = |client_id: &str| {
        format!(
            "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&scope=workspace%3Aread&state=xyz&resource={}",
            h.base,
            urlencoding::encode(client_id),
            urlencoding::encode(REDIRECT),
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier")),
            urlencoding::encode(RESOURCE),
        )
    };

    // The marked client is not served from its row: resolution reaches
    // the egress guard, which refuses the unallowlisted host.
    let refused = h
        .no_redirect
        .get(authorize(META))
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    // The unmarked client resolves from its row and proceeds to consent.
    let proceeded = h
        .no_redirect
        .get(authorize(PREREGISTERED_URL))
        .bearer_auth(&t.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(proceeded.status(), StatusCode::TEMPORARY_REDIRECT);
}
