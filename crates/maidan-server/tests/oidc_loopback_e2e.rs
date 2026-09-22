//! Full OIDC authorization-code flow against a real loopback provider.
//! Unlike `oidc_e2e`, this traverses discovery, PKCE, token exchange, ES256
//! JWKS verification, nonce/audience/issuer validation, provisioning, session,
//! and RP-initiated logout without `MAIDAN_OIDC_MOCK`.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc,
    },
};

use axum::{
    extract::{Form, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, NewWorkspace};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use reqwest::{redirect::Policy, StatusCode as ClientStatus};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::Mutex;
use uuid::Uuid;

const CLIENT_ID: &str = "maidan-loopback-test";
const SESSION_SECRET: &str = "loopback-oidc-session-secret-32bytes";
const KEY_ID: &str = "loopback-es256";

#[derive(Clone, Copy, Default)]
enum TokenMode {
    #[default]
    Valid,
    BadNonce,
    BadSignature,
    BadAudience,
    BadIssuer,
}

struct AuthorizationGrant {
    client_id: String,
    redirect_uri: String,
    nonce: String,
    code_challenge: String,
}

struct ProviderState {
    issuer: String,
    signing_key: SigningKey,
    bad_signing_key: SigningKey,
    grants: Mutex<HashMap<String, AuthorizationGrant>>,
    next_mode: Mutex<TokenMode>,
    bad_state_next: AtomicBool,
    discovery_seen: AtomicBool,
    authorize_seen: AtomicBool,
    token_seen: AtomicBool,
    jwks_seen: AtomicBool,
    logout_seen: AtomicBool,
}

#[derive(Deserialize)]
struct AuthorizationQuery {
    client_id: String,
    redirect_uri: String,
    response_type: String,
    state: String,
    nonce: String,
    code_challenge: String,
    code_challenge_method: String,
}

#[derive(Deserialize)]
struct TokenForm {
    grant_type: String,
    code: String,
    redirect_uri: String,
    client_id: Option<String>,
    code_verifier: String,
}

#[derive(Deserialize)]
struct LogoutQuery {
    client_id: String,
    post_logout_redirect_uri: String,
}

async fn discovery(State(state): State<Arc<ProviderState>>) -> Json<Value> {
    state.discovery_seen.store(true, Ordering::SeqCst);
    Json(json!({
        "issuer": state.issuer,
        "authorization_endpoint": format!("{}/authorize", state.issuer),
        "token_endpoint": format!("{}/token", state.issuer),
        "jwks_uri": format!("{}/jwks", state.issuer),
        "end_session_endpoint": format!("{}/logout", state.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["ES256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["openid", "profile", "email"],
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "nonce", "email", "email_verified"]
    }))
}

async fn jwks(State(state): State<Arc<ProviderState>>) -> Json<Value> {
    state.jwks_seen.store(true, Ordering::SeqCst);
    let point = state.signing_key.verifying_key().to_encoded_point(false);
    Json(json!({
        "keys": [{
            "kty": "EC",
            "use": "sig",
            "kid": KEY_ID,
            "alg": "ES256",
            "crv": "P-256",
            "x": URL_SAFE_NO_PAD.encode(point.x().expect("uncompressed x coordinate")),
            "y": URL_SAFE_NO_PAD.encode(point.y().expect("uncompressed y coordinate"))
        }]
    }))
}

async fn authorize(
    State(provider): State<Arc<ProviderState>>,
    Query(query): Query<AuthorizationQuery>,
) -> Redirect {
    provider.authorize_seen.store(true, Ordering::SeqCst);
    assert_eq!(query.client_id, CLIENT_ID);
    assert_eq!(query.response_type, "code");
    assert_eq!(query.code_challenge_method, "S256");
    assert!(!query.state.is_empty());
    assert!(!query.nonce.is_empty());
    assert!(!query.code_challenge.is_empty());

    let code = format!("code-{}", Uuid::new_v4());
    provider.grants.lock().await.insert(
        code.clone(),
        AuthorizationGrant {
            client_id: query.client_id,
            redirect_uri: query.redirect_uri.clone(),
            nonce: query.nonce,
            code_challenge: query.code_challenge,
        },
    );
    let mut callback = reqwest::Url::parse(&query.redirect_uri).expect("valid redirect URI");
    let returned_state = if provider.bad_state_next.swap(false, Ordering::SeqCst) {
        "unknown-state"
    } else {
        &query.state
    };
    callback
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", returned_state);
    Redirect::temporary(callback.as_str())
}

fn id_token(provider: &ProviderState, grant: &AuthorizationGrant, mode: TokenMode) -> String {
    let now = Utc::now().timestamp();
    let issuer = match mode {
        TokenMode::BadIssuer => format!("{}/wrong", provider.issuer),
        _ => provider.issuer.clone(),
    };
    let audience = match mode {
        TokenMode::BadAudience => "some-other-client",
        _ => grant.client_id.as_str(),
    };
    let nonce = match mode {
        TokenMode::BadNonce => "wrong-nonce",
        _ => grant.nonce.as_str(),
    };
    let header = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({"alg": "ES256", "kid": KEY_ID, "typ": "JWT"}))
            .expect("serialize JWT header"),
    );
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "iss": issuer,
            "sub": "loopback-user",
            "aud": audience,
            "exp": now + 300,
            "iat": now,
            "nonce": nonce,
            "email": "loopback@example.com",
            "email_verified": true
        }))
        .expect("serialize ID-token claims"),
    );
    let signing_input = format!("{header}.{claims}");
    let key = match mode {
        TokenMode::BadSignature => &provider.bad_signing_key,
        _ => &provider.signing_key,
    };
    let signature: Signature = key.sign(signing_input.as_bytes());
    format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    )
}

async fn token(
    State(provider): State<Arc<ProviderState>>,
    Form(form): Form<TokenForm>,
) -> Result<Json<Value>, (StatusCode, String)> {
    provider.token_seen.store(true, Ordering::SeqCst);
    if form.grant_type != "authorization_code" {
        return Err((StatusCode::BAD_REQUEST, "wrong grant type".into()));
    }
    let grant = provider
        .grants
        .lock()
        .await
        .remove(&form.code)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "unknown or reused code".into()))?;
    if form.redirect_uri != grant.redirect_uri
        || form.client_id.as_deref() != Some(grant.client_id.as_str())
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "client or redirect mismatch".into(),
        ));
    }
    let actual_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(form.code_verifier.as_bytes()));
    if actual_challenge != grant.code_challenge {
        return Err((StatusCode::BAD_REQUEST, "PKCE verification failed".into()));
    }
    let mode = std::mem::take(&mut *provider.next_mode.lock().await);
    Ok(Json(json!({
        "access_token": "loopback-access-token",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": id_token(provider.as_ref(), &grant, mode)
    })))
}

async fn provider_logout(
    State(provider): State<Arc<ProviderState>>,
    Query(query): Query<LogoutQuery>,
) -> Redirect {
    provider.logout_seen.store(true, Ordering::SeqCst);
    assert_eq!(query.client_id, CLIENT_ID);
    Redirect::temporary(&query.post_logout_redirect_uri)
}

struct EnvGuard(Vec<(&'static str, Option<String>)>);

impl EnvGuard {
    fn set(values: &[(&'static str, Option<&str>)]) -> Self {
        let previous = values
            .iter()
            .map(|(name, _)| (*name, std::env::var(name).ok()))
            .collect();
        for (name, value) in values {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        Self(previous)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in self.0.drain(..) {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

async fn follow_login(
    client: &reqwest::Client,
    base: &str,
    workspace_id: Uuid,
) -> reqwest::Response {
    let login = client
        .get(format!(
            "{base}/auth/oidc/login?workspace_id={workspace_id}"
        ))
        .send()
        .await
        .expect("start OIDC login");
    assert_eq!(login.status(), ClientStatus::TEMPORARY_REDIRECT);
    let authorization_url = login
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("authorization redirect")
        .to_str()
        .expect("authorization URL text");
    let authorization = client
        .get(authorization_url)
        .send()
        .await
        .expect("loopback authorization request");
    assert_eq!(authorization.status(), ClientStatus::TEMPORARY_REDIRECT);
    let callback_url = authorization
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("callback redirect")
        .to_str()
        .expect("callback URL text");
    client
        .get(callback_url)
        .send()
        .await
        .expect("Maidan OIDC callback")
}

#[tokio::test]
async fn real_loopback_oidc_checks_validation_provisioning_session_and_logout() {
    let idp_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback IdP");
    let idp_addr = idp_listener.local_addr().expect("IdP address");
    let issuer = format!("http://{idp_addr}");
    let provider = Arc::new(ProviderState {
        issuer: issuer.clone(),
        signing_key: SigningKey::from_slice(&[0x41; 32]).expect("fixture signing key"),
        bad_signing_key: SigningKey::from_slice(&[0x42; 32]).expect("bad fixture key"),
        grants: Mutex::new(HashMap::new()),
        next_mode: Mutex::new(TokenMode::Valid),
        bad_state_next: AtomicBool::new(false),
        discovery_seen: AtomicBool::new(false),
        authorize_seen: AtomicBool::new(false),
        token_seen: AtomicBool::new(false),
        jwks_seen: AtomicBool::new(false),
        logout_seen: AtomicBool::new(false),
    });
    let idp_app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/jwks", get(jwks))
        .route("/logout", get(provider_logout))
        .with_state(provider.clone());
    let idp_server = tokio::spawn(async move {
        axum::serve(idp_listener, idp_app)
            .await
            .expect("serve loopback IdP");
    });

    let maidan_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Maidan");
    let maidan_addr: SocketAddr = maidan_listener.local_addr().expect("Maidan address");
    let base = format!("http://{maidan_addr}");
    let redirect_uri = format!("{base}/auth/oidc/callback");
    let signed_out_uri = format!("{base}/signed-out");
    let _env = EnvGuard::set(&[
        ("MAIDAN_SESSION_SECRET", Some(SESSION_SECRET)),
        ("MAIDAN_OIDC_CLIENT_ID", Some(CLIENT_ID)),
        ("MAIDAN_OIDC_CLIENT_SECRET", None),
    ]);
    let oidc = OidcRuntime::init(OidcSettings {
        enabled: true,
        mock: false,
        issuer: issuer.clone(),
        redirect_uri,
        auto_provision: true,
        link_email: false,
        session_ttl_secs: 3600,
        pending_ttl_secs: 600,
        cookie_secure: false,
        post_logout_redirect_uri: Some(signed_out_uri.clone()),
        first_admin_mint: true,
        auto_mint: false,
    })
    .await
    .expect("discover loopback provider");

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    run_sqlite_migrations(&pool).await.expect("migrate sqlite");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "loopback-oidc".into(),
        })
        .await
        .expect("create workspace");
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifact_dir = tempfile::tempdir().expect("artifact tempdir");
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(artifact_dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.oidc = Some(Arc::new(oidc));
    let maidan_server = tokio::spawn(async move {
        axum::serve(maidan_listener, router(state))
            .await
            .expect("serve Maidan");
    });
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .expect("HTTP client");

    provider.bad_state_next.store(true, Ordering::SeqCst);
    let bad_state = follow_login(&client, &base, workspace.id.0).await;
    assert!(bad_state.status().is_client_error());
    assert!(bad_state
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .all(|value| !value.as_bytes().starts_with(b"maidan_session=")));

    for (mode, label) in [
        (TokenMode::BadNonce, "nonce"),
        (TokenMode::BadSignature, "signature"),
        (TokenMode::BadAudience, "audience"),
        (TokenMode::BadIssuer, "issuer"),
    ] {
        *provider.next_mode.lock().await = mode;
        let rejected = follow_login(&client, &base, workspace.id.0).await;
        assert!(
            rejected.status().is_client_error(),
            "invalid {label} unexpectedly created a session: {}",
            rejected.status()
        );
        assert!(
            rejected
                .headers()
                .get_all(reqwest::header::SET_COOKIE)
                .iter()
                .all(|value| !value.as_bytes().starts_with(b"maidan_session=")),
            "invalid {label} set a session cookie"
        );
    }

    let callback = follow_login(&client, &base, workspace.id.0).await;
    assert_eq!(callback.status(), ClientStatus::TEMPORARY_REDIRECT);
    let session_cookie = callback
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            value
                .split(';')
                .next()
                .filter(|pair| pair.starts_with("maidan_session="))
        })
        .expect("session cookie")
        .to_string();

    let session = client
        .get(format!("{base}/auth/session"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("read session");
    assert_eq!(session.status(), ClientStatus::OK);
    let session_body: Value = session.json().await.expect("session JSON");
    assert_eq!(session_body["workspace_id"], workspace.id.0.to_string());
    let member_id = MemberId(
        Uuid::parse_str(
            session_body["member_id"]
                .as_str()
                .expect("session member id"),
        )
        .expect("member UUID"),
    );
    let member = store
        .get_member(member_id)
        .await
        .expect("provisioned member");
    assert_eq!(member.handle, "loopback");
    let identity = store
        .get_oidc_identity(workspace.id, &issuer, "loopback-user")
        .await
        .expect("OIDC identity link");
    assert_eq!(identity.member_id, member_id);

    let logout = client
        .post(format!("{base}/auth/logout"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("logout");
    assert_eq!(logout.status(), ClientStatus::TEMPORARY_REDIRECT);
    assert!(logout
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .any(|value| value
            .to_str()
            .map(|value| value.contains("Max-Age=0"))
            .unwrap_or(false)));
    let provider_logout_url = logout
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("provider logout redirect")
        .to_str()
        .expect("provider logout URL");
    let provider_logout_response = client
        .get(provider_logout_url)
        .send()
        .await
        .expect("provider logout");
    assert_eq!(
        provider_logout_response
            .headers()
            .get(reqwest::header::LOCATION)
            .expect("post-logout redirect"),
        signed_out_uri.as_str()
    );
    let stale_session = client
        .get(format!("{base}/auth/session"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .expect("check deleted session");
    assert_eq!(stale_session.status(), ClientStatus::UNAUTHORIZED);

    assert!(provider.discovery_seen.load(Ordering::SeqCst));
    assert!(provider.authorize_seen.load(Ordering::SeqCst));
    assert!(provider.token_seen.load(Ordering::SeqCst));
    assert!(provider.jwks_seen.load(Ordering::SeqCst));
    assert!(provider.logout_seen.load(Ordering::SeqCst));

    maidan_server.abort();
    idp_server.abort();
}
