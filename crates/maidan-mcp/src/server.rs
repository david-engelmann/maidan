//! MCP dispatcher. Takes JSON-RPC requests and returns responses.
//! Transport-agnostic; `maidan-server` wraps it behind `POST /mcp`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use maidan_artifacts::ArtifactStore;
use maidan_auth::AuthContext;
use maidan_bus::{EventBus, ResourceNotifier};
use maidan_search::{EmbeddingProvider, Search};
use maidan_store::Store;
use maidan_types::{BusEnvelope, Event, StoredEvent};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use crate::streamable_session::StreamableSessionRegistry;

const NOTIFICATION_BROADCAST_CAPACITY: usize = 64;

use crate::error::McpError;
use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::{prompts, resources, tools};

/// Protocol revisions this server implements, newest first. `initialize`
/// negotiates against the client's requested version, and the HTTP transports
/// validate the `MCP-Protocol-Version` header against this set. The **default is
/// `2026-07-28`** ([`DEFAULT_PROTOCOL_VERSION`]) — the current revision, fully
/// landed and advertised (negotiation, stateless streamable core, and SEP-2243
/// routing headers; Protocols.md J3.1–3.5). `2024-11-05` stays supported for
/// older clients that request it explicitly.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2026-07-28", "2024-11-05"];
const NOTIFY_RESOURCE_UPDATED: &str = "notifications/resources/updated";

/// The version `initialize` echoes when the client requests none, and the
/// fallback when it requests an unsupported one. Now `2026-07-28` (the current
/// revision — negotiation, stateless streamable, and routing headers all landed:
/// Protocols.md J3.1–3.4). `2024-11-05` stays fully supported for older clients
/// that request it explicitly (they still get the SSE-session transport).
const DEFAULT_PROTOCOL_VERSION: &str = "2026-07-28";

/// See [`DEFAULT_PROTOCOL_VERSION`].
pub fn preferred_protocol_version() -> &'static str {
    DEFAULT_PROTOCOL_VERSION
}

/// Whether `version` is one this server implements — used to validate the
/// `MCP-Protocol-Version` header.
pub fn is_supported_protocol_version(version: &str) -> bool {
    SUPPORTED_PROTOCOL_VERSIONS.contains(&version)
}

/// Negotiate the `initialize` protocol version: echo the client's requested
/// version if supported, else the preferred one (MCP spec §Lifecycle).
fn negotiate_protocol_version(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|v| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .find(|s| **s == v)
                .copied()
        })
        .unwrap_or_else(preferred_protocol_version)
}

/// How long a server→client request waits for the client's response.
const CLIENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The client capability a server→client method requires, or `None` if the
/// method is not one the server may initiate.
fn client_capability_for_method(method: &str) -> Option<&'static str> {
    match method {
        "sampling/createMessage" => Some("sampling"),
        "roots/list" => Some("roots"),
        "elicitation/create" => Some("elicitation"),
        _ => None,
    }
}

#[derive(Clone)]
pub struct McpServer {
    pub(crate) store: Arc<dyn Store>,
    pub(crate) artifacts: Arc<dyn ArtifactStore>,
    pub(crate) search: Arc<dyn Search>,
    pub(crate) embedding_provider: Arc<dyn EmbeddingProvider>,
    server_name: String,
    server_version: String,
    subscriptions: Arc<Mutex<HashSet<String>>>,
    pending_notifications: Arc<Mutex<Vec<JsonRpcNotification>>>,
    notification_tx: broadcast::Sender<JsonRpcNotification>,
    streamable_sessions: Arc<StreamableSessionRegistry>,
    pub(crate) event_bus: Option<Arc<dyn EventBus>>,
    resource_notifier: Option<Arc<dyn ResourceNotifier>>,
    /// Server-injected slash-command dispatcher (Cluster 345). Set once at
    /// startup via [`Self::set_slash_dispatcher`]; `None` (never set) means MCP
    /// posts skip slash dispatch (the pre-345 behaviour, and how tests run).
    slash_dispatcher: std::sync::OnceLock<Arc<dyn crate::slash_dispatch::SlashDispatcher>>,
}

impl McpServer {
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        search: Arc<dyn Search>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        let (notification_tx, _) = broadcast::channel(NOTIFICATION_BROADCAST_CAPACITY);
        Self {
            store,
            artifacts,
            search,
            embedding_provider,
            server_name: "maidan".into(),
            server_version: env!("CARGO_PKG_VERSION").into(),
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
            pending_notifications: Arc::new(Mutex::new(Vec::new())),
            notification_tx,
            streamable_sessions: Arc::new(StreamableSessionRegistry::new()),
            event_bus: None,
            resource_notifier: None,
            slash_dispatcher: std::sync::OnceLock::new(),
        }
    }

    /// Attach the server-side slash-command dispatcher (Cluster 345). Called once
    /// at startup from the server binary (only `main.rs`), so the resulting
    /// `AppState`↔`McpServer` reference is a process-lifetime shared-state graph,
    /// never created in tests/embedders (which leave it unset → MCP posts skip
    /// slash dispatch). Works through `&self` (the server is already `Arc`-shared).
    pub fn set_slash_dispatcher(
        &self,
        dispatcher: Arc<dyn crate::slash_dispatch::SlashDispatcher>,
    ) {
        let _ = self.slash_dispatcher.set(dispatcher);
    }

    pub(crate) fn slash_dispatcher(
        &self,
    ) -> Option<&Arc<dyn crate::slash_dispatch::SlashDispatcher>> {
        self.slash_dispatcher.get()
    }

    /// When set, message mutations append to the event log and publish for an in-process indexer.
    pub fn with_event_bus(mut self, bus: Arc<dyn EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// When set, MCP resource-update notifications fan out to every server
    /// replica (Cluster 102). Call [`McpServer::spawn_resource_notify_listener`]
    /// after wrapping the server in an `Arc` so this process delivers
    /// cross-replica updates to its own SSE subscribers.
    pub fn with_resource_notifier(mut self, notifier: Arc<dyn ResourceNotifier>) -> Self {
        self.resource_notifier = Some(notifier);
        self
    }

    pub(crate) async fn publish_event(&self, event: Event) -> Option<i64> {
        let bus = self.event_bus.as_ref()?;
        let stored = self.store.append_event(&event).await.ok()?;
        let envelope = BusEnvelope {
            log_id: stored.id,
            event,
        };
        let _ = bus.publish(envelope).await;
        Some(stored.id)
    }

    /// Bus-notify an event that was **already durably appended** by a `*_with_event`
    /// store call — the MCP analogue of the REST `publish_stored`. Hydrates the
    /// `Event` from the stored payload; a missing bus (embedded use) is a no-op.
    /// Unlike [`Self::publish_event`], this does NOT append (no double-log).
    pub(crate) async fn publish_stored(&self, stored: &StoredEvent) {
        if let Some(bus) = self.event_bus.as_ref() {
            if let Ok(event) = serde_json::from_value::<Event>(stored.payload.clone()) {
                let _ = bus
                    .publish(BusEnvelope {
                        log_id: stored.id,
                        event,
                    })
                    .await;
            }
        }
    }

    pub fn streamable_sessions(&self) -> Arc<StreamableSessionRegistry> {
        self.streamable_sessions.clone()
    }

    /// Register a new streamable HTTP session id, or validate an existing open session.
    pub async fn touch_streamable_session(&self, existing: Option<&str>) -> String {
        let registry = self.streamable_sessions();
        if let Some(id) = existing.filter(|s| !s.is_empty()) {
            if registry.is_open(id).await {
                registry.touch(id).await;
                return id.to_string();
            }
        }
        uuid::Uuid::new_v4().to_string()
    }

    /// Live stream of MCP JSON-RPC notifications (HTTP SSE transport).
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }

    /// Issue a server→client JSON-RPC request over a client's streamable
    /// session (MCP `sampling/createMessage`, `roots/list`,
    /// `elicitation/create`) and await its response (Cluster 148). Requires the
    /// client to have declared the corresponding capability in `initialize`;
    /// the request rides the session's SSE stream and the client's response
    /// arrives as an inbound POST routed by [`Self::resolve_client_response`].
    pub async fn request_client(
        &self,
        session_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, McpError> {
        let capability = client_capability_for_method(method)
            .ok_or_else(|| McpError::InvalidParams(format!("{method} is not a server request")))?;
        let registry = &self.streamable_sessions;
        if !registry.client_supports(session_id, capability).await {
            return Err(McpError::Forbidden(format!(
                "client did not declare the {capability} capability"
            )));
        }
        let Some((request_id, rx)) = registry.register_client_request(session_id).await else {
            return Err(McpError::Internal("unknown or closed mcp session".into()));
        };
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        let data =
            serde_json::to_string(&request).map_err(|e| McpError::Internal(e.to_string()))?;
        // Server→client requests ride the spec-canonical `GET /mcp/streamable`
        // stream (Cluster 154), not the POST leg's response mpsc. No open GET
        // leg → fail fast rather than await a reply that can't arrive.
        if !registry.push_client_request(session_id, data).await {
            registry.cancel_client_request(session_id, request_id).await;
            return Err(McpError::Internal(
                "mcp session has no open GET stream to receive the request".into(),
            ));
        }
        match tokio::time::timeout(CLIENT_REQUEST_TIMEOUT, rx).await {
            Ok(Ok(response)) => {
                if let Some(err) = response.get("error") {
                    Err(McpError::Internal(format!("client returned error: {err}")))
                } else {
                    Ok(response.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            _ => {
                registry.cancel_client_request(session_id, request_id).await;
                Err(McpError::Internal(
                    "client did not respond to server request".into(),
                ))
            }
        }
    }

    /// Route an inbound JSON-RPC *response* (from the client on the streamable
    /// endpoint) to the awaiting [`Self::request_client`]. Returns whether a
    /// pending request matched.
    pub async fn resolve_client_response(&self, session_id: &str, response: Value) -> bool {
        let Some(request_id) = response.get("id").and_then(|v| v.as_i64()) else {
            return false;
        };
        self.streamable_sessions
            .resolve_client_response(session_id, request_id, response)
            .await
    }

    /// Invoke a tool by name (used by slash-command dispatch and tests).
    pub async fn call_tool(
        &self,
        auth: &AuthContext,
        name: &str,
        args: &Value,
    ) -> Result<Value, McpError> {
        let params = json!({ "name": name, "arguments": args });
        self.tools_call(&params, auth, None).await
    }

    pub async fn handle(&self, request: JsonRpcRequest, auth: &AuthContext) -> JsonRpcResponse {
        self.handle_in_session(request, auth, None).await
    }

    /// Like [`Self::handle`], but carries the streamable `Mcp-Session-Id` so a
    /// tool that issues a server→client request (e.g. the sampling-backed
    /// `summarize_thread`) can target the client on that session. Non-streamable
    /// transports pass `None`.
    pub async fn handle_in_session(
        &self,
        request: JsonRpcRequest,
        auth: &AuthContext,
        session_id: Option<&str>,
    ) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        match self.dispatch(&request, auth, session_id).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(err) => {
                tracing::debug!(method = %request.method, error = %err, "mcp dispatch error");
                JsonRpcResponse::failure(id, err.to_jsonrpc())
            }
        }
    }

    async fn dispatch(
        &self,
        request: &JsonRpcRequest,
        auth: &AuthContext,
        session_id: Option<&str>,
    ) -> Result<Value, McpError> {
        match request.method.as_str() {
            "initialize" => self.initialize(&request.params).await,
            // The client's post-initialize handshake notification is accepted
            // (and ignored) rather than treated as an unknown method.
            "notifications/initialized" | "notifications/cancelled" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::catalog_for(auth) })),
            "tools/call" => self.tools_call(&request.params, auth, session_id).await,
            "resources/list" => Ok(json!({ "resources": resources::catalog() })),
            "resources/read" => self.resources_read(&request.params, auth).await,
            "resources/subscribe" => self.resources_subscribe(&request.params, auth).await,
            "resources/unsubscribe" => self.resources_unsubscribe(&request.params, auth).await,
            "prompts/list" => Ok(json!({ "prompts": prompts::catalog() })),
            "prompts/get" => self.prompts_get(&request.params, auth).await,
            other => Err(McpError::MethodNotFound(other.into())),
        }
    }

    async fn initialize(&self, params: &Value) -> Result<Value, McpError> {
        let requested = params.get("protocolVersion").and_then(|v| v.as_str());
        let protocol_version = negotiate_protocol_version(requested);
        Ok(json!({
            "protocolVersion": protocol_version,
            "capabilities": {
                "tools": {},
                "resources": { "subscribe": true },
                "prompts": {}
            },
            "serverInfo": {
                "name": self.server_name,
                "version": self.server_version
            },
            // Cluster 336: the spec `instructions` field — an agent's cold-start guide.
            // Call `whoami` first for your member_id, then the six-tool hero loop.
            "instructions": "Maidan is a shared room for AI agents. Call `whoami` first to get your \
                member_id, workspace_id, and capabilities. Hero loop for task work: \
                `claim_next_thread` (take the next ready task in a channel) → `get_thread_context` \
                (read it, grounded in the workspace glossary) → do the work → `set_thread_result` \
                (record the outcome) → `wait_for_ready`/`wait_for_result` (coordinate with other \
                agents). Tools are capability-filtered to your token, so `tools/list` shows only \
                what you can call."
        }))
    }

    async fn tools_call(
        &self,
        params: &Value,
        auth: &AuthContext,
        session_id: Option<&str>,
    ) -> Result<Value, McpError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing tool name".into()))?;
        if !auth.bypass {
            let cap = tools::required_capability(name)?;
            auth.require_capability(cap).map_err(McpError::from)?;
        }
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let result = tools::dispatch(self, auth, name, &args, session_id).await?;
        self.queue_resource_updates(name, &args, &result).await;
        Ok(result)
    }

    async fn prompts_get(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing prompt name".into()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        prompts::get(&self.store, name, &args).await
    }

    async fn resources_read(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        // Gate channel/thread resource content on per-channel access (Cluster
        // 161); workspace/artifact resources remain workspace-scoped.
        if !auth.bypass {
            if let Some(rest) = uri.strip_prefix("maidan://") {
                let mut parts = rest.splitn(2, '/');
                match (parts.next(), parts.next()) {
                    (Some("threads"), Some(id)) => {
                        if let Ok(u) = id.parse() {
                            maidan_auth::ensure_thread_access(
                                self.store.as_ref(),
                                auth,
                                maidan_types::ThreadId(u),
                            )
                            .await?;
                        }
                    }
                    (Some("channels"), Some(id)) => {
                        if let Ok(u) = id.parse() {
                            maidan_auth::ensure_channel_access(
                                self.store.as_ref(),
                                auth,
                                maidan_types::ChannelId(u),
                            )
                            .await?;
                        }
                    }
                    // Cluster 204/332: artifacts are deduped across tenants, so gate
                    // the read on the caller's per-workspace access ref. A missing ref
                    // is NotFound (no cross-tenant existence oracle), like REST.
                    (Some("artifacts"), Some(sha)) => {
                        if !self
                            .store
                            .artifact_ref_exists(auth.workspace_id, sha)
                            .await?
                        {
                            return Err(McpError::NotFound);
                        }
                    }
                    _ => {}
                }
            }
        }
        resources::read(&self.store, uri).await
    }

    async fn resources_subscribe(
        &self,
        params: &Value,
        auth: &AuthContext,
    ) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::validate_uri(uri)?;
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.insert(uri.to_string());
        Ok(json!({
            "ok": true,
            "uri": uri
        }))
    }

    async fn resources_unsubscribe(
        &self,
        params: &Value,
        auth: &AuthContext,
    ) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::validate_uri(uri)?;
        let mut subscriptions = self.subscriptions.lock().await;
        let removed = subscriptions.remove(uri);
        Ok(json!({
            "ok": true,
            "uri": uri,
            "removed": removed
        }))
    }

    async fn queue_resource_updates(&self, tool_name: &str, args: &Value, result: &Value) {
        let uris = crate::resource_updates::uris_for_tool_mutation(
            self.store.as_ref(),
            tool_name,
            args,
            result,
        )
        .await;
        self.publish_resource_uris(uris).await;
    }

    /// Fan-out `notifications/resources/updated` for HTTP and tool mutations.
    ///
    /// Two delivery surfaces:
    /// 1. **Inline** — matching URIs are queued for the current request's caller
    ///    ([`McpServer::take_pending_notifications`]); local and synchronous.
    /// 2. **Live SSE** — when a [`ResourceNotifier`] is wired (Cluster 102), the
    ///    *unfiltered* URI set is published cluster-wide and each replica's
    ///    listener ([`McpServer::spawn_resource_notify_listener`]) delivers to
    ///    its own SSE subscribers. Without a notifier, SSE delivery happens
    ///    locally (legacy single-process path).
    pub async fn publish_resource_uris(&self, uris: Vec<String>) {
        if uris.is_empty() {
            return;
        }
        self.queue_pending_for_subscriptions(&uris).await;
        match &self.resource_notifier {
            Some(notifier) => {
                let _ = notifier.publish_uris(uris).await;
            }
            None => {
                self.broadcast_to_subscribed_sse(&uris).await;
            }
        }
    }

    /// Queue matching URIs for the current request's caller (inline response).
    async fn queue_pending_for_subscriptions(&self, uris: &[String]) {
        let subscriptions = self.subscriptions.lock().await;
        let matching: Vec<&String> = uris.iter().filter(|u| subscriptions.contains(*u)).collect();
        if matching.is_empty() {
            return;
        }
        let mut pending = self.pending_notifications.lock().await;
        for uri in matching {
            pending.push(JsonRpcNotification::new(
                NOTIFY_RESOURCE_UPDATED,
                json!({ "uri": uri }),
            ));
        }
    }

    /// Deliver matching URIs to this process's live SSE subscribers
    /// ([`McpServer::subscribe_notifications`]). Used directly in the
    /// no-notifier path and by the cross-replica listener loop.
    pub(crate) async fn broadcast_to_subscribed_sse(&self, uris: &[String]) {
        let subscriptions = self.subscriptions.lock().await;
        let matching: Vec<&String> = uris.iter().filter(|u| subscriptions.contains(*u)).collect();
        if matching.is_empty() {
            return;
        }
        for uri in matching {
            let _ = self.notification_tx.send(JsonRpcNotification::new(
                NOTIFY_RESOURCE_UPDATED,
                json!({ "uri": uri }),
            ));
        }
    }

    /// Spawn the loop delivering cross-replica resource URIs to this process's
    /// SSE subscribers. No-op when no [`ResourceNotifier`] is wired. Call once
    /// at startup after wrapping the server in an `Arc`.
    pub fn spawn_resource_notify_listener(self: &Arc<Self>) {
        let Some(notifier) = self.resource_notifier.clone() else {
            return;
        };
        let mut rx = notifier.subscribe();
        let server = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(uris) => server.broadcast_to_subscribed_sse(&uris).await,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "mcp resource-notify listener lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub async fn take_pending_notifications(&self) -> Vec<JsonRpcNotification> {
        let mut pending = self.pending_notifications.lock().await;
        std::mem::take(&mut *pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::AuthContext;
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mk_server() -> (McpServer, ThreadId, MemberId) {
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

        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let workspace = store
            .create_workspace(NewWorkspace {
                name: "mcp-subscribe".into(),
            })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: workspace.id,
                handle: "alice".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: workspace.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        (server, thread.id, member.id)
    }

    fn request(id: i64, method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn protocol_version_negotiation_follows_the_spec_rule() {
        // Supported requested version is echoed; unsupported/absent fall back to preferred.
        assert_eq!(negotiate_protocol_version(Some("2024-11-05")), "2024-11-05");
        // 2026-07-28 is negotiable: a current client that requests it gets it (J3.1).
        assert_eq!(negotiate_protocol_version(Some("2026-07-28")), "2026-07-28");
        assert_eq!(
            negotiate_protocol_version(Some("1999-01-01")),
            preferred_protocol_version()
        );
        assert_eq!(
            negotiate_protocol_version(None),
            preferred_protocol_version()
        );
        assert!(is_supported_protocol_version("2024-11-05"));
        assert!(is_supported_protocol_version("2026-07-28"));
        assert!(!is_supported_protocol_version("1999-01-01"));
        // The current default is 2026-07-28 (the full revision landed, J3.5); an
        // older client that explicitly requests 2024-11-05 still gets it.
        assert_eq!(preferred_protocol_version(), "2026-07-28");
    }

    #[tokio::test]
    async fn request_client_pushes_the_request_and_awaits_the_clients_response() {
        let (server, _thread, _member) = mk_server().await;
        let registry = server.streamable_sessions();
        let _mpsc_rx = registry.open("sess".to_string()).await;
        registry
            .set_client_capabilities("sess", json!({"sampling": {}}))
            .await;
        // The client listens on the canonical GET stream for server→client requests.
        let mut client_rx = registry.subscribe_client_requests("sess").await.unwrap();

        // Issue the server→client request from one task…
        let server_bg = server.clone();
        let call = tokio::spawn(async move {
            server_bg
                .request_client("sess", "sampling/createMessage", json!({"prompt": "hi"}))
                .await
        });

        // …the client reads it off the GET stream and replies by id.
        let data = client_rx.recv().await.unwrap();
        let pushed: Value = serde_json::from_str(&data).unwrap();
        assert_eq!(pushed["method"], "sampling/createMessage");
        let request_id = pushed["id"].as_i64().unwrap();
        let response = json!({"jsonrpc": "2.0", "id": request_id, "result": {"text": "ok"}});
        assert!(server.resolve_client_response("sess", response).await);

        let result = call.await.unwrap().unwrap();
        assert_eq!(result, json!({"text": "ok"}));
    }

    #[tokio::test]
    async fn request_client_is_forbidden_without_the_declared_capability() {
        let (server, _thread, _member) = mk_server().await;
        let _rx = server.streamable_sessions().open("sess".to_string()).await;
        // No capabilities declared → the server may not initiate sampling.
        let err = server
            .request_client("sess", "sampling/createMessage", json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::Forbidden(_)));
        // An unknown method is a param error, not a capability check.
        let err = server
            .request_client("sess", "tools/list", json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    #[tokio::test]
    async fn mcp_denies_non_members_in_private_channels() {
        use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ};
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace {
                name: "rbac".into(),
            })
            .await
            .unwrap();
        let mk = |handle: &'static str| {
            let store = store.clone();
            async move {
                store
                    .create_member(NewMember {
                        workspace_id: ws.id,
                        handle: handle.into(),
                        display_name: None,
                        kind: MemberKind::Human,
                    })
                    .await
                    .unwrap()
            }
        };
        let alice = mk("alice").await;
        let bob = mk("bob").await;
        let ch = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "secret".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        store
            .add_channel_member(ch.id, alice.id, ChannelMemberRole::Admin)
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: ch.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );

        let caps = vec![WORKSPACE_READ.to_string(), MESSAGE_POST.to_string()];
        let bob_auth = AuthContext::from_session(bob.id, ws.id, caps.clone());
        let alice_auth = AuthContext::from_session(alice.id, ws.id, caps);
        let args = json!({ "thread_id": thread.id.0 });

        // Non-member Bob is denied; member Alice is allowed.
        let err = server
            .call_tool(&bob_auth, "list_messages", &args)
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::Forbidden(_)));
        server
            .call_tool(&alice_auth, "list_messages", &args)
            .await
            .unwrap();

        // Aggregate reads (Cluster 162): the private channel is filtered out of
        // list_channels + get_workspace_context for Bob, present for Alice.
        let unwrap_content = |v: Value| -> Value {
            let text = v["content"][0]["text"].as_str().unwrap().to_string();
            serde_json::from_str(&text).unwrap()
        };
        let ws_args = json!({ "workspace_id": ws.id.0 });

        let bob_channels = unwrap_content(
            server
                .call_tool(&bob_auth, "list_channels", &ws_args)
                .await
                .unwrap(),
        );
        assert!(
            !bob_channels
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == json!(ch.id.0)),
            "bob must not see the private channel"
        );
        let alice_channels = unwrap_content(
            server
                .call_tool(&alice_auth, "list_channels", &ws_args)
                .await
                .unwrap(),
        );
        assert!(
            alice_channels
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == json!(ch.id.0)),
            "alice sees the private channel"
        );

        let bob_ctx = unwrap_content(
            server
                .call_tool(&bob_auth, "get_workspace_context", &ws_args)
                .await
                .unwrap(),
        );
        assert!(
            bob_ctx["threads"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["channel_id"] != json!(ch.id.0)),
            "bob's workspace context excludes the private channel's threads"
        );
    }

    #[tokio::test]
    async fn mcp_assignment_read_side_claims_lists_and_filters() {
        use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "aq".into() })
            .await
            .unwrap();
        let agent = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let public = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "open".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let secret = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "secret".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        let mk_thread = |channel_id, store: Arc<dyn Store>| async move {
            store
                .create_thread(NewThread {
                    channel_id,
                    parent_thread_id: None,
                    title: Some("t".into()),
                })
                .await
                .unwrap()
        };
        let pub_thread = mk_thread(public.id, store.clone()).await;
        let secret_thread = mk_thread(secret.id, store.clone()).await;
        // The agent is assigned a thread in a private channel it is NOT a member
        // of (assignment is orthogonal to membership).
        store
            .assign_thread(secret_thread.id, agent.id)
            .await
            .unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::from_session(
            agent.id,
            ws.id,
            vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
        );
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // claim_next takes the oldest unassigned thread in the public channel.
        let claimed = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "claim_next_thread",
                    &json!({ "channel_id": public.id.0, "member_id": agent.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(claimed["id"], json!(pub_thread.id.0));

        // list_assigned shows the public-channel assignment, but the private one
        // is filtered out — the agent isn't a member of that channel.
        let queue = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "list_assigned_threads",
                    &json!({ "member_id": agent.id.0 }),
                )
                .await
                .unwrap(),
        );
        let ids: Vec<Value> = queue
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].clone())
            .collect();
        assert!(
            ids.contains(&json!(pub_thread.id.0)),
            "public assignment visible"
        );
        assert!(
            !ids.contains(&json!(secret_thread.id.0)),
            "private-channel assignment filtered from a non-member caller"
        );

        // No unassigned work left in the public channel → null.
        let none = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "claim_next_thread",
                    &json!({ "channel_id": public.id.0, "member_id": agent.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert!(none.is_null());
    }

    #[tokio::test]
    async fn wait_for_mention_returns_next_mention_and_filters_private() {
        use chrono::Utc;
        use maidan_auth::capability::WORKSPACE_READ;
        use maidan_bus::InMemoryBus;
        use std::time::Duration;

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "wm".into() })
            .await
            .unwrap();
        let agent = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let public = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "open".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let secret = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "secret".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        let mk_thread = |channel_id, store: Arc<dyn Store>| async move {
            store
                .create_thread(NewThread {
                    channel_id,
                    parent_thread_id: None,
                    title: Some("t".into()),
                })
                .await
                .unwrap()
        };
        let pub_thread = mk_thread(public.id, store.clone()).await;
        let secret_thread = mk_thread(secret.id, store.clone()).await;

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
        .with_event_bus(Arc::new(InMemoryBus::new()));
        // Not a member of the private channel — public is workspace-open.
        let auth = AuthContext::from_session(agent.id, ws.id, vec![WORKSPACE_READ.to_string()]);
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        let mention = |thread_id| Event::MentionRecorded {
            occurred_at: Utc::now(),
            workspace_id: ws.id,
            thread_id,
            message_id: MessageId(uuid::Uuid::new_v4()),
            member_id: agent.id,
        };

        // A mention in the public thread wakes the waiter. `join!` polls the
        // waiter first (it subscribes, then parks on the stream), then the
        // delayed publisher fires — no cross-task subscribe race.
        let live_args = json!({ "member_id": agent.id.0, "timeout_ms": 5000 });
        let (out, _) = tokio::join!(
            server.call_tool(&auth, "wait_for_mention", &live_args),
            async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                server.publish_event(mention(pub_thread.id)).await;
            }
        );
        let got = unwrap_content(out.unwrap());
        assert_eq!(got["kind"], json!("mention_recorded"));
        assert_eq!(got["member_id"], json!(agent.id.0));
        assert_eq!(got["thread_id"], json!(pub_thread.id.0));

        // A mention in a private channel the agent can't access is skipped, so
        // the waiter times out to null despite a matching event being published.
        let filtered_args = json!({ "member_id": agent.id.0, "timeout_ms": 400 });
        let (out, _) = tokio::join!(
            server.call_tool(&auth, "wait_for_mention", &filtered_args),
            async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                server.publish_event(mention(secret_thread.id)).await;
            }
        );
        assert!(
            unwrap_content(out.unwrap()).is_null(),
            "a mention in an inaccessible private channel is filtered → timeout"
        );
    }

    #[tokio::test]
    async fn wait_for_ready_returns_next_ready_and_filters_private() {
        use chrono::Utc;
        use maidan_auth::capability::WORKSPACE_READ;
        use maidan_bus::InMemoryBus;
        use std::time::Duration;

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "wr".into() })
            .await
            .unwrap();
        let agent = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let public = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "open".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let secret = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "secret".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        let mk_thread = |channel_id, store: Arc<dyn Store>| async move {
            store
                .create_thread(NewThread {
                    channel_id,
                    parent_thread_id: None,
                    title: Some("t".into()),
                })
                .await
                .unwrap()
        };
        let pub_thread = mk_thread(public.id, store.clone()).await;
        let secret_thread = mk_thread(secret.id, store.clone()).await;

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
        .with_event_bus(Arc::new(InMemoryBus::new()));
        // Not a member of the private channel — public is workspace-open.
        let auth = AuthContext::from_session(agent.id, ws.id, vec![WORKSPACE_READ.to_string()]);
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        let ready = |thread: Thread| Event::ThreadReady {
            occurred_at: Utc::now(),
            workspace_id: ws.id,
            channel_id: thread.channel_id,
            thread_id: thread.id,
            thread,
        };

        // A ready task in the public thread wakes the workspace-wide waiter.
        let live_args = json!({ "timeout_ms": 5000 });
        let (out, _) = tokio::join!(
            server.call_tool(&auth, "wait_for_ready", &live_args),
            async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                server.publish_event(ready(pub_thread.clone())).await;
            }
        );
        let got = unwrap_content(out.unwrap());
        assert_eq!(got["kind"], json!("thread_ready"));
        assert_eq!(got["thread_id"], json!(pub_thread.id.0));

        // A ready task in a private channel the agent can't access is skipped, so
        // the waiter times out to null despite a matching event being published.
        let filtered_args = json!({ "timeout_ms": 400 });
        let (out, _) = tokio::join!(
            server.call_tool(&auth, "wait_for_ready", &filtered_args),
            async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                server.publish_event(ready(secret_thread.clone())).await;
            }
        );
        assert!(
            unwrap_content(out.unwrap()).is_null(),
            "a ready task in an inaccessible private channel is filtered → timeout"
        );
    }

    #[tokio::test]
    async fn result_tools_set_get_wait_and_aggregate() {
        use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
        use maidan_bus::InMemoryBus;
        use std::time::Duration;

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "res".into() })
            .await
            .unwrap();
        let agent = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "tasks".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let mk = |title: &str| NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some(title.into()),
        };
        // parent depends on dep1 + dep2.
        let parent = store.create_thread(mk("parent")).await.unwrap();
        let dep1 = store.create_thread(mk("dep1")).await.unwrap();
        let dep2 = store.create_thread(mk("dep2")).await.unwrap();
        store
            .add_thread_dependency(parent.id, dep1.id)
            .await
            .unwrap();
        store
            .add_thread_dependency(parent.id, dep2.id)
            .await
            .unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
        .with_event_bus(Arc::new(InMemoryBus::new()));
        // A real session member so `produced_by` (a NOT-NULL FK) resolves — the
        // bypass nil member would FK-fail set_thread_result.
        let auth = AuthContext::from_session(
            agent.id,
            ws.id,
            vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
        );
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // No result yet → null.
        let miss = server
            .call_tool(
                &auth,
                "get_thread_result",
                &json!({ "thread_id": dep1.id.0 }),
            )
            .await
            .unwrap();
        assert!(unwrap_content(miss).is_null());

        // Set dep1's result, then read it back.
        let payload = json!({ "answer": 42, "ok": true });
        let set = server
            .call_tool(
                &auth,
                "set_thread_result",
                &json!({ "thread_id": dep1.id.0, "result": payload }),
            )
            .await
            .unwrap();
        let set = unwrap_content(set);
        assert_eq!(set["result"], payload);
        assert_eq!(set["produced_by"], json!(agent.id.0));
        let got = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "get_thread_result",
                    &json!({ "thread_id": dep1.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(got["result"], payload);

        // wait_for_result on dep2 wakes when its result is produced after subscribe.
        let dep2_payload = json!({ "answer": 7 });
        let wait_args = json!({ "thread_id": dep2.id.0, "timeout_ms": 5000 });
        let set_args = json!({ "thread_id": dep2.id.0, "result": dep2_payload });
        let (out, _) = tokio::join!(
            server.call_tool(&auth, "wait_for_result", &wait_args),
            async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                server
                    .call_tool(&auth, "set_thread_result", &set_args)
                    .await
                    .unwrap();
            }
        );
        let woke = unwrap_content(out.unwrap());
        assert_eq!(
            woke["result"], dep2_payload,
            "wait_for_result returns the payload"
        );

        // The parent aggregates both dependencies' results.
        let agg = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "get_dependency_results",
                    &json!({ "thread_id": parent.id.0 }),
                )
                .await
                .unwrap(),
        );
        let deps = agg["dependencies"].as_array().unwrap();
        assert_eq!(deps.len(), 2);
        let by_id = |tid: uuid::Uuid| {
            deps.iter()
                .find(|d| d["thread_id"] == json!(tid))
                .expect("dependency present")["result"]
                .clone()
        };
        assert_eq!(by_id(dep1.id.0), payload);
        assert_eq!(by_id(dep2.id.0), dep2_payload);
    }

    #[tokio::test]
    async fn get_queue_depth_tool_reports_counts() {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "qd".into() })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "queue".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let mk = |title: &str| NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some(title.into()),
        };
        // t1 has no deps → ready; t2 depends on t1 → blocked.
        let t1 = store.create_thread(mk("t1")).await.unwrap();
        let t2 = store.create_thread(mk("t2")).await.unwrap();
        store.add_thread_dependency(t2.id, t1.id).await.unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::bypass();
        let out = server
            .call_tool(
                &auth,
                "get_queue_depth",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap();
        let depth: Value =
            serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(depth["open"], json!(2));
        assert_eq!(depth["ready"], json!(1), "t1 has no deps");
        assert_eq!(depth["blocked"], json!(1), "t2 waits on t1");
        assert_eq!(depth["assigned"], json!(0));
    }

    #[tokio::test]
    async fn notification_tools_list_count_mark_and_wait() {
        use chrono::Utc;
        use maidan_bus::InMemoryBus;
        use std::time::Duration;

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "n".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "me".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        // Seed two notifications for the member.
        let mut ids = Vec::new();
        for log_id in 1..=2 {
            let n = store
                .create_notification(NewNotification {
                    workspace_id: ws.id,
                    member_id: member.id,
                    kind: EventKind::MentionRecorded,
                    source_log_id: log_id,
                    channel_id: Some(channel.id),
                    thread_id: Some(thread.id),
                    message_id: None,
                    actor_id: None,
                })
                .await
                .unwrap();
            ids.push(n.id);
        }

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
        .with_event_bus(Arc::new(InMemoryBus::new()));
        let auth = AuthContext::bypass();
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        let mid = json!({ "member_id": member.id.0 });

        // Count + list see both.
        let count = unwrap_content(
            server
                .call_tool(&auth, "get_unread_count", &mid)
                .await
                .unwrap(),
        );
        assert_eq!(count["count"], json!(2));
        let list = unwrap_content(
            server
                .call_tool(&auth, "list_notifications", &mid)
                .await
                .unwrap(),
        );
        assert_eq!(list.as_array().unwrap().len(), 2);

        // Mark one read → count drops, unread_only filters.
        let marked = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "mark_notification_read",
                    &json!({ "member_id": member.id.0, "notification_id": ids[0].0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(marked["marked"], json!(true));
        let count = unwrap_content(
            server
                .call_tool(&auth, "get_unread_count", &mid)
                .await
                .unwrap(),
        );
        assert_eq!(count["count"], json!(1));
        let unread = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "list_notifications",
                    &json!({ "member_id": member.id.0, "unread_only": true }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(unread.as_array().unwrap().len(), 1);

        // wait_for_notification wakes on the member's next mention event.
        let wait_args = json!({ "member_id": member.id.0, "timeout_ms": 5000 });
        let mention = Event::MentionRecorded {
            occurred_at: Utc::now(),
            workspace_id: ws.id,
            thread_id: thread.id,
            message_id: MessageId::new(),
            member_id: member.id,
        };
        let (out, _) = tokio::join!(
            server.call_tool(&auth, "wait_for_notification", &wait_args),
            async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                server.publish_event(mention).await;
            }
        );
        let woke = unwrap_content(out.unwrap());
        assert_eq!(woke["kind"], json!("mention_recorded"));
        assert_eq!(woke["member_id"], json!(member.id.0));
    }

    #[tokio::test]
    async fn notification_pref_tools_set_and_list() {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "p".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "m".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::bypass();
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // Set a mute.
        let pref = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "set_notification_pref",
                    &json!({ "member_id": member.id.0, "kind": "mention_recorded", "muted": true }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(pref["kind"], json!("mention_recorded"));
        assert_eq!(pref["muted"], json!(true));

        // List reflects it.
        let list = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "list_notification_prefs",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["muted"], json!(true));

        // An unknown kind is rejected.
        assert!(server
            .call_tool(
                &auth,
                "set_notification_pref",
                &json!({ "member_id": member.id.0, "kind": "not_a_kind", "muted": true }),
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn delivery_mode_tools_get_and_set() {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "d".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "m".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::bypass();
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // Default is immediate.
        let got = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "get_delivery_mode",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(got["mode"], json!("immediate"));

        // Switch to digest.
        let set = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "set_delivery_mode",
                    &json!({ "member_id": member.id.0, "mode": "digest" }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set["mode"], json!("digest"));

        // Get reflects it.
        let got = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "get_delivery_mode",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(got["mode"], json!("digest"));

        // An unknown mode is rejected.
        assert!(server
            .call_tool(
                &auth,
                "set_delivery_mode",
                &json!({ "member_id": member.id.0, "mode": "carrier-pigeon" }),
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn email_tools_set_get_delete() {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "e".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "m".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::bypass();
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // Unset → null.
        let got = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "get_member_email",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert!(got.is_null());

        // Set, then get returns it.
        let set = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "set_member_email",
                    &json!({ "member_id": member.id.0, "email": "m@example.com" }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set["email"], json!("m@example.com"));
        let got = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "get_member_email",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(got["email"], json!("m@example.com"));

        // A garbage address is rejected.
        assert!(server
            .call_tool(
                &auth,
                "set_member_email",
                &json!({ "member_id": member.id.0, "email": "nope" }),
            )
            .await
            .is_err());

        // Delete → {deleted:true}, then get is null again.
        let del = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "delete_member_email",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(del["deleted"], json!(true));
        let got = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "get_member_email",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert!(got.is_null());
    }

    #[tokio::test]
    async fn follow_tools_channel_and_thread() {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "f".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "m".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "c".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::bypass();
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        let args = json!({ "member_id": member.id.0, "channel_id": channel.id.0 });

        // Follow → list shows it → unfollow removes it.
        let followed = unwrap_content(
            server
                .call_tool(&auth, "follow_channel", &args)
                .await
                .unwrap(),
        );
        assert_eq!(followed["following"], json!(true));
        let list = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "list_channel_follows",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(list.as_array().unwrap().len(), 1);
        let removed = unwrap_content(
            server
                .call_tool(&auth, "unfollow_channel", &args)
                .await
                .unwrap(),
        );
        assert_eq!(removed["removed"], json!(true));
        let empty = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "list_channel_follows",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert!(empty.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn projector_link_tools_slack_and_github() {
        use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "p".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "m".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "c".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        // A real workspace-scoped session: `list_*` reads `auth.workspace_id`, and
        // link resolves the thread's own workspace/channel (public → member has access).
        let auth = AuthContext::from_session(
            member.id,
            ws.id,
            vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
        );
        let unwrap_content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // Slack: link resolves ws/channel from the thread → list → unlink.
        let link = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "link_slack_channel",
                    &json!({ "thread_id": thread.id.0, "slack_channel_id": "C123", "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(link["slack_channel_id"], json!("C123"));
        assert_eq!(link["channel_id"], json!(channel.id.0));
        let list = unwrap_content(
            server
                .call_tool(&auth, "list_slack_channel_links", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(list.as_array().unwrap().len(), 1);
        let un = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "unlink_slack_channel",
                    &json!({ "slack_channel_id": "C123" }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(un["unlinked"], json!(true));
        assert!(unwrap_content(
            server
                .call_tool(&auth, "list_slack_channel_links", &json!({}))
                .await
                .unwrap()
        )
        .as_array()
        .unwrap()
        .is_empty());

        // GitHub: same round-trip.
        let gh = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "link_github_issue",
                    &json!({ "thread_id": thread.id.0, "repo": "o/r", "issue_number": 7, "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(gh["repo"], json!("o/r"));
        assert_eq!(gh["issue_number"], json!(7));
        let ghlist = unwrap_content(
            server
                .call_tool(&auth, "list_github_issue_links", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(ghlist.as_array().unwrap().len(), 1);
        let ghun = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "unlink_github_issue",
                    &json!({ "repo": "o/r", "issue_number": 7 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(ghun["unlinked"], json!(true));
    }

    #[tokio::test]
    async fn task_schedule_tools_create_and_list() {
        use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "sch".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "q".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        // A real member session so `created_by` satisfies its FK.
        let auth = AuthContext::from_session(
            member.id,
            ws.id,
            vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
        );
        let content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        let created = content(
            server
                .call_tool(
                    &auth,
                    "create_task_schedule",
                    &json!({ "channel_id": channel.id.0, "title": "nightly", "interval_secs": 3600 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(created["title"], json!("nightly"));
        assert_eq!(created["interval_secs"], json!(3600));
        assert_eq!(created["active"], json!(true));

        let list = content(
            server
                .call_tool(&auth, "list_task_schedules", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["id"], created["id"]);
    }

    #[tokio::test]
    async fn mcp_edit_message_appends_messageedited_event() {
        use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "e".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "g".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        let msg = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "v1".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::from_session(
            member.id,
            ws.id,
            vec![MESSAGE_POST.to_string(), WORKSPACE_READ.to_string()],
        );

        server
            .call_tool(
                &auth,
                "edit_message",
                &json!({ "message_id": msg.id.0, "editor_id": member.id.0, "body": "v2" }),
            )
            .await
            .unwrap();

        // The edit now appends a MessageEdited event — so as-of replay sees "v2"
        // and the indexer reindexes (previously the MCP edit was event-less).
        let events = store
            .list_thread_events_through(thread.id, i64::MAX)
            .await
            .unwrap();
        assert!(
            events.iter().any(|e| e.kind == EventKind::MessageEdited),
            "MCP edit_message must append a MessageEdited event, got {:?}",
            events.iter().map(|e| e.kind).collect::<Vec<_>>()
        );
        // And the reconstructed as-of message shows the edited body.
        let msgs = maidan_types::reconstruct_messages_through(&events);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "v2");
    }

    #[tokio::test]
    async fn mcp_write_tools_emit_domain_events() {
        use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE};
        use maidan_bus::InMemoryBus;

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "w".into() })
            .await
            .unwrap();
        let alice = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "alice".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let bob = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "bob".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "g".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        let msg = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: alice.id,
                body: "seed".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();

        // A bus so publish_event-based events (MessagePosted / MentionRecorded) log.
        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
        .with_event_bus(Arc::new(InMemoryBus::new()));
        let auth = AuthContext::from_session(
            alice.id,
            ws.id,
            vec![
                MESSAGE_POST.to_string(),
                WORKSPACE_WRITE.to_string(),
                WORKSPACE_READ.to_string(),
            ],
        );

        server
            .call_tool(
                &auth,
                "cast_vote",
                &json!({ "message_id": msg.id.0, "member_id": alice.id.0, "kind": "up" }),
            )
            .await
            .unwrap();
        server
            .call_tool(
                &auth,
                "add_reference",
                &json!({
                    "src_kind": "message", "src_id": msg.id.0,
                    "dst_kind": "thread", "dst_id": thread.id.0, "relation": "grounds"
                }),
            )
            .await
            .unwrap();
        server
            .call_tool(
                &auth,
                "post_message",
                &json!({ "thread_id": thread.id.0, "author_id": alice.id.0, "body": "ping @bob" }),
            )
            .await
            .unwrap();

        // Workspace-scoped (ReferenceAdded isn't thread-scoped, so a thread query
        // would miss it) — captures every event these tools appended.
        let events = store.list_events_after(ws.id, 0, 1000).await.unwrap();
        let kinds: Vec<EventKind> = events.iter().map(|e| e.kind).collect();
        // The social tools now append their domain events (were event-less). (VoteCast
        // is the representative message-scoped `*_with_event`; add_reference uses the
        // same path but ReferenceAdded isn't workspace-hoisted, so it's not asserted here.)
        assert!(kinds.contains(&EventKind::VoteCast), "VoteCast: {kinds:?}");
        // …and an MCP post with an @mention now publishes MentionRecorded for the mentioned member.
        assert!(
            kinds.contains(&EventKind::MentionRecorded),
            "MentionRecorded: {kinds:?}"
        );
        let mentions = store.list_mentions_for_member(bob.id, 10).await.unwrap();
        assert_eq!(mentions.len(), 1, "bob was @mentioned");
    }

    #[tokio::test]
    async fn mcp_post_runs_registered_slash_command_via_dispatcher() {
        use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE};
        use maidan_bus::InMemoryBus;

        // Stands in for the server-side slash runtime (Cluster 345): returns the
        // slash metadata the real `ServerSlashDispatcher` would produce.
        struct MockSlash;
        #[async_trait::async_trait]
        impl crate::slash_dispatch::SlashDispatcher for MockSlash {
            async fn dispatch(
                &self,
                _auth: &maidan_auth::AuthContext,
                parsed: &maidan_router::ParsedSlashCommand,
                _ws: WorkspaceId,
                _ch: ChannelId,
                _th: ThreadId,
                _author: MemberId,
                _msg: MessageId,
            ) -> serde_json::Value {
                json!({
                    "slash_command": { "name": parsed.name, "args": parsed.args },
                    "slash_response": { "ok": true, "handled": parsed.name }
                })
            }
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "w".into() })
            .await
            .unwrap();
        let alice = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "alice".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "g".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        store
            .create_slash_command(NewSlashCommand {
                workspace_id: ws.id,
                name: "echo".into(),
                description: None,
                handler_kind: SlashHandlerKind::Http,
                handler_target: "https://example.test/hook".into(),
                secret_ciphertext: String::new(),
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
        .with_event_bus(Arc::new(InMemoryBus::new()));
        server.set_slash_dispatcher(Arc::new(MockSlash));
        let auth = AuthContext::from_session(
            alice.id,
            ws.id,
            vec![
                MESSAGE_POST.to_string(),
                WORKSPACE_WRITE.to_string(),
                WORKSPACE_READ.to_string(),
            ],
        );

        // A `/echo` post runs the registered command → the message carries the
        // merged slash metadata (the parity with REST).
        server
            .call_tool(
                &auth,
                "post_message",
                &json!({ "thread_id": thread.id.0, "author_id": alice.id.0, "body": "/echo hi there" }),
            )
            .await
            .unwrap();
        let msgs = store.list_messages(thread.id, 10).await.unwrap();
        let slashed = msgs.iter().find(|m| m.body == "/echo hi there").unwrap();
        assert_eq!(slashed.metadata["slash_command"]["name"], "echo");
        assert_eq!(slashed.metadata["slash_command"]["args"], "hi there");
        assert_eq!(slashed.metadata["slash_response"]["ok"], true);

        // A plain post is untouched (no slash metadata).
        server
            .call_tool(
                &auth,
                "post_message",
                &json!({ "thread_id": thread.id.0, "author_id": alice.id.0, "body": "plain message" }),
            )
            .await
            .unwrap();
        let msgs = store.list_messages(thread.id, 10).await.unwrap();
        let plain = msgs.iter().find(|m| m.body == "plain message").unwrap();
        assert!(plain.metadata.get("slash_command").is_none());

        // An unregistered slash name is a literal post (no dispatch).
        server
            .call_tool(
                &auth,
                "post_message",
                &json!({ "thread_id": thread.id.0, "author_id": alice.id.0, "body": "/unknown x" }),
            )
            .await
            .unwrap();
        let msgs = store.list_messages(thread.id, 10).await.unwrap();
        let unknown = msgs.iter().find(|m| m.body == "/unknown x").unwrap();
        assert!(unknown.metadata.get("slash_command").is_none());
    }

    #[tokio::test]
    async fn whoami_returns_identity_and_initialize_carries_instructions() {
        use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "w".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::from_token(
            maidan_types::ApiTokenId(uuid::Uuid::new_v4()),
            member.id,
            ws.id,
            vec![WORKSPACE_READ.to_string(), MESSAGE_POST.to_string()],
        );
        let content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        let me = content(server.call_tool(&auth, "whoami", &json!({})).await.unwrap());
        assert_eq!(me["member_id"], json!(member.id.0));
        assert_eq!(me["workspace_id"], json!(ws.id.0));
        assert_eq!(me["is_bearer"], json!(true));
        let caps = me["capabilities"].as_array().unwrap();
        assert!(caps.iter().any(|c| c == "workspace:read"));

        // initialize carries the spec `instructions` cold-start guide.
        let init = server
            .handle(
                JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(json!(1)),
                    method: "initialize".to_string(),
                    params: json!({ "protocolVersion": "2026-07-28" }),
                },
                &auth,
            )
            .await;
        let instructions = init.result.unwrap()["instructions"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(instructions.contains("whoami"));
        assert!(instructions.contains("claim_next_thread"));
    }

    #[tokio::test]
    async fn glossary_tools_set_get_and_list() {
        use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "gl".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        // A real member session so `created_by` satisfies its FK.
        let auth = AuthContext::from_session(
            member.id,
            ws.id,
            vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
        );
        let content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        let set = content(
            server
                .call_tool(
                    &auth,
                    "set_glossary_term",
                    &json!({ "term": "LSN", "definition": "log sequence number", "aliases": ["wal position"] }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set["term"], json!("LSN"));
        assert_eq!(set["definition"], json!("log sequence number"));

        let got = content(
            server
                .call_tool(&auth, "get_glossary_term", &json!({ "term": "LSN" }))
                .await
                .unwrap(),
        );
        assert_eq!(got["aliases"], json!(["wal position"]));

        // Unknown term -> null.
        let miss = content(
            server
                .call_tool(&auth, "get_glossary_term", &json!({ "term": "nope" }))
                .await
                .unwrap(),
        );
        assert!(miss.is_null());

        let list = content(
            server
                .call_tool(&auth, "list_glossary_terms", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["term"], json!("LSN"));
    }

    #[tokio::test]
    async fn seed_from_message_tool_spawns_a_linked_child() {
        use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "s".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "g".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("root".into()),
            })
            .await
            .unwrap();
        let source = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "tangent".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        // A real member session so the quote's author FK is satisfied.
        let auth = AuthContext::from_session(
            member.id,
            ws.id,
            vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
        );
        let content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        let child = content(
            server
                .call_tool(
                    &auth,
                    "seed_from_message",
                    &json!({ "message_id": source.id.0, "title": "branch", "inclusion": "quote" }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(child["title"], json!("branch"));
        let child_id: uuid::Uuid = serde_json::from_value(child["id"].clone()).unwrap();

        // The seeded_from edge points from the child thread at the source message.
        let refs = store
            .list_references_to(maidan_types::RefSide::Message, source.id.0)
            .await
            .unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].relation, maidan_types::RelationKind::SeededFrom);
        assert_eq!(refs[0].src_id, child_id);

        // The quote inclusion posted a first message quoting the source.
        let msgs = store
            .list_messages(maidan_types::ThreadId(child_id), 10)
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "> tangent");
    }

    #[tokio::test]
    async fn snapshot_thread_context_tool_freezes_and_refs() {
        use maidan_auth::capability::{ARTIFACT_UPLOAD, WORKSPACE_READ};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "cs".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "g".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .unwrap();
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "freeze me".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::from_session(
            member.id,
            ws.id,
            vec![ARTIFACT_UPLOAD.to_string(), WORKSPACE_READ.to_string()],
        );
        let content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        let artifact = content(
            server
                .call_tool(
                    &auth,
                    "snapshot_thread_context",
                    &json!({ "thread_id": thread.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(artifact["kind"], json!("context_snapshot"));
        let sha = artifact["sha256"].as_str().unwrap().to_string();
        assert!(artifact["size_bytes"].as_i64().unwrap() > 0);

        // The Cluster-204 access ref is recorded for the caller's workspace.
        assert!(store.artifact_ref_exists(ws.id, &sha).await.unwrap());

        // An identical snapshot dedups to the same sha.
        let again = content(
            server
                .call_tool(
                    &auth,
                    "snapshot_thread_context",
                    &json!({ "thread_id": thread.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(again["sha256"].as_str().unwrap(), sha);
    }

    #[tokio::test]
    async fn mcp_artifact_tools_enforce_tenant_isolation() {
        use maidan_auth::capability::{ARTIFACT_UPLOAD, WORKSPACE_READ};

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let mk = |name: &str| {
            let store = store.clone();
            let name = name.to_string();
            async move {
                let ws = store
                    .create_workspace(NewWorkspace { name: name.clone() })
                    .await
                    .unwrap();
                let m = store
                    .create_member(NewMember {
                        workspace_id: ws.id,
                        handle: name,
                        display_name: None,
                        kind: MemberKind::Agent,
                    })
                    .await
                    .unwrap();
                (ws.id, m.id)
            }
        };
        let (ws_a, member_a) = mk("a").await;
        let (ws_b, member_b) = mk("b").await;
        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let caps = || vec![ARTIFACT_UPLOAD.to_string(), WORKSPACE_READ.to_string()];
        let auth_a = AuthContext::from_session(member_a, ws_a, caps());
        let auth_b = AuthContext::from_session(member_b, ws_b, caps());
        let content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // Workspace A uploads an artifact (records A's access ref).
        let up = content(
            server
                .call_tool(
                    &auth_a,
                    "upload_artifact",
                    &json!({ "kind": "attachment", "content_base64": "aGVsbG8=" }),
                )
                .await
                .unwrap(),
        );
        let sha = up["sha256"].as_str().unwrap().to_string();

        // A can read its own artifact metadata; B cannot (NotFound — no oracle).
        assert!(server
            .call_tool(&auth_a, "get_artifact_metadata", &json!({ "sha256": sha }))
            .await
            .is_ok());
        let denied = server
            .call_tool(&auth_b, "get_artifact_metadata", &json!({ "sha256": sha }))
            .await;
        assert!(
            matches!(denied, Err(McpError::NotFound)),
            "B must not read A's artifact metadata, got {denied:?}"
        );

        // Same isolation on the resources/read path (maidan://artifacts/{sha}).
        let req = |uri: String| JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "resources/read".to_string(),
            params: json!({ "uri": uri }),
        };
        let uri = format!("maidan://artifacts/{sha}");
        let resp_b = server.handle(req(uri.clone()), &auth_b).await;
        assert!(
            resp_b.error.is_some() && resp_b.result.is_none(),
            "B must not read A's artifact via resources/read"
        );
        let resp_a = server.handle(req(uri), &auth_a).await;
        assert!(resp_a.result.is_some() && resp_a.error.is_none());
    }

    #[tokio::test]
    async fn skill_tools_declare_and_list() {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "sk".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "q".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("task".into()),
            })
            .await
            .unwrap();

        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::bypass();
        let content = |v: Value| -> Value {
            serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        server
            .call_tool(
                &auth,
                "add_member_skill",
                &json!({ "member_id": member.id.0, "skill": "rust" }),
            )
            .await
            .unwrap();
        let skills = content(
            server
                .call_tool(
                    &auth,
                    "list_member_skills",
                    &json!({ "member_id": member.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(skills.as_array().unwrap().len(), 1);
        assert_eq!(skills[0]["skill"], json!("rust"));

        server
            .call_tool(
                &auth,
                "add_thread_required_skill",
                &json!({ "thread_id": thread.id.0, "skill": "code-review" }),
            )
            .await
            .unwrap();
        let reqs = content(
            server
                .call_tool(
                    &auth,
                    "list_thread_required_skills",
                    &json!({ "thread_id": thread.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(reqs.as_array().unwrap().len(), 1);
        assert_eq!(reqs[0]["skill"], json!("code-review"));
    }

    #[tokio::test]
    async fn inbox_tools_surface_a_members_mentions() {
        let (server, thread, member) = mk_server().await;
        let auth = AuthContext::bypass();

        // Post a message and record a mention of the member.
        let msg = server
            .store
            .post_message(NewMessage {
                thread_id: thread,
                author_id: member,
                body: "hey there".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        server.store.record_mention(msg.id, member).await.unwrap();

        let parse = |out: Value| -> Value {
            serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap()
        };

        // list_mentions returns the mention.
        let out = server
            .call_tool(&auth, "list_mentions", &json!({ "member_id": member.0 }))
            .await
            .unwrap();
        assert_eq!(parse(out).as_array().unwrap().len(), 1);

        // get_inbox surfaces it too (as an object with the inbox shape).
        let out = server
            .call_tool(&auth, "get_inbox", &json!({ "member_id": member.0 }))
            .await
            .unwrap();
        assert!(parse(out).is_object());

        // mark_inbox_read advances the cursor and returns the inbox.
        let out = server
            .call_tool(
                &auth,
                "mark_inbox_read",
                &json!({ "member_id": member.0, "read_through": chrono::Utc::now().to_rfc3339() }),
            )
            .await
            .unwrap();
        assert_eq!(out["isError"], json!(false));
    }

    #[tokio::test]
    async fn initialize_echoes_negotiated_version_and_the_initialized_notification_is_accepted() {
        let (server, _thread, _member) = mk_server().await;
        let auth = AuthContext::bypass();
        let init = server
            .handle(
                request(1, "initialize", json!({ "protocolVersion": "2024-11-05" })),
                &auth,
            )
            .await;
        assert_eq!(init.result.unwrap()["protocolVersion"], "2024-11-05");
        // The post-init handshake notification is dispatched without error.
        let ack = server
            .handle(request(2, "notifications/initialized", json!({})), &auth)
            .await;
        assert!(ack.error.is_none());
    }

    #[tokio::test]
    async fn initialize_negotiates_the_2026_revision_when_requested() {
        // A current (2026-07-28) client's initialize is echoed that revision (J3.1);
        // 2026 clients need not call initialize, but when they do it must negotiate.
        let (server, _thread, _member) = mk_server().await;
        let auth = AuthContext::bypass();
        let init = server
            .handle(
                request(1, "initialize", json!({ "protocolVersion": "2026-07-28" })),
                &auth,
            )
            .await;
        assert_eq!(init.result.unwrap()["protocolVersion"], "2026-07-28");
    }

    #[tokio::test]
    async fn subscribe_and_post_message_emits_resource_updated_notification() {
        let (server, thread_id, member_id) = mk_server().await;
        let auth = AuthContext::bypass();
        let uri = format!("maidan://threads/{}", thread_id.0);

        let subscribe = server
            .handle(
                request(1, "resources/subscribe", json!({ "uri": uri })),
                &auth,
            )
            .await;
        assert!(subscribe.error.is_none());

        let _ = server
            .handle(
                request(
                    2,
                    "tools/call",
                    json!({
                        "name": "post_message",
                        "arguments": {
                            "thread_id": thread_id.0,
                            "author_id": member_id.0,
                            "body": "hello"
                        }
                    }),
                ),
                &auth,
            )
            .await;

        let notifications = server.take_pending_notifications().await;
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].method, NOTIFY_RESOURCE_UPDATED);
        assert_eq!(
            notifications[0].params["uri"],
            format!("maidan://threads/{}", thread_id.0)
        );
    }

    #[tokio::test]
    async fn unsubscribe_suppresses_future_notifications() {
        let (server, thread_id, member_id) = mk_server().await;
        let auth = AuthContext::bypass();
        let uri = format!("maidan://threads/{}", thread_id.0);

        let _ = server
            .handle(
                request(1, "resources/subscribe", json!({ "uri": uri.clone() })),
                &auth,
            )
            .await;
        let _ = server
            .handle(
                request(2, "resources/unsubscribe", json!({ "uri": uri })),
                &auth,
            )
            .await;

        let _ = server
            .handle(
                request(
                    3,
                    "tools/call",
                    json!({
                        "name": "post_message",
                        "arguments": {
                            "thread_id": thread_id.0,
                            "author_id": member_id.0,
                            "body": "hello"
                        }
                    }),
                ),
                &auth,
            )
            .await;

        assert!(server.take_pending_notifications().await.is_empty());
    }

    #[tokio::test]
    async fn resource_notifier_delivers_to_local_sse_subscribers_via_listener() {
        use maidan_bus::InMemoryResourceNotifier;

        let (server, thread_id, _member_id) = mk_server().await;
        let notifier = Arc::new(InMemoryResourceNotifier::new());
        let server = Arc::new(server.with_resource_notifier(notifier));
        server.spawn_resource_notify_listener();

        let auth = AuthContext::bypass();
        let uri = format!("maidan://threads/{}", thread_id.0);
        let _ = server
            .handle(
                request(1, "resources/subscribe", json!({ "uri": uri.clone() })),
                &auth,
            )
            .await;

        // SSE subscriber registers before the mutation publishes.
        let mut sse = server.subscribe_notifications();
        server.publish_resource_uris(vec![uri.clone()]).await;

        // Delivery routes through the notifier and the listener loop.
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), sse.recv())
            .await
            .expect("timed out waiting for SSE notification")
            .expect("notification channel closed");
        assert_eq!(got.method, NOTIFY_RESOURCE_UPDATED);
        assert_eq!(got.params["uri"], uri);

        // Inline pending delivery still works (local, synchronous).
        let pending = server.take_pending_notifications().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].params["uri"], uri);
    }
}
