//! MCP dispatcher. Takes JSON-RPC requests and returns responses.
//! Transport-agnostic; `maidan-server` wraps it behind `POST /mcp` and the
//! profile endpoints `POST /mcp/worker` and `POST /mcp/reviewer`.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use maidan_artifacts::ArtifactStore;
use maidan_auth::AuthContext;
use maidan_bus::{EventBus, ResourceNotifier, ResourceUpdate};
use maidan_search::{EmbeddingProvider, Search};
use maidan_store::{NewMcpSubscription, Store};
use maidan_types::{BusEnvelope, Event, MemberId, OccupancyPresence, StoredEvent, WorkspaceId};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::streamable_session::{streamable_session_ttl, StreamableSessionRegistry};
use crate::subscriptions::{
    AddressedNotification, McpSession, NotificationListener, Principal, ResourceSubscriptions,
    Subscriber, SubscriptionLimit, MAX_SUBSCRIPTIONS_PER_SUBSCRIBER,
};

/// Every addressed notification reaches every open listener, which keeps its
/// own; sized for many subscribers per mutation.
const NOTIFICATION_BROADCAST_CAPACITY: usize = 1024;

/// Read-only bridge to the server's ephemeral presence hub. Kept transport-
/// agnostic so embedded MCP users may omit it; omission reads as offline.
pub trait PresenceReader: Send + Sync {
    fn member_presence(&self, workspace_id: WorkspaceId, member_id: MemberId) -> OccupancyPresence;
}

use crate::error::McpError;
use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::{caching, prompts, resources, tools};

/// Protocol revisions this server implements, newest first. `initialize`
/// negotiates against the client's requested version, and the HTTP transports
/// validate the `MCP-Protocol-Version` header against this set. The **default is
/// `2026-07-28`** ([`DEFAULT_PROTOCOL_VERSION`]) — the current revision, fully
/// landed and advertised (negotiation, stateless streamable core, and SEP-2243
/// routing headers; Protocols.md J3.1–3.5).
///
/// The 2025 revisions are here because the clients most people run speak them:
/// the official TypeScript SDK (2.0), and the Inspector built on it, request
/// `2025-11-25` and do not accept `2026-07-28`, so without these a handshake
/// with them failed before a single tool was listed. All three are served
/// over the same stateless streamable transport as `2026-07-28` — sessions are
/// optional from `2025-03-26` on — and the method surface is identical; newer
/// result fields are additive, and clients ignore what they do not know.
/// `2024-11-05` alone keeps the SSE-session model ([`SESSION_PROTOCOL_VERSION`]).
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// The one revision served over protocol-level sessions (an SSE leg plus
/// `Mcp-Session-Id`). Every other supported revision is stateless.
pub const SESSION_PROTOCOL_VERSION: &str = "2024-11-05";
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
pub fn negotiate_protocol_version(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|v| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .find(|s| **s == v)
                .copied()
        })
        .unwrap_or_else(preferred_protocol_version)
}

/// The spec `instructions`: an agent's cold-start guide, sent by `initialize`
/// and `server/discover`. The first line is what a client keeps when it stores
/// only that line, at most 250 characters, and the whole text is at most 2048.
/// Call `whoami` first, then the claim lifecycle (the "waiter loop" in
/// Integration.md), not just the read/write pair. A claimant's process dies
/// with its fencing tokens, so the guide also says to free a previous run's
/// claims once at startup, and when not to.
pub const INSTRUCTIONS: &str = "Call `whoami` first. Then `claim_next_thread` or `claim_next_workspace_thread`, `acknowledge_claim`, `get_thread_context`, `report_usage`, `set_thread_result`, `transition_thread`, and `release_claim`.\n\
A claim is leased: `renew_claim` before `assignment_expires_at`, or the task returns to the queue. `acknowledge_claim`, `renew_claim`, and `release_claim` each send `claim_lease_id`, and you release on every exit you control. A claim that returns null has nothing ready; sleep and ask again, or call `wait_for_ready`. `request_approval` opens a gate and returns; poll `get_approval_gate`.\n\
At startup, before your first claim, call `release_own_claims` once per workspace: it frees the claims a previous run of yours left leased. Skip it when another live process shares your member token, or you release that process's claims too.\n\
On `POST /mcp`, `tools/list` includes only tools your token can call. `POST /mcp/worker` and `POST /mcp/reviewer` each serve one fixed list, the same bytes for every caller. A tool your token cannot call is refused when you call it, on every endpoint.";

/// Mark a result final. `2026-07-28` requires `resultType` on every result.
/// `"input_required"` is the multi-round-trip interim, which this server sends
/// in one place: `approval_decide` asking a client that declared URL-mode
/// elicitation to open a confirmation link, and that result keeps its own
/// `resultType`. Earlier clients ignore the field, except in an empty result, which
/// the official SDK reads strictly, so `ping` (gone from `2026-07-28`) is
/// answered without it.
fn complete(mut result: Value) -> Value {
    if let Some(object) = result.as_object_mut() {
        object
            .entry("resultType")
            .or_insert_with(|| Value::from("complete"));
    }
    result
}

/// What `initialize` and `server/discover` advertise for `protocol_version`.
///
/// On `2026-07-28`, `resources.subscribe` means per-resource updates through
/// `subscriptions/listen` (`notifications.resourceSubscriptions`), which
/// replaced `resources/subscribe`. That method is not implemented, so a 2026
/// advertisement omits the flag. Earlier revisions still subscribe with
/// `resources/subscribe`, and those handshakes keep it.
fn server_capabilities(protocol_version: &str) -> Value {
    let resources = if protocol_version == "2026-07-28" {
        json!({})
    } else {
        json!({ "subscribe": true })
    };
    json!({
        "tools": {},
        "resources": resources,
        "prompts": {}
    })
}

/// The MCP Apps extension (SEP-1724's `capabilities.extensions`), said back
/// only to a client that declared it, so every other client's handshake is
/// unchanged.
fn with_ui_extension(mut capabilities: Value, ui: crate::call_context::UiSupport) -> Value {
    if ui == crate::call_context::UiSupport::Declared
        && let Some(object) = capabilities.as_object_mut()
    {
        object.insert(
            "extensions".into(),
            json!({
                crate::call_context::UI_EXTENSION: {
                    "mimeTypes": [crate::call_context::UI_MIME_TYPE]
                }
            }),
        );
    }
    capabilities
}

/// How long a confirmation link lives when the workspace sets no other on its
/// approval policy (`confirm_link_ttl_seconds`).
pub const DEFAULT_CONFIRMATION_TTL: std::time::Duration = std::time::Duration::from_secs(
    maidan_store::approval_policy::DEFAULT_CONFIRM_LINK_TTL_SECONDS as u64,
);

/// What `approval_decide` needs to send a person a confirmation link: the
/// secret its tokens are derived with and the console's public origin. How
/// long a link lives is the workspace's approval policy. Unset, the tool's confirmation path refuses rather than
/// send a link nobody could use.
#[derive(Clone)]
pub struct ApprovalConfirmationKeys {
    pub(crate) secret: Arc<[u8]>,
    pub(crate) console_origin: Option<String>,
}

impl ApprovalConfirmationKeys {
    pub fn new(secret: &[u8], console_origin: Option<String>) -> Self {
        Self {
            secret: Arc::from(secret),
            console_origin: console_origin
                .map(|o| o.trim().trim_end_matches('/').to_string())
                .filter(|o| !o.is_empty()),
        }
    }

    /// The console link for a gate's confirmation. The token rides in the
    /// fragment, which a browser never sends, so it reaches no server or proxy
    /// log. Without a configured origin the link is host-relative.
    pub(crate) fn link(&self, gate_id: maidan_types::ApprovalGateId, token: &str) -> String {
        let path = format!("/ui/#confirm-approval={}.{token}", gate_id.0);
        match &self.console_origin {
            Some(origin) => format!("{origin}{path}"),
            None => path,
        }
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
    subscriptions: Arc<ResourceSubscriptions>,
    notification_tx: broadcast::Sender<AddressedNotification>,
    streamable_sessions: Arc<StreamableSessionRegistry>,
    pub(crate) event_bus: Option<Arc<dyn EventBus>>,
    resource_notifier: Option<Arc<dyn ResourceNotifier>>,
    /// Server-injected slash-command dispatcher. Set once at startup via
    /// [`Self::set_slash_dispatcher`]; `None` (never set) means MCP posts skip
    /// slash dispatch (the pre-345 behaviour, and how tests run).
    slash_dispatcher: std::sync::OnceLock<Arc<dyn crate::slash_dispatch::SlashDispatcher>>,
    /// The at-rest encryption key for resolving named secrets. Set once at
    /// startup from the server's keyring; unset in tests/embedders that don't
    /// configure one, in which case `resolve_secret` reports it's unavailable.
    encryption_key: std::sync::OnceLock<Arc<[u8; 32]>>,
    /// The instance-wide ceiling on the hosts a workspace may trust with its
    /// secret values (`MAIDAN_SECRET_EGRESS_ALLOWLIST`). Unset means no
    /// ceiling: the workspace lists alone decide. REST and the egress broker
    /// read it from here too.
    secret_egress_ceiling: std::sync::OnceLock<Vec<String>>,
    /// Operator Ed25519 key for signed workspace export. Unset means
    /// `export_workspace` refuses — never emit an unsigned bundle.
    export_signing: std::sync::OnceLock<maidan_auth::ExportSigningKey>,
    /// Optional public-key pin for `verify_workspace_export` / import.
    /// Empty = integrity against the embedded key only (blank-instance default).
    export_verify_keys: std::sync::OnceLock<Vec<[u8; 32]>>,
    presence_reader: Arc<std::sync::RwLock<Option<Arc<dyn PresenceReader>>>>,
    /// The lease a `claim_next_thread` claim gets when the caller names none
    /// (`MAIDAN_CLAIM_DEFAULT_LEASE_SECS`). REST reads it from here too.
    claim_leases: crate::claim_lease::ClaimLeasePolicy,
    /// What the instance keeps (`MAIDAN_RETENTION_*_DAYS`): the ceiling a
    /// workspace's own retention is held to. REST reads it from here too.
    instance_retention: maidan_types::RetentionDays,
    /// Drops a slash-command signing secret from the HTTP cache after revoke.
    slash_secret_forget: std::sync::OnceLock<Arc<dyn Fn(uuid::Uuid) + Send + Sync>>,
    /// Drops an FSM-hook signing secret from the HTTP cache after revoke.
    fsm_secret_forget: std::sync::OnceLock<Arc<dyn Fn(uuid::Uuid) + Send + Sync>>,
    /// Optional land-gate advisor. Unset means `advise_land_gate` is not found
    /// and no gate row is written.
    land_gate_advisor: std::sync::OnceLock<Arc<dyn crate::land_gate_advice::LandGateAdvising>>,
    /// Where tool calls are counted for the workspace's usage meters. Unset
    /// (tests, embedders) counts nothing.
    usage_meters: Option<Arc<maidan_store::usage_meters::UsageMeterBuffer>>,
    /// Reads tracked pull requests from GitHub before `get_swarm_metrics`.
    /// Unset means the metrics are what the store holds. Shared by every
    /// clone, since the GitHub sender is attached after the server is built.
    pull_refresher: Arc<std::sync::OnceLock<Arc<dyn crate::pull_refresh::OpenPullRefresher>>>,
    /// Shared by every clone, so it can be set after the server is cloned
    /// into the router's state.
    approval_confirmations: Arc<std::sync::OnceLock<ApprovalConfirmationKeys>>,
    /// Where this instance takes payment and what overage costs, for
    /// `get_credits` and `top_up_credits`. Shared by every clone, since the
    /// server sets it after the router's state holds a clone. The default
    /// takes no payment.
    credit_terms: Arc<std::sync::RwLock<Arc<maidan_types::CreditTerms>>>,
    /// Whether `set_thread_result` refuses a change that would push a
    /// workflow file (`MAIDAN_CHANGE_REFUSE_WORKFLOW_PATHS`). REST reads it
    /// from here too, so both doors refuse alike. Shared by every clone.
    refuse_workflow_paths: Arc<std::sync::atomic::AtomicBool>,
    /// The `wait_for_*` calls each workspace has parked, held to
    /// [`crate::parked_waits::MAX_PARKED_WAITS_PER_WORKSPACE`].
    parked_waits: Arc<crate::parked_waits::ParkedWaits>,
}

/// The tool error a call refused past the allowance answers: the same text
/// as the HTTP 402, and the meter in `structuredContent`.
fn over_allowance_result(meter: maidan_types::UsageMeter) -> Value {
    let text = maidan_types::over_allowance_detail(meter);
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": { "error": "over_allowance", "meter": meter.as_str(), "detail": text },
        "isError": true,
    })
}

/// What a caller with no credential may do on a dev instance: discover the
/// server and read. A subscription, or a tool that writes, is refused with the
/// reason so the client can ask for a token. A tool the catalog does not have
/// gets the usual unknown-tool error, and a method not listed here is not
/// found, so a method added later stays closed to this caller until it is
/// listed.
fn anonymous_may(request: &JsonRpcRequest) -> Result<(), McpError> {
    match request.method.as_str() {
        "initialize"
        | "server/discover"
        | "ping"
        | "notifications/initialized"
        | "notifications/cancelled"
        | "tools/list"
        | "resources/list"
        | "resources/templates/list"
        | "resources/read"
        | "prompts/list"
        | "prompts/get" => Ok(()),
        "tools/call" => {
            let name = request
                .params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if tools::required_capability(name).is_ok() && !tools::is_read_only(name) {
                Err(McpError::Forbidden(format!(
                    "{name} is not a read-only tool, and this endpoint only reads for a caller \
                     with no credential. Send a bearer token to call it"
                )))
            } else {
                Ok(())
            }
        }
        "resources/subscribe" | "resources/unsubscribe" => Err(McpError::Forbidden(format!(
            "{} is not open to a caller with no credential. Send a bearer token",
            request.method
        ))),
        other => Err(McpError::MethodNotFound(other.into())),
    }
}

/// The tools a caller with no credential sees: the read-only ones, each
/// declaring per tool that it needs no sign-in, which is how ChatGPT tells a
/// tool it may call anonymously from one that needs a linked account.
fn anonymous_catalog(tools: Vec<Value>) -> Vec<Value> {
    tools
        .into_iter()
        .filter(|tool| tool["name"].as_str().is_some_and(tools::is_read_only))
        .map(|mut tool| {
            tool["securitySchemes"] = json!([{ "type": "noauth" }]);
            tool
        })
        .collect()
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
            subscriptions: Arc::new(ResourceSubscriptions::new(streamable_session_ttl())),
            notification_tx,
            streamable_sessions: Arc::new(StreamableSessionRegistry::new()),
            event_bus: None,
            resource_notifier: None,
            slash_dispatcher: std::sync::OnceLock::new(),
            encryption_key: std::sync::OnceLock::new(),
            secret_egress_ceiling: std::sync::OnceLock::new(),
            export_signing: std::sync::OnceLock::new(),
            export_verify_keys: std::sync::OnceLock::new(),
            presence_reader: Arc::new(std::sync::RwLock::new(None)),
            claim_leases: crate::claim_lease::ClaimLeasePolicy::from_env(),
            instance_retention: maidan_store::retention_policy::instance_retention_from_env(),
            slash_secret_forget: std::sync::OnceLock::new(),
            fsm_secret_forget: std::sync::OnceLock::new(),
            land_gate_advisor: std::sync::OnceLock::new(),
            usage_meters: None,
            pull_refresher: Arc::default(),
            approval_confirmations: Arc::new(std::sync::OnceLock::new()),
            credit_terms: Arc::default(),
            refuse_workflow_paths: Arc::new(std::sync::atomic::AtomicBool::new(
                maidan_types::switch_is_on(
                    std::env::var(maidan_types::REFUSE_WORKFLOW_PATHS_ENV)
                        .ok()
                        .as_deref(),
                ),
            )),
            parked_waits: Arc::default(),
        }
    }

    /// Whether a change result that would push a workflow file is refused
    /// when it is set ([`maidan_types::workflow_paths_refusal`]).
    pub fn refuses_workflow_paths(&self) -> bool {
        self.refuse_workflow_paths
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Turn the workflow-path refusal on or off (tests and embedders; the
    /// server reads it from the environment).
    pub fn set_refuse_workflow_paths(&self, on: bool) {
        self.refuse_workflow_paths
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// The `wait_for_*` calls `workspace_id` has parked right now.
    pub fn parked_waits(&self, workspace_id: maidan_types::WorkspaceId) -> usize {
        self.parked_waits.parked(workspace_id)
    }

    /// Install what `approval_decide` sends confirmation links with. Set once;
    /// a later call is ignored, since the secret does not change while the
    /// server runs.
    pub fn set_approval_confirmations(&self, keys: ApprovalConfirmationKeys) {
        let _ = self.approval_confirmations.set(keys);
    }

    pub(crate) fn approval_confirmations(&self) -> Option<&ApprovalConfirmationKeys> {
        self.approval_confirmations.get()
    }

    /// How long a subscription outlives its last listener (the server reads
    /// it from `MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS`). Set before anything
    /// subscribes or listens.
    #[must_use]
    pub fn with_resource_subscription_idle_ttl(mut self, ttl: std::time::Duration) -> Self {
        self.subscriptions = Arc::new(ResourceSubscriptions::new(ttl));
        self
    }

    /// Replace the claim-lease policy (tests and embedders; the server reads
    /// it from the environment).
    #[must_use]
    pub fn with_claim_lease_policy(mut self, policy: crate::claim_lease::ClaimLeasePolicy) -> Self {
        self.claim_leases = policy;
        self
    }

    /// The claim-lease policy both REST and MCP resolve leases with.
    pub fn claim_lease_policy(&self) -> crate::claim_lease::ClaimLeasePolicy {
        self.claim_leases
    }

    /// Replace what the instance keeps (tests and embedders; the server reads
    /// it from the environment the retention sweeper reads).
    #[must_use]
    pub fn with_instance_retention(mut self, instance: maidan_types::RetentionDays) -> Self {
        self.instance_retention = instance;
        self
    }

    pub(crate) fn usage_meters(
        &self,
    ) -> Option<&Arc<maidan_store::usage_meters::UsageMeterBuffer>> {
        self.usage_meters.as_ref()
    }

    /// Count tool calls into `meters`, the buffer the server flushes.
    #[must_use]
    pub fn with_usage_meters(
        mut self,
        meters: Arc<maidan_store::usage_meters::UsageMeterBuffer>,
    ) -> Self {
        self.usage_meters = Some(meters);
        self
    }

    /// Replace the version `initialize` reports as this server's (embedders;
    /// the standalone default is this crate's version). The workspace crates
    /// are versioned 0.0.0 and releases are baked in by the embedder, so the
    /// server crate passes its own release version through here — a
    /// connected client should see the release it is talking to.
    #[must_use]
    pub fn with_server_version(mut self, version: impl Into<String>) -> Self {
        self.server_version = version.into();
        self
    }

    /// The ceiling both REST and MCP check a workspace's retention against.
    pub fn instance_retention(&self) -> maidan_types::RetentionDays {
        self.instance_retention
    }

    pub fn attach_presence_reader(&self, reader: Arc<dyn PresenceReader>) {
        *self
            .presence_reader
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reader);
    }

    pub(crate) fn member_presence(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> OccupancyPresence {
        self.presence_reader
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|reader| reader.member_presence(workspace_id, member_id))
            .unwrap_or(OccupancyPresence::Offline)
    }

    /// Set the at-rest encryption key for `resolve_secret`. Called once at
    /// startup; unset in tests/embedders. Works through `&self` (the server is
    /// `Arc`-shared).
    pub fn set_encryption_key(&self, key: Arc<[u8; 32]>) {
        let _ = self.encryption_key.set(key);
    }

    pub(crate) fn encryption_key(&self) -> Option<&Arc<[u8; 32]>> {
        self.encryption_key.get()
    }

    /// Install the HTTP slash-secret cache forget hook. Called once at startup.
    pub fn set_slash_secret_forget(&self, hook: Arc<dyn Fn(uuid::Uuid) + Send + Sync>) {
        let _ = self.slash_secret_forget.set(hook);
    }

    pub(crate) fn forget_slash_secret(&self, id: uuid::Uuid) {
        if let Some(hook) = self.slash_secret_forget.get() {
            hook(id);
        }
    }

    /// Install the HTTP FSM-hook secret cache forget hook. Called once at startup.
    pub fn set_fsm_secret_forget(&self, hook: Arc<dyn Fn(uuid::Uuid) + Send + Sync>) {
        let _ = self.fsm_secret_forget.set(hook);
    }

    pub(crate) fn forget_fsm_secret(&self, id: uuid::Uuid) {
        if let Some(hook) = self.fsm_secret_forget.get() {
            hook(id);
        }
    }

    /// Install the land-gate advisor shared with REST. Called once at startup.
    pub fn set_land_gate_advisor(
        &self,
        advisor: Arc<dyn crate::land_gate_advice::LandGateAdvising>,
    ) {
        let _ = self.land_gate_advisor.set(advisor);
    }

    pub(crate) fn land_gate_advisor(
        &self,
    ) -> Option<&Arc<dyn crate::land_gate_advice::LandGateAdvising>> {
        self.land_gate_advisor.get()
    }

    /// Set the instance's credit terms (the server does, at boot).
    pub fn set_credit_terms(&self, terms: Arc<maidan_types::CreditTerms>) {
        if let Ok(mut held) = self.credit_terms.write() {
            *held = terms;
        }
    }

    pub(crate) fn credit_terms(&self) -> Arc<maidan_types::CreditTerms> {
        self.credit_terms
            .read()
            .map(|held| held.clone())
            .unwrap_or_default()
    }

    /// Install the GitHub refresh `get_swarm_metrics` runs first, as the
    /// route does. Called once, when the GitHub sender is attached.
    pub fn set_pull_refresher(&self, refresher: Arc<dyn crate::pull_refresh::OpenPullRefresher>) {
        let _ = self.pull_refresher.set(refresher);
    }

    pub(crate) fn pull_refresher(
        &self,
    ) -> Option<&Arc<dyn crate::pull_refresh::OpenPullRefresher>> {
        self.pull_refresher.get()
    }

    /// Set the instance ceiling on secret-egress hosts (lowercase hostnames).
    /// Called once at startup when `MAIDAN_SECRET_EGRESS_ALLOWLIST` is set; an
    /// empty list means no host may receive a secret value.
    pub fn set_secret_egress_ceiling(&self, hosts: Vec<String>) {
        let _ = self.secret_egress_ceiling.set(hosts);
    }

    /// The instance ceiling, or `None` when there is none.
    pub fn secret_egress_ceiling(&self) -> Option<&[String]> {
        self.secret_egress_ceiling.get().map(Vec::as_slice)
    }

    /// Install the operator signing key for `export_workspace`. Called once at
    /// startup; unset means the tool refuses.
    pub fn set_export_signing(&self, key: maidan_auth::ExportSigningKey) {
        let _ = self.export_signing.set(key);
    }

    pub(crate) fn export_signing(&self) -> Option<&maidan_auth::ExportSigningKey> {
        self.export_signing.get()
    }

    /// Install the optional authenticity pin for verify/import. Empty or
    /// unset = integrity against the envelope's embedded public key only.
    pub fn set_export_verify_keys(&self, keys: Vec<[u8; 32]>) {
        let _ = self.export_verify_keys.set(keys);
    }

    pub(crate) fn export_verify_keys(&self) -> &[[u8; 32]] {
        self.export_verify_keys
            .get()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Attach the server-side slash-command dispatcher. Called once at startup
    /// from the server binary (only `main.rs`), so the resulting
    /// `AppState`↔`McpServer` reference is a process-lifetime shared-state
    /// graph, never created in tests/embedders (which leave it unset → MCP
    /// posts skip slash dispatch). Works through `&self` (the server is already
    /// `Arc`-shared).
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
    /// replica. Call [`McpServer::spawn_resource_notify_listener`] after
    /// wrapping the server in an `Arc` so this process delivers cross-replica
    /// updates to its own SSE subscribers.
    pub fn with_resource_notifier(mut self, notifier: Arc<dyn ResourceNotifier>) -> Self {
        self.resource_notifier = Some(notifier);
        self
    }

    pub(crate) async fn publish_event(&self, event: Event) -> Option<i64> {
        let bus = self.event_bus.as_ref()?;
        let stored = self.store.append_event(&event).await.ok()?;
        maidan_observability::observe_event(&event);
        let envelope = BusEnvelope {
            log_id: stored.id,
            event,
            attribution: stored.attribution(),
            trace: stored.trace.clone(),
        };
        let _ = bus.publish(envelope).await;
        Some(stored.id)
    }

    /// Bus-notify an event that was **already durably appended** by a `*_with_event`
    /// store call — the MCP analogue of the REST `publish_stored`. Hydrates the
    /// `Event` from the stored payload; a missing bus (embedded use) is a no-op.
    /// Unlike [`Self::publish_event`], this does NOT append (no double-log).
    pub(crate) async fn publish_stored(&self, stored: &StoredEvent) {
        if let Ok(event) = stored.opened_event() {
            maidan_observability::observe_event(&event);
        }
        if let Some(bus) = self.event_bus.as_ref()
            && let Ok(envelope) = BusEnvelope::from_stored(stored)
        {
            let _ = bus.publish(envelope).await;
        }
    }

    pub fn streamable_sessions(&self) -> Arc<StreamableSessionRegistry> {
        self.streamable_sessions.clone()
    }

    /// Register a new streamable HTTP session id, or validate an existing
    /// session this caller opened.
    pub async fn touch_streamable_session(
        &self,
        existing: Option<&str>,
        auth: &AuthContext,
    ) -> String {
        let registry = self.streamable_sessions();
        if let Some(id) = existing.filter(|s| !s.is_empty())
            && registry.is_open_for(id, &Principal::of(auth)).await
        {
            registry.touch(id).await;
            return id.to_string();
        }
        uuid::Uuid::new_v4().to_string()
    }

    /// Close a streamable session this caller opened, and end every
    /// subscription made in it.
    pub async fn close_streamable_session(&self, session_id: &str, auth: &AuthContext) {
        if self
            .streamable_sessions
            .close_for(session_id, &Principal::of(auth))
            .await
        {
            self.subscriptions.end_session(session_id);
        }
    }

    /// Expire idle streamable sessions and every subscription that outlived
    /// its session. Returns how many sessions remain open.
    pub async fn reap_streamable_sessions(&self) -> usize {
        let open = self.streamable_sessions.open_ids().await;
        self.subscriptions.reap(&open);
        open.len()
    }

    /// How many resources this process's sessions (streamable and stdio)
    /// watch, summed over subscribers. Stateless subscriptions are in the
    /// store, shared by every replica, and not counted here.
    pub fn resource_subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// Open a notification stream for `auth` in `session`. It yields only the
    /// `notifications/resources/updated` for what this caller subscribed to in
    /// this session, and keeps those subscriptions alive while it is open. A
    /// stateless caller's stream may be open on any replica, whichever one
    /// took its subscriptions.
    pub async fn listen(&self, auth: &AuthContext, session: McpSession) -> NotificationListener {
        let subscriber = Subscriber::new(auth, session);
        let listener = NotificationListener::open(
            self.notification_tx.subscribe(),
            subscriber.clone(),
            auth,
            self.subscriptions.clone(),
        );
        // Opening a listener is activity: a subscription taken long before it
        // must not lapse before the next upkeep pass notices the listener.
        if subscriber.session == McpSession::Stateless {
            self.extend_stateless_subscriptions(vec![subscriber.principal.key()])
                .await;
        }
        listener
    }

    fn stateless_expiry(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        chrono::Duration::from_std(self.subscriptions.idle_ttl())
            .ok()
            .and_then(|ttl| now.checked_add_signed(ttl))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }

    async fn extend_stateless_subscriptions(&self, subscribers: Vec<String>) {
        if subscribers.is_empty() {
            return;
        }
        let now = Utc::now();
        if let Err(err) = self
            .store
            .extend_mcp_resource_subscriptions(&subscribers, now, self.stateless_expiry(now))
            .await
        {
            tracing::warn!(error = %err, "mcp subscription upkeep: extending failed");
        }
    }

    /// Keep the stateless subscriptions of every caller listening in this
    /// process from lapsing, and delete everyone's that have. A caller whose
    /// listener closed, or whose replica died, is no longer extended by
    /// anyone, so its subscriptions go once the idle TTL passes.
    pub async fn keep_stateless_subscriptions(&self) {
        let mut listening: Vec<String> = self
            .subscriptions
            .stateless_listeners()
            .into_iter()
            .map(|(subscriber, _)| subscriber.principal.key())
            .collect();
        listening.sort();
        listening.dedup();
        self.extend_stateless_subscriptions(listening).await;
        if let Err(err) = self.store.reap_mcp_resource_subscriptions(Utc::now()).await {
            tracing::warn!(error = %err, "mcp subscription upkeep: reaping failed");
        }
    }

    /// Invoke a tool by name (used by slash-command dispatch and tests).
    pub async fn call_tool(
        &self,
        auth: &AuthContext,
        name: &str,
        args: &Value,
    ) -> Result<Value, McpError> {
        let params = json!({ "name": name, "arguments": args });
        let started = std::time::Instant::now();
        let result = self.tools_call(&params, auth, None).await;
        crate::request_log::record(
            "tools/call",
            &params,
            auth,
            crate::request_log::Transport::Slash,
            None,
            started.elapsed(),
            &result,
        );
        maidan_auth::record_delegated_authorization(
            self.store.as_ref(),
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            &format!("tools/call:{name}"),
            if result.is_ok() {
                maidan_auth::AuthorizationOutcome::Allowed
            } else {
                maidan_auth::AuthorizationOutcome::Denied
            },
        )
        .await;
        result
    }

    /// Handle a request on a stateless transport.
    pub async fn handle(&self, request: JsonRpcRequest, auth: &AuthContext) -> JsonRpcResponse {
        self.handle_in(request, auth, &McpSession::Stateless).await
    }

    /// Handle a request on a profile endpoint. `tools/list` is that profile's
    /// fixed list; `tools/call` refuses a tool the profile does not name.
    pub async fn handle_profile(
        &self,
        request: JsonRpcRequest,
        auth: &AuthContext,
        profile: crate::profiles::Profile,
    ) -> JsonRpcResponse {
        self.respond(request, auth, &McpSession::Stateless, Some(profile))
            .await
    }

    /// Handle a request that arrived in `session`; a `resources/subscribe`
    /// belongs to that session.
    pub async fn handle_in(
        &self,
        request: JsonRpcRequest,
        auth: &AuthContext,
        session: &McpSession,
    ) -> JsonRpcResponse {
        self.respond(request, auth, session, None).await
    }

    async fn respond(
        &self,
        request: JsonRpcRequest,
        auth: &AuthContext,
        session: &McpSession,
        profile: Option<crate::profiles::Profile>,
    ) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        let action = if request.method == "tools/call" {
            request
                .params
                .get("name")
                .and_then(Value::as_str)
                .map(|name| format!("tools/call:{name}"))
                .unwrap_or_else(|| "tools/call".into())
        } else {
            request.method.clone()
        };
        let started = std::time::Instant::now();
        let result = self.dispatch(&request, auth, session, profile).await;
        crate::request_log::record(
            &request.method,
            &request.params,
            auth,
            crate::request_log::Transport::of(session),
            profile,
            started.elapsed(),
            &result,
        );
        maidan_auth::record_delegated_authorization(
            self.store.as_ref(),
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            &action,
            if result.is_ok() {
                maidan_auth::AuthorizationOutcome::Allowed
            } else {
                maidan_auth::AuthorizationOutcome::Denied
            },
        )
        .await;
        let response = match result {
            Ok(result) if request.method == "ping" => JsonRpcResponse::success(id, result),
            Ok(result) => JsonRpcResponse::success(id, complete(result)),
            Err(err) => {
                tracing::debug!(method = %request.method, error = %err, "mcp dispatch error");
                JsonRpcResponse::failure(id, err.to_jsonrpc())
            }
        };
        #[cfg(feature = "frame-capture")]
        crate::request_log::capture(&request, &response);
        response
    }

    async fn dispatch(
        &self,
        request: &JsonRpcRequest,
        auth: &AuthContext,
        session: &McpSession,
        profile: Option<crate::profiles::Profile>,
    ) -> Result<Value, McpError> {
        if auth.is_anonymous() {
            anonymous_may(request)?;
        }
        match request.method.as_str() {
            "initialize" => self.initialize(&request.params).await,
            "server/discover" => {
                let ui = crate::call_context::UiSupport::from_params(&request.params);
                // A card-declaring client gets the extension back; that answer
                // varies by request, so no shared cache keeps it.
                let hint = if ui == crate::call_context::UiSupport::Declared {
                    caching::CacheHint {
                        scope: caching::CacheScope::Private,
                        ..caching::DISCOVER
                    }
                } else {
                    caching::DISCOVER
                };
                Ok(caching::with_hint(self.discover(ui), hint))
            }
            // The client's post-initialize handshake notification is accepted
            // (and ignored) rather than treated as an unknown method.
            "notifications/initialized" | "notifications/cancelled" => Ok(json!({})),
            // Either side may ping at any time, and the answer is an empty result.
            "ping" => Ok(json!({})),
            "tools/list" => {
                let (tools, hint) = match profile {
                    Some(profile) => (profile.catalog(), caching::PROFILE_TOOLS_LIST),
                    None => (tools::catalog_for(auth), caching::TOOLS_LIST),
                };
                let mut tools = if auth.is_anonymous() {
                    anonymous_catalog(tools)
                } else {
                    tools
                };
                // A client that declared no MCP Apps gets the catalog without
                // the card's link. Those bytes answer what the request
                // declared, so no shared cache keeps them for anyone else.
                let hint =
                    if crate::call_context::UiSupport::from_params(&request.params).offers_card() {
                        hint
                    } else {
                        crate::apps_card::strip_tool_links(&mut tools);
                        caching::CacheHint {
                            scope: caching::CacheScope::Private,
                            ..hint
                        }
                    };
                Ok(caching::with_hint(json!({ "tools": tools }), hint))
            }
            "tools/call" => self.tools_call(&request.params, auth, profile).await,
            "resources/list" => {
                let mut listed = resources::listed(auth);
                listed.extend(crate::apps_card::listed(
                    crate::call_context::UiSupport::from_params(&request.params),
                ));
                Ok(caching::with_hint(
                    json!({ "resources": listed }),
                    caching::RESOURCES_LIST,
                ))
            }
            "resources/templates/list" => Ok(caching::with_hint(
                json!({ "resourceTemplates": resources::templates() }),
                caching::RESOURCE_TEMPLATES_LIST,
            )),
            "resources/read" => self.resources_read(&request.params, auth).await,
            "resources/subscribe" => {
                self.resources_subscribe(&request.params, auth, session)
                    .await
            }
            "resources/unsubscribe" => {
                self.resources_unsubscribe(&request.params, auth, session)
                    .await
            }
            "prompts/list" => Ok(caching::with_hint(
                json!({ "prompts": prompts::catalog() }),
                caching::PROMPTS_LIST,
            )),
            "prompts/get" => self.prompts_get(&request.params, auth).await,
            other => Err(McpError::MethodNotFound(other.into())),
        }
    }

    async fn initialize(&self, params: &Value) -> Result<Value, McpError> {
        let requested = params.get("protocolVersion").and_then(|v| v.as_str());
        let protocol_version = negotiate_protocol_version(requested);
        let ui = crate::call_context::UiSupport::from_params(params);
        Ok(json!({
            "protocolVersion": protocol_version,
            "capabilities": with_ui_extension(server_capabilities(protocol_version), ui),
            "serverInfo": self.server_info(),
            "instructions": INSTRUCTIONS,
        }))
    }

    /// `server/discover`, which `2026-07-28` makes the one place a client is
    /// told the versions, capabilities and instructions: that revision has no
    /// `initialize`, so a client speaking only it never sees what `initialize`
    /// says.
    fn discover(&self, ui: crate::call_context::UiSupport) -> Value {
        json!({
            "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS,
            "capabilities": with_ui_extension(server_capabilities("2026-07-28"), ui),
            "instructions": INSTRUCTIONS,
            "_meta": { "io.modelcontextprotocol/serverInfo": self.server_info() },
        })
    }

    fn server_info(&self) -> Value {
        json!({ "name": self.server_name, "version": self.server_version })
    }

    /// Every MCP write passes through here, whatever the transport, so this is
    /// where tool calls take on the caller's attribution. Over HTTP the auth
    /// middleware has already set the same value; stdio has no middleware, so
    /// without this its writes would read as the system's.
    /// Run a tool as its caller, and make sure a change leaves a record — the
    /// MCP half of the rule `run_as` enforces over REST. A tool that changed
    /// something without writing an event or audit row of its own gets one
    /// here: the tool, the ids it was called with, and who acted for whom.
    /// Only ids are kept from the arguments; the rest can carry secrets.
    async fn tools_call(
        &self,
        params: &Value,
        auth: &AuthContext,
        profile: Option<crate::profiles::Profile>,
    ) -> Result<Value, McpError> {
        let attribution = auth.attribution();
        let (result, recorded) = maidan_store::attribution::with_attribution_tracked(
            attribution,
            self.tools_call_unscoped(params, auth, profile),
        )
        .await;
        // Past the allowance the refusal is a tool error, not a protocol
        // error, so the model reads what to do (top up) rather than its
        // client swallowing a JSON-RPC error.
        let result = match result {
            Err(McpError::OverAllowance(meter)) => Ok(over_allowance_result(meter)),
            other => other,
        };
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let succeeded = result
            .as_ref()
            .is_ok_and(|out| out.get("isError") != Some(&Value::Bool(true)));
        if attribution.is_some() && succeeded && !recorded && !tools::is_read_only(name) {
            let ids: serde_json::Map<String, Value> = params
                .get("arguments")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter(|(key, value)| key.ends_with("_id") && value.is_string())
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let event = maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(auth.workspace_id()),
                actor_id: Some(auth.actor_id()),
                action: "mutation".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(auth.workspace_id().0),
                metadata: json!({
                    "surface": "mcp",
                    "operation": format!("tools/call:{name}"),
                    "ids": ids,
                }),
            };
            let written = maidan_store::attribution::with_attribution(
                attribution,
                self.store.append_audit(event),
            )
            .await;
            if let Err(err) = written {
                tracing::error!(target: "audit", %err, tool = name, "audit.write_failed");
            }
        }
        result
    }

    async fn tools_call_unscoped(
        &self,
        params: &Value,
        auth: &AuthContext,
        profile: Option<crate::profiles::Profile>,
    ) -> Result<Value, McpError> {
        let (name, args) = tools::tool_call(params)?;
        if let Some(profile) = profile
            && !profile.contains(name)
        {
            return Err(McpError::Forbidden(format!(
                "{name} is not on the {} profile",
                profile.name()
            )));
        }
        if !auth.bypass {
            let cap = tools::required_capability(name)?;
            maidan_auth::require_observed_capability(
                auth,
                maidan_auth::AuthorizationSurface::Mcp,
                cap,
            )
            .map_err(McpError::from)?;
        }
        // Held until the call returns. A long-poll gives back the server's
        // in-flight permit while it parks, so this is what keeps one tenant
        // from parking without bound (RED-008). Auth-disabled callers share
        // no tenant to protect.
        let _parked = if crate::parked_waits::parks(name) && !auth.bypass {
            Some(
                self.parked_waits
                    .park(auth.workspace_id())
                    .ok_or(McpError::RateLimited {
                        retry_after_ms: 1_000,
                    })?,
            )
        } else {
            None
        };
        // Counted once the call is authorized to run, whatever it answers.
        if let (Some(meters), false) = (&self.usage_meters, auth.bypass) {
            // The hard allowance cutoff: refused before it runs, not counted.
            // The credit tools stay open past it, so an agent cut off can
            // read its balance and ask for a top-up to pay its own way back.
            if meters.is_cut_off(auth.workspace_id(), maidan_types::UsageMeter::McpCalls)
                && !tools::REACHABLE_PAST_CUTOFF.contains(&name)
            {
                return Err(McpError::OverAllowance(maidan_types::UsageMeter::McpCalls));
            }
            // Stored bytes are refused by the store's message and artifact
            // writes, whichever tool reaches them.
            meters.record(auth.workspace_id(), maidan_types::UsageMeter::McpCalls, 1);
        }
        let deadline = tools::deadline(name);
        let call = crate::call_context::CallContext::from_params(params);
        // Boxed: the dispatch future is as large as its largest tool's, and
        // carried inline through every caller an unoptimized build ran out of
        // a 2 MiB thread stack.
        let dispatch = Box::pin(tools::dispatch(self, auth, name, &args, &call));
        let result = tokio::time::timeout(deadline, dispatch)
            .await
            .map_err(|_| {
                McpError::Internal(format!(
                    "tool {name} exceeded its {}s deadline",
                    deadline.as_secs()
                ))
            })??;
        self.queue_resource_updates(name, &args, &result, auth)
            .await;
        Ok(result)
    }

    async fn prompts_get(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            maidan_auth::require_observed_capability(
                auth,
                maidan_auth::AuthorizationSurface::Mcp,
                maidan_auth::capability::WORKSPACE_READ,
            )
            .map_err(McpError::from)?;
        }
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing prompt name".into()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        prompts::get(&self.store, name, &args).await
    }

    /// The per-resource check `resources/read` makes, shared with
    /// `resources/subscribe` so a caller cannot watch what it cannot read.
    /// A workspace URI must be the caller's own: `resources/read` of another
    /// workspace's URI returned its record.
    async fn authorize_resource(&self, uri: &str, auth: &AuthContext) -> Result<(), McpError> {
        if auth.bypass {
            return Ok(());
        }

        if let Some(rest) = uri.strip_prefix("maidan://") {
            let mut parts = rest.splitn(2, '/');
            match (parts.next(), parts.next()) {
                (Some("workspaces"), Some(id)) => {
                    if id.parse::<uuid::Uuid>().ok() != Some(auth.workspace_id().0) {
                        return Err(McpError::NotFound);
                    }
                }
                (Some("threads") | Some("digests"), Some(id)) => {
                    if let Ok(u) = id.parse() {
                        maidan_auth::ensure_thread_access(
                            self.store.as_ref(),
                            auth,
                            maidan_types::ThreadId(u),
                        )
                        .await?;
                    }
                }
                (Some("channels") | Some("boots"), Some(id)) => {
                    if let Ok(u) = id.parse() {
                        maidan_auth::ensure_channel_access(
                            self.store.as_ref(),
                            auth,
                            maidan_types::ChannelId(u),
                        )
                        .await?;
                    }
                }
                // The one artifact read rule, as REST applies it: another
                // tenant's artifact, or one a private channel or DM the
                // caller is not in references, is NotFound. The ref is stored
                // as canonical lowercase hex; an uppercase digest in the URI
                // is the same artifact.
                (Some("artifacts"), Some(sha)) => {
                    let owned = maidan_artifacts::Sha256::from_hex(sha)
                        .map(|digest| digest.to_hex())
                        .unwrap_or_else(|_| sha.to_string());
                    if !maidan_auth::can_read_artifact(self.store.as_ref(), auth, &owned).await? {
                        return Err(McpError::NotFound);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn resources_read(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            maidan_auth::require_observed_capability(
                auth,
                maidan_auth::AuthorizationSurface::Mcp,
                maidan_auth::capability::WORKSPACE_READ,
            )
            .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        // The card is static and the same for every caller: it carries no
        // workspace's data, so reading it needs no more than workspace:read.
        if crate::apps_card::is_ui_uri(uri) {
            return Ok(caching::with_hint(
                crate::apps_card::read(uri)?,
                caching::resource_read(uri),
            ));
        }
        let canonical = resources::canonical_uri(uri);
        self.authorize_resource(canonical.as_ref(), auth).await?;
        let read = resources::read(&self.store, canonical.as_ref(), auth).await?;
        Ok(caching::with_hint(
            read,
            caching::resource_read(canonical.as_ref()),
        ))
    }

    async fn resources_subscribe(
        &self,
        params: &Value,
        auth: &AuthContext,
        session: &McpSession,
    ) -> Result<Value, McpError> {
        if !auth.bypass {
            maidan_auth::require_observed_capability(
                auth,
                maidan_auth::AuthorizationSurface::Mcp,
                maidan_auth::capability::WORKSPACE_READ,
            )
            .map_err(McpError::from)?;
        }
        let raw = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::validate_uri(raw)?;
        let canonical = resources::canonical_uri(raw);
        let uri = canonical.as_ref();
        self.authorize_resource(uri, auth).await?;
        let at_limit = || {
            McpError::InvalidParams(format!(
                "already subscribed to {MAX_SUBSCRIPTIONS_PER_SUBSCRIBER} resources; \
                 unsubscribe from one first"
            ))
        };
        let subscriber = Subscriber::new(auth, session.clone());
        if subscriber.session == McpSession::Stateless {
            let caller = (!auth.bypass).then_some(auth);
            let subscribed = self
                .store
                .subscribe_mcp_resource(
                    &NewMcpSubscription {
                        subscriber: subscriber.principal.key(),
                        workspace_id: caller.map(|a| a.workspace_id()),
                        member_id: caller.map(|a| a.member_id()),
                        uri: uri.to_string(),
                        expires_at: self.stateless_expiry(Utc::now()),
                    },
                    MAX_SUBSCRIPTIONS_PER_SUBSCRIBER,
                )
                .await?;
            if !subscribed {
                return Err(at_limit());
            }
        } else {
            self.reap_streamable_sessions().await;
            self.subscriptions
                .subscribe(subscriber, auth, uri)
                .map_err(|SubscriptionLimit| at_limit())?;
        }
        Ok(json!({
            "ok": true,
            "uri": uri
        }))
    }

    async fn resources_unsubscribe(
        &self,
        params: &Value,
        auth: &AuthContext,
        session: &McpSession,
    ) -> Result<Value, McpError> {
        if !auth.bypass {
            maidan_auth::require_observed_capability(
                auth,
                maidan_auth::AuthorizationSurface::Mcp,
                maidan_auth::capability::WORKSPACE_READ,
            )
            .map_err(McpError::from)?;
        }
        let raw = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::validate_uri(raw)?;
        let canonical = resources::canonical_uri(raw);
        let uri = canonical.as_ref();
        let subscriber = Subscriber::new(auth, session.clone());
        let removed = if subscriber.session == McpSession::Stateless {
            self.store
                .unsubscribe_mcp_resource(&subscriber.principal.key(), uri)
                .await?
        } else {
            self.subscriptions.unsubscribe(&subscriber, uri)
        };
        Ok(json!({
            "ok": true,
            "uri": uri,
            "removed": removed
        }))
    }

    async fn queue_resource_updates(
        &self,
        tool_name: &str,
        args: &Value,
        result: &Value,
        auth: &AuthContext,
    ) {
        let updates = crate::resource_updates::uris_for_tool_mutation(
            self.store.as_ref(),
            auth.workspace_id(),
            tool_name,
            args,
            result,
        )
        .await;
        self.publish_resource_uris(updates).await;
    }

    /// Fan out `notifications/resources/updated` for a mutation's updates.
    ///
    /// With a [`ResourceNotifier`] wired, the unfiltered set is published
    /// cluster-wide and every replica's listener
    /// ([`McpServer::spawn_resource_notify_listener`]) delivers to its own
    /// subscribers — this one included, so there is a single delivery path.
    /// Without one, delivery happens here.
    pub async fn publish_resource_uris(&self, updates: Vec<ResourceUpdate>) {
        if updates.is_empty() {
            return;
        }
        match &self.resource_notifier {
            Some(notifier) => {
                if let Err(err) = notifier.publish(updates).await {
                    tracing::warn!(error = %err, "mcp resource-notify publish failed");
                }
            }
            None => self.deliver_resource_updates(&updates).await,
        }
    }

    /// Address each update to the subscribers that may hear it: those that
    /// subscribed to its URI, from the workspace the change happened in, and
    /// can still read the resource. A subscriber that has lost access loses
    /// the subscription; a store error withholds this delivery only.
    ///
    /// Session subscriptions are this process's own. Stateless ones are
    /// shared, so every replica looks up those of the stateless callers
    /// listening to it, wherever they subscribed.
    pub(crate) async fn deliver_resource_updates(&self, updates: &[ResourceUpdate]) {
        for update in updates {
            for (subscriber, auth) in self.subscriptions.watchers(&update.uri) {
                if !auth.bypass && auth.workspace_id() != update.workspace_id {
                    continue;
                }
                if !self.address(subscriber.clone(), &auth, &update.uri).await {
                    self.subscriptions.unsubscribe(&subscriber, &update.uri);
                }
            }
        }
        self.deliver_to_stateless_listeners(updates).await;
    }

    async fn deliver_to_stateless_listeners(&self, updates: &[ResourceUpdate]) {
        let listening: HashMap<String, (Subscriber, AuthContext)> = self
            .subscriptions
            .stateless_listeners()
            .into_iter()
            .map(|(subscriber, auth)| (subscriber.principal.key(), (subscriber, auth)))
            .collect();
        if listening.is_empty() {
            return;
        }
        let keys: Vec<String> = listening.keys().cloned().collect();
        let mut by_workspace: HashMap<WorkspaceId, Vec<String>> = HashMap::new();
        for update in updates {
            by_workspace
                .entry(update.workspace_id)
                .or_default()
                .push(update.uri.clone());
        }
        for (workspace_id, uris) in by_workspace {
            let watches = match self
                .store
                .mcp_resource_watchers(workspace_id, &uris, &keys, Utc::now())
                .await
            {
                Ok(watches) => watches,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "mcp resource notifications withheld: subscription lookup failed"
                    );
                    continue;
                }
            };
            for watch in watches {
                let Some((subscriber, auth)) = listening.get(&watch.subscriber) else {
                    continue;
                };
                if !auth.bypass && auth.workspace_id() != workspace_id {
                    continue;
                }
                if !self.address(subscriber.clone(), auth, &watch.uri).await
                    && let Err(err) = self
                        .store
                        .unsubscribe_mcp_resource(&watch.subscriber, &watch.uri)
                        .await
                {
                    tracing::warn!(
                        error = %err,
                        uri = %watch.uri,
                        "mcp subscription outlived its access: removing it failed"
                    );
                }
            }
        }
    }

    /// Send `uri`'s update to `subscriber` if it can still read the resource.
    /// `false` when it cannot, and so should lose the subscription.
    async fn address(&self, subscriber: Subscriber, auth: &AuthContext, uri: &str) -> bool {
        match self.authorize_resource(uri, auth).await {
            Ok(()) => {
                let _ = self.notification_tx.send(AddressedNotification {
                    subscriber: Arc::new(subscriber),
                    notification: JsonRpcNotification::new(
                        NOTIFY_RESOURCE_UPDATED,
                        json!({ "uri": uri }),
                    ),
                });
                true
            }
            Err(McpError::Internal(err)) => {
                tracing::warn!(
                    error = %err,
                    uri = %uri,
                    "mcp resource notification withheld: access re-check failed"
                );
                true
            }
            Err(_) => false,
        }
    }

    /// Spawn the loop delivering cross-replica resource updates to this
    /// process's subscribers, and the upkeep keeping its stateless listeners'
    /// subscriptions from lapsing. Without a [`ResourceNotifier`] only the
    /// upkeep runs. Call once at startup after wrapping the server in an
    /// `Arc`.
    pub fn spawn_resource_notify_listener(self: &Arc<Self>) {
        // Several passes per TTL, so a listener's subscriptions are extended
        // well before they would lapse.
        let every = (self.subscriptions.idle_ttl() / 4).clamp(
            std::time::Duration::from_millis(100),
            std::time::Duration::from_secs(60),
        );
        let upkeep = Arc::downgrade(self);
        // Started at boot, so it carries no request's scope; spawned the same
        // way as the delivery loop below so the attribution contract holds.
        maidan_store::attribution::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(server) = upkeep.upgrade() else {
                    break;
                };
                server.keep_stateless_subscriptions().await;
            }
        });
        let Some(notifier) = self.resource_notifier.clone() else {
            return;
        };
        let mut rx = notifier.subscribe();
        let server = Arc::clone(self);
        maidan_store::attribution::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(updates) => server.deliver_resource_updates(&updates).await,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "mcp resource-notify listener lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
