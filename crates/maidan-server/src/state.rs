use std::collections::HashMap;
use std::sync::{atomic::AtomicI64, Arc, RwLock};

use maidan_artifacts::ArtifactStore;
use maidan_bus::{EventBus, HydrateStats, ListenerHealth};
use maidan_search::{EmbeddingProvider, Search};
use maidan_store::Store;
use maidan_types::PeerId;
use tokio::sync::RwLock as AsyncRwLock;

use crate::oidc::OidcRuntime;
use crate::subscribe_resume;

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
    /// When true, all routes accept requests without a bearer token.
    pub auth_disabled: bool,
    /// When true, unauthenticated bootstrap routes are allowed (see `MAIDAN_BOOTSTRAP`).
    pub bootstrap_enabled: bool,
    pub federation: FederationRuntime,
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
    /// Postgres pool for outbox relay metrics; `None` on SQLite.
    pub outbox_pool: Option<sqlx::PgPool>,
    /// OIDC client + settings when `MAIDAN_OIDC_ENABLED=1`.
    pub oidc: Option<Arc<OidcRuntime>>,
    /// HMAC secret for subscribe resume tokens (when OIDC is off).
    pub subscribe_resume_secret: Option<Arc<[u8]>>,
    /// TTL for signed resume tokens (seconds).
    pub subscribe_resume_ttl_secs: u64,
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
        Self {
            store,
            artifacts,
            bus,
            search,
            embedding_provider,
            auth_disabled,
            bootstrap_enabled,
            federation,
            indexer_last_event_unix_ms,
            indexer_last_error: Arc::new(AsyncRwLock::new(None)),
            bus_listener_health,
            bus_hydrate_stats: None,
            outbox_relay: false,
            outbox_pool: None,
            oidc: None,
            subscribe_resume_secret: None,
            subscribe_resume_ttl_secs: subscribe_resume::ttl_secs_from_env(),
        }
    }

    pub fn subscribe_resume_secret(&self) -> &[u8] {
        if let Some(oidc) = &self.oidc {
            return oidc.session_secret.as_ref();
        }
        self.subscribe_resume_secret
            .as_deref()
            .expect("subscribe resume secret must be configured")
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
