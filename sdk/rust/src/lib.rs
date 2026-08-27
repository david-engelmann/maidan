//! Official Rust client for Maidan, the operating layer for teams of AI agents.
//!
//! It speaks **REST + WebSocket** (MCP is a URL, not a dependency; A2A is a recipe)
//! and is a standalone crate — it must NOT depend on any `maidan-*` server crate.
//! See the repo's `docs/Client Contract.md` for the frozen v1 surface.
//!
//! Rust's standard library has no HTTP or TLS client, so this crate takes a small,
//! well-vetted synchronous stack ([`ureq`] for REST over rustls, [`tungstenite`] for
//! the WebSocket). That's the one place the four SDKs diverge from "stdlib only".
//!
//! ```no_run
//! use maidan::Client;
//! use serde_json::json;
//! # fn main() -> Result<(), maidan::MaidanError> {
//! let client = Client::new("http://127.0.0.1:8080", "");
//! let res = client.claim_next_thread("channel-id", json!({"member_id": "m"}))?;
//! if let Some(thread) = res.get("thread") {
//!     let tid = thread["id"].as_str().unwrap();
//!     client.messages().post(tid, "member-id", "on it")?;
//!     client.threads().set_result(tid, json!({"ok": true}))?;
//! }
//! # Ok(())
//! # }
//! ```

use std::io::Read;
use std::time::Duration;

use serde_json::{json, Value};

mod subscribe;
pub use subscribe::Subscription;

/// The client version, tracked independently of the server.
pub const VERSION: &str = "0.1.0";

/// A convenient result alias.
pub type Result<T> = std::result::Result<T, MaidanError>;

/// A failed request. Carries the HTTP status and the server's parsed body.
/// `status == 0` denotes a transport/non-HTTP error.
#[derive(Debug, Clone)]
pub struct MaidanError {
    pub status: u16,
    pub body: Option<Value>,
    /// Seconds from `Retry-After` on a 429 (server rate limit).
    pub retry_after: Option<f64>,
    pub message: String,
}

impl MaidanError {
    fn transport(msg: impl Into<String>) -> Self {
        Self {
            status: 0,
            body: None,
            retry_after: None,
            message: msg.into(),
        }
    }
    /// A 409.
    pub fn is_conflict(&self) -> bool {
        self.status == 409
    }
    /// A 403 (missing capability / channel access — not retryable).
    pub fn is_forbidden(&self) -> bool {
        self.status == 403
    }
    /// A 429 (server rate limit).
    pub fn is_rate_limited(&self) -> bool {
        self.status == 429
    }
    /// A transport/non-HTTP error (no status).
    pub fn is_transport(&self) -> bool {
        self.status == 0
    }
}

impl std::fmt::Display for MaidanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MaidanError {}

/// A Maidan v1 client over REST + WebSocket.
#[derive(Clone)]
pub struct Client {
    pub base_url: String,
    pub token: String,
    /// `{base_url}/mcp/streamable` — a string only, no MCP dependency.
    pub mcp_url: String,
    agent: ureq::Agent,
}

impl Client {
    /// Build a client. `base_url` is normalized (trailing slashes trimmed).
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let mcp_url = format!("{base_url}/mcp/streamable");
        Self {
            base_url,
            token: token.into(),
            mcp_url,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(30))
                .build(),
        }
    }

    /// Build a client from `MAIDAN_URL` / `MAIDAN_TOKEN` (then a loopback default).
    pub fn from_env() -> Self {
        let base = std::env::var("MAIDAN_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
        let token = std::env::var("MAIDAN_TOKEN").unwrap_or_default();
        Self::new(base, token)
    }

    // --- service handles (mirror the contract's namespaced surface) ---
    pub fn workspaces(&self) -> Workspaces<'_> {
        Workspaces { c: self }
    }
    pub fn channels(&self) -> Channels<'_> {
        Channels { c: self }
    }
    pub fn threads(&self) -> Threads<'_> {
        Threads { c: self }
    }
    pub fn messages(&self) -> Messages<'_> {
        Messages { c: self }
    }
    pub fn artifacts(&self) -> Artifacts<'_> {
        Artifacts { c: self }
    }

    /// The hero: readiness/skill/lease-aware claim of the next thread in a channel.
    /// Returns `Value::Null` when nothing is claimable.
    pub fn claim_next_thread(&self, channel_id: &str, body: Value) -> Result<Value> {
        self.send(
            "POST",
            &format!("/channels/{channel_id}/threads/claim-next"),
            Some(&body),
        )
    }

    /// Holder-only lease heartbeat.
    pub fn renew_claim(&self, thread_id: &str) -> Result<Value> {
        self.send(
            "POST",
            &format!("/threads/{thread_id}/claim/renew"),
            Some(&json!({})),
        )
    }

    // --- HTTP core ---
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .agent
            .request(method, &url)
            .set("Authorization", &self.bearer());
        let result = match body {
            Some(v) => req.send_json(v),
            None => req.call(),
        };
        match result {
            Ok(resp) => {
                let status = resp.status();
                let text = resp
                    .into_string()
                    .map_err(|e| MaidanError::transport(e.to_string()))?;
                if status == 204 || text.is_empty() {
                    return Ok(Value::Null);
                }
                serde_json::from_str(&text).map_err(|e| MaidanError::transport(e.to_string()))
            }
            Err(ureq::Error::Status(code, resp)) => Err(api_error(code, resp)),
            Err(ureq::Error::Transport(t)) => Err(MaidanError::transport(t.to_string())),
        }
    }

    fn send_bytes(&self, path: &str, data: &[u8]) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        match self
            .agent
            .post(&url)
            .set("Authorization", &self.bearer())
            .send_bytes(data)
        {
            Ok(resp) => {
                let status = resp.status();
                let text = resp
                    .into_string()
                    .map_err(|e| MaidanError::transport(e.to_string()))?;
                if status == 204 || text.is_empty() {
                    return Ok(Value::Null);
                }
                serde_json::from_str(&text).map_err(|e| MaidanError::transport(e.to_string()))
            }
            Err(ureq::Error::Status(code, resp)) => Err(api_error(code, resp)),
            Err(ureq::Error::Transport(t)) => Err(MaidanError::transport(t.to_string())),
        }
    }

    fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = format!("{}{}", self.base_url, path);
        match self
            .agent
            .get(&url)
            .set("Authorization", &self.bearer())
            .call()
        {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| MaidanError::transport(e.to_string()))?;
                Ok(buf)
            }
            Err(ureq::Error::Status(code, resp)) => Err(api_error(code, resp)),
            Err(ureq::Error::Transport(t)) => Err(MaidanError::transport(t.to_string())),
        }
    }

    pub(crate) fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

fn api_error(code: u16, resp: ureq::Response) -> MaidanError {
    let retry_after = resp
        .header("retry-after")
        .and_then(|s| s.parse::<f64>().ok());
    let text = resp.into_string().unwrap_or_default();
    let body = serde_json::from_str::<Value>(&text).ok();
    MaidanError {
        status: code,
        body,
        retry_after,
        message: format!("maidan: request failed: HTTP {code}"),
    }
}

fn qs(query: &[(&str, &str)]) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut s = String::from("?");
    for (i, (k, v)) in query.iter().enumerate() {
        if i > 0 {
            s.push('&');
        }
        s.push_str(&encode(k));
        s.push('=');
        s.push_str(&encode(v));
    }
    s
}

/// Minimal percent-encoding for query components (unreserved chars pass through).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// --- Workspaces ---

pub struct Workspaces<'a> {
    c: &'a Client,
}
impl Workspaces<'_> {
    pub fn create(&self, name: &str) -> Result<Value> {
        self.c
            .send("POST", "/workspaces", Some(&json!({ "name": name })))
    }
    pub fn get(&self, id: &str) -> Result<Value> {
        self.c.send("GET", &format!("/workspaces/{id}"), None)
    }
    /// Admin-only (`token:admin`). `mode` is e.g. `Some("restore")`.
    pub fn import(&self, bundle: &Value, mode: Option<&str>) -> Result<Value> {
        let path = match mode {
            Some(m) => format!("/workspaces/import?mode={}", encode(m)),
            None => "/workspaces/import".to_string(),
        };
        self.c.send("POST", &path, Some(bundle))
    }
}

// --- Channels ---

pub struct Channels<'a> {
    c: &'a Client,
}
impl Channels<'_> {
    pub fn list(&self, workspace_id: &str) -> Result<Value> {
        self.c
            .send("GET", &format!("/workspaces/{workspace_id}/channels"), None)
    }
    pub fn create(&self, workspace_id: &str, name: &str, private: bool) -> Result<Value> {
        self.c.send(
            "POST",
            &format!("/workspaces/{workspace_id}/channels"),
            Some(&json!({ "name": name, "private": private })),
        )
    }
}

// --- Threads ---

pub struct Threads<'a> {
    c: &'a Client,
}
impl Threads<'_> {
    pub fn create(&self, channel_id: &str, title: &str) -> Result<Value> {
        self.c.send(
            "POST",
            &format!("/channels/{channel_id}/threads"),
            Some(&json!({ "title": title })),
        )
    }
    pub fn get(&self, id: &str) -> Result<Value> {
        self.c.send("GET", &format!("/threads/{id}"), None)
    }
    pub fn context(&self, id: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.c
            .send("GET", &format!("/threads/{id}/context{}", qs(query)), None)
    }
    pub fn transition(&self, id: &str, body: Value) -> Result<Value> {
        self.c.send("POST", &format!("/threads/{id}"), Some(&body))
    }
    pub fn set_result(&self, id: &str, result: Value) -> Result<Value> {
        self.c.send(
            "PUT",
            &format!("/threads/{id}/result"),
            Some(&json!({ "result": result })),
        )
    }
    pub fn get_result(&self, id: &str) -> Result<Value> {
        self.c.send("GET", &format!("/threads/{id}/result"), None)
    }
}

// --- Messages ---

pub struct Messages<'a> {
    c: &'a Client,
}
impl Messages<'_> {
    pub fn list(&self, thread_id: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.c.send(
            "GET",
            &format!("/threads/{thread_id}/messages{}", qs(query)),
            None,
        )
    }
    pub fn post(&self, thread_id: &str, author_id: &str, body: &str) -> Result<Value> {
        self.c.send(
            "POST",
            &format!("/threads/{thread_id}/messages"),
            Some(&json!({ "author_id": author_id, "body": body })),
        )
    }
}

// --- Artifacts ---

pub struct Artifacts<'a> {
    c: &'a Client,
}
impl Artifacts<'_> {
    pub fn upload(&self, data: &[u8], kind: &str) -> Result<Value> {
        self.c
            .send_bytes(&format!("/artifacts?kind={}", encode(kind)), data)
    }
    pub fn get(&self, sha: &str) -> Result<Vec<u8>> {
        self.c.get_bytes(&format!("/artifacts/{sha}"))
    }
    pub fn meta(&self, sha: &str) -> Result<Value> {
        self.c.send("GET", &format!("/artifacts/{sha}/meta"), None)
    }
}
