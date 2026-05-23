use std::sync::Arc;

use maidan_artifacts::ArtifactStore;
use maidan_bus::EventBus;
use maidan_search::Search;
use maidan_store::Store;

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
}

impl AppState {
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn EventBus>,
        search: Arc<dyn Search>,
        auth_disabled: bool,
    ) -> Self {
        Self {
            store,
            artifacts,
            bus,
            search,
            auth_disabled,
        }
    }
}
