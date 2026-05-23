//! Postgres implementation of [`crate::store::Store`].

mod artifacts;
mod audit;
mod channels;
mod members;
mod mentions;
mod messages;
mod refs;
mod thread_transitions;
mod threads;
mod votes;
mod workspaces;

use async_trait::async_trait;
use maidan_types::*;
use sqlx::PgPool;

use crate::error::StoreError;
use crate::store::Store;

#[derive(Debug, Clone)]
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl Store for PostgresStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace, StoreError> {
        workspaces::create(&self.pool, new).await
    }
    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace, StoreError> {
        workspaces::get(&self.pool, id).await
    }

    async fn create_member(&self, new: NewMember) -> Result<Member, StoreError> {
        members::create(&self.pool, new).await
    }
    async fn get_member(&self, id: MemberId) -> Result<Member, StoreError> {
        members::get(&self.pool, id).await
    }
    async fn list_members(&self, workspace_id: WorkspaceId) -> Result<Vec<Member>, StoreError> {
        members::list(&self.pool, workspace_id).await
    }

    async fn create_channel(&self, new: NewChannel) -> Result<Channel, StoreError> {
        channels::create(&self.pool, new).await
    }
    async fn get_channel(&self, id: ChannelId) -> Result<Channel, StoreError> {
        channels::get(&self.pool, id).await
    }
    async fn list_channels(&self, workspace_id: WorkspaceId) -> Result<Vec<Channel>, StoreError> {
        channels::list(&self.pool, workspace_id).await
    }

    async fn create_thread(&self, new: NewThread) -> Result<Thread, StoreError> {
        threads::create(&self.pool, new).await
    }
    async fn get_thread(&self, id: ThreadId) -> Result<Thread, StoreError> {
        threads::get(&self.pool, id).await
    }
    async fn list_threads(&self, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError> {
        threads::list(&self.pool, channel_id).await
    }

    async fn transition_thread(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
        action: maidan_fsm::ThreadAction,
    ) -> Result<ThreadTransitionResult, StoreError> {
        thread_transitions::transition(&self.pool, thread_id, actor_id, action).await
    }

    async fn post_message(&self, new: NewMessage) -> Result<Message, StoreError> {
        messages::create(&self.pool, new).await
    }
    async fn get_message(&self, id: MessageId) -> Result<Message, StoreError> {
        messages::get(&self.pool, id).await
    }
    async fn list_messages(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<Message>, StoreError> {
        messages::list(&self.pool, thread_id, limit).await
    }
    async fn tombstone_message(&self, id: MessageId) -> Result<(), StoreError> {
        messages::tombstone(&self.pool, id).await
    }

    async fn record_mention(
        &self,
        message_id: MessageId,
        member_id: MemberId,
    ) -> Result<(), StoreError> {
        mentions::record(&self.pool, message_id, member_id).await
    }
    async fn list_mentions_for_member(
        &self,
        member_id: MemberId,
        limit: i64,
    ) -> Result<Vec<Mention>, StoreError> {
        mentions::list_for_member(&self.pool, member_id, limit).await
    }

    async fn cast_vote(&self, new: NewVote) -> Result<(), StoreError> {
        votes::cast(&self.pool, new).await
    }
    async fn list_votes_for_message(&self, message_id: MessageId) -> Result<Vec<Vote>, StoreError> {
        votes::list(&self.pool, message_id).await
    }

    async fn add_reference(&self, new: NewReference) -> Result<Reference, StoreError> {
        refs::create(&self.pool, new).await
    }
    async fn list_references_from(
        &self,
        src_kind: RefSide,
        src_id: uuid::Uuid,
    ) -> Result<Vec<Reference>, StoreError> {
        refs::list_from(&self.pool, src_kind, src_id).await
    }

    async fn upsert_artifact(&self, new: NewArtifact) -> Result<Artifact, StoreError> {
        artifacts::upsert(&self.pool, new).await
    }
    async fn get_artifact_by_sha(&self, sha256: &str) -> Result<Artifact, StoreError> {
        artifacts::get_by_sha(&self.pool, sha256).await
    }

    async fn append_audit(&self, new: NewAuditEvent) -> Result<AuditEvent, StoreError> {
        audit::append(&self.pool, new).await
    }
    async fn list_audit(&self, limit: i64) -> Result<Vec<AuditEvent>, StoreError> {
        audit::list(&self.pool, limit).await
    }
}
