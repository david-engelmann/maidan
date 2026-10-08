//! `approval_decide`: a model asks to accept or decline an approval gate over
//! MCP (Open Work Next 17, built on #1325's accepting credential).
//!
//! - Declining works on any member credential that may write.
//! - Accepting happens directly only for a token holding `approval:grant`, on
//!   a gate whose risk is below the workspace's confirmation threshold.
//! - Every other accept gets a "confirmation required" result with a one-time
//!   console link (or a URL-mode elicitation, where the client declared one),
//!   which only the bound person's signed-in session, sent from the console
//!   page, can spend. The model can never finish it.
//! - The gate and the audit row record the client and that a model asked.
//!
//! Runs with auth enabled, sessions configured, and a console origin, as in
//! production.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, ApprovalRisk, MemberId, MemberKind,
    NewApiToken, NewApprovalGate, NewMaidanSession, NewMember, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

const SESSION_SECRET: &[u8] = b"approval-decide-e2e-session-secret-32";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: SqlitePool,
}

async fn spawn() -> Env {
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(&b"approval-decide-e2e-secret-key-01!"[..]));
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    state.console_origin = Some(base.clone());
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Env {
        base,
        client: reqwest::Client::new(),
        store,
        pool,
    }
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str, kind: MemberKind) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind,
        })
        .await
        .unwrap()
        .id
}

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

const WRITE: &[&str] = &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE];
const GRANT: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::APPROVAL_GRANT,
];

async fn open_gate(
    store: &dyn Store,
    ws: WorkspaceId,
    requested_by: MemberId,
    risk: ApprovalRisk,
) -> ApprovalGate {
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws,
            thread_id: None,
            requested_by,
            prompt: "Deploy v9 to prod?".into(),
            schema: None,
            risk,
        })
        .await
        .unwrap()
}

/// The cookie of a session a person signed in to (no token behind it).
async fn signed_in(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let session = store
        .create_session(NewMaidanSession {
            workspace_id: ws,
            member_id: member,
            api_token_id: None,
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
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// How a call names its MCP revision and client.
#[derive(Clone, Copy)]
enum Client {
    /// A pre-2026 stateless call: no `_meta`, so no client is known.
    Legacy,
    /// A `2026-07-28` call from "test-client 1.2.3" with the given
    /// elicitation capability (`None` for none).
    Current(Option<&'static str>),
}

impl Env {
    async fn decide(&self, bearer: &str, client: Client, args: Value) -> Value {
        self.decide_with(bearer, client, args, None).await
    }

    async fn decide_with(
        &self,
        bearer: &str,
        client: Client,
        args: Value,
        input_responses: Option<Value>,
    ) -> Value {
        let mut params = json!({ "name": "approval_decide", "arguments": args });
        let mut req = self
            .client
            .post(format!("{}/mcp", self.base))
            .bearer_auth(bearer);
        if let Client::Current(elicitation) = client {
            let caps = match elicitation {
                Some("url") => json!({ "elicitation": { "url": {} } }),
                Some("form") => json!({ "elicitation": {} }),
                _ => json!({}),
            };
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": caps,
                "io.modelcontextprotocol/clientInfo": { "name": "test-client", "version": "1.2.3" },
            });
            req = req
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "approval_decide");
        }
        if let Some(responses) = input_responses {
            params["inputResponses"] = responses;
        }
        req.json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// POST the confirmation from a cookie session. `from_console` sends what
    /// a browser sends for a fetch from the console page.
    async fn confirm(&self, cookie: &str, from_console: bool, body: Value) -> reqwest::Response {
        let mut b = self
            .client
            .post(format!("{}/auth/approval-confirmations/confirm", self.base))
            .header(reqwest::header::COOKIE, cookie);
        if from_console {
            b = b
                .header("sec-fetch-site", "same-origin")
                .header(reqwest::header::ORIGIN, self.base.clone());
        }
        b.json(&body).send().await.unwrap()
    }

    async fn gate(&self, id: ApprovalGateId) -> ApprovalGate {
        self.store.get_approval_gate(id).await.unwrap().unwrap()
    }

    async fn audit(&self, ws: WorkspaceId, action: &str) -> Vec<Value> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT metadata FROM maidan_audit WHERE workspace_id = ? AND action = ? ORDER BY id",
        )
        .bind(ws.0)
        .bind(action)
        .fetch_all(&self.pool)
        .await
        .unwrap();
        rows.into_iter()
            .map(|(m,)| serde_json::from_str(&m).unwrap())
            .collect()
    }
}

/// The tool's JSON payload, from a successful result.
fn payload(res: &Value) -> Value {
    assert!(res.get("error").is_none(), "error: {res}");
    assert_eq!(res["result"]["isError"], false, "tool error: {res}");
    let text = res["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn refused(res: &Value) -> String {
    if let Some(e) = res.get("error") {
        return e["message"].as_str().unwrap_or_default().to_string();
    }
    assert_eq!(res["result"]["isError"], true, "expected a refusal: {res}");
    res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// `(gate_id, token)` from a confirmation link.
fn link_parts(url: &str) -> (String, String) {
    let fragment = url.split("#confirm-approval=").nth(1).unwrap();
    let (gate, token) = fragment.split_once('.').unwrap();
    (gate.to_string(), token.to_string())
}

struct Team {
    ws: WorkspaceId,
    requester: MemberId,
    approver: MemberId,
    approver_write: String,
    approver_grant: String,
}

async fn team(env: &Env, name: &str) -> Team {
    let s = env.store.as_ref();
    let ws = workspace(s, name).await;
    let requester = member(s, ws, "requester", MemberKind::Human).await;
    let approver = member(s, ws, "approver", MemberKind::Human).await;
    Team {
        ws,
        requester,
        approver,
        approver_write: mint(s, ws, approver, WRITE).await,
        approver_grant: mint(s, ws, approver, GRANT).await,
    }
}

async fn set_policy(env: &Env, t: &Team, confirm_at: &str) {
    let admin = mint(
        env.store.as_ref(),
        t.ws,
        t.approver,
        &[capability::WORKSPACE_READ, capability::TOKEN_ADMIN],
    )
    .await;
    let res = env
        .client
        .put(format!("{}/workspaces/{}/approval-policy", env.base, t.ws))
        .bearer_auth(&admin)
        .json(&json!({ "confirm_at": confirm_at }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "{}",
        res.text().await.unwrap()
    );
}

/// The headline case: a plain human bearer (no `approval:grant`) asking to
/// accept gets the confirmation path, and the gate stays pending.
#[tokio::test]
async fn a_plain_human_bearer_gets_the_confirmation_path_not_an_acceptance() {
    let env = spawn().await;
    let t = team(&env, "plain").await;
    // Even a low-risk gate under the most permissive policy: the credential,
    // not the risk, is what is missing.
    set_policy(&env, &t, "high").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::Low).await;

    let res = env
        .decide(
            &t.approver_write,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await;
    let out = payload(&res);
    assert_eq!(out["status"], "confirmation_required", "{out}");
    assert_eq!(out["reused"], false);
    let url = out["confirmation_url"].as_str().unwrap();
    assert!(
        url.starts_with(&format!("{}/ui/#confirm-approval={}.", env.base, gate.id)),
        "{url}"
    );
    assert!(
        !env.gate(gate.id).await.state.is_resolved(),
        "a bearer's accept resolved the gate"
    );

    // The request is audited with the client and that a model asked.
    let rows = env
        .audit(t.ws, "approval_gate.confirmation_requested")
        .await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["client_name"], "test-client");
    assert_eq!(rows[0]["client_version"], "1.2.3");
    assert_eq!(rows[0]["model_asked"], true);

    // The pending list names the request on the gate's card.
    let list: Value = env
        .client
        .get(format!("{}/workspaces/{}/approval-gates", env.base, t.ws))
        .bearer_auth(&t.approver_write)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["gate"]["id"] == json!(gate.id))
        .unwrap();
    assert_eq!(row["model_request"]["client_name"], "test-client");
    assert_eq!(row["model_request"]["member_id"], json!(t.approver));
    assert!(
        !row.to_string().contains(&link_parts(url).1),
        "the list leaked the link's token"
    );
}

/// An OAuth-style token without approval:grant (any token, however issued)
/// is a plain bearer here, on a legacy call too.
#[tokio::test]
async fn a_legacy_call_without_approval_grant_gets_the_link_in_the_result() {
    let env = spawn().await;
    let t = team(&env, "legacy").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::Low).await;
    let out = payload(
        &env.decide(
            &t.approver_write,
            Client::Legacy,
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    assert_eq!(out["status"], "confirmation_required");
    let rows = env
        .audit(t.ws, "approval_gate.confirmation_requested")
        .await;
    assert_eq!(rows[0]["client_name"], Value::Null, "no client was named");
}

#[tokio::test]
async fn approval_grant_accepts_below_the_threshold_and_confirms_at_or_above() {
    let env = spawn().await;
    let t = team(&env, "grant").await;

    // The default threshold is low: every model acceptance needs a person.
    let low = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::Low).await;
    let out = payload(
        &env.decide(
            &t.approver_grant,
            Client::Current(None),
            json!({ "gate_id": low.id, "decision": "accept" }),
        )
        .await,
    );
    assert_eq!(out["status"], "confirmation_required", "{out}");

    // Raise it to high: medium is now below it and accepts directly.
    set_policy(&env, &t, "high").await;
    let medium = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::Medium).await;
    let out = payload(
        &env.decide(
            &t.approver_grant,
            Client::Current(None),
            json!({ "gate_id": medium.id, "decision": "accept", "note": "ship it" }),
        )
        .await,
    );
    assert_eq!(out["status"], "accepted", "{out}");
    let decided = env.gate(medium.id).await;
    assert_eq!(decided.resolved_by, Some(t.approver));
    let via = decided.decided_via.expect("decided_via");
    assert_eq!(via.client_name.as_deref(), Some("test-client"));
    assert_eq!(via.client_version.as_deref(), Some("1.2.3"));
    assert!(via.model_asked);
    let rows = env.audit(t.ws, "approval_gate.decided").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["path"], "approval:grant");
    assert_eq!(rows[0]["model_asked"], true);
    assert_eq!(rows[0]["client_name"], "test-client");

    // At the threshold, approval:grant still gets the confirmation path.
    let high = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::High).await;
    let out = payload(
        &env.decide(
            &t.approver_grant,
            Client::Current(None),
            json!({ "gate_id": high.id, "decision": "accept" }),
        )
        .await,
    );
    assert_eq!(out["status"], "confirmation_required", "{out}");
    assert!(!env.gate(high.id).await.state.is_resolved());
}

#[tokio::test]
async fn the_policy_is_read_by_members_and_set_by_admins_only() {
    let env = spawn().await;
    let t = team(&env, "policy").await;
    let url = format!("{}/workspaces/{}/approval-policy", env.base, t.ws);
    let got: Value = env
        .client
        .get(&url)
        .bearer_auth(&t.approver_write)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["confirm_at"], "low");
    assert_eq!(got["is_default"], true);
    let res = env
        .client
        .put(&url)
        .bearer_auth(&t.approver_grant)
        .json(&json!({ "confirm_at": "high" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "a non-admin set the policy"
    );
    set_policy(&env, &t, "medium").await;
    let got: Value = env
        .client
        .get(&url)
        .bearer_auth(&t.approver_write)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["confirm_at"], "medium");
    assert_eq!(got["is_default"], false);
    assert_eq!(env.audit(t.ws, "approval_policy.set").await.len(), 1);
}

#[tokio::test]
async fn a_plain_bearer_declines_with_a_note() {
    let env = spawn().await;
    let t = team(&env, "decline").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::High).await;
    let out = payload(
        &env.decide(
            &t.approver_write,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "decline", "note": "not this week" }),
        )
        .await,
    );
    assert_eq!(out["status"], "declined", "{out}");
    let g = env.gate(gate.id).await;
    assert_eq!(g.content, Some(json!({ "note": "not this week" })));
    assert!(g.decided_via.unwrap().model_asked);
    // A second call reads the outcome, not an error.
    let out = payload(
        &env.decide(
            &t.approver_write,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    assert_eq!(out["status"], "already_resolved");
    assert_eq!(out["state"], "declined");
}

#[tokio::test]
async fn nobody_accepts_their_own_request() {
    let env = spawn().await;
    let t = team(&env, "self").await;
    set_policy(&env, &t, "high").await;
    let own = mint(env.store.as_ref(), t.ws, t.requester, GRANT).await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::Low).await;
    let msg = refused(
        &env.decide(
            &own,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    assert!(msg.contains("whoever requested it"), "{msg}");
    // Nor through the confirmation path: no link is minted for one's own gate.
    let own_plain = mint(env.store.as_ref(), t.ws, t.requester, WRITE).await;
    refused(
        &env.decide(
            &own_plain,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    assert!(env
        .audit(t.ws, "approval_gate.confirmation_requested")
        .await
        .is_empty());
    assert!(!env.gate(gate.id).await.state.is_resolved());
}

#[tokio::test]
async fn an_agent_without_approval_grant_cannot_get_a_link() {
    let env = spawn().await;
    let t = team(&env, "agent").await;
    let bot = member(env.store.as_ref(), t.ws, "bot", MemberKind::Agent).await;
    let bot_token = mint(env.store.as_ref(), t.ws, bot, WRITE).await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::Low).await;
    let msg = refused(
        &env.decide(
            &bot_token,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    assert!(msg.contains("no person can confirm"), "{msg}");
}

/// The link: only the bound person's signed-in session, from the console
/// page, spends it, once.
#[tokio::test]
async fn the_link_is_confirmed_once_by_the_bound_persons_session() {
    let env = spawn().await;
    let t = team(&env, "confirm").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::High).await;
    let out = payload(
        &env.decide(
            &t.approver_write,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept", "note": "looks right" }),
        )
        .await,
    );
    let (gid, token) = link_parts(out["confirmation_url"].as_str().unwrap());
    assert_eq!(gid, gate.id.to_string());
    let body = json!({ "gate_id": gate.id, "token": token });

    // A bearer cannot spend it, even one holding approval:grant: the model
    // has the bearer.
    let res = env
        .client
        .post(format!("{}/auth/approval-confirmations/confirm", env.base))
        .bearer_auth(&t.approver_grant)
        .header(reqwest::header::ORIGIN, env.base.clone())
        .json(&body)
        .send()
        .await
        .unwrap();
    // The route is on the session-only tree: a bearer is not a session.
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Nor a session made from the bearer.
    let res = env
        .client
        .post(format!("{}/auth/session/from-token", env.base))
        .bearer_auth(&t.approver_grant)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let token_cookie = res
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let res = env.confirm(&token_cookie, true, body.clone()).await;
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "a token's session confirmed"
    );

    let cookie = signed_in(env.store.as_ref(), t.ws, t.approver).await;
    // The signed-in session without the console's origin (curl with the
    // cookie) is refused by the strict origin check.
    let res = env.confirm(&cookie, false, body.clone()).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Someone else in the workspace, signed in, cannot spend a link bound to
    // the approver, and learns nothing: the same 404 as a wrong token.
    let other = member(env.store.as_ref(), t.ws, "other", MemberKind::Human).await;
    let other_cookie = signed_in(env.store.as_ref(), t.ws, other).await;
    let res = env.confirm(&other_cookie, true, body.clone()).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = env
        .confirm(
            &cookie,
            true,
            json!({ "gate_id": gate.id, "token": "0".repeat(64) }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    // The link's token on another gate id is not found either.
    let res = env
        .confirm(
            &cookie,
            true,
            json!({ "gate_id": uuid::Uuid::now_v7(), "token": body["token"] }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(!env.gate(gate.id).await.state.is_resolved());

    // The bound person, signed in, from the console: accepted.
    let res = env.confirm(&cookie, true, body.clone()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let g: Value = res.json().await.unwrap();
    assert_eq!(g["state"], "accepted");
    assert_eq!(g["decided_via"]["client_name"], "test-client");
    assert_eq!(g["decided_via"]["model_asked"], true);
    assert_eq!(g["content"], json!({ "note": "looks right" }));
    let stored = env.gate(gate.id).await;
    assert_eq!(stored.resolved_by, Some(t.approver));
    let rows = env.audit(t.ws, "approval_gate.decided").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["path"], "confirmation");
    assert_eq!(rows[0]["model_asked"], true);
    assert_eq!(rows[0]["client_name"], "test-client");

    // Single use.
    let res = env.confirm(&cookie, true, body.clone()).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    // The model reads the outcome.
    let out = payload(
        &env.decide(
            &t.approver_write,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    assert_eq!(out["status"], "already_resolved");
    assert_eq!(out["state"], "accepted");
}

/// A repeat ask while a link is live gets that link back: one link per gate
/// and person, not a new one per call.
#[tokio::test]
async fn a_repeat_ask_reuses_the_live_link() {
    let env = spawn().await;
    let t = team(&env, "dedupe").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::High).await;
    let args = json!({ "gate_id": gate.id, "decision": "accept" });
    let first = payload(
        &env.decide(&t.approver_write, Client::Current(None), args.clone())
            .await,
    );
    let second = payload(
        &env.decide(&t.approver_write, Client::Legacy, args.clone())
            .await,
    );
    assert_eq!(first["confirmation_url"], second["confirmation_url"]);
    assert_eq!(second["reused"], true);
    assert_eq!(first["expires_at"], second["expires_at"]);
    assert_eq!(
        env.audit(t.ws, "approval_gate.confirmation_requested")
            .await
            .len(),
        1,
        "a reused link is not a new request"
    );
}

#[tokio::test]
async fn an_expired_link_is_refused_and_a_new_ask_mints_a_new_one() {
    let env = spawn().await;
    let t = team(&env, "expiry").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::High).await;
    let args = json!({ "gate_id": gate.id, "decision": "accept" });
    let first = payload(
        &env.decide(&t.approver_write, Client::Current(None), args.clone())
            .await,
    );
    let (_, token) = link_parts(first["confirmation_url"].as_str().unwrap());
    sqlx::query("UPDATE maidan_approval_confirmations SET expires_at = '2000-01-01T00:00:00.000Z'")
        .execute(&env.pool)
        .await
        .unwrap();
    let cookie = signed_in(env.store.as_ref(), t.ws, t.approver).await;
    let res = env
        .confirm(&cookie, true, json!({ "gate_id": gate.id, "token": token }))
        .await;
    assert_eq!(
        res.status(),
        StatusCode::NOT_FOUND,
        "an expired link confirmed"
    );
    assert!(!env.gate(gate.id).await.state.is_resolved());

    let second = payload(
        &env.decide(&t.approver_write, Client::Current(None), args)
            .await,
    );
    assert_eq!(second["reused"], false);
    assert_ne!(first["confirmation_url"], second["confirmation_url"]);
    let (_, fresh) = link_parts(second["confirmation_url"].as_str().unwrap());
    // The old token stays dead; the new one works.
    let res = env
        .confirm(&cookie, true, json!({ "gate_id": gate.id, "token": token }))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = env
        .confirm(&cookie, true, json!({ "gate_id": gate.id, "token": fresh }))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_link_is_refused_once_the_gate_resolves() {
    let env = spawn().await;
    let t = team(&env, "resolved").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::High).await;
    let out = payload(
        &env.decide(
            &t.approver_write,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    let (_, token) = link_parts(out["confirmation_url"].as_str().unwrap());
    // Someone else declines it in the meantime.
    let other = member(env.store.as_ref(), t.ws, "other", MemberKind::Human).await;
    let other_write = mint(env.store.as_ref(), t.ws, other, WRITE).await;
    let out = payload(
        &env.decide(
            &other_write,
            Client::Legacy,
            json!({ "gate_id": gate.id, "decision": "decline" }),
        )
        .await,
    );
    assert_eq!(out["status"], "declined");
    let cookie = signed_in(env.store.as_ref(), t.ws, t.approver).await;
    let res = env
        .confirm(&cookie, true, json!({ "gate_id": gate.id, "token": token }))
        .await;
    assert!(
        matches!(res.status(), StatusCode::NOT_FOUND | StatusCode::CONFLICT),
        "{}",
        res.status()
    );
    assert_eq!(env.gate(gate.id).await.state, ApprovalGateState::Declined);
}

/// Two tenants: B can neither use nor probe A's link or gate.
#[tokio::test]
async fn another_workspace_cannot_use_or_probe_the_link_or_the_gate() {
    let env = spawn().await;
    let a = team(&env, "tenant-a").await;
    let b = team(&env, "tenant-b").await;
    let gate = open_gate(env.store.as_ref(), a.ws, a.requester, ApprovalRisk::High).await;
    let out = payload(
        &env.decide(
            &a.approver_write,
            Client::Current(None),
            json!({ "gate_id": gate.id, "decision": "accept" }),
        )
        .await,
    );
    let (_, token) = link_parts(out["confirmation_url"].as_str().unwrap());

    // B's signed-in session with A's whole link: the same 404 as nonsense.
    let b_cookie = signed_in(env.store.as_ref(), b.ws, b.approver).await;
    let res = env
        .confirm(
            &b_cookie,
            true,
            json!({ "gate_id": gate.id, "token": token }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let text = res.text().await.unwrap();
    assert!(!text.contains("Deploy v9"), "{text}");

    // B's tokens on A's gate, either decision: "no such gate", as for an id
    // that does not exist.
    for (bearer, decision) in [
        (&b.approver_grant, "accept"),
        (&b.approver_write, "decline"),
    ] {
        let msg = refused(
            &env.decide(
                bearer,
                Client::Current(None),
                json!({ "gate_id": gate.id, "decision": decision }),
            )
            .await,
        );
        let missing = refused(
            &env.decide(
                bearer,
                Client::Current(None),
                json!({ "gate_id": uuid::Uuid::now_v7(), "decision": decision }),
            )
            .await,
        );
        assert_eq!(msg, missing, "B can tell A's gate from no gate");
    }
    // B cannot read or set A's policy.
    let res = env
        .client
        .get(format!("{}/workspaces/{}/approval-policy", env.base, a.ws))
        .bearer_auth(&b.approver_write)
        .send()
        .await
        .unwrap();
    assert!(!res.status().is_success());
    assert!(!env.gate(gate.id).await.state.is_resolved());
    assert!(env
        .audit(b.ws, "approval_gate.confirmation_requested")
        .await
        .is_empty());
}

/// Where the client declared URL-mode elicitation on `2026-07-28`, the link
/// comes as an `input_required` URL elicitation, never a form.
#[tokio::test]
async fn a_client_with_url_elicitation_is_asked_to_open_the_link() {
    let env = spawn().await;
    let t = team(&env, "elicit").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, ApprovalRisk::High).await;
    let args = json!({ "gate_id": gate.id, "decision": "accept" });

    let res = env
        .decide(
            &t.approver_write,
            Client::Current(Some("url")),
            args.clone(),
        )
        .await;
    let result = &res["result"];
    assert_eq!(result["resultType"], "input_required", "{res}");
    let ask = &result["inputRequests"]["confirm_in_console"];
    assert_eq!(ask["method"], "elicitation/create");
    assert_eq!(ask["params"]["mode"], "url");
    assert!(ask["params"].get("requestedSchema").is_none(), "a form");
    let url = ask["params"]["url"].as_str().unwrap();
    assert!(url.starts_with(&format!("{}/ui/#confirm-approval={}.", env.base, gate.id)));
    assert!(!env.gate(gate.id).await.state.is_resolved());

    // The retry answering it gets the plain result with the same link, never
    // another elicitation, and an "accept" in the response changes nothing.
    let retry = payload(
        &env.decide_with(
            &t.approver_write,
            Client::Current(Some("url")),
            args.clone(),
            Some(json!({ "confirm_in_console": { "action": "accept" } })),
        )
        .await,
    );
    assert_eq!(retry["status"], "confirmation_required");
    assert_eq!(retry["confirmation_url"], url);
    assert!(!env.gate(gate.id).await.state.is_resolved());

    // A form-only client, and a client with no elicitation, get the result.
    for client in [Client::Current(Some("form")), Client::Current(None)] {
        let res = env.decide(&t.approver_write, client, args.clone()).await;
        assert_eq!(res["result"]["resultType"], "complete", "{res}");
        assert_eq!(payload(&res)["status"], "confirmation_required");
    }
}
