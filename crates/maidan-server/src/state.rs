use std::collections::HashMap;
use std::sync::{atomic::AtomicI64, Arc, RwLock};

use maidan_artifacts::ArtifactStore;
use maidan_bus::EventBus;
use maidan_search::Search;
use maidan_store::Store;
use maidan_types::PeerId;

/// Shared handles passed to every request handler. `Arc`s are cheap to
/// clone; the inner trait objects implement the relevant backend logic.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub bus: Arc<dyn EventBus>,
    pub search: Arc<dyn Search>,
    /// When true, all routes accept requests without a bearer token.
    pub auth_disabled: bool,
    /// When true, the federation poll worker is not started.
    pub federation_disabled: bool,
    /// Peer bearer secrets shown once at create; used for outbound poll until restart.
    pub federation_secrets: Arc<RwLock<HashMap<PeerId, String>>>,
    /// Milliseconds since Unix epoch when the indexer last handled an event (0 = never).
    pub indexer_last_event_unix_ms: Arc<AtomicI64>,
}

impl AppState {
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn EventBus>,
        search: Arc<dyn Search>,
        auth_disabled: bool,
        federation_disabled: bool,
        indexer_last_event_unix_ms: Arc<AtomicI64>,
    ) -> Self {
        Self {
            store,
            artifacts,
            bus,
            search,
            auth_disabled,
            federation_disabled,
            federation_secrets: Arc::new(RwLock::new(HashMap::new())),
            indexer_last_event_unix_ms,
        }
    }

    /// E2E harness: auth and federation disabled, fresh indexer heartbeat.
    pub fn for_tests(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn EventBus>,
        search: Arc<dyn Search>,
    ) -> Self {
        Self::new(
            store,
            artifacts,
            bus,
            search,
            true,
            true,
            Arc::new(AtomicI64::new(0)),
        )
    }
}
