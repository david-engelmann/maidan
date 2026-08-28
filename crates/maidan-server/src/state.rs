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
    /// When true, unauthenticated bootstrap routes are allowed (see `MAIDAN_BOOTSTRAP`).
    pub bootstrap_enabled: bool,
    pub federation: FederationRuntime,
    pub webhooks: WebhookRuntime,
    pub slash: SlashRuntime,
    pub fsm_hooks: FsmHookRuntime,
    /// Milliseconds since Unix epoch when the indexer last handled an event (0 = never).
    pub indexer_last_event_unix_ms: Arc<AtomicI64>,
    /// Most recent indexer-side embedding failure, if any.
    pub indexer_last_error: Arc<AsyncRwLock<Option<String>>>,
    /// Postgres `LISTEN` task health; `None` when using [`maidan_bus::InMemoryBus`].
    pub bus_listener_health: Option<Arc<ListenerHealth>>,
    /// Postgres NOTIFY hydrate outcomes; `None` when using [`maidan_bus::InMemoryBus`].
    pub bus_hydrate_stats: Option<Arc<HydrateStats>>,
    /// When true, `publish` enqueues outbox only; [`crate::outbox_relay`] calls `bus.publish`.
    pub outbox_relay: bool,
    /// Capacity-1 nudge to the outbox relay: a freshly enqueued row wakes an
    /// idle relay promptly (Cluster 108). `None` when relay/nudge isn't wired.
    pub outbox_nudge: Option<tokio::sync::mpsc::Sender<()>>,
    /// Outbox backend for relay metrics; `None` when outbox relay is disabled.
    pub outbox_backend: Option<OutboxBackend>,
    /// OIDC client + settings when `MAIDAN_OIDC_ENABLED=1`.
    pub oidc: Option<Arc<OidcRuntime>>,
    /// HMAC secret for subscribe resume tokens (when OIDC is off).
    pub subscribe_resume_secret: Option<Arc<[u8]>>,
    /// TTL for signed resume tokens (seconds).
    pub subscribe_resume_ttl_secs: u64,
    /// Ephemeral presence/typing fan-out for WebSocket subscribers.
    pub presence: Arc<PresenceHub>,
    /// Optional Redis backend for global and per-token rate limits (Cluster 54).
    pub rate_limit_redis: Option<redis::aio::ConnectionManager>,
    /// Apply a built-in global per-client rate limit when `MAIDAN_RATE_LIMIT_MAX`
    /// is unset (Cluster 183). The server bootstrap turns this on so a deployment
    /// that configures nothing still has a DoS floor; an explicit
    /// `MAIDAN_RATE_LIMIT_MAX` (including `0` to disable) always overrides. Left
    /// `false` in [`AppState::new`] so tests are unaffected unless they opt in.
    pub rate_limit_default_on: bool,
    /// Live embedding-indexer counters (queue depth, throughput) for metrics
    /// (Cluster 116). Default-zeroed unless the batching indexer is wired.
    pub indexer_metrics: Arc<maidan_search::IndexerMetrics>,
    /// At-least-once delivery (Cluster 125): stability window + reconcile poll
    /// cadence for `at_least_once` subscriptions. Read from env once at startup.
    pub delivery_stability: std::time::Duration,
    pub delivery_reconcile_interval: std::time::Duration,
    /// Off-platform email transport (Cluster 249), built from `MAIDAN_SMTP_*` at
    /// startup. `None` when SMTP isn't configured — the config gate: no transport,
    /// no email. Set only by the server binary via [`AppState::attach_mail`], so
    /// tests/embedders (which build via [`AppState::new`]) never send email.
    pub mail: Option<Arc<dyn crate::mail::MailTransport>>,
    /// Slack projector config (Cluster 307), built from `MAIDAN_SLACK_*` at startup.
    /// `None` when unset — the config gate: the `/integrations/slack/events` route
    /// then returns `404`. Set only by the server binary via [`AppState::attach_slack`].
    pub slack: Option<Arc<crate::slack::SlackConfig>>,
    /// Slack projector egress sender (Cluster 309), a `chat.postMessage` client set
    /// when `MAIDAN_SLACK_BOT_TOKEN` is configured. `None` disables egress (inbound
    /// still works). Set via [`AppState::attach_slack_sender`]; tests inject a mock.
    pub slack_sender: Option<Arc<dyn crate::slack::SlackSender>>,
    /// Git/GitHub projector config (Cluster 310), built from `MAIDAN_GITHUB_*` at
    /// startup. `None` when unset — the `/integrations/github/events` route then
    /// returns `404`. Set via [`AppState::attach_github`].
    pub github: Option<Arc<crate::github::GithubConfig>>,
    /// A2A Agent Card transport advertisement config (Cluster 288): public origin
    /// for absolute interface URLs + the advertised gRPC address. Default empty
    /// (host-relative URLs, no gRPC interface); the server binary sets it from env.
    pub a2a_card: crate::a2a_agent::A2aCardConfig,
    /// A read replica is configured (`MAIDAN_DB_REPLICA_URL`), so the server should
    /// stamp a `Maidan-Consistency-Token` on writes and route replica-eligible
    /// reads (Cluster 263+). Left `false` in [`AppState::new`]; the server binary
    /// sets it when it builds the store with a replica reader, so tests/embedders
    /// (no replica) skip the token round-trip and behave exactly as before.
    pub read_replica_enabled: bool,
    /// Read-routing counters (Cluster 265) for `maidan_replica_reads_total`, captured
    /// from the `PostgresStore` when a replica is configured. `None` otherwise (SQLite
    /// / single-pool), so the metric simply isn't emitted.
    pub read_routing_metrics: Option<std::sync::Arc<maidan_store::postgres::ReadRoutingMetrics>>,
    /// Search read-routing counters (Cluster 272) for `maidan_search_replica_reads_total`,
    /// captured from the `PostgresSearch` when a replica is configured. `None` otherwise,
    /// so the metric simply isn't emitted.
    pub search_read_routing_metrics: Option<std::sync::Arc<maidan_search::SearchReadMetrics>>,
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
        let mcp = Arc::new(McpServer::new(
            store.clone(),
            artifacts.clone(),
            search.clone(),
            embedding_provider.clone(),
        ));
        Self {
            store,
            artifacts,
            bus,
            search,
            embedding_provider,
            mcp,
            auth_disabled,
            bootstrap_enabled,
            federation,
            webhooks: WebhookRuntime::new(None),
            slash: SlashRuntime::new(None),
            fsm_hooks: FsmHookRuntime::new(None),
            indexer_last_event_unix_ms,
            indexer_last_error: Arc::new(AsyncRwLock::new(None)),
            bus_listener_health,
            bus_hydrate_stats: None,
            outbox_relay: false,
            outbox_nudge: None,
            outbox_backend: None,
            oidc: None,
            subscribe_resume_secret: None,
            subscribe_resume_ttl_secs: subscribe_resume::ttl_secs_from_env(),
            presence: Arc::new(PresenceHub::default()),
            rate_limit_redis: None,
            rate_limit_default_on: false,
            indexer_metrics: Arc::new(maidan_search::IndexerMetrics::default()),
            delivery_stability: crate::event_stream::reconcile_stability_window_from_env(),
            delivery_reconcile_interval: crate::event_stream::reconcile_interval_from_env(),
            mail: None,
            slack: None,
            slack_sender: None,
            github: None,
            a2a_card: crate::a2a_agent::A2aCardConfig::default(),
            read_replica_enabled: false,
            read_routing_metrics: None,
            search_read_routing_metrics: None,
        }
    }

    /// Wire the email transport (Cluster 249). Called by the server binary when
    /// `MAIDAN_SMTP_*` is configured; left `None` (no email) otherwise and in tests.
    pub fn attach_mail(&mut self, transport: Arc<dyn crate::mail::MailTransport>) {
        self.mail = Some(transport);
    }

    /// Wire the Slack projector config (Cluster 307). Called by the server binary
    /// when `MAIDAN_SLACK_SIGNING_SECRET` is set; left `None` (route disabled)
    /// otherwise and in tests.
    pub fn attach_slack(&mut self, cfg: Arc<crate::slack::SlackConfig>) {
        self.slack = Some(cfg);
    }

    /// Wire the Slack egress sender (Cluster 309). Called by the server binary when
    /// `MAIDAN_SLACK_BOT_TOKEN` is set; tests inject a mock. `None` disables egress.
    pub fn attach_slack_sender(&mut self, sender: Arc<dyn crate::slack::SlackSender>) {
        self.slack_sender = Some(sender);
    }

    /// Wire the Git/GitHub projector config (Cluster 310). Called by the server
    /// binary when `MAIDAN_GITHUB_WEBHOOK_SECRET` is set; left `None` otherwise.
    pub fn attach_github(&mut self, cfg: Arc<crate::github::GithubConfig>) {
        self.github = Some(cfg);
    }

    /// Wire cross-replica MCP resource-update notifications (Cluster 102).
    ///
    /// Rebuilds the MCP dispatcher with `notifier` so `resources/subscribe`
    /// SSE updates reach subscribers on any replica. The caller must then call
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
            .with_resource_notifier(notifier),
        );
    }

    /// Wire cross-replica presence/typing fan-out (Cluster 103).
    ///
    /// Rebuilds the presence hub with `notifier` so presence, typing, and the
    /// roster stay consistent across replicas. The caller must then call
    /// `state.presence.spawn_tasks()` (from an async context) to start the
    /// listener + heartbeat.
    pub fn attach_presence_notifier(&mut self, notifier: Arc<dyn PresenceNotifier>) {
        self.presence = Arc::new(PresenceHub::default().with_presence_notifier(notifier));
    }

    pub fn subscribe_resume_secret(&self) -> &[u8] {
        if let Some(oidc) = &self.oidc {
            return oidc.session_secret.as_ref();
        }
        match self.subscribe_resume_secret.as_deref() {
            Some(secret) => secret,
            // Invariant established at construction; no Result to thread here.
            None => panic!("subscribe resume secret must be configured"),
        }
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
        state.subscribe_resume_secret =
            Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
        state
    }
}
