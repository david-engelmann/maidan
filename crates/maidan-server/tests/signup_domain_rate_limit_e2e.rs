//! Next 28 abuse controls: an invite mint and a finished sign-up are each
//! charged to the email's domain, on top of the per-address budget the
//! launch surface already has. Two addresses share one domain's budget, and
//! a second domain keeps its own. The limits are tightened through the
//! environment the specs resolve, in this binary only, and every test uses
//! its own domains because the limiter's buckets are process-global.

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
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
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
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_SIGNUP_DOMAIN_RATE_LIMIT_MAX", "2") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_SIGNUP_DOMAIN_RATE_LIMIT_WINDOW_SECS", "3600") };
    // Each request names its own address, so the tests can show that a
    // spread of addresses still meets the domain's one budget.
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_TRUSTED_PROXY_HOPS", "1") };
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

    async fn invite(&self, bearer: &str, entry_id: &str, ip: &str) -> (StatusCode, Value) {
        let res = self
            .client
            .post(format!("{}/operator/waitlist/{entry_id}/invite", self.base))
            .header(reqwest::header::AUTHORIZATION, bearer)
            .header("x-forwarded-for", ip)
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
        ip: &str,
    ) -> reqwest::Response {
        let callback = self.signup_start(start_url, subject, email, ip).await;
        self.signup_callback(&callback, ip).await
    }

    /// Start a sign-up and return the callback path the mock provider
    /// would send the browser to, without following it.
    async fn signup_start(
        &self,
        start_url: String,
        subject: &str,
        email: &str,
        ip: &str,
    ) -> String {
        let start = self
            .client
            .get(&start_url)
            .header("x-forwarded-for", ip)
            .send()
            .await
            .unwrap();
        assert_eq!(
            start.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "signup start"
        );
        start.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"))
            .replace(
                "mock_email=human@example.com",
                &format!("mock_email={email}"),
            )
    }

    async fn signup_callback(&self, location: &str, ip: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{location}", self.base))
            .header("x-forwarded-for", ip)
            .send()
            .await
            .unwrap()
    }
}

/// Join the waitlist as each of `emails` and return their entry ids, in order.
async fn entries(env: &Env, op: &str, emails: &[&str]) -> Vec<String> {
    for email in emails {
        let (status, body) = env.join(email, "swarms", true).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let (_, list) = env.operator_list(op).await;
    emails
        .iter()
        .map(|email| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|e| e["email"] == *email)
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn invites_for_one_domain_share_its_budget_across_two_addresses() {
    let env = spawn(true, vec![]).await;
    let op = env.token(&["operator:global"]).await;
    let ids = entries(
        &env,
        &op,
        &[
            "a@inv-one.example",
            "b@inv-one.example",
            "c@inv-one.example",
        ],
    )
    .await;
    // Two addresses, one domain: the domain budget (2) binds across both.
    assert_eq!(env.invite(&op, &ids[0], "10.1.0.1").await.0, StatusCode::OK);
    assert_eq!(env.invite(&op, &ids[1], "10.1.0.2").await.0, StatusCode::OK);
    let (status, body) = env.invite(&op, &ids[2], "10.1.0.3").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    // The refused mint changed nothing: the entry is still waiting.
    let (_, list) = env.operator_list(&op).await;
    let refused = list
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == ids[2].as_str())
        .unwrap();
    assert_eq!(refused["status"], "waiting");
}

#[tokio::test]
async fn invites_for_a_second_domain_keep_their_own_budget() {
    let env = spawn(true, vec![]).await;
    let op = env.token(&["operator:global"]).await;
    let ids = entries(
        &env,
        &op,
        &[
            "a@inv-two.example",
            "b@inv-two.example",
            "c@inv-two.example",
            "a@inv-three.example",
        ],
    )
    .await;
    assert_eq!(env.invite(&op, &ids[0], "10.2.0.1").await.0, StatusCode::OK);
    assert_eq!(env.invite(&op, &ids[1], "10.2.0.1").await.0, StatusCode::OK);
    assert_eq!(
        env.invite(&op, &ids[2], "10.2.0.1").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    // The same address, another domain: not charged to the first.
    assert_eq!(env.invite(&op, &ids[3], "10.2.0.1").await.0, StatusCode::OK);
}

#[tokio::test]
async fn signups_for_one_domain_share_its_budget_across_two_addresses() {
    let env = spawn(true, vec!["su-one.example".to_string()]).await;
    let start = format!("{}/auth/oidc/signup", env.base);
    let first = env
        .signup_roundtrip(start.clone(), "sub-1", "a@su-one.example", "10.3.0.1")
        .await;
    assert_eq!(first.status(), StatusCode::TEMPORARY_REDIRECT);
    // Case and a second address do not mint a fresh bucket.
    let second = env
        .signup_roundtrip(start.clone(), "sub-2", "b@SU-ONE.example", "10.3.0.2")
        .await;
    assert_eq!(second.status(), StatusCode::TEMPORARY_REDIRECT);
    let third = env
        .signup_roundtrip(start.clone(), "sub-3", "c@su-one.example", "10.3.0.3")
        .await;
    assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
    // The refused sign-up created nothing for its identity.
    assert!(env
        .store
        .list_subject_workspaces(ISSUER, "sub-3", 1)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn signups_for_a_second_domain_keep_their_own_budget() {
    let env = spawn(
        true,
        vec!["su-two.example".to_string(), "su-three.example".to_string()],
    )
    .await;
    let start = format!("{}/auth/oidc/signup", env.base);
    for (sub, email) in [("sub-a", "a@su-two.example"), ("sub-b", "b@su-two.example")] {
        let res = env
            .signup_roundtrip(start.clone(), sub, email, "10.4.0.1")
            .await;
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    }
    let res = env
        .signup_roundtrip(start.clone(), "sub-c", "c@su-two.example", "10.4.0.1")
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let res = env
        .signup_roundtrip(start.clone(), "sub-d", "d@su-three.example", "10.4.0.1")
        .await;
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
}

#[tokio::test]
async fn a_refused_signup_repeated_does_not_spend_the_domain_budget() {
    let env = spawn(true, vec!["su-four.example".to_string()]).await;
    let start = format!("{}/auth/oidc/signup", env.base);
    let first = env
        .signup_roundtrip(start.clone(), "sub-1", "a@su-four.example", "10.5.0.1")
        .await;
    assert_eq!(first.status(), StatusCode::TEMPORARY_REDIRECT);
    // The same identity again: refused for already having a workspace, more
    // times than the budget (2) holds.
    for _ in 0..3 {
        let again = env
            .signup_roundtrip(start.clone(), "sub-1", "a@su-four.example", "10.5.0.1")
            .await;
        assert_eq!(again.status(), StatusCode::FORBIDDEN);
    }
    // A domain that may not sign up is refused before any charge too.
    for _ in 0..3 {
        let barred = env
            .signup_roundtrip(start.clone(), "sub-x", "x@not-listed.example", "10.5.0.1")
            .await;
        assert_eq!(barred.status(), StatusCode::FORBIDDEN);
    }
    // The refusals spent nothing: a second address still has the budget's
    // last sign-up.
    let second = env
        .signup_roundtrip(start.clone(), "sub-2", "b@su-four.example", "10.5.0.1")
        .await;
    assert_eq!(second.status(), StatusCode::TEMPORARY_REDIRECT);
}

#[tokio::test]
async fn an_invite_that_expired_mid_login_does_not_spend_the_domain_budget() {
    let env = spawn(true, vec!["su-five.example".to_string()]).await;
    let op = env.token(&["operator:global"]).await;
    let ids = entries(&env, &op, &["a@su-five.example"]).await;
    let (status, minted) = env.invite(&op, &ids[0], "10.6.0.1").await;
    assert_eq!(status, StatusCode::OK, "{minted}");
    let start = format!("{}{}", env.base, minted["signup_path"].as_str().unwrap());
    // Three logins start while the invite is live...
    let mut callbacks = Vec::new();
    for sub in ["sub-1", "sub-2", "sub-3"] {
        callbacks.push(
            env.signup_start(start.clone(), sub, "a@su-five.example", "10.6.0.1")
                .await,
        );
    }
    // ...and it expires before any comes back.
    let entry_id = maidan_types::WaitlistEntryId(ids[0].parse().unwrap());
    let hash = env
        .store
        .get_waitlist_entry(entry_id)
        .await
        .unwrap()
        .invite_token_hash
        .unwrap();
    env.store
        .set_waitlist_invite(
            entry_id,
            hash,
            chrono::Utc::now() - chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    for callback in &callbacks {
        let res = env.signup_callback(callback, "10.6.0.1").await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
    // The three refusals spent nothing: the domain's whole budget (2) is
    // still there.
    let open = format!("{}/auth/oidc/signup", env.base);
    for (sub, email) in [
        ("sub-4", "b@su-five.example"),
        ("sub-5", "c@su-five.example"),
    ] {
        let res = env
            .signup_roundtrip(open.clone(), sub, email, "10.6.0.1")
            .await;
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    }
}

#[tokio::test]
async fn rt_reinvite_of_signed_up_entry_is_409_and_free() {
    let env = spawn(true, vec!["rt-409.example".to_string()]).await;
    let op = env.token(&["operator:global"]).await;
    let ids = entries(&env, &op, &["a@rt-409.example", "b@rt-409.example"]).await;
    let (s, minted) = env.invite(&op, &ids[0], "10.7.0.1").await;
    assert_eq!(s, StatusCode::OK, "{minted}");
    let start = format!("{}{}", env.base, minted["signup_path"].as_str().unwrap());
    let res = env
        .signup_roundtrip(start, "sub-a", "a@rt-409.example", "10.7.0.1")
        .await;
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    for _ in 0..3 {
        let (s, _) = env.invite(&op, &ids[0], "10.7.0.1").await;
        assert_eq!(s, StatusCode::CONFLICT);
    }
    let (s, _) = env.invite(&op, &ids[1], "10.7.0.1").await;
    assert_eq!(s, StatusCode::OK, "refused re-invites spent the budget");
}
