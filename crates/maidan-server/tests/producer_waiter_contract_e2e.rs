//! The producer's waiter, on the wire it uses. This mirrors the producer's own
//! contract check (which runs its real waiter image against a throwaway Maidan
//! before a deploy) inside Maidan's own CI, so a Maidan change that breaks that
//! client fails here first.
//!
//! On 2026-10-12 a stricter MCP revision check refused every request the
//! producer's waiter sent, and its deployment crash-looped. Every Maidan test
//! passed, because none of them spoke the way a real client does. The requests
//! below copy the producer's client (its MCP HTTP transport and its waiter,
//! version 1.16.2): the endpoint, the headers, the JSON-RPC envelope, the tool
//! names, the argument names, the order of the calls, the REST result write
//! and the fields the waiter reads from each reply.
//!
//! A change to any of these calls or fields, including a reply field the
//! waiter parses, is a breaking change for the producer. Tell its maintainers
//! before deploying it, so the client can move first; don't just update this
//! test.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::Method;
use serde_json::{json, Value};
use sha2::Digest;
use sqlx::sqlite::SqlitePoolOptions;

/// The waiter connects to `{base}/mcp` (its client's `connect_http`),
/// not to `/mcp/streamable`.
const WAITER_MCP_ENDPOINT: &str = "/mcp";

/// The revision the waiter's client (1.16.2) asks for in `initialize`
/// (its `CLIENT_PROTOCOL_VERSION`).
const WAITER_REVISION: &str = "2025-11-25";

/// The revision the 1.16.1 client claimed, without the `_meta` it requires: the
/// requests Maidan began refusing on 2026-10-12.
const STATELESS_REVISION: &str = "2026-07-28";

struct Maidan {
    base: String,
    http: reqwest::Client,
    workspace_id: String,
    admin: String,
    // Holds the artifact store's directory for as long as the server runs.
    _artifacts: tempfile::TempDir,
}

/// A server with auth enabled, seeded the way `maidan init` seeds one: a
/// workspace, a human admin, and a token holding every capability, minted
/// through the store. The admin token is for setup and the final read only;
/// everything the waiter does is done with its own worker token.
async fn spawn() -> Maidan {
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
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "waiter-check".into(),
        })
        .await
        .unwrap();
    let admin = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "check".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: workspace.id,
            member_id: admin.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("maidan init (admin)".into()),
            capabilities: capability::all(),
            expires_at: None,
        })
        .await
        .unwrap();

    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Maidan {
        base: format!("http://{addr}"),
        http: reqwest::Client::new(),
        workspace_id: workspace.id.0.to_string(),
        admin: secret.as_str().to_string(),
        _artifacts: dir,
    }
}

impl Maidan {
    /// A JSON REST call that must succeed.
    async fn rest(&self, method: Method, path: &str, bearer: &str, body: Option<Value>) -> Value {
        let mut request = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .bearer_auth(bearer);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(
            status.is_success(),
            "{method} {path}: HTTP {status}: {text}"
        );
        serde_json::from_str(&text).unwrap()
    }

    /// The waiter's MCP client for `bearer`, before its handshake.
    fn waiter(&self, endpoint: &str, bearer: &str) -> WaiterMcp {
        WaiterMcp {
            http: self.http.clone(),
            url: format!("{}{endpoint}", self.base),
            bearer: bearer.to_string(),
            revision: WAITER_REVISION.to_string(),
            next_id: 1,
        }
    }
}

/// The producer's MCP `Client` over its `HttpTransport`, request for request.
///
/// Every POST carries `MCP-Protocol-Version` (the revision asked for until the
/// handshake, the one Maidan echoed after it) and the SEP-2243 `Mcp-Method`,
/// plus `Mcp-Name` on a `tools/call`. A request accepts only
/// `application/json`, and the client reads each reply body as one JSON-RPC object,
/// so a reply as SSE breaks it. The client never echoes `Mcp-Session-Id`: a session
/// Maidan required would break it as well.
struct WaiterMcp {
    http: reqwest::Client,
    url: String,
    bearer: String,
    revision: String,
    next_id: u64,
}

impl WaiterMcp {
    fn post(&self, method: &str, name: Option<&str>) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .post(&self.url)
            .bearer_auth(&self.bearer)
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", &self.revision)
            .header("Mcp-Method", method);
        if let Some(name) = name {
            request = request.header("Mcp-Name", name);
        }
        request
    }

    /// One JSON-RPC request; the reply's raw JSON object.
    async fn send(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let name = (method == "tools/call")
            .then(|| params["name"].as_str())
            .flatten();
        let response = self
            .post(method, name)
            .header("Accept", "application/json")
            .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        let body = String::from_utf8_lossy(&bytes);
        // The client's transport turns any non-2xx into an error before reading the body.
        assert!(status.is_success(), "{method}: HTTP {status}: {body}");
        let reply: Value = serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            panic!("{method}: the waiter reads the reply as one JSON-RPC object, not this ({err}): {body}")
        });
        assert_eq!(reply["id"], id, "{method}: the reply answers another id");
        reply
    }

    /// One request that must succeed; its `result`.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let reply = self.send(method, params).await;
        assert!(
            reply["error"].is_null(),
            "{method}: JSON-RPC error: {reply}"
        );
        reply["result"].clone()
    }

    /// `Client::initialize`: the handshake on the revision the client asks for,
    /// then the `notifications/initialized` it sends with `"id": null` and no params.
    async fn initialize(&mut self) {
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": WAITER_REVISION,
                    "clientInfo": { "name": "pi-mcp", "version": "1.16.2" },
                    "capabilities": { "tools": {} },
                }),
            )
            .await;
        let echoed = init["protocolVersion"]
            .as_str()
            .unwrap_or_else(|| panic!("initialize: no protocolVersion: {init}"));
        // The client stamps the echoed revision on every later request; a revision it
        // did not ask for would put it back on the rules that broke it.
        assert_eq!(echoed, WAITER_REVISION, "initialize: {init}");
        self.revision = echoed.to_string();

        let initialized = self
            .post("notifications/initialized", None)
            .json(&json!({ "jsonrpc": "2.0", "id": null, "method": "notifications/initialized" }))
            .send()
            .await
            .unwrap();
        assert!(
            initialized.status().is_success(),
            "notifications/initialized: HTTP {}",
            initialized.status()
        );
    }

    /// `Client::call_tool` plus the waiter's `extract_text` and `parse_text`:
    /// the tool must not report `isError`, and its first content block must
    /// be text holding JSON.
    async fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert_ne!(result["isError"], true, "{name}: isError: {result}");
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: no text in the first content block: {result}"));
        serde_json::from_str(text).unwrap_or_else(|err| {
            panic!("{name}: the first text block is not JSON ({err}): {text}")
        })
    }
}

/// The states the waiter's `ThreadState` can parse; any other fails the waiter.
const WAITER_THREAD_STATES: [&str; 4] = ["open", "in_review", "closed", "archived"];

/// The fields the waiter's `MaidanClaimedThread` requires.
fn assert_waiter_reads_a_claimed_thread(
    tool: &str,
    thread: &Value,
    thread_id: &str,
    channel: &str,
) {
    assert_eq!(thread["id"], thread_id, "{tool}: {thread}");
    assert_eq!(thread["channel_id"], channel, "{tool}: {thread}");
    assert!(
        thread["state"]
            .as_str()
            .is_some_and(|s| WAITER_THREAD_STATES.contains(&s)),
        "{tool}: a state the waiter cannot parse: {thread}"
    );
}

#[tokio::test]
async fn the_producers_waiter_round_trip_works_on_the_wire_it_uses() {
    let maidan = spawn().await;
    let admin = maidan.admin.clone();
    let wid = maidan.workspace_id.clone();

    // Setup, as the contract check makes it, with the admin token.
    let channel = maidan
        .rest(
            Method::POST,
            &format!("/workspaces/{wid}/channels"),
            &admin,
            Some(json!({ "name": "check", "private": false })),
        )
        .await;
    let cid = channel["id"].as_str().unwrap().to_string();
    let agent = maidan
        .rest(
            Method::POST,
            &format!("/workspaces/{wid}/agents"),
            &admin,
            Some(json!({ "handle": "waiter" })),
        )
        .await;
    let worker = agent["token"]["secret"].as_str().unwrap().to_string();
    let waiter = agent["member"]["id"].as_str().unwrap().to_string();
    assert_eq!(agent["token"]["member_id"], waiter.as_str());
    let thread = maidan
        .rest(
            Method::POST,
            &format!("/channels/{cid}/threads"),
            &admin,
            Some(json!({ "title": "contract check", "parent_thread_id": null })),
        )
        .await;
    let tid = thread["id"].as_str().unwrap().to_string();
    maidan
        .rest(
            Method::POST,
            &format!("/threads/{tid}/messages"),
            &admin,
            Some(json!({
                "body": "Contract check: a change task.",
                "metadata": { "pi.task_kind": "change" },
            })),
        )
        .await;

    // The waiter, with the worker token only, in `run_waiter_once`'s order.
    let mut client = maidan.waiter(WAITER_MCP_ENDPOINT, &worker);
    client.initialize().await;

    // No `lease_secs`: the waiter sends one only when configured with one.
    let claimed = client
        .call_tool("claim_next_thread", json!({ "channel_id": cid }))
        .await;
    assert_waiter_reads_a_claimed_thread("claim_next_thread", &claimed, &tid, &cid);
    assert_eq!(claimed["assignee_id"], waiter.as_str(), "{claimed}");
    let lease = claimed["claim_lease_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the waiter fences on claim_lease_id: {claimed}"))
        .to_string();

    let acked = client
        .call_tool(
            "acknowledge_claim",
            json!({ "thread_id": tid, "claim_lease_id": lease }),
        )
        .await;
    assert_waiter_reads_a_claimed_thread("acknowledge_claim", &acked, &tid, &cid);

    // The waiter's `MaidanThreadContext`: the first text block is the pack, with
    // `messages` of `{thread_id, author_id, body, metadata}`. The waiter runs
    // only when a message someone else wrote is there, and reads the task
    // kind from its metadata.
    let context = client
        .call_tool("get_thread_context", json!({ "thread_id": tid }))
        .await;
    let messages = context["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("get_thread_context: no messages: {context}"));
    for message in messages {
        for field in ["thread_id", "author_id", "body"] {
            assert!(
                message[field].is_string(),
                "get_thread_context: a message without {field}: {message}"
            );
        }
    }
    assert!(
        messages
            .iter()
            .any(|m| m["author_id"] != waiter.as_str()
                && m["metadata"]["pi.task_kind"] == "change"),
        "get_thread_context: the task message is not in the pack: {context}"
    );

    // The seat's proof, uploaded the way `ArtifactRest::upload` does it.
    let screenshot = b"\x89PNG\r\n\x1a\nproducer waiter contract check".to_vec();
    let sha = hex::encode(sha2::Sha256::digest(&screenshot));
    let uploaded = maidan
        .http
        .post(format!("{}/artifacts", maidan.base))
        .query(&[
            ("kind", "screenshot"),
            ("filename", "after.png"),
            ("mime_type", "image/png"),
        ])
        .bearer_auth(&worker)
        .header("content-type", "application/octet-stream")
        .body(screenshot.clone())
        .send()
        .await
        .unwrap();
    let status = uploaded.status();
    let uploaded: Value = uploaded.json().await.unwrap();
    assert!(
        status.is_success(),
        "POST /artifacts: HTTP {status}: {uploaded}"
    );
    // The waiter refuses an upload whose stored sha256 is not the one it sent.
    assert_eq!(uploaded["sha256"], sha.as_str(), "{uploaded}");

    // `usd_micros` is the waiter's own charge for the line, which Maidan recomputes:
    // 1200 × 3 + 340 × 15 micro-dollars.
    let usage = client
        .call_tool(
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
    // The waiter's `MaidanUsageReport`; `stopped: true` would abort the run.
    assert_eq!(usage["stopped"], false, "report_usage: {usage}");
    let budget = &usage["budget"];
    assert_eq!(budget["thread_id"], tid.as_str(), "report_usage: {usage}");
    for field in ["used_tokens", "used_usd_micros", "used_turns"] {
        assert!(budget[field].is_i64(), "report_usage: no {field}: {usage}");
    }

    // The result goes over REST, not the MCP tool of the same name: only the
    // REST write publishes the event that routes `deliver_to`.
    let run_id = uuid::Uuid::now_v7().to_string();
    let envelope = json!({
        "schema": maidan_types::WAITER_RESULT_SCHEMA,
        "status": "no_change",
        "result_kind": "pi.change.result/1",
        "run_id": run_id,
        "cost_usd": 0.0087,
        "deliver_to": [],
        "sandbox": "seat-cli",
        "edited": false,
        "exit_status": 0,
        "diff": "",
        "base_sha": "0123456789abcdef0123456789abcdef01234567",
        "summary": "Nothing to change.",
        "pi.result_attachments": [{
            "sha256": sha,
            "filename": "after.png",
            "mime_type": "image/png",
            "size_bytes": screenshot.len(),
            "role": "proof",
            "caption": "the page after the run",
        }],
    });
    maidan
        .rest(
            Method::PUT,
            &format!("/threads/{tid}/result"),
            &worker,
            Some(json!({ "result": envelope })),
        )
        .await;

    // Without `--keep-claimable` the waiter parks a finished task so it is not
    // handed out again, then releases its claim.
    client
        .call_tool(
            "mark_unclaimable",
            json!({ "thread_id": tid, "reason": "pi-waiter: completed" }),
        )
        .await;
    client
        .call_tool(
            "release_claim",
            json!({ "thread_id": tid, "claim_lease_id": lease }),
        )
        .await;

    // What the contract check reads back, as the admin.
    let stored = maidan
        .rest(Method::GET, &format!("/threads/{tid}/result"), &admin, None)
        .await;
    assert_eq!(stored["produced_by"], waiter.as_str(), "{stored}");
    let result = &stored["result"];
    assert!(
        result.as_object().is_some_and(|r| !r.is_empty()),
        "no result is stored for the claimed thread: {stored}"
    );
    assert_eq!(result["schema"], "maidan.waiter.result/1", "{result}");
    assert!(
        result["status"].as_str().is_some_and(|s| !s.is_empty()),
        "{result}"
    );
    assert_eq!(result["run_id"], run_id.as_str(), "{result}");
    assert_eq!(result["result_kind"], "pi.change.result/1", "{result}");
    assert_eq!(
        result["pi.result_attachments"], envelope["pi.result_attachments"],
        "the attachments did not round-trip"
    );

    let parsed = maidan_types::parse_waiter_result(result).unwrap_or_else(|| {
        panic!("Maidan does not route on the envelope the waiter wrote: {result}")
    });
    assert_eq!(parsed.result_kind, maidan_types::CHANGE_RESULT_KIND);
    assert!(parsed.deliver_to.is_empty());
    assert_eq!(
        parsed.attachments,
        vec![maidan_types::ResultAttachment {
            sha: sha.clone(),
            filename: "after.png".into(),
            caption: Some("the page after the run".into()),
        }]
    );
    let change = maidan_types::parse_change_result(result).unwrap();
    assert_eq!(change.status, "no_change");
    assert!(!change.is_changed(), "a no_change result commits nothing");
}

/// The 1.16.2 client's fix, pinned: a client that handshakes on `2025-11-25` and sends
/// no `_meta` is served. The 1.16.1 client claimed `2026-07-28` in the header without
/// the `_meta` that revision requires, and Maidan refuses that: the contract
/// that broke the waiter on 2026-10-12. Both MCP endpoints keep both rules.
#[tokio::test]
async fn a_2025_11_25_client_without_meta_is_served_and_a_2026_claim_without_it_is_refused() {
    let maidan = spawn().await;
    for endpoint in [WAITER_MCP_ENDPOINT, "/mcp/streamable"] {
        let mut fixed = maidan.waiter(endpoint, &maidan.admin);
        fixed.initialize().await;
        let me = fixed.call_tool("whoami", json!({})).await;
        assert!(me["member_id"].is_string(), "{endpoint}: whoami: {me}");

        let mut broken = maidan.waiter(endpoint, &maidan.admin);
        broken.revision = STATELESS_REVISION.to_string();
        for (method, params) in [
            (
                "initialize",
                json!({
                    "protocolVersion": STATELESS_REVISION,
                    "clientInfo": { "name": "pi-mcp", "version": "1.16.1" },
                    "capabilities": { "tools": {} },
                }),
            ),
            ("tools/call", json!({ "name": "whoami", "arguments": {} })),
        ] {
            let name = params["name"].as_str();
            let refused = broken
                .post(method, name)
                .header("Accept", "application/json")
                .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
                .send()
                .await
                .unwrap();
            let status = refused.status();
            let reply: Value = refused.json().await.unwrap();
            assert!(
                status.is_client_error(),
                "{endpoint} {method}: HTTP {status}: {reply}"
            );
            assert!(reply["result"].is_null(), "{endpoint} {method}: {reply}");
            assert_eq!(
                reply["error"]["code"], -32602,
                "{endpoint} {method}: {reply}"
            );
            assert!(
                reply["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("io.modelcontextprotocol/protocolVersion")),
                "{endpoint} {method}: the refusal does not name the missing _meta: {reply}"
            );
        }
    }
}
