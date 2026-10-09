//! OpenAPI 3.1 document for the Maidan HTTP API (Track W.1).

#[cfg(test)]
mod lint;
mod paths;
mod responses;
mod schemas;

use axum::Json;
use utoipa::openapi::path::{Operation, Parameter, ParameterBuilder, ParameterIn, PathItem};
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{Ref, Required};
use utoipa::{Modify, OpenApi};

use crate::dto::*;
use crate::error::ProblemDetails;
use crate::federation::{IngestSummary, WellKnownA2a, WellKnownAuth, WellKnownMaidan};
use crate::health::{HealthResponse, SubsystemStatus};
use crate::land_gate_advisor::{
    LandGateAdvice, LandGateAdviceRequest, LandGateAdviceThresholds, LandGateAdviceUsage,
};
use crate::openapi::responses::{
    BadRequest, Conflict, Forbidden, IdempotencyKeyReused, InternalServerError, NotFound,
    Overloaded, PayloadTooLarge, TooManyRequests, Unauthorized, UnsupportedMediaType,
};
use crate::openapi::schemas::{LivenessOk, SearchHit};
use crate::share_consumer::*;
use crate::status::{OperatorStatus, QueueStatus, ReplicaStatus, SearchProgress, StatusCheck};
use crate::thread_context::{BootPack, ThreadContext, WorkspaceContext};
use maidan_types::*;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearerAuth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("API or peer token")
                    .build(),
            ),
        );
        components.add_security_scheme(
            "sessionCookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                "maidan_session",
                "A browser session. An OIDC session reaches the `/ui/api` routes with a fixed \
                 set of capabilities. A session from `POST /auth/session/from-token` holds \
                 that token's authority and also reaches every `bearerAuth` route except MCP. \
                 An unsafe request on a session must come from this origin.",
            ))),
        );
        components.add_security_scheme(
            "shareTicketAuth",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "Authorization",
                "Use `ShareTicket maid_share_…`; this credential is not an API bearer token.",
            ))),
        );
    }
}

/// The responses a middleware layer adds to every operation it wraps (see
/// `app.rs`), attached here rather than restated on each path stub, so an
/// operation added later carries them without anyone remembering to:
///
/// - **401** — every operation with a security requirement sits behind a layer
///   that refuses a missing or bad credential: bearer (`auth::middleware`),
///   session cookie, share ticket, or federation peer.
/// - **429** — `rate_limit::middleware` wraps every route but the ones
///   [`rate_limit::exempt_path`] names, and `quota::middleware` enforces
///   per-token capability quotas on the bearer routes.
/// - **500** — every operation: `panic_guard` answers a panicking handler with
///   a problem, and any handler can fail.
/// - **503** — `load_shed::middleware` refuses a request past the in-flight
///   ceiling on every route it does not exempt (the rate limiter's exemptions).
///
/// The client errors [`crate::extract`] answers with are attached by
/// [`ExtractorResponses`]; the rest a handler produces itself (400, 403, 404,
/// 409) are declared on its path stub.
struct MiddlewareResponses;

impl Modify for MiddlewareResponses {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        for (path, item) in openapi.paths.paths.iter_mut() {
            let rate_limited = !crate::rate_limit::exempt_path(path);
            let sheddable = !crate::load_shed::exempt_path(path);
            for op in operations_mut(item) {
                let requires_credential = requires_credential(op);
                let responses = &mut op.responses.responses;
                if requires_credential {
                    responses
                        .entry("401".to_owned())
                        .or_insert_with(|| Ref::from_response_name("Unauthorized").into());
                }
                if rate_limited {
                    responses
                        .entry("429".to_owned())
                        .or_insert_with(|| Ref::from_response_name("TooManyRequests").into());
                }
                responses
                    .entry("500".to_owned())
                    .or_insert_with(|| Ref::from_response_name("InternalServerError").into());
                if sheddable {
                    responses
                        .entry("503".to_owned())
                        .or_insert_with(|| Ref::from_response_name("Overloaded").into());
                }
            }
        }
    }
}

/// What `crate::idempotency::middleware` adds to every credentialed write it
/// wraps (all but SCIM and A2A, which keep their own error envelopes): the
/// optional `Idempotency-Key` header, 400 for a malformed key, 409 for a retry
/// while the first request runs, and 422 for a key reused on a different
/// request. An operation's own description of a status is kept.
struct IdempotencyResponses;

impl Modify for IdempotencyResponses {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        for (path, item) in openapi.paths.paths.iter_mut() {
            if path.starts_with("/scim/") || path.starts_with("/a2a/") {
                continue;
            }
            let writes = [
                &mut item.post,
                &mut item.put,
                &mut item.patch,
                &mut item.delete,
            ];
            for op in writes.into_iter().filter_map(Option::as_mut) {
                if !requires_credential(op) {
                    continue;
                }
                let key = ParameterBuilder::new()
                    .name(crate::idempotency::IDEMPOTENCY_KEY_HEADER)
                    .parameter_in(ParameterIn::Header)
                    .required(Required::False)
                    .description(Some(
                        "1 to 255 visible ASCII characters, scoped to the caller. A retry with \
                         the same key and request gets the first response back with \
                         `Idempotent-Replayed: true` instead of running again; kept 24 hours.",
                    ))
                    .schema(Some(
                        utoipa::openapi::schema::ObjectBuilder::new()
                            .schema_type(utoipa::openapi::schema::Type::String)
                            .min_length(Some(1))
                            .max_length(Some(255)),
                    ))
                    .build();
                op.parameters.get_or_insert_with(Vec::new).push(key);
                let responses = &mut op.responses.responses;
                for (status, name) in [
                    ("400", "BadRequest"),
                    ("409", "Conflict"),
                    ("422", "IdempotencyKeyReused"),
                ] {
                    responses
                        .entry(status.to_owned())
                        .or_insert_with(|| Ref::from_response_name(name).into());
                }
            }
        }
    }
}

/// The client errors the request extractors in [`crate::extract`] answer with,
/// derived from what each operation declares it takes:
///
/// - **400** — a path parameter (a UUID or number that does not parse, or a
///   segment that is not UTF-8), a query parameter that is required or typed,
///   or a JSON body that does not parse or match its schema.
/// - **413** — any request body over `MAIDAN_MAX_BODY_BYTES`.
/// - **415** — a JSON body sent without a JSON `Content-Type`.
///
/// An operation's own description of a status, where it has one, is kept.
struct ExtractorResponses;

impl Modify for ExtractorResponses {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        for item in openapi.paths.paths.values_mut() {
            for op in operations_mut(item) {
                let body = op.request_body.as_ref().map(|body| &body.content);
                let json_body =
                    body.is_some_and(|content| content.contains_key("application/json"));
                let rejectable_parameter = op
                    .parameters
                    .iter()
                    .flatten()
                    .any(parameter_can_be_rejected);
                let responses = &mut op.responses.responses;
                if json_body || rejectable_parameter {
                    responses
                        .entry("400".to_owned())
                        .or_insert_with(|| Ref::from_response_name("BadRequest").into());
                }
                if body.is_some() {
                    responses
                        .entry("413".to_owned())
                        .or_insert_with(|| Ref::from_response_name("PayloadTooLarge").into());
                }
                if json_body {
                    responses
                        .entry("415".to_owned())
                        .or_insert_with(|| Ref::from_response_name("UnsupportedMediaType").into());
                }
            }
        }
    }
}

/// Whether [`crate::extract`] can refuse a request over this parameter. Any
/// path segment can fail to percent-decode to UTF-8. A query parameter can be
/// missing when required, or fail to parse when it is anything but free text.
fn parameter_can_be_rejected(parameter: &Parameter) -> bool {
    match parameter.parameter_in {
        ParameterIn::Path => true,
        ParameterIn::Query => {
            matches!(parameter.required, Required::True)
                || parameter.schema.as_ref().is_some_and(|schema| {
                    serde_json::to_value(schema).ok()
                        != Some(serde_json::json!({ "type": "string" }))
                })
        }
        ParameterIn::Header | ParameterIn::Cookie => false,
    }
}

fn operations_mut(item: &mut PathItem) -> impl Iterator<Item = &mut Operation> {
    [
        &mut item.get,
        &mut item.put,
        &mut item.post,
        &mut item.delete,
        &mut item.options,
        &mut item.head,
        &mut item.patch,
        &mut item.trace,
    ]
    .into_iter()
    .filter_map(Option::as_mut)
}

/// An operation requires a credential when any of its security requirements
/// names a scheme; `security(())` (an empty requirement) marks a public one.
fn requires_credential(op: &Operation) -> bool {
    op.security.iter().flatten().any(|requirement| {
        serde_json::to_value(requirement)
            .ok()
            .and_then(|value| value.as_object().map(|scheme| !scheme.is_empty()))
            .unwrap_or(false)
    })
}

/// Generated OpenAPI document (stable `v1.0.0` HTTP surface).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Maidan API",
        version = "2.1.0",
        description = "The operating layer for teams of AI agents. A team of agents coordinates work, keeps a durable searchable record, and pulls exactly the context each step needs, so they do better work for fewer tokens. Human login uses OIDC + `maidan_session` cookie (see `auth` tag).\n\n\
            WebSocket GET /ws/subscribe: after upgrade, send one JSON text frame with `filter`, optional `after_id`, optional `resume_token` (replaces filter and after_id), and optional bearer `token` when auth is enabled. Control frames: `subscribe_ack` (resume_token + after_id watermark), `replay_hint`, `replay_truncated` (after_id + limit 500), then event envelopes with log_id.\n\n\
            MCP SSE GET /mcp/stream: query workspace_id, after_id, or resume_token; narrow with channel_id, thread_id, member_id, and kinds (comma-separated snake_case event kinds, e.g. mention_recorded) so an agent can await just its mentions or one thread; same control frames; requires bearer event:subscribe.\n\
            MCP SSE GET /mcp/notifications: JSON-RPC notifications (e.g. notifications/resources/updated); requires workspace:read.\n\
            MCP JSON-RPC POST /mcp: one request or a top-level array as a batch; a notification (no `id`) is executed for effect and answered `202 Accepted` with no body. No session, ever. `POST /mcp/worker` and `POST /mcp/reviewer` serve a fixed tool profile, the same bytes for every caller; a tool the token cannot call is refused at `tools/call`.\n\
            MCP streamable HTTP POST /mcp/streamable: send `MCP-Protocol-Version: 2026-07-28` (the revision `initialize` negotiates by default) and the request lands cold and returns a single JSON-RPC response — no `Mcp-Session-Id` is minted or required. Optional SEP-2243 `Mcp-Method` / `Mcp-Name` routing headers let a gateway route without parsing the body; when present they must agree with the body, else 400. One request per call here — batches go to POST /mcp. Omitting the version header falls back to `2024-11-05`, which is still fully supported and keeps the session model: an SSE-accepting POST opens the stream and returns `Mcp-Session-Id`, follow-up POSTs on that id return `202 Accepted` and multiplex their JSON-RPC results on the stream, frames carry an `id:` so GET /mcp/streamable with `Last-Event-ID` replays retained frames, and DELETE /mcp/streamable closes it. On either revision, `Accept: application/json` (no `text/event-stream`) gets a single JSON body. An unsupported `MCP-Protocol-Version` is a 400.\n\
            The server issues no sampling or roots requests, and asks for input in one place only: `approval_decide`, when a model asks to accept a gate that needs a person, returns a URL-mode elicitation (as an `input_required` result) to a `2026-07-28` client that declared `elicitation.url`, pointing at a one-time console link, and the link in its result to any other client. It never asks for form input. Human approval is a durable gate — the MCP `request_approval` tool returns `input_required` with a `gate_id` immediately, a human answers over POST /approval-gates/:id/answer or the `/ui`, and the agent polls `get_approval_gate`. Live waits ride GET /mcp/stream, the WebSocket, or the `wait_for_*` tools.\n\n\
            GET /metrics: Prometheus exposition (HTTP latency + maidan_bus_lag_total, maidan_subscribe_replay_total, maidan_indexer_last_event_age_seconds, maidan_bus_listener_ok, maidan_bus_notify_hydrate_total, maidan_outbox_pending, maidan_outbox_quarantined, maidan_outbox_oldest_pending_seconds, maidan_outbox_relay_total{result} on Postgres). Fixed label cardinality only.",
        license(name = "MIT OR Apache-2.0", url = "https://github.com/david-engelmann/maidan")
    ),
    servers((url = "/", description = "The server this document is served from")),
    paths(
        paths::health_live,
        paths::health_ready,
        paths::health,
        paths::get_workspace,
        paths::rename_workspace,
        paths::purge_workspace,
        paths::erase_workspace,
        paths::export_workspace,
        paths::verify_workspace_export,
        paths::export_public_key,
        paths::import_workspace,
        paths::get_workspace_usage,
        paths::get_workspace_usage_rollup,
        paths::list_workspace_tombstones,
        paths::get_workspace_kind_census,
        paths::list_workspace_results,
        paths::list_run_threads,
        paths::get_run_occupancy,
        paths::place_legal_hold,
        paths::lift_legal_hold,
        paths::list_workspace_legal_holds,
        paths::get_preserved_messages,
        paths::list_legal_holds,
        paths::provision_workspace,
        paths::operator_status,
        paths::set_wip_limit,
        paths::get_wip_limit,
        paths::set_approval_policy,
        paths::get_approval_policy,
        paths::set_delegation_policy,
        paths::get_delegation_policy,
        paths::set_retention_policy,
        paths::get_retention_policy,
        paths::get_member_wip,
        paths::get_me,
        paths::allow_egress_target,
        paths::list_egress_targets,
        paths::revoke_egress_target,
        paths::link_slack_channel,
        paths::list_slack_channel_links,
        paths::unlink_slack_channel,
        paths::link_github_issue,
        paths::list_github_issue_links,
        paths::unlink_github_issue,
        paths::list_glossary_terms,
        paths::set_glossary_term,
        paths::get_glossary_term,
        paths::delete_glossary_term,
        paths::list_assigned_threads,
        paths::claim_next_thread,
        paths::claim_next_workspace_thread,
        paths::renew_claim,
        paths::acknowledge_claim,
        paths::release_claim,
        paths::list_workspace_audit,
        paths::get_workspace_context,
        paths::list_quarantined_outbox,
        paths::replay_quarantined_outbox,
        paths::list_events,
        paths::verify_event_chain,
        paths::get_log_snapshot,
        paths::catch_up_events,
        paths::search_messages,
        paths::list_members,
        paths::mint_api_token,
        paths::list_api_tokens,
        paths::create_delegation_grant,
        paths::list_delegation_grants,
        paths::revoke_delegation_grant,
        paths::create_share_ticket,
        paths::list_share_tickets,
        paths::revoke_share_ticket,
        paths::get_share_manifest,
        paths::list_shared_threads,
        paths::list_shared_messages,
        paths::download_shared_artifact,
        paths::register_app,
        paths::list_apps,
        paths::install_app,
        paths::authorize_app_install,
        paths::list_app_installations,
        paths::revoke_app_installation,
        paths::mint_app_token,
        paths::open_dm_conversation,
        paths::list_dm_conversations,
        paths::get_dm_conversation,
        paths::post_dm_message,
        paths::list_dm_messages,
        paths::open_group_dm,
        paths::list_group_dms,
        paths::get_group_dm,
        paths::post_group_dm_message,
        paths::list_channels,
        paths::create_channel,
        paths::list_peers,
        paths::create_peer,
        paths::delete_peer,
        paths::list_webhooks,
        paths::create_webhook,
        paths::revoke_webhook,
        paths::get_mention_webhook,
        paths::set_mention_webhook,
        paths::list_slash_commands,
        paths::create_slash_command,
        paths::revoke_slash_command,
        paths::list_fsm_hooks,
        paths::create_fsm_hook,
        paths::revoke_fsm_hook,
        paths::list_automation_dlq,
        paths::list_automation_deliveries,
        paths::get_automation_delivery,
        paths::replay_automation_delivery,
        paths::list_unified_deliveries,
        paths::get_unified_delivery,
        paths::replay_unified_delivery,
        paths::list_global_audit,
        paths::list_dead_mail,
        paths::requeue_dead_mail,
        paths::list_dead_egress,
        paths::requeue_dead_egress,
        paths::mark_pull_ready,
        paths::start_reindex_embeddings,
        paths::get_reindex_embeddings_job,
        paths::get_member,
        paths::get_member_usage_rollup,
        paths::list_mentions_for_member,
        paths::get_member_inbox,
        paths::mark_member_inbox_read,
        paths::list_member_notifications,
        paths::list_member_notifications_grouped,
        paths::list_member_decisions,
        paths::get_member_manager_digest,
        paths::get_member_waiting,
        paths::member_unread_notification_count,
        paths::mark_all_member_notifications_read,
        paths::mark_member_notification_read,
        paths::snooze_member_notification,
        paths::set_member_notification_pref,
        paths::list_member_notification_prefs,
        paths::follow_member_channel,
        paths::unfollow_member_channel,
        paths::list_member_channel_follows,
        paths::follow_member_thread,
        paths::unfollow_member_thread,
        paths::list_member_thread_follows,
        paths::follow_member_occupancy,
        paths::unfollow_member_occupancy,
        paths::list_member_occupancy_follows,
        paths::get_member_occupancy,
        paths::set_member_email,
        paths::get_member_email,
        paths::delete_member_email,
        paths::set_member_delivery_mode,
        paths::get_member_delivery_mode,
        paths::register_push_subscription,
        paths::get_vapid_public_key,
        paths::list_push_subscriptions,
        paths::delete_push_subscription,
        paths::get_channel,
        paths::get_channel_boot,
        paths::get_channel_queue_depth,
        paths::list_channel_unclaimable,
        paths::list_channel_blocked,
        paths::get_channel_occupancy,
        paths::get_workspace_queue_depth,
        paths::get_workspace_occupancy,
        paths::mute_channel,
        paths::unmute_channel,
        paths::list_channel_dlq,
        paths::add_channel_member,
        paths::list_channel_members,
        paths::remove_channel_member,
        paths::list_threads,
        paths::list_recently_active_threads,
        paths::create_thread,
        paths::get_thread,
        paths::get_thread_context,
        paths::snapshot_thread_context,
        paths::get_tool_transcript,
        paths::transition_thread,
        paths::assign_thread,
        paths::unassign_thread,
        paths::set_thread_owner,
        paths::remove_thread_owner,
        paths::rename_thread,
        paths::set_thread_budget,
        paths::patch_thread_budget,
        paths::get_thread_budget,
        paths::report_thread_usage,
        paths::get_thread_usage_rollup,
        paths::report_thread_usage_otel,
        paths::claim_thread,
        paths::mark_thread_unclaimable,
        paths::mark_thread_claimable,
        paths::set_thread_block,
        paths::get_thread_block,
        paths::clear_thread_block,
        paths::get_thread_version,
        paths::get_review_packet,
        paths::list_thread_artifacts,
        paths::link_thread_artifact,
        paths::unlink_thread_artifact,
        paths::set_thread_wait,
        paths::cancel_thread_wait,
        paths::get_thread_wait,
        paths::set_thread_priority,
        paths::get_thread_priority,
        paths::list_messages,
        paths::post_message,
        paths::get_message,
        paths::list_message_backlinks,
        paths::edit_message,
        paths::list_message_edits,
        paths::tombstone_message,
        paths::purge_message,
        paths::create_mention,
        paths::seed_from_message,
        paths::cast_vote,
        paths::list_votes,
        paths::retract_vote,
        paths::add_reaction,
        paths::remove_reaction,
        paths::list_reactions,
        paths::pin_message,
        paths::unpin_message,
        paths::list_pins,
        paths::add_thread_dependency,
        paths::list_thread_dependencies,
        paths::remove_thread_dependency,
        paths::list_thread_dependents,
        paths::create_task_schedule,
        paths::list_task_schedules,
        paths::set_task_schedule_active,
        paths::delete_task_schedule,
        paths::create_recipe,
        paths::list_recipes,
        paths::get_recipe,
        paths::delete_recipe,
        paths::instantiate_recipe,
        paths::create_secret,
        paths::list_secrets,
        paths::resolve_secret,
        paths::delete_secret,
        paths::allow_secret_egress_host,
        paths::list_secret_egress_hosts,
        paths::revoke_secret_egress_host,
        paths::freeze_member,
        paths::unfreeze_member,
        paths::get_member_freeze,
        paths::list_frozen_members,
        paths::create_memory_block,
        paths::list_memory_blocks,
        paths::get_memory_block,
        paths::set_memory_block_value,
        paths::delete_memory_block,
        paths::list_thread_memory_blocks,
        paths::attach_memory_block,
        paths::detach_memory_block,
        paths::set_review_requirement,
        paths::get_review_requirement,
        paths::clear_review_requirement,
        paths::add_reviewer,
        paths::list_reviewers,
        paths::remove_reviewer,
        paths::submit_review,
        paths::list_reviews,
        paths::list_review_history,
        paths::get_review_status,
        paths::set_land_gate,
        paths::get_land_gate,
        paths::list_land_gate_history,
        paths::clear_land_gate,
        paths::require_land_gate,
        paths::advise_land_gate,
        paths::set_spawn_budget,
        paths::get_spawn_budget,
        paths::add_member_skill,
        paths::list_member_skills,
        paths::remove_member_skill,
        paths::add_thread_required_skill,
        paths::list_thread_required_skills,
        paths::remove_thread_required_skill,
        paths::set_thread_result,
        paths::get_thread_result,
        paths::list_thread_deliveries,
        paths::replay_thread_delivery,
        paths::set_thread_lineage,
        paths::get_thread_lineage,
        paths::clear_thread_lineage,
        paths::set_thread_steer,
        paths::get_thread_steer,
        paths::list_child_threads,
        paths::mute_thread,
        paths::unmute_thread,
        paths::list_approval_gates,
        paths::answer_approval_gate,
        paths::upload_artifact,
        paths::get_artifact,
        paths::get_artifact_metadata,
        paths::erase_artifact,
        paths::begin_multipart_artifact_doc,
        paths::abort_multipart_artifact_doc,
        paths::complete_multipart_artifact_doc,
        paths::upload_multipart_artifact_part_doc,
        paths::create_reference,
        paths::list_references,
        paths::revoke_api_token,
        paths::rotate_api_token,
        paths::attenuate_api_token,
        paths::delegate_api_token,
        paths::list_capability_sets,
        paths::get_workspace_room,
        paths::get_workspace_handle,
        paths::set_workspace_handle,
        paths::well_known,
        paths::well_known_room,
        paths::ingest_events,
        paths::oidc_login,
        paths::oidc_callback,
        paths::oidc_logout,
        paths::get_auth_session,
        paths::list_auth_session_workspaces,
        paths::mint_auth_session_token,
        paths::confirm_approval,
        paths::session_from_token,
        paths::ui_list_events,
        paths::ui_list_channels,
        paths::ui_rename_workspace,
        paths::ui_create_channel,
        paths::ui_create_thread,
        paths::ui_post_message,
        paths::ui_list_threads,
        paths::ui_list_messages,
        paths::ui_list_reviews,
        paths::ui_list_thread_artifacts,
        paths::ui_search_messages,
        paths::ui_list_audit,
        paths::ui_list_peers,
        paths::ui_list_message_edits,
    ),
    components(
    responses(
        BadRequest,
        Unauthorized,
        Forbidden,
        NotFound,
        Conflict,
        PayloadTooLarge,
        UnsupportedMediaType,
        TooManyRequests,
        InternalServerError,
        Overloaded,
        IdempotencyKeyReused,
    ),
    schemas(
        LivenessOk,
        // Every type a schema or operation names. utoipa 4 does not collect
        // these itself, and an unlisted one is a `$ref` that resolves to nothing
        // (`openapi_well_formed`).
        maidan_types::ApiTokenId,
        maidan_types::ApprovalGateId,
        maidan_types::ApprovalGateState,
        maidan_types::ArtifactId,
        maidan_types::AuditEvent,
        maidan_types::ChannelId,
        maidan_types::ClaimLeaseId,
        maidan_types::DelegationGrantId,
        maidan_types::DlqEntryId,
        maidan_types::EgressOutboxId,
        maidan_types::EgressTargetId,
        crate::health::EmbeddingStatus,
        maidan_types::FsmHookId,
        maidan_types::GroupDmConversation,
        maidan_types::GroupDmConversationId,
        maidan_types::MemberId,
        maidan_types::MemoryBlockId,
        crate::dto::MentionWebhookConfig,
        maidan_types::MessageId,
        maidan_types::NotificationId,
        maidan_types::OpenGroupDmBody,
        maidan_types::PeerId,
        maidan_types::Pin,
        maidan_types::PostDmMessage,
        maidan_types::PushSubscriptionId,
        maidan_types::Reaction,
        maidan_types::RecipeId,
        maidan_types::RecipeRunId,
        maidan_types::RelationKind,
        maidan_types::ResultDeliveryId,
        maidan_types::SecretId,
        crate::dto::SetMentionWebhook,
        maidan_types::ShareTicket,
        maidan_types::ShareTicketId,
        maidan_types::SlashCommandId,
        maidan_types::SlashHandlerKind,
        maidan_types::TaskScheduleId,
        maidan_types::ThreadId,
        maidan_types::TokenQuota,
        maidan_types::WebhookSubscriptionId,
        crate::federation::WellKnownMcp,
        maidan_types::WorkspaceId,
        maidan_types::WorkspacePurgeResult,
        maidan_types::ArtifactErasure,
        HealthResponse,
        SubsystemStatus,
        OperatorStatus,
        StatusCheck,
        SearchProgress,
        ReplicaStatus,
        QueueStatus,
        ProblemDetails,
        LandGateAdviceRequest,
        LandGateAdvice,
        LandGateAdviceThresholds,
        LandGateAdviceUsage,
        Workspace,
        WorkspaceEraseResult,
        WorkspaceUsage,
        TombstoneRecord,
        TombstoneEntityKind,
        MessageBacklinks,
        KindCensus,
        KindCount,
        SetWipLimit,
        WipLimitView,
        SetDelegationPolicy,
        DelegationPolicy,
        SetApprovalPolicy,
        ApprovalPolicy,
        ApprovalRisk,
        GateDecisionVia,
        ClientIdentitySource,
        ModelRequestView,
        RetentionDays,
        RetentionPolicy,
        MemberWipView,
        WhoAmI,
        LinkSlackChannel,
        SlackChannelLink,
        LinkGithubIssue,
        GithubIssueLink,
        DeadEgress,
        AllowEgressTarget,
        AllowedEgressTarget,
        EgressSurface,
        Member,
        MemberKind,
        Channel,
        maidan_types::ChannelMember,
        maidan_types::ChannelMemberRole,
        crate::dto::AddChannelMember,
        Thread,
        ThreadState,
        Message,
        ContentBlock,
        MessageEdit,
        Mention,
        Vote,
        VoteKind,
        Reference,
        RefSide,
        Artifact,
        ArtifactKind,
        StoredEvent,
        EventKind,
        EventLink,
        ChainVerifyReport,
        ChainBreakReason,
        LogSnapshot,
        CatchUpPage,
        SearchHit,
        maidan_types::ReindexJob,
        maidan_types::ReindexJobStatus,
        crate::reindex_ops::StartReindexEmbeddings,
        EraseWorkspace,
        CreateChannel,
        CreateThread,
        ThreadTransition,
        ThreadContext,
        crate::thread_context::MessageEditView,
        BootPack,
        maidan_types::PackElision,
        maidan_types::ParentGrounding,
        maidan_types::AcceptedDecision,
        WorkspaceContext,
        ToolTranscript,
        ToolCallEntry,
        ToolCallResult,
        OrphanToolResult,
        TransitionThread,
        TransitionAction,
        AssignThread,
        SetThreadOwner,
        RenameThread,
        maidan_types::ThreadBudget, BudgetPatch,
        maidan_types::AccountedUsageRequest,
        maidan_types::TokenUsage,
        maidan_types::PriceSnapshot,
        maidan_types::PayerStamp,
        maidan_types::UsageLedgerEntry,
        maidan_types::DlqEntry,
        ClaimThread,
        ClaimNextThread,
        ClaimedThread,
        StrongRef,
        RenewClaim,
        AcknowledgeClaim,
        ReleaseClaim,
        UnassignThread,
        ThreadClaimResult,
        QueueDepth,
        maidan_types::ThreadUnclaimable,
        MarkUnclaimable,
        maidan_types::BlockedReason,
        maidan_types::ThreadBlock,
        ThreadVersion,
        ReviewPacket,
        EvidenceManifest,
        ResultEvidence,
        EvidenceAttestation,
        AttestationTier,
        EvidenceKind,
        ThreadArtifact,
        SetThreadBlock,
        maidan_types::ThreadWait,
        maidan_types::EscalationPolicy,
        maidan_types::ThreadPriority,
        SetThreadPriority,
        maidan_types::LegalHold,
        maidan_types::LegalHoldId,
        maidan_types::PreservedMessage,
        PlaceLegalHold,
        maidan_types::PushSubscription,
        RegisterPushSubscription,
        VapidPublicKey,
        PushKeys,
        maidan_types::WaitingInbox,
        maidan_types::WaitingItem,
        maidan_types::WaitingKind,
        SetThreadWait,
        ChannelOccupancy,
        AddThreadDependency,
        ThreadDependenciesView,
        ThreadDependency,
        TaskSchedule,
        CreateTaskSchedule,
        Recipe,
        RecipeSpec,
        RecipeParam,
        RecipeChild,
        RecipeRetry,
        RecipeRun,
        CreateRecipe,
        InstantiateRecipe,
        Secret,
        CreateSecret,
        SecretValue,
        SecretEgressHost,
        AllowSecretEgressHost,
        MemberFreeze,
        FreezeMember,
        FreezeResult,
        MemoryBlock,
        CreateMemoryBlock,
        SetMemoryBlockValue,
        ReviewDecision,
        ThreadReview,
        ReviewVerdict,
        ThreadReviewRequirement,
        ReviewStatus,
        SetReviewRequirement,
        AddReviewer,
        SubmitReview,
        LandGateStatus,
        LandColor,
        LandGatePointer,
        LandGateStanding,
        LandGateVerdict,
        SetLandGate,
        SetSpawnBudget,
        SpawnBudgetView,
        SetTaskScheduleActive,
        AddSkill,
        MemberSkill,
        ThreadRequiredSkill,
        SetThreadResult,
        ThreadResult,
        SetThreadLineage,
        ThreadLineage,
        RunOccupancy,
        ResultDelivery,
        SetThreadSteer,
        ThreadSteer,
        ChildThreadSummary,
        ApprovalGate,
        ApprovalGateView,
        AnswerApprovalGate,
        GlossaryTerm,
        SetGlossaryTerm,
        CreateMessage,
        EditMessageRequest,
        CreateMention,
        SeedFromMessage,
        CreateVote,
        CreateReaction,
        RemoveReaction,
        RetractVote,
        PinMessage,
        CreateReference,
        InboxItem,
        InboxItemKind,
        MemberInbox,
        MarkInboxRead,
        Notification,
        maidan_types::NotificationThreadGroup,
        maidan_types::BuriedDecision,
        maidan_types::ManagerDigest,
        maidan_types::ManagerDigestChannel,
        NotificationPref,
        SetNotificationPref,
        ChannelFollow,
        ThreadFollow,
        MemberFollow,
        MemberOccupancy,
        OccupancyPresence,
        FollowChannel,
        FollowThread,
        FollowMember,
        MemberEmail,
        SetEmail,
        EmailDeliveryMode,
        SetDeliveryMode,
        DeliveryModeView,
        UnreadCount,
        SnoozeNotification,
        ImportMode,
        ImportResult,
        VerifyExportResult,
        ExportPublicKey,
        maidan_types::TokenPolicy,
        MarkAllRead,
        SearchMode,
        MintApiToken,
        MintApiTokenResponse,
        AttenuateToken,
        DelegateToken,
        DelegateTokenResponse,
        CreateDelegationGrant,
        maidan_types::DelegationGrant,
        CapabilitySetView,
        SetWorkspaceHandle,
        maidan_types::RoomCard,
        maidan_types::RoomDiscovery,
        maidan_types::WorkspaceHandle,
        ApiTokenSummary,
        CreateShareTicket,
        ShareTicketResponse,
        MintShareTicketResponse,
        SharedArtifact,
        ShareManifest,
        SharedThread,
        SharedThreadPage,
        SharedMessage,
        SharedMessagePage,
        CreatePeer,
        PeerResponse,
        MintPeerResponse,
        CreateWebhook,
        WebhookResponse,
        MintWebhookResponse,
        CreateSlashCommand,
        SlashCommandResponse,
        MintSlashCommandResponse,
        CreateFsmHook,
        FsmHookResponse,
        MintFsmHookResponse,
        WellKnownMaidan,
        WellKnownA2a,
        WellKnownAuth,
        IngestSummary,
        SessionResponse,
        SessionWorkspace,
        SessionWorkspaces,
    )),
    modifiers(
        &SecurityAddon,
        &MiddlewareResponses,
        &ExtractorResponses,
        &IdempotencyResponses
    ),
    tags(
        (name = "health", description = "Liveness and readiness"),
        (name = "workspaces", description = "Workspaces and event log"),
        (name = "members", description = "Members and mentions"),
        (name = "channels", description = "Channels"),
        (name = "threads", description = "Threads and FSM transitions"),
        (name = "messages", description = "Messages, votes, mentions"),
        (name = "artifacts", description = "Content-addressed blobs"),
        (name = "references", description = "Cross-entity references"),
        (name = "search", description = "Lexical and semantic search"),
        (name = "tokens", description = "API token mint and revoke"),
        (name = "share", description = "Time-boxed read-only cross-organization sharing"),
        (name = "federation", description = "A2A federation and peers"),
        (name = "fsm", description = "FSM automation hooks on thread state transitions"),
        (name = "apps", description = "Agent app registration and installations"),
        (name = "dm", description = "Direct message conversations"),
        (name = "automation", description = "Automation delivery operator API"),
        (name = "auth", description = "OIDC login and browser session (requires MAIDAN_OIDC_ENABLED)"),
    )
)]
pub struct ApiDoc;

#[cfg(feature = "bootstrap")]
#[derive(OpenApi)]
#[openapi(
    paths(
        paths::create_workspace,
        paths::create_member_bootstrap,
    ),
    modifiers(&MiddlewareResponses, &ExtractorResponses),
    tags(
        (name = "bootstrap", description = "Unauthenticated seed routes (require MAIDAN_BOOTSTRAP=1 when auth is enabled)"),
    )
)]
pub struct BootstrapApiDoc;

/// The document `GET /openapi.json` serves: the stable surface, plus the
/// bootstrap routes when this build carries them.
pub fn document() -> utoipa::openapi::OpenApi {
    #[cfg(feature = "bootstrap")]
    {
        let mut doc = ApiDoc::openapi();
        doc.merge(BootstrapApiDoc::openapi());
        doc
    }
    #[cfg(not(feature = "bootstrap"))]
    {
        ApiDoc::openapi()
    }
}

/// `GET /openapi.json`
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(document())
}
