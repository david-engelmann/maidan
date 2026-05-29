//! REST API path docs (mirrors `app.rs` routes; WS/MCP excluded).

use uuid::Uuid;

use crate::dto::*;
use crate::error::ProblemDetails;
use crate::federation::{IngestSummary, WellKnownMaidan};
use crate::openapi::schemas::SearchHit;
use maidan_types::*;

// --- bootstrap (no bearer) ---

#[utoipa::path(post, path = "/workspaces", tag = "bootstrap",
    request_body = CreateWorkspace,
    responses((status = 201, description = "Created", body = Workspace)))]
pub fn create_workspace() {}

#[utoipa::path(post, path = "/workspaces/{wid}/members", tag = "bootstrap",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateMember,
    responses((status = 201, description = "Created", body = Member)))]
pub fn create_member_bootstrap() {}

// --- workspaces ---

#[utoipa::path(get, path = "/workspaces/{id}", tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Workspace),
        (status = 401, body = ProblemDetails),
        (status = 403, body = ProblemDetails),
        (status = 404, body = ProblemDetails)
    ))]
pub fn get_workspace() {}

#[utoipa::path(get, path = "/workspaces/{wid}/events", tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ListEventsQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<StoredEvent>)))]
pub fn list_events() {}

#[utoipa::path(get, path = "/workspaces/{wid}/search", tag = "search",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        SearchQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<SearchHit>)))]
pub fn search_messages() {}

#[utoipa::path(get, path = "/workspaces/{wid}/members", tag = "members",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Member>)))]
pub fn list_members() {}

#[utoipa::path(post, path = "/workspaces/{wid}/members/{mid}/tokens", tag = "tokens",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("mid" = Uuid, Path, description = "Member id"),
    ),
    request_body = MintApiToken,
    security(("bearerAuth" = [])),
    responses((status = 201, body = MintApiTokenResponse)))]
pub fn mint_api_token() {}

#[utoipa::path(get, path = "/workspaces/{wid}/channels", tag = "channels",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Channel>)))]
pub fn list_channels() {}

#[utoipa::path(post, path = "/workspaces/{wid}/channels", tag = "channels",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateChannel,
    security(("bearerAuth" = [])),
    responses((status = 201, body = Channel)))]
pub fn create_channel() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/peers",
    tag = "federation",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<PeerResponse>))
)]
pub fn list_peers() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/peers",
    tag = "federation",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreatePeer,
    security(("bearerAuth" = [])),
    responses((status = 201, body = MintPeerResponse))
)]
pub fn create_peer() {}

#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/peers/{pid}",
    tag = "federation",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("pid" = Uuid, Path, description = "Peer id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Deleted"))
)]
pub fn delete_peer() {}

// --- members ---

#[utoipa::path(get, path = "/members/{id}", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Member)))]
pub fn get_member() {}

#[utoipa::path(get, path = "/members/{id}/mentions", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ListMentionsQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Mention>)))]
pub fn list_mentions_for_member() {}

#[utoipa::path(get, path = "/members/{id}/inbox", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ListInboxQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = MemberInbox)))]
pub fn get_member_inbox() {}

#[utoipa::path(post, path = "/members/{id}/inbox/read", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = MarkInboxRead,
    security(("bearerAuth" = [])),
    responses((status = 200, body = MemberInbox)))]
pub fn mark_member_inbox_read() {}

// --- channels ---

#[utoipa::path(get, path = "/channels/{id}", tag = "channels",
    params(("id" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Channel)))]
pub fn get_channel() {}

#[utoipa::path(get, path = "/channels/{cid}/threads", tag = "threads",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Thread>)))]
pub fn list_threads() {}

#[utoipa::path(post, path = "/channels/{cid}/threads", tag = "threads",
    params(("cid" = Uuid, Path, description = "Channel id")),
    request_body = CreateThread,
    security(("bearerAuth" = [])),
    responses((status = 201, body = Thread)))]
pub fn create_thread() {}

// --- threads ---

#[utoipa::path(get, path = "/threads/{id}", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Thread)))]
pub fn get_thread() {}

#[utoipa::path(post, path = "/threads/{id}", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = TransitionThread,
    security(("bearerAuth" = [])),
    responses((status = 200, body = Thread)))]
pub fn transition_thread() {}

#[utoipa::path(get, path = "/threads/{tid}/messages", tag = "messages",
    params(
        ("tid" = Uuid, Path, description = "Thread id"),
        ListMessagesQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Message>)))]
pub fn list_messages() {}

#[utoipa::path(post, path = "/threads/{tid}/messages", tag = "messages",
    params(("tid" = Uuid, Path, description = "Thread id")),
    request_body = CreateMessage,
    security(("bearerAuth" = [])),
    responses((status = 201, body = Message)))]
pub fn post_message() {}

// --- messages ---

#[utoipa::path(get, path = "/messages/{id}", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Message)))]
pub fn get_message() {}

#[utoipa::path(patch, path = "/messages/{id}", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = EditMessageRequest,
    security(("bearerAuth" = [])),
    responses((status = 200, body = Message)))]
pub fn edit_message() {}

#[utoipa::path(get, path = "/messages/{id}/edits", tag = "messages",
    params(
        ("id" = Uuid, Path, description = "Message id"),
        crate::dto::ListMessageEditsQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<MessageEdit>)))]
pub fn list_message_edits() {}

#[utoipa::path(delete, path = "/messages/{id}", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Tombstoned")))]
pub fn tombstone_message() {}

#[utoipa::path(post, path = "/messages/{id}/mentions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = CreateMention,
    security(("bearerAuth" = [])),
    responses((status = 201, body = Mention)))]
pub fn create_mention() {}

#[utoipa::path(post, path = "/messages/{id}/votes", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = CreateVote,
    security(("bearerAuth" = [])),
    responses((status = 201, body = Vote)))]
pub fn cast_vote() {}

#[utoipa::path(get, path = "/messages/{id}/votes", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Vote>)))]
pub fn list_votes() {}

#[utoipa::path(post, path = "/messages/{id}/reactions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = CreateReaction,
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn add_reaction() {}

#[utoipa::path(delete, path = "/messages/{id}/reactions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = RemoveReaction,
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn remove_reaction() {}

#[utoipa::path(get, path = "/messages/{id}/reactions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Reaction>)))]
pub fn list_reactions() {}

#[utoipa::path(post, path = "/threads/{id}/pins", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = PinMessage,
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn pin_message() {}

#[utoipa::path(delete, path = "/threads/{id}/pins", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = PinMessage,
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn unpin_message() {}

#[utoipa::path(get, path = "/threads/{id}/pins", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Pin>)))]
pub fn list_pins() {}

// --- artifacts ---

#[utoipa::path(post, path = "/artifacts", tag = "artifacts",
    params(UploadArtifactQuery),
    request_body(content = String, description = "Raw bytes", content_type = "application/octet-stream"),
    security(("bearerAuth" = [])),
    responses((status = 201, body = Artifact)))]
pub fn upload_artifact() {}

#[utoipa::path(get, path = "/artifacts/{sha}", tag = "artifacts",
    params(("sha" = String, Path, description = "SHA-256 hex")),
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Raw bytes", content_type = "application/octet-stream")))]
pub fn get_artifact() {}

#[utoipa::path(get, path = "/artifacts/{sha}/meta", tag = "artifacts",
    params(("sha" = String, Path, description = "SHA-256 hex")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Artifact)))]
pub fn get_artifact_metadata() {}

// --- references ---

#[utoipa::path(post, path = "/references", tag = "references",
    request_body = CreateReference,
    security(("bearerAuth" = [])),
    responses((status = 201, body = Reference)))]
pub fn create_reference() {}

#[utoipa::path(get, path = "/references", tag = "references",
    params(ListReferencesQuery),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Reference>)))]
pub fn list_references() {}

// --- tokens ---

#[utoipa::path(delete, path = "/tokens/{id}", tag = "tokens",
    params(("id" = Uuid, Path, description = "API token id")),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Revoked")))]
pub fn revoke_api_token() {}

// --- federation ---

#[utoipa::path(get, path = "/.well-known/maidan.json", tag = "federation",
    responses((status = 200, body = WellKnownMaidan)))]
pub fn well_known() {}

#[utoipa::path(
    post,
    path = "/a2a/v1/events",
    tag = "federation",
    security(("bearerAuth" = [])),
    request_body(content = String, description = "FederatedEventBatch JSON", content_type = "application/json"),
    responses((status = 200, body = IngestSummary))
)]
pub fn ingest_events() {}
