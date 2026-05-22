use axum::{
    routing::{get, post},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::{health, routes, state::AppState};

/// Build the axum [`Router`] with all routes wired up.
///
/// Tested in `tests/health_e2e.rs` by binding the router to a TCP port
/// and curling it; the binary's `main.rs` uses the same router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::handler))
        // workspaces
        .route("/workspaces", post(routes::create_workspace))
        .route("/workspaces/:id", get(routes::get_workspace))
        // members
        .route(
            "/workspaces/:wid/members",
            post(routes::create_member).get(routes::list_members),
        )
        .route("/members/:id", get(routes::get_member))
        .route(
            "/members/:id/mentions",
            get(routes::list_mentions_for_member),
        )
        // channels
        .route(
            "/workspaces/:wid/channels",
            post(routes::create_channel).get(routes::list_channels),
        )
        .route("/channels/:id", get(routes::get_channel))
        // threads
        .route(
            "/channels/:cid/threads",
            post(routes::create_thread).get(routes::list_threads),
        )
        .route("/threads/:id", get(routes::get_thread))
        // messages
        .route(
            "/threads/:tid/messages",
            post(routes::post_message).get(routes::list_messages),
        )
        .route(
            "/messages/:id",
            get(routes::get_message).delete(routes::tombstone_message),
        )
        .route("/messages/:id/mentions", post(routes::create_mention))
        .route(
            "/messages/:id/votes",
            post(routes::cast_vote).get(routes::list_votes),
        )
        // references
        .route(
            "/references",
            post(routes::create_reference).get(routes::list_references),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
