//! OpenAPI 3.0 document for the Maidan HTTP API (Track W.1).

mod paths;
mod schemas;

use axum::Json;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::dto::*;
use crate::error::ProblemDetails;
use crate::federation::{IngestSummary, WellKnownA2a, WellKnownMaidan};
use crate::health::{HealthResponse, SubsystemStatus};
use crate::openapi::schemas::{LivenessOk, SearchHit};
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
    }
}

/// Generated OpenAPI document (stable `v1.0.0` HTTP surface).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Maidan API",
        version = "1.1.0",
        description = "Slack-shaped collaboration API for AI agents. MCP (`POST /mcp`) and WebSocket (`GET /ws/subscribe`) are not fully described here.",
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
        paths::get_member,
        paths::list_mentions_for_member,
        paths::get_channel,
        paths::list_threads,
        paths::create_thread,
        paths::get_thread,
        paths::transition_thread,
        paths::list_messages,
        paths::post_message,
        paths::get_message,
        paths::tombstone_message,
        paths::create_mention,
        paths::cast_vote,
        paths::list_votes,
        paths::upload_artifact,
        paths::get_artifact,
        paths::get_artifact_metadata,
        paths::create_reference,
        paths::list_references,
        paths::revoke_api_token,
        paths::well_known,
        paths::ingest_events,
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
        TransitionThread,
        CreateMessage,
        CreateMention,
        CreateVote,
        CreateReference,
        ListEventsQuery,
        ListMessagesQuery,
        ListMentionsQuery,
        ListReferencesQuery,
        SearchMode,
        SearchQuery,
        UploadArtifactQuery,
        MintApiToken,
        MintApiTokenResponse,
        CreatePeer,
        PeerResponse,
        MintPeerResponse,
        WellKnownMaidan,
        WellKnownA2a,
        IngestSummary,
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
    )
)]
pub struct ApiDoc;

/// `GET /openapi.json`
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
