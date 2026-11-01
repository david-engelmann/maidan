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
    spawn_with(signup, domains, None).await
}

async fn spawn_with(signup: bool, domains: Vec<String>, public_origin: Option<&str>) -> Env {
    spawn_full(signup, domains, public_origin, false, [None, None]).await
}

async fn spawn_full(
    signup: bool,
    domains: Vec<String>,
    public_origin: Option<&str>,
    signup_open: bool,
    [privacy_url, terms_url]: [Option<&str>; 2],
) -> Env {
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
            signup_open,
            emails_verified: false,
            privacy_url: privacy_url.map(str::to_string),
            terms_url: terms_url.map(str::to_string),
        },
        session_secret: Arc::from(SECRET),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    state.public_origin = public_origin.map(str::to_string);
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
async fn open_signup_gives_each_verified_address_its_own_workspace_without_an_invite() {
    let env = spawn_full(true, vec![], None, true, [None, None]).await;
    // Two people on unrelated domains, neither invited nor listed.
    let ada = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup", env.base),
            "sub-ada",
            "ada@example.org",
        )
        .await;
    assert_eq!(ada.status(), StatusCode::TEMPORARY_REDIRECT);
    let ada_cookie = session_cookie(&ada);
    let bob = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup", env.base),
            "sub-bob",
            "bob@other.net",
        )
        .await;
    assert_eq!(bob.status(), StatusCode::TEMPORARY_REDIRECT);
    let bob_cookie = session_cookie(&bob);

    // Each session sees only its own, fresh workspace.
    let ada_ws = env
        .store
        .list_subject_workspaces(ISSUER, "sub-ada", 10)
        .await
        .unwrap();
    let bob_ws = env
        .store
        .list_subject_workspaces(ISSUER, "sub-bob", 10)
        .await
        .unwrap();
    assert_eq!(ada_ws.len(), 1, "ada has exactly one workspace");
    assert_eq!(bob_ws.len(), 1, "bob has exactly one workspace");
    assert_ne!(
        ada_ws[0].workspace_id, bob_ws[0].workspace_id,
        "the tenants are separate"
    );
    assert_ne!(ada_cookie, bob_cookie);

    // The audit row says how the door was opened.
    let events = env
        .store
        .list_audit_for_workspace(ada_ws[0].workspace_id, 50)
        .await
        .unwrap();
    let signup = events
        .iter()
        .find(|e| e.action == "workspace.signup")
        .expect("a sign-up audit row");
    assert_eq!(signup.metadata["via"], "open");

    // Open does not mean repeatable: the same identity is sent to sign in.
    let again = env
        .signup_roundtrip(
            format!("{}/auth/oidc/signup", env.base),
            "sub-ada",
            "ada@example.org",
        )
        .await;
    assert_eq!(again.status(), StatusCode::FORBIDDEN);
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
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_OIDC_ENABLED", "1") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_SESSION_SECRET", "x") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_OIDC_ISSUER", "https://idp.example") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_OIDC_REDIRECT_URI", "http://127.0.0.1/callback") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_SIGNUP", "1") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_OIDC_AUTO_PROVISION", "1") };
    let refused = OidcSettings::from_env();
    for (k, v) in saved {
        match v {
            // SAFETY: test setup. Maidan reads this variable only through std,
            // whose environment lock serializes the read with this write.
            Some(v) => unsafe { std::env::set_var(k, v) },
            // SAFETY: test setup. Maidan reads this variable only through std,
            // whose environment lock serializes the read with this write.
            None => unsafe { std::env::remove_var(k) },
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

async fn raw_join(env: &Env, body: Value, extra: &[(&str, &str)]) -> (StatusCode, String) {
    let mut req = env
        .client
        .post(format!("{}/waitlist", env.base))
        .json(&body);
    for (name, value) in extra {
        req = req.header(*name, *value);
    }
    let res = req.send().await.unwrap();
    let status = res.status();
    (status, res.text().await.unwrap())
}

#[tokio::test]
async fn a_repeat_join_gets_a_byte_identical_answer_to_a_fresh_one() {
    let env = spawn(true, vec![]).await;
    let body = json!({"email": "grace@example.com", "use_case": "review swarm", "consent": true});
    let fresh = raw_join(&env, body.clone(), &[]).await;
    let repeat = raw_join(&env, body, &[]).await;
    assert_eq!(fresh.0, StatusCode::CREATED);
    assert_eq!(fresh, repeat, "a listed address must not be told apart");
}

#[tokio::test]
async fn a_filled_honeypot_is_answered_like_a_join_and_stores_nothing() {
    let env = spawn(true, vec![]).await;
    let human = raw_join(
        &env,
        json!({"email": "lin@example.com", "use_case": "agents", "consent": true, "website": ""}),
        &[],
    )
    .await;
    let bot = raw_join(
        &env,
        json!({"email": "bot@example.com", "use_case": "agents", "consent": true, "website": "https://spam.example"}),
        &[],
    )
    .await;
    assert_eq!(human.0, StatusCode::CREATED);
    assert_eq!(bot.0, StatusCode::CREATED);
    assert_eq!(
        bot.1.replace("bot@", "lin@"),
        human.1,
        "the bot's answer has the human's shape"
    );
    let op = env.token(&["operator:global"]).await;
    let (_, list) = env.operator_list(&op).await;
    let emails: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["email"].as_str())
        .collect();
    assert_eq!(emails, vec!["lin@example.com"]);
}

#[tokio::test]
async fn the_waitlist_refuses_shapes_that_are_not_email_addresses() {
    let env = spawn(true, vec![]).await;
    let long_local = format!("{}@example.com", "a".repeat(65));
    let long_total = format!("a@{}.com", "b".repeat(260));
    for email in [
        "plain",
        "@example.com",
        "ada@",
        "ada@localhost",
        "ada@example..com",
        "ada@-example.com",
        "ada@exa_mple.com",
        "Ada <ada@example.com>",
        "ada@example.com\u{0}",
        "ada@@example.com",
        "a,b@example.com",
        long_local.as_str(),
        long_total.as_str(),
    ] {
        let (status, _) = env.join(email, "swarms", true).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{email:?}");
    }
    let too_long_note = "x".repeat(2_001);
    let (status, _) = env.join("ada@example.com", &too_long_note, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_join_posted_from_another_site_is_refused_and_one_from_this_site_is_taken() {
    let env = spawn(true, vec![]).await;
    let host = env.base.trim_start_matches("http://").to_string();
    let body = json!({"email": "kay@example.com", "use_case": "swarms", "consent": true});
    let (status, _) = raw_join(&env, body.clone(), &[("origin", "https://evil.example")]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = raw_join(&env, body.clone(), &[("origin", "null")]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = raw_join(&env, body.clone(), &[("sec-fetch-site", "cross-site")]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let same = format!("http://{host}");
    let (status, text) = raw_join(&env, body, &[("origin", same.as_str())]).await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
}

#[tokio::test]
async fn the_waitlist_page_is_served_with_a_strict_policy_only_where_sign_up_is_on() {
    let env = spawn(true, vec![]).await;
    let res = env
        .client
        .get(format!("{}/", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers().clone();
    let csp = h["content-security-policy"].to_str().unwrap().to_string();
    assert!(csp.contains("default-src 'none'"), "{csp}");
    assert!(csp.contains("script-src 'self'"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(
        !csp.contains("script-src 'unsafe-inline'")
            && !csp.contains("script-src 'self' 'unsafe-inline'"),
        "{csp}"
    );
    assert!(csp.contains("style-src 'self'"), "{csp}");
    assert!(
        csp.contains("style-src-attr 'unsafe-inline'"),
        "motion sets transform as a style attribute: {csp}"
    );
    assert!(!csp.contains("style-src 'unsafe-inline'"), "{csp}");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["referrer-policy"], "no-referrer");
    let html = res.text().await.unwrap();
    assert!(
        html.contains("id=\"wl-website\""),
        "the honeypot field is on the page"
    );
    let lower = html.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("<script") {
        let tag = &rest[at..];
        let end = tag.find('>').unwrap_or(tag.len());
        assert!(
            tag[..end].contains(" src="),
            "a script without src: {}",
            &tag[..end]
        );
        assert!(tag[end..].starts_with("></script>"), "a script with a body");
        rest = &tag[end..];
    }
    assert!(!lower.contains("<style"), "no style element");
    assert!(!lower.contains("style="), "no inline style");
    assert!(!has_inline_handler(&lower), "no inline handlers");
    for asset in [
        "style.css",
        "app.js",
        "vendor.js",
        "geist.woff2",
        "og.png",
        "icon.png",
    ] {
        let res = env
            .client
            .get(format!("{}/landing/{asset}", env.base))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{asset}");
        assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    }
    // Without sign-up, `/` is a route index: JSON by default, HTML for a
    // browser, and no script or style either way.
    let off = spawn(false, vec![]).await;
    let res = off
        .client
        .get(format!("{}/", off.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let index: serde_json::Value = res.json().await.unwrap();
    assert_eq!(index["routes"]["openapi"], "/openapi.json");
    assert_eq!(index["routes"]["mcp_worker"], "/mcp/worker");
    assert_eq!(index["routes"]["health"], "/health");
    assert_eq!(index["routes"]["console"], "/ui");
    let res = off
        .client
        .get(format!("{}/", off.base))
        .header("accept", "application/json, text/html")
        .send()
        .await
        .unwrap();
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    let res = off
        .client
        .get(format!("{}/", off.base))
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()["content-security-policy"].to_str().unwrap(),
        maidan_server::app::ROUTE_INDEX_CSP
    );
    let html = res.text().await.unwrap();
    for (_, path, _) in maidan_server::app::ROUTE_INDEX {
        assert!(html.contains(&format!("href=\"{path}\"")), "{path}: {html}");
    }
    assert!(!html.contains("<script") && !html.contains("<style") && !has_inline_handler(&html));
    // Unknown paths still 404, and the problem points at the API description.
    let res = off
        .client
        .get(format!("{}/no-such-route", off.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let problem: serde_json::Value = res.json().await.unwrap();
    assert!(problem["detail"]
        .as_str()
        .unwrap()
        .contains("/openapi.json"));
}

#[tokio::test]
async fn the_waitlist_page_links_the_privacy_policy_and_terms_where_the_instance_names_them() {
    let env = spawn_full(
        true,
        vec![],
        None,
        false,
        [
            Some("/privacy"),
            Some("https://example.com/terms?v=2&lang=en"),
        ],
    )
    .await;
    let html = env
        .client
        .get(format!("{}/", env.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("<a href=\"/privacy\">Privacy</a>"), "{html}");
    assert!(
        html.contains("<a href=\"https://example.com/terms?v=2&amp;lang=en\">Terms</a>"),
        "{html}"
    );
    assert!(!html.contains("policy-links"), "the slot is filled");

    // Without them the footer has neither link, and no leftover slot.
    let plain = spawn(true, vec![]).await;
    let html = plain
        .client
        .get(format!("{}/", plain.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!html.contains(">Privacy<") && !html.contains(">Terms<"));
    assert!(html.contains("Source on GitHub"));
}

fn has_inline_handler(html: &str) -> bool {
    [
        "onclick=",
        "onload=",
        "onerror=",
        "onsubmit=",
        "oninput=",
        "onchange=",
    ]
    .iter()
    .any(|h| html.contains(h))
}

#[tokio::test]
async fn an_origin_matching_the_public_url_is_taken_even_when_the_host_differs() {
    let env = spawn_with(true, vec![], Some("https://maidan.example.com")).await;
    let body = json!({"email": "pat@example.com", "use_case": "swarms", "consent": true});
    let (status, text) = raw_join(
        &env,
        body.clone(),
        &[("origin", "https://maidan.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    let host_origin = env.base.clone();
    let (status, _) = raw_join(&env, body.clone(), &[("origin", host_origin.as_str())]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the Host header no longer vouches"
    );
    let (status, _) = raw_join(&env, body, &[("origin", "https://evil.example")]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn landing_asset_paths_outside_the_built_bundle_are_not_found() {
    let env = spawn(true, vec![]).await;
    for path in [
        "/landing/../Cargo.toml",
        "/landing/%2e%2e%2fCargo.toml",
        "/landing/..%2fsrc%2fapp.rs",
        "/landing/index.html",
        "/landing/landing.css.map",
        "/landing/nope.js",
        "/landing/",
    ] {
        let res = env
            .client
            .get(format!("{}{path}", env.base))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn landing_assets_are_not_found_where_sign_up_is_off() {
    let off = spawn(false, vec![]).await;
    for asset in [
        "style.css",
        "app.js",
        "vendor.js",
        "geist.woff2",
        "og.png",
        "icon.png",
    ] {
        let res = off
            .client
            .get(format!("{}/landing/{asset}", off.base))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{asset}");
    }
}

#[tokio::test]
async fn a_cross_site_plain_text_or_form_post_is_refused_and_stores_nothing() {
    let env = spawn(true, vec![]).await;
    let json_text = r#"{"email":"eve@example.com","use_case":"x","consent":true}"#;
    for (content_type, body) in [
        ("text/plain", json_text.to_string()),
        (
            "application/x-www-form-urlencoded",
            "email=eve%40example.com&use_case=x&consent=true".to_string(),
        ),
        (
            "multipart/form-data; boundary=x",
            format!("--x\r\n{json_text}\r\n--x--"),
        ),
    ] {
        let res = env
            .client
            .post(format!("{}/waitlist", env.base))
            .header("content-type", content_type)
            .header("origin", "https://evil.example")
            .header("sec-fetch-site", "cross-site")
            .body(body)
            .send()
            .await
            .unwrap();
        assert!(
            res.status().is_client_error(),
            "{content_type}: {}",
            res.status()
        );
    }
    let op = env.token(&["operator:global"]).await;
    let (_, list) = env.operator_list(&op).await;
    assert_eq!(list.as_array().map(Vec::len), Some(0), "nothing was stored");
}
