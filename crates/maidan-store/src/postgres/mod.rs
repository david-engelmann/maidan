//! Postgres implementation of [`crate::store::Store`].

mod apps;
mod artifacts;
mod audit;
mod automation_deliveries;
mod channels;
pub mod delivery_cursor;
mod dm;
mod erase_workspace;
pub mod events;
mod fsm_hooks;
mod inbox;
mod members;
mod mentions;
mod message_edits;
mod messages;
mod oidc;
pub mod outbox;
mod peers;
mod pins;
mod purge_workspace;
mod reactions;
mod refs;
mod sessions;
mod slash_commands;
mod thread_transitions;
mod threads;
mod token_quotas;
mod tokens;
mod votes;
mod webhooks;
mod workspaces;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

    async fn open_dm_conversation(
        &self,
        workspace_id: WorkspaceId,
        member_a: MemberId,
        member_b: MemberId,
    ) -> Result<DmConversation, StoreError> {
        dm::open(&self.pool, workspace_id, member_a, member_b).await
    }
    async fn get_dm_conversation(
        &self,
        id: DmConversationId,
    ) -> Result<DmConversation, StoreError> {
        dm::get(&self.pool, id).await
    }
    async fn list_dm_conversations_for_member(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<DmConversation>, StoreError> {
        dm::list_for_member(&self.pool, workspace_id, member_id).await
    }
    async fn dm_conversation_for_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<DmConversation>, StoreError> {
        dm::get_for_thread(&self.pool, thread_id).await
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

    async fn list_thread_transitions(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<ThreadTransition>, StoreError> {
        thread_transitions::list(&self.pool, thread_id, limit).await
    }

    async fn post_message(&self, new: NewMessage) -> Result<Message, StoreError> {
        messages::create(&self.pool, new).await
    }
    async fn edit_message(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
    ) -> Result<Message, StoreError> {
        messages::edit(&self.pool, id, editor_id, edit).await
    }
    async fn list_message_edits(
        &self,
        message_id: MessageId,
        limit: i64,
    ) -> Result<Vec<MessageEdit>, StoreError> {
        message_edits::list(&self.pool, message_id, limit).await
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
    async fn purge_workspace_messages(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspacePurgeResult, StoreError> {
        purge_workspace::purge(&self.pool, workspace_id).await
    }
    async fn erase_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceEraseResult, StoreError> {
        erase_workspace::erase(&self.pool, workspace_id).await
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

    async fn get_inbox_last_read_at(
        &self,
        member_id: MemberId,
    ) -> Result<DateTime<Utc>, StoreError> {
        inbox::get_last_read_at(&self.pool, member_id).await
    }

    async fn advance_inbox_last_read_at(
        &self,
        member_id: MemberId,
        read_through: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, StoreError> {
        inbox::advance_last_read_at(&self.pool, member_id, read_through).await
    }

    async fn list_member_inbox(
        &self,
        member_id: MemberId,
        limit: i64,
    ) -> Result<MemberInbox, StoreError> {
        inbox::list_for_member(&self.pool, member_id, limit).await
    }

    async fn cast_vote(&self, new: NewVote) -> Result<(), StoreError> {
        votes::cast(&self.pool, new).await
    }
    async fn list_votes_for_message(&self, message_id: MessageId) -> Result<Vec<Vote>, StoreError> {
        votes::list(&self.pool, message_id).await
    }

    async fn add_reaction(&self, new: NewReaction) -> Result<(), StoreError> {
        reactions::add(&self.pool, new).await
    }
    async fn remove_reaction(
        &self,
        message_id: MessageId,
        member_id: MemberId,
        emoji: &str,
    ) -> Result<bool, StoreError> {
        reactions::remove(&self.pool, message_id, member_id, emoji).await
    }
    async fn list_reactions_for_message(
        &self,
        message_id: MessageId,
    ) -> Result<Vec<Reaction>, StoreError> {
        reactions::list(&self.pool, message_id).await
    }

    async fn pin_message(&self, new: NewPin) -> Result<(), StoreError> {
        pins::pin(&self.pool, new).await
    }
    async fn unpin_message(
        &self,
        thread_id: ThreadId,
        message_id: MessageId,
    ) -> Result<bool, StoreError> {
        pins::unpin(&self.pool, thread_id, message_id).await
    }
    async fn list_pins_for_thread(&self, thread_id: ThreadId) -> Result<Vec<Pin>, StoreError> {
        pins::list_for_thread(&self.pool, thread_id).await
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
    async fn list_audit_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<AuditEvent>, StoreError> {
        audit::list_for_workspace(&self.pool, workspace_id, limit).await
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

    async fn create_app(&self, new: NewApp) -> Result<App, StoreError> {
        apps::create_app(&self.pool, new).await
    }

    async fn get_app(&self, id: AppId) -> Result<App, StoreError> {
        apps::get_app(&self.pool, id).await
    }

    async fn list_apps(&self, workspace_id: WorkspaceId) -> Result<Vec<App>, StoreError> {
        apps::list_apps(&self.pool, workspace_id).await
    }

    async fn create_app_installation(
        &self,
        new: NewAppInstallation,
    ) -> Result<AppInstallation, StoreError> {
        apps::create_installation(&self.pool, new).await
    }

    async fn get_app_installation(
        &self,
        id: AppInstallationId,
    ) -> Result<AppInstallation, StoreError> {
        apps::get_installation(&self.pool, id).await
    }

    async fn list_app_installations(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<AppInstallation>, StoreError> {
        apps::list_installations(&self.pool, workspace_id).await
    }

    async fn revoke_app_installation(
        &self,
        id: AppInstallationId,
    ) -> Result<AppInstallation, StoreError> {
        apps::revoke_installation(&self.pool, id).await
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

    async fn replace_token_quotas(
        &self,
        token_id: ApiTokenId,
        quotas: &[TokenQuota],
    ) -> Result<(), StoreError> {
        token_quotas::replace(&self.pool, token_id, quotas).await
    }

    async fn list_token_quotas(&self, token_id: ApiTokenId) -> Result<Vec<TokenQuota>, StoreError> {
        token_quotas::list(&self.pool, token_id).await
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
        consumer_id: &str,
        workspace_id: WorkspaceId,
    ) -> Result<i64, StoreError> {
        delivery_cursor::get_cursor(&self.pool, consumer_id, workspace_id).await
    }

    async fn advance_delivery_cursor(
        &self,
        consumer_id: &str,
        workspace_id: WorkspaceId,
        log_id: i64,
    ) -> Result<i64, StoreError> {
        delivery_cursor::advance_cursor(&self.pool, consumer_id, workspace_id, log_id).await
    }

    async fn create_webhook_subscription(
        &self,
        new: NewWebhookSubscription,
    ) -> Result<WebhookSubscription, StoreError> {
        webhooks::create(&self.pool, new).await
    }

    async fn list_webhook_subscriptions(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WebhookSubscription>, StoreError> {
        webhooks::list(&self.pool, workspace_id).await
    }

    async fn revoke_webhook_subscription(
        &self,
        id: WebhookSubscriptionId,
    ) -> Result<WebhookSubscription, StoreError> {
        webhooks::revoke(&self.pool, id).await
    }

    async fn list_enabled_webhook_subscriptions(
        &self,
    ) -> Result<Vec<WebhookSubscriptionWithSecret>, StoreError> {
        let rows = webhooks::list_enabled(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| WebhookSubscriptionWithSecret {
                subscription: row.subscription,
                secret_ciphertext: row.secret_ciphertext,
            })
            .collect())
    }

    async fn get_webhook_subscription(
        &self,
        id: WebhookSubscriptionId,
    ) -> Result<WebhookSubscriptionWithSecret, StoreError> {
        let row = webhooks::get(&self.pool, id).await?;
        Ok(WebhookSubscriptionWithSecret {
            subscription: row.subscription,
            secret_ciphertext: row.secret_ciphertext,
        })
    }

    async fn enqueue_webhook_delivery(
        &self,
        subscription_id: WebhookSubscriptionId,
        log_id: i64,
        payload: &str,
    ) -> Result<i64, StoreError> {
        webhooks::enqueue_delivery(&self.pool, subscription_id, log_id, payload).await
    }

    async fn list_pending_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<WebhookSubscriptionDelivery>, StoreError> {
        webhooks::list_pending_deliveries(&self.pool, limit).await
    }

    async fn mark_webhook_delivery_delivered(&self, delivery_id: i64) -> Result<(), StoreError> {
        webhooks::mark_delivered(&self.pool, delivery_id).await
    }

    async fn record_webhook_delivery_attempt(
        &self,
        delivery_id: i64,
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<i32, StoreError> {
        webhooks::record_delivery_attempt(&self.pool, delivery_id, error, next_attempt_at).await
    }

    async fn quarantine_webhook_delivery(&self, delivery_id: i64) -> Result<(), StoreError> {
        webhooks::quarantine_delivery(&self.pool, delivery_id).await
    }

    async fn enqueue_automation_delivery(
        &self,
        new: NewAutomationDelivery,
    ) -> Result<i64, StoreError> {
        automation_deliveries::enqueue(&self.pool, new).await
    }

    async fn list_pending_automation_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<AutomationDeliveryPending>, StoreError> {
        automation_deliveries::list_pending(&self.pool, limit).await
    }

    async fn list_automation_deliveries(
        &self,
        workspace_id: WorkspaceId,
        filter: crate::AutomationDeliveryFilter,
        limit: i64,
    ) -> Result<Vec<AutomationDelivery>, StoreError> {
        automation_deliveries::list_for_workspace(&self.pool, workspace_id, filter, limit).await
    }

    async fn get_automation_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<AutomationDelivery, StoreError> {
        automation_deliveries::get(&self.pool, delivery_id, workspace_id).await
    }

    async fn mark_automation_delivery_delivered(&self, delivery_id: i64) -> Result<(), StoreError> {
        automation_deliveries::mark_delivered(&self.pool, delivery_id).await
    }

    async fn record_automation_delivery_attempt(
        &self,
        delivery_id: i64,
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<i32, StoreError> {
        automation_deliveries::record_attempt(&self.pool, delivery_id, error, next_attempt_at).await
    }

    async fn quarantine_automation_delivery(&self, delivery_id: i64) -> Result<(), StoreError> {
        automation_deliveries::quarantine(&self.pool, delivery_id).await
    }

    async fn replay_automation_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<AutomationDelivery, StoreError> {
        automation_deliveries::replay(&self.pool, delivery_id, workspace_id).await
    }

    async fn create_slash_command(&self, new: NewSlashCommand) -> Result<SlashCommand, StoreError> {
        slash_commands::create(&self.pool, new).await
    }

    async fn list_slash_commands(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SlashCommand>, StoreError> {
        slash_commands::list(&self.pool, workspace_id).await
    }

    async fn revoke_slash_command(&self, id: SlashCommandId) -> Result<SlashCommand, StoreError> {
        slash_commands::revoke(&self.pool, id).await
    }

    async fn get_slash_command(
        &self,
        id: SlashCommandId,
    ) -> Result<SlashCommandWithSecret, StoreError> {
        slash_commands::get(&self.pool, id).await
    }

    async fn get_slash_command_by_name(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<SlashCommandWithSecret, StoreError> {
        slash_commands::get_by_name(&self.pool, workspace_id, name).await
    }

    async fn create_fsm_hook(&self, new: NewFsmHook) -> Result<FsmHook, StoreError> {
        fsm_hooks::create(&self.pool, new).await
    }

    async fn list_fsm_hooks(&self, workspace_id: WorkspaceId) -> Result<Vec<FsmHook>, StoreError> {
        fsm_hooks::list(&self.pool, workspace_id).await
    }

    async fn revoke_fsm_hook(&self, id: FsmHookId) -> Result<FsmHook, StoreError> {
        fsm_hooks::revoke(&self.pool, id).await
    }

    async fn get_fsm_hook(&self, id: FsmHookId) -> Result<FsmHookWithSecret, StoreError> {
        fsm_hooks::get(&self.pool, id).await
    }

    async fn list_matching_fsm_hooks(
        &self,
        workspace_id: WorkspaceId,
        from_state: ThreadState,
        to_state: ThreadState,
    ) -> Result<Vec<FsmHookWithSecret>, StoreError> {
        fsm_hooks::list_matching(&self.pool, workspace_id, from_state, to_state).await
    }
}
