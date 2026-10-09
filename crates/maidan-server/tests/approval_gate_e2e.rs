//! The held gate over HTTP: a human lists pending approval gates (each with a
//! server-issued `request_state`) and answers one accept/decline/cancel. Runs
//! with auth ENABLED so `resolved_by` is a real member and the HMAC
//! `request_state` has a configured secret. Covers the CAS no-op on a
//! double-answer (silence is not consent), a tampered `request_state` (403),
//! and an unknown action (400).
//!
//! Accepting is a property of the credential (Next 17): a browser session a
//! person signed in to, sent from the console page, or a token holding
//! `approval:grant`. A plain human bearer token, or a session made from one,
//! declines and cancels but does not accept.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGate, ApprovalGateState, MemberId, MemberKind, NewApiToken, NewApprovalGate,
    NewMaidanSession, NewMember, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

const SESSION_SECRET: &[u8] = b"approval-gate-e2e-session-secret-32b";

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
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
    // The `request_state` HMAC is keyed on the server secret.
    state.subscribe_resume_secret = Some(Arc::from(&b"approval-gate-e2e-secret-key-0001!"[..]));
    // Browser sessions, as in production: the token exchange and the cookie.
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn list_and_answer_an_approval_gate() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap();
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "oncall".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let write_token = mint(
        store.as_ref(),
        ws.id,
        human.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;
    // Accepting needs approval:grant on a token (or a signed-in session); the
    // plain token above lists, declines and cancels.
    let grant_token = mint(
        store.as_ref(),
        ws.id,
        human.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
            capability::APPROVAL_GRANT.into(),
        ],
    )
    .await;

    // An agent opened a gate (seeded via the store — the MCP request_approval
    // path is covered by the maidan-mcp inline test). Not the human answering
    // it: no one accepts their own request.
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "deployer".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: agent.id,
            prompt: "Deploy v9 to prod?".into(),
            schema: None,
        })
        .await
        .unwrap();

    // The pending list carries the gate and its request_state.
    let list: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/approval-gates", ws.id))
        .bearer_auth(&write_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["gate"]["id"], json!(gate.id.0.to_string()));
    assert_eq!(list[0]["gate"]["state"], json!("pending"));
    let request_state = list[0]["request_state"].as_str().unwrap().to_string();

    // A tampered request_state is refused (integrity of the untrusted round-trip).
    let bad = client
        .post(format!("{base}/approval-gates/{}/answer", gate.id))
        .bearer_auth(&write_token)
        .json(&json!({ "request_state": "deadbeef", "action": "accept" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::FORBIDDEN);

    // An unknown action is a bad request — never a silent accept.
    let bad_action = client
        .post(format!("{base}/approval-gates/{}/answer", gate.id))
        .bearer_auth(&write_token)
        .json(&json!({ "request_state": request_state, "action": "maybe" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_action.status(), StatusCode::BAD_REQUEST);

    // The human accepts, with structured content.
    let answered: Value = {
        let resp = client
            .post(format!("{base}/approval-gates/{}/answer", gate.id))
            .bearer_auth(&grant_token)
            .json(&json!({
                "request_state": request_state,
                "action": "accept",
                "content": { "note": "ship it" }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        resp.json().await.unwrap()
    };
    assert_eq!(answered["state"], json!("accepted"));
    assert_eq!(answered["content"]["note"], json!("ship it"));
    assert_eq!(answered["resolved_by"], json!(human.id.0.to_string()));

    // Silence is not consent, and a second answer cannot flip it: the CAS on
    // `pending` finds nothing → 409.
    let again = client
        .post(format!("{base}/approval-gates/{}/answer", gate.id))
        .bearer_auth(&write_token)
        .json(&json!({ "request_state": request_state, "action": "decline" }))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::CONFLICT);

    // A resolved gate leaves the pending list.
    let list_after: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/approval-gates", ws.id))
        .bearer_auth(&write_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_after.is_empty());
}

/// An approval gate is human-control state. A worker agent holds
/// `workspace:write`, which is all the answer route asks for, so without this
/// rule agent B could accept agent A's gate and no human would ever see it.
/// Declining stays open to any writer; accepting needs a signed-in browser
/// session or a token an admin granted `approval:grant`.
#[tokio::test]
async fn an_agent_cannot_accept_another_agents_gate_without_approval_grant() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "pair".into(),
        })
        .await
        .unwrap();
    let agent = |handle: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let requester = agent("deployer").await;
    let peer = agent("peer").await;
    let approver = agent("approver-bot").await;
    let work = vec![
        capability::WORKSPACE_READ.to_string(),
        capability::WORKSPACE_WRITE.to_string(),
    ];
    let peer_token = mint(store.as_ref(), ws.id, peer.id, work.clone()).await;
    let mut granted = work.clone();
    granted.push(capability::APPROVAL_GRANT.to_string());
    let approver_token = mint(store.as_ref(), ws.id, approver.id, granted).await;

    let open = |prompt: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_approval_gate(&NewApprovalGate {
                    workspace_id: ws.id,
                    thread_id: None,
                    requested_by: requester.id,
                    prompt: prompt.into(),
                    schema: None,
                })
                .await
                .unwrap()
        }
    };
    let first = open("Deploy v9 to prod?").await;
    let second = open("Rotate the signing key?").await;

    let request_state = |token: String, gate_id: String| {
        let client = client.clone();
        let base = base.clone();
        async move {
            let list: Vec<Value> = client
                .get(format!("{base}/workspaces/{}/approval-gates", ws.id))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            list.iter()
                .find(|g| g["gate"]["id"] == json!(gate_id))
                .and_then(|g| g["request_state"].as_str())
                .unwrap()
                .to_string()
        }
    };
    let answer = |token: String, gate: String, state: String, action: &'static str| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .post(format!("{base}/approval-gates/{gate}/answer"))
                .bearer_auth(&token)
                .json(&json!({ "request_state": state, "action": action }))
                .send()
                .await
                .unwrap()
        }
    };

    // A worker agent cannot accept another agent's gate.
    let state = request_state(peer_token.clone(), first.id.0.to_string()).await;
    let refused = answer(
        peer_token.clone(),
        first.id.0.to_string(),
        state.clone(),
        "accept",
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let body: Value = refused.json().await.unwrap();
    assert!(
        body.to_string()
            .contains("missing capability: approval:grant"),
        "the refusal names what accepting needs: {body}"
    );
    let still = store.get_approval_gate(first.id).await.unwrap().unwrap();
    assert_eq!(
        still.state,
        ApprovalGateState::Pending,
        "the refused accept changed nothing"
    );

    // Declining is not approving, and stays open to any writer.
    let state2 = request_state(peer_token.clone(), second.id.0.to_string()).await;
    let declined = answer(peer_token, second.id.0.to_string(), state2, "decline").await;
    assert_eq!(declined.status(), StatusCode::OK);

    // An agent an admin granted approval:grant may accept.
    let accepted = answer(approver_token, first.id.0.to_string(), state, "accept").await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let body: Value = accepted.json().await.unwrap();
    assert_eq!(body["state"], json!("accepted"));
    assert_eq!(body["resolved_by"], json!(approver.id.0.to_string()));
}

const WORK: &[&str] = &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE];

fn caps(extra: &[&str]) -> Vec<String> {
    WORK.iter().chain(extra).map(|c| c.to_string()).collect()
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

async fn open_gate(store: &dyn Store, ws: WorkspaceId, requested_by: MemberId) -> ApprovalGate {
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws,
            thread_id: None,
            requested_by,
            prompt: "Deploy v9 to prod?".into(),
            schema: None,
        })
        .await
        .unwrap()
}

/// The cookie of a browser session a person signed in to: the row the OIDC
/// callback writes (no token behind it), signed as the callback signs it.
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
    cookie_pair(headers.get(axum::http::header::SET_COOKIE).unwrap())
}

fn cookie_pair(set_cookie: &axum::http::HeaderValue) -> String {
    set_cookie
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// A session made from `token` through the exchange the console uses.
async fn token_session(client: &reqwest::Client, base: &str, token: &str) -> String {
    let resp = client
        .post(format!("{base}/auth/session/from-token"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let set_cookie = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    set_cookie.split(';').next().unwrap().to_string()
}

/// What a session request says about where it came from.
#[derive(Clone, Copy)]
enum Sent {
    /// What a browser sends for a fetch from the console page.
    FromConsole,
    /// A same-origin `Origin` and no `Sec-Fetch-Site` (an older browser).
    WithOriginOnly,
    /// Neither header: curl holding the cookie.
    Bare,
    /// `Sec-Fetch-Site: none`, which a request typed into the address bar sends.
    Typed,
}

enum Cred<'a> {
    Bearer(&'a str),
    Cookie(&'a str, Sent),
}

struct Api {
    base: String,
    client: reqwest::Client,
}

impl Api {
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        cred: &Cred<'_>,
    ) -> reqwest::RequestBuilder {
        let b = self.client.request(method, format!("{}{path}", self.base));
        match cred {
            Cred::Bearer(token) => b.bearer_auth(token),
            Cred::Cookie(cookie, sent) => {
                let b = b.header(reqwest::header::COOKIE, *cookie);
                match sent {
                    Sent::FromConsole => b
                        .header("sec-fetch-site", "same-origin")
                        .header(reqwest::header::ORIGIN, self.base.clone()),
                    // reqwest sends the URL's authority as Host, which is
                    // what the Origin names.
                    Sent::WithOriginOnly => b.header(reqwest::header::ORIGIN, self.base.clone()),
                    Sent::Bare => b,
                    Sent::Typed => b.header("sec-fetch-site", "none"),
                }
            }
        }
    }

    /// The list route the credential is used on: the bearer tree for a
    /// token, the session proxy for a cookie.
    fn prefix(cred: &Cred<'_>) -> &'static str {
        match cred {
            Cred::Bearer(_) => "",
            Cred::Cookie(..) => "/ui/api",
        }
    }

    async fn list(&self, ws: WorkspaceId, cred: &Cred<'_>) -> reqwest::Response {
        let path = format!("{}/workspaces/{}/approval-gates", Self::prefix(cred), ws);
        self.request(reqwest::Method::GET, &path, cred)
            .send()
            .await
            .unwrap()
    }

    async fn request_state(&self, ws: WorkspaceId, gate: &ApprovalGate, cred: &Cred<'_>) -> String {
        let resp = self.list(ws, cred).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let list: Vec<Value> = resp.json().await.unwrap();
        list.iter()
            .find(|g| g["gate"]["id"] == json!(gate.id.0.to_string()))
            .and_then(|g| g["request_state"].as_str())
            .unwrap()
            .to_string()
    }

    async fn answer_at(
        &self,
        prefix: &str,
        gate: &ApprovalGate,
        state: &str,
        action: &str,
        cred: &Cred<'_>,
    ) -> reqwest::Response {
        let path = format!("{prefix}/approval-gates/{}/answer", gate.id);
        self.request(reqwest::Method::POST, &path, cred)
            .json(&json!({ "request_state": state, "action": action }))
            .send()
            .await
            .unwrap()
    }

    async fn answer(
        &self,
        gate: &ApprovalGate,
        state: &str,
        action: &str,
        cred: &Cred<'_>,
    ) -> reqwest::Response {
        self.answer_at(Self::prefix(cred), gate, state, action, cred)
            .await
    }
}

async fn assert_refused_with(resp: reqwest::Response, needle: &str) {
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["detail"].as_str().unwrap_or_default().contains(needle),
        "the refusal says {needle:?}: {body}"
    );
}

async fn assert_pending(store: &dyn Store, gate: &ApprovalGate) {
    let now = store.get_approval_gate(gate.id).await.unwrap().unwrap();
    assert_eq!(
        now.state,
        ApprovalGateState::Pending,
        "the gate is untouched"
    );
}

/// The token a person hands an agent is a plain human bearer token. The
/// member it names is human, which used to be enough to accept, so an agent
/// holding it could accept any gate with curl. It still declines and cancels.
#[tokio::test]
async fn a_plain_human_bearer_token_cannot_accept_a_gate() {
    let (addr, client, store) = spawn().await;
    let api = Api {
        base: format!("http://{addr}"),
        client,
    };
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap();
    let human = member(store.as_ref(), ws.id, "oncall", MemberKind::Human).await;
    let agent = member(store.as_ref(), ws.id, "deployer", MemberKind::Agent).await;
    let token = mint(store.as_ref(), ws.id, human, caps(&[])).await;
    let bearer = Cred::Bearer(&token);

    let gate = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &gate, &bearer).await;
    assert_refused_with(
        api.answer(&gate, &state, "accept", &bearer).await,
        "missing capability: approval:grant",
    )
    .await;
    // The same refusal on the session proxy, where the console's bearer goes.
    assert_refused_with(
        api.answer_at("/ui/api", &gate, &state, "accept", &bearer)
            .await,
        "missing capability: approval:grant",
    )
    .await;
    assert_pending(store.as_ref(), &gate).await;

    // Declining and cancelling are not accepting, and are unchanged.
    let declined = api.answer(&gate, &state, "decline", &bearer).await;
    assert_eq!(declined.status(), StatusCode::OK);
    let declined: Value = declined.json().await.unwrap();
    assert_eq!(declined["state"], json!("declined"));
    let other = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &other, &bearer).await;
    let cancelled = api.answer(&other, &state, "cancel", &bearer).await;
    assert_eq!(cancelled.status(), StatusCode::OK);
}

/// Whoever holds a person's token can trade it for a session cookie, so a
/// session made from a token proves no more than the token: it accepts only
/// when that token holds `approval:grant`.
#[tokio::test]
async fn a_session_made_from_a_token_accepts_only_with_the_tokens_approval_grant() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let api = Api {
        base: base.clone(),
        client: client.clone(),
    };
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap();
    let human = member(store.as_ref(), ws.id, "oncall", MemberKind::Human).await;
    let agent = member(store.as_ref(), ws.id, "deployer", MemberKind::Agent).await;
    let plain = mint(store.as_ref(), ws.id, human, caps(&[])).await;
    let granted = mint(
        store.as_ref(),
        ws.id,
        human,
        caps(&[capability::APPROVAL_GRANT]),
    )
    .await;

    let plain_cookie = token_session(&client, &base, &plain).await;
    let from_plain = Cred::Cookie(&plain_cookie, Sent::FromConsole);
    let gate = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &gate, &from_plain).await;
    assert_refused_with(
        api.answer(&gate, &state, "accept", &from_plain).await,
        "missing capability: approval:grant",
    )
    .await;
    // The bearer tree takes a token's session too, and refuses it the same way.
    assert_refused_with(
        api.answer_at("", &gate, &state, "accept", &from_plain)
            .await,
        "missing capability: approval:grant",
    )
    .await;
    assert_pending(store.as_ref(), &gate).await;

    let granted_cookie = token_session(&client, &base, &granted).await;
    let from_granted = Cred::Cookie(&granted_cookie, Sent::FromConsole);
    let accepted = api.answer(&gate, &state, "accept", &from_granted).await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted: Value = accepted.json().await.unwrap();
    assert_eq!(accepted["state"], json!("accepted"));
    assert_eq!(accepted["resolved_by"], json!(human.0.to_string()));
}

/// The console's own path: a person signed in through the identity provider,
/// answering from the console page. The origin check is the strict one, so the
/// cookie without a page behind it (curl, or a typed URL) does not accept, but
/// still declines.
#[tokio::test]
async fn a_signed_in_session_accepts_from_the_console_page_and_not_without_it() {
    let (addr, client, store) = spawn().await;
    let api = Api {
        base: format!("http://{addr}"),
        client,
    };
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap();
    let human = member(store.as_ref(), ws.id, "oncall", MemberKind::Human).await;
    let agent = member(store.as_ref(), ws.id, "deployer", MemberKind::Agent).await;
    let cookie = signed_in(store.as_ref(), ws.id, human).await;
    let page = Cred::Cookie(&cookie, Sent::FromConsole);

    let gate = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &gate, &page).await;
    for sent in [Sent::Bare, Sent::Typed] {
        assert_refused_with(
            api.answer(&gate, &state, "accept", &Cred::Cookie(&cookie, sent))
                .await,
            "from the console page",
        )
        .await;
    }
    assert_pending(store.as_ref(), &gate).await;

    let accepted = api.answer(&gate, &state, "accept", &page).await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted: Value = accepted.json().await.unwrap();
    assert_eq!(accepted["state"], json!("accepted"));
    assert_eq!(accepted["resolved_by"], json!(human.0.to_string()));

    // An older browser that sends only a matching Origin is the page too.
    let second = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &second, &page).await;
    let accepted = api
        .answer(
            &second,
            &state,
            "accept",
            &Cred::Cookie(&cookie, Sent::WithOriginOnly),
        )
        .await;
    assert_eq!(accepted.status(), StatusCode::OK);

    // Declining keeps the session's ordinary origin check.
    let third = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &third, &page).await;
    let declined = api
        .answer(
            &third,
            &state,
            "decline",
            &Cred::Cookie(&cookie, Sent::Bare),
        )
        .await;
    assert_eq!(declined.status(), StatusCode::OK);
}

/// Nobody accepts their own request, whatever the credential: not with a
/// signed-in session, not with `approval:grant`.
#[tokio::test]
async fn no_credential_lets_the_requester_accept_its_own_gate() {
    let (addr, client, store) = spawn().await;
    let api = Api {
        base: format!("http://{addr}"),
        client,
    };
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap();
    let asker = member(store.as_ref(), ws.id, "asker", MemberKind::Human).await;
    let granted = mint(
        store.as_ref(),
        ws.id,
        asker,
        caps(&[capability::APPROVAL_GRANT]),
    )
    .await;
    let cookie = signed_in(store.as_ref(), ws.id, asker).await;

    let gate = open_gate(store.as_ref(), ws.id, asker).await;
    for cred in [
        Cred::Bearer(&granted),
        Cred::Cookie(&cookie, Sent::FromConsole),
    ] {
        let state = api.request_state(ws.id, &gate, &cred).await;
        assert_refused_with(
            api.answer(&gate, &state, "accept", &cred).await,
            "whoever requested it",
        )
        .await;
    }
    assert_pending(store.as_ref(), &gate).await;
}

/// A signed-in session whose member is not a human does not accept: the
/// session proves a sign-in, and the gate is still human-control state.
#[tokio::test]
async fn a_signed_in_session_for_an_agent_member_cannot_accept() {
    let (addr, client, store) = spawn().await;
    let api = Api {
        base: format!("http://{addr}"),
        client,
    };
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap();
    let bot = member(store.as_ref(), ws.id, "bot", MemberKind::Agent).await;
    let agent = member(store.as_ref(), ws.id, "deployer", MemberKind::Agent).await;
    let cookie = signed_in(store.as_ref(), ws.id, bot).await;
    let page = Cred::Cookie(&cookie, Sent::FromConsole);
    let gate = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &gate, &page).await;
    assert_refused_with(
        api.answer(&gate, &state, "accept", &page).await,
        "human member",
    )
    .await;
    assert_pending(store.as_ref(), &gate).await;
}

/// Workspace B's credentials that do accept in B, a signed-in session and an
/// `approval:grant` token, can neither see workspace A's gates nor accept one,
/// even holding A's valid `request_state`.
#[tokio::test]
async fn another_workspaces_session_or_approval_grant_cannot_see_or_accept_a_gate() {
    let (addr, client, store) = spawn().await;
    let api = Api {
        base: format!("http://{addr}"),
        client,
    };
    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let owner_a = member(store.as_ref(), ws_a.id, "owner-a", MemberKind::Human).await;
    let agent_a = member(store.as_ref(), ws_a.id, "deployer-a", MemberKind::Agent).await;
    let human_b = member(store.as_ref(), ws_b.id, "owner-b", MemberKind::Human).await;
    let agent_b = member(store.as_ref(), ws_b.id, "deployer-b", MemberKind::Agent).await;
    let token_a = mint(store.as_ref(), ws_a.id, owner_a, caps(&[])).await;
    let grant_b = mint(
        store.as_ref(),
        ws_b.id,
        human_b,
        caps(&[capability::APPROVAL_GRANT]),
    )
    .await;
    let cookie_b = signed_in(store.as_ref(), ws_b.id, human_b).await;

    let gate_a = open_gate(store.as_ref(), ws_a.id, agent_a).await;
    let state_a = api
        .request_state(ws_a.id, &gate_a, &Cred::Bearer(&token_a))
        .await;

    for cred in [
        Cred::Bearer(&grant_b),
        Cred::Cookie(&cookie_b, Sent::FromConsole),
    ] {
        let listed = api.list(ws_a.id, &cred).await;
        assert!(
            !listed.status().is_success(),
            "B cannot list A's gates: {}",
            listed.status()
        );
        let body = listed.text().await.unwrap();
        assert!(!body.contains(&gate_a.id.0.to_string()), "{body}");

        let answered = api.answer(&gate_a, &state_a, "accept", &cred).await;
        assert_eq!(
            answered.status(),
            StatusCode::NOT_FOUND,
            "A's gate does not exist for B"
        );
        assert_pending(store.as_ref(), &gate_a).await;
    }

    // The same credentials do accept in their own workspace, so the refusal
    // above is the tenant boundary, not a broken credential.
    let gate_b = open_gate(store.as_ref(), ws_b.id, agent_b).await;
    let page_b = Cred::Cookie(&cookie_b, Sent::FromConsole);
    let state_b = api.request_state(ws_b.id, &gate_b, &page_b).await;
    let accepted = api.answer(&gate_b, &state_b, "accept", &page_b).await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let gate_b2 = open_gate(store.as_ref(), ws_b.id, agent_b).await;
    let state_b2 = api
        .request_state(ws_b.id, &gate_b2, &Cred::Bearer(&grant_b))
        .await;
    let accepted = api
        .answer(&gate_b2, &state_b2, "accept", &Cred::Bearer(&grant_b))
        .await;
    assert_eq!(accepted.status(), StatusCode::OK);
}

/// An OAuth-issued token is a token, so it can never accept an approval gate.
/// The token endpoint strips `approval:grant` at mint, and the authorize
/// step refuses it as a scope: even a grant for a member who holds the
/// capability cannot carry it into an OAuth token.
#[tokio::test]
async fn an_oauth_token_cannot_accept_a_gate() {
    let (addr, client, store) = spawn().await;
    let api = Api {
        base: format!("http://{addr}"),
        client,
    };
    let ws = store
        .create_workspace(NewWorkspace {
            name: "oauth-gate".into(),
        })
        .await
        .unwrap();
    let human = member(store.as_ref(), ws.id, "owner", MemberKind::Human).await;
    let agent = member(store.as_ref(), ws.id, "worker", MemberKind::Agent).await;

    // The human holds approval:grant (like an admin), and the OAuth client
    // is allowed workspace:write. The grant must still not carry approval:grant.
    let human_token = mint(
        store.as_ref(),
        ws.id,
        human,
        caps(&["workspace:read", "workspace:write", "approval:grant"]),
    )
    .await;
    let human_bearer = Cred::Bearer(&human_token);

    store
        .create_oauth_client(maidan_types::NewOAuthClient {
            client_id: "test-mcp-client".into(),
            name: "Test MCP client".into(),
            redirect_uris: vec!["https://client.example/callback".into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into(), "workspace:write".into()],
        })
        .await
        .unwrap();

    // PKCE: S256 challenge from a random verifier.
    let verifier: String = {
        use base64::Engine;
        use rand::RngCore;
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
    };
    let challenge = {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    };

    // Authorize: the human's bearer goes in, a pending request comes out via
    // redirect to the consent page. A no-redirect client: we read the
    // Location header ourselves.
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let auth_url = format!(
        "{}/oauth/authorize?client_id=test-mcp-client&redirect_uri={}&code_challenge={}&code_challenge_method=S256&scope=workspace%3Aread%20workspace%3Awrite&state=xyz",
        api.base,
        urlencoding::encode("https://client.example/callback"),
        challenge,
    );
    let resp = no_redirect
        .get(&auth_url)
        .header("Authorization", format!("Bearer {human_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        location.starts_with("/ui/oauth/consent?request="),
        "authorize redirects to consent, got {location}"
    );
    let request_id = location.split("request=").nth(1).unwrap().to_string();

    // Consent: the human approves on their signed-in session (a cookie, not
    // a bearer), via same-origin POST as the consent page sends it. The
    // handler 303-redirects to the client's redirect URI; we read the
    // Location ourselves instead of following it.
    let cookie = signed_in(store.as_ref(), ws.id, human).await;
    let consent_resp = no_redirect
        .post(format!("{}/ui/api/oauth/consent", api.base))
        .header("Cookie", &cookie)
        .header("Sec-Fetch-Site", "same-origin")
        .form(&[("request_id", request_id.as_str()), ("approved", "true")])
        .send()
        .await
        .unwrap();
    assert_eq!(consent_resp.status(), StatusCode::SEE_OTHER);
    let redirect_to = consent_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let code = redirect_to
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    assert!(!code.is_empty());

    // Exchange the code for a token.
    let token_resp = api
        .client
        .post(format!("{}/oauth/token", api.base))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", "https://client.example/callback"),
            ("client_id", "test-mcp-client"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token_resp.status(), StatusCode::OK);
    let token_body: Value = token_resp.json().await.unwrap();
    let oauth_token = token_body["access_token"].as_str().unwrap().to_string();
    let oauth_bearer = Cred::Bearer(&oauth_token);

    // The OAuth token cannot accept a gate, even though the human behind it
    // holds approval:grant. The capability was stripped at mint.
    let gate = open_gate(store.as_ref(), ws.id, agent).await;
    let state = api.request_state(ws.id, &gate, &human_bearer).await;
    assert_refused_with(
        api.answer(&gate, &state, "accept", &oauth_bearer).await,
        "missing capability: approval:grant",
    )
    .await;
    assert_pending(store.as_ref(), &gate).await;

    // Declining and cancelling are not accepting, and are unchanged.
    let declined = api.answer(&gate, &state, "decline", &oauth_bearer).await;
    assert_eq!(declined.status(), StatusCode::OK);
}
