//! Domain models. Each `<X>` has a paired `New<X>` for inserts so the
//! caller can build state-less values without populating server-assigned
//! fields (id, timestamps).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    Human,
    Agent,
}

impl MemberKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    Open,
    InReview,
    Closed,
    Archived,
}

impl ThreadState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InReview => "in_review",
            Self::Closed => "closed",
            Self::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "in_review" => Some(Self::InReview),
            "closed" => Some(Self::Closed),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RefSide {
    Thread,
    Message,
}

impl RefSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Message => "message",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Screenshot,
    Recording,
    Transcript,
    CodeDump,
    Attachment,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Screenshot => "screenshot",
            Self::Recording => "recording",
            Self::Transcript => "transcript",
            Self::CodeDump => "code_dump",
            Self::Attachment => "attachment",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "screenshot" => Some(Self::Screenshot),
            "recording" => Some(Self::Recording),
            "transcript" => Some(Self::Transcript),
            "code_dump" => Some(Self::CodeDump),
            "attachment" => Some(Self::Attachment),
            _ => None,
        }
    }

    pub fn default_mime(self) -> &'static str {
        match self {
            Self::Screenshot => "image/png",
            Self::Recording | Self::Attachment => "application/octet-stream",
            Self::Transcript | Self::CodeDump => "text/plain",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewWorkspace {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Member {
    pub id: MemberId,
    pub workspace_id: WorkspaceId,
    pub handle: String,
    pub display_name: Option<String>,
    pub kind: MemberKind,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewMember {
    pub workspace_id: WorkspaceId,
    pub handle: String,
    pub display_name: Option<String>,
    pub kind: MemberKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Channel {
    pub id: ChannelId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub topic: Option<String>,
    pub private: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewChannel {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub topic: Option<String>,
    pub private: bool,
}

/// System channel name for DM threads in a workspace.
pub const DM_CHANNEL_NAME: &str = "__dm__";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DmConversation {
    pub id: DmConversationId,
    pub workspace_id: WorkspaceId,
    pub member_low_id: MemberId,
    pub member_high_id: MemberId,
    pub thread_id: ThreadId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenDmConversation {
    pub other_member_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PostDmMessage {
    pub author_id: uuid::Uuid,
    pub body: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Thread {
    pub id: ThreadId,
    pub channel_id: ChannelId,
    pub parent_thread_id: Option<ThreadId>,
    pub title: Option<String>,
    pub state: ThreadState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewThread {
    pub channel_id: ChannelId,
    pub parent_thread_id: Option<ThreadId>,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThreadTransitionResult {
    pub thread: Thread,
    pub from_state: ThreadState,
    pub to_state: ThreadState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadTransition {
    pub id: uuid::Uuid,
    pub thread_id: ThreadId,
    pub from_state: ThreadState,
    pub to_state: ThreadState,
    pub actor_id: MemberId,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Message {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub author_id: MemberId,
    pub body: String,
    pub metadata: serde_json::Value,
    pub posted_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub thread_id: ThreadId,
    pub author_id: MemberId,
    pub body: String,
    pub metadata: serde_json::Value,
}

/// Body/metadata replacement for [`Store::edit_message`] (Cluster 29).
#[derive(Debug, Clone)]
pub struct EditMessage {
    pub body: String,
    pub metadata: serde_json::Value,
}

/// One recorded body change for a message (Cluster 46).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageEdit {
    pub id: i64,
    pub message_id: MessageId,
    pub editor_id: MemberId,
    pub body_before: String,
    pub body_after: String,
    pub edited_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Mention {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum InboxItemKind {
    Mention,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InboxItem {
    pub kind: InboxItemKind,
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
    pub unread: bool,
    pub message_body: String,
    pub thread_id: ThreadId,
    pub channel_id: ChannelId,
    pub author_id: MemberId,
    pub author_handle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberInbox {
    pub items: Vec<InboxItem>,
    pub unread_count: i64,
    pub last_read_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MarkInboxRead {
    pub read_through: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Vote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewVote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Reaction {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReaction {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Pin {
    pub thread_id: ThreadId,
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPin {
    pub thread_id: ThreadId,
    pub message_id: MessageId,
    pub member_id: MemberId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Reference {
    pub id: uuid::Uuid,
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReference {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Artifact {
    pub id: ArtifactId,
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub kind: ArtifactKind,
    pub uploaded_by: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewArtifact {
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub kind: ArtifactKind,
    pub uploaded_by: Option<MemberId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub occurred_at: DateTime<Utc>,
    pub actor_id: Option<MemberId>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: AppId,
    pub workspace_id: WorkspaceId,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewApp {
    pub workspace_id: WorkspaceId,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: MemberId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInstallation {
    pub id: AppInstallationId,
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub bot_member_id: MemberId,
    pub granted_capabilities: Vec<String>,
    pub installed_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewAppInstallation {
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub bot_member_id: MemberId,
    pub granted_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: ApiTokenId,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub app_installation_id: Option<AppInstallationId>,
    pub token_hash: String,
    pub label: Option<String>,
    pub capabilities: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewApiToken {
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub app_installation_id: Option<AppInstallationId>,
    pub token_hash: String,
    pub label: Option<String>,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TokenQuota {
    pub capability: String,
    pub max_per_window: u32,
    pub window_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Peer {
    pub id: PeerId,
    pub workspace_id: WorkspaceId,
    pub remote_workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    #[serde(skip_serializing)]
    pub outbound_secret_ciphertext: Option<String>,
    pub enabled: bool,
    pub last_synced_event_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPeer {
    pub workspace_id: WorkspaceId,
    pub remote_workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    pub token_hash: String,
    pub outbound_secret_ciphertext: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcIdentity {
    pub id: OidcIdentityId,
    pub workspace_id: WorkspaceId,
    pub issuer: String,
    pub subject: String,
    pub member_id: MemberId,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_login_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MaidanSession {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub csrf_secret: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewMaidanSession {
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub csrf_secret: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewOidcIdentity {
    pub workspace_id: WorkspaceId,
    pub issuer: String,
    pub subject: String,
    pub member_id: MemberId,
    pub email: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OidcPendingAuth {
    pub state: String,
    pub workspace_id: WorkspaceId,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewOidcPendingAuth {
    pub state: String,
    pub workspace_id: WorkspaceId,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAuditEvent {
    pub actor_id: Option<MemberId>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookSubscription {
    pub id: WebhookSubscriptionId,
    pub workspace_id: WorkspaceId,
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewWebhookSubscription {
    pub workspace_id: WorkspaceId,
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionDelivery {
    pub id: i64,
    pub subscription_id: WebhookSubscriptionId,
    pub log_id: i64,
    pub payload: String,
    pub attempts: i32,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionWithSecret {
    pub subscription: WebhookSubscription,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SlashHandlerKind {
    Http,
    McpTool,
}

impl SlashHandlerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::McpTool => "mcp_tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "http" => Some(Self::Http),
            "mcp_tool" => Some(Self::McpTool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlashCommand {
    pub id: SlashCommandId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewSlashCommand {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone)]
pub struct SlashCommandWithSecret {
    pub command: SlashCommand,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FsmHook {
    pub id: FsmHookId,
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub from_state: Option<ThreadState>,
    pub to_state: Option<ThreadState>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewFsmHook {
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub from_state: Option<ThreadState>,
    pub to_state: Option<ThreadState>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone)]
pub struct FsmHookWithSecret {
    pub hook: FsmHook,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AutomationSourceKind {
    SlashCommand,
    FsmHook,
}

impl AutomationSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SlashCommand => "slash_command",
            Self::FsmHook => "fsm_hook",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "slash_command" => Some(Self::SlashCommand),
            "fsm_hook" => Some(Self::FsmHook),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AutomationDelivery {
    pub id: i64,
    pub workspace_id: WorkspaceId,
    pub source_kind: AutomationSourceKind,
    pub source_id: uuid::Uuid,
    pub target_url: String,
    pub header_name: String,
    pub header_value: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub quarantined_at: Option<DateTime<Utc>>,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookDelivery {
    pub id: i64,
    pub workspace_id: WorkspaceId,
    pub subscription_id: WebhookSubscriptionId,
    pub log_id: i64,
    pub target_url: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub quarantined_at: Option<DateTime<Utc>>,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperatorDelivery {
    Automation(AutomationDelivery),
    Webhook(WebhookDelivery),
}

#[derive(Debug, Clone)]
pub struct AutomationDeliveryPending {
    pub id: i64,
    pub workspace_id: WorkspaceId,
    pub source_kind: AutomationSourceKind,
    pub source_id: uuid::Uuid,
    pub target_url: String,
    pub header_name: String,
    pub header_value: String,
    pub payload: String,
    pub attempts: i32,
}

#[derive(Debug, Clone)]
pub struct NewAutomationDelivery {
    pub workspace_id: WorkspaceId,
    pub source_kind: AutomationSourceKind,
    pub source_id: uuid::Uuid,
    pub target_url: String,
    pub header_name: String,
    pub header_value: String,
    pub payload: String,
}
