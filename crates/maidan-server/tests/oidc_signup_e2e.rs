//! Next 26: the waitlist, the operator's invite, and the invited sign-up.
//!
//! A signed-out visitor joins the waitlist; an operator reads it, exports
//! it and turns an entry into an invite; the invite starts an OIDC sign-up
//! whose callback creates the workspace, its first member, the identity and
//! the session in one transaction. Two tenants throughout: the person
//! signing up already has a handle twin in an existing workspace, and the
//! sign-up must still land in a workspace born with it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::{redirect::Policy, StatusCode};
use serde_json::{json, Value};

const SECRET: &[u8] = b"oidc-signup-e2e-secret-32-bytes!!!!!!";
const ISSUER: &str = "https://mock.idp.local";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn(signup: bool, domains: Vec<String>) -> Env {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
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
            signup,
            signup_email_domains: domains,
        },
        session_secret: Arc::from(SECRET),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Env {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

impl Env {
    async fn join(&self, email: &str, use_case: &str, consent: bool) -> (StatusCode, Value) {
        let res = self
            .client
            .post(format!("{}/waitlist", self.base))
            .json(&json!({"email": email, "use_case": use_case, "consent": consent}))
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn token(&self, capabilities: &[&str]) -> String {
        let ws = self
            .store
            .create_workspace(NewWorkspace { name: "ops".into() })
            .await
            .unwrap();
        let member = self
            .store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "op".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        self.store
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
        format!("Bearer {}", secret.as_str())
    }

    async fn operator_list(&self, bearer: &str) -> (StatusCode, Value) {
        let res = self
            .client
            .get(format!("{}/operator/waitlist", self.base))
            .header(reqwest::header::AUTHORIZATION, bearer)
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn invite(&self, bearer: &str, entry_id: &str) -> (StatusCode, Value) {
        let res = self
            .client
            .post(format!("{}/operator/waitlist/{entry_id}/invite", self.base))
            .header(reqwest::header::AUTHORIZATION, bearer)
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// Start a sign-up and follow the mock provider back to the callback.
    /// Returns the callback response.
    async fn signup_roundtrip(
        &self,
        start_url: String,
        subject: &str,
        email: &str,
    ) -> reqwest::Response {
        let start = self.client.get(&start_url).send().await.unwrap();
        assert_eq!(
            start.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "signup start"
        );
        let location = start.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"))
            .replace(
                "mock_email=human@example.com",
                &format!("mock_email={email}"),
            );
        self.client
            .get(format!("{}{location}", self.base))
            .send()
            .await
            .unwrap()
    }
}

fn session_cookie(res: &reqwest::Response) -> String {
    res.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("maidan_session="))
        .map(|v| v.split(';').next().unwrap().to_string())
        .expect("session cookie")
}

#[tokio::test]
async fn waitlist_join_validates_and_answers_the_same_for_a_repeat() {
    let env = spawn(true, vec![]).await;
    // No consent, no join.
    let (status, _) = env.join("ada@example.com", "swarms", false).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Not an address, and nothing to go on.
    let (status, _) = env.join("not-an-email", "swarms", true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = env.join("ada@example.com", "  ", true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The join itself.
    let (status, body) = env
        .join("Ada@Example.com", "running a support swarm", true)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "waiting");
    // A repeat gets the same answer and does not add a row.
    let (status, body) = env.join("ada@example.com", "different words", true).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "waiting");
    let op = env.token(&["operator:global"]).await;
    let (status, list) = env.operator_list(&op).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["use_case"], "running a support swarm");
}

#[tokio::test]
async fn operator_reads_exports_and_invites_the_waitlist() {
    let env = spawn(true, vec![]).await;
    env.join("ada@example.com", "swarms", true).await;
    env.join("grace@example.com", "compilers", true).await;

    // A plain workspace member is not an operator.
    let member = env.token(&["workspace:read"]).await;
    let (status, _) = env.operator_list(&member).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // No credential at all.
    let res = env
        .client
        .get(format!("{}/operator/waitlist", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let op = env.token(&["operator:global"]).await;
    let (status, list) = env.operator_list(&op).await;
    assert_eq!(status, StatusCode::OK);
    let entries = list.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["email"], "ada@example.com");
    assert!(
        entries[0].get("invite_token_hash").is_none(),
        "the list never carries the invite hash: {entries:?}"
    );

    // The export is a CSV download carrying both addresses.
    let res = env
        .client
        .get(format!("{}/operator/waitlist/export", env.base))
        .header(reqwest::header::AUTHORIZATION, &op)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let csv = res.text().await.unwrap();
    assert!(csv.contains("ada@example.com"), "{csv}");
    assert!(csv.contains("grace@example.com"), "{csv}");
    // A use case written as a spreadsheet formula is exported inert.
    env.join(
        "mallory@example.com",
        "=HYPERLINK(\"https://x.example\")",
        true,
    )
    .await;
    let res = env
        .client
        .get(format!("{}/operator/waitlist/export", env.base))
        .header(reqwest::header::AUTHORIZATION, &op)
        .send()
        .await
        .unwrap();
    let csv = res.text().await.unwrap();
    assert!(
        csv.contains("'=HYPERLINK"),
        "formula cells must be prefixed inert: {csv}"
    );

    // The invite mints a token shown once, with the sign-up path.
    let entry_id = entries[0]["id"].as_str().unwrap().to_string();
    let (status, minted) = env.invite(&op, &entry_id).await;
    assert_eq!(status, StatusCode::OK, "{minted}");
    let token = minted["invite_token"].as_str().unwrap();
    assert!(token.len() >= 32, "token: {token}");
    assert_eq!(
        minted["signup_path"],
        format!("/auth/oidc/signup?invite={token}")
    );
    assert_eq!(minted["entry"]["status"], "invited");
    // A missing entry does not exist.
    let (status, _) = env
        .invite(&op, "0191c5f4-0000-7000-8000-000000000000")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invited_signup_creates_a_fresh_workspace_and_session() {
    let env = spawn(true, vec![]).await;
    // An existing workspace whose admin shares the handle the sign-up will
    // derive. The sign-up must not land here.
    let existing = env
        .store
        .create_workspace(NewWorkspace {
            name: "existing".into(),
        })
        .await
        .unwrap();
    env.store
        .create_member(NewMember {
            workspace_id: existing.id,
            handle: "ada".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();

    env.join("ada@example.com", "swarms", true).await;
    let op = env.token(&["operator:global"]).await;
    let (_, list) = env.operator_list(&op).await;
    let entry_id = list[0]["id"].as_str().unwrap().to_string();
    let (_, minted) = env.invite(&op, &entry_id).await;
    let token = minted["invite_token"].as_str().unwrap().to_string();

    let callback = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup?invite={token}", env.base),
            "sub-ada",
            "ada@example.com",
        )
        .await;
    assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
    let cookie = session_cookie(&callback);

    // The session names a NEW workspace, and the existing one kept its
    // single member.
    let res = env
        .client
        .get(format!("{}/auth/session", env.base))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let session: Value = res.json().await.unwrap();
    let new_ws = session["workspace_id"].as_str().unwrap();
    assert_ne!(new_ws, existing.id.0.to_string());
    assert_eq!(env.store.list_members(existing.id).await.unwrap().len(), 1);
    let new_id = WorkspaceId(new_ws.parse().unwrap());
    let members = env.store.list_members(new_id).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].handle, "ada");

    // The entry is signed up and bound to the new workspace; the spent
    // invite no longer starts a sign-up.
    let (_, list) = env.operator_list(&op).await;
    assert_eq!(list[0]["status"], "signed_up");
    assert_eq!(list[0]["workspace_id"], new_ws);
    let start = env
        .client
        .get(format!("{}/auth/oidc/signup?invite={token}", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_invite_only_signs_up_the_address_it_was_minted_for() {
    let env = spawn(true, vec![]).await;
    env.join("ada@example.com", "swarms", true).await;
    let op = env.token(&["operator:global"]).await;
    let (_, list) = env.operator_list(&op).await;
    let entry_id = list[0]["id"].as_str().unwrap().to_string();
    let (_, minted) = env.invite(&op, &entry_id).await;
    let token = minted["invite_token"].as_str().unwrap().to_string();

    // The provider verifies a different address: refused, and the invite
    // is not spent.
    let callback = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup?invite={token}", env.base),
            "sub-mallory",
            "mallory@example.com",
        )
        .await;
    assert_eq!(callback.status(), StatusCode::FORBIDDEN);
    let (_, list) = env.operator_list(&op).await;
    assert_eq!(list[0]["status"], "invited");

    // The right address still completes.
    let callback = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup?invite={token}", env.base),
            "sub-ada",
            "ada@example.com",
        )
        .await;
    assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
}

#[tokio::test]
async fn signup_start_is_not_found_until_the_instance_enables_it() {
    let env = spawn(false, vec![]).await;
    let res = env
        .client
        .get(format!("{}/auth/oidc/signup?invite=whatever", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    // The whole surface is dark while sign-up is off: no join is collected
    // and the operator list does not exist either.
    let (status, _) = env.join("ada@example.com", "swarms", true).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let op = env.token(&["operator:global"]).await;
    let (status, _) = env.operator_list(&op).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Without an invite and no domain list, an enabled instance refuses.
    let env = spawn(true, vec![]).await;
    let res = env
        .client
        .get(format!("{}/auth/oidc/signup", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // An unrecognized invite is refused the same way.
    let res = env
        .client
        .get(format!("{}/auth/oidc/signup?invite=nope", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_listed_domain_signs_up_without_an_invite() {
    let env = spawn(true, vec!["example.org".to_string()]).await;
    let callback = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup", env.base),
            "sub-ada",
            "ada@example.org",
        )
        .await;
    assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
    session_cookie(&callback);
    // The same identity cannot sign up a second time; it has a workspace
    // now and belongs at the front door.
    let callback = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup", env.base),
            "sub-ada",
            "ada@example.org",
        )
        .await;
    assert_eq!(callback.status(), StatusCode::FORBIDDEN);
    // An unlisted domain is refused at the callback.
    let callback = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup", env.base),
            "sub-eve",
            "eve@other.net",
        )
        .await;
    assert_eq!(callback.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn boot_refuses_signup_beside_auto_provisioning() {
    // from_env reads the process environment; no other test in this binary
    // calls it, so setting the flags here cannot leak into a sibling.
    let saved: Vec<(&str, Option<String>)> = [
        "MAIDAN_OIDC_ENABLED",
        "MAIDAN_SESSION_SECRET",
        "MAIDAN_OIDC_ISSUER",
        "MAIDAN_OIDC_REDIRECT_URI",
        "MAIDAN_SIGNUP",
        "MAIDAN_OIDC_AUTO_PROVISION",
    ]
    .iter()
    .map(|k| (*k, std::env::var(k).ok()))
    .collect();
    std::env::set_var("MAIDAN_OIDC_ENABLED", "1");
    std::env::set_var("MAIDAN_SESSION_SECRET", "x");
    std::env::set_var("MAIDAN_OIDC_ISSUER", "https://idp.example");
    std::env::set_var("MAIDAN_OIDC_REDIRECT_URI", "http://127.0.0.1/callback");
    std::env::set_var("MAIDAN_SIGNUP", "1");
    std::env::set_var("MAIDAN_OIDC_AUTO_PROVISION", "1");
    let refused = OidcSettings::from_env();
    for (k, v) in saved {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
    match refused {
        Err(maidan_server::config::ConfigError::Invalid(var, _)) => {
            assert_eq!(
                var, "MAIDAN_SIGNUP",
                "boot must refuse the pair on MAIDAN_SIGNUP"
            );
        }
        Err(other) => panic!("boot must refuse the pair, got a different error: {other}"),
        Ok(_) => panic!("boot must refuse the pair, got settings"),
    }
}
