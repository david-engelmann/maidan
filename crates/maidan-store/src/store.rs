use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
    async fn count_workspaces(&self) -> Result<i64, StoreError>;
    /// Live per-workspace usage counts (members/channels/threads/messages,
    /// excluding tombstoned rows) for metering (Cluster 188).
    async fn workspace_usage(&self, id: WorkspaceId) -> Result<WorkspaceUsage, StoreError>;

    async fn create_member(&self, new: NewMember) -> Result<Member, StoreError>;
    async fn get_member(&self, id: MemberId) -> Result<Member, StoreError>;
    async fn get_member_by_handle(
        &self,
        workspace_id: WorkspaceId,
        handle: &str,
    ) -> Result<Member, StoreError>;
    async fn list_members(&self, workspace_id: WorkspaceId) -> Result<Vec<Member>, StoreError>;

    async fn upsert_oidc_identity(&self, new: NewOidcIdentity) -> Result<OidcIdentity, StoreError>;
    async fn get_oidc_identity(
        &self,
        workspace_id: WorkspaceId,
        issuer: &str,
        subject: &str,
    ) -> Result<OidcIdentity, StoreError>;
    async fn insert_oidc_pending(&self, new: NewOidcPendingAuth) -> Result<(), StoreError>;
    async fn take_oidc_pending(&self, state: &str) -> Result<OidcPendingAuth, StoreError>;

    async fn create_session(&self, new: NewMaidanSession) -> Result<MaidanSession, StoreError>;
    async fn get_session(&self, id: SessionId) -> Result<MaidanSession, StoreError>;
    async fn delete_session(&self, id: SessionId) -> Result<(), StoreError>;

    async fn create_channel(&self, new: NewChannel) -> Result<Channel, StoreError>;
    async fn get_channel(&self, id: ChannelId) -> Result<Channel, StoreError>;
    async fn list_channels(&self, workspace_id: WorkspaceId) -> Result<Vec<Channel>, StoreError>;

    /// Per-channel membership (Cluster 159). Public channels are open to the
    /// workspace and need no rows; these gate private channels. `add` is an
    /// idempotent upsert of the role.
    async fn add_channel_member(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        role: ChannelMemberRole,
    ) -> Result<ChannelMember, StoreError>;
    async fn remove_channel_member(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
    ) -> Result<(), StoreError>;
    async fn list_channel_members(
        &self,
        channel_id: ChannelId,
    ) -> Result<Vec<ChannelMember>, StoreError>;
    async fn channel_is_member(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
    ) -> Result<bool, StoreError>;

    async fn open_dm_conversation(
        &self,
        workspace_id: WorkspaceId,
        member_a: MemberId,
        member_b: MemberId,
    ) -> Result<DmConversation, StoreError>;
    async fn get_dm_conversation(&self, id: DmConversationId)
        -> Result<DmConversation, StoreError>;
    async fn list_dm_conversations_for_member(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<DmConversation>, StoreError>;
    async fn dm_conversation_for_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<DmConversation>, StoreError>;

    async fn open_group_dm_conversation(
        &self,
        workspace_id: WorkspaceId,
        member_ids: &[MemberId],
        title: Option<String>,
    ) -> Result<GroupDmConversation, StoreError>;
    async fn get_group_dm_conversation(
        &self,
        id: GroupDmConversationId,
    ) -> Result<GroupDmConversation, StoreError>;
    async fn list_group_dm_conversations_for_member(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<GroupDmConversation>, StoreError>;
    async fn group_dm_conversation_for_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<GroupDmConversation>, StoreError>;
    async fn group_dm_has_member(
        &self,
        id: GroupDmConversationId,
        member_id: MemberId,
    ) -> Result<bool, StoreError>;

    async fn create_thread(&self, new: NewThread) -> Result<Thread, StoreError>;
    async fn get_thread(&self, id: ThreadId) -> Result<Thread, StoreError>;
    async fn list_threads(&self, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError>;

    /// All threads across a workspace's channels, in one query (Cluster 106).
    /// Avoids the per-channel `list_threads` N+1 when assembling workspace
    /// context. Ordered by `created_at DESC`.
    async fn list_threads_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<Thread>, StoreError>;

    /// One keyset page of a workspace's **live** threads, ordered
    /// `(created_at, id)` ascending. `after` is an exclusive cursor (the last
    /// thread id of the prior page); `None` starts from the beginning. Unlike
    /// [`list_threads_for_workspace`](Self::list_threads_for_workspace), this
    /// filters tombstoned threads and applies `LIMIT` in SQL, so workspace
    /// context assembly does not load every thread into memory.
    async fn page_threads_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        after: Option<ThreadId>,
        limit: i64,
    ) -> Result<Vec<Thread>, StoreError>;

    async fn transition_thread(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
        action: maidan_fsm::ThreadAction,
    ) -> Result<ThreadTransitionResult, StoreError>;

    async fn list_thread_transitions(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<ThreadTransition>, StoreError>;

    /// Set a thread's assignee unconditionally (assign / handoff). `NotFound` if
    /// the thread doesn't exist (Cluster 171).
    async fn assign_thread(
        &self,
        thread_id: ThreadId,
        assignee_id: MemberId,
    ) -> Result<Thread, StoreError>;

    /// Atomically claim an unassigned thread for `member_id` (Cluster 171). The
    /// compare-and-set (`WHERE assignee_id IS NULL`) makes concurrent claims
    /// race-safe: exactly one wins. `claimed` is `false` when it was already
    /// assigned; `NotFound` only when the thread doesn't exist.
    async fn claim_thread(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<ThreadClaimResult, StoreError>;

    /// Clear a thread's assignee (Cluster 171). `NotFound` if it doesn't exist.
    async fn unassign_thread(&self, thread_id: ThreadId) -> Result<Thread, StoreError>;

    /// Threads in `workspace_id` assigned to `member_id` — the agent's work queue
    /// (Cluster 190). Live threads only, oldest first.
    async fn list_assigned_threads(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<Thread>, StoreError>;

    /// Atomically claim the oldest unassigned live thread in `channel_id` for
    /// `member_id` — the "pull the next task" primitive (Cluster 190). `None` when
    /// there is no unassigned work. Concurrent claimers each get a distinct thread.
    async fn claim_next_thread(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
    ) -> Result<Option<Thread>, StoreError>;

    async fn post_message(&self, new: NewMessage) -> Result<Message, StoreError>;
    async fn edit_message(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
    ) -> Result<Message, StoreError>;
    async fn list_message_edits(
        &self,
        message_id: MessageId,
        limit: i64,
    ) -> Result<Vec<MessageEdit>, StoreError>;

    /// Edits for many messages in one windowed query (Cluster 106), at most
    /// `limit_per` per message (newest-last, like `list_message_edits`). Avoids
    /// the per-message edit N+1 in thread-context assembly. Returns a flat list
    /// the caller groups by `message_id`.
    async fn list_message_edits_for_messages(
        &self,
        message_ids: &[MessageId],
        limit_per: i64,
    ) -> Result<Vec<MessageEdit>, StoreError>;
    async fn get_message(&self, id: MessageId) -> Result<Message, StoreError>;
    async fn list_messages(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<Message>, StoreError>;
    /// Messages after `after` (exclusive), ordered by `posted_at ASC` then `id ASC`.
    async fn list_messages_after(
        &self,
        thread_id: ThreadId,
        after: Option<MessageId>,
        limit: i64,
    ) -> Result<Vec<Message>, StoreError>;
    async fn tombstone_message(&self, id: MessageId) -> Result<(), StoreError>;
    /// Hard-delete a tombstoned message (GDPR erasure). Fails if not tombstoned.
    async fn purge_message(&self, id: MessageId) -> Result<(), StoreError>;
    /// Tombstone then hard-delete all messages in a workspace (GDPR erasure).
    async fn purge_workspace_messages(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspacePurgeResult, StoreError>;
    /// Deep purge then delete the workspace row and CASCADE-owned data (Cluster 53).
    async fn erase_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceEraseResult, StoreError>;

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

    async fn get_inbox_last_read_at(
        &self,
        member_id: MemberId,
    ) -> Result<DateTime<Utc>, StoreError>;

    async fn advance_inbox_last_read_at(
        &self,
        member_id: MemberId,
        read_through: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, StoreError>;

    async fn list_member_inbox(
        &self,
        member_id: MemberId,
        limit: i64,
    ) -> Result<MemberInbox, StoreError>;

    async fn cast_vote(&self, new: NewVote) -> Result<(), StoreError>;
    async fn list_votes_for_message(&self, message_id: MessageId) -> Result<Vec<Vote>, StoreError>;

    async fn add_reaction(&self, new: NewReaction) -> Result<(), StoreError>;
    async fn remove_reaction(
        &self,
        message_id: MessageId,
        member_id: MemberId,
        emoji: &str,
    ) -> Result<bool, StoreError>;
    async fn list_reactions_for_message(
        &self,
        message_id: MessageId,
    ) -> Result<Vec<Reaction>, StoreError>;

    async fn pin_message(&self, new: NewPin) -> Result<(), StoreError>;
    async fn unpin_message(
        &self,
        thread_id: ThreadId,
        message_id: MessageId,
    ) -> Result<bool, StoreError>;
    async fn list_pins_for_thread(&self, thread_id: ThreadId) -> Result<Vec<Pin>, StoreError>;

    async fn add_reference(&self, new: NewReference) -> Result<Reference, StoreError>;
    async fn list_references_from(
        &self,
        src_kind: RefSide,
        src_id: uuid::Uuid,
    ) -> Result<Vec<Reference>, StoreError>;

    /// References from many sources of one kind in a single query (Cluster 106).
    /// Avoids the per-message `list_references_from` N+1. Returns a flat list the
    /// caller groups by `src_id`; ordered by `src_id` then `created_at ASC`.
    async fn list_references_from_many(
        &self,
        src_kind: RefSide,
        src_ids: &[uuid::Uuid],
    ) -> Result<Vec<Reference>, StoreError>;

    async fn upsert_artifact(&self, new: NewArtifact) -> Result<Artifact, StoreError>;
    async fn get_artifact_by_sha(&self, sha256: &str) -> Result<Artifact, StoreError>;

    async fn append_audit(&self, new: NewAuditEvent) -> Result<AuditEvent, StoreError>;
    async fn list_audit(&self, limit: i64) -> Result<Vec<AuditEvent>, StoreError>;
    async fn list_audit_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<AuditEvent>, StoreError>;

    async fn append_event(&self, event: &Event) -> Result<StoredEvent, StoreError>;
    async fn get_stored_event(&self, log_id: i64) -> Result<StoredEvent, StoreError>;
    async fn list_events_after(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Like [`Store::list_events_after`] but only rows inserted at or before
    /// `stable_before` (the at-least-once reconcile read; Cluster 125).
    async fn list_events_after_stable(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
        stable_before: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    async fn create_app(&self, new: NewApp) -> Result<App, StoreError>;
    async fn get_app(&self, id: AppId) -> Result<App, StoreError>;
    async fn list_apps(&self, workspace_id: WorkspaceId) -> Result<Vec<App>, StoreError>;
    async fn create_app_installation(
        &self,
        new: NewAppInstallation,
    ) -> Result<AppInstallation, StoreError>;
    async fn get_app_installation(
        &self,
        id: AppInstallationId,
    ) -> Result<AppInstallation, StoreError>;
    async fn list_app_installations(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<AppInstallation>, StoreError>;
    async fn revoke_app_installation(
        &self,
        id: AppInstallationId,
    ) -> Result<AppInstallation, StoreError>;

    /// Persist a one-time OAuth authorization code (Cluster 104). The plaintext
    /// code is never stored — `code_hash` is its SHA-256 digest.
    async fn insert_oauth_code(&self, new: NewOAuthCode) -> Result<(), StoreError>;

    /// Atomically consume (delete) a non-expired authorization code by hash.
    /// Returns `None` if the code is unknown, already consumed, or expired —
    /// guaranteeing single use across replicas.
    async fn consume_oauth_code(&self, code_hash: &str) -> Result<Option<OAuthCode>, StoreError>;

    /// Insert or update an embedding reindex job (Cluster 104). Keyed by
    /// `job_id`, so the start record and later status updates upsert the row,
    /// making job status visible on any replica.
    async fn upsert_reindex_job(&self, job: ReindexJob) -> Result<(), StoreError>;

    /// Fetch a reindex job by id, or `None` if unknown.
    async fn get_reindex_job(&self, job_id: uuid::Uuid) -> Result<Option<ReindexJob>, StoreError>;

    async fn create_api_token(&self, new: NewApiToken) -> Result<ApiToken, StoreError>;
    async fn get_api_token(&self, id: ApiTokenId) -> Result<ApiToken, StoreError>;
    async fn get_active_api_token_by_hash(&self, token_hash: &str) -> Result<ApiToken, StoreError>;
    async fn revoke_api_token(&self, id: ApiTokenId) -> Result<ApiToken, StoreError>;
    async fn list_api_tokens_for_member(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<ApiToken>, StoreError>;

    async fn get_workspace_mention_webhook_id(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WebhookSubscriptionId>, StoreError>;
    async fn set_workspace_mention_webhook_id(
        &self,
        workspace_id: WorkspaceId,
        webhook_id: Option<WebhookSubscriptionId>,
    ) -> Result<(), StoreError>;
    async fn replace_token_quotas(
        &self,
        token_id: ApiTokenId,
        quotas: &[TokenQuota],
    ) -> Result<(), StoreError>;
    async fn list_token_quotas(&self, token_id: ApiTokenId) -> Result<Vec<TokenQuota>, StoreError>;
    async fn workspace_has_active_capability(
        &self,
        workspace_id: WorkspaceId,
        capability: &str,
    ) -> Result<bool, StoreError>;

    async fn create_peer(&self, new: NewPeer) -> Result<Peer, StoreError>;
    async fn get_peer(&self, id: PeerId) -> Result<Peer, StoreError>;
    async fn get_peer_by_token_hash(&self, token_hash: &str) -> Result<Peer, StoreError>;
    async fn list_peers(&self, workspace_id: WorkspaceId) -> Result<Vec<Peer>, StoreError>;
    async fn list_enabled_peers(&self) -> Result<Vec<Peer>, StoreError>;
    async fn update_peer_cursor(
        &self,
        id: PeerId,
        last_synced_event_id: i64,
    ) -> Result<Peer, StoreError>;
    async fn delete_peer(&self, id: PeerId) -> Result<(), StoreError>;
    async fn federated_ingest_exists(
        &self,
        peer_id: PeerId,
        remote_event_id: i64,
    ) -> Result<bool, StoreError>;
    async fn try_record_federated_ingest(
        &self,
        peer_id: PeerId,
        remote_event_id: i64,
        local_event_id: i64,
    ) -> Result<bool, StoreError>;
    async fn is_federated_local_event(&self, local_event_id: i64) -> Result<bool, StoreError>;

    /// Last `log_id` delivered to `consumer_id` in `workspace_id` (0 if none).
    async fn get_delivery_cursor(
        &self,
        consumer_id: &str,
        workspace_id: WorkspaceId,
    ) -> Result<i64, StoreError>;

    /// Monotonic advance; returns the stored cursor after update.
    async fn advance_delivery_cursor(
        &self,
        consumer_id: &str,
        workspace_id: WorkspaceId,
        log_id: i64,
    ) -> Result<i64, StoreError>;

    /// Lowest `last_delivered_log_id` across all at-least-once delivery cursors,
    /// or `None` when no cursor exists. An event with `id` at or below this has
    /// been delivered to every durable consumer and is safe to prune — the
    /// retention floor for the event log (Cluster 186).
    async fn min_delivery_cursor(&self) -> Result<Option<i64>, StoreError>;

    /// Delete up to `limit` oldest event-log rows with `id <= max_id` **and**
    /// `occurred_at < cutoff`. The `max_id` floor (see [`Store::min_delivery_cursor`])
    /// keeps events a lagging at-least-once consumer still needs. Returns the row
    /// count deleted; the caller loops until it is below `limit` (Cluster 186).
    async fn prune_events(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        max_id: i64,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// Delete up to `limit` oldest audit rows with `occurred_at < cutoff`.
    async fn prune_audit(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// Delete up to `limit` oldest **terminal** (delivered or quarantined)
    /// webhook + automation delivery rows created before `cutoff`. In-flight
    /// (pending/retrying) rows are never pruned regardless of age.
    async fn prune_deliveries(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    async fn create_webhook_subscription(
        &self,
        new: NewWebhookSubscription,
    ) -> Result<WebhookSubscription, StoreError>;
    async fn list_webhook_subscriptions(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WebhookSubscription>, StoreError>;
    async fn revoke_webhook_subscription(
        &self,
        id: WebhookSubscriptionId,
    ) -> Result<WebhookSubscription, StoreError>;
    async fn list_enabled_webhook_subscriptions(
        &self,
    ) -> Result<Vec<WebhookSubscriptionWithSecret>, StoreError>;
    /// Enabled webhook subscriptions for one workspace — the per-event hot path
    /// (avoids scanning every workspace's subscriptions; Cluster 166).
    async fn list_enabled_webhook_subscriptions_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WebhookSubscriptionWithSecret>, StoreError>;
    async fn get_webhook_subscription(
        &self,
        id: WebhookSubscriptionId,
    ) -> Result<WebhookSubscriptionWithSecret, StoreError>;
    async fn enqueue_webhook_delivery(
        &self,
        subscription_id: WebhookSubscriptionId,
        log_id: i64,
        payload: &str,
    ) -> Result<i64, StoreError>;
    async fn list_pending_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<WebhookSubscriptionDelivery>, StoreError>;
    async fn mark_webhook_delivery_delivered(&self, delivery_id: i64) -> Result<(), StoreError>;
    async fn record_webhook_delivery_attempt(
        &self,
        delivery_id: i64,
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<i32, StoreError>;
    async fn quarantine_webhook_delivery(&self, delivery_id: i64) -> Result<(), StoreError>;
    async fn list_webhook_deliveries(
        &self,
        workspace_id: WorkspaceId,
        filter: crate::AutomationDeliveryFilter,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, StoreError>;
    async fn get_webhook_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<WebhookDelivery, StoreError>;
    async fn replay_webhook_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<WebhookDelivery, StoreError>;

    async fn enqueue_automation_delivery(
        &self,
        new: NewAutomationDelivery,
    ) -> Result<i64, StoreError>;
    async fn list_pending_automation_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<AutomationDeliveryPending>, StoreError>;
    async fn list_automation_deliveries(
        &self,
        workspace_id: WorkspaceId,
        filter: crate::AutomationDeliveryFilter,
        limit: i64,
    ) -> Result<Vec<AutomationDelivery>, StoreError>;
    async fn get_automation_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<AutomationDelivery, StoreError>;
    async fn mark_automation_delivery_delivered(&self, delivery_id: i64) -> Result<(), StoreError>;
    async fn record_automation_delivery_attempt(
        &self,
        delivery_id: i64,
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<i32, StoreError>;
    async fn quarantine_automation_delivery(&self, delivery_id: i64) -> Result<(), StoreError>;
    async fn replay_automation_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<AutomationDelivery, StoreError>;

    async fn create_slash_command(&self, new: NewSlashCommand) -> Result<SlashCommand, StoreError>;
    async fn list_slash_commands(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SlashCommand>, StoreError>;
    async fn revoke_slash_command(&self, id: SlashCommandId) -> Result<SlashCommand, StoreError>;
    async fn get_slash_command(
        &self,
        id: SlashCommandId,
    ) -> Result<SlashCommandWithSecret, StoreError>;
    async fn get_slash_command_by_name(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<SlashCommandWithSecret, StoreError>;

    async fn create_fsm_hook(&self, new: NewFsmHook) -> Result<FsmHook, StoreError>;
    async fn list_fsm_hooks(&self, workspace_id: WorkspaceId) -> Result<Vec<FsmHook>, StoreError>;
    async fn revoke_fsm_hook(&self, id: FsmHookId) -> Result<FsmHook, StoreError>;
    async fn get_fsm_hook(&self, id: FsmHookId) -> Result<FsmHookWithSecret, StoreError>;
    async fn list_matching_fsm_hooks(
        &self,
        workspace_id: WorkspaceId,
        from_state: ThreadState,
        to_state: ThreadState,
    ) -> Result<Vec<FsmHookWithSecret>, StoreError>;

    async fn upsert_a2a_push_config(
        &self,
        workspace_id: WorkspaceId,
        push_url: &str,
    ) -> Result<(), StoreError>;
    async fn get_a2a_push_config(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<String>, StoreError>;

    async fn upsert_a2a_task(
        &self,
        workspace_id: WorkspaceId,
        task_id: &str,
        task_json: serde_json::Value,
    ) -> Result<(), StoreError>;
    async fn get_a2a_task(&self, task_id: &str) -> Result<Option<serde_json::Value>, StoreError>;
    async fn get_a2a_task_workspace(
        &self,
        task_id: &str,
    ) -> Result<Option<WorkspaceId>, StoreError>;
}
