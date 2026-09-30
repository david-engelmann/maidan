//! REST API path docs (mirrors `app.rs` routes; WS/MCP excluded).

use crate::openapi::responses::*;
use maidan_types::DelegationGrant;
use uuid::Uuid;

use crate::dto::*;
use crate::error::ProblemDetails;
use crate::federation::{IngestSummary, WellKnownMaidan};
use crate::land_gate_advisor::{LandGateAdvice, LandGateAdviceRequest};
use crate::openapi::schemas::SearchHit;
use crate::share_consumer::*;
use crate::thread_context::ThreadContext;
use maidan_types::*;

// --- bootstrap (no bearer) ---

/// Create a workspace
#[cfg(feature = "bootstrap")]
#[utoipa::path(post, path = "/workspaces", tag = "bootstrap",
    request_body = CreateWorkspace,
    security(()),
    responses(
        (status = 201, description = "Created", body = Workspace),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    ))]
pub fn create_workspace() {}

/// Create a member
#[cfg(feature = "bootstrap")]
#[utoipa::path(post, path = "/workspaces/{wid}/members", tag = "bootstrap",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateMember,
    security(()),
    responses(
        (status = 201, description = "Created", body = Member),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    ))]
pub fn create_member_bootstrap() {}

// --- workspaces ---

/// Get a workspace
#[utoipa::path(get, path = "/workspaces/{id}", tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Workspace),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_workspace() {}

/// Erase a workspace
#[utoipa::path(
    delete,
    path = "/workspaces/{id}",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = EraseWorkspace,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WorkspaceEraseResult),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    )
)]
pub fn erase_workspace() {}

/// List a workspace's events
#[utoipa::path(get, path = "/workspaces/{wid}/events", tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ListEventsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<StoredEvent>),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, description = "Cursor too old; must_refetch", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn list_events() {}

/// Verify a workspace's event chain
#[utoipa::path(get, path = "/workspaces/{wid}/events/verify", tag = "workspaces",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ChainVerifyReport),
        (status = 403, response = Forbidden),
        (status = 409, description = "Event log chain broken", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn verify_event_chain() {}

/// Get a workspace's log snapshot
#[utoipa::path(get, path = "/workspaces/{wid}/snapshot", tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        LogSnapshotQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = LogSnapshot),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_log_snapshot() {}

/// Catch up on a workspace's events
#[utoipa::path(get, path = "/workspaces/{wid}/events/catch-up", tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        CatchUpQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = CatchUpPage),
        (status = 403, response = Forbidden),
        (status = 409, description = "Cursor too old or event log chain broken", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn catch_up_events() {}

/// Search a workspace's messages
#[utoipa::path(get, path = "/workspaces/{wid}/search", tag = "search",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        SearchQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<SearchHit>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn search_messages() {}

/// List a workspace's members
#[utoipa::path(get, path = "/workspaces/{wid}/members", tag = "members",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Member>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_members() {}

/// Mint an API token for a member
#[utoipa::path(post, path = "/workspaces/{wid}/members/{mid}/tokens", tag = "tokens",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("mid" = Uuid, Path, description = "Member id"),
    ),
    request_body = MintApiToken,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintApiTokenResponse),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn mint_api_token() {}

/// List a member's API tokens
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/members/{mid}/tokens",
    tag = "tokens",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("mid" = Uuid, Path, description = "Member id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ApiTokenSummary>),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn list_api_tokens() {}

/// Create a delegation grant
#[utoipa::path(post, path = "/workspaces/{wid}/delegation-grants", tag = "tokens",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateDelegationGrant,
    security(("bearerAuth" = ["token:admin"])),
    responses(
        (status = 201, body = DelegationGrant),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn create_delegation_grant() {}

/// List a workspace's delegation grants
#[utoipa::path(get, path = "/workspaces/{wid}/delegation-grants", tag = "tokens",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = ["token:admin"])),
    responses(
        (status = 200, body = Vec<DelegationGrant>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_delegation_grants() {}

/// Revoke a delegation grant
#[utoipa::path(delete, path = "/workspaces/{wid}/delegation-grants/{gid}", tag = "tokens",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("gid" = Uuid, Path, description = "Delegation grant id"),
    ),
    security(("bearerAuth" = ["token:admin"])),
    responses(
        (status = 200, body = DelegationGrant),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn revoke_delegation_grant() {}

/// Create a share ticket
#[utoipa::path(post, path = "/workspaces/{wid}/share-tickets", tag = "share",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateShareTicket,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintShareTicketResponse),
        (status = 400, description = "Invalid scope or expiry", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, response = Forbidden),
    ))]
pub fn create_share_ticket() {}

/// List a workspace's share tickets
#[utoipa::path(get, path = "/workspaces/{wid}/share-tickets", tag = "share",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ShareTicketResponse>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_share_tickets() {}

/// Revoke a share ticket
#[utoipa::path(delete, path = "/workspaces/{wid}/share-tickets/{tid}", tag = "share",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("tid" = Uuid, Path, description = "Share ticket id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 403, response = Forbidden),
        (status = 404, description = "Ticket not found", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn revoke_share_ticket() {}

/// Get a share ticket's manifest
#[utoipa::path(get, path = "/share/manifest", tag = "share",
    security(("shareTicketAuth" = [])),
    responses(
        (status = 200, body = ShareManifest),
        (status = 401, description = "Invalid, expired, or revoked ticket", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, response = NotFound),
    ))]
pub fn get_share_manifest() {}

/// List shared threads
#[utoipa::path(get, path = "/share/threads", tag = "share",
    params(SharePageQuery),
    security(("shareTicketAuth" = [])),
    responses(
        (status = 200, body = SharedThreadPage),
        (status = 401, description = "Invalid, expired, or revoked ticket", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_shared_threads() {}

/// List a shared thread's messages
#[utoipa::path(get, path = "/share/threads/{tid}/messages", tag = "share",
    params(("tid" = Uuid, Path, description = "Thread id in the shared channel"), SharePageQuery),
    security(("shareTicketAuth" = [])),
    responses(
        (status = 200, body = SharedMessagePage),
        (status = 403, response = Forbidden),
        (status = 404, description = "Thread outside the shared channel", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn list_shared_messages() {}

/// Download a shared artifact
#[utoipa::path(get, path = "/share/artifacts/{sha}", tag = "share",
    params(("sha" = String, Path, description = "Allowlisted artifact SHA-256")),
    security(("shareTicketAuth" = [])),
    responses(
        (status = 200, description = "Artifact bytes, with the same headers as `GET /artifacts/{sha}`"),
        (status = 404, description = "Artifact not allowlisted", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn download_shared_artifact() {}

/// List a workspace's channels
#[utoipa::path(get, path = "/workspaces/{wid}/channels", tag = "channels",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Channel>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_channels() {}

/// Create a channel
#[utoipa::path(post, path = "/workspaces/{wid}/channels", tag = "channels",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateChannel,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Channel),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    ))]
pub fn create_channel() {}

/// List a workspace's federation peers
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/peers",
    tag = "federation",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<PeerResponse>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_peers() {}

/// Register a federation peer
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/peers",
    tag = "federation",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreatePeer,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintPeerResponse),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn create_peer() {}

/// Remove a federation peer
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/peers/{pid}",
    tag = "federation",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("pid" = Uuid, Path, description = "Peer id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn delete_peer() {}

/// List a workspace's webhooks
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/webhooks",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<WebhookResponse>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_webhooks() {}

/// Create a webhook
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/webhooks",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateWebhook,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintWebhookResponse),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn create_webhook() {}

/// Revoke a webhook
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/webhooks/{whid}",
    tag = "webhooks",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("whid" = Uuid, Path, description = "Webhook subscription id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn revoke_webhook() {}

/// Get a workspace's mention webhook
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/mention-webhook",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MentionWebhookConfig),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_mention_webhook() {}

/// Set a workspace's mention webhook
#[utoipa::path(
    put,
    path = "/workspaces/{wid}/mention-webhook",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = SetMentionWebhook,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MentionWebhookConfig),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn set_mention_webhook() {}

/// List a workspace's slash commands
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/slash-commands",
    tag = "slash",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<SlashCommandResponse>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_slash_commands() {}

/// Create a slash command
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/slash-commands",
    tag = "slash",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateSlashCommand,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintSlashCommandResponse),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn create_slash_command() {}

/// Revoke a slash command
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/slash-commands/{cid}",
    tag = "slash",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("cid" = Uuid, Path, description = "Slash command id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn revoke_slash_command() {}

/// List a workspace's FSM hooks
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/fsm-hooks",
    tag = "fsm",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<FsmHookResponse>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_fsm_hooks() {}

/// Create an FSM hook
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/fsm-hooks",
    tag = "fsm",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateFsmHook,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintFsmHookResponse),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn create_fsm_hook() {}

/// Revoke an FSM hook
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/fsm-hooks/{hid}",
    tag = "fsm",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("hid" = Uuid, Path, description = "FSM hook id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn revoke_fsm_hook() {}

// --- members ---

/// Get a member
#[utoipa::path(get, path = "/members/{id}", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Member),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member() {}

/// List a member's mentions
#[utoipa::path(get, path = "/members/{id}/mentions", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ListMentionsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Mention>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_mentions_for_member() {}

/// Get a member's inbox
#[utoipa::path(get, path = "/members/{id}/inbox", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ListInboxQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemberInbox),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member_inbox() {}

/// Mark a member's inbox items read
#[utoipa::path(post, path = "/members/{id}/inbox/read", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = MarkInboxRead,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemberInbox),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn mark_member_inbox_read() {}

/// List a member's notifications
#[utoipa::path(get, path = "/members/{id}/notifications", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ListNotificationsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Notification>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_notifications() {}

/// List a member's notifications grouped by thread
#[utoipa::path(get, path = "/members/{id}/notifications/grouped", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ListNotificationsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [NotificationThreadGroup]),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_notifications_grouped() {}

/// List a member's buried decisions
#[utoipa::path(get, path = "/members/{id}/decisions", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        DecisionsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [BuriedDecision]),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_decisions() {}

/// Get a member's manager digest
#[utoipa::path(get, path = "/members/{id}/manager-digest", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ManagerDigestQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ManagerDigest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member_manager_digest() {}

/// Get what is waiting on a member
#[utoipa::path(get, path = "/members/{id}/waiting", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        WaitingQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WaitingInbox, description = "What is waiting on the member, oldest first"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member_waiting() {}

/// Count a member's unread notifications
#[utoipa::path(get, path = "/members/{id}/notifications/unread-count", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = UnreadCount),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn member_unread_notification_count() {}

/// Mark all of a member's notifications read
#[utoipa::path(post, path = "/members/{id}/notifications/read-all", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MarkAllRead),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn mark_all_member_notifications_read() {}

/// Mark a notification read
#[utoipa::path(post, path = "/members/{id}/notifications/{nid}/read", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("nid" = Uuid, Path, description = "Notification id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = UnreadCount),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn mark_member_notification_read() {}

/// Snooze a notification
#[utoipa::path(post, path = "/members/{id}/notifications/{nid}/snooze", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("nid" = Uuid, Path, description = "Notification id"),
    ),
    request_body = SnoozeNotification,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = UnreadCount),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, description = "Not this member's notification", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn snooze_member_notification() {}

/// Set a member's notification preference
#[utoipa::path(put, path = "/members/{id}/notification-prefs", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = SetNotificationPref,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = NotificationPref),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_member_notification_pref() {}

/// List a member's notification preferences
#[utoipa::path(get, path = "/members/{id}/notification-prefs", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<NotificationPref>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_notification_prefs() {}

/// Follow a channel
#[utoipa::path(post, path = "/members/{id}/channel-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = FollowChannel,
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Following"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn follow_member_channel() {}

/// Unfollow a channel
#[utoipa::path(delete, path = "/members/{id}/channel-follows/{cid}", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("cid" = Uuid, Path, description = "Channel id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Unfollowed"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn unfollow_member_channel() {}

/// List the channels a member follows
#[utoipa::path(get, path = "/members/{id}/channel-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ChannelFollow>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_channel_follows() {}

/// Follow a thread
#[utoipa::path(post, path = "/members/{id}/thread-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = FollowThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Following"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn follow_member_thread() {}

/// Unfollow a thread
#[utoipa::path(delete, path = "/members/{id}/thread-follows/{tid}", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("tid" = Uuid, Path, description = "Thread id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Unfollowed"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn unfollow_member_thread() {}

/// List the threads a member follows
#[utoipa::path(get, path = "/members/{id}/thread-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ThreadFollow>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_thread_follows() {}

/// Follow a member's occupancy
#[utoipa::path(post, path = "/members/{id}/member-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Follower member id")),
    request_body = FollowMember,
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Following"),
        (status = 400, description = "Self-follow", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn follow_member_occupancy() {}

/// Unfollow a member's occupancy
#[utoipa::path(delete, path = "/members/{id}/member-follows/{followed_id}", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Follower member id"),
        ("followed_id" = Uuid, Path, description = "Followed member id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Unfollowed"),
        (status = 403, response = Forbidden),
        (status = 404, description = "Not following", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn unfollow_member_occupancy() {}

/// List the members whose occupancy a member follows
#[utoipa::path(get, path = "/members/{id}/member-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Follower member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<MemberFollow>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_occupancy_follows() {}

/// Get a member's occupancy
#[utoipa::path(get, path = "/members/{id}/occupancy", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemberOccupancy),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member_occupancy() {}

/// Set a member's delivery email
#[utoipa::path(put, path = "/members/{id}/email", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = SetEmail,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemberEmail),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_member_email() {}

/// Get a member's delivery email
#[utoipa::path(get, path = "/members/{id}/email", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemberEmail),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member_email() {}

/// Clear a member's delivery email
#[utoipa::path(delete, path = "/members/{id}/email", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Cleared"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn delete_member_email() {}

/// Set a member's email delivery mode
#[utoipa::path(put, path = "/members/{id}/delivery-mode", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = SetDeliveryMode,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = DeliveryModeView),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_member_delivery_mode() {}

/// Get a member's email delivery mode
#[utoipa::path(get, path = "/members/{id}/delivery-mode", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = DeliveryModeView),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member_delivery_mode() {}

/// Register a Web Push subscription
#[utoipa::path(post, path = "/members/{id}/push-subscriptions", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = RegisterPushSubscription,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = PushSubscription, description = "The registered Web Push subscription"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn register_push_subscription() {}

/// List a member's Web Push subscriptions
#[utoipa::path(get, path = "/members/{id}/push-subscriptions", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [PushSubscription], description = "The member's Web Push subscriptions"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_push_subscriptions() {}

/// Remove a Web Push subscription
#[utoipa::path(delete, path = "/members/{id}/push-subscriptions/{sub_id}", tag = "members",
    params(("id" = Uuid, Path, description = "Member id"), ("sub_id" = Uuid, Path, description = "Subscription id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, response = Forbidden),
        (status = 404, description = "No such subscription", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn delete_push_subscription() {}

// --- channels ---

/// Get a channel
#[utoipa::path(get, path = "/channels/{id}", tag = "channels",
    params(("id" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Channel),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_channel() {}

/// Get a channel's task-queue depth
#[utoipa::path(get, path = "/channels/{cid}/queue-depth", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = QueueDepth),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_channel_queue_depth() {}

/// List a channel's unclaimable threads
#[utoipa::path(get, path = "/channels/{cid}/unclaimable", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ThreadUnclaimable>, description = "Parked threads, newest first"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_channel_unclaimable() {}

/// List a channel's blocked threads
#[utoipa::path(get, path = "/channels/{cid}/blocked", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ThreadBlock>, description = "Explicitly blocked threads, newest first"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_channel_blocked() {}

/// Get a channel's occupancy
#[utoipa::path(get, path = "/channels/{cid}/occupancy", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ChannelOccupancy),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_channel_occupancy() {}

/// Mute a channel for the caller
#[utoipa::path(post, path = "/channels/{cid}/mute", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Channel muted for the caller"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn mute_channel() {}

/// Unmute a channel for the caller
#[utoipa::path(delete, path = "/channels/{cid}/mute", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Channel unmuted"),
        (status = 403, response = Forbidden),
        (status = 404, description = "Was not muted", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn unmute_channel() {}

/// List a channel's dead-lettered runs
#[utoipa::path(get, path = "/channels/{cid}/dlq", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id"), DlqQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [DlqEntry]),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_channel_dlq() {}

/// Add a member to a channel
#[utoipa::path(post, path = "/channels/{cid}/members", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    request_body = AddChannelMember,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = ChannelMember),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn add_channel_member() {}

/// List a channel's members
#[utoipa::path(get, path = "/channels/{cid}/members", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ChannelMember>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_channel_members() {}

/// Remove a member from a channel
#[utoipa::path(delete, path = "/channels/{cid}/members/{mid}", tag = "channels",
    params(
        ("cid" = Uuid, Path, description = "Channel id"),
        ("mid" = Uuid, Path, description = "Member id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "removed"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn remove_channel_member() {}

/// List a channel's threads
#[utoipa::path(get, path = "/channels/{cid}/threads", tag = "threads",
    params(("cid" = Uuid, Path, description = "Channel id"), ListThreadsQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Thread>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_threads() {}

/// List a channel's recently active threads
#[utoipa::path(get, path = "/channels/{cid}/recent-threads", tag = "threads",
    params(("cid" = Uuid, Path, description = "Channel id"), ListThreadsQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Thread>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_recently_active_threads() {}

/// Create a thread in a channel
#[utoipa::path(post, path = "/channels/{cid}/threads", tag = "threads",
    params(("cid" = Uuid, Path, description = "Channel id")),
    request_body = CreateThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Thread),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn create_thread() {}

// --- threads ---

/// Get a thread
#[utoipa::path(get, path = "/threads/{id}", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_thread() {}

/// Get a thread's context pack
#[utoipa::path(get, path = "/threads/{id}/context", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ThreadContextQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadContext),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_thread_context() {}

/// Snapshot a thread's context pack
#[utoipa::path(post, path = "/threads/{id}/context/snapshot", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ThreadContextQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Artifact, description = "The frozen context pack as a content-addressed artifact"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn snapshot_thread_context() {}

/// Get a thread's tool-call transcript
#[utoipa::path(get, path = "/threads/{id}/tool-transcript", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ToolTranscriptQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ToolTranscript),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_tool_transcript() {}

/// Transition a thread's state
#[utoipa::path(post, path = "/threads/{id}", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = TransitionThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn transition_thread() {}

/// Assign a thread
#[utoipa::path(put, path = "/threads/{id}/assignee", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AssignThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn assign_thread() {}

/// Unassign a thread
#[utoipa::path(delete, path = "/threads/{id}/assignee", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = UnassignThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn unassign_thread() {}

/// Set a thread's owner
#[utoipa::path(put, path = "/threads/{id}/owner", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadOwner,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_thread_owner() {}

/// Clear a thread's owner
#[utoipa::path(delete, path = "/threads/{id}/owner", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn remove_thread_owner() {}

/// Rename a thread
#[utoipa::path(put, path = "/threads/{id}/title", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = RenameThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread),
        (status = 400, description = "Empty title", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn rename_thread() {}

/// Replace a thread's budget
#[utoipa::path(put, path = "/threads/{id}/budget", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = BudgetPatch,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadBudget),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_thread_budget() {}

/// Update a thread's budget
#[utoipa::path(patch, path = "/threads/{id}/budget", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = BudgetPatch,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadBudget),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn patch_thread_budget() {}

/// Get a thread's budget
#[utoipa::path(get, path = "/threads/{id}/budget", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadBudget),
        (status = 403, response = Forbidden),
        (status = 404, description = "No budget set", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn get_thread_budget() {}

/// Report usage against a thread's budget
#[utoipa::path(post, path = "/threads/{id}/usage", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AccountedUsageRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = UsageLedgerEntry),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn report_thread_usage() {}

/// Claim a thread
#[utoipa::path(post, path = "/threads/{id}/assignee/claim", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = ClaimThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadClaimResult),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn claim_thread() {}

/// Mark a thread unclaimable
#[utoipa::path(put, path = "/threads/{id}/unclaimable", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = MarkUnclaimable,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadUnclaimable, description = "The thread parked from dispatch"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn mark_thread_unclaimable() {}

/// Mark a thread claimable again
#[utoipa::path(delete, path = "/threads/{id}/unclaimable", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Un-parked"),
        (status = 403, response = Forbidden),
        (status = 404, description = "Was not parked", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn mark_thread_claimable() {}

/// Block a thread
#[utoipa::path(put, path = "/threads/{id}/block", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadBlock,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadBlock, description = "The explicit dispatch block"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_thread_block() {}

/// Get a thread's block
#[utoipa::path(get, path = "/threads/{id}/block", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadBlock),
        (status = 403, response = Forbidden),
        (status = 404, description = "Not blocked", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn get_thread_block() {}

/// Clear a thread's block
#[utoipa::path(delete, path = "/threads/{id}/block", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Cleared; BlockedResolved emitted"),
        (status = 403, response = Forbidden),
        (status = 404, description = "Was not blocked", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn clear_thread_block() {}

/// Set a thread's wait
#[utoipa::path(put, path = "/threads/{id}/wait", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadWait,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadWait, description = "The wait timer (set/reset)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_thread_wait() {}

/// Cancel a thread's wait
#[utoipa::path(delete, path = "/threads/{id}/wait", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Cancelled"),
        (status = 403, response = Forbidden),
        (status = 404, description = "No wait was set", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn cancel_thread_wait() {}

/// Get a thread's wait
#[utoipa::path(get, path = "/threads/{id}/wait", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadWait),
        (status = 403, response = Forbidden),
        (status = 404, description = "No wait is set", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn get_thread_wait() {}

/// Set a thread's priority
#[utoipa::path(put, path = "/threads/{id}/priority", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadPriority,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadPriority, description = "The dispatch priority (set/updated)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_thread_priority() {}

/// Get a thread's priority
#[utoipa::path(get, path = "/threads/{id}/priority", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadPriority),
        (status = 403, response = Forbidden),
        (status = 404, description = "No explicit priority (defaults to 0)", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn get_thread_priority() {}

/// List a thread's messages
#[utoipa::path(get, path = "/threads/{tid}/messages", tag = "messages",
    params(
        ("tid" = Uuid, Path, description = "Thread id"),
        ListMessagesQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Message>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_messages() {}

/// Post a message to a thread
#[utoipa::path(post, path = "/threads/{tid}/messages", tag = "messages",
    params(("tid" = Uuid, Path, description = "Thread id")),
    request_body = CreateMessage,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Message),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn post_message() {}

// --- messages ---

/// Get a message
#[utoipa::path(get, path = "/messages/{id}", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Message),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_message() {}

/// List a message's backlinks
#[utoipa::path(get, path = "/messages/{id}/backlinks", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MessageBacklinks, description = "Incoming RelationKind edges plus pins, reactions, and votes"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_message_backlinks() {}

/// Edit a message
#[utoipa::path(patch, path = "/messages/{id}", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = EditMessageRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Message),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn edit_message() {}

/// List a message's edit history
#[utoipa::path(get, path = "/messages/{id}/edits", tag = "messages",
    params(
        ("id" = Uuid, Path, description = "Message id"),
        crate::dto::ListMessageEditsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<MessageEdit>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_message_edits() {}

/// Withdraw a message
#[utoipa::path(delete, path = "/messages/{id}", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Tombstoned"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn tombstone_message() {}

/// Mention a member in a message
#[utoipa::path(post, path = "/messages/{id}/mentions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = CreateMention,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Mention),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn create_mention() {}

/// Seed a thread from a message
#[utoipa::path(post, path = "/messages/{id}/seed", tag = "messages",
    params(("id" = Uuid, Path, description = "Source message id")),
    request_body = SeedFromMessage,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Thread, description = "The seeded child thread, linked to the source by a `seeded_from` reference edge"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn seed_from_message() {}

/// Vote on a message
#[utoipa::path(post, path = "/messages/{id}/votes", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = CreateVote,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Vote),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn cast_vote() {}

/// List a message's votes
#[utoipa::path(get, path = "/messages/{id}/votes", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Vote>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_votes() {}

/// React to a message
#[utoipa::path(post, path = "/messages/{id}/reactions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = CreateReaction,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn add_reaction() {}

/// Remove a reaction
#[utoipa::path(delete, path = "/messages/{id}/reactions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    request_body = RemoveReaction,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn remove_reaction() {}

/// List a message's reactions
#[utoipa::path(get, path = "/messages/{id}/reactions", tag = "messages",
    params(("id" = Uuid, Path, description = "Message id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Reaction>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_reactions() {}

/// Pin a message
#[utoipa::path(post, path = "/threads/{id}/pins", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = PinMessage,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn pin_message() {}

/// Unpin a message
#[utoipa::path(delete, path = "/threads/{id}/pins", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = PinMessage,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn unpin_message() {}

/// List a thread's pins
#[utoipa::path(get, path = "/threads/{id}/pins", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Pin>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_pins() {}

/// Add a thread dependency
#[utoipa::path(post, path = "/threads/{id}/dependencies", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AddThreadDependency,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn add_thread_dependency() {}

/// List a thread's dependencies
#[utoipa::path(get, path = "/threads/{id}/dependencies", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadDependenciesView),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_thread_dependencies() {}

/// Remove a thread dependency
#[utoipa::path(delete, path = "/threads/{id}/dependencies/{dep_id}", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ("dep_id" = Uuid, Path, description = "Dependency thread id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn remove_thread_dependency() {}

/// List a thread's dependents
#[utoipa::path(get, path = "/threads/{id}/dependents", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ThreadDependency>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_thread_dependents() {}

// --- task schedules ---

/// Create a task schedule
#[utoipa::path(post, path = "/workspaces/{wid}/task-schedules", tag = "schedules",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateTaskSchedule,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = TaskSchedule),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn create_task_schedule() {}

/// List a workspace's task schedules
#[utoipa::path(get, path = "/workspaces/{wid}/task-schedules", tag = "schedules",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<TaskSchedule>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_task_schedules() {}

/// Pause or resume a task schedule
#[utoipa::path(put, path = "/task-schedules/{id}", tag = "schedules",
    params(("id" = Uuid, Path, description = "Task schedule id")),
    request_body = SetTaskScheduleActive,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = TaskSchedule),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_task_schedule_active() {}

/// Delete a task schedule
#[utoipa::path(delete, path = "/task-schedules/{id}", tag = "schedules",
    params(("id" = Uuid, Path, description = "Task schedule id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn delete_task_schedule() {}

// --- recipes ---

/// Create a recipe
#[utoipa::path(post, path = "/workspaces/{wid}/recipes", tag = "recipes",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateRecipe,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Recipe),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn create_recipe() {}

/// List a workspace's recipes
#[utoipa::path(get, path = "/workspaces/{wid}/recipes", tag = "recipes",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Recipe>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_recipes() {}

/// Get a recipe
#[utoipa::path(get, path = "/workspaces/{wid}/recipes/{id}", tag = "recipes",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("id" = Uuid, Path, description = "Recipe id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Recipe),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_recipe() {}

/// Delete a recipe
#[utoipa::path(delete, path = "/workspaces/{wid}/recipes/{id}", tag = "recipes",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("id" = Uuid, Path, description = "Recipe id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn delete_recipe() {}

/// Instantiate a recipe
#[utoipa::path(post, path = "/workspaces/{wid}/recipes/{id}/instantiate", tag = "recipes",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("id" = Uuid, Path, description = "Recipe id")),
    request_body = InstantiateRecipe,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = RecipeRun),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn instantiate_recipe() {}

// --- secrets ---

/// Create a secret
#[utoipa::path(post, path = "/workspaces/{wid}/secrets", tag = "secrets",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateSecret,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Secret),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    ))]
pub fn create_secret() {}

/// List a workspace's secrets
#[utoipa::path(get, path = "/workspaces/{wid}/secrets", tag = "secrets",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Secret>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_secrets() {}

/// Resolve a secret's value
#[utoipa::path(post, path = "/workspaces/{wid}/secrets/{name}/resolve", tag = "secrets",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("name" = String, Path, description = "Secret name")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = SecretValue),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn resolve_secret() {}

/// Delete a secret
#[utoipa::path(delete, path = "/workspaces/{wid}/secrets/{name}", tag = "secrets",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("name" = String, Path, description = "Secret name")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn delete_secret() {}

// --- member freeze kill-switch ---

/// Freeze a member
#[utoipa::path(post, path = "/members/{id}/freeze", tag = "freeze",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = FreezeMember,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = FreezeResult),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn freeze_member() {}

/// Unfreeze a member
#[utoipa::path(delete, path = "/members/{id}/freeze", tag = "freeze",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn unfreeze_member() {}

/// Get a member's freeze
#[utoipa::path(get, path = "/members/{id}/freeze", tag = "freeze",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemberFreeze),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_member_freeze() {}

/// List a workspace's frozen members
#[utoipa::path(get, path = "/workspaces/{wid}/frozen-members", tag = "freeze",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<MemberFreeze>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_frozen_members() {}

// --- memory blocks (attachable labeled memory) ---

/// Create a memory block
#[utoipa::path(post, path = "/workspaces/{wid}/memory-blocks", tag = "memory",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateMemoryBlock,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MemoryBlock),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn create_memory_block() {}

/// List a workspace's memory blocks
#[utoipa::path(get, path = "/workspaces/{wid}/memory-blocks", tag = "memory",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<MemoryBlock>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_memory_blocks() {}

/// Get a memory block
#[utoipa::path(get, path = "/workspaces/{wid}/memory-blocks/{id}", tag = "memory",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("id" = Uuid, Path, description = "Memory block id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemoryBlock),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_memory_block() {}

/// Set a memory block's value
#[utoipa::path(put, path = "/workspaces/{wid}/memory-blocks/{id}", tag = "memory",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("id" = Uuid, Path, description = "Memory block id")),
    request_body = SetMemoryBlockValue,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemoryBlock),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_memory_block_value() {}

/// Delete a memory block
#[utoipa::path(delete, path = "/workspaces/{wid}/memory-blocks/{id}", tag = "memory",
    params(("wid" = Uuid, Path, description = "Workspace id"),
        ("id" = Uuid, Path, description = "Memory block id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn delete_memory_block() {}

/// List a thread's memory blocks
#[utoipa::path(get, path = "/threads/{id}/memory-blocks", tag = "memory",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<MemoryBlock>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_thread_memory_blocks() {}

/// Attach a memory block to a thread
#[utoipa::path(post, path = "/threads/{id}/memory-blocks/{block_id}", tag = "memory",
    params(("id" = Uuid, Path, description = "Thread id"),
        ("block_id" = Uuid, Path, description = "Memory block id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn attach_memory_block() {}

/// Detach a memory block from a thread
#[utoipa::path(delete, path = "/threads/{id}/memory-blocks/{block_id}", tag = "memory",
    params(("id" = Uuid, Path, description = "Thread id"),
        ("block_id" = Uuid, Path, description = "Memory block id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn detach_memory_block() {}

// --- required reviewers ---

/// Set a thread's review requirement
#[utoipa::path(put, path = "/threads/{id}/review-requirement", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetReviewRequirement,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadReviewRequirement),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn set_review_requirement() {}

/// Get a thread's review requirement
#[utoipa::path(get, path = "/threads/{id}/review-requirement", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadReviewRequirement),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_review_requirement() {}

/// Remove a thread's review requirement
#[utoipa::path(delete, path = "/threads/{id}/review-requirement", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn clear_review_requirement() {}

/// Add a reviewer to a thread
#[utoipa::path(post, path = "/threads/{id}/reviewers", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AddReviewer,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn add_reviewer() {}

/// List a thread's reviewers
#[utoipa::path(get, path = "/threads/{id}/reviewers", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<MemberId>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_reviewers() {}

/// Remove a reviewer from a thread
#[utoipa::path(delete, path = "/threads/{id}/reviewers/{member_id}", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id"),
        ("member_id" = Uuid, Path, description = "Reviewer member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn remove_reviewer() {}

/// Submit a review
#[utoipa::path(post, path = "/threads/{id}/reviews", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SubmitReview,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadReview),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn submit_review() {}

/// List a thread's reviews
#[utoipa::path(get, path = "/threads/{id}/reviews", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ThreadReview>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_reviews() {}

/// List every verdict submitted on a thread
///
/// Oldest first. Each submission is kept: a re-submission replaces the
/// reviewer's current review (`GET /threads/{id}/reviews`) but not its
/// earlier verdicts.
#[utoipa::path(get, path = "/threads/{id}/reviews/history", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ReviewVerdict>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_review_history() {}

/// Get a thread's review status
#[utoipa::path(get, path = "/threads/{id}/review-status", tag = "review",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ReviewStatus),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_review_status() {}

// --- land_gate gate pointer ---

/// Set a thread's land gate
#[utoipa::path(put, path = "/threads/{id}/land-gate", tag = "land_gate",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetLandGate,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = LandGateStanding),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_land_gate() {}

/// Get a thread's land gate
#[utoipa::path(get, path = "/threads/{id}/land-gate", tag = "land_gate",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = LandGateStanding),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_land_gate() {}

/// List every land-gate verdict recorded on a thread
///
/// Oldest first. Each recorded pointer is kept; a later pointer or clearing
/// the gate leaves it in place.
#[utoipa::path(get, path = "/threads/{id}/land-gate/history", tag = "land_gate",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<LandGateVerdict>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_land_gate_history() {}

/// Remove a thread's land gate
#[utoipa::path(delete, path = "/threads/{id}/land-gate", tag = "land_gate",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn clear_land_gate() {}

/// Require a land gate on a thread
#[utoipa::path(put, path = "/threads/{id}/land-gate/requirement", tag = "land_gate",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = LandGateStanding),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn require_land_gate() {}

/// Ask for land-gate advice
#[utoipa::path(post, path = "/threads/{id}/land-gate/advice", tag = "land_gate",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = LandGateAdviceRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = LandGateAdvice, description = "Advisory result; never writes the gate pointer"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, description = "Experimental advisor disabled", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 502, description = "Decision provider unavailable", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn advise_land_gate() {}

// --- spawn budget ---

/// Set a workspace's spawn budget
#[utoipa::path(put, path = "/workspaces/{id}/spawn-budget", tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = SetSpawnBudget,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = SpawnBudgetView, description = "The spawn budget (set or cleared)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    ))]
pub fn set_spawn_budget() {}

/// Get a workspace's spawn budget
#[utoipa::path(get, path = "/workspaces/{id}/spawn-budget", tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = SpawnBudgetView, description = "The spawn budget (a null axis is unlimited)"),
        (status = 403, response = Forbidden),
    ))]
pub fn get_spawn_budget() {}

// --- skills (capability registry) ---

/// Add a skill to a member
#[utoipa::path(post, path = "/members/{id}/skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = AddSkill,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn add_member_skill() {}

/// List a member's skills
#[utoipa::path(get, path = "/members/{id}/skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<MemberSkill>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_member_skills() {}

/// Remove a skill from a member
#[utoipa::path(delete, path = "/members/{id}/skills/{skill}", tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("skill" = String, Path, description = "Skill tag"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn remove_member_skill() {}

/// Require a skill on a thread
#[utoipa::path(post, path = "/threads/{id}/required-skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AddSkill,
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn add_thread_required_skill() {}

/// List a thread's required skills
#[utoipa::path(get, path = "/threads/{id}/required-skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ThreadRequiredSkill>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_thread_required_skills() {}

/// Remove a thread's required skill
#[utoipa::path(delete, path = "/threads/{id}/required-skills/{skill}", tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ("skill" = String, Path, description = "Skill tag"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn remove_thread_required_skill() {}

// --- task results ---

/// Set a thread's result
#[utoipa::path(put, path = "/threads/{id}/result", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadResult,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadResult),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn set_thread_result() {}

/// Get a thread's result
#[utoipa::path(get, path = "/threads/{id}/result", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadResult),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_thread_result() {}

/// List a thread's result deliveries
#[utoipa::path(get, path = "/threads/{id}/deliveries", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [ResultDelivery]),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_thread_deliveries() {}

/// Replay a thread's result delivery
#[utoipa::path(post, path = "/threads/{id}/deliveries/{did}/replay", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ("did" = Uuid, Path, description = "Result-delivery id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ResultDelivery),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn replay_thread_delivery() {}

/// Set a thread's run lineage
#[utoipa::path(put, path = "/threads/{id}/lineage", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadLineage,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadLineage),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_thread_lineage() {}

/// Get a thread's run lineage
#[utoipa::path(get, path = "/threads/{id}/lineage", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadLineage),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_thread_lineage() {}

/// Clear a thread's run lineage
#[utoipa::path(delete, path = "/threads/{id}/lineage", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn clear_thread_lineage() {}

/// Set a thread's steer
#[utoipa::path(put, path = "/threads/{id}/steer", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadSteer,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadSteer),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn set_thread_steer() {}

/// Get a thread's steer
#[utoipa::path(get, path = "/threads/{id}/steer", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ThreadSteer),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_thread_steer() {}

/// List a thread's child threads
#[utoipa::path(get, path = "/threads/{id}/children", tag = "threads",
    params(("id" = Uuid, Path, description = "Parent thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [ChildThreadSummary]),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_child_threads() {}

/// Mute a thread for the caller
#[utoipa::path(post, path = "/threads/{id}/mute", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Thread muted for the caller"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn mute_thread() {}

/// Unmute a thread for the caller
#[utoipa::path(delete, path = "/threads/{id}/mute", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Thread unmuted"),
        (status = 403, response = Forbidden),
        (status = 404, description = "Was not muted", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn unmute_thread() {}

// --- approval gates (the held gate) ---

/// List a workspace's pending approval gates
#[utoipa::path(get, path = "/workspaces/{wid}/approval-gates", tag = "approval-gates",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<ApprovalGateView>),
        (status = 403, response = Forbidden),
    ))]
pub fn list_approval_gates() {}

/// Answer a pending approval gate
#[utoipa::path(post, path = "/approval-gates/{id}/answer", tag = "approval-gates",
    params(("id" = Uuid, Path, description = "Approval gate id")),
    request_body = AnswerApprovalGate,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ApprovalGate),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn answer_approval_gate() {}

// --- artifacts ---

/// Upload an artifact
#[utoipa::path(post, path = "/artifacts", tag = "artifacts",
    params(UploadArtifactQuery),
    request_body(content = String, description = "Raw bytes", content_type = "application/octet-stream"),
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Artifact),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    ))]
pub fn upload_artifact() {}

/// Download an artifact
///
/// `Content-Type` is the type this workspace stored at upload
/// (`application/octet-stream` when none, or not a media type), never
/// sniffed. PNG, JPEG, GIF and WebP are `inline`; every other type, SVG
/// included, is an `attachment`. Always `nosniff` and a sandboxing CSP.
#[utoipa::path(get, path = "/artifacts/{sha}", tag = "artifacts",
    params(("sha" = String, Path, description = "SHA-256 hex")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Raw bytes", content_type = "application/octet-stream",
            headers(
                ("Content-Disposition" = String, description = "`inline` for png/jpeg/gif/webp, else `attachment`; with `filename` and `filename*` when the upload named it"),
                ("X-Content-Type-Options" = String, description = "Always `nosniff`"),
                ("Content-Security-Policy" = String, description = "`default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; sandbox`"),
                ("X-Artifact-Kind" = String, description = "The artifact's kind"),
            )),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_artifact() {}

/// Erase this workspace's copy of an artifact
///
/// `token:admin`; audited as `artifact.erase`. The bytes are deleted only
/// with the last workspace reference (`last_reference`).
#[utoipa::path(delete, path = "/artifacts/{sha}", tag = "artifacts",
    params(("sha" = String, Path, description = "SHA-256 hex")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ArtifactErasure),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn erase_artifact() {}

/// Get an artifact's metadata
#[utoipa::path(get, path = "/artifacts/{sha}/meta", tag = "artifacts",
    params(("sha" = String, Path, description = "SHA-256 hex")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Artifact),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn get_artifact_metadata() {}

/// Begin a multipart artifact upload
#[utoipa::path(
    post,
    path = "/artifacts/multipart",
    tag = "artifacts",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "Multipart upload started"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn begin_multipart_artifact_doc() {}

/// Abort a multipart artifact upload
#[utoipa::path(
    delete,
    path = "/artifacts/multipart",
    tag = "artifacts",
    params(AbortMultipartQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Aborted"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn abort_multipart_artifact_doc() {}

/// Complete a multipart artifact upload
#[utoipa::path(
    post,
    path = "/artifacts/multipart/{upload_id}/complete",
    params(("upload_id" = String, Path, description = "Multipart upload id")),
    request_body = CompleteMultipartArtifact,
    tag = "artifacts",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Artifact),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn complete_multipart_artifact_doc() {}

/// Upload one part of a multipart artifact
#[utoipa::path(
    put,
    path = "/artifacts/multipart/{upload_id}/parts/{part_number}",
    params(("upload_id" = String, Path, description = "Multipart upload id"), ("part_number" = i32, Path, description = "Part number, from 1"), MultipartUploadQuery),
    request_body(content = String, description = "The part's raw bytes", content_type = "application/octet-stream"),
    tag = "artifacts",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Part stored"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn upload_multipart_artifact_part_doc() {}

// --- references ---

/// Create a reference
#[utoipa::path(post, path = "/references", tag = "references",
    request_body = CreateReference,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = Reference),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn create_reference() {}

/// List references
#[utoipa::path(get, path = "/references", tag = "references",
    params(ListReferencesQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Reference>),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn list_references() {}

// --- tokens ---

/// Revoke an API token
#[utoipa::path(delete, path = "/tokens/{id}", tag = "tokens",
    params(("id" = Uuid, Path, description = "API token id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn revoke_api_token() {}

/// Rotate an API token: a new secret for the same authority; the old one stops working
#[utoipa::path(post, path = "/tokens/{id}/rotate", tag = "tokens",
    params(("id" = Uuid, Path, description = "API token id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MintApiTokenResponse, description = "The successor, with its secret shown once"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    ))]
pub fn rotate_api_token() {}

// --- federation ---

/// Discover this server for federation
#[utoipa::path(get, path = "/.well-known/maidan.json", tag = "federation",
    security(()),
    responses(
        (status = 200, body = WellKnownMaidan),
    ))]
pub fn well_known() {}

/// Ingest events pushed by a federation peer
#[utoipa::path(
    post,
    path = "/a2a/v1/events",
    tag = "federation",
    security(("bearerAuth" = [])),
    request_body(content = String, description = "FederatedEventBatch JSON", content_type = "application/json"),
    responses(
        (status = 200, body = IngestSummary),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 409, response = Conflict),
    )
)]
pub fn ingest_events() {}
