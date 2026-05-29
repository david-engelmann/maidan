use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::{
    a2a_agent, auth, bootstrap, dm, federation, health, mcp, mcp_notifications, mcp_stream,
    mcp_streamable, metrics, oidc, openapi, rate_limit, request_id, routes, session,
    state::AppState, ws,
};

/// Build the axum [`Router`] with all routes wired up.
///
/// Tested in `tests/health_e2e.rs` by binding the router to a TCP port
/// and curling it; the binary's `main.rs` uses the same router.
pub fn router(state: AppState) -> Router {
    let bootstrap = Router::new()
        .route("/workspaces", post(routes::create_workspace))
        .route("/workspaces/:wid/members", post(routes::create_member))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bootstrap::middleware,
        ));

    let ws_only = Router::new().route("/ws/subscribe", get(ws::subscribe));

    let protected = Router::new()
        .route("/mcp", post(mcp::handler))
        .route("/mcp/streamable", post(mcp_streamable::streamable))
        .route("/a2a/v1/rpc", post(a2a_agent::json_rpc))
        .route("/mcp/notifications", get(mcp_notifications::stream))
        .route("/mcp/stream", get(mcp_stream::stream))
        .route("/workspaces/:id", get(routes::get_workspace))
        .route("/workspaces/:id/purge", post(routes::purge_workspace))
        .route("/workspaces/:id/audit", get(routes::list_workspace_audit))
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
            "/workspaces/:wid/dm",
            post(dm::open_dm_conversation).get(dm::list_dm_conversations),
        )
        .route("/dm/:id", get(dm::get_dm_conversation))
        .route(
            "/dm/:id/messages",
            post(dm::post_dm_message).get(dm::list_dm_messages),
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
            get(routes::get_message)
                .patch(routes::edit_message)
                .delete(routes::tombstone_message),
        )
        .route("/messages/:id/purge", delete(routes::purge_message))
        .route("/messages/:id/mentions", post(routes::create_mention))
        .route(
            "/messages/:id/votes",
            post(routes::cast_vote).get(routes::list_votes),
        )
        .route("/artifacts", post(routes::upload_artifact))
        .route(
            "/artifacts/multipart",
            post(routes::begin_multipart_artifact).delete(routes::abort_multipart_artifact),
        )
        .route(
            "/artifacts/multipart/:upload_id/complete",
            post(routes::complete_multipart_artifact),
        )
        .route(
            "/artifacts/multipart/:upload_id/parts/:part_number",
            put(routes::upload_multipart_artifact_part),
        )
        .route("/artifacts/:sha", get(routes::get_artifact))
        .route("/artifacts/:sha/meta", get(routes::get_artifact_metadata))
        .route(
            "/references",
            post(routes::create_reference).get(routes::list_references),
        )
        .route("/tokens/:id", delete(routes::revoke_api_token))
        .route(
            "/workspaces/:wid/peers",
            post(federation::create_peer).get(federation::list_peers),
        )
        .route(
            "/workspaces/:wid/peers/:pid",
            delete(federation::delete_peer),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware,
        ))
        .layer(TraceLayer::new_for_http());

    let a2a = Router::new()
        .route("/a2a/v1/events", post(federation::ingest_events))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            federation::peer_auth_middleware,
        ));

    async fn ui_index() -> axum::response::Html<&'static str> {
        axum::response::Html(include_str!("../static/index.html"))
    }

    let session_auth = middleware::from_fn_with_state(state.clone(), session::require_middleware);

    let auth_routes = Router::new()
        .route("/auth/oidc/login", get(oidc::login))
        .route("/auth/oidc/callback", get(oidc::callback))
        .route("/auth/logout", post(oidc::logout))
        .route(
            "/auth/session",
            get(session::get_session).layer(session_auth.clone()),
        )
        .route(
            "/auth/session/mint",
            post(session::mint_first_admin_token).layer(session_auth),
        );

    let ui_api = Router::new()
        .route("/ui/api/workspaces/:wid/events", get(routes::list_events))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session_or_bearer_middleware,
        ));

    Router::new()
        .route("/health", get(health::handler))
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .route("/.well-known/maidan.json", get(federation::well_known))
        .route("/openapi.json", get(openapi::openapi_json))
        .route("/metrics", get(metrics::scrape))
        .route("/ui", get(ui_index))
        .route("/ui/", get(ui_index))
        .merge(bootstrap)
        .merge(auth_routes)
        .merge(ui_api)
        .merge(ws_only)
        .merge(a2a)
        .merge(protected)
        .layer(middleware::from_fn(metrics::middleware))
        .layer(middleware::from_fn(rate_limit::middleware))
        .layer(middleware::from_fn(request_id::middleware))
        .with_state(state)
}
