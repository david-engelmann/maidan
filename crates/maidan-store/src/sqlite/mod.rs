//! SQLite implementation of [`crate::store::Store`]. Mirrors the Postgres
//! impl 1:1, with SQLite-flavored SQL (`?` placeholders, TEXT-encoded
//! UUIDs and timestamps, no `RETURNING` for the audit table's
//! AUTOINCREMENT id — we read it back via `last_insert_rowid()`).

mod artifacts;
mod audit;
mod channels;
mod events;
mod members;
mod mentions;
mod messages;
mod oidc;
mod peers;
mod pragmas;
mod refs;
mod sessions;
mod thread_transitions;
mod threads;
mod tokens;
mod votes;
mod workspaces;

use async_trait::async_trait;
use maidan_types::*;
use sqlx::SqlitePool;

use crate::error::StoreError;
use crate::store::Store;

pub use pragmas::configure_pool;

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait]
impl Store for SqliteStore {
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
    async fn count_workspaces(&self) -> Result<i64, StoreError> {
        workspaces::count(&self.pool).await
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
    async fn get_member_by_handle(
        &self,
        workspace_id: WorkspaceId,
        handle: &str,
    ) -> Result<Member, StoreError> {
        members::get_by_handle(&self.pool, workspace_id, handle).await
    }

    async fn upsert_oidc_identity(&self, new: NewOidcIdentity) -> Result<OidcIdentity, StoreError> {
        oidc::upsert_identity(&self.pool, new).await
    }
    async fn get_oidc_identity(
        &self,
        workspace_id: WorkspaceId,
        issuer: &str,
        subject: &str,
    ) -> Result<OidcIdentity, StoreError> {
        oidc::get_identity(&self.pool, workspace_id, issuer, subject).await
    }
    async fn insert_oidc_pending(&self, new: NewOidcPendingAuth) -> Result<(), StoreError> {
        oidc::insert_pending(&self.pool, new).await
    }
    async fn take_oidc_pending(&self, state: &str) -> Result<OidcPendingAuth, StoreError> {
        oidc::take_pending(&self.pool, state).await
    }

    async fn create_session(&self, new: NewMaidanSession) -> Result<MaidanSession, StoreError> {
        sessions::create(&self.pool, new).await
    }
    async fn get_session(&self, id: SessionId) -> Result<MaidanSession, StoreError> {
        sessions::get(&self.pool, id).await
    }
    async fn delete_session(&self, id: SessionId) -> Result<(), StoreError> {
        sessions::delete(&self.pool, id).await
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
    async fn purge_message(&self, id: MessageId) -> Result<(), StoreError> {
        messages::purge(&self.pool, id).await
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

    async fn append_event(&self, event: &Event) -> Result<StoredEvent, StoreError> {
        events::append(&self.pool, event).await
    }

    async fn get_stored_event(&self, log_id: i64) -> Result<StoredEvent, StoreError> {
        events::get_by_id(&self.pool, log_id).await
    }

    async fn list_events_after(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        events::list_after(&self.pool, workspace_id, after_id, limit).await
    }

    async fn create_api_token(&self, new: NewApiToken) -> Result<ApiToken, StoreError> {
        tokens::create(&self.pool, new).await
    }

    async fn get_api_token(&self, id: ApiTokenId) -> Result<ApiToken, StoreError> {
        tokens::get_by_id(&self.pool, id).await
    }

    async fn get_active_api_token_by_hash(&self, token_hash: &str) -> Result<ApiToken, StoreError> {
        tokens::get_active_by_hash(&self.pool, token_hash).await
    }

    async fn revoke_api_token(&self, id: ApiTokenId) -> Result<ApiToken, StoreError> {
        tokens::revoke(&self.pool, id).await
    }

    async fn workspace_has_active_capability(
        &self,
        workspace_id: WorkspaceId,
        capability: &str,
    ) -> Result<bool, StoreError> {
        tokens::workspace_has_active_capability(&self.pool, workspace_id, capability).await
    }

    async fn create_peer(&self, new: NewPeer) -> Result<Peer, StoreError> {
        peers::create(&self.pool, new).await
    }

    async fn get_peer(&self, id: PeerId) -> Result<Peer, StoreError> {
        peers::get(&self.pool, id).await
    }

    async fn get_peer_by_token_hash(&self, token_hash: &str) -> Result<Peer, StoreError> {
        peers::get_by_token_hash(&self.pool, token_hash).await
    }

    async fn list_peers(&self, workspace_id: WorkspaceId) -> Result<Vec<Peer>, StoreError> {
        peers::list(&self.pool, workspace_id).await
    }

    async fn list_enabled_peers(&self) -> Result<Vec<Peer>, StoreError> {
        peers::list_enabled(&self.pool).await
    }

    async fn update_peer_cursor(
        &self,
        id: PeerId,
        last_synced_event_id: i64,
    ) -> Result<Peer, StoreError> {
        peers::update_cursor(&self.pool, id, last_synced_event_id).await
    }

    async fn delete_peer(&self, id: PeerId) -> Result<(), StoreError> {
        peers::delete(&self.pool, id).await
    }

    async fn federated_ingest_exists(
        &self,
        peer_id: PeerId,
        remote_event_id: i64,
    ) -> Result<bool, StoreError> {
        peers::ingest_exists(&self.pool, peer_id, remote_event_id).await
    }

    async fn try_record_federated_ingest(
        &self,
        peer_id: PeerId,
        remote_event_id: i64,
        local_event_id: i64,
    ) -> Result<bool, StoreError> {
        peers::try_record_ingest(&self.pool, peer_id, remote_event_id, local_event_id).await
    }

    async fn is_federated_local_event(&self, local_event_id: i64) -> Result<bool, StoreError> {
        peers::is_federated_local_event(&self.pool, local_event_id).await
    }

    async fn get_delivery_cursor(
        &self,
        _consumer_id: &str,
        _workspace_id: WorkspaceId,
    ) -> Result<i64, StoreError> {
        Ok(0)
    }

    async fn advance_delivery_cursor(
        &self,
        _consumer_id: &str,
        _workspace_id: WorkspaceId,
        log_id: i64,
    ) -> Result<i64, StoreError> {
        Ok(log_id)
    }
}
