//! The inline approval card (Open Work Next 17, David's 1a/2a, 2026-10-09):
//! an MCP Apps View (SEP-1865, stable `2026-01-26`) served as a `ui://`
//! resource, linked from `get_approval_gate` and `approval_decide`, and fed
//! from their `structuredContent`.
//!
//! - The card is listed, readable at exactly its URI, and linked from both
//!   tools with `_meta.ui.resourceUri`.
//! - A client that declares its capabilities without MCP Apps sees exactly
//!   what it saw before: no link, no listing, no `structuredContent`, no
//!   extension in the handshake.
//! - `get_approval_gate`'s `structuredContent` carries the requester and the
//!   thread's review evidence with tiers and the self-reported-only warning.
//! - Another workspace reads nothing of a gate through the card or the tool.
//! - A model's accept from the card, under a plain bearer, is the same
//!   confirmation-required answer: the gate stays pending.
//! - "Decided via" prefers the credential, then self-reported `clientInfo`,
//!   then nothing, on the gate, the confirmation and the audit row.
//!
//! Runs with auth enabled, sessions configured, and a console origin.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, ApprovalRisk, ClientIdentitySource, MemberId,
    MemberKind, NewApiToken, NewApp, NewAppInstallation, NewApprovalGate, NewChannel, NewMember,
    NewThread, NewWorkspace, ThreadId, WorkspaceId,
};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

const CARD_URI: &str = "ui://maidan/approval-card.html";
const CARD_MIME: &str = "text/html;profile=mcp-app";
const UI_EXTENSION: &str = "io.modelcontextprotocol/ui";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: SqlitePool,
    _dir: tempfile::TempDir,
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
    state.subscribe_resume_secret = Some(Arc::from(&b"apps-card-e2e-secret-key-000000001!"[..]));
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(&b"apps-card-e2e-session-secret-0000032"[..]),
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
        _dir: dir,
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
            display_name: Some(format!("{handle} (display)")),
            kind,
        })
        .await
        .unwrap()
        .id
}

async fn mint_for(
    store: &dyn Store,
    ws: WorkspaceId,
    member: MemberId,
    installation: Option<maidan_types::AppInstallationId>,
    caps: &[&str],
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: installation,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
    mint_for(store, ws, member, None, caps).await
}

const WRITE: &[&str] = &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE];
const WORKER: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::MESSAGE_POST,
    capability::THREAD_TRANSITION,
];

async fn open_gate(
    store: &dyn Store,
    ws: WorkspaceId,
    requested_by: MemberId,
    thread_id: Option<ThreadId>,
    prompt: &str,
) -> ApprovalGate {
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws,
            thread_id,
            requested_by,
            prompt: prompt.into(),
            schema: None,
            risk: ApprovalRisk::High,
        })
        .await
        .unwrap()
}

/// How a request describes its client.
#[derive(Clone)]
enum Caps {
    /// A pre-2026 stateless request: no `_meta`, nothing declared.
    None,
    /// A `2026-07-28` request whose capabilities declare MCP Apps.
    WithUi,
    /// A `2026-07-28` request whose capabilities say nothing of MCP Apps.
    WithoutUi,
}

fn ui_caps() -> Value {
    json!({ "extensions": { UI_EXTENSION: { "mimeTypes": [CARD_MIME] } } })
}

impl Env {
    async fn rpc(
        &self,
        bearer: &str,
        method: &str,
        mut params: Value,
        caps: Caps,
        client_name: Option<&str>,
    ) -> Value {
        let mut req = self
            .client
            .post(format!("{}/mcp", self.base))
            .bearer_auth(bearer);
        let capabilities = match caps {
            Caps::None => None,
            Caps::WithUi => Some(ui_caps()),
            Caps::WithoutUi => Some(json!({ "elicitation": { "url": {} } })),
        };
        if let Some(capabilities) = capabilities {
            let mut meta = json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": capabilities,
            });
            if let Some(name) = client_name {
                meta["io.modelcontextprotocol/clientInfo"] =
                    json!({ "name": name, "version": "1.0" });
            }
            params["_meta"] = meta;
            req = req
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", method);
            if let Some(name) = params
                .get("name")
                .or_else(|| params.get("uri"))
                .and_then(Value::as_str)
            {
                req = req.header("mcp-name", name);
            }
        }
        req.json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn call_tool(
        &self,
        bearer: &str,
        name: &str,
        args: Value,
        caps: Caps,
        client_name: Option<&str>,
    ) -> Value {
        let res = self
            .rpc(
                bearer,
                "tools/call",
                json!({ "name": name, "arguments": args }),
                caps,
                client_name,
            )
            .await;
        assert!(res.get("error").is_none(), "{name}: {res}");
        res["result"].clone()
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

    async fn gate(&self, id: ApprovalGateId) -> ApprovalGate {
        self.store.get_approval_gate(id).await.unwrap().unwrap()
    }
}

fn text_payload(result: &Value) -> Value {
    assert_eq!(result["isError"], false, "tool error: {result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn tool<'a>(tools: &'a Value, name: &str) -> &'a Value {
    tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("{name} not listed"))
}

struct Team {
    ws: WorkspaceId,
    requester: MemberId,
    operator: MemberId,
    requester_tok: String,
    operator_tok: String,
}

async fn team(env: &Env, name: &str) -> Team {
    let ws = workspace(env.store.as_ref(), name).await;
    let requester = member(env.store.as_ref(), ws, "deployer", MemberKind::Agent).await;
    let operator = member(env.store.as_ref(), ws, "operator", MemberKind::Human).await;
    Team {
        ws,
        requester,
        operator,
        requester_tok: mint(env.store.as_ref(), ws, requester, WORKER).await,
        operator_tok: mint(env.store.as_ref(), ws, operator, WRITE).await,
    }
}

#[tokio::test]
async fn the_card_is_listed_read_and_linked_from_both_approval_tools() {
    let env = spawn().await;
    let t = team(&env, "alpha").await;
    for caps in [Caps::None, Caps::WithUi] {
        let tools = env
            .rpc(&t.operator_tok, "tools/list", json!({}), caps.clone(), None)
            .await;
        for name in ["get_approval_gate", "approval_decide"] {
            let ui = &tool(&tools, name)["_meta"]["ui"];
            assert_eq!(ui["resourceUri"], CARD_URI, "{name}: {ui}");
            assert_eq!(ui["visibility"], json!(["model", "app"]), "{name}");
        }
        // Only the gate tools link it: the card renders a gate, nothing else.
        let linked: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["_meta"]["ui"].is_object())
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(linked, vec!["get_approval_gate", "approval_decide"]);

        let listed = env
            .rpc(
                &t.operator_tok,
                "resources/list",
                json!({}),
                caps.clone(),
                None,
            )
            .await;
        let card = listed["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["uri"] == CARD_URI)
            .expect("the card is listed");
        assert_eq!(card["mimeType"], CARD_MIME);

        let read = env
            .rpc(
                &t.operator_tok,
                "resources/read",
                json!({ "uri": CARD_URI }),
                caps,
                None,
            )
            .await;
        let content = &read["result"]["contents"][0];
        assert_eq!(content["uri"], CARD_URI, "{read}");
        assert_eq!(content["mimeType"], CARD_MIME);
        let html = content["text"].as_str().unwrap();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Content-Security-Policy"));
        assert_eq!(content["_meta"]["ui"]["csp"]["connectDomains"], json!([]));
        assert_eq!(content["_meta"]["ui"]["csp"]["resourceDomains"], json!([]));
    }

    // A client that declares MCP Apps hears the extension back; one that
    // declares nothing hears the handshake it always did.
    let declared = env
        .rpc(
            &t.operator_tok,
            "initialize",
            json!({ "protocolVersion": "2025-11-25", "capabilities": ui_caps(),
                    "clientInfo": { "name": "host", "version": "1" } }),
            Caps::None,
            None,
        )
        .await;
    assert_eq!(
        declared["result"]["capabilities"]["extensions"][UI_EXTENSION]["mimeTypes"],
        json!([CARD_MIME]),
        "{declared}"
    );
    let plain = env
        .rpc(
            &t.operator_tok,
            "initialize",
            json!({ "protocolVersion": "2025-11-25", "capabilities": {},
                    "clientInfo": { "name": "host", "version": "1" } }),
            Caps::None,
            None,
        )
        .await;
    assert!(
        plain["result"]["capabilities"].get("extensions").is_none(),
        "{plain}"
    );
}

#[tokio::test]
async fn a_client_without_mcp_apps_sees_no_change() {
    let env = spawn().await;
    let t = team(&env, "alpha").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, None, "Ship it?").await;

    let tools = env
        .rpc(
            &t.operator_tok,
            "tools/list",
            json!({}),
            Caps::WithoutUi,
            None,
        )
        .await;
    for name in ["get_approval_gate", "approval_decide"] {
        assert!(
            tool(&tools, name).get("_meta").is_none(),
            "{name} keeps no link"
        );
    }
    let listed = env
        .rpc(
            &t.operator_tok,
            "resources/list",
            json!({}),
            Caps::WithoutUi,
            None,
        )
        .await;
    assert!(
        !listed["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["uri"] == CARD_URI),
        "{listed}"
    );
    let got = env
        .call_tool(
            &t.operator_tok,
            "get_approval_gate",
            json!({ "gate_id": gate.id }),
            Caps::WithoutUi,
            None,
        )
        .await;
    assert!(got.get("structuredContent").is_none(), "{got}");
    assert_eq!(text_payload(&got)["id"], json!(gate.id));
    let decided = env
        .call_tool(
            &t.operator_tok,
            "approval_decide",
            json!({ "gate_id": gate.id, "decision": "decline" }),
            Caps::WithoutUi,
            None,
        )
        .await;
    assert!(decided.get("structuredContent").is_none(), "{decided}");
    assert_eq!(text_payload(&decided)["status"], "declined");

    let discovered = env
        .rpc(
            &t.operator_tok,
            "server/discover",
            json!({}),
            Caps::WithoutUi,
            None,
        )
        .await;
    assert!(
        discovered["result"]["capabilities"]
            .get("extensions")
            .is_none(),
        "{discovered}"
    );
}

#[tokio::test]
async fn the_gate_result_carries_requester_and_review_evidence_tiers() {
    let env = spawn().await;
    let t = team(&env, "alpha").await;
    let channel = env
        .store
        .create_channel(NewChannel {
            workspace_id: t.ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = env
        .store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("deploy".into()),
            description: None,
        })
        .await
        .unwrap();
    // The requester works the thread and hands it to review with a result
    // only it vouches for: every attestation is self-reported.
    for (method, path, body) in [
        (
            "POST",
            format!("/threads/{}/assignee/claim", thread.id.0),
            json!({}),
        ),
        (
            "PUT",
            format!("/threads/{}/result", thread.id.0),
            json!({ "result": { "status": "done" } }),
        ),
        (
            "POST",
            format!("/threads/{}", thread.id.0),
            json!({ "action": "start_review" }),
        ),
    ] {
        let resp = env
            .client
            .request(method.parse().unwrap(), format!("{}{path}", env.base))
            .bearer_auth(&t.requester_tok)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "{path}: {}",
            resp.text().await.unwrap()
        );
    }
    let packet = env
        .store
        .latest_review_packet(thread.id)
        .await
        .unwrap()
        .expect("handed to review");
    assert!(packet.self_reported_only);

    let gate = open_gate(
        env.store.as_ref(),
        t.ws,
        t.requester,
        Some(thread.id),
        "Deploy?",
    )
    .await;
    let got = env
        .call_tool(
            &t.operator_tok,
            "get_approval_gate",
            json!({ "gate_id": gate.id }),
            Caps::WithUi,
            None,
        )
        .await;
    // The text is what it always was; the card's view rides beside it.
    assert_eq!(text_payload(&got)["id"], json!(gate.id));
    let view = &got["structuredContent"];
    assert_eq!(view["kind"], "maidan.approval_gate");
    assert_eq!(view["gate"]["prompt"], "Deploy?");
    assert_eq!(view["gate"]["risk"], "high");
    assert_eq!(view["requester"]["handle"], "deployer");
    assert_eq!(view["requester"]["display_name"], "deployer (display)");
    assert_eq!(view["requester"]["kind"], "agent");
    assert_eq!(view["review"]["self_reported_only"], true);
    assert_eq!(view["review"]["evidence_root"], packet.evidence_root);
    let tiers: Vec<&str> = view["review"]["attestations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["tier"].as_str().unwrap())
        .collect();
    assert!(!tiers.is_empty(), "{view}");
    assert!(tiers.iter().all(|t| *t == "self_reported"), "{tiers:?}");

    // An unattached gate has no review to show.
    let bare = open_gate(env.store.as_ref(), t.ws, t.requester, None, "Bare?").await;
    let got = env
        .call_tool(
            &t.operator_tok,
            "get_approval_gate",
            json!({ "gate_id": bare.id }),
            Caps::None,
            None,
        )
        .await;
    assert_eq!(got["structuredContent"]["review"], Value::Null, "{got}");
}

#[tokio::test]
async fn another_workspace_reads_nothing_of_a_gate_through_the_card() {
    let env = spawn().await;
    let a = team(&env, "alpha").await;
    let b = team(&env, "bravo").await;
    let secret_prompt = "Alpha's secret deploy";
    let gate = open_gate(env.store.as_ref(), a.ws, a.requester, None, secret_prompt).await;

    let got = env
        .call_tool(
            &b.operator_tok,
            "get_approval_gate",
            json!({ "gate_id": gate.id }),
            Caps::WithUi,
            None,
        )
        .await;
    assert_eq!(text_payload(&got), Value::Null);
    assert_eq!(got["structuredContent"]["gate"], Value::Null, "{got}");
    assert!(!got.to_string().contains(secret_prompt));

    // The card is the same static page for everyone, and a URI naming the
    // gate is not a resource.
    let card = env
        .rpc(
            &b.operator_tok,
            "resources/read",
            json!({ "uri": CARD_URI }),
            Caps::WithUi,
            None,
        )
        .await;
    let html = card["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(!html.contains(secret_prompt));
    assert!(!html.contains(&gate.id.0.to_string()));
    for uri in [
        format!("{CARD_URI}?gate_id={}", gate.id.0),
        format!("ui://maidan/approval-gates/{}", gate.id.0),
    ] {
        let res = env
            .rpc(
                &b.operator_tok,
                "resources/read",
                json!({ "uri": uri }),
                Caps::WithUi,
                None,
            )
            .await;
        assert!(res.get("error").is_some(), "{uri}: {res}");
        assert!(!res.to_string().contains(secret_prompt));
    }

    // Nor can B decide it through the card's tool.
    let decided = env
        .rpc(
            &b.operator_tok,
            "tools/call",
            json!({ "name": "approval_decide", "arguments": { "gate_id": gate.id, "decision": "decline" } }),
            Caps::WithUi,
            None,
        )
        .await;
    assert!(
        decided.get("error").is_some() || decided["result"]["isError"] == true,
        "{decided}"
    );
    assert_eq!(env.gate(gate.id).await.state, ApprovalGateState::Pending);
}

#[tokio::test]
async fn an_accept_from_the_card_on_a_plain_bearer_only_asks_for_confirmation() {
    let env = spawn().await;
    let t = team(&env, "alpha").await;
    let gate = open_gate(env.store.as_ref(), t.ws, t.requester, None, "Ship it?").await;
    let answer = env
        .call_tool(
            &t.operator_tok,
            "approval_decide",
            json!({ "gate_id": gate.id, "decision": "accept" }),
            Caps::WithUi,
            Some("ChatGPT"),
        )
        .await;
    let sc = &answer["structuredContent"];
    assert_eq!(sc["kind"], "maidan.approval_decision");
    assert_eq!(sc["status"], "confirmation_required", "{answer}");
    assert!(sc["confirmation_url"]
        .as_str()
        .unwrap()
        .contains(&format!("#confirm-approval={}.", gate.id.0)));
    assert_eq!(text_payload(&answer)["status"], "confirmation_required");
    assert_eq!(env.gate(gate.id).await.state, ApprovalGateState::Pending);
}

/// Credential, then self-reported `clientInfo`, then nothing: on the gate,
/// on the pending confirmation, and in the audit row.
#[tokio::test]
async fn decided_via_prefers_the_credential_then_self_reported_then_none() {
    let env = spawn().await;
    let t = team(&env, "alpha").await;

    // An installed app's token: the name and id come from the registration,
    // whatever clientInfo claims.
    let app = env
        .store
        .create_app(NewApp {
            workspace_id: t.ws,
            slug: "release-bot".into(),
            name: "Release Bot".into(),
            description: None,
            created_by: t.operator,
        })
        .await
        .unwrap();
    let bot = member(env.store.as_ref(), t.ws, "release-bot", MemberKind::Agent).await;
    let installation = env
        .store
        .create_app_installation(NewAppInstallation {
            app_id: app.id,
            workspace_id: t.ws,
            bot_member_id: bot,
            granted_capabilities: WRITE.iter().map(|c| (*c).to_string()).collect(),
        })
        .await
        .unwrap();
    let app_tok = mint_for(env.store.as_ref(), t.ws, bot, Some(installation.id), WRITE).await;

    let by_app = open_gate(env.store.as_ref(), t.ws, t.requester, None, "app").await;
    let res = env
        .call_tool(
            &app_tok,
            "approval_decide",
            json!({ "gate_id": by_app.id, "decision": "decline" }),
            Caps::WithUi,
            Some("Totally ChatGPT"),
        )
        .await;
    assert_eq!(text_payload(&res)["status"], "declined", "{res}");
    let via = env.gate(by_app.id).await.decided_via.unwrap();
    assert_eq!(via.client_source, ClientIdentitySource::Credential);
    assert_eq!(via.client_name.as_deref(), Some("Release Bot"));
    assert_eq!(
        via.client_id.as_deref(),
        Some(app.id.0.to_string().as_str())
    );
    assert_eq!(
        res["structuredContent"]["gate"]["decided_via"]["client_source"],
        "credential"
    );

    // A person's plain token with clientInfo: self-reported.
    let by_info = open_gate(env.store.as_ref(), t.ws, t.requester, None, "info").await;
    env.call_tool(
        &t.operator_tok,
        "approval_decide",
        json!({ "gate_id": by_info.id, "decision": "decline" }),
        Caps::WithUi,
        Some("ChatGPT"),
    )
    .await;
    let via = env.gate(by_info.id).await.decided_via.unwrap();
    assert_eq!(via.client_source, ClientIdentitySource::SelfReported);
    assert_eq!(via.client_name.as_deref(), Some("ChatGPT"));
    assert_eq!(via.client_id, None);

    // Nothing named: none.
    let by_none = open_gate(env.store.as_ref(), t.ws, t.requester, None, "none").await;
    env.call_tool(
        &t.operator_tok,
        "approval_decide",
        json!({ "gate_id": by_none.id, "decision": "decline" }),
        Caps::None,
        None,
    )
    .await;
    let via = env.gate(by_none.id).await.decided_via.unwrap();
    assert_eq!(via.client_source, ClientIdentitySource::None);
    assert_eq!(via.client_name, None);

    let rows = env.audit(t.ws, "approval_gate.decided").await;
    let sources: Vec<(&str, Value, Value)> = rows
        .iter()
        .map(|r| {
            (
                r["client_source"].as_str().unwrap(),
                r["client_name"].clone(),
                r["client_id"].clone(),
            )
        })
        .collect();
    assert_eq!(
        sources,
        vec![
            (
                "credential",
                json!("Release Bot"),
                json!(app.id.0.to_string())
            ),
            ("self_reported", json!("ChatGPT"), Value::Null),
            ("none", Value::Null, Value::Null),
        ]
    );

    // A confirmation carries the source from the request to the decision.
    let asked = open_gate(env.store.as_ref(), t.ws, t.requester, None, "confirm").await;
    env.call_tool(
        &t.operator_tok,
        "approval_decide",
        json!({ "gate_id": asked.id, "decision": "accept" }),
        Caps::WithUi,
        Some("ChatGPT"),
    )
    .await;
    let pending = env
        .store
        .list_live_approval_confirmations(t.ws, chrono::Utc::now())
        .await
        .unwrap();
    let confirmation = pending.iter().find(|c| c.gate_id == asked.id).unwrap();
    assert_eq!(
        confirmation.client_source,
        ClientIdentitySource::SelfReported
    );
    let requested = env
        .audit(t.ws, "approval_gate.confirmation_requested")
        .await;
    assert_eq!(requested.last().unwrap()["client_source"], "self_reported");
}
