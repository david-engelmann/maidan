//! Workspace invites end to end, on the mock identity provider: an admin
//! mints an invite, the person signs in with `?invite=`, and joins as a human
//! member only when the verified email matches. An invite never grants more
//! than its inviter holds, never crosses workspaces, works once, and a
//! viewer's session cannot write or mint the first admin token.

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
use reqwest::{header, redirect::Policy, StatusCode};
use serde_json::{json, Value};

const SECRET: &[u8] = b"oidc-invite-e2e-secret-32-bytes!!!!!!!";
const ISSUER: &str = "https://mock.idp.local";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Env {
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
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
            signup: false,
            signup_email_domains: vec![],
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
    async fn workspace(&self, name: &str) -> WorkspaceId {
        self.store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id
    }

    async fn token(&self, ws: WorkspaceId, handle: &str, capabilities: &[&str]) -> String {
        let member = self
            .store
            .create_member(NewMember {
                workspace_id: ws,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: ws,
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

    async fn mint(
        &self,
        bearer: &str,
        ws: WorkspaceId,
        email: &str,
        role: &str,
    ) -> (StatusCode, Value) {
        let res = self
            .client
            .post(format!("{}/workspaces/{}/invites", self.base, ws.0))
            .header(header::AUTHORIZATION, bearer)
            .json(&json!({"email": email, "role": role}))
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn list(&self, bearer: &str, ws: WorkspaceId) -> (StatusCode, Value) {
        let res = self
            .client
            .get(format!("{}/workspaces/{}/invites", self.base, ws.0))
            .header(header::AUTHORIZATION, bearer)
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn revoke(&self, bearer: &str, ws: WorkspaceId, id: &str) -> StatusCode {
        self.client
            .delete(format!("{}/workspaces/{}/invites/{id}", self.base, ws.0))
            .header(header::AUTHORIZATION, bearer)
            .send()
            .await
            .unwrap()
            .status()
    }

    /// Open the invite link and follow the mock provider back to the
    /// callback as `subject` with `email`. `Err` is the start's refusal.
    async fn accept(
        &self,
        accept_path: &str,
        subject: &str,
        email: &str,
    ) -> Result<reqwest::Response, StatusCode> {
        let start = self
            .client
            .get(format!("{}{accept_path}", self.base))
            .send()
            .await
            .unwrap();
        if start.status() != StatusCode::TEMPORARY_REDIRECT {
            return Err(start.status());
        }
        let location = start.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"))
            .replace(
                "mock_email=human@example.com",
                &format!("mock_email={email}"),
            );
        Ok(self
            .client
            .get(format!("{}{location}", self.base))
            .send()
            .await
            .unwrap())
    }

    async fn me(&self, cookie: &str) -> Value {
        let res = self
            .client
            .get(format!("{}/ui/api/me", self.base))
            .header(header::COOKIE, cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        res.json().await.unwrap()
    }

    async fn create_channel(&self, cookie: &str, ws: WorkspaceId) -> StatusCode {
        self.client
            .post(format!("{}/ui/api/workspaces/{}/channels", self.base, ws.0))
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, &self.base)
            .json(&json!({"name": format!("c-{}", uuid::Uuid::new_v4().simple())}))
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn mint_first_admin(&self, cookie: &str) -> StatusCode {
        self.client
            .post(format!("{}/auth/session/mint", self.base))
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, &self.base)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status()
    }

    /// Confirm an approval with a made-up gate and token. The write check
    /// runs before the lookup, so a viewer gets 403 where a writer gets 404.
    async fn confirm_unknown_gate(&self, cookie: &str) -> StatusCode {
        self.client
            .post(format!("{}/auth/approval-confirmations/confirm", self.base))
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, &self.base)
            .json(&json!({"gate_id": uuid::Uuid::now_v7(), "token": "nope"}))
            .send()
            .await
            .unwrap()
            .status()
    }
}

fn session_cookie(res: &reqwest::Response) -> String {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("maidan_session="))
        .map(|v| v.split(';').next().unwrap().to_string())
        .expect("session cookie")
}

fn caps(me: &Value) -> Vec<String> {
    me["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn member_invite_joins_once_and_only_with_the_matching_email() {
    let env = spawn().await;
    let ws = env.workspace("acme").await;
    let admin = env
        .token(
            ws,
            "admin",
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;

    let (status, body) = env.mint(&admin, ws, "Ada@A.example", "member").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let secret = body["secret"].as_str().unwrap();
    assert!(secret.starts_with("maid_inv_"));
    let path = body["accept_path"].as_str().unwrap().to_string();
    assert_eq!(path, format!("/auth/oidc/login?invite={secret}"));
    assert_eq!(body["invite"]["email"], "ada@a.example");
    assert_eq!(body["invite"]["role"], "member");

    // The list never carries the secret or its hash.
    let (status, list) = env.list(&admin, ws).await;
    assert_eq!(status, StatusCode::OK);
    let text = list.to_string();
    assert!(
        !text.contains(secret) && !text.contains(&hash_secret(secret)),
        "{text}"
    );

    // A different verified email is refused and spends nothing.
    let wrong = env.accept(&path, "sub-eve", "eve@a.example").await.unwrap();
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

    // The named person joins as a human member of this workspace.
    let ok = env.accept(&path, "sub-ada", "ADA@a.example").await.unwrap();
    assert!(ok.status().is_redirection(), "{}", ok.status());
    let cookie = session_cookie(&ok);
    let me = env.me(&cookie).await;
    assert_eq!(me["workspace_id"], ws.0.to_string());
    // A member's sign-in can post and move tasks in the console. The card
    // reads this list, so the read router's subset would show those as "Can't".
    let held = caps(&me);
    for cap in [
        "workspace:read",
        "workspace:write",
        "message:post",
        "thread:transition",
        "artifact:upload",
        "event:subscribe",
        "search:query",
    ] {
        assert!(
            held.iter().any(|c| c == cap),
            "a member's session can {cap} in the console, got {held:?}"
        );
    }
    for cap in ["federation:admin", "operator:global", "token:admin"] {
        assert!(
            !held.iter().any(|c| c == cap),
            "a member's session does not hold {cap}, got {held:?}"
        );
    }
    assert_eq!(env.create_channel(&cookie, ws).await, StatusCode::CREATED);
    let member_id: uuid::Uuid = me["member_id"].as_str().unwrap().parse().unwrap();
    let member = env
        .store
        .get_member(maidan_types::MemberId(member_id))
        .await
        .unwrap();
    assert_eq!(member.kind, MemberKind::Human);
    // An invited member cannot mint the first admin token either, and a
    // writer's confirmation passes the write check (then finds no gate).
    assert_eq!(env.mint_first_admin(&cookie).await, StatusCode::FORBIDDEN);
    assert_eq!(
        env.confirm_unknown_gate(&cookie).await,
        StatusCode::NOT_FOUND
    );

    // Reuse: the link is dead at the start, for anyone.
    assert_eq!(
        env.accept(&path, "sub-other", "ada@a.example").await.err(),
        Some(StatusCode::FORBIDDEN)
    );
    // A made-up invite is refused the same way.
    assert_eq!(
        env.accept(
            "/auth/oidc/login?invite=maid_inv_nope",
            "s",
            "ada@a.example"
        )
        .await
        .err(),
        Some(StatusCode::FORBIDDEN)
    );
}

#[tokio::test]
async fn an_invite_never_grants_above_the_inviter_and_never_crosses_workspaces() {
    let env = spawn().await;
    let ws = env.workspace("acme").await;
    let other = env.workspace("rival").await;
    let read_only_admin = env
        .token(ws, "ro-admin", &["workspace:admin", "workspace:read"])
        .await;
    let plain = env
        .token(ws, "plain", &["workspace:read", "workspace:write"])
        .await;
    let rival = env
        .token(
            other,
            "rival",
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;

    // No workspace:admin, no invite.
    let (status, _) = env.mint(&plain, ws, "x@b.example", "viewer").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(env.list(&plain, ws).await.0, StatusCode::FORBIDDEN);

    // An admin without workspace:write cannot invite a member, only a viewer.
    let (status, body) = env
        .mint(&read_only_admin, ws, "x@b.example", "member")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = env
        .mint(&read_only_admin, ws, "vi@b.example", "viewer")
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let viewer_path = body["accept_path"].as_str().unwrap().to_string();
    let viewer_id = body["invite"]["id"].as_str().unwrap().to_string();
    // No role beyond the two, and no unknown fields.
    let (status, _) = env.mint(&read_only_admin, ws, "x@b.example", "admin").await;
    assert!(status.is_client_error());

    // Another workspace's admin can neither mint, list nor revoke here.
    assert!(env
        .mint(&rival, ws, "x@b.example", "viewer")
        .await
        .0
        .is_client_error());
    assert!(env.list(&rival, ws).await.0.is_client_error());
    assert!(env.revoke(&rival, ws, &viewer_id).await.is_client_error());
    assert!(env
        .list(&rival, other)
        .await
        .1
        .as_array()
        .unwrap()
        .is_empty());

    // The viewer joins with read capabilities only, and cannot write or mint
    // the first admin token even though the workspace has no token admin.
    let ok = env
        .accept(&viewer_path, "sub-vi", "vi@b.example")
        .await
        .unwrap();
    assert!(ok.status().is_redirection(), "{}", ok.status());
    let cookie = session_cookie(&ok);
    let me = env.me(&cookie).await;
    assert_eq!(me["workspace_id"], ws.0.to_string());
    let held = caps(&me);
    assert!(held.iter().any(|c| c == "workspace:read"), "{me}");
    for cap in ["event:subscribe", "search:query"] {
        assert!(
            held.iter().any(|c| c == cap),
            "a viewer can {cap}, got {held:?}"
        );
    }
    for cap in [
        "workspace:write",
        "message:post",
        "thread:transition",
        "artifact:upload",
        "federation:admin",
        "operator:global",
        "token:admin",
    ] {
        assert!(
            !held.iter().any(|c| c == cap),
            "a viewer does not hold {cap}, got {held:?}"
        );
    }
    assert_eq!(env.create_channel(&cookie, ws).await, StatusCode::FORBIDDEN);
    assert_eq!(env.mint_first_admin(&cookie).await, StatusCode::FORBIDDEN);
    // Confirming a model's approval is a write: a viewer is refused before
    // any lookup.
    assert_eq!(
        env.confirm_unknown_gate(&cookie).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_revoked_invite_is_refused() {
    let env = spawn().await;
    let ws = env.workspace("acme").await;
    let admin = env
        .token(
            ws,
            "admin",
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;
    let (_, body) = env.mint(&admin, ws, "re@c.example", "member").await;
    let id = body["invite"]["id"].as_str().unwrap().to_string();
    let path = body["accept_path"].as_str().unwrap().to_string();
    assert_eq!(env.revoke(&admin, ws, &id).await, StatusCode::NO_CONTENT);
    assert_eq!(
        env.accept(&path, "sub-re", "re@c.example").await.err(),
        Some(StatusCode::FORBIDDEN)
    );
}
