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
use crate::thread_context::{ThreadContext, ThreadFsmContext};
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
            MCP SSE GET /mcp/stream: query workspace_id, after_id, or resume_token; same control frames; requires bearer event:subscribe.\n\
            MCP SSE GET /mcp/notifications: JSON-RPC notifications (e.g. notifications/resources/updated); requires workspace:read.\n\
            MCP streamable HTTP POST /mcp/streamable: first request opens SSE + `Mcp-Session-Id`; follow-up POSTs on open session return JSON and push to SSE (Cluster 35).\n\n\
            GET /metrics: Prometheus exposition (HTTP latency + maidan_bus_lag_total, maidan_subscribe_replay_total, maidan_indexer_last_event_age_seconds, maidan_bus_listener_ok, maidan_bus_notify_hydrate_total, maidan_outbox_pending, maidan_outbox_quarantined, maidan_outbox_oldest_pending_seconds, maidan_outbox_relay_total{result} on Postgres). Fixed label cardinality only.",
        license(name = "MIT OR Apache-2.0", url = "https://github.com/david-engelmann/maidan")
    ),
    paths(
        paths::health_live,
        paths::health_ready,
        paths::health,
        paths::create_workspace,
        paths::create_member_bootstrap,
        paths::get_workspace,
        paths::list_events,
        paths::search_messages,
        paths::list_members,
        paths::mint_api_token,
        paths::list_channels,
        paths::create_channel,
        paths::list_peers,
        paths::create_peer,
        paths::delete_peer,
        paths::list_webhooks,
        paths::create_webhook,
        paths::revoke_webhook,
        paths::list_slash_commands,
        paths::create_slash_command,
        paths::revoke_slash_command,
        paths::list_fsm_hooks,
        paths::create_fsm_hook,
        paths::revoke_fsm_hook,
        paths::get_member,
        paths::list_mentions_for_member,
        paths::get_member_inbox,
        paths::mark_member_inbox_read,
        paths::get_channel,
        paths::list_threads,
        paths::create_thread,
        paths::get_thread,
        paths::get_thread_context,
        paths::transition_thread,
        paths::list_messages,
        paths::post_message,
        paths::get_message,
        paths::edit_message,
        paths::list_message_edits,
        paths::tombstone_message,
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
        Member,
        MemberKind,
        Channel,
        Thread,
        ThreadState,
        Message,
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
        CreateWorkspace,
        CreateMember,
        CreateChannel,
        CreateThread,
        ThreadTransition,
        ThreadContext,
        ThreadFsmContext,
        ThreadContextQuery,
        TransitionThread,
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
        (name = "bootstrap", description = "Unauthenticated seed routes (require MAIDAN_BOOTSTRAP=1 when auth is enabled)"),
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
        (name = "auth", description = "OIDC login and browser session (requires MAIDAN_OIDC_ENABLED)"),
    )
)]
pub struct ApiDoc;

/// `GET /openapi.json`
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
