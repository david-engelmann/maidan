//! Grant management (Next 23, phase four): a member lists the OAuth
//! grants clients hold on them and revokes one, over the API and in the
//! console. Reads are scoped to the caller's own workspace and member —
//! another tenant's grant is a 404 — and a revoke ends the grant's
//! whole lineage, refresh and access tokens alike.

use std::sync::{atomic::AtomicI64, Arc};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMaidanSession, NewMember, NewOAuthClient, NewOAuthGrant,
    NewOAuthRefreshToken, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;

const SESSION_SECRET: &[u8] = b"grant management test secret";

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
    state.public_origin = Some("https://maidan.example.com".into());
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

/// A workspace with a human member and that member's own bearer token.
struct Tenant {
    ws: WorkspaceId,
    member: MemberId,
    bearer: String,
    token_id: maidan_types::ApiTokenId,
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
    let token = h
        .store
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
        token_id: token.id,
    }
}

async fn register_client(h: &Harness, client_id: &str, name: &str) {
    h.store
        .create_oauth_client(NewOAuthClient {
            client_id: client_id.into(),
            name: name.into(),
            redirect_uris: vec!["https://client.example/callback".into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into()],
        })
        .await
        .unwrap();
}

/// A grant for `member` from `client_id`, with a live access token and
/// refresh token under its lineage. Returns the grant and the refresh
/// token's hash.
async fn grant_with_tokens(
    h: &Harness,
    t: &Tenant,
    client_id: &str,
) -> (maidan_types::OAuthGrant, String) {
    let lineage_id = uuid::Uuid::new_v4();
    let grant = h
        .store
        .create_oauth_grant(NewOAuthGrant {
            client_id: client_id.into(),
            member_id: t.member,
            workspace_id: t.ws,
            scope: vec!["workspace:read".into()],
            lineage_id,
        })
        .await
        .unwrap();
    let access_secret = TokenSecret::generate();
    let refresh_plain = format!("refresh-{}", uuid::Uuid::new_v4());
    let refresh_hash = URL_SAFE_NO_PAD.encode(Sha256::digest(refresh_plain.as_bytes()));
    let member = t.member;
    let ws = t.ws;
    h.store
        .mint_oauth_token_pair_audited(
            NewApiToken {
                workspace_id: ws,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(access_secret.as_str()),
                label: None,
                capabilities: vec!["workspace:read".into()],
                expires_at: None,
            },
            grant.id,
            NewOAuthRefreshToken {
                token_hash: refresh_hash.clone(),
                grant_id: grant.id,
                lineage_id,
                client_id: client_id.into(),
                resource: "https://maidan.example.com/mcp/streamable".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::days(30),
            },
            Box::new(move |_| maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(ws),
                actor_id: None,
                action: "oauth_token.mint".into(),
                target_kind: None,
                target_id: None,
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
    (grant, refresh_hash)
}

async fn grant_ids(h: &Harness, t: &Tenant) -> Vec<Value> {
    let resp = h
        .client
        .get(format!("{}/workspaces/{}/oauth-grants", h.base, t.ws.0))
        .header("Authorization", format!("Bearer {}", t.bearer))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    resp.json::<Vec<Value>>().await.unwrap()
}

/// A signed-in browser session cookie for `member`, as the OIDC callback
/// writes it; with `token_id`, a session a token made.
async fn session_cookie(
    h: &Harness,
    t: &Tenant,
    token_id: Option<maidan_types::ApiTokenId>,
) -> String {
    let session = h
        .store
        .create_session(NewMaidanSession {
            workspace_id: t.ws,
            member_id: t.member,
            api_token_id: token_id,
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
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_member_lists_their_own_grants_and_no_one_elses() {
    let h = spawn().await;
    let a = tenant(&h, "alpha").await;
    let b = tenant(&h, "beta").await;
    register_client(&h, "registered-app", "Registered App").await;
    grant_with_tokens(&h, &a, "registered-app").await;
    // A metadata-document client holds its grant under its URL; its
    // registry row is created on first use at /authorize, and the grant
    // lists with the document's name like any other.
    register_client(&h, "https://apps.example/oauth.json", "Metadata App").await;
    grant_with_tokens(&h, &a, "https://apps.example/oauth.json").await;
    grant_with_tokens(&h, &b, "registered-app").await;

    let grants = grant_ids(&h, &a).await;
    assert_eq!(grants.len(), 2, "alpha sees exactly its two grants");
    let registered = grants
        .iter()
        .find(|g| g["client_id"] == "registered-app")
        .expect("the registered client's grant");
    assert_eq!(registered["client_name"], "Registered App");
    let metadata = grants
        .iter()
        .find(|g| g["client_id"] == "https://apps.example/oauth.json")
        .expect("the metadata client's grant");
    assert_eq!(metadata["client_name"], "Metadata App");

    let b_grants = grant_ids(&h, &b).await;
    assert_eq!(b_grants.len(), 1, "beta sees only its own grant");
    assert_eq!(b_grants[0]["client_id"], "registered-app");

    // Another workspace's list is not readable at all.
    let resp = h
        .client
        .get(format!("{}/workspaces/{}/oauth-grants", h.base, a.ws.0))
        .header("Authorization", format!("Bearer {}", b.bearer))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn another_tenants_grant_is_a_404_and_revoking_your_own_ends_the_lineage() {
    let h = spawn().await;
    let a = tenant(&h, "alpha").await;
    let b = tenant(&h, "beta").await;
    register_client(&h, "registered-app", "Registered App").await;
    let (a_grant, a_refresh) = grant_with_tokens(&h, &a, "registered-app").await;
    let (b_grant, _b_refresh) = grant_with_tokens(&h, &b, "registered-app").await;

    // Alpha addresses beta's grant under alpha's own workspace path: the
    // grant is not alpha's, in a workspace that is not alpha's — a 404,
    // and beta's grant is untouched.
    let resp = h
        .client
        .post(format!(
            "{}/workspaces/{}/oauth-grants/{}/revoke",
            h.base, a.ws.0, b_grant.id.0
        ))
        .header("Authorization", format!("Bearer {}", a.bearer))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(grant_ids(&h, &b).await.len(), 1, "beta's grant survives");

    // A second member of alpha's workspace gets the same 404 for alpha's
    // grant: membership in the workspace is not ownership of the grant.
    let colleague = h
        .store
        .create_member(NewMember {
            workspace_id: a.ws,
            handle: "alpha-colleague".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: a.ws,
            member_id: colleague.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec!["workspace:read".into(), "workspace:write".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let resp = h
        .client
        .post(format!(
            "{}/workspaces/{}/oauth-grants/{}/revoke",
            h.base, a.ws.0, a_grant.id.0
        ))
        .header("Authorization", format!("Bearer {}", secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(grant_ids(&h, &a).await.len(), 1, "alpha's grant survives");

    // The owner revokes: the lineage ends — grant, refresh token and all.
    let resp = h
        .client
        .post(format!(
            "{}/workspaces/{}/oauth-grants/{}/revoke",
            h.base, a.ws.0, a_grant.id.0
        ))
        .header("Authorization", format!("Bearer {}", a.bearer))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["access_tokens_revoked"], 1);
    assert_eq!(body["refresh_tokens_revoked"], 1);
    assert!(
        grant_ids(&h, &a).await.is_empty(),
        "the grant list is empty"
    );
    let refresh = h
        .store
        .get_oauth_refresh_token(&a_refresh)
        .await
        .unwrap()
        .expect("the refresh token row remains");
    assert!(refresh.revoked_at.is_some(), "the refresh token is revoked");
    let revocations: Vec<_> = h
        .store
        .list_audit_for_workspace(a.ws, 200)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| {
            e.action == "oauth_lineage.revoke" && e.metadata["reason"] == "member_revocation"
        })
        .collect();
    assert_eq!(revocations.len(), 1, "one audited lineage revocation");
    assert_eq!(revocations[0].actor_id, Some(a.member));
}

#[tokio::test]
async fn the_console_lists_and_revokes_and_refuses_token_sessions() {
    let h = spawn().await;
    let a = tenant(&h, "alpha").await;
    let b = tenant(&h, "beta").await;
    register_client(&h, "registered-app", "Registered App").await;
    let (a_grant, _r) = grant_with_tokens(&h, &a, "registered-app").await;
    grant_with_tokens(&h, &b, "registered-app").await;

    let cookie = session_cookie(&h, &a, None).await;
    let resp = h
        .client
        .get(format!("{}/ui/oauth/grants", h.base))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let page = resp.text().await.unwrap();
    assert!(page.contains("Registered App"), "the page names the client");
    assert!(
        page.contains(&a_grant.id.0.to_string()),
        "the page carries alpha's grant"
    );
    assert_eq!(
        page.matches("Revoke").count(),
        1,
        "only alpha's grant is offered"
    );

    // A session a token made is refused: no person owns that consent.
    let token_cookie = session_cookie(&h, &a, Some(a.token_id)).await;
    let resp = h
        .client
        .get(format!("{}/ui/oauth/grants", h.base))
        .header("Cookie", &token_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The revoke POST is same-origin and ends the grant.
    let resp = h
        .no_redirect
        .post(format!(
            "{}/ui/api/oauth/grants/{}/revoke",
            h.base, a_grant.id.0
        ))
        .header("Cookie", &cookie)
        .header("Sec-Fetch-Site", "same-origin")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert!(grant_ids(&h, &a).await.is_empty(), "the grant is gone");
    assert_eq!(
        grant_ids(&h, &b).await.len(),
        1,
        "beta's grant is untouched"
    );
}
