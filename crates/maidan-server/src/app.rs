use axum::{extract::DefaultBodyLimit, middleware, Router};

/// Default request body-size cap: 2 MiB. Matches axum's implicit extractor
/// default, but now explicit + tunable via `MAIDAN_MAX_BODY_BYTES` — a
/// deployment can tighten it (smaller JSON payloads) or widen it (larger
/// single-shot artifacts) without a rebuild.
const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

fn parse_max_body_bytes(raw: Option<String>) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_BODY_BYTES)
}

/// Resolve the request body-size cap from `MAIDAN_MAX_BODY_BYTES`.
pub fn max_body_bytes_from_env() -> usize {
    parse_max_body_bytes(std::env::var("MAIDAN_MAX_BODY_BYTES").ok())
}

#[cfg(feature = "bootstrap")]
use crate::bootstrap;
use crate::{
    a2a_agent, agui_stream, app_oauth, apps, auth, automation_deliveries, consistency,
    delivery_ops, dm, federation, fsm_hooks, github, group_dm, health, load_shed, mcp,
    mcp_notifications, mcp_stream, mcp_streamable, metrics, oidc, openapi, panic_guard, quota,
    rate_limit, reindex_ops, request_id, room_lsn, routes,
    routing::{delete, get, patch, post, put, ProblemFallbacks},
    scim, scim_groups, session, share_consumer, slack, slash_commands,
    state::AppState,
    trace_context, trace_redaction, webhooks, ws,
};

/// Content-Security-Policy for the board (`/ui`, `/ui/`) and `/ui/static/*`.
///
/// The document loads one script, `/ui/static/main.js`, which imports the
/// other modules beside it. Nothing on the page is an inline script and the
/// modules do not call `eval`, so `script-src` is `'self'` with no
/// `'unsafe-inline'`. Styles are `/ui/static/board.css` only: the mint banner
/// is shown with a class, and avatar hues are classes, because `style-src`
/// does not allow `'unsafe-inline'`.
///
/// `img-src` allows `data:` (the favicon) and `blob:` (raster attachment
/// previews). `connect-src` is `'self'`, `https:` and `wss:`. The API base
/// defaults to this page's origin, and `'self'` already covers that origin's
/// `http` and `ws`; another host is reached over TLS. Sign-out posts to that
/// same default origin, which is what `form-action 'self'` allows.
pub const BOARD_UI_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self' https: wss:; font-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

/// Build the axum [`Router`] with all routes wired up.
///
/// Tested in `tests/health_e2e.rs` by binding the router to a TCP port
/// and curling it; the binary's `main.rs` uses the same router.
pub fn router(state: AppState) -> Router {
    #[cfg(feature = "bootstrap")]
    let bootstrap = Router::new()
        .route("/workspaces", post(routes::create_workspace))
        .route("/workspaces/{wid}/members", post(routes::create_member))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bootstrap::middleware,
        ));

    let ws_only = Router::new().route("/ws/subscribe", get(ws::subscribe));

    let protected = Router::new()
        .route("/mcp", post(mcp::handler))
        .route("/mcp/worker", post(mcp::worker))
        .route("/mcp/reviewer", post(mcp::reviewer))
        .route("/mcp/streamable", post(mcp_streamable::streamable))
        .route("/mcp/streamable", get(mcp_streamable::stream_get))
        .route("/mcp/streamable", delete(mcp_streamable::close_session))
        // A2A v1.0: the JSON-RPC binding (§9), with and without the trailing
        // slash clients append to the advertised URL, and the HTTP+JSON
        // binding (§11).
        .route("/a2a/v1/rpc", post(a2a_agent::json_rpc))
        .route("/a2a/v1/rpc/", post(a2a_agent::json_rpc))
        // `message:send` and `message:stream`: the router captures the
        // `:method` suffix after the static `message` prefix.
        .route("/a2a/v1/message{method}", post(a2a_agent::rest_message))
        .route("/a2a/v1/tasks", get(a2a_agent::rest_list_tasks))
        .route(
            "/a2a/v1/tasks/{id}",
            get(a2a_agent::rest_task_get).merge(post(a2a_agent::rest_task_post)),
        )
        .route(
            "/a2a/v1/tasks/{id}/pushNotificationConfigs",
            post(a2a_agent::rest_create_push_config).merge(get(a2a_agent::rest_list_push_configs)),
        )
        .route(
            "/a2a/v1/tasks/{id}/pushNotificationConfigs/{config_id}",
            get(a2a_agent::rest_get_push_config).merge(delete(a2a_agent::rest_delete_push_config)),
        )
        .route(
            "/a2a/v1/extendedAgentCard",
            get(a2a_agent::rest_extended_agent_card),
        )
        .route("/mcp/notifications", get(mcp_notifications::stream))
        .route("/mcp/stream", get(mcp_stream::stream))
        .route("/agui/stream", get(agui_stream::stream))
        .route("/workspaces/{id}", get(routes::get_workspace))
        .route("/workspaces/{id}", patch(routes::rename_workspace))
        .route("/workspaces/{id}/purge", post(routes::purge_workspace))
        .route("/workspaces/{id}", delete(routes::erase_workspace))
        .route("/workspaces/{id}/audit", get(routes::list_workspace_audit))
        .route("/workspaces/{id}/export", get(routes::export_workspace))
        .route(
            "/workspaces/export/verify",
            post(routes::verify_workspace_export),
        )
        .route("/workspaces/import", post(routes::import_workspace))
        .route("/workspaces/{id}/usage", get(routes::get_workspace_usage))
        .route(
            "/workspaces/{id}/usage-rollup",
            get(routes::get_workspace_usage_rollup),
        )
        .route(
            "/workspaces/{id}/tombstones",
            get(routes::list_workspace_tombstones),
        )
        .route(
            "/workspaces/{id}/kind-census",
            get(routes::get_workspace_kind_census),
        )
        .route(
            "/workspaces/{id}/results",
            get(routes::list_workspace_results),
        )
        .route(
            "/workspaces/{id}/run-threads",
            get(routes::list_run_threads),
        )
        .route(
            "/workspaces/{id}/run-occupancy",
            get(routes::get_run_occupancy),
        )
        .route(
            "/workspaces/{id}/legal-holds",
            post(routes::place_legal_hold).merge(get(routes::list_workspace_legal_holds)),
        )
        .route(
            "/workspaces/{id}/legal-holds/preserved",
            get(routes::get_preserved_messages),
        )
        .route(
            "/workspaces/{id}/legal-holds/{hold_id}",
            delete(routes::lift_legal_hold),
        )
        // SCIM 2.0 provisioning — outside OpenAPI/capability-map (like /mcp);
        // each handler enforces token:admin inline and scopes to the token's
        // workspace.
        .route(
            "/scim/v2/ServiceProviderConfig",
            get(scim::service_provider_config),
        )
        .route(
            "/scim/v2/Users",
            post(scim::create_user).merge(get(scim::list_users)),
        )
        .route(
            "/scim/v2/Users/{id}",
            get(scim::get_user)
                .merge(put(scim::replace_user))
                .merge(patch(scim::patch_user))
                .merge(delete(scim::delete_user)),
        )
        .route(
            "/scim/v2/Groups",
            post(scim_groups::create_group).merge(get(scim_groups::list_groups)),
        )
        .route(
            "/scim/v2/Groups/{id}",
            get(scim_groups::get_group)
                .merge(put(scim_groups::replace_group))
                .merge(patch(scim_groups::patch_group))
                .merge(delete(scim_groups::delete_group)),
        )
        .route(
            "/workspaces/{id}/wip-limit",
            put(routes::set_wip_limit).merge(get(routes::get_wip_limit)),
        )
        .route(
            "/workspaces/{id}/delegation-policy",
            put(routes::set_delegation_policy).merge(get(routes::get_delegation_policy)),
        )
        .route(
            "/workspaces/{id}/retention",
            put(routes::set_retention_policy).merge(get(routes::get_retention_policy)),
        )
        .route(
            "/workspaces/{id}/spawn-budget",
            put(routes::set_spawn_budget).merge(get(routes::get_spawn_budget)),
        )
        .route("/me", get(routes::get_me))
        .route(
            "/workspaces/{wid}/glossary",
            get(routes::list_glossary_terms),
        )
        .route(
            "/workspaces/{wid}/glossary/{term}",
            put(routes::set_glossary_term)
                .merge(get(routes::get_glossary_term))
                .merge(delete(routes::delete_glossary_term)),
        )
        .route("/workspaces/{wid}/events", get(routes::list_events))
        .route(
            "/workspaces/{wid}/events/verify",
            get(routes::verify_event_chain),
        )
        .route(
            "/workspaces/{wid}/events/catch-up",
            get(routes::catch_up_events),
        )
        .route("/workspaces/{wid}/snapshot", get(routes::get_log_snapshot))
        .route(
            "/workspaces/{wid}/outbox/{oid}/replay",
            post(routes::replay_quarantined_outbox),
        )
        .route(
            "/workspaces/{wid}/outbox/quarantined",
            get(routes::list_quarantined_outbox),
        )
        .route(
            "/workspaces/{wid}/context",
            get(routes::get_workspace_context),
        )
        .route("/workspaces/{wid}/search", get(routes::search_messages))
        .route("/workspaces/{wid}/members", get(routes::list_members))
        .route(
            "/workspaces/{wid}/members/{mid}/tokens",
            post(routes::mint_api_token).merge(get(routes::list_api_tokens)),
        )
        .route(
            "/workspaces/{wid}/delegation-grants",
            post(routes::create_delegation_grant).merge(get(routes::list_delegation_grants)),
        )
        .route(
            "/workspaces/{wid}/delegation-grants/{gid}",
            delete(routes::revoke_delegation_grant),
        )
        .route(
            "/workspaces/{wid}/apps",
            post(apps::register_app).merge(get(apps::list_apps)),
        )
        .route(
            "/workspaces/{wid}/apps/{app_id}/install",
            post(apps::install_app),
        )
        .route(
            "/workspaces/{wid}/apps/{app_id}/oauth/authorize",
            post(app_oauth::authorize_app_install),
        )
        .route(
            "/workspaces/{wid}/app-installations",
            get(apps::list_app_installations),
        )
        .route(
            "/workspaces/{wid}/app-installations/{iid}",
            delete(apps::revoke_app_installation),
        )
        .route(
            "/workspaces/{wid}/app-installations/{iid}/tokens",
            post(apps::mint_app_token),
        )
        .route("/members/{id}", get(routes::get_member))
        .route(
            "/members/{id}/assigned-threads",
            get(routes::list_assigned_threads),
        )
        .route("/members/{id}/wip", get(routes::get_member_wip))
        .route("/members/{id}/occupancy", get(routes::get_member_occupancy))
        .route(
            "/members/{id}/usage-rollup",
            get(routes::get_member_usage_rollup),
        )
        .route(
            "/members/{id}/mentions",
            get(routes::list_mentions_for_member),
        )
        .route("/members/{id}/inbox", get(routes::get_member_inbox))
        .route(
            "/members/{id}/inbox/read",
            post(routes::mark_member_inbox_read),
        )
        .route(
            "/members/{id}/notifications",
            get(routes::list_member_notifications),
        )
        .route(
            "/members/{id}/notifications/grouped",
            get(routes::list_member_notifications_grouped),
        )
        .route(
            "/members/{id}/decisions",
            get(routes::list_member_decisions),
        )
        .route(
            "/members/{id}/manager-digest",
            get(routes::get_member_manager_digest),
        )
        .route("/members/{id}/waiting", get(routes::get_member_waiting))
        .route(
            "/members/{id}/notifications/unread-count",
            get(routes::member_unread_notification_count),
        )
        .route(
            "/members/{id}/notifications/read-all",
            post(routes::mark_all_member_notifications_read),
        )
        .route(
            "/members/{id}/notifications/{nid}/read",
            post(routes::mark_member_notification_read),
        )
        .route(
            "/members/{id}/notifications/{nid}/snooze",
            post(routes::snooze_member_notification),
        )
        .route(
            "/members/{id}/notification-prefs",
            put(routes::set_member_notification_pref)
                .merge(get(routes::list_member_notification_prefs)),
        )
        .route(
            "/members/{id}/channel-follows",
            post(routes::follow_member_channel).merge(get(routes::list_member_channel_follows)),
        )
        .route(
            "/members/{id}/channel-follows/{cid}",
            delete(routes::unfollow_member_channel),
        )
        .route(
            "/members/{id}/thread-follows",
            post(routes::follow_member_thread).merge(get(routes::list_member_thread_follows)),
        )
        .route(
            "/members/{id}/thread-follows/{tid}",
            delete(routes::unfollow_member_thread),
        )
        .route(
            "/members/{id}/member-follows",
            post(routes::follow_member_occupancy).merge(get(routes::list_member_occupancy_follows)),
        )
        .route(
            "/members/{id}/member-follows/{followed_id}",
            delete(routes::unfollow_member_occupancy),
        )
        .route(
            "/members/{id}/email",
            put(routes::set_member_email)
                .merge(get(routes::get_member_email))
                .merge(delete(routes::delete_member_email)),
        )
        .route(
            "/members/{id}/delivery-mode",
            put(routes::set_member_delivery_mode).merge(get(routes::get_member_delivery_mode)),
        )
        .route(
            "/members/{id}/push-subscriptions",
            post(routes::register_push_subscription).merge(get(routes::list_push_subscriptions)),
        )
        .route(
            "/members/{id}/push-subscriptions/{sub_id}",
            delete(routes::delete_push_subscription),
        )
        .route(
            "/web-push/vapid-public-key",
            get(routes::get_vapid_public_key),
        )
        .route(
            "/members/{id}/skills",
            post(routes::add_member_skill).merge(get(routes::list_member_skills)),
        )
        .route(
            "/members/{id}/skills/{skill}",
            delete(routes::remove_member_skill),
        )
        .route(
            "/threads/{id}/required-skills",
            post(routes::add_thread_required_skill).merge(get(routes::list_thread_required_skills)),
        )
        .route(
            "/threads/{id}/required-skills/{skill}",
            delete(routes::remove_thread_required_skill),
        )
        .route(
            "/threads/{id}/result",
            put(routes::set_thread_result).merge(get(routes::get_thread_result)),
        )
        .route(
            "/threads/{id}/deliveries",
            get(routes::list_thread_deliveries),
        )
        .route(
            "/threads/{id}/deliveries/{did}/replay",
            post(routes::replay_thread_delivery),
        )
        .route(
            "/threads/{id}/steer",
            put(routes::set_thread_steer).merge(get(routes::get_thread_steer)),
        )
        .route(
            "/threads/{id}/lineage",
            put(routes::set_thread_lineage)
                .merge(get(routes::get_thread_lineage))
                .merge(delete(routes::clear_thread_lineage)),
        )
        .route("/threads/{id}/children", get(routes::list_child_threads))
        .route(
            "/threads/{id}/mute",
            post(routes::mute_thread).merge(delete(routes::unmute_thread)),
        )
        .route(
            "/workspaces/{wid}/approval-gates",
            get(routes::list_approval_gates),
        )
        .route(
            "/approval-gates/{id}/answer",
            post(routes::answer_approval_gate),
        )
        .route(
            "/workspaces/{wid}/dm",
            post(dm::open_dm_conversation).merge(get(dm::list_dm_conversations)),
        )
        .route("/dm/{id}", get(dm::get_dm_conversation))
        .route(
            "/dm/{id}/messages",
            post(dm::post_dm_message).merge(get(dm::list_dm_messages)),
        )
        .route(
            "/workspaces/{wid}/group-dms",
            post(group_dm::open_group_dm).merge(get(group_dm::list_group_dms)),
        )
        .route("/group-dms/{id}", get(group_dm::get_group_dm))
        .route(
            "/group-dms/{id}/messages",
            post(group_dm::post_group_dm_message),
        )
        .route(
            "/workspaces/{wid}/channels",
            post(routes::create_channel).merge(get(routes::list_channels)),
        )
        .route("/channels/{id}", get(routes::get_channel))
        .route("/channels/{id}/boot", get(routes::get_channel_boot))
        .route(
            "/channels/{cid}/queue-depth",
            get(routes::get_channel_queue_depth),
        )
        .route(
            "/channels/{cid}/unclaimable",
            get(routes::list_channel_unclaimable),
        )
        .route("/channels/{cid}/blocked", get(routes::list_channel_blocked))
        .route(
            "/channels/{cid}/occupancy",
            get(routes::get_channel_occupancy),
        )
        .route(
            "/channels/{cid}/mute",
            post(routes::mute_channel).merge(delete(routes::unmute_channel)),
        )
        .route("/channels/{cid}/dlq", get(routes::list_channel_dlq))
        .route(
            "/channels/{cid}/members",
            post(routes::add_channel_member).merge(get(routes::list_channel_members)),
        )
        .route(
            "/channels/{cid}/members/{mid}",
            delete(routes::remove_channel_member),
        )
        .route(
            "/channels/{cid}/threads",
            post(routes::create_thread).merge(get(routes::list_threads)),
        )
        .route(
            "/channels/{cid}/recent-threads",
            get(routes::list_recently_active_threads),
        )
        .route(
            "/channels/{cid}/threads/claim-next",
            post(routes::claim_next_thread),
        )
        .route(
            "/workspaces/{wid}/threads/claim-next",
            post(routes::claim_next_workspace_thread),
        )
        .route(
            "/workspaces/{wid}/queue-depth",
            get(routes::get_workspace_queue_depth),
        )
        .route(
            "/workspaces/{wid}/occupancy",
            get(routes::get_workspace_occupancy),
        )
        .route("/threads/{id}/claim/renew", post(routes::renew_claim))
        .route(
            "/threads/{id}/claim/acknowledge",
            post(routes::acknowledge_claim),
        )
        .route("/threads/{id}/claim/release", post(routes::release_claim))
        .route(
            "/threads/{id}",
            get(routes::get_thread).merge(post(routes::transition_thread)),
        )
        .route("/threads/{id}/context", get(routes::get_thread_context))
        .route(
            "/threads/{id}/context/snapshot",
            post(routes::snapshot_thread_context),
        )
        .route(
            "/threads/{id}/tool-transcript",
            get(routes::get_tool_transcript),
        )
        .route(
            "/threads/{id}/assignee",
            put(routes::assign_thread).merge(delete(routes::unassign_thread)),
        )
        .route(
            "/threads/{id}/owner",
            put(routes::set_thread_owner).merge(delete(routes::remove_thread_owner)),
        )
        .route("/threads/{id}/title", put(routes::rename_thread))
        .route(
            "/threads/{id}/budget",
            put(routes::set_thread_budget)
                .merge(patch(routes::patch_thread_budget))
                .merge(get(routes::get_thread_budget)),
        )
        .route("/threads/{id}/usage", post(routes::report_thread_usage))
        .route(
            "/threads/{id}/usage-rollup",
            get(routes::get_thread_usage_rollup),
        )
        .route(
            "/threads/{id}/usage/otel",
            post(routes::report_thread_usage_otel),
        )
        .route("/threads/{id}/assignee/claim", post(routes::claim_thread))
        .route(
            "/threads/{id}/unclaimable",
            put(routes::mark_thread_unclaimable).merge(delete(routes::mark_thread_claimable)),
        )
        .route(
            "/threads/{id}/block",
            put(routes::set_thread_block)
                .merge(get(routes::get_thread_block))
                .merge(delete(routes::clear_thread_block)),
        )
        .route(
            "/threads/{id}/wait",
            put(routes::set_thread_wait)
                .merge(delete(routes::cancel_thread_wait))
                .merge(get(routes::get_thread_wait)),
        )
        .route(
            "/threads/{id}/priority",
            put(routes::set_thread_priority).merge(get(routes::get_thread_priority)),
        )
        .route(
            "/threads/{id}/dependencies",
            post(routes::add_thread_dependency).merge(get(routes::list_thread_dependencies)),
        )
        .route(
            "/threads/{id}/dependencies/{dep_id}",
            delete(routes::remove_thread_dependency),
        )
        .route(
            "/threads/{id}/dependents",
            get(routes::list_thread_dependents),
        )
        .route(
            "/threads/{tid}/messages",
            post(routes::post_message).merge(get(routes::list_messages)),
        )
        .route(
            "/messages/{id}",
            get(routes::get_message)
                .merge(patch(routes::edit_message))
                .merge(delete(routes::tombstone_message)),
        )
        .route("/messages/{id}/edits", get(routes::list_message_edits))
        .route(
            "/messages/{id}/backlinks",
            get(routes::list_message_backlinks),
        )
        .route("/messages/{id}/purge", delete(routes::purge_message))
        .route("/messages/{id}/seed", post(routes::seed_from_message))
        .route("/messages/{id}/mentions", post(routes::create_mention))
        .route(
            "/messages/{id}/votes",
            post(routes::cast_vote).merge(get(routes::list_votes)),
        )
        .route(
            "/messages/{id}/reactions",
            post(routes::add_reaction)
                .merge(get(routes::list_reactions))
                .merge(delete(routes::remove_reaction)),
        )
        .route(
            "/threads/{id}/pins",
            post(routes::pin_message)
                .merge(get(routes::list_pins))
                .merge(delete(routes::unpin_message)),
        )
        .route("/artifacts", post(routes::upload_artifact))
        .route(
            "/artifacts/multipart",
            post(routes::begin_multipart_artifact).merge(delete(routes::abort_multipart_artifact)),
        )
        .route(
            "/artifacts/multipart/{upload_id}/complete",
            post(routes::complete_multipart_artifact),
        )
        .route(
            "/artifacts/multipart/{upload_id}/parts/{part_number}",
            put(routes::upload_multipart_artifact_part),
        )
        .route(
            "/artifacts/{sha}",
            get(routes::get_artifact).merge(delete(routes::erase_artifact)),
        )
        .route("/artifacts/{sha}/meta", get(routes::get_artifact_metadata))
        .route(
            "/references",
            post(routes::create_reference).merge(get(routes::list_references)),
        )
        .route("/tokens/{id}", delete(routes::revoke_api_token))
        .route("/tokens/{id}/rotate", post(routes::rotate_api_token))
        // On the bearer tree because what it exchanges is the bearer.
        .route(
            "/auth/session/from-token",
            post(session::session_from_token),
        )
        .route("/tokens/attenuate", post(routes::attenuate_api_token))
        .route("/tokens/delegate", post(routes::delegate_api_token))
        .route(
            "/workspaces/{wid}/share-tickets",
            post(routes::create_share_ticket).merge(get(routes::list_share_tickets)),
        )
        .route(
            "/workspaces/{wid}/share-tickets/{tid}",
            delete(routes::revoke_share_ticket),
        )
        .route("/capability-sets", get(routes::list_capability_sets))
        .route("/workspaces/{id}/room", get(routes::get_workspace_room))
        .route(
            "/workspaces/{id}/handle",
            get(routes::get_workspace_handle).merge(put(routes::set_workspace_handle)),
        )
        .route(
            "/workspaces/{wid}/peers",
            post(federation::create_peer).merge(get(federation::list_peers)),
        )
        .route(
            "/workspaces/{wid}/peers/{pid}",
            delete(federation::delete_peer),
        )
        .route(
            "/workspaces/{wid}/task-schedules",
            post(routes::create_task_schedule).merge(get(routes::list_task_schedules)),
        )
        .route(
            "/task-schedules/{id}",
            put(routes::set_task_schedule_active).merge(delete(routes::delete_task_schedule)),
        )
        .route(
            "/workspaces/{wid}/recipes",
            post(routes::create_recipe).merge(get(routes::list_recipes)),
        )
        .route(
            "/workspaces/{wid}/recipes/{id}",
            get(routes::get_recipe).merge(delete(routes::delete_recipe)),
        )
        .route(
            "/workspaces/{wid}/recipes/{id}/instantiate",
            post(routes::instantiate_recipe),
        )
        .route(
            "/workspaces/{wid}/secrets",
            post(routes::create_secret).merge(get(routes::list_secrets)),
        )
        .route(
            "/workspaces/{wid}/secrets/{name}/resolve",
            post(routes::resolve_secret),
        )
        .route(
            "/workspaces/{wid}/secrets/{name}",
            delete(routes::delete_secret),
        )
        .route(
            "/workspaces/{wid}/secret-egress-hosts",
            post(routes::allow_secret_egress_host).merge(get(routes::list_secret_egress_hosts)),
        )
        .route(
            "/workspaces/{wid}/secret-egress-hosts/{host}",
            delete(routes::revoke_secret_egress_host),
        )
        .route(
            "/members/{id}/freeze",
            post(routes::freeze_member)
                .merge(delete(routes::unfreeze_member))
                .merge(get(routes::get_member_freeze)),
        )
        .route(
            "/workspaces/{wid}/frozen-members",
            get(routes::list_frozen_members),
        )
        .route(
            "/workspaces/{wid}/memory-blocks",
            post(routes::create_memory_block).merge(get(routes::list_memory_blocks)),
        )
        .route(
            "/workspaces/{wid}/memory-blocks/{id}",
            get(routes::get_memory_block)
                .merge(put(routes::set_memory_block_value))
                .merge(delete(routes::delete_memory_block)),
        )
        .route(
            "/threads/{id}/memory-blocks",
            get(routes::list_thread_memory_blocks),
        )
        .route(
            "/threads/{id}/memory-blocks/{block_id}",
            post(routes::attach_memory_block).merge(delete(routes::detach_memory_block)),
        )
        .route(
            "/threads/{id}/review-requirement",
            put(routes::set_review_requirement)
                .merge(get(routes::get_review_requirement))
                .merge(delete(routes::clear_review_requirement)),
        )
        .route(
            "/threads/{id}/reviewers",
            post(routes::add_reviewer).merge(get(routes::list_reviewers)),
        )
        .route(
            "/threads/{id}/reviewers/{member_id}",
            delete(routes::remove_reviewer),
        )
        .route(
            "/threads/{id}/reviews",
            post(routes::submit_review).merge(get(routes::list_reviews)),
        )
        .route(
            "/threads/{id}/reviews/history",
            get(routes::list_review_history),
        )
        .route(
            "/threads/{id}/review-status",
            get(routes::get_review_status),
        )
        .route(
            "/threads/{id}/land-gate",
            put(routes::set_land_gate)
                .merge(get(routes::get_land_gate))
                .merge(delete(routes::clear_land_gate)),
        )
        .route(
            "/threads/{id}/land-gate/history",
            get(routes::list_land_gate_history),
        )
        .route(
            "/threads/{id}/land-gate/requirement",
            put(routes::require_land_gate),
        )
        .route(
            "/threads/{id}/land-gate/advice",
            post(routes::advise_land_gate),
        )
        .route(
            "/workspaces/{wid}/webhooks",
            post(webhooks::create_webhook).merge(get(webhooks::list_webhooks)),
        )
        .route(
            "/workspaces/{wid}/webhooks/{whid}",
            delete(webhooks::revoke_webhook),
        )
        .route(
            "/workspaces/{wid}/mention-webhook",
            get(webhooks::get_mention_webhook).merge(put(webhooks::set_mention_webhook)),
        )
        .route(
            "/workspaces/{wid}/slash-commands",
            post(slash_commands::create_slash_command)
                .merge(get(slash_commands::list_slash_commands)),
        )
        .route(
            "/workspaces/{wid}/slash-commands/{cid}",
            delete(slash_commands::revoke_slash_command),
        )
        .route(
            "/workspaces/{wid}/slack-links",
            post(slack::link_slack_channel).merge(get(slack::list_slack_channel_links)),
        )
        .route(
            "/workspaces/{wid}/slack-links/{slack_channel_id}",
            delete(slack::unlink_slack_channel),
        )
        .route(
            "/workspaces/{wid}/github-links",
            post(github::link_github_issue)
                .merge(get(github::list_github_issue_links))
                .merge(delete(github::unlink_github_issue)),
        )
        .route(
            "/workspaces/{wid}/egress-targets",
            post(routes::allow_egress_target).merge(get(routes::list_egress_targets)),
        )
        .route(
            "/workspaces/{wid}/egress-targets/{tid}",
            delete(routes::revoke_egress_target),
        )
        .route(
            "/workspaces/{wid}/fsm-hooks",
            post(fsm_hooks::create_fsm_hook).merge(get(fsm_hooks::list_fsm_hooks)),
        )
        .route(
            "/workspaces/{wid}/fsm-hooks/{hid}",
            delete(fsm_hooks::revoke_fsm_hook),
        )
        .route(
            "/workspaces/{wid}/deliveries",
            get(delivery_ops::list_deliveries),
        )
        .route(
            "/workspaces/{wid}/deliveries/{did}",
            get(delivery_ops::get_delivery),
        )
        .route(
            "/workspaces/{wid}/deliveries/{did}/replay",
            post(delivery_ops::replay_delivery),
        )
        .route(
            "/operator/reindex-embeddings",
            post(reindex_ops::start_reindex_embeddings),
        )
        .route(
            "/operator/reindex-embeddings/{job_id}",
            get(reindex_ops::get_reindex_embeddings_job),
        )
        .route(
            "/operator/export-public-key",
            get(routes::export_public_key),
        )
        .route("/operator/audit", get(routes::list_global_audit))
        .route("/operator/legal-holds", get(routes::list_legal_holds))
        .route("/operator/workspaces", post(routes::provision_workspace))
        .route("/operator/status", get(crate::status::operator_status))
        .route("/operator/mail/dead", get(routes::list_dead_mail))
        .route(
            "/operator/mail/dead/{id}/requeue",
            post(routes::requeue_dead_mail),
        )
        .route("/operator/egress/dead", get(routes::list_dead_egress))
        .route(
            "/operator/egress/dead/{id}/requeue",
            post(routes::requeue_dead_egress),
        )
        .route("/operator/github/mark-ready", post(routes::mark_pull_ready))
        .route(
            "/workspaces/{wid}/automation/dlq",
            get(automation_deliveries::list_quarantined_automation_deliveries),
        )
        .route(
            "/workspaces/{wid}/automation/deliveries",
            get(automation_deliveries::list_automation_deliveries),
        )
        .route(
            "/workspaces/{wid}/automation/deliveries/{did}",
            get(automation_deliveries::get_automation_delivery),
        )
        .route(
            "/workspaces/{wid}/automation/deliveries/{did}/replay",
            post(automation_deliveries::replay_automation_delivery),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            quota::middleware,
        ))
        // Inside auth (it needs the caller) and outside quota (a replay is
        // not a new write).
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::idempotency::middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            consistency::middleware,
        ));

    let a2a = Router::new()
        .route("/a2a/v1/events", post(federation::ingest_events))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            federation::peer_auth_middleware,
        ));

    // Cross-organization share tickets have their own credential type and
    // read-only route tree. Never merge these routes into `protected`: doing
    // so would let the ordinary API-token resolver define their authority.
    let share = Router::new()
        .route("/share/manifest", get(share_consumer::manifest))
        .route("/share/threads", get(share_consumer::list_threads))
        .route(
            "/share/threads/{tid}/messages",
            get(share_consumer::list_messages),
        )
        .route(
            "/share/artifacts/{sha}",
            get(share_consumer::download_artifact),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            share_consumer::middleware,
        ));

    async fn ui_index() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_SECURITY_POLICY, BOARD_UI_CSP)],
            axum::response::Html(include_str!("../static/index.html")),
        )
    }

    // Board modules next to the page. The name must be one of these files.
    // Anything else is a 404. cargo build does not run a bundler.
    async fn ui_asset(
        crate::extract::ApiPath(name): crate::extract::ApiPath<String>,
    ) -> Result<
        (
            [(axum::http::header::HeaderName, &'static str); 2],
            &'static str,
        ),
        axum::http::StatusCode,
    > {
        let (content_type, body) = match name.as_str() {
            "board.css" => (
                "text/css; charset=utf-8",
                include_str!("../static/ui/board.css"),
            ),
            "main.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/main.js"),
            ),
            "state.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/state.js"),
            ),
            "client.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/client.js"),
            ),
            "api.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/api.js"),
            ),
            "feedback.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/feedback.js"),
            ),
            "people.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/people.js"),
            ),
            "session.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/session.js"),
            ),
            "board.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/board.js"),
            ),
            "needs.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/needs.js"),
            ),
            "thread.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/thread.js"),
            ),
            "dm.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/dm.js"),
            ),
            "artifacts.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/artifacts.js"),
            ),
            "tools.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/tools.js"),
            ),
            "palette.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/palette.js"),
            ),
            "realtime.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/realtime.js"),
            ),
            "push.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/push.js"),
            ),
            "sw.js" => (
                "text/javascript; charset=utf-8",
                include_str!("../static/ui/sw.js"),
            ),
            _ => return Err(axum::http::StatusCode::NOT_FOUND),
        };
        Ok((
            [
                (axum::http::header::CONTENT_TYPE, content_type),
                (axum::http::header::CONTENT_SECURITY_POLICY, BOARD_UI_CSP),
            ],
            body,
        ))
    }

    // The agent-facing index (llmstxt.org): how to connect and the work loop,
    // with paths relative to this server. Public like `/ui`, and static.
    async fn llms_txt() -> impl axum::response::IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE,
                "text/markdown; charset=utf-8",
            )],
            include_str!("../static/llms.txt"),
        )
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

    let ui_api_read = Router::new()
        .route("/ui/api/workspaces/{wid}/events", get(routes::list_events))
        .route(
            "/ui/api/workspaces/{wid}/channels",
            get(routes::list_channels),
        )
        .route("/ui/api/channels/{cid}/threads", get(routes::list_threads))
        .route("/ui/api/threads/{tid}/messages", get(routes::list_messages))
        .route("/ui/api/threads/{tid}", get(routes::get_thread))
        .route(
            "/ui/api/threads/{tid}/review-status",
            get(routes::get_review_status),
        )
        .route("/ui/api/workspaces/{wid}", get(routes::get_workspace))
        .route(
            "/ui/api/workspaces/{wid}/search",
            get(routes::search_messages),
        )
        .route(
            "/ui/api/workspaces/{wid}/audit",
            get(routes::list_workspace_audit),
        )
        .route(
            "/ui/api/workspaces/{wid}/peers",
            get(federation::list_peers),
        )
        .route(
            "/ui/api/messages/{mid}/edits",
            get(routes::list_message_edits),
        )
        .route(
            "/ui/api/messages/{mid}/reactions",
            get(routes::list_reactions),
        )
        .route("/ui/api/threads/{tid}/pins", get(routes::list_pins))
        .route(
            "/ui/api/workspaces/{wid}/group-dms",
            get(group_dm::list_group_dms),
        )
        .route("/ui/api/group-dms/{id}", get(group_dm::get_group_dm))
        .route(
            "/ui/api/workspaces/{wid}/dm",
            get(dm::list_dm_conversations),
        )
        .route(
            "/ui/api/workspaces/{wid}/slash-commands",
            get(slash_commands::list_slash_commands),
        )
        .route(
            "/ui/api/workspaces/{wid}/deliveries",
            get(delivery_ops::list_deliveries),
        )
        .route(
            "/ui/api/workspaces/{wid}/members",
            get(routes::list_members),
        )
        .route(
            "/ui/api/workspaces/{wid}/members/{mid}/tokens",
            get(routes::list_api_tokens),
        )
        .route(
            "/ui/api/workspaces/{wid}/app-installations",
            get(apps::list_app_installations),
        )
        .route(
            "/ui/api/members/{id}/notifications",
            get(routes::list_member_notifications),
        )
        .route(
            "/ui/api/members/{id}/notifications/unread-count",
            get(routes::member_unread_notification_count),
        )
        .route(
            "/ui/api/workspaces/{wid}/approval-gates",
            get(routes::list_approval_gates),
        )
        .route("/ui/api/me", get(routes::get_me))
        // Work tab: the human work-observability console — queue depth,
        // occupancy, task results, DAG dependencies, and schedules.
        .route(
            "/ui/api/channels/{cid}/queue-depth",
            get(routes::get_channel_queue_depth),
        )
        .route(
            "/ui/api/channels/{cid}/occupancy",
            get(routes::get_channel_occupancy),
        )
        .route(
            "/ui/api/threads/{tid}/result",
            get(routes::get_thread_result),
        )
        .route(
            "/ui/api/threads/{tid}/dependencies",
            get(routes::list_thread_dependencies),
        )
        .route(
            "/ui/api/workspaces/{wid}/task-schedules",
            get(routes::list_task_schedules),
        )
        // Prefs console: a member's notification preferences, delivery, and
        // follows — self-only reads.
        .route(
            "/ui/api/members/{id}/notification-prefs",
            get(routes::list_member_notification_prefs),
        )
        .route(
            "/ui/api/members/{id}/delivery-mode",
            get(routes::get_member_delivery_mode),
        )
        .route("/ui/api/members/{id}/email", get(routes::get_member_email))
        .route(
            "/ui/api/web-push/vapid-public-key",
            get(routes::get_vapid_public_key),
        )
        .route(
            "/ui/api/members/{id}/channel-follows",
            get(routes::list_member_channel_follows),
        )
        .route(
            "/ui/api/members/{id}/thread-follows",
            get(routes::list_member_thread_follows),
        )
        // Looking-glass explorer: artifact metadata by sha
        // (events/threads/peers reuse the routes above).
        .route(
            "/ui/api/artifacts/{sha}/meta",
            get(routes::get_artifact_metadata),
        )
        // The bytes, so a signed-in viewer's thread can show an image.
        .route("/ui/api/artifacts/{sha}", get(routes::get_artifact))
        // Waiting-on-you inbox.
        .route(
            "/ui/api/members/{id}/waiting",
            get(routes::get_member_waiting),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session_or_bearer_middleware,
        ));

    let ui_api_write = Router::new()
        .route("/ui/api/workspaces/{wid}", patch(routes::rename_workspace))
        .route(
            "/ui/api/workspaces/{wid}/channels",
            post(routes::create_channel),
        )
        .route(
            "/ui/api/channels/{cid}/threads",
            post(routes::create_thread),
        )
        .route("/ui/api/threads/{tid}/messages", post(routes::post_message))
        .route("/ui/api/threads/{tid}", post(routes::transition_thread))
        .route(
            "/ui/api/threads/{tid}/block",
            delete(routes::clear_thread_block),
        )
        .route("/ui/api/threads/{tid}/reviews", post(routes::submit_review))
        .route("/ui/api/messages/{mid}", patch(routes::edit_message))
        .route("/ui/api/artifacts", post(routes::upload_artifact))
        .route(
            "/ui/api/messages/{mid}/reactions",
            post(routes::add_reaction).merge(delete(routes::remove_reaction)),
        )
        .route(
            "/ui/api/threads/{tid}/pins",
            post(routes::pin_message).merge(delete(routes::unpin_message)),
        )
        .route(
            "/ui/api/workspaces/{wid}/group-dms",
            post(group_dm::open_group_dm),
        )
        .route(
            "/ui/api/group-dms/{id}/messages",
            post(group_dm::post_group_dm_message),
        )
        .route(
            "/ui/api/workspaces/{wid}/dm",
            post(dm::open_dm_conversation),
        )
        .route("/ui/api/dm/{id}/messages", post(dm::post_dm_message))
        .route(
            "/ui/api/workspaces/{wid}/slash-commands",
            post(slash_commands::create_slash_command),
        )
        .route(
            "/ui/api/workspaces/{wid}/slash-commands/{cid}",
            delete(slash_commands::revoke_slash_command),
        )
        .route(
            "/ui/api/workspaces/{wid}/deliveries/{did}/replay",
            post(delivery_ops::replay_delivery),
        )
        .route(
            "/ui/api/operator/reindex-embeddings",
            post(reindex_ops::start_reindex_embeddings),
        )
        // GET on the write router: a workspace-scoped reindex job needs
        // workspace:write to read, which only the write session grants.
        .route(
            "/ui/api/operator/reindex-embeddings/{job_id}",
            get(reindex_ops::get_reindex_embeddings_job),
        )
        .route(
            "/ui/api/members/{id}/notifications/{nid}/read",
            post(routes::mark_member_notification_read),
        )
        .route(
            "/ui/api/members/{id}/notifications/read-all",
            post(routes::mark_all_member_notifications_read),
        )
        .route(
            "/ui/api/approval-gates/{id}/answer",
            post(routes::answer_approval_gate),
        )
        // Prefs console: self-only writes.
        .route(
            "/ui/api/members/{id}/notification-prefs",
            put(routes::set_member_notification_pref),
        )
        .route(
            "/ui/api/members/{id}/delivery-mode",
            put(routes::set_member_delivery_mode),
        )
        .route(
            "/ui/api/members/{id}/email",
            put(routes::set_member_email).merge(delete(routes::delete_member_email)),
        )
        .route(
            "/ui/api/members/{id}/push-subscriptions",
            post(routes::register_push_subscription),
        )
        .route(
            "/ui/api/members/{id}/channel-follows",
            post(routes::follow_member_channel),
        )
        .route(
            "/ui/api/members/{id}/channel-follows/{cid}",
            delete(routes::unfollow_member_channel),
        )
        .route(
            "/ui/api/members/{id}/thread-follows",
            post(routes::follow_member_thread),
        )
        .route(
            "/ui/api/members/{id}/thread-follows/{tid}",
            delete(routes::unfollow_member_thread),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::ui_session_or_bearer_middleware,
        ));

    let ui_api = ui_api_read.merge(ui_api_write);

    Router::new()
        .route("/health", get(health::handler))
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .route("/.well-known/maidan.json", get(federation::well_known))
        .route("/.well-known/maidan-room", get(routes::well_known_room))
        .route("/.well-known/agent-card.json", get(a2a_agent::agent_card))
        .route("/oauth/app/token", post(app_oauth::exchange_app_code))
        // Slack projector ingress: unauthed — Slack authenticates via its
        // request signature (verified in-handler), not a Maidan bearer. Returns
        // 404 unless the projector is configured.
        .route("/integrations/slack/events", post(slack::slack_events))
        // GitHub projector ingress: unauthed — GitHub authenticates via its
        // X-Hub-Signature-256, verified in-handler. 404 unless configured.
        .route("/integrations/github/events", post(github::github_events))
        .route("/openapi.json", get(openapi::openapi_json))
        .route("/metrics", get(metrics::scrape))
        .route("/ui", get(ui_index))
        .route("/ui/", get(ui_index))
        .route("/ui/static/{name}", get(ui_asset))
        .route("/llms.txt", get(llms_txt))
        .merge({
            #[cfg(feature = "bootstrap")]
            {
                bootstrap
            }
            #[cfg(not(feature = "bootstrap"))]
            {
                Router::new()
            }
        })
        .merge(auth_routes)
        .merge(ui_api)
        .merge(ws_only)
        .merge(a2a)
        .merge(share)
        .merge(protected)
        // An unknown path and a known one asked with the wrong method answer
        // with a problem like every other client error, not axum's empty body.
        // The 405 keeps axum's `Allow` header.
        .problem_fallbacks()
        // Innermost of the shared layers, so a panic in a handler or in a
        // route group's own middleware (auth, quota) becomes a 500 problem
        // that the metrics, rate limiter and request id still see.
        .layer(panic_guard::layer())
        .layer(middleware::from_fn(metrics::middleware))
        // Room-LSN sits *inside* the rate limiter: a later `.layer` is the
        // outer one, so this order makes the limiter outermost. It used to wrap
        // the limiter, which meant a 429 — the response whose job is to stop
        // work — still ran `MAX(id)` against the primary, and an
        // unauthenticated client could force one database round-trip per
        // request with nothing able to shed it.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            room_lsn::middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::middleware,
        ))
        // Shedding sits outside the rate limiter and every credential check,
        // so a refused request costs no Redis round-trip, token lookup or
        // pooled connection.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            load_shed::middleware,
        ))
        // Response headers are marked sensitive before the trace layer logs
        // them; request headers are marked before it makes its span.
        .layer(trace_redaction::response_layer())
        .layer(trace_redaction::trace_layer())
        .layer(trace_redaction::request_layer())
        .layer(middleware::from_fn(request_id::middleware))
        // Outside the request-id span so the exported server span's parent
        // is the caller's trace, not an internal child.
        .layer(middleware::from_fn(trace_context::middleware))
        // Cap request bodies before extractors buffer them.
        .layer(DefaultBodyLimit::max(max_body_bytes_from_env()))
        .with_state(state)
}

#[cfg(test)]
mod body_limit_tests {
    use super::*;

    #[test]
    fn max_body_bytes_parses_and_falls_back() {
        assert_eq!(parse_max_body_bytes(None), DEFAULT_MAX_BODY_BYTES);
        assert_eq!(
            parse_max_body_bytes(Some("  ".into())),
            DEFAULT_MAX_BODY_BYTES
        );
        assert_eq!(
            parse_max_body_bytes(Some("nope".into())),
            DEFAULT_MAX_BODY_BYTES
        );
        assert_eq!(
            parse_max_body_bytes(Some("0".into())),
            DEFAULT_MAX_BODY_BYTES
        );
        assert_eq!(parse_max_body_bytes(Some("4096".into())), 4096);
    }
}
