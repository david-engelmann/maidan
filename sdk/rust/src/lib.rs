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
//! Responses are the typed models in [`models`] (re-exported at the root), and
//! a failure is the [`MaidanError`] variant its RFC 9457 problem `type` names.
//!
//! ```no_run
//! use maidan::{Client, MaidanError};
//! use serde_json::json;
//! # fn main() -> Result<(), MaidanError> {
//! let client = Client::new("http://127.0.0.1:8080", "");
//! if let Some(claim) = client.claim_next_thread("channel-id", None)? {
//!     client.messages().post(&claim.id, "on it")?;
//!     client.threads().set_result(&claim.id, json!({"ok": true}))?;
//! }
//! match client.threads().get("no-such-thread") {
//!     Err(MaidanError::NotFound(problem)) => eprintln!("gone: {:?}", problem.detail),
//!     other => drop(other),
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

mod error;
pub mod models;
mod subscribe;
pub use error::{MaidanError, Problem, PROBLEM_BASE, PROBLEM_TYPES};
pub use models::*;
pub use subscribe::{Follow, Subscription};

/// The client version, tracked independently of the server.
pub const VERSION: &str = "0.3.0";

/// Wire name of the projector-lag header (HTTP is case-insensitive).
/// Distinct from `Maidan-Consistency-Token` (Postgres WAL LSN).
pub const ROOM_LSN_HEADER: &str = "maidan-room-lsn";

/// Parse `Maidan-Room-LSN`. Rejects WAL text (`0/hex`) so this is never
/// confused with `Maidan-Consistency-Token`.
pub fn parse_room_lsn(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.contains('/') {
        return None;
    }
    trimmed.parse::<i64>().ok().filter(|&n| n >= 0)
}

/// Observable `$type` for an event `kind` (`message_posted` → `maidan.event.message_posted/1`).
pub fn event_type(kind: &str) -> String {
    format!("maidan.event.{kind}/1")
}

/// A convenient result alias.
pub type Result<T> = std::result::Result<T, MaidanError>;

const IN_FLIGHT_TYPE: &str = "https://maidan.dev/problems/idempotency-key-in-flight";

/// 64 random bits from the std hasher's per-process random keys, mixed with
/// the clock and a counter. Enough for a unique key and for jitter; not a
/// secret.
fn random_u64() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.finish()
}

/// A fresh `Idempotency-Key` (a v4-shaped UUID): one per logical write,
/// reused by its retries.
pub fn new_idempotency_key() -> String {
    let (a, b) = (random_u64(), random_u64());
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The wait before retry `attempt` (0-based): the server's `Retry-After` when
/// it sent one (capped at 60s), else 0.5s·2^attempt capped at 8s, jittered by
/// `unit` in `[0, 1)`.
pub fn retry_delay(attempt: u32, retry_after: Option<&str>, unit: f64) -> Duration {
    if let Some(ra) = retry_after.and_then(|s| s.trim().parse::<f64>().ok()) {
        if ra >= 0.0 {
            return Duration::from_secs_f64(ra.min(60.0));
        }
    }
    let base = (0.5 * 2f64.powi(attempt.min(16) as i32)).min(8.0);
    Duration::from_secs_f64(base / 2.0 + unit * base / 2.0)
}

fn retryable(status: u16, raw: &[u8]) -> bool {
    match status {
        408 | 429 | 500 | 502 | 503 | 504 => true,
        409 => serde_json::from_slice::<Value>(raw)
            .ok()
            .and_then(|v| {
                v.get("type")
                    .and_then(Value::as_str)
                    .map(|t| t == IN_FLIGHT_TYPE)
            })
            .unwrap_or(false),
        _ => false,
    }
}

enum Payload<'a> {
    None,
    Json(&'a Value),
    Bytes(&'a [u8]),
}

/// One answer: status, `Retry-After`, body.
struct Answer {
    status: u16,
    retry_after: Option<String>,
    raw: Vec<u8>,
}

type Sleeper = Arc<dyn Fn(Duration) + Send + Sync>;

/// A Maidan v1 client over REST + WebSocket.
#[derive(Clone)]
pub struct Client {
    pub base_url: String,
    pub token: String,
    /// `{base_url}/mcp/streamable` — a string only, no MCP dependency.
    pub mcp_url: String,
    agent: ureq::Agent,
    /// Last seen `Maidan-Room-LSN` (event-log high-water). Not a WAL token.
    last_room_lsn: Arc<AtomicI64>,
    max_retries: u32,
    sleep: Sleeper,
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
            last_room_lsn: Arc::new(AtomicI64::new(-1)),
            max_retries: 2,
            sleep: Arc::new(std::thread::sleep),
        }
    }

    /// Bound the retries of a request that failed in transit or answered
    /// 408, 429 (honouring `Retry-After`), 500/502/503/504, or a 409
    /// `idempotency-key-in-flight`. Default 2; 0 turns retries off. Writes
    /// carry one `Idempotency-Key` across their attempts.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Replace how the client waits between attempts (a test seam).
    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Arc::new(sleep);
        self
    }

    /// Highest `maidan_events.id` from the last REST response, if the server
    /// stamped `Maidan-Room-LSN`. `None` until a stamped response is seen.
    pub fn last_room_lsn(&self) -> Option<i64> {
        let value = self.last_room_lsn.load(Ordering::Relaxed);
        (value >= 0).then_some(value)
    }

    fn capture_room_lsn_header(&self, raw: Option<&str>) {
        if let Some(n) = raw.and_then(parse_room_lsn) {
            self.last_room_lsn.store(n, Ordering::Relaxed);
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

    /// `GET /workspaces/{id}/events` — projector-shaped HTTP backfill.
    /// Query keys: `after_id`, `limit`, `channel_id`, `thread_id`, `types`,
    /// `consumer_id`. A pruned-gap cursor is 409 [`MaidanError::is_cursor_too_old`].
    pub fn list_events(
        &self,
        workspace_id: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<StoredEvent>> {
        self.send_as(
            "GET",
            &format!("/workspaces/{workspace_id}/events{}", qs(query)),
            None,
        )
    }
    /// Member provisioning. [`Members::create`] is the unauthenticated seed
    /// route, present only on a server built with the `bootstrap` feature;
    /// production turns it off and provisions through `maidan init` plus
    /// [`Client::tokens`].
    pub fn members(&self) -> Members<'_> {
        Members { c: self }
    }
    /// Per-agent bearer tokens. Needs `token:admin` — the capability the admin
    /// token from `maidan init` carries.
    pub fn tokens(&self) -> Tokens<'_> {
        Tokens { c: self }
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

    /// The hero: readiness/skill/lease-aware claim of the next thread in a
    /// channel, with an optional lease length in seconds. `Ok(None)` when
    /// nothing is claimable.
    pub fn claim_next_thread(
        &self,
        channel_id: &str,
        lease_secs: Option<i64>,
    ) -> Result<Option<ClaimedThread>> {
        let body = match lease_secs {
            Some(secs) => json!({ "lease_secs": secs }),
            None => json!({}),
        };
        self.send_as(
            "POST",
            &format!("/channels/{channel_id}/threads/claim-next"),
            Some(&body),
        )
    }

    /// Holder-only lease heartbeat.
    pub fn renew_claim(
        &self,
        thread_id: &str,
        claim_lease_id: &str,
        lease_secs: i64,
    ) -> Result<Thread> {
        self.send_as(
            "POST",
            &format!("/threads/{thread_id}/claim/renew"),
            Some(&json!({
                "claim_lease_id": claim_lease_id,
                "lease_secs": lease_secs,
            })),
        )
    }

    // --- HTTP core ---

    /// Send with retries and return the last answer. A write carries one
    /// `Idempotency-Key` across all its attempts, so a retry after a lost
    /// response gets the first answer back instead of writing twice.
    fn exchange(&self, method: &str, path: &str, payload: Payload<'_>) -> Result<Answer> {
        let url = format!("{}{}", self.base_url, path);
        let key = matches!(method, "POST" | "PUT" | "PATCH" | "DELETE").then(new_idempotency_key);
        let mut attempt = 0u32;
        loop {
            let mut req = self
                .agent
                .request(method, &url)
                .set("Authorization", &self.bearer());
            if let Some(key) = &key {
                req = req.set("Idempotency-Key", key);
            }
            let result = match payload {
                Payload::None => req.call(),
                Payload::Json(v) => req.send_json(v),
                Payload::Bytes(b) => req.send_bytes(b),
            };
            let resp = match result {
                Ok(resp) | Err(ureq::Error::Status(_, resp)) => resp,
                Err(ureq::Error::Transport(t)) => {
                    if attempt >= self.max_retries {
                        return Err(MaidanError::transport(t.to_string()));
                    }
                    (self.sleep)(retry_delay(attempt, None, unit()));
                    attempt += 1;
                    continue;
                }
            };
            self.capture_room_lsn_header(resp.header(ROOM_LSN_HEADER));
            let status = resp.status();
            let retry_after = resp.header("retry-after").map(str::to_string);
            let mut raw = Vec::new();
            resp.into_reader()
                .read_to_end(&mut raw)
                .map_err(|e| MaidanError::transport(e.to_string()))?;
            if attempt < self.max_retries && retryable(status, &raw) {
                (self.sleep)(retry_delay(attempt, retry_after.as_deref(), unit()));
                attempt += 1;
                continue;
            }
            return Ok(Answer {
                status,
                retry_after,
                raw,
            });
        }
    }

    fn decode<T: DeserializeOwned>(answer: Answer) -> Result<T> {
        if answer.status >= 400 {
            return Err(api_error(answer));
        }
        let raw: &[u8] = if answer.status == 204 || answer.raw.is_empty() {
            b"null"
        } else {
            &answer.raw
        };
        serde_json::from_slice(raw).map_err(|e| MaidanError::Decode(e.to_string()))
    }

    fn send_as<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let payload = body.map_or(Payload::None, Payload::Json);
        Self::decode(self.exchange(method, path, payload)?)
    }

    fn send_bytes<T: DeserializeOwned>(&self, path: &str, data: &[u8]) -> Result<T> {
        Self::decode(self.exchange("POST", path, Payload::Bytes(data))?)
    }

    fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let answer = self.exchange("GET", path, Payload::None)?;
        if answer.status >= 400 {
            return Err(api_error(answer));
        }
        Ok(answer.raw)
    }

    /// Every event after `after_id`, `limit` (default 100) per page, fetched
    /// as the iterator is consumed. Other `query` keys pass through.
    pub fn list_events_all<'a>(
        &'a self,
        workspace_id: &'a str,
        query: &[(&str, &str)],
    ) -> impl Iterator<Item = Result<StoredEvent>> + 'a {
        let owned: Vec<(String, String)> = query
            .iter()
            .filter(|(k, _)| *k != "after_id" && *k != "limit")
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let find = |key: &str| query.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        let limit: usize = find("limit")
            .and_then(|v| v.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(100);
        let mut after: i64 = find("after_id").and_then(|v| v.parse().ok()).unwrap_or(0);
        Pager::new(move || {
            let limit_s = limit.to_string();
            let after_s = after.to_string();
            let mut q: Vec<(&str, &str)> = owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            q.push(("after_id", &after_s));
            q.push(("limit", &limit_s));
            let page = self.list_events(workspace_id, &q)?;
            if let Some(last) = page.iter().map(|row| row.id).max() {
                after = after.max(last);
            }
            let done = page.len() < limit;
            Ok((page, done))
        })
    }

    pub(crate) fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

fn unit() -> f64 {
    (random_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// An iterator over items fetched a page at a time by `fetch`, which returns
/// the page and whether it was the last. An error ends the iteration after
/// it is yielded.
struct Pager<T, F> {
    fetch: F,
    buf: std::collections::VecDeque<T>,
    done: bool,
}

impl<T, F> Pager<T, F> {
    fn new(fetch: F) -> Self {
        Self {
            fetch,
            buf: Default::default(),
            done: false,
        }
    }
}

impl<T, F: FnMut() -> Result<(Vec<T>, bool)>> Iterator for Pager<T, F> {
    type Item = Result<T>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(v) = self.buf.pop_front() {
                return Some(Ok(v));
            }
            if self.done {
                return None;
            }
            match (self.fetch)() {
                Ok((page, last)) => {
                    self.done = last;
                    self.buf.extend(page);
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

fn api_error(answer: Answer) -> MaidanError {
    let retry_after = answer
        .retry_after
        .and_then(|s| s.trim().parse::<f64>().ok());
    MaidanError::from_response(answer.status, &answer.raw, retry_after)
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
    pub fn create(&self, name: &str) -> Result<Workspace> {
        self.c
            .send_as("POST", "/workspaces", Some(&json!({ "name": name })))
    }
    pub fn get(&self, id: &str) -> Result<Workspace> {
        self.c.send_as("GET", &format!("/workspaces/{id}"), None)
    }
    /// Admin-only (`token:admin`). `bundle` is a signed
    /// `maidan.workspace.export/1` envelope; `mode` `None` is the server's
    /// default ([`ImportMode::New`]).
    pub fn import(&self, bundle: &Value, mode: Option<ImportMode>) -> Result<ImportResult> {
        let path = match mode {
            Some(m) => format!("/workspaces/import?mode={}", encode(m.as_str())),
            None => "/workspaces/import".to_string(),
        };
        self.c.send_as("POST", &path, Some(bundle))
    }
}

// --- Members ---

pub struct Members<'a> {
    c: &'a Client,
}

impl Members<'_> {
    pub fn create(
        &self,
        workspace_id: &str,
        handle: &str,
        kind: MemberKind,
        display_name: Option<&str>,
    ) -> Result<Member> {
        let mut body = json!({ "handle": handle, "kind": kind });
        if let Some(name) = display_name {
            body["display_name"] = json!(name);
        }
        self.c.send_as(
            "POST",
            &format!("/workspaces/{workspace_id}/members"),
            Some(&body),
        )
    }

    pub fn list(&self, workspace_id: &str) -> Result<Vec<Member>> {
        self.c
            .send_as("GET", &format!("/workspaces/{workspace_id}/members"), None)
    }
}

// --- Tokens ---

/// Optional fields of a mint. `capability_set` is a named set
/// (`maidan.agent.worker` / `maidan.human.admin`); combined with the requested
/// capabilities it is a progressive grant, so the request must be a subset.
#[derive(Debug, Default, Clone)]
pub struct MintOptions<'a> {
    pub label: Option<&'a str>,
    pub capability_set: Option<&'a str>,
    pub expires_at: Option<&'a str>,
}

pub struct Tokens<'a> {
    c: &'a Client,
}

impl Tokens<'_> {
    /// Returns the secret **once**, in the response; it is never retrievable
    /// again.
    pub fn mint(
        &self,
        workspace_id: &str,
        member_id: &str,
        capabilities: &[&str],
        opts: &MintOptions<'_>,
    ) -> Result<MintedToken> {
        let mut body = json!({ "capabilities": capabilities });
        if let Some(label) = opts.label {
            body["label"] = json!(label);
        }
        if let Some(set) = opts.capability_set {
            body["capability_set"] = json!(set);
        }
        if let Some(expires) = opts.expires_at {
            body["expires_at"] = json!(expires);
        }
        self.c.send_as(
            "POST",
            &format!("/workspaces/{workspace_id}/members/{member_id}/tokens"),
            Some(&body),
        )
    }

    /// Token metadata only — never a secret.
    pub fn list(&self, workspace_id: &str, member_id: &str) -> Result<Vec<TokenSummary>> {
        self.c.send_as(
            "GET",
            &format!("/workspaces/{workspace_id}/members/{member_id}/tokens"),
            None,
        )
    }
}

// --- Channels ---

pub struct Channels<'a> {
    c: &'a Client,
}
impl Channels<'_> {
    pub fn list(&self, workspace_id: &str) -> Result<Vec<Channel>> {
        self.c
            .send_as("GET", &format!("/workspaces/{workspace_id}/channels"), None)
    }
    pub fn create(&self, workspace_id: &str, name: &str, private: bool) -> Result<Channel> {
        self.c.send_as(
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
    pub fn create(&self, channel_id: &str, title: &str) -> Result<Thread> {
        self.c.send_as(
            "POST",
            &format!("/channels/{channel_id}/threads"),
            Some(&json!({ "title": title })),
        )
    }
    pub fn get(&self, id: &str) -> Result<Thread> {
        self.c.send_as("GET", &format!("/threads/{id}"), None)
    }
    /// `GET /channels/{cid}/threads` — one page (`limit`, `cursor` = last thread id).
    pub fn list(&self, channel_id: &str, query: &[(&str, &str)]) -> Result<Vec<Thread>> {
        self.c.send_as(
            "GET",
            &format!("/channels/{channel_id}/threads{}", qs(query)),
            None,
        )
    }
    /// Every live thread in the channel, `page_size` (0 = 100) per request,
    /// fetched as the iterator is consumed.
    pub fn list_all(
        &self,
        channel_id: &str,
        page_size: usize,
    ) -> impl Iterator<Item = Result<Thread>> + '_ {
        let page_size = if page_size == 0 { 100 } else { page_size };
        let channel_id = channel_id.to_string();
        let c = self.c;
        let mut cursor: Option<String> = None;
        Pager::new(move || {
            let limit = page_size.to_string();
            let mut q = vec![("limit", limit.as_str())];
            if let Some(cur) = &cursor {
                q.push(("cursor", cur.as_str()));
            }
            let page = c.threads().list(&channel_id, &q)?;
            cursor = page.last().map(|t| t.id.clone());
            let done = page.len() < page_size;
            Ok((page, done))
        })
    }
    pub fn context(&self, id: &str, query: &[(&str, &str)]) -> Result<ThreadContext> {
        self.c
            .send_as("GET", &format!("/threads/{id}/context{}", qs(query)), None)
    }
    /// Move the thread's FSM: `action` is `start_review`, `close` or `archive`.
    pub fn transition(&self, id: &str, action: &str) -> Result<Thread> {
        self.c.send_as(
            "POST",
            &format!("/threads/{id}"),
            Some(&json!({ "action": action })),
        )
    }
    pub fn set_result(&self, id: &str, result: Value) -> Result<ThreadResult> {
        self.c.send_as(
            "PUT",
            &format!("/threads/{id}/result"),
            Some(&json!({ "result": result })),
        )
    }
    pub fn get_result(&self, id: &str) -> Result<ThreadResult> {
        self.c
            .send_as("GET", &format!("/threads/{id}/result"), None)
    }
}

// --- Messages ---

pub struct Messages<'a> {
    c: &'a Client,
}
impl Messages<'_> {
    pub fn list(&self, thread_id: &str, query: &[(&str, &str)]) -> Result<Vec<Message>> {
        self.c.send_as(
            "GET",
            &format!("/threads/{thread_id}/messages{}", qs(query)),
            None,
        )
    }
    pub fn post(&self, thread_id: &str, body: &str) -> Result<Message> {
        self.c.send_as(
            "POST",
            &format!("/threads/{thread_id}/messages"),
            Some(&json!({ "body": body })),
        )
    }
}

// --- Artifacts ---

pub struct Artifacts<'a> {
    c: &'a Client,
}
impl Artifacts<'_> {
    pub fn upload(&self, data: &[u8], kind: ArtifactKind) -> Result<Artifact> {
        self.c
            .send_bytes(&format!("/artifacts?kind={}", encode(kind.as_str())), data)
    }
    pub fn get(&self, sha: &str) -> Result<Vec<u8>> {
        self.c.get_bytes(&format!("/artifacts/{sha}"))
    }
    pub fn meta(&self, sha: &str) -> Result<Artifact> {
        self.c
            .send_as("GET", &format!("/artifacts/{sha}/meta"), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(status: u16, body: Value) -> MaidanError {
        MaidanError::from_response(status, body.to_string().as_bytes(), None)
    }

    #[test]
    fn cursor_too_old_is_409_must_refetch_not_a_plain_conflict() {
        let too_old = err(
            409,
            json!({
                "type": "https://maidan.dev/problems/cursor-too-old",
                "must_refetch": true,
                "snapshot": "/workspaces/w/snapshot"
            }),
        );
        assert!(too_old.is_conflict());
        assert!(too_old.is_cursor_too_old());
        let MaidanError::CursorTooOld(problem) = &too_old else {
            panic!("expected CursorTooOld, got {too_old:?}");
        };
        assert_eq!(problem.snapshot(), Some("/workspaces/w/snapshot"));

        let by_type_only = err(
            409,
            json!({ "type": "https://maidan.dev/problems/cursor-too-old" }),
        );
        assert!(by_type_only.is_cursor_too_old());

        let plain = err(
            409,
            json!({ "type": "https://maidan.dev/problems/conflict" }),
        );
        assert!(plain.is_conflict());
        assert!(!plain.is_cursor_too_old());

        let refetch_wrong_status = err(500, json!({ "must_refetch": true }));
        assert!(!refetch_wrong_status.is_cursor_too_old());
    }

    #[test]
    fn parse_room_lsn_accepts_decimal_and_rejects_wal() {
        assert_eq!(parse_room_lsn("42"), Some(42));
        assert_eq!(parse_room_lsn(" 0 "), Some(0));
        assert_eq!(parse_room_lsn("0/3000128"), None);
        assert_eq!(parse_room_lsn("-1"), None);
        assert_eq!(parse_room_lsn(""), None);
        assert_eq!(
            event_type("message_posted"),
            "maidan.event.message_posted/1"
        );
    }
}
