//! The launch gate: the whole hosted path a new team takes, end to end, on
//! Postgres. Each step below is a line of the launch checklist.
//!
//! 1. A visitor joins the waitlist, an operator invites them, and the
//!    invited OIDC sign-up creates a fresh workspace and a session.
//! 2. The person creates a channel for the work.
//! 3. The workspace's agent seat connects an MCP client whose metadata
//!    document lives on a host the instance lists: `/oauth/authorize`,
//!    consent with the seat's session, and the token endpoint with PKCE.
//!    The token works on `/mcp/streamable` and nowhere else.
//! 4. Over MCP with that token the agent runs a change: `create_thread`
//!    (kind change, engine session, with acceptance criteria),
//!    `set_thread_owner` to the person, `claim_next_workspace_thread`,
//!    `acknowledge_claim`, `set_thread_result`, `report_usage`,
//!    `transition_thread start_review`, and `release_claim`.
//! 5. Separation of duties: while it holds the claim the agent cannot close
//!    its own work.
//! 6. The person, over the session, reads the review packet, approves it
//!    with its `evidence_root`, and closes the thread.
//! 7. Metering: after a flush the workspace's meters count its requests and
//!    MCP calls.
//! 8. Two tenants throughout: a second signed-up workspace sees nothing of
//!    the first, its own agent token is refused on the first workspace's
//!    thread and queue, and nothing of the first's counts for it.

// The client's document server is a plain axum server, not the API (see
// clippy.toml).
#![allow(clippy::disallowed_methods)]

mod common;

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use axum::{routing::get, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{
    oauth::registry::ClientMetadataHosts,
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, PostgresStore};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMaidanSession, NewMember, NewWorkspace, UsageMeter,
    WorkspaceId,
};
use reqwest::{header, redirect::Policy, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ORIGIN: &str = "https://maidan.example.com";
const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";
const REDIRECT: &str = "https://client.example/callback";
const CLIENT: &str = "https://docs.client.example/oauth/client.json";
const SECRET: &[u8] = b"launch-gate-e2e-session-secret-32-bytes!";
const ISSUER: &str = "https://mock.idp.local";
const SCOPE: &str = "workspace:read workspace:write thread:transition";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    state: AppState,
    _dir: tempfile::TempDir,
}

/// The client's metadata document, on the listed host.
async fn document_server() -> String {
    let app = Router::new().route(
        "/oauth/client.json",
        get(|| async {
            axum::Json(json!({
                "client_name": "Launch Agent",
                "redirect_uris": [REDIRECT],
                "scope": SCOPE,
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn spawn(pool: sqlx::PgPool) -> Env {
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
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
    state.oauth_client_metadata_hosts = ClientMetadataHosts::parse("docs.client.example")
        .unwrap()
        .with_fetch_origin_override(document_server().await);
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
            signup: true,
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
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state.clone());
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
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        state,
        _dir: dir,
    }
}

fn location(resp: &reqwest::Response) -> String {
    resp.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

fn session_cookie(resp: &reqwest::Response) -> String {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| s.split(';').next())
        .find(|p| p.starts_with("maidan_session=") && p.len() > "maidan_session=".len())
        .expect("a session cookie")
        .to_string()
}

/// A workspace a person signed up into.
struct Tenant {
    ws: WorkspaceId,
    person: MemberId,
    cookie: String,
}

impl Env {
    /// An operator's bearer, in a workspace of its own.
    async fn operator(&self) -> String {
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
                capabilities: vec!["operator:global".into()],
                expires_at: None,
            })
            .await
            .unwrap();
        secret.as_str().to_string()
    }

    /// Step 1: waitlist, invite, and the invited sign-up.
    async fn sign_up(&self, operator: &str, handle: &str) -> Tenant {
        let email = format!("{handle}@example.com");
        let joined = self
            .client
            .post(format!("{}/waitlist", self.base))
            .json(&json!({"email": email, "use_case": "running agents", "consent": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(joined.status(), StatusCode::CREATED);
        let list: Value = self
            .client
            .get(format!("{}/operator/waitlist", self.base))
            .bearer_auth(operator)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let entry = list
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["email"] == email.as_str())
            .unwrap_or_else(|| panic!("{email} is on the waitlist: {list}"));
        let invited = self
            .client
            .post(format!(
                "{}/operator/waitlist/{}/invite",
                self.base,
                entry["id"].as_str().unwrap()
            ))
            .bearer_auth(operator)
            .send()
            .await
            .unwrap();
        assert_eq!(invited.status(), StatusCode::OK);
        let invited: Value = invited.json().await.unwrap();
        let start = self
            .client
            .get(format!(
                "{}{}",
                self.base,
                invited["signup_path"].as_str().unwrap()
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(start.status(), StatusCode::TEMPORARY_REDIRECT);
        let to = location(&start)
            .replace("mock_sub=mock-user", &format!("mock_sub=sub-{handle}"))
            .replace(
                "mock_email=human@example.com",
                &format!("mock_email={email}"),
            );
        let callback = self
            .client
            .get(format!("{}{to}", self.base))
            .send()
            .await
            .unwrap();
        assert!(callback.status().is_redirection(), "{}", callback.status());
        let cookie = session_cookie(&callback);
        let session: Value = self
            .client
            .get(format!("{}/auth/session", self.base))
            .header(header::COOKIE, &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let ws = WorkspaceId(session["workspace_id"].as_str().unwrap().parse().unwrap());
        let members = self.store.list_members(ws).await.unwrap();
        assert_eq!(members.len(), 1, "a fresh workspace, born with its person");
        Tenant {
            ws,
            person: members[0].id,
            cookie,
        }
    }

    /// The workspace's agent seat, signed in to the console as the OIDC
    /// callback signs a member in.
    async fn agent_seat(&self, t: &Tenant) -> (MemberId, String) {
        let agent = self
            .store
            .create_member(NewMember {
                workspace_id: t.ws,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let session = self
            .store
            .create_session(NewMaidanSession {
                workspace_id: t.ws,
                member_id: agent.id,
                api_token_id: None,
                oidc_identity_id: None,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            })
            .await
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        maidan_server::session::set_session_cookie(&mut headers, session.id, 3600, false, SECRET)
            .unwrap();
        let cookie = headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        (agent.id, cookie)
    }

    /// Step 3: authorize, consent and the PKCE token exchange; the access
    /// token.
    async fn connect_client(&self, consenting_cookie: &str) -> String {
        let verifier = URL_SAFE_NO_PAD.encode(uuid::Uuid::now_v7().as_bytes()) + "-pkce-verifier";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let authorize = self
            .client
            .get(format!(
                "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&scope={}&state=xyz&resource={}",
                self.base,
                urlencoding::encode(CLIENT),
                urlencoding::encode(REDIRECT),
                urlencoding::encode(SCOPE),
                urlencoding::encode(RESOURCE),
            ))
            .header(header::COOKIE, consenting_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(
            authorize.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "authorize"
        );
        let consent_at = location(&authorize);
        assert!(consent_at.starts_with("/ui/oauth/consent?request="));
        let request_id = consent_at.split("request=").nth(1).unwrap().to_string();
        let consent = self
            .client
            .post(format!("{}/ui/api/oauth/consent", self.base))
            .header(header::COOKIE, consenting_cookie)
            .header("Sec-Fetch-Site", "same-origin")
            .form(&[("request_id", request_id.as_str()), ("approved", "true")])
            .send()
            .await
            .unwrap();
        assert_eq!(consent.status(), StatusCode::SEE_OTHER, "consent");
        let back = location(&consent);
        assert!(back.starts_with(REDIRECT), "{back}");
        let code = back
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let code = urlencoding::decode(code).unwrap().into_owned();
        let token = self
            .client
            .post(format!("{}/oauth/token", self.base))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
                ("code_verifier", verifier.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(token.status(), StatusCode::OK, "token exchange");
        let token: Value = token.json().await.unwrap();
        token["access_token"].as_str().unwrap().to_string()
    }

    /// One MCP `tools/call` over `/mcp/streamable`: the decoded tool output,
    /// or the error text.
    async fn tool(&self, token: &str, name: &str, args: Value) -> Result<Value, String> {
        let resp = self
            .client
            .post(format!("{}/mcp/streamable", self.base))
            .bearer_auth(token)
            .header(header::ACCEPT, "application/json")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": name, "arguments": args},
            }))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let body = resp.text().await.unwrap();
        if !status.is_success() {
            return Err(format!("HTTP {status}: {body}"));
        }
        let reply: Value = serde_json::from_str(&body).unwrap();
        if !reply["error"].is_null() {
            return Err(reply["error"].to_string());
        }
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        if result["isError"] == true {
            return Err(text.to_string());
        }
        Ok(serde_json::from_str(text).unwrap_or(Value::String(text.to_string())))
    }

    async fn ok(&self, token: &str, name: &str, args: Value) -> Value {
        self.tool(token, name, args)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// A console request with the person's session.
    async fn ui(
        &self,
        t: &Tenant,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> reqwest::Response {
        let mut req = self
            .client
            .request(method, format!("{}/ui/api{path}", self.base))
            .header(header::COOKIE, &t.cookie)
            .header("Sec-Fetch-Site", "same-origin");
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send().await.unwrap()
    }

    async fn meters(&self, ws: WorkspaceId) -> Vec<(UsageMeter, i64)> {
        let today = chrono::Utc::now().date_naive();
        self.store
            .list_usage_meters(ws, today)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.meter, row.quantity))
            .collect()
    }
}

#[tokio::test]
async fn launch_gate_e2e() {
    let Some((_pg, pool)) = common::postgres_pool().await else {
        return;
    };
    let env = spawn(pool).await;
    let operator = env.operator().await;

    // 1. Two people sign up, each into a workspace of their own.
    let ada = env.sign_up(&operator, "ada").await;
    let grace = env.sign_up(&operator, "grace").await;
    assert_ne!(ada.ws, grace.ws);

    // 2. A channel for the work.
    let channel = env
        .ui(
            &ada,
            reqwest::Method::POST,
            &format!("/workspaces/{}/channels", ada.ws.0),
            Some(json!({"name": "launch"})),
        )
        .await;
    assert!(
        channel.status().is_success(),
        "create channel: {}",
        channel.status()
    );
    let channel: Value = channel.json().await.unwrap();
    let cid = channel["id"].as_str().unwrap().to_string();

    // 3. Each workspace's agent seat connects the client from the listed
    //    host. The token is for MCP only.
    let (agent, agent_cookie) = env.agent_seat(&ada).await;
    let token = env.connect_client(&agent_cookie).await;
    let (_, grace_agent_cookie) = env.agent_seat(&grace).await;
    let grace_token = env.connect_client(&grace_agent_cookie).await;
    let me = env
        .client
        .get(format!("{}/me", env.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::UNAUTHORIZED, "/me");
    let plain_mcp = env
        .client
        .post(format!("{}/mcp", env.base))
        .bearer_auth(&token)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "whoami", "arguments": {}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(plain_mcp.status(), StatusCode::UNAUTHORIZED, "/mcp");

    // 4. The agent runs a change over MCP.
    let mut mcp_calls = 0;
    let thread = env
        .ok(
            &token,
            "create_thread",
            json!({
                "channel_id": cid,
                "title": "Ship the launch",
                "attributes": {
                    "kind": "change",
                    "engine": "session",
                    "acceptance": "the launch gate passes",
                },
            }),
        )
        .await;
    mcp_calls += 1;
    let tid = thread["id"].as_str().unwrap().to_string();
    env.ok(
        &token,
        "set_thread_owner",
        json!({"thread_id": tid, "owner_id": ada.person.0}),
    )
    .await;
    mcp_calls += 1;
    let claimed = env
        .ok(
            &token,
            "claim_next_workspace_thread",
            json!({"workspace_id": ada.ws.0, "engine": "session"}),
        )
        .await;
    mcp_calls += 1;
    assert_eq!(claimed["id"], tid.as_str(), "{claimed}");
    assert_eq!(claimed["assignee_id"], agent.0.to_string(), "{claimed}");
    let lease = claimed["claim_lease_id"].as_str().unwrap().to_string();
    env.ok(
        &token,
        "acknowledge_claim",
        json!({"thread_id": tid, "claim_lease_id": lease}),
    )
    .await;
    mcp_calls += 1;
    env.ok(
        &token,
        "set_thread_result",
        json!({
            "thread_id": tid,
            "claim_lease_id": lease,
            "result": {"summary": "the launch path works end to end"},
        }),
    )
    .await;
    mcp_calls += 1;
    let usage = env
        .ok(
            &token,
            "report_usage",
            json!({
                "thread_id": tid,
                "usage_report_id": uuid::Uuid::now_v7(),
                "claim_lease_id": lease,
                "model": "claude-opus-4-5",
                "tokens": {
                    "input": 1200,
                    "output": 340,
                    "cache_read": 0,
                    "cache_write_5m": 0,
                    "cache_write_1h": 0,
                },
                "usd_micros": 8700,
                "price_snapshot": {
                    "input_usd_micros_per_million": 3_000_000,
                    "output_usd_micros_per_million": 15_000_000,
                    "cache_read_usd_micros_per_million": 300_000,
                    "cache_write_5m_usd_micros_per_million": 3_750_000,
                    "cache_write_1h_usd_micros_per_million": 6_000_000,
                },
                "turns": 1,
            }),
        )
        .await;
    mcp_calls += 1;
    assert_eq!(usage["stopped"], false, "{usage}");
    env.ok(
        &token,
        "transition_thread",
        json!({"thread_id": tid, "action": "start_review"}),
    )
    .await;
    mcp_calls += 1;

    // 5. The agent cannot close its own work.
    let own_close = env
        .tool(
            &token,
            "transition_thread",
            json!({"thread_id": tid, "action": "close"}),
        )
        .await;
    mcp_calls += 1;
    let err = own_close.expect_err("the claimer closing its own owned work is refused");
    assert!(err.contains("separation of duties"), "{err}");

    env.ok(
        &token,
        "release_claim",
        json!({"thread_id": tid, "claim_lease_id": lease}),
    )
    .await;
    mcp_calls += 1;

    // 8. (during) The second tenant's agent is refused on the first's
    //    thread and queue.
    assert!(env
        .tool(&grace_token, "get_thread", json!({"thread_id": tid}))
        .await
        .is_err());
    assert!(env
        .tool(
            &grace_token,
            "claim_next_workspace_thread",
            json!({"workspace_id": ada.ws.0}),
        )
        .await
        .is_err());

    // 6. The person reviews and closes.
    let packet = env
        .ui(
            &ada,
            reqwest::Method::GET,
            &format!("/threads/{tid}/review-packet"),
            None,
        )
        .await;
    assert_eq!(packet.status(), StatusCode::OK);
    let packet: Value = packet.json().await.unwrap();
    let root = packet["evidence_root"].as_str().unwrap().to_string();
    let review = env
        .ui(
            &ada,
            reqwest::Method::POST,
            &format!("/threads/{tid}/reviews"),
            Some(json!({"decision": "approve", "evidence_root": root})),
        )
        .await;
    assert!(review.status().is_success(), "approve: {}", review.status());
    let closed = env
        .ui(
            &ada,
            reqwest::Method::POST,
            &format!("/threads/{tid}"),
            Some(json!({"action": "close"})),
        )
        .await;
    let status = closed.status();
    let closed: Value = closed.json().await.unwrap();
    assert!(status.is_success(), "close: {status}: {closed}");
    let shown = env
        .ui(&ada, reqwest::Method::GET, &format!("/threads/{tid}"), None)
        .await;
    let shown: Value = shown.json().await.unwrap();
    assert_eq!(shown["state"], "closed", "{shown}");

    // 8. The second person sees nothing of it.
    for path in [
        format!("/threads/{tid}"),
        format!("/threads/{tid}/review-packet"),
        format!("/channels/{cid}/threads"),
    ] {
        let resp = env.ui(&grace, reqwest::Method::GET, &path, None).await;
        assert!(
            matches!(resp.status(), StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{path}: {}",
            resp.status()
        );
    }

    // 7. Metering, after a flush: the first workspace's meters count its MCP
    //    calls and requests; the second's count only its own refused calls.
    maidan_server::usage_meters::flush_once(&env.state).await;
    let ada_meters = env.meters(ada.ws).await;
    let count = |m: &[(UsageMeter, i64)], which: UsageMeter| {
        m.iter()
            .find(|(meter, _)| *meter == which)
            .map(|(_, q)| *q)
            .unwrap_or(0)
    };
    assert_eq!(
        count(&ada_meters, UsageMeter::McpCalls),
        mcp_calls,
        "{ada_meters:?}"
    );
    assert!(
        count(&ada_meters, UsageMeter::Requests) >= mcp_calls,
        "{ada_meters:?}"
    );
    let grace_meters = env.meters(grace.ws).await;
    assert!(
        count(&grace_meters, UsageMeter::McpCalls) <= 2,
        "only the second agent's own two calls count for it: {grace_meters:?}"
    );
}
