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
    collections::BTreeMap,
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
    MemberId, MemberKind, NewApiToken, NewChannel, NewMaidanSession, NewMember, NewThread,
    NewWorkspace, UsageMeter, WorkspaceId,
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
    // A workspace made by sign-up opens on #general, so its board can take
    // a task before anyone visits Tools.
    let channels: Value = env
        .ui(
            &ada,
            reqwest::Method::GET,
            &format!("/workspaces/{}/channels", ada.ws.0),
            None,
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        channels
            .as_array()
            .map(|c| c.iter().map(|c| c["name"].clone()).collect::<Vec<_>>()),
        Some(vec![json!("general")]),
        "a new workspace's channels: {channels}"
    );

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

/// The hosted Worker forwards a fixed set of secrets. This boots a server
/// from that set alone (nothing from the process environment) and checks the
/// features those secrets exist for.
#[tokio::test]
async fn hosted_config_features_work_on_the_worker_env() {
    use maidan_auth::init_decrypt_fallback_keys;
    use maidan_server::{
        oidc::{OidcRuntime, OidcSettings},
        session::SessionSettings,
        web_push::WebPushConfig,
        WebhookRuntime,
    };
    use maidan_store::A2aTaskWrite;
    use maidan_types::ContentKeyring;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/cloudflare");
    let server_env_js = std::fs::read_to_string(root.join("src/server-env.js")).unwrap();
    let start = server_env_js.find("export const SERVER_ENV").unwrap();
    let array = &server_env_js[start..];
    let array = &array[array.find('[').unwrap()..];
    let array = &array[..array.find(']').unwrap()];
    let forwarded: Vec<String> = array
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, name)| name.to_string())
        .collect();

    let kek1 = "11".repeat(32);
    let kek2 = "22".repeat(32);
    let fed1 = "33".repeat(32);
    let fed2 = "44".repeat(32);
    let sign1 = "55".repeat(32);
    let sign2 = "66".repeat(32);
    let session = "hosted-session-secret-32-bytes!!";
    let resume = "hosted-resume-secret-32-bytes!!!";
    let (vapid_priv, vapid_pub) = (
        "dQRRiijxL6Nlm0JU2z53WG-anqr7-sUR0OA3vTHrpPM",
        "BNxudIfkQhu4xj9aB9JKgh1z24YsxNWylirQBvf51yaucqBCF1RY7MzW7N7sbUn96dDhxCxej_gOjCYkGZTDRIc",
    );
    let mut bindings = BTreeMap::new();
    let prod = std::fs::read_to_string(root.join("wrangler.prod.jsonc")).unwrap();
    // Documented prod secrets plus the rotation companions and the prod vars.
    for (name, value) in [
        ("DATABASE_URL", "sqlite::memory:"),
        ("MAIDAN_CONTENT_KEK", kek1.as_str()),
        ("MAIDAN_CONTENT_KEK_PREVIOUS", ""),
        ("MAIDAN_SESSION_SECRET", session),
        ("MAIDAN_SUBSCRIBE_RESUME_SECRET", resume),
        ("FEDERATION_ENCRYPTION_KEY", fed1.as_str()),
        ("FEDERATION_DECRYPT_KEYS", ""),
        ("MAIDAN_EXPORT_SIGNING_KEY", sign1.as_str()),
        ("MAIDAN_EXPORT_VERIFY_KEYS", ""),
        ("MAIDAN_VAPID_PRIVATE_KEY", vapid_priv),
        ("MAIDAN_VAPID_PUBLIC_KEY", vapid_pub),
        ("MAIDAN_VAPID_SUBJECT", "mailto:ops@maidan.world"),
        ("MAIDAN_OIDC_CLIENT_SECRET", "oidc-secret"),
        ("S3_ENDPOINT", "https://example.r2.cloudflarestorage.com"),
        ("S3_ACCESS_KEY_ID", "r2-key"),
        ("S3_SECRET_ACCESS_KEY", "r2-secret"),
        ("MAIDAN_ALLOW_INSECURE_NO_AUTH", "1"),
        ("MAIDAN_OIDC_MOCK", "1"),
        ("MAIDAN_BOOTSTRAP", "1"),
    ] {
        bindings.insert(name.to_string(), value.to_string());
    }
    for line in prod
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
    {
        let line = line.trim().trim_end_matches(',');
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().trim_matches('"');
            let value = value.trim().trim_matches('"').trim_matches(',');
            if key.chars().all(|c| c.is_ascii_uppercase() || c == '_') && !value.is_empty() {
                bindings.insert(key.to_string(), value.trim_matches('"').to_string());
            }
        }
    }
    let mut hosted = BTreeMap::new();
    for name in &forwarded {
        if let Some(value) = bindings.get(name).filter(|value| !value.is_empty()) {
            hosted.insert(name.clone(), value.clone());
        }
    }
    for blocked in [
        "MAIDAN_ALLOW_INSECURE_NO_AUTH",
        "MAIDAN_OIDC_MOCK",
        "MAIDAN_BOOTSTRAP",
        "MAIDAN_ALLOW_PRIVATE_EGRESS",
    ] {
        assert!(
            !hosted.contains_key(blocked),
            "{blocked} leaked into the container"
        );
    }
    assert_eq!(hosted["FEDERATION_ENCRYPTION_KEY"], fed1);
    assert_eq!(hosted["MAIDAN_CONTENT_KEK"], kek1);
    assert!(hosted["MAIDAN_PUBLIC_ORIGIN"].contains("maidan.world"));
    // Swarm's internal host is a var, not a secret, and the filter keeps the
    // name the server reads (`MAIDAN_INTERNAL_EGRESS_HOSTS`), not the Worker map.
    let swarm = std::fs::read_to_string(root.join("wrangler.swarm.jsonc")).unwrap();
    assert!(swarm.contains("seats.internal"));
    assert_hosted_boots(&root, &forwarded);

    fn key32(hex_key: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex_key[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    maidan_store::run_sqlite_migrations(&pool).await.unwrap();
    let keys = Arc::new(ContentKeyring::new(key32(&kek1), Vec::new()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone(), keys));
    let fed = Arc::new(key32(&fed1));
    let mut state = hosted_state(store.clone(), fed.clone());
    state.webhooks = WebhookRuntime::new(Some(fed.clone()));
    let signing = maidan_auth::ExportSigningKey::from_seed(key32(&sign1));
    let old_public = signing.public_key_bytes();
    state.attach_export_signing(signing);
    let vapid = WebPushConfig::from_values(
        Some(vapid_priv.into()),
        Some(vapid_pub.into()),
        Some("mailto:ops@maidan.world".into()),
    )
    .unwrap();
    state.attach_web_push_public_key(vapid.public_key_b64());
    state.sessions = Some(SessionSettings {
        secret: Arc::from(session.as_bytes()),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    state.subscribe_resume_secret = Some(Arc::from(resume.as_bytes()));
    state.oidc = Some(Arc::new(OidcRuntime {
        settings: OidcSettings {
            enabled: true,
            mock: false,
            issuer: hosted["MAIDAN_OIDC_ISSUER"].clone(),
            redirect_uri: hosted["MAIDAN_OIDC_REDIRECT_URI"].clone(),
            auto_provision: false,
            link_email: true,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
            post_logout_redirect_uri: None,
            first_admin_mint: false,
            auto_mint: false,
            signup: true,
            signup_email_domains: Vec::new(),
            signup_open: false,
            emails_verified: true,
            privacy_url: Some("/privacy".into()),
            terms_url: Some("/terms".into()),
        },
        session_secret: Arc::from(session.as_bytes()),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    let (addr, client) = serve(state).await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "hosted".into(),
        })
        .await
        .unwrap();
    let person = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "ada".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let token = mint_all(store.as_ref(), ws.id, person.id).await;
    let auth = format!("Bearer {token}");

    let page = client
        .get(&base)
        .header(reqwest::header::ACCEPT, "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = page.text().await.unwrap();
    assert!(
        html.contains("/privacy") && html.contains("/terms"),
        "{html}"
    );

    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("hosted".into()),
            description: None,
            created_by: Some(person.id),
        })
        .await
        .unwrap();
    let posted = client
        .post(format!("{base}/threads/{}/messages", thread.id.0))
        .header("authorization", &auth)
        .json(&json!({"body": "sealed under the first key"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        posted.status(),
        StatusCode::CREATED,
        "{}",
        posted.text().await.unwrap()
    );

    let session_res = client
        .post(format!("{base}/auth/session/from-token"))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(session_res.status(), StatusCode::CREATED);
    let cookie = session_res
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let me = client
        .get(format!("{base}/me"))
        .header(reqwest::header::COOKIE, &cookie)
        .header("sec-fetch-site", "same-origin")
        .header(reqwest::header::ORIGIN, &base)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK, "{}", me.text().await.unwrap());

    let webhook = client
        .post(format!("{base}/workspaces/{}/webhooks", ws.id.0))
        .header("authorization", &auth)
        .json(&json!({"url": "https://hooks.example/wake", "event_kinds": ["thread_ready"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        webhook.status(),
        StatusCode::CREATED,
        "{}",
        webhook.text().await.unwrap()
    );
    unsafe { std::env::set_var("MAIDAN_INTERNAL_EGRESS_HOSTS", "seats.internal") };
    let internal = client
        .post(format!("{base}/workspaces/{}/webhooks", ws.id.0))
        .header("authorization", &auth)
        .json(&json!({"url": "http://seats.internal/wake", "event_kinds": ["thread_ready"]}))
        .send()
        .await
        .unwrap();
    unsafe { std::env::remove_var("MAIDAN_INTERNAL_EGRESS_HOSTS") };
    assert_eq!(
        internal.status(),
        StatusCode::CREATED,
        "{}",
        internal.text().await.unwrap()
    );

    let peer = client
        .post(format!("{base}/workspaces/{}/peers", ws.id.0))
        .header("authorization", &auth)
        .json(&json!({"name": "east", "base_url": "https://east.example"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        peer.status(),
        StatusCode::CREATED,
        "{}",
        peer.text().await.unwrap()
    );

    let secret = client
        .post(format!("{base}/workspaces/{}/secrets", ws.id.0))
        .header("authorization", &auth)
        .json(&json!({"name": "ship", "value": "the-value"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        secret.status(),
        StatusCode::CREATED,
        "{}",
        secret.text().await.unwrap()
    );

    store
        .upsert_a2a_task(A2aTaskWrite {
            workspace_id: ws.id,
            task_id: "hosted-task",
            context_id: Some("hosted-ctx"),
            thread_id: None,
            state: "TASK_STATE_SUBMITTED",
            status_at: chrono::Utc::now(),
            task_json: json!({
                "id": "hosted-task",
                "contextId": "hosted-ctx",
                "status": {"state": "TASK_STATE_SUBMITTED"}
            }),
        })
        .await
        .unwrap();
    let push = client
        .post(format!("{base}/a2a/v1/tasks/hosted-task/pushNotificationConfigs"))
        .header("authorization", &auth)
        .header("A2A-Version", "1.0")
        .json(&json!({"url": "https://hooks.example/a2a", "token": "tok", "authentication": {"scheme": "Bearer", "credentials": "cred"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        push.status(),
        StatusCode::OK,
        "{}",
        push.text().await.unwrap()
    );

    let exported = client
        .get(format!("{base}/workspaces/{}/export", ws.id.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(exported.status(), StatusCode::OK);
    let envelope: Value = exported.json().await.unwrap();

    let vapid_body = client
        .get(format!("{base}/web-push/vapid-public-key"))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(vapid_body.status(), StatusCode::OK);
    let vapid_body: Value = vapid_body.json().await.unwrap();
    assert_eq!(vapid_body["public_key"], json!(vapid.public_key_b64()));

    let filter = maidan_types::EventFilter::workspace(ws.id);
    let resume_token =
        maidan_auth::subscribe::sign_resume_token(&filter, 1, resume.as_bytes(), 3600).unwrap();

    let entry = store
        .insert_waitlist_entry(maidan_types::NewWaitlistEntry {
            email: "ada@example.com".into(),
            use_case: "try the hosted server".into(),
            consented_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    let invite = client
        .post(format!("{base}/operator/waitlist/{}/invite", entry.id.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(
        invite.status(),
        StatusCode::OK,
        "{}",
        invite.text().await.unwrap()
    );

    // A fresh process. The new keys arrive only if `server-env.js` forwards
    // the rotation companions; the filter is the same whitelist as `serverEnv`.
    let old_public_hex: String = old_public
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut rotated = hosted.clone();
    for (name, value) in [
        ("MAIDAN_CONTENT_KEK", kek2.as_str()),
        ("MAIDAN_CONTENT_KEK_PREVIOUS", kek1.as_str()),
        ("FEDERATION_ENCRYPTION_KEY", fed2.as_str()),
        ("FEDERATION_DECRYPT_KEYS", fed1.as_str()),
        ("MAIDAN_EXPORT_SIGNING_KEY", sign2.as_str()),
        ("MAIDAN_EXPORT_VERIFY_KEYS", old_public_hex.as_str()),
    ] {
        rotated.insert(name.to_string(), value.to_string());
    }
    rotated.retain(|name, value| forwarded.iter().any(|kept| kept == name) && !value.is_empty());
    let keys2 = Arc::new(ContentKeyring::new(
        key32(&rotated["MAIDAN_CONTENT_KEK"]),
        vec![key32(&rotated["MAIDAN_CONTENT_KEK_PREVIOUS"])],
    ));
    let store2: Arc<dyn Store> = Arc::new(SqliteStore::new(pool, keys2));
    let fed2_key = Arc::new(key32(&rotated["FEDERATION_ENCRYPTION_KEY"]));
    init_decrypt_fallback_keys(vec![key32(&rotated["FEDERATION_DECRYPT_KEYS"])]);
    let mut state2 = hosted_state(store2.clone(), fed2_key.clone());
    state2.webhooks = WebhookRuntime::new(Some(fed2_key.clone()));
    let signing2 =
        maidan_auth::ExportSigningKey::from_seed(key32(&rotated["MAIDAN_EXPORT_SIGNING_KEY"]));
    let signing2_public = signing2.public_key_bytes();
    state2.attach_export_signing(signing2);
    let pinned = key32(&rotated["MAIDAN_EXPORT_VERIFY_KEYS"]);
    assert_eq!(pinned, old_public);
    state2.attach_export_verify_keys(vec![pinned, signing2_public]);
    state2.subscribe_resume_secret = Some(Arc::from(resume.as_bytes()));
    let (addr2, client2) = serve(state2).await;
    let base2 = format!("http://{addr2}");
    let messages = client2
        .get(format!("{base2}/threads/{}/messages", thread.id.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    let messages = messages.text().await.unwrap();
    assert!(
        messages.contains("sealed under the first key"),
        "{messages}"
    );
    let again = client2
        .post(format!("{base2}/threads/{}/messages", thread.id.0))
        .header("authorization", &auth)
        .json(&json!({"body": "sealed under the second key"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        again.status(),
        StatusCode::CREATED,
        "{}",
        again.text().await.unwrap()
    );

    let resolved = client2
        .post(format!(
            "{base2}/workspaces/{}/secrets/ship/resolve",
            ws.id.0
        ))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resolved.status(),
        StatusCode::OK,
        "{}",
        resolved.text().await.unwrap()
    );
    let resolved: Value = resolved.json().await.unwrap();
    assert_eq!(resolved["value"], "the-value");
    let (verified, after_id) =
        maidan_auth::subscribe::verify_resume_token(&resume_token, resume.as_bytes()).unwrap();
    assert_eq!(verified.workspace_id, Some(ws.id));
    assert_eq!(after_id, 1);

    let verify = client2
        .post(format!("{base2}/workspaces/export/verify"))
        .header("authorization", &auth)
        .json(&envelope)
        .send()
        .await
        .unwrap();
    assert_eq!(
        verify.status(),
        StatusCode::OK,
        "{}",
        verify.text().await.unwrap()
    );
}

/// `unknown_vars` plus the production outbox rule, for each hosted instance,
/// using only the Worker whitelist. Ambient process env is not consulted.
fn assert_hosted_boots(root: &std::path::Path, forwarded: &[String]) {
    let secrets = [
        (
            "DATABASE_URL",
            "postgres://hosted.example/maidan".to_string(),
        ),
        ("MAIDAN_CONTENT_KEK", "11".repeat(32)),
        (
            "MAIDAN_SESSION_SECRET",
            "hosted-session-secret-32-bytes!!".to_string(),
        ),
        ("FEDERATION_ENCRYPTION_KEY", "33".repeat(32)),
        ("MAIDAN_EXPORT_SIGNING_KEY", "55".repeat(32)),
        (
            "S3_ENDPOINT",
            "https://example.r2.cloudflarestorage.com".to_string(),
        ),
        ("S3_ACCESS_KEY_ID", "r2-key".to_string()),
        ("S3_SECRET_ACCESS_KEY", "r2-secret".to_string()),
    ];
    let insecure = [
        "MAIDAN_ALLOW_INSECURE_DEV_KEK",
        "MAIDAN_ALLOW_INSECURE_NO_AUTH",
        "MAIDAN_ALLOW_PRIVATE_EGRESS",
        "MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE",
        "MAIDAN_OIDC_MOCK",
        "MAIDAN_OPS_FAULT",
        "MAIDAN_BOOTSTRAP",
    ];
    for file in [
        "wrangler.prod.jsonc",
        "wrangler.jsonc",
        "wrangler.swarm.jsonc",
    ] {
        let mut bindings = assignments(&std::fs::read_to_string(root.join(file)).unwrap());
        for (name, value) in &secrets {
            bindings.insert((*name).to_string(), value.clone());
        }
        for name in insecure {
            bindings.insert(name.to_string(), "1".to_string());
        }
        let passed = passed_through(forwarded, &bindings);
        for name in insecure {
            assert!(!passed.contains_key(name), "{file} forwarded {name}");
        }
        assert!(passed.contains_key("FEDERATION_ENCRYPTION_KEY"), "{file}");
        assert!(passed.contains_key("MAIDAN_EXPORT_SIGNING_KEY"), "{file}");
        let unknown = maidan_env::unknown_vars(
            bindings
                .keys()
                .filter(|name| name.starts_with("MAIDAN_"))
                .cloned(),
            false,
        );
        assert!(unknown.is_empty(), "{file}: {unknown:?}");
        maidan_server::outbox_relay::validate_startup(true, true).unwrap();
        if file == "wrangler.swarm.jsonc" {
            assert_eq!(passed["MAIDAN_INTERNAL_EGRESS_HOSTS"], "seats.internal");
            assert!(!passed.contains_key("MAIDAN_INTERNAL_HOSTS"));
        }
    }
    let demo = std::fs::read_to_string(root.join("src/demo-config.js")).unwrap();
    let demo_env = assignments(&demo);
    assert!(demo_env.contains_key("MAIDAN_DEMO_INSTANCE"));
    for name in insecure {
        assert!(!demo_env.contains_key(name), "demo sets {name}");
    }
    let unknown = maidan_env::unknown_vars(
        demo_env
            .keys()
            .filter(|name| name.starts_with("MAIDAN_"))
            .cloned(),
        false,
    );
    assert!(unknown.is_empty(), "try.maidan.world: {unknown:?}");
    let demo_passed = passed_through(forwarded, &demo_env);
    assert!(!demo_passed
        .keys()
        .any(|name| name.starts_with("MAIDAN_DEMO_")));
}

fn assignments(src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in src
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
    {
        let line = line.trim().trim_end_matches(',');
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            continue;
        }
        let value = value
            .trim()
            .trim_matches(|c: char| c == '"' || c == ',' || c.is_whitespace());
        if !value.is_empty() {
            out.insert(key.to_string(), value.to_string());
        }
    }
    out
}

fn passed_through(
    forwarded: &[String],
    bindings: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    bindings
        .iter()
        .filter(|(name, value)| forwarded.iter().any(|kept| kept == *name) && !value.is_empty())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn hosted_state(store: Arc<dyn Store>, key: Arc<[u8; 32]>) -> AppState {
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let search_pool = maidan_search::sqlite_pool_options()
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(search_pool));
    AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(false, Some(key)),
        Arc::new(AtomicI64::new(0)),
        None,
    )
}

async fn serve(state: AppState) -> (SocketAddr, reqwest::Client) {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new())
}

async fn mint_all(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: maidan_auth::capability::all(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}
