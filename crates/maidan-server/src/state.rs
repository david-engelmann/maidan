use std::collections::HashMap;
use std::sync::{atomic::AtomicI64, Arc, RwLock};

use maidan_artifacts::ArtifactStore;
use maidan_bus::{EventBus, HydrateStats, ListenerHealth, PresenceNotifier, ResourceNotifier};
use maidan_mcp::McpServer;
use maidan_search::{EmbeddingProvider, Search};
use maidan_store::{OutboxBackend, Store};
use maidan_types::{FsmHookId, PeerId, SlashCommandId, WebhookSubscriptionId};
use tokio::sync::RwLock as AsyncRwLock;

use crate::oidc::OidcRuntime;
use crate::presence::PresenceHub;
use crate::subscribe_resume;

/// Webhook signing secrets: encryption key + in-memory cache after mint.
#[derive(Clone)]
pub struct WebhookRuntime {
    pub encryption_key: Option<Arc<[u8; 32]>>,
    pub secrets: Arc<RwLock<HashMap<WebhookSubscriptionId, String>>>,
}

impl WebhookRuntime {
    pub fn new(encryption_key: Option<Arc<[u8; 32]>>) -> Self {
        Self {
            encryption_key,
            secrets: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// Slash command signing secrets for outbound HTTP handlers.
#[derive(Clone)]
pub struct SlashRuntime {
    pub encryption_key: Option<Arc<[u8; 32]>>,
    pub secrets: Arc<RwLock<HashMap<SlashCommandId, String>>>,
}

impl SlashRuntime {
    pub fn new(encryption_key: Option<Arc<[u8; 32]>>) -> Self {
        Self {
            encryption_key,
            secrets: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// FSM hook signing secrets for outbound HTTP handlers.
#[derive(Clone)]
pub struct FsmHookRuntime {
    pub encryption_key: Option<Arc<[u8; 32]>>,
    pub secrets: Arc<RwLock<HashMap<FsmHookId, String>>>,
}

impl FsmHookRuntime {
    pub fn new(encryption_key: Option<Arc<[u8; 32]>>) -> Self {
        Self {
            encryption_key,
            secrets: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// Outbound federation poll: encryption key, in-memory secret cache, disable flag.
#[derive(Clone)]
pub struct FederationRuntime {
    pub disabled: bool,
    pub encryption_key: Option<Arc<[u8; 32]>>,
    pub outbound_secrets: Arc<RwLock<HashMap<PeerId, String>>>,
}

impl FederationRuntime {
    pub fn new(disabled: bool, encryption_key: Option<Arc<[u8; 32]>>) -> Self {
        Self {
            disabled,
            encryption_key,
            outbound_secrets: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// Shared handles passed to every request handler. `Arc`s are cheap to
/// clone; the inner trait objects implement the relevant backend logic.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub bus: Arc<dyn EventBus>,
    pub search: Arc<dyn Search>,
    pub embedding_provider: Arc<dyn EmbeddingProvider>,
    /// Shared MCP dispatcher (subscriptions + notification fan-out).
    pub mcp: Arc<McpServer>,
    /// When true, all routes accept requests without a bearer token.
    pub auth_disabled: bool,
    /// The caller an MCP `POST` with no credential reads as, on a dev instance
    /// that set `MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE` ([`crate::dev_anonymous`]).
    pub dev_anonymous_reader: Option<maidan_auth::AuthContext>,
    /// Enables the explicit test-identity header used by in-process E2E
    /// harnesses. Never enabled by the server binary.
    pub test_identity_header: bool,
    /// When true, unauthenticated bootstrap routes are allowed (see `MAIDAN_BOOTSTRAP`).
    pub bootstrap_enabled: bool,
    pub federation: FederationRuntime,
    pub webhooks: WebhookRuntime,
    pub slash: SlashRuntime,
    pub fsm_hooks: FsmHookRuntime,
    /// Milliseconds since Unix epoch when the indexer last handled an event (0 = never).
    pub indexer_last_event_unix_ms: Arc<AtomicI64>,
    /// Highest event-log id the indexer has handled (0 = none yet).
    pub indexer_processed_log_id: Arc<AtomicI64>,
    /// Most recent indexer-side embedding failure, if any.
    pub indexer_last_error: Arc<AsyncRwLock<Option<String>>>,
    /// Postgres `LISTEN` task health; `None` when using [`maidan_bus::InMemoryBus`].
    pub bus_listener_health: Option<Arc<ListenerHealth>>,
    /// Postgres NOTIFY hydrate outcomes; `None` when using [`maidan_bus::InMemoryBus`].
    pub bus_hydrate_stats: Option<Arc<HydrateStats>>,
    /// When true, `publish` enqueues outbox only; [`crate::outbox_relay`] calls `bus.publish`.
    pub outbox_relay: bool,
    /// Capacity-1 nudge to the outbox relay: a freshly enqueued row wakes an
    /// idle relay promptly. `None` when relay/nudge isn't wired.
    pub outbox_nudge: Option<tokio::sync::mpsc::Sender<()>>,
    /// Outbox backend for relay metrics; `None` when outbox relay is disabled.
    pub outbox_backend: Option<OutboxBackend>,
    /// OIDC client + settings when `MAIDAN_OIDC_ENABLED=1`.
    pub oidc: Option<Arc<OidcRuntime>>,
    /// Browser-session keys when `MAIDAN_SESSION_SECRET` is set, for the token
    /// exchange on a server without OIDC. Read through
    /// [`Self::browser_sessions`], which prefers OIDC's.
    pub sessions: Option<crate::session::SessionSettings>,
    /// HMAC secret for subscribe resume tokens (when OIDC is off).
    pub subscribe_resume_secret: Option<Arc<[u8]>>,
    /// TTL for signed resume tokens (seconds).
    pub subscribe_resume_ttl_secs: u64,
    /// Ephemeral presence/typing fan-out for WebSocket subscribers.
    pub presence: Arc<PresenceHub>,
    /// Live `/ws/subscribe` connections, for the ceiling and the gauge.
    pub ws_connections: Arc<std::sync::atomic::AtomicUsize>,
    /// Set on SIGTERM. Readiness then fails, so the load balancer stops sending
    /// new requests before the listener closes (see `shutdown_drain_from_env`).
    pub draining: Arc<std::sync::atomic::AtomicBool>,
    /// The most concurrent subscriber connections accepted
    /// (`MAIDAN_MAX_WS_CONNECTIONS`, default 10 000); past it, upgrades get 503.
    pub max_ws_connections: usize,
    /// The in-flight HTTP request ceiling (`MAIDAN_MAX_CONCURRENT_REQUESTS`,
    /// default 1024, `0` off); past it, requests are shed with a 503.
    pub request_limit: crate::load_shed::RequestLimit,
    /// Optional Redis backend for global and per-token rate limits.
    pub rate_limit_redis: Option<redis::aio::ConnectionManager>,
    /// Apply the built-in per-client and per-workspace rate limits when
    /// `MAIDAN_RATE_LIMIT_MAX` or `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` is unset.
    /// The server bootstrap turns this on so a deployment that configures
    /// nothing still has a DoS floor and tenant fairness; an explicit value
    /// (including `0` to disable) always overrides.
    /// Left `false` in [`AppState::new`] so tests are unaffected unless they
    /// opt in.
    pub rate_limit_default_on: bool,
    /// Live embedding-indexer counters (queue depth, throughput) for metrics.
    /// Default-zeroed unless the batching indexer is wired.
    pub indexer_metrics: Arc<maidan_search::IndexerMetrics>,
    /// At-least-once delivery: stability window + reconcile poll cadence for
    /// `at_least_once` subscriptions. Read from env once at startup.
    pub delivery_stability: std::time::Duration,
    pub delivery_reconcile_interval: std::time::Duration,
    /// Off-platform email transport, built from `MAIDAN_SMTP_*` at startup.
    /// `None` when SMTP isn't configured — the config gate: no transport, no
    /// email. Set only by the server binary via [`AppState::attach_mail`], so
    /// tests/embedders (which build via [`AppState::new`]) never send email.
    pub mail: Option<Arc<dyn crate::mail::MailTransport>>,
    /// Web Push sender, built from `MAIDAN_VAPID_*` at startup. `None` when
    /// VAPID is not configured: no sender, no web push. Set only by the server
    /// binary via [`AppState::attach_web_push`], so tests and embedders (which
    /// build via [`AppState::new`]) never send.
    pub web_push: Option<Arc<dyn crate::web_push::WebPushSender>>,
    /// Uncompressed VAPID public key (base64url) for the board to subscribe.
    /// `None` when Web Push is not configured.
    pub web_push_public_key: Option<String>,
    /// Why [`Self::web_push_public_key`] is absent. `vapid_unset` when the
    /// `MAIDAN_VAPID_*` values are missing; `vapid_invalid` when they are set
    /// but unusable. No default key is invented.
    pub web_push_disabled_reason: Option<String>,
    /// Slack projector config, built from `MAIDAN_SLACK_*` at startup. `None`
    /// when unset — the config gate: the `/integrations/slack/events` route
    /// then returns `404`. Set only by the server binary via
    /// [`AppState::attach_slack`].
    pub slack: Option<Arc<crate::slack::SlackConfig>>,
    /// Slack projector egress sender, a `chat.postMessage` client set when
    /// `MAIDAN_SLACK_BOT_TOKEN` is configured. `None` disables egress (inbound
    /// still works). Set via [`AppState::attach_slack_sender`]; tests inject a
    /// mock.
    pub slack_sender: Option<Arc<dyn crate::slack::SlackSender>>,
    /// Git/GitHub projector config, built from `MAIDAN_GITHUB_*` at startup.
    /// `None` when unset — the `/integrations/github/events` route then returns
    /// `404`. Set via [`AppState::attach_github`].
    pub github: Option<Arc<crate::github::GithubConfig>>,
    /// GitHub projector egress sender, an issue-comment client set when
    /// `MAIDAN_GITHUB_TOKEN` is configured. `None` disables egress (inbound
    /// still works). Set via [`AppState::attach_github_sender`]; tests inject a
    /// mock.
    pub github_sender: Option<Arc<dyn crate::github::GithubSender>>,
    /// The one app whose installations may call mark-ready
    /// (`MAIDAN_MARK_READY_APP_ID`). An app id is unique across the instance,
    /// where an app slug is unique only within its workspace, so naming the
    /// app here keeps another workspace's look-alike from acting through the
    /// instance's GitHub token. `None` refuses every mark-ready call.
    pub mark_ready_app_id: Option<maidan_types::AppId>,
    /// A2A Agent Card transport advertisement config: public origin for
    /// absolute interface URLs + the advertised gRPC address. Default empty
    /// (host-relative URLs, no gRPC interface); the server binary sets it from
    /// env.
    pub a2a_card: crate::a2a_agent::A2aCardConfig,
    /// A read replica is configured (`MAIDAN_DB_REPLICA_URL`), so the server
    /// should stamp a `Maidan-Consistency-Token` on writes and route
    /// replica-eligible reads. Left `false` in [`AppState::new`]; the server
    /// binary sets it when it builds the store with a replica reader, so
    /// tests/embedders (no replica) skip the token round-trip and behave
    /// exactly as before.
    pub read_replica_enabled: bool,
    /// Read-routing counters for `maidan_replica_reads_total`, captured from
    /// the `PostgresStore` when a replica is configured. `None` otherwise
    /// (SQLite / single-pool), so the metric simply isn't emitted.
    pub read_routing_metrics: Option<std::sync::Arc<maidan_store::postgres::ReadRoutingMetrics>>,
    /// Search read-routing counters for `maidan_search_replica_reads_total`,
    /// captured from the `PostgresSearch` when a replica is configured. `None`
    /// otherwise, so the metric simply isn't emitted.
    pub search_read_routing_metrics: Option<std::sync::Arc<maidan_search::SearchReadMetrics>>,
    /// Operator Ed25519 seed for signed workspace export. `None` in
    /// [`AppState::new`] — tests attach a fixture key; the binary loads
    /// `MAIDAN_EXPORT_SIGNING_KEY`. Export refuses when unset (fail closed;
    /// never emit an unsigned bundle).
    pub export_signing: Option<maidan_auth::ExportSigningKey>,
    /// Optional public-key pin for import/verify (`MAIDAN_EXPORT_VERIFY_KEYS`).
    /// Empty = integrity against the embedded key only (blank-instance default).
    pub export_verify_keys: Vec<[u8; 32]>,
    /// Optional, advisory-only Jev land-gate scorer. `None` is the default and
    /// makes the advice route unavailable without changing gate writes.
    pub land_gate_advisor: Option<Arc<dyn crate::land_gate_advisor::LandGateAdvisor>>,
    /// Per-host bound on retries, shared by every outbound delivery worker in
    /// this process. Tests swap in one on a manual clock.
    pub retry_budget: Arc<crate::retry_budget::RetryBudget>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn EventBus>,
        search: Arc<dyn Search>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
        auth_disabled: bool,
        bootstrap_enabled: bool,
        federation: FederationRuntime,
        indexer_last_event_unix_ms: Arc<AtomicI64>,
        bus_listener_health: Option<Arc<ListenerHealth>>,
    ) -> Self {
        let presence = Arc::new(PresenceHub::default());
        let mcp = Arc::new(
            McpServer::new(
                store.clone(),
                artifacts.clone(),
                search.clone(),
                embedding_provider.clone(),
            )
            .with_event_bus(bus.clone()),
        );
        mcp.attach_presence_reader(presence.clone());
        let slash = SlashRuntime::new(None);
        let fsm_hooks = FsmHookRuntime::new(None);
        let slash_secrets = slash.secrets.clone();
        let fsm_secrets = fsm_hooks.secrets.clone();
        mcp.set_slash_secret_forget(Arc::new(move |id| {
            if let Ok(mut guard) = slash_secrets.write() {
                guard.remove(&SlashCommandId(id));
            }
        }));
        mcp.set_fsm_secret_forget(Arc::new(move |id| {
            if let Ok(mut guard) = fsm_secrets.write() {
                guard.remove(&FsmHookId(id));
            }
        }));
        Self {
            store,
            artifacts,
            bus,
            search,
            embedding_provider,
            mcp,
            auth_disabled,
            dev_anonymous_reader: None,
            test_identity_header: false,
            bootstrap_enabled,
            federation,
            webhooks: WebhookRuntime::new(None),
            slash,
            fsm_hooks,
            indexer_last_event_unix_ms,
            indexer_processed_log_id: Arc::new(AtomicI64::new(0)),
            indexer_last_error: Arc::new(AsyncRwLock::new(None)),
            bus_listener_health,
            bus_hydrate_stats: None,
            outbox_relay: false,
            outbox_nudge: None,
            outbox_backend: None,
            oidc: None,
            sessions: None,
            subscribe_resume_secret: None,
            subscribe_resume_ttl_secs: subscribe_resume::ttl_secs_from_env(),
            presence,
            ws_connections: Arc::default(),
            draining: Arc::default(),
            max_ws_connections: std::env::var("MAIDAN_MAX_WS_CONNECTIONS")
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .filter(|n: &usize| *n > 0)
                .unwrap_or(10_000),
            request_limit: crate::load_shed::RequestLimit::from_env(),
            rate_limit_redis: None,
            rate_limit_default_on: false,
            indexer_metrics: Arc::new(maidan_search::IndexerMetrics::default()),
            delivery_stability: crate::event_stream::reconcile_stability_window_from_env(),
            delivery_reconcile_interval: crate::event_stream::reconcile_interval_from_env(),
            mail: None,
            web_push: None,
            web_push_public_key: None,
            web_push_disabled_reason: Some("vapid_unset".into()),
            slack: None,
            slack_sender: None,
            github: None,
            github_sender: None,
            mark_ready_app_id: None,
            a2a_card: crate::a2a_agent::A2aCardConfig::default(),
            read_replica_enabled: false,
            read_routing_metrics: None,
            search_read_routing_metrics: None,
            export_signing: None,
            export_verify_keys: Vec::new(),
            land_gate_advisor: None,
            retry_budget: Arc::new(crate::retry_budget::RetryBudget::new()),
        }
    }

    pub fn attach_land_gate_advisor(
        &mut self,
        advisor: Arc<dyn crate::land_gate_advisor::LandGateAdvisor>,
    ) {
        self.mcp
            .set_land_gate_advisor(Arc::new(crate::land_gate_advisor::McpLandGateBridge(
                advisor.clone(),
            )));
        self.land_gate_advisor = Some(advisor);
    }

    /// Wire the operator export signing key. Called by the server binary when
    /// `MAIDAN_EXPORT_SIGNING_KEY` is set; tests attach a fixture.
    pub fn attach_export_signing(&mut self, key: maidan_auth::ExportSigningKey) {
        self.export_signing = Some(key);
    }

    /// Pin public keys that import/verify will accept.
    pub fn attach_export_verify_keys(&mut self, keys: Vec<[u8; 32]>) {
        self.export_verify_keys = keys;
    }

    /// Wire the Web Push sender. Called by the server binary when
    /// `MAIDAN_VAPID_*` is configured; left `None` (no web push) otherwise
    /// and in tests.
    pub fn attach_web_push(&mut self, sender: Arc<dyn crate::web_push::WebPushSender>) {
        self.web_push = Some(sender);
    }

    /// Publish the VAPID public key the board uses to subscribe.
    pub fn attach_web_push_public_key(&mut self, public_key: String) {
        self.web_push_public_key = Some(public_key);
        self.web_push_disabled_reason = None;
    }

    /// Record why Web Push is not sending. `reason` is `vapid_unset` or
    /// `vapid_invalid`.
    pub fn set_web_push_disabled(&mut self, reason: &str) {
        self.web_push_public_key = None;
        self.web_push = None;
        self.web_push_disabled_reason = Some(reason.to_string());
    }

    /// Wire the email transport. Called by the server binary when
    /// `MAIDAN_SMTP_*` is configured; left `None` (no email) otherwise and in
    /// tests.
    pub fn attach_mail(&mut self, transport: Arc<dyn crate::mail::MailTransport>) {
        self.mail = Some(transport);
    }

    /// Wire the Slack projector config. Called by the server binary when
    /// `MAIDAN_SLACK_SIGNING_SECRET` is set; left `None` (route disabled)
    /// otherwise and in tests.
    pub fn attach_slack(&mut self, cfg: Arc<crate::slack::SlackConfig>) {
        self.slack = Some(cfg);
    }

    /// Wire the Slack egress sender. Called by the server binary when
    /// `MAIDAN_SLACK_BOT_TOKEN` is set; tests inject a mock. `None` disables
    /// egress.
    pub fn attach_slack_sender(&mut self, sender: Arc<dyn crate::slack::SlackSender>) {
        self.slack_sender = Some(sender);
    }

    /// Wire the Git/GitHub projector config. Called by the server binary when
    /// `MAIDAN_GITHUB_WEBHOOK_SECRET` is set; left `None` otherwise.
    pub fn attach_github(&mut self, cfg: Arc<crate::github::GithubConfig>) {
        self.github = Some(cfg);
    }

    /// Wire the GitHub egress sender. Called by the server binary when
    /// `MAIDAN_GITHUB_TOKEN` is set; tests inject a mock. `None` disables
    /// egress.
    pub fn attach_github_sender(&mut self, sender: Arc<dyn crate::github::GithubSender>) {
        self.github_sender = Some(sender);
    }

    /// Wire cross-replica MCP resource-update notifications.
    ///
    /// Rebuilds the MCP dispatcher with `notifier` so `resources/subscribe` SSE
    /// updates reach subscribers on any replica. The caller must then call
    /// `state.mcp.spawn_resource_notify_listener()` (from an async context) so
    /// this process delivers cross-replica updates to its own SSE subscribers.
    pub fn attach_resource_notifier(&mut self, notifier: Arc<dyn ResourceNotifier>) {
        self.mcp = Arc::new(
            McpServer::new(
                self.store.clone(),
                self.artifacts.clone(),
                self.search.clone(),
                self.embedding_provider.clone(),
            )
            .with_event_bus(self.bus.clone())
            .with_resource_notifier(notifier),
        );
        self.mcp.attach_presence_reader(self.presence.clone());
    }

    /// Wire cross-replica presence/typing fan-out.
    ///
    /// Rebuilds the presence hub with `notifier` so presence, typing, and the
    /// roster stay consistent across replicas. The caller must then call
    /// `state.presence.spawn_tasks()` (from an async context) to start the
    /// listener + heartbeat.
    pub fn attach_presence_notifier(&mut self, notifier: Arc<dyn PresenceNotifier>) {
        self.presence = Arc::new(PresenceHub::default().with_presence_notifier(notifier));
        self.mcp.attach_presence_reader(self.presence.clone());
    }

    /// How browser sessions are signed and how long they last, or `None` when
    /// this deployment has none. OIDC's settings when OIDC is on (they come
    /// from the same `MAIDAN_SESSION_SECRET`), else [`Self::sessions`].
    pub fn browser_sessions(&self) -> Option<crate::session::SessionSettings> {
        if let Some(oidc) = &self.oidc {
            return Some(crate::session::SessionSettings {
                secret: oidc.session_secret.clone(),
                ttl_secs: oidc.settings.session_ttl_secs,
                cookie_secure: oidc.settings.cookie_secure,
            });
        }
        self.sessions.clone()
    }

    /// The key that signs subscribe-resume and approval-gate `requestState`
    /// tokens, or `None` when none is configured.
    ///
    /// This used to `panic!` on the `None` arm, on the grounds that `main.rs`
    /// establishes the invariant — and it does: all four of its branches either
    /// set a secret or refuse to boot. But `AppState::new` leaves the field
    /// `None`, so anything embedding this crate as a library could construct a
    /// state that aborts the process on a *request*. A panic on a getter is
    /// exactly what the repo's own "no panic in library code" rule is for, so
    /// the absence is now a value the caller handles — in practice a `500`,
    /// which is the honest answer to "this deployment cannot sign that token".
    pub fn subscribe_resume_secret(&self) -> Option<&[u8]> {
        if let Some(oidc) = &self.oidc {
            return Some(oidc.session_secret.as_ref());
        }
        self.subscribe_resume_secret.as_deref()
    }

    /// E2E harness: auth and federation disabled, fresh indexer heartbeat.
    pub fn for_tests(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn EventBus>,
        search: Arc<dyn Search>,
    ) -> Self {
        let mut state = Self::new(
            store,
            artifacts,
            bus,
            search,
            Arc::new(maidan_search::HashV1Provider),
            true,
            false,
            FederationRuntime::new(true, None),
            Arc::new(AtomicI64::new(0)),
            None,
        );
        state.test_identity_header = true;
        state.subscribe_resume_secret =
            Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
        state
    }
}
