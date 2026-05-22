use axum::{routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::{health, state::AppState};

/// Build the axum [`Router`] with all routes wired up.
///
/// Tested in `tests/health_e2e.rs` by binding the router to a TCP port
/// and curling it; the binary's `main.rs` uses the same router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
