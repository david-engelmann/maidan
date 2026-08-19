//! Request DTOs. Response shapes use the `maidan_types` models directly
//! since they already derive `Serialize`. The DTOs here mirror the
//! domain `New<X>` structs but exist so the HTTP boundary can evolve
//! independently of the storage layer (e.g., omitting `workspace_id` in
//! a nested route in favor of the path parameter).

use chrono::{DateTime, Utc};
use maidan_types::{
    ApiTokenId, AppId, AppInstallationId, ArtifactKind, ChannelId, ContentBlock, EventKind,
    MemberId, MemberKind, RefSide, ThreadId, WebhookSubscriptionId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    pub name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EraseWorkspace {
    pub confirm_workspace_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMember {
    pub handle: String,
    pub display_name: Option<String>,
    pub kind: MemberKind,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateChannel {
    pub name: String,
    pub topic: Option<String>,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddChannelMember {
    pub member_id: uuid::Uuid,
    /// `member` (default) or `admin`.
    pub role: Option<maidan_types::ChannelMemberRole>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateThread {
    pub title: Option<String>,
    pub parent_thread_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TransitionThread {
    pub actor_id: uuid::Uuid,
    pub action: String,
}

/// Assign / hand off a thread to a member (Cluster 171). `actor_id` records who
/// performed the assignment (carried on the emitted event).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignThread {
    pub actor_id: uuid::Uuid,
    pub assignee_id: uuid::Uuid,
    /// Optional handoff note for the assignee (Cluster 195).
    #[serde(default)]
    pub note: Option<String>,
}

/// Atomically claim an unassigned thread for a member (Cluster 171). The
/// claimer is both the actor and the assignee.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClaimThread {
    pub member_id: uuid::Uuid,
}

/// Claim the next unassigned/expired thread in a channel (Cluster 190/192).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClaimNextThread {
    pub member_id: uuid::Uuid,
    /// Optional lease deadline in seconds; the claim is reclaimable after it
    /// lapses (Cluster 192). Omit for a durable claim.
    #[serde(default)]
    pub lease_secs: Option<i64>,
}

/// Extend a claimed thread's lease, for the current assignee (Cluster 192).
#[derive(Debug, Deserialize, ToSchema)]
pub struct RenewClaim {
    pub member_id: uuid::Uuid,
    pub lease_secs: i64,
}

/// Add a task-dependency edge — the thread in the path depends on
/// `depends_on_thread_id` (Cluster 219).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddThreadDependency {
    pub depends_on_thread_id: uuid::Uuid,
}

/// Create a task schedule (Cluster 228). When due, the sweeper creates a thread
/// titled `title` in `channel_id`. `interval_secs` omitted (or null) = one-shot;
/// a positive value = recurring. `first_run_at` omitted = fire on the next tick
/// (defaults to now).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTaskSchedule {
    pub channel_id: uuid::Uuid,
    pub title: String,
    #[serde(default)]
    pub interval_secs: Option<i64>,
    #[serde(default)]
    pub first_run_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Pause (`false`) or resume (`true`) a schedule (Cluster 228).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetTaskScheduleActive {
    pub active: bool,
}

/// Add a skill — to a member (`declares`) or a thread (`requires`) — Cluster 232.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddSkill {
    pub skill: String,
}

/// Set a task's structured result (Cluster 235). `result` is arbitrary JSON.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadResult {
    #[schema(value_type = Object)]
    pub result: serde_json::Value,
}

/// A task's dependency edges plus whether it is ready to run (Cluster 219).
#[derive(Debug, Serialize, ToSchema)]
pub struct ThreadDependenciesView {
    pub dependencies: Vec<maidan_types::ThreadDependency>,
    /// True when every dependency is terminal (closed/archived) — the task is
    /// ready to claim.
    pub ready: bool,
}

/// Clear a thread's assignee (Cluster 171). `actor_id` records who unassigned it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UnassignThread {
    pub actor_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMessage {
    pub author_id: uuid::Uuid,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Typed structured content (Cluster 173). When present and `body` is empty,
    /// the server derives `body` from these blocks for search/back-compat.
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EditMessageRequest {
    pub editor_id: uuid::Uuid,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Replacement structured content (Cluster 173). Omitted → keep existing.
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMention {
    pub member_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateVote {
    pub member_id: uuid::Uuid,
    pub kind: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateReaction {
    pub member_id: uuid::Uuid,
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RemoveReaction {
    pub member_id: uuid::Uuid,
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PinMessage {
    pub message_id: uuid::Uuid,
    pub member_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateReference {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ThreadContextQuery {
    #[serde(default = "default_limit")]
    pub message_limit: i64,
    #[serde(default = "default_transition_limit")]
    pub transition_limit: i64,
    pub message_cursor: Option<uuid::Uuid>,
    /// Include full `body_before`/`body_after` on each edit (heavy). Default
    /// `false` returns edit metadata only — the largest token lever on a pack.
    #[serde(default)]
    pub include_edits: bool,
}

/// Query for `GET /threads/:id/tool-transcript` (Cluster 197).
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ToolTranscriptQuery {
    /// Max messages to scan (default 200, clamped 1..=500).
    pub limit: Option<i64>,
}

fn default_transition_limit() -> i64 {
    50
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct WorkspaceContextQuery {
    #[serde(default = "default_workspace_thread_limit")]
    pub thread_limit: i64,
    #[serde(default)]
    pub message_limit: i64,
    #[serde(default)]
    pub transition_limit: i64,
    pub thread_cursor: Option<uuid::Uuid>,
    /// Include full edit bodies on every nested thread pack (heavy). Default
    /// `false` returns edit metadata only.
    #[serde(default)]
    pub include_edits: bool,
}

fn default_workspace_thread_limit() -> i64 {
    10
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListMessagesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListMessageEditsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListReferencesQuery {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListEventsQuery {
    #[serde(default)]
    pub after_id: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListAuditQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Lexical,
    /// Embed `q` with the configured provider and rank by cosine similarity.
    Semantic,
    /// Run lexical + semantic and fuse their normalized `[0,1]` scores
    /// (`hybrid_weight` controls the semantic share).
    Hybrid,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default = "default_search_limit")]
    pub limit: i64,
    /// Restrict hits to messages by this member.
    pub author: Option<uuid::Uuid>,
    /// Restrict hits to messages in threads under this channel.
    pub channel: Option<uuid::Uuid>,
    /// Restrict hits to messages whose author has this kind (`human` / `agent`).
    pub kind: Option<MemberKind>,
    /// Semantic only: query this model's embedding table (default: active provider).
    pub embedding_model: Option<String>,
    /// Hybrid only: semantic weight in `[0,1]` (default `0.5`). `combined =
    /// w*semantic + (1-w)*lexical` over the normalized scores.
    pub hybrid_weight: Option<f64>,
    /// Drop the full message `body` from each hit, returning only the bounded
    /// `snippet` (semantic hits get a truncated body prefix as their snippet).
    /// Default `false` keeps today's full-body response.
    #[serde(default)]
    pub snippet_only: bool,
}

fn default_search_limit() -> i64 {
    25
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListMentionsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListInboxQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListNotificationsQuery {
    /// When true, only unread notifications are returned.
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

/// The unread-notification badge count for a member (Cluster 239).
#[derive(Debug, Serialize, ToSchema)]
pub struct UnreadCount {
    pub count: i64,
}

/// Result of marking all of a member's notifications read (Cluster 239).
#[derive(Debug, Serialize, ToSchema)]
pub struct MarkAllRead {
    pub cleared: i64,
}

/// Set a member's mute preference for one event kind (Cluster 242).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetNotificationPref {
    pub kind: EventKind,
    pub muted: bool,
}

/// Follow a channel (Cluster 245).
#[derive(Debug, Deserialize, ToSchema)]
pub struct FollowChannel {
    pub channel_id: ChannelId,
}

/// Follow a thread (Cluster 245).
#[derive(Debug, Deserialize, ToSchema)]
pub struct FollowThread {
    pub thread_id: ThreadId,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct UploadArtifactQuery {
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
    pub uploaded_by: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MultipartUploadResponse {
    pub upload_id: String,
    pub object_key: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct MultipartUploadQuery {
    pub object_key: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct AbortMultipartQuery {
    pub upload_id: String,
    pub object_key: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MultipartPartResponse {
    pub part_number: i32,
    pub etag: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MultipartPartInput {
    pub part_number: i32,
    pub etag: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CompleteMultipartArtifact {
    pub object_key: String,
    pub parts: Vec<MultipartPartInput>,
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
    pub uploaded_by: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintWebhookResponse {
    pub webhook: WebhookResponse,
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub handler_kind: String,
    pub handler_target: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SlashCommandResponse {
    pub id: maidan_types::SlashCommandId,
    pub workspace_id: maidan_types::WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub handler_kind: maidan_types::SlashHandlerKind,
    pub handler_target: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<maidan_types::SlashCommand> for SlashCommandResponse {
    fn from(c: maidan_types::SlashCommand) -> Self {
        Self {
            id: c.id,
            workspace_id: c.workspace_id,
            name: c.name,
            description: c.description,
            handler_kind: c.handler_kind,
            handler_target: c.handler_target,
            enabled: c.enabled,
            created_at: c.created_at,
            revoked_at: c.revoked_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintSlashCommandResponse {
    pub command: SlashCommandResponse,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFsmHook {
    pub label: Option<String>,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub handler_kind: String,
    pub handler_target: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FsmHookResponse {
    pub id: maidan_types::FsmHookId,
    pub workspace_id: maidan_types::WorkspaceId,
    pub label: Option<String>,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub handler_kind: maidan_types::SlashHandlerKind,
    pub handler_target: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<maidan_types::FsmHook> for FsmHookResponse {
    fn from(h: maidan_types::FsmHook) -> Self {
        Self {
            id: h.id,
            workspace_id: h.workspace_id,
            label: h.label,
            from_state: h.from_state.map(|s| s.as_str().to_string()),
            to_state: h.to_state.map(|s| s.as_str().to_string()),
            handler_kind: h.handler_kind,
            handler_target: h.handler_target,
            enabled: h.enabled,
            created_at: h.created_at,
            revoked_at: h.revoked_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintFsmHookResponse {
    pub hook: FsmHookResponse,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWebhook {
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookResponse {
    pub id: maidan_types::WebhookSubscriptionId,
    pub workspace_id: maidan_types::WorkspaceId,
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<maidan_types::WebhookSubscription> for WebhookResponse {
    fn from(w: maidan_types::WebhookSubscription) -> Self {
        Self {
            id: w.id,
            workspace_id: w.workspace_id,
            url: w.url,
            label: w.label,
            event_kinds: w.event_kinds,
            enabled: w.enabled,
            created_at: w.created_at,
            revoked_at: w.revoked_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterApp {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AppResponse {
    pub id: AppId,
    pub workspace_id: WorkspaceId,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
}

impl From<maidan_types::App> for AppResponse {
    fn from(a: maidan_types::App) -> Self {
        Self {
            id: a.id,
            workspace_id: a.workspace_id,
            slug: a.slug,
            name: a.name,
            description: a.description,
            created_by: a.created_by,
            created_at: a.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InstallApp {
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AppInstallationResponse {
    pub id: AppInstallationId,
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub bot_member_id: MemberId,
    pub granted_capabilities: Vec<String>,
    pub installed_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<maidan_types::AppInstallation> for AppInstallationResponse {
    fn from(i: maidan_types::AppInstallation) -> Self {
        Self {
            id: i.id,
            app_id: i.app_id,
            workspace_id: i.workspace_id,
            bot_member_id: i.bot_member_id,
            granted_capabilities: i.granted_capabilities,
            installed_at: i.installed_at,
            revoked_at: i.revoked_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MintAppToken {
    pub label: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub quotas: Vec<maidan_types::TokenQuota>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintAppTokenResponse {
    pub id: ApiTokenId,
    pub secret: String,
    pub workspace_id: WorkspaceId,
    pub app_installation_id: AppInstallationId,
    pub bot_member_id: MemberId,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub quotas: Vec<maidan_types::TokenQuota>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MintApiToken {
    pub label: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub quotas: Vec<maidan_types::TokenQuota>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePeer {
    pub name: String,
    pub base_url: String,
    /// Workspace on the remote peer to poll; defaults to the path workspace when omitted.
    pub remote_workspace_id: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PeerResponse {
    pub id: maidan_types::PeerId,
    pub workspace_id: maidan_types::WorkspaceId,
    pub remote_workspace_id: maidan_types::WorkspaceId,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    pub last_synced_event_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<maidan_types::Peer> for PeerResponse {
    fn from(p: maidan_types::Peer) -> Self {
        Self {
            id: p.id,
            workspace_id: p.workspace_id,
            remote_workspace_id: p.remote_workspace_id,
            name: p.name,
            base_url: p.base_url,
            enabled: p.enabled,
            last_synced_event_id: p.last_synced_event_id,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintPeerResponse {
    pub peer: PeerResponse,
    pub secret: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiTokenSummary {
    pub id: ApiTokenId,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub label: Option<String>,
    pub capabilities: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetMentionWebhook {
    pub webhook_id: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MentionWebhookConfig {
    pub webhook_id: Option<WebhookSubscriptionId>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintApiTokenResponse {
    pub id: ApiTokenId,
    pub secret: String,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub quotas: Vec<maidan_types::TokenQuota>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct OidcLoginQuery {
    pub workspace_id: uuid::Uuid,
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct OidcCallbackQuery {
    pub state: String,
    pub code: Option<String>,
    pub mock_sub: Option<String>,
    pub mock_email: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionResponse {
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    pub expires_at: DateTime<Utc>,
}
