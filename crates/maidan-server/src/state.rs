use std::sync::Arc;

use maidan_artifacts::ArtifactStore;
use maidan_bus::EventBus;
use maidan_store::Store;

/// Shared handles passed to every request handler. `Arc`s are cheap to
/// clone; the inner trait objects implement the relevant backend logic.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub bus: Arc<dyn EventBus>,
}

impl AppState {
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            store,
            artifacts,
            bus,
        }
    }
}
