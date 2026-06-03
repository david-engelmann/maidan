use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::{
    a2a_agent, app_oauth, apps, auth, automation_deliveries, bootstrap, delivery_ops, dm,
    federation, fsm_hooks, health, mcp, mcp_notifications, mcp_stream, mcp_streamable, metrics,
    oidc, openapi, quota, rate_limit, reindex_ops, request_id, routes, session, slash_commands,
    state::AppState, webhooks, ws,
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
        .route("/mcp/streamable", delete(mcp_streamable::close_session))
        .route("/a2a/v1/rpc", post(a2a_agent::json_rpc))
        .route("/mcp/notifications", get(mcp_notifications::stream))
        .route("/mcp/stream", get(mcp_stream::stream))
        .route("/workspaces/:id", get(routes::get_workspace))
        .route("/workspaces/:id/purge", post(routes::purge_workspace))
        .route("/workspaces/:id", delete(routes::erase_workspace))
        .route("/workspaces/:id/audit", get(routes::list_workspace_audit))
        .route("/workspaces/:wid/events", get(routes::list_events))
        .route(
            "/workspaces/:wid/outbox/:oid/replay",
            post(routes::replay_quarantined_outbox),
        )
        .route(
            "/workspaces/:wid/outbox/quarantined",
            get(routes::list_quarantined_outbox),
        )
        .route(
            "/workspaces/:wid/context",
            get(routes::get_workspace_context),
        )
        .route("/workspaces/:wid/search", get(routes::search_messages))
        .route("/workspaces/:wid/members", get(routes::list_members))
        .route(
            "/workspaces/:wid/members/:mid/tokens",
            post(routes::mint_api_token),
        )
        .route(
            "/workspaces/:wid/apps",
            post(apps::register_app).get(apps::list_apps),
        )
        .route(
            "/workspaces/:wid/apps/:app_id/install",
            post(apps::install_app),
        )
        .route(
            "/workspaces/:wid/apps/:app_id/oauth/authorize",
            post(app_oauth::authorize_app_install),
        )
        .route(
            "/workspaces/:wid/app-installations",
            get(apps::list_app_installations),
        )
        .route(
            "/workspaces/:wid/app-installations/:iid",
            delete(apps::revoke_app_installation),
        )
        .route(
            "/workspaces/:wid/app-installations/:iid/tokens",
            post(apps::mint_app_token),
        )
        .route("/members/:id", get(routes::get_member))
        .route(
            "/members/:id/mentions",
            get(routes::list_mentions_for_member),
        )
        .route("/members/:id/inbox", get(routes::get_member_inbox))
        .route(
            "/members/:id/inbox/read",
            post(routes::mark_member_inbox_read),
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
        .route("/threads/:id/context", get(routes::get_thread_context))
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
        .route("/messages/:id/edits", get(routes::list_message_edits))
        .route("/messages/:id/purge", delete(routes::purge_message))
        .route("/messages/:id/mentions", post(routes::create_mention))
        .route(
            "/messages/:id/votes",
            post(routes::cast_vote).get(routes::list_votes),
        )
        .route(
            "/messages/:id/reactions",
            post(routes::add_reaction)
                .get(routes::list_reactions)
                .delete(routes::remove_reaction),
        )
        .route(
            "/threads/:id/pins",
            post(routes::pin_message)
                .get(routes::list_pins)
                .delete(routes::unpin_message),
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
        .route(
            "/workspaces/:wid/webhooks",
            post(webhooks::create_webhook).get(webhooks::list_webhooks),
        )
        .route(
            "/workspaces/:wid/webhooks/:whid",
            delete(webhooks::revoke_webhook),
        )
        .route(
            "/workspaces/:wid/slash-commands",
            post(slash_commands::create_slash_command).get(slash_commands::list_slash_commands),
        )
        .route(
            "/workspaces/:wid/slash-commands/:cid",
            delete(slash_commands::revoke_slash_command),
        )
        .route(
            "/workspaces/:wid/fsm-hooks",
            post(fsm_hooks::create_fsm_hook).get(fsm_hooks::list_fsm_hooks),
        )
        .route(
            "/workspaces/:wid/fsm-hooks/:hid",
            delete(fsm_hooks::revoke_fsm_hook),
        )
        .route(
            "/workspaces/:wid/deliveries",
            get(delivery_ops::list_deliveries),
        )
        .route(
            "/workspaces/:wid/deliveries/:did",
            get(delivery_ops::get_delivery),
        )
        .route(
            "/workspaces/:wid/deliveries/:did/replay",
            post(delivery_ops::replay_delivery),
        )
        .route(
            "/operator/reindex-embeddings",
            post(reindex_ops::start_reindex_embeddings),
        )
        .route(
            "/operator/reindex-embeddings/:job_id",
            get(reindex_ops::get_reindex_embeddings_job),
        )
        .route(
            "/workspaces/:wid/automation/dlq",
            get(automation_deliveries::list_quarantined_automation_deliveries),
        )
        .route(
            "/workspaces/:wid/automation/deliveries",
            get(automation_deliveries::list_automation_deliveries),
        )
        .route(
            "/workspaces/:wid/automation/deliveries/:did",
            get(automation_deliveries::get_automation_delivery),
        )
        .route(
            "/workspaces/:wid/automation/deliveries/:did/replay",
            post(automation_deliveries::replay_automation_delivery),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            quota::middleware,
        ))
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
        .route(
            "/ui/api/workspaces/:wid/channels",
            get(routes::list_channels),
        )
        .route("/ui/api/channels/:cid/threads", get(routes::list_threads))
        .route("/ui/api/threads/:tid/messages", get(routes::list_messages))
        .route(
            "/ui/api/workspaces/:wid/search",
            get(routes::search_messages),
        )
        .route(
            "/ui/api/workspaces/:wid/audit",
            get(routes::list_workspace_audit),
        )
        .route("/ui/api/workspaces/:wid/peers", get(federation::list_peers))
        .route(
            "/ui/api/messages/:mid/edits",
            get(routes::list_message_edits),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session_or_bearer_middleware,
        ));

    Router::new()
        .route("/health", get(health::handler))
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .route("/.well-known/maidan.json", get(federation::well_known))
        .route("/.well-known/agent-card.json", get(a2a_agent::agent_card))
        .route("/oauth/app/token", post(app_oauth::exchange_app_code))
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::middleware,
        ))
        .layer(middleware::from_fn(request_id::middleware))
        .with_state(state)
}
