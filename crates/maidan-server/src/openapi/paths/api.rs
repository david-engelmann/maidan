//! REST API path docs (mirrors `app.rs` routes; WS/MCP excluded).

use uuid::Uuid;

use crate::dto::*;
use crate::error::ProblemDetails;
use crate::federation::{IngestSummary, WellKnownMaidan};
use crate::openapi::schemas::SearchHit;
use maidan_types::*;

// --- bootstrap (no bearer) ---

#[cfg(feature = "bootstrap")]
#[utoipa::path(post, path = "/workspaces", tag = "bootstrap",
    request_body = CreateWorkspace,
    responses((status = 201, description = "Created", body = Workspace)))]
pub fn create_workspace() {}

#[cfg(feature = "bootstrap")]
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

#[utoipa::path(
    delete,
    path = "/workspaces/{id}",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = EraseWorkspace,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WorkspaceEraseResult),
        (status = 400, body = ProblemDetails),
        (status = 401, body = ProblemDetails),
        (status = 403, body = ProblemDetails),
        (status = 404, body = ProblemDetails),
    )
)]
pub fn erase_workspace() {}

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

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/members/{mid}/tokens",
    tag = "tokens",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("mid" = Uuid, Path, description = "Member id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<ApiTokenSummary>))
)]
pub fn list_api_tokens() {}

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

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/webhooks",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<WebhookResponse>))
)]
pub fn list_webhooks() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/webhooks",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateWebhook,
    security(("bearerAuth" = [])),
    responses((status = 201, body = MintWebhookResponse))
)]
pub fn create_webhook() {}

#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/webhooks/{whid}",
    tag = "webhooks",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("whid" = Uuid, Path, description = "Webhook subscription id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Revoked"))
)]
pub fn revoke_webhook() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/mention-webhook",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = MentionWebhookConfig))
)]
pub fn get_mention_webhook() {}

#[utoipa::path(
    put,
    path = "/workspaces/{wid}/mention-webhook",
    tag = "webhooks",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = SetMentionWebhook,
    security(("bearerAuth" = [])),
    responses((status = 200, body = MentionWebhookConfig))
)]
pub fn set_mention_webhook() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/slash-commands",
    tag = "slash",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<SlashCommandResponse>))
)]
pub fn list_slash_commands() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/slash-commands",
    tag = "slash",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateSlashCommand,
    security(("bearerAuth" = [])),
    responses((status = 201, body = MintSlashCommandResponse))
)]
pub fn create_slash_command() {}

#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/slash-commands/{cid}",
    tag = "slash",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("cid" = Uuid, Path, description = "Slash command id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Revoked"))
)]
pub fn revoke_slash_command() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/fsm-hooks",
    tag = "fsm",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<FsmHookResponse>))
)]
pub fn list_fsm_hooks() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/fsm-hooks",
    tag = "fsm",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateFsmHook,
    security(("bearerAuth" = [])),
    responses((status = 201, body = MintFsmHookResponse))
)]
pub fn create_fsm_hook() {}

#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/fsm-hooks/{hid}",
    tag = "fsm",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("hid" = Uuid, Path, description = "FSM hook id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Revoked"))
)]
pub fn revoke_fsm_hook() {}

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

#[utoipa::path(get, path = "/members/{id}/notifications", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ListNotificationsQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Notification>)))]
pub fn list_member_notifications() {}

#[utoipa::path(get, path = "/members/{id}/notifications/unread-count", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = UnreadCount)))]
pub fn member_unread_notification_count() {}

#[utoipa::path(post, path = "/members/{id}/notifications/read-all", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = MarkAllRead)))]
pub fn mark_all_member_notifications_read() {}

#[utoipa::path(post, path = "/members/{id}/notifications/{nid}/read", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("nid" = Uuid, Path, description = "Notification id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = UnreadCount)))]
pub fn mark_member_notification_read() {}

#[utoipa::path(put, path = "/members/{id}/notification-prefs", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = SetNotificationPref,
    security(("bearerAuth" = [])),
    responses((status = 200, body = NotificationPref)))]
pub fn set_member_notification_pref() {}

#[utoipa::path(get, path = "/members/{id}/notification-prefs", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<NotificationPref>)))]
pub fn list_member_notification_prefs() {}

#[utoipa::path(post, path = "/members/{id}/channel-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = FollowChannel,
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Following")))]
pub fn follow_member_channel() {}

#[utoipa::path(delete, path = "/members/{id}/channel-follows/{cid}", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("cid" = Uuid, Path, description = "Channel id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Unfollowed")))]
pub fn unfollow_member_channel() {}

#[utoipa::path(get, path = "/members/{id}/channel-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<ChannelFollow>)))]
pub fn list_member_channel_follows() {}

#[utoipa::path(post, path = "/members/{id}/thread-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = FollowThread,
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Following")))]
pub fn follow_member_thread() {}

#[utoipa::path(delete, path = "/members/{id}/thread-follows/{tid}", tag = "members",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("tid" = Uuid, Path, description = "Thread id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Unfollowed")))]
pub fn unfollow_member_thread() {}

#[utoipa::path(get, path = "/members/{id}/thread-follows", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<ThreadFollow>)))]
pub fn list_member_thread_follows() {}

#[utoipa::path(put, path = "/members/{id}/email", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = SetEmail,
    security(("bearerAuth" = [])),
    responses((status = 200, body = MemberEmail)))]
pub fn set_member_email() {}

#[utoipa::path(get, path = "/members/{id}/email", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = MemberEmail)))]
pub fn get_member_email() {}

#[utoipa::path(delete, path = "/members/{id}/email", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Cleared")))]
pub fn delete_member_email() {}

#[utoipa::path(put, path = "/members/{id}/delivery-mode", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = SetDeliveryMode,
    security(("bearerAuth" = [])),
    responses((status = 200, body = DeliveryModeView)))]
pub fn set_member_delivery_mode() {}

#[utoipa::path(get, path = "/members/{id}/delivery-mode", tag = "members",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = DeliveryModeView)))]
pub fn get_member_delivery_mode() {}

// --- channels ---

#[utoipa::path(get, path = "/channels/{id}", tag = "channels",
    params(("id" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Channel)))]
pub fn get_channel() {}

#[utoipa::path(get, path = "/channels/{cid}/queue-depth", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = QueueDepth)))]
pub fn get_channel_queue_depth() {}

#[utoipa::path(get, path = "/channels/{cid}/occupancy", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = ChannelOccupancy)))]
pub fn get_channel_occupancy() {}

#[utoipa::path(post, path = "/channels/{cid}/members", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    request_body = AddChannelMember,
    security(("bearerAuth" = [])),
    responses((status = 201, body = ChannelMember)))]
pub fn add_channel_member() {}

#[utoipa::path(get, path = "/channels/{cid}/members", tag = "channels",
    params(("cid" = Uuid, Path, description = "Channel id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<ChannelMember>)))]
pub fn list_channel_members() {}

#[utoipa::path(delete, path = "/channels/{cid}/members/{mid}", tag = "channels",
    params(
        ("cid" = Uuid, Path, description = "Channel id"),
        ("mid" = Uuid, Path, description = "Member id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204, description = "removed")))]
pub fn remove_channel_member() {}

#[utoipa::path(get, path = "/channels/{cid}/threads", tag = "threads",
    params(("cid" = Uuid, Path, description = "Channel id"), ListThreadsQuery),
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

#[utoipa::path(get, path = "/threads/{id}/context", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ThreadContextQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = ThreadContext)))]
pub fn get_thread_context() {}

#[utoipa::path(post, path = "/threads/{id}/context/snapshot", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ThreadContextQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 201, body = Artifact, description = "The frozen context pack as a content-addressed artifact")))]
pub fn snapshot_thread_context() {}

#[utoipa::path(get, path = "/threads/{id}/tool-transcript", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ToolTranscriptQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = ToolTranscript)))]
pub fn get_tool_transcript() {}

#[utoipa::path(post, path = "/threads/{id}", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = TransitionThread,
    security(("bearerAuth" = [])),
    responses((status = 200, body = Thread)))]
pub fn transition_thread() {}

#[utoipa::path(put, path = "/threads/{id}/assignee", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AssignThread,
    security(("bearerAuth" = [])),
    responses((status = 200, body = Thread)))]
pub fn assign_thread() {}

#[utoipa::path(delete, path = "/threads/{id}/assignee", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = UnassignThread,
    security(("bearerAuth" = [])),
    responses((status = 200, body = Thread)))]
pub fn unassign_thread() {}

#[utoipa::path(post, path = "/threads/{id}/assignee/claim", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = ClaimThread,
    security(("bearerAuth" = [])),
    responses((status = 200, body = ThreadClaimResult)))]
pub fn claim_thread() {}

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

#[utoipa::path(post, path = "/messages/{id}/seed", tag = "messages",
    params(("id" = Uuid, Path, description = "Source message id")),
    request_body = SeedFromMessage,
    security(("bearerAuth" = [])),
    responses((status = 201, body = Thread, description = "The seeded child thread, linked to the source by a `seeded_from` reference edge")))]
pub fn seed_from_message() {}

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

#[utoipa::path(post, path = "/threads/{id}/dependencies", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AddThreadDependency,
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn add_thread_dependency() {}

#[utoipa::path(get, path = "/threads/{id}/dependencies", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = ThreadDependenciesView)))]
pub fn list_thread_dependencies() {}

#[utoipa::path(delete, path = "/threads/{id}/dependencies/{dep_id}", tag = "threads",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ("dep_id" = Uuid, Path, description = "Dependency thread id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn remove_thread_dependency() {}

#[utoipa::path(get, path = "/threads/{id}/dependents", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<ThreadDependency>)))]
pub fn list_thread_dependents() {}

// --- task schedules ---

#[utoipa::path(post, path = "/workspaces/{wid}/task-schedules", tag = "schedules",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateTaskSchedule,
    security(("bearerAuth" = [])),
    responses((status = 201, body = TaskSchedule)))]
pub fn create_task_schedule() {}

#[utoipa::path(get, path = "/workspaces/{wid}/task-schedules", tag = "schedules",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<TaskSchedule>)))]
pub fn list_task_schedules() {}

#[utoipa::path(put, path = "/task-schedules/{id}", tag = "schedules",
    params(("id" = Uuid, Path, description = "Task schedule id")),
    request_body = SetTaskScheduleActive,
    security(("bearerAuth" = [])),
    responses((status = 200, body = TaskSchedule)))]
pub fn set_task_schedule_active() {}

#[utoipa::path(delete, path = "/task-schedules/{id}", tag = "schedules",
    params(("id" = Uuid, Path, description = "Task schedule id")),
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn delete_task_schedule() {}

// --- skills (capability registry) ---

#[utoipa::path(post, path = "/members/{id}/skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = AddSkill,
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn add_member_skill() {}

#[utoipa::path(get, path = "/members/{id}/skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<MemberSkill>)))]
pub fn list_member_skills() {}

#[utoipa::path(delete, path = "/members/{id}/skills/{skill}", tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Member id"),
        ("skill" = String, Path, description = "Skill tag"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn remove_member_skill() {}

#[utoipa::path(post, path = "/threads/{id}/required-skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AddSkill,
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn add_thread_required_skill() {}

#[utoipa::path(get, path = "/threads/{id}/required-skills", tag = "skills",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<ThreadRequiredSkill>)))]
pub fn list_thread_required_skills() {}

#[utoipa::path(delete, path = "/threads/{id}/required-skills/{skill}", tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Thread id"),
        ("skill" = String, Path, description = "Skill tag"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 204)))]
pub fn remove_thread_required_skill() {}

// --- task results ---

#[utoipa::path(put, path = "/threads/{id}/result", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = SetThreadResult,
    security(("bearerAuth" = [])),
    responses((status = 200, body = ThreadResult)))]
pub fn set_thread_result() {}

#[utoipa::path(get, path = "/threads/{id}/result", tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = ThreadResult)))]
pub fn get_thread_result() {}

// --- approval gates (the held gate, Cluster 350) ---

#[utoipa::path(get, path = "/workspaces/{wid}/approval-gates", tag = "approval-gates",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<ApprovalGateView>)))]
pub fn list_approval_gates() {}

#[utoipa::path(post, path = "/approval-gates/{id}/answer", tag = "approval-gates",
    params(("id" = Uuid, Path, description = "Approval gate id")),
    request_body = AnswerApprovalGate,
    security(("bearerAuth" = [])),
    responses((status = 200, body = ApprovalGate)))]
pub fn answer_approval_gate() {}

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

#[utoipa::path(
    post,
    path = "/artifacts/multipart",
    tag = "artifacts",
    security(("bearerAuth" = [])),
    responses((status = 201, description = "Multipart upload started"))
)]
pub fn begin_multipart_artifact_doc() {}

#[utoipa::path(
    delete,
    path = "/artifacts/multipart",
    tag = "artifacts",
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Aborted"))
)]
pub fn abort_multipart_artifact_doc() {}

#[utoipa::path(
    post,
    path = "/artifacts/multipart/{upload_id}/complete",
    tag = "artifacts",
    security(("bearerAuth" = [])),
    responses((status = 200, body = Artifact))
)]
pub fn complete_multipart_artifact_doc() {}

#[utoipa::path(
    put,
    path = "/artifacts/multipart/{upload_id}/parts/{part_number}",
    tag = "artifacts",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Part stored"))
)]
pub fn upload_multipart_artifact_part_doc() {}

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
