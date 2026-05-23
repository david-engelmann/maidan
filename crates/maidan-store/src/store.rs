use async_trait::async_trait;
use maidan_types::*;

use crate::error::StoreError;

/// Backend-agnostic Maidan storage interface.
///
/// Implementations live in submodules (`postgres`, `sqlite`). Methods are
/// minimal CRUD plus the few list/query operations the server needs in
/// Cluster A. Richer queries (search, threading rollups) arrive in later
/// clusters via dedicated traits and crates.
#[async_trait]
pub trait Store: Send + Sync {
    async fn health_check(&self) -> Result<(), StoreError>;

    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace, StoreError>;
    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace, StoreError>;

    async fn create_member(&self, new: NewMember) -> Result<Member, StoreError>;
    async fn get_member(&self, id: MemberId) -> Result<Member, StoreError>;
    async fn list_members(&self, workspace_id: WorkspaceId) -> Result<Vec<Member>, StoreError>;

    async fn create_channel(&self, new: NewChannel) -> Result<Channel, StoreError>;
    async fn get_channel(&self, id: ChannelId) -> Result<Channel, StoreError>;
    async fn list_channels(&self, workspace_id: WorkspaceId) -> Result<Vec<Channel>, StoreError>;

    async fn create_thread(&self, new: NewThread) -> Result<Thread, StoreError>;
    async fn get_thread(&self, id: ThreadId) -> Result<Thread, StoreError>;
    async fn list_threads(&self, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError>;

    async fn transition_thread(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
        action: maidan_fsm::ThreadAction,
    ) -> Result<ThreadTransitionResult, StoreError>;

    async fn post_message(&self, new: NewMessage) -> Result<Message, StoreError>;
    async fn get_message(&self, id: MessageId) -> Result<Message, StoreError>;
    async fn list_messages(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<Message>, StoreError>;
    async fn tombstone_message(&self, id: MessageId) -> Result<(), StoreError>;

    async fn record_mention(
        &self,
        message_id: MessageId,
        member_id: MemberId,
    ) -> Result<(), StoreError>;
    async fn list_mentions_for_member(
        &self,
        member_id: MemberId,
        limit: i64,
    ) -> Result<Vec<Mention>, StoreError>;

    async fn cast_vote(&self, new: NewVote) -> Result<(), StoreError>;
    async fn list_votes_for_message(&self, message_id: MessageId) -> Result<Vec<Vote>, StoreError>;

    async fn add_reference(&self, new: NewReference) -> Result<Reference, StoreError>;
    async fn list_references_from(
        &self,
        src_kind: RefSide,
        src_id: uuid::Uuid,
    ) -> Result<Vec<Reference>, StoreError>;

    async fn upsert_artifact(&self, new: NewArtifact) -> Result<Artifact, StoreError>;
    async fn get_artifact_by_sha(&self, sha256: &str) -> Result<Artifact, StoreError>;

    async fn append_audit(&self, new: NewAuditEvent) -> Result<AuditEvent, StoreError>;
    async fn list_audit(&self, limit: i64) -> Result<Vec<AuditEvent>, StoreError>;

    async fn append_event(&self, event: &Event) -> Result<StoredEvent, StoreError>;
    async fn list_events_after(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;
}
