use axum::{
    middleware,
    routing::{delete, get, post},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::{auth, health, mcp, routes, state::AppState, ws};

/// Build the axum [`Router`] with all routes wired up.
///
/// Tested in `tests/health_e2e.rs` by binding the router to a TCP port
/// and curling it; the binary's `main.rs` uses the same router.
pub fn router(state: AppState) -> Router {
    let bootstrap = Router::new()
        .route("/workspaces", post(routes::create_workspace))
        .route("/workspaces/:wid/members", post(routes::create_member));

    let ws_only = Router::new().route("/ws/subscribe", get(ws::subscribe));

    let protected = Router::new()
        .route("/mcp", post(mcp::handler))
        .route("/workspaces/:id", get(routes::get_workspace))
        .route("/workspaces/:wid/events", get(routes::list_events))
        .route("/workspaces/:wid/search", get(routes::search_messages))
        .route("/workspaces/:wid/members", get(routes::list_members))
        .route(
            "/workspaces/:wid/members/:mid/tokens",
            post(routes::mint_api_token),
        )
        .route("/members/:id", get(routes::get_member))
        .route(
            "/members/:id/mentions",
            get(routes::list_mentions_for_member),
        )
        .route(
            "/workspaces/:wid/channels",
            post(routes::create_channel).get(routes::list_channels),
        )
        .route("/channels/:id", get(routes::get_channel))
        .route(
            "/channels/:cid/threads",
            post(routes::create_thread).get(routes::list_threads),
        )
        .route(
            "/threads/:id",
            get(routes::get_thread).post(routes::transition_thread),
        )
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
        .route("/artifacts", post(routes::upload_artifact))
        .route("/artifacts/:sha", get(routes::get_artifact))
        .route("/artifacts/:sha/meta", get(routes::get_artifact_metadata))
        .route(
            "/references",
            post(routes::create_reference).get(routes::list_references),
        )
        .route("/tokens/:id", delete(routes::revoke_api_token))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware,
        ))
        .layer(TraceLayer::new_for_http());

    Router::new()
        .route("/health", get(health::handler))
        .merge(bootstrap)
        .merge(ws_only)
        .merge(protected)
        .with_state(state)
}
