//! OpenAPI 3.0 document for the Maidan HTTP API (Track W.1).

mod paths;
mod schemas;

use axum::Json;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::dto::*;
use crate::error::ProblemDetails;
use crate::federation::{IngestSummary, WellKnownA2a, WellKnownMaidan};
use crate::health::{HealthResponse, SubsystemStatus};
use crate::openapi::schemas::{LivenessOk, SearchHit};
use crate::thread_context::{ThreadContext, ThreadFsmContext, WorkspaceContext};
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
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("maidan_session"))),
        );
    }
}

/// Generated OpenAPI document (stable `v1.0.0` HTTP surface).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Maidan API",
        version = "2.1.0",
        description = "Slack-shaped collaboration API for AI agents. Human login uses OIDC + `maidan_session` cookie (see `auth` tag).\n\n\
            WebSocket GET /ws/subscribe: after upgrade, send one JSON text frame with `filter`, optional `after_id`, optional `resume_token` (replaces filter and after_id), and optional bearer `token` when auth is enabled. Control frames: `subscribe_ack` (resume_token + after_id watermark), `replay_hint`, `replay_truncated` (after_id + limit 500), then event envelopes with log_id.\n\n\
            MCP SSE GET /mcp/stream: query workspace_id, after_id, or resume_token; narrow with channel_id, thread_id, member_id, and kinds (comma-separated snake_case event kinds, e.g. mention_recorded) so an agent can await just its mentions or one thread (Cluster 150); same control frames; requires bearer event:subscribe.\n\
            MCP SSE GET /mcp/notifications: JSON-RPC notifications (e.g. notifications/resources/updated); requires workspace:read.\n\
            MCP streamable HTTP POST /mcp/streamable: first request opens SSE + `Mcp-Session-Id`; follow-up POSTs on open session return `202 Accepted` and multiplex JSON-RPC on the SSE stream (Cluster 78). A client that sends `Accept: application/json` (no `text/event-stream`) instead gets a single JSON body. GET /mcp/streamable opens a server→client SSE stream (unsolicited notifications) for an open `Mcp-Session-Id` (Cluster 146). Supports JSON-RPC batches and notifications (202); `MCP-Protocol-Version` header validated (Cluster 145). SSE frames carry an `id:`; reconnect with `Last-Event-ID` to replay retained frames (Cluster 147). The server may issue requests to the client (sampling / roots / elicitation) on the session's stream, gated on the client's declared capabilities; the client's response is POSTed back as a JSON-RPC response (Cluster 148).\n\n\
            GET /metrics: Prometheus exposition (HTTP latency + maidan_bus_lag_total, maidan_subscribe_replay_total, maidan_indexer_last_event_age_seconds, maidan_bus_listener_ok, maidan_bus_notify_hydrate_total, maidan_outbox_pending, maidan_outbox_quarantined, maidan_outbox_oldest_pending_seconds, maidan_outbox_relay_total{result} on Postgres). Fixed label cardinality only.",
        license(name = "MIT OR Apache-2.0", url = "https://github.com/david-engelmann/maidan")
    ),
    paths(
        paths::health_live,
        paths::health_ready,
        paths::health,
        paths::get_workspace,
        paths::purge_workspace,
        paths::erase_workspace,
        paths::export_workspace,
        paths::list_workspace_audit,
        paths::get_workspace_context,
        paths::list_quarantined_outbox,
        paths::replay_quarantined_outbox,
        paths::list_events,
        paths::search_messages,
        paths::list_members,
        paths::mint_api_token,
        paths::list_api_tokens,
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
        paths::start_reindex_embeddings,
        paths::get_reindex_embeddings_job,
        paths::get_member,
        paths::list_mentions_for_member,
        paths::get_member_inbox,
        paths::mark_member_inbox_read,
        paths::get_channel,
        paths::add_channel_member,
        paths::list_channel_members,
        paths::remove_channel_member,
        paths::list_threads,
        paths::create_thread,
        paths::get_thread,
        paths::get_thread_context,
        paths::transition_thread,
        paths::assign_thread,
        paths::unassign_thread,
        paths::claim_thread,
        paths::list_messages,
        paths::post_message,
        paths::get_message,
        paths::edit_message,
        paths::list_message_edits,
        paths::tombstone_message,
        paths::purge_message,
        paths::create_mention,
        paths::cast_vote,
        paths::list_votes,
        paths::add_reaction,
        paths::remove_reaction,
        paths::list_reactions,
        paths::pin_message,
        paths::unpin_message,
        paths::list_pins,
        paths::upload_artifact,
        paths::get_artifact,
        paths::get_artifact_metadata,
        paths::begin_multipart_artifact_doc,
        paths::abort_multipart_artifact_doc,
        paths::complete_multipart_artifact_doc,
        paths::upload_multipart_artifact_part_doc,
        paths::create_reference,
        paths::list_references,
        paths::revoke_api_token,
        paths::well_known,
        paths::ingest_events,
        paths::oidc_login,
        paths::oidc_callback,
        paths::oidc_logout,
        paths::get_auth_session,
        paths::mint_auth_session_token,
        paths::ui_list_events,
        paths::ui_list_channels,
        paths::ui_create_channel,
        paths::ui_create_thread,
        paths::ui_post_message,
        paths::ui_list_threads,
        paths::ui_list_messages,
        paths::ui_search_messages,
        paths::ui_list_audit,
        paths::ui_list_peers,
        paths::ui_list_message_edits,
    ),
    components(schemas(
        LivenessOk,
        HealthResponse,
        SubsystemStatus,
        ProblemDetails,
        Workspace,
        WorkspaceEraseResult,
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
        Reference,
        RefSide,
        Artifact,
        ArtifactKind,
        StoredEvent,
        EventKind,
        SearchHit,
        maidan_types::ReindexJob,
        maidan_types::ReindexJobStatus,
        crate::reindex_ops::StartReindexEmbeddings,
        CreateWorkspace,
        EraseWorkspace,
        CreateMember,
        CreateChannel,
        CreateThread,
        ThreadTransition,
        ThreadContext,
        crate::thread_context::MessageEditView,
        ThreadFsmContext,
        WorkspaceContext,
        ThreadContextQuery,
        WorkspaceContextQuery,
        TransitionThread,
        AssignThread,
        ClaimThread,
        UnassignThread,
        ThreadClaimResult,
        CreateMessage,
        EditMessageRequest,
        CreateMention,
        CreateVote,
        CreateReaction,
        RemoveReaction,
        PinMessage,
        CreateReference,
        ListEventsQuery,
        ListMessagesQuery,
        ListMessageEditsQuery,
        ListMentionsQuery,
        ListInboxQuery,
        InboxItem,
        InboxItemKind,
        MemberInbox,
        MarkInboxRead,
        ListReferencesQuery,
        SearchMode,
        SearchQuery,
        UploadArtifactQuery,
        MintApiToken,
        MintApiTokenResponse,
        ApiTokenSummary,
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
        IngestSummary,
        OidcLoginQuery,
        OidcCallbackQuery,
        SessionResponse,
    )),
    modifiers(&SecurityAddon),
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
    tags(
        (name = "bootstrap", description = "Unauthenticated seed routes (require MAIDAN_BOOTSTRAP=1 when auth is enabled)"),
    )
)]
pub struct BootstrapApiDoc;

/// `GET /openapi.json`
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    #[cfg(feature = "bootstrap")]
    {
        let mut doc = ApiDoc::openapi();
        doc.merge(BootstrapApiDoc::openapi());
        Json(doc)
    }
    #[cfg(not(feature = "bootstrap"))]
    {
        Json(ApiDoc::openapi())
    }
}
