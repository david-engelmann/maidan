//! Postgres implementation of [`crate::store::Store`].

mod a2a;
mod apps;
mod artifacts;
mod audit;
mod automation_deliveries;
mod channel_members;
mod channels;
pub mod delivery_cursor;
mod dm;
mod erase_workspace;
pub mod events;
mod fsm_hooks;
mod group_dm;
mod inbox;
mod member_skills;
mod members;
mod mentions;
mod message_edits;
mod messages;
mod notifications;
mod oauth_codes;
mod oidc;
pub mod outbox;
mod peers;
mod pins;
mod purge_workspace;
mod reactions;
mod refs;
mod reindex_jobs;
mod retention;
mod sessions;
mod slash_commands;
mod task_schedules;
mod thread_deps;
mod thread_results;
mod thread_skills;
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
    async fn create_workspace_with_event(
        &self,
        new: NewWorkspace,
    ) -> Result<(Workspace, StoredEvent), StoreError> {
        workspaces::create_with_event(&self.pool, new).await
    }
    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace, StoreError> {
        workspaces::get(&self.pool, id).await
    }
    async fn count_workspaces(&self) -> Result<i64, StoreError> {
        workspaces::count(&self.pool).await
    }
    async fn workspace_usage(&self, id: WorkspaceId) -> Result<WorkspaceUsage, StoreError> {
        workspaces::usage(&self.pool, id).await
    }

    async fn create_member(&self, new: NewMember) -> Result<Member, StoreError> {
        members::create(&self.pool, new).await
    }
    async fn create_member_with_event(
        &self,
        new: NewMember,
    ) -> Result<(Member, StoredEvent), StoreError> {
        members::create_with_event(&self.pool, new).await
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

    async fn add_member_skill(&self, member_id: MemberId, skill: &str) -> Result<(), StoreError> {
        member_skills::add(&self.pool, member_id, skill).await
    }
    async fn remove_member_skill(
        &self,
        member_id: MemberId,
        skill: &str,
    ) -> Result<bool, StoreError> {
        member_skills::remove(&self.pool, member_id, skill).await
    }
    async fn list_member_skills(
        &self,
        member_id: MemberId,
    ) -> Result<Vec<MemberSkill>, StoreError> {
        member_skills::list(&self.pool, member_id).await
    }
    async fn add_thread_required_skill(
        &self,
        thread_id: ThreadId,
        skill: &str,
    ) -> Result<(), StoreError> {
        thread_skills::add(&self.pool, thread_id, skill).await
    }
    async fn remove_thread_required_skill(
        &self,
        thread_id: ThreadId,
        skill: &str,
    ) -> Result<bool, StoreError> {
        thread_skills::remove(&self.pool, thread_id, skill).await
    }
    async fn list_thread_required_skills(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadRequiredSkill>, StoreError> {
        thread_skills::list(&self.pool, thread_id).await
    }
    async fn set_thread_result(
        &self,
        thread_id: ThreadId,
        produced_by: MemberId,
        result: &serde_json::Value,
    ) -> Result<ThreadResult, StoreError> {
        thread_results::set(&self.pool, thread_id, produced_by, result).await
    }
    async fn get_thread_result(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadResult>, StoreError> {
        thread_results::get(&self.pool, thread_id).await
    }

    async fn create_notification(&self, new: NewNotification) -> Result<Notification, StoreError> {
        notifications::create(&self.pool, new).await
    }
    async fn list_notifications(
        &self,
        member_id: MemberId,
        unread_only: bool,
        limit: i64,
    ) -> Result<Vec<Notification>, StoreError> {
        notifications::list_for_member(&self.pool, member_id, unread_only, limit).await
    }
    async fn mark_notification_read(&self, id: NotificationId) -> Result<bool, StoreError> {
        notifications::mark_read(&self.pool, id).await
    }
    async fn mark_all_notifications_read(&self, member_id: MemberId) -> Result<u64, StoreError> {
        notifications::mark_all_read(&self.pool, member_id).await
    }
    async fn unread_notification_count(&self, member_id: MemberId) -> Result<i64, StoreError> {
        notifications::unread_count(&self.pool, member_id).await
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

    async fn create_channel_with_event(
        &self,
        new: NewChannel,
    ) -> Result<(Channel, StoredEvent), StoreError> {
        channels::create_with_event(&self.pool, new).await
    }
    async fn get_channel(&self, id: ChannelId) -> Result<Channel, StoreError> {
        channels::get(&self.pool, id).await
    }
    async fn list_channels(&self, workspace_id: WorkspaceId) -> Result<Vec<Channel>, StoreError> {
        channels::list(&self.pool, workspace_id).await
    }

    async fn add_channel_member(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        role: ChannelMemberRole,
    ) -> Result<ChannelMember, StoreError> {
        channel_members::add(&self.pool, channel_id, member_id, role).await
    }
    async fn remove_channel_member(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
    ) -> Result<(), StoreError> {
        channel_members::remove(&self.pool, channel_id, member_id).await
    }
    async fn list_channel_members(
        &self,
        channel_id: ChannelId,
    ) -> Result<Vec<ChannelMember>, StoreError> {
        channel_members::list(&self.pool, channel_id).await
    }
    async fn channel_is_member(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
    ) -> Result<bool, StoreError> {
        channel_members::is_member(&self.pool, channel_id, member_id).await
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

    async fn open_group_dm_conversation(
        &self,
        workspace_id: WorkspaceId,
        member_ids: &[MemberId],
        title: Option<String>,
    ) -> Result<GroupDmConversation, StoreError> {
        group_dm::open(&self.pool, workspace_id, member_ids, title).await
    }
    async fn get_group_dm_conversation(
        &self,
        id: GroupDmConversationId,
    ) -> Result<GroupDmConversation, StoreError> {
        group_dm::get(&self.pool, id).await
    }
    async fn list_group_dm_conversations_for_member(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<GroupDmConversation>, StoreError> {
        group_dm::list_for_member(&self.pool, workspace_id, member_id).await
    }
    async fn group_dm_conversation_for_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<GroupDmConversation>, StoreError> {
        group_dm::get_for_thread(&self.pool, thread_id).await
    }
    async fn group_dm_has_member(
        &self,
        id: GroupDmConversationId,
        member_id: MemberId,
    ) -> Result<bool, StoreError> {
        group_dm::is_member(&self.pool, id, member_id).await
    }

    async fn create_thread(&self, new: NewThread) -> Result<Thread, StoreError> {
        threads::create(&self.pool, new).await
    }

    async fn create_thread_with_event(
        &self,
        new: NewThread,
    ) -> Result<(Thread, StoredEvent), StoreError> {
        threads::create_with_event(&self.pool, new).await
    }
    async fn get_thread(&self, id: ThreadId) -> Result<Thread, StoreError> {
        threads::get(&self.pool, id).await
    }
    async fn list_threads(&self, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError> {
        threads::list(&self.pool, channel_id).await
    }
    async fn list_threads_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<Thread>, StoreError> {
        threads::list_for_workspace(&self.pool, workspace_id).await
    }

    async fn page_threads_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        after: Option<ThreadId>,
        limit: i64,
    ) -> Result<Vec<Thread>, StoreError> {
        threads::page_for_workspace(&self.pool, workspace_id, after, limit).await
    }

    async fn transition_thread(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
        action: maidan_fsm::ThreadAction,
    ) -> Result<ThreadTransitionResult, StoreError> {
        thread_transitions::transition(&self.pool, thread_id, actor_id, action).await
    }
    async fn transition_thread_with_event(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
        action: maidan_fsm::ThreadAction,
    ) -> Result<(ThreadTransitionResult, StoredEvent), StoreError> {
        thread_transitions::transition_with_event(&self.pool, thread_id, actor_id, action).await
    }

    async fn list_thread_transitions(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<ThreadTransition>, StoreError> {
        thread_transitions::list(&self.pool, thread_id, limit).await
    }

    async fn channel_queue_depth(&self, channel_id: ChannelId) -> Result<QueueDepth, StoreError> {
        threads::channel_queue_depth(&self.pool, channel_id).await
    }

    async fn create_task_schedule(&self, new: NewTaskSchedule) -> Result<TaskSchedule, StoreError> {
        task_schedules::create(&self.pool, new).await
    }
    async fn get_task_schedule(&self, id: TaskScheduleId) -> Result<TaskSchedule, StoreError> {
        task_schedules::get(&self.pool, id).await
    }
    async fn list_task_schedules(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<TaskSchedule>, StoreError> {
        task_schedules::list(&self.pool, workspace_id).await
    }
    async fn delete_task_schedule(&self, id: TaskScheduleId) -> Result<bool, StoreError> {
        task_schedules::delete(&self.pool, id).await
    }
    async fn due_task_schedules(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<TaskSchedule>, StoreError> {
        task_schedules::due(&self.pool, now, limit).await
    }
    async fn claim_next_due_schedule(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Option<TaskSchedule>, StoreError> {
        task_schedules::claim_next_due(&self.pool, now).await
    }
    async fn set_task_schedule_active(
        &self,
        id: TaskScheduleId,
        active: bool,
    ) -> Result<TaskSchedule, StoreError> {
        task_schedules::set_active(&self.pool, id, active).await
    }

    async fn assign_thread(
        &self,
        thread_id: ThreadId,
        assignee_id: MemberId,
    ) -> Result<Thread, StoreError> {
        threads::assign(&self.pool, thread_id, assignee_id).await
    }
    async fn assign_thread_with_event(
        &self,
        thread_id: ThreadId,
        assignee_id: MemberId,
        actor_id: MemberId,
        note: Option<String>,
    ) -> Result<(Thread, StoredEvent), StoreError> {
        threads::assign_with_event(&self.pool, thread_id, assignee_id, actor_id, note).await
    }

    async fn claim_thread(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<ThreadClaimResult, StoreError> {
        threads::claim(&self.pool, thread_id, member_id).await
    }
    async fn claim_thread_with_event(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<(ThreadClaimResult, Option<StoredEvent>), StoreError> {
        threads::claim_with_event(&self.pool, thread_id, member_id).await
    }

    async fn unassign_thread(&self, thread_id: ThreadId) -> Result<Thread, StoreError> {
        threads::unassign(&self.pool, thread_id).await
    }
    async fn unassign_thread_with_event(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
    ) -> Result<(Thread, StoredEvent), StoreError> {
        threads::unassign_with_event(&self.pool, thread_id, actor_id).await
    }
    async fn list_assigned_threads(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<Thread>, StoreError> {
        threads::list_assigned(&self.pool, workspace_id, member_id).await
    }
    async fn claim_next_thread(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        lease_secs: Option<i64>,
    ) -> Result<Option<Thread>, StoreError> {
        threads::claim_next(&self.pool, channel_id, member_id, lease_secs).await
    }
    async fn claim_next_thread_with_event(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        lease_secs: Option<i64>,
    ) -> Result<(Option<Thread>, Option<StoredEvent>), StoreError> {
        threads::claim_next_with_event(&self.pool, channel_id, member_id, lease_secs).await
    }
    async fn renew_claim(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
        lease_secs: i64,
    ) -> Result<Thread, StoreError> {
        threads::renew_claim(&self.pool, thread_id, member_id, lease_secs).await
    }

    async fn add_thread_dependency(
        &self,
        thread_id: ThreadId,
        depends_on: ThreadId,
    ) -> Result<(), StoreError> {
        thread_deps::add(&self.pool, thread_id, depends_on).await
    }
    async fn remove_thread_dependency(
        &self,
        thread_id: ThreadId,
        depends_on: ThreadId,
    ) -> Result<bool, StoreError> {
        thread_deps::remove(&self.pool, thread_id, depends_on).await
    }
    async fn list_thread_dependencies(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadDependency>, StoreError> {
        thread_deps::list_dependencies(&self.pool, thread_id).await
    }
    async fn list_thread_dependents(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadDependency>, StoreError> {
        thread_deps::list_dependents(&self.pool, thread_id).await
    }
    async fn thread_dependencies_satisfied(&self, thread_id: ThreadId) -> Result<bool, StoreError> {
        thread_deps::dependencies_satisfied(&self.pool, thread_id).await
    }
    async fn newly_ready_dependents(&self, thread_id: ThreadId) -> Result<Vec<Thread>, StoreError> {
        thread_deps::newly_ready_dependents(&self.pool, thread_id).await
    }

    async fn post_message(&self, new: NewMessage) -> Result<Message, StoreError> {
        messages::create(&self.pool, new).await
    }
    async fn post_message_with_event(
        &self,
        new: NewMessage,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<(Message, StoredEvent), StoreError> {
        messages::create_with_event(&self.pool, new, dm_conversation_id).await
    }
    async fn edit_message_with_posted_event(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<(Message, StoredEvent), StoreError> {
        messages::edit_with_posted_event(&self.pool, id, editor_id, edit, dm_conversation_id).await
    }
    async fn edit_message(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
    ) -> Result<Message, StoreError> {
        messages::edit(&self.pool, id, editor_id, edit).await
    }
    async fn edit_message_with_event(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<(Message, StoredEvent), StoreError> {
        messages::edit_with_event(&self.pool, id, editor_id, edit, dm_conversation_id).await
    }
    async fn list_message_edits(
        &self,
        message_id: MessageId,
        limit: i64,
    ) -> Result<Vec<MessageEdit>, StoreError> {
        message_edits::list(&self.pool, message_id, limit).await
    }
    async fn list_message_edits_for_messages(
        &self,
        message_ids: &[MessageId],
        limit_per: i64,
    ) -> Result<Vec<MessageEdit>, StoreError> {
        message_edits::list_for_messages(&self.pool, message_ids, limit_per).await
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

    async fn list_messages_after(
        &self,
        thread_id: ThreadId,
        after: Option<MessageId>,
        limit: i64,
    ) -> Result<Vec<Message>, StoreError> {
        messages::list_after(&self.pool, thread_id, after, limit).await
    }

    async fn tombstone_message(&self, id: MessageId) -> Result<(), StoreError> {
        messages::tombstone(&self.pool, id).await
    }
    async fn tombstone_message_with_event(
        &self,
        id: MessageId,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<StoredEvent, StoreError> {
        messages::tombstone_with_event(&self.pool, id, dm_conversation_id).await
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
    async fn record_mention_with_event(
        &self,
        message_id: MessageId,
        member_id: MemberId,
    ) -> Result<StoredEvent, StoreError> {
        mentions::record_with_event(&self.pool, message_id, member_id).await
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
    async fn cast_vote_with_event(&self, new: NewVote) -> Result<StoredEvent, StoreError> {
        votes::cast_with_event(&self.pool, new).await
    }
    async fn list_votes_for_message(&self, message_id: MessageId) -> Result<Vec<Vote>, StoreError> {
        votes::list(&self.pool, message_id).await
    }

    async fn add_reaction(&self, new: NewReaction) -> Result<(), StoreError> {
        reactions::add(&self.pool, new).await
    }
    async fn add_reaction_with_event(&self, new: NewReaction) -> Result<StoredEvent, StoreError> {
        reactions::add_with_event(&self.pool, new).await
    }
    async fn remove_reaction(
        &self,
        message_id: MessageId,
        member_id: MemberId,
        emoji: &str,
    ) -> Result<bool, StoreError> {
        reactions::remove(&self.pool, message_id, member_id, emoji).await
    }
    async fn remove_reaction_with_event(
        &self,
        message_id: MessageId,
        member_id: MemberId,
        emoji: &str,
    ) -> Result<(bool, Option<StoredEvent>), StoreError> {
        reactions::remove_with_event(&self.pool, message_id, member_id, emoji).await
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
    async fn pin_message_with_event(&self, new: NewPin) -> Result<StoredEvent, StoreError> {
        pins::pin_with_event(&self.pool, new).await
    }
    async fn unpin_message(
        &self,
        thread_id: ThreadId,
        message_id: MessageId,
    ) -> Result<bool, StoreError> {
        pins::unpin(&self.pool, thread_id, message_id).await
    }
    async fn unpin_message_with_event(
        &self,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
    ) -> Result<(bool, Option<StoredEvent>), StoreError> {
        pins::unpin_with_event(&self.pool, thread_id, message_id, member_id).await
    }
    async fn list_pins_for_thread(&self, thread_id: ThreadId) -> Result<Vec<Pin>, StoreError> {
        pins::list_for_thread(&self.pool, thread_id).await
    }

    async fn add_reference(&self, new: NewReference) -> Result<Reference, StoreError> {
        refs::create(&self.pool, new).await
    }
    async fn add_reference_with_event(
        &self,
        new: NewReference,
    ) -> Result<(Reference, StoredEvent), StoreError> {
        refs::create_with_event(&self.pool, new).await
    }
    async fn list_references_from(
        &self,
        src_kind: RefSide,
        src_id: uuid::Uuid,
    ) -> Result<Vec<Reference>, StoreError> {
        refs::list_from(&self.pool, src_kind, src_id).await
    }
    async fn list_references_from_many(
        &self,
        src_kind: RefSide,
        src_ids: &[uuid::Uuid],
    ) -> Result<Vec<Reference>, StoreError> {
        refs::list_from_many(&self.pool, src_kind, src_ids).await
    }

    async fn upsert_artifact(&self, new: NewArtifact) -> Result<Artifact, StoreError> {
        artifacts::upsert(&self.pool, new).await
    }
    async fn upsert_artifact_with_event(
        &self,
        new: NewArtifact,
        ref_workspace: Option<WorkspaceId>,
    ) -> Result<(Artifact, StoredEvent), StoreError> {
        artifacts::upsert_with_event(&self.pool, new, ref_workspace).await
    }
    async fn get_artifact_by_sha(&self, sha256: &str) -> Result<Artifact, StoreError> {
        artifacts::get_by_sha(&self.pool, sha256).await
    }

    async fn record_artifact_ref(
        &self,
        workspace_id: WorkspaceId,
        sha256: &str,
    ) -> Result<(), StoreError> {
        artifacts::record_ref(&self.pool, workspace_id, sha256).await
    }

    async fn artifact_ref_exists(
        &self,
        workspace_id: WorkspaceId,
        sha256: &str,
    ) -> Result<bool, StoreError> {
        artifacts::ref_exists(&self.pool, workspace_id, sha256).await
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

    async fn list_events_after_stable(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
        stable_before: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        events::list_after_stable(&self.pool, workspace_id, after_id, stable_before, limit).await
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

    async fn insert_oauth_code(&self, new: NewOAuthCode) -> Result<(), StoreError> {
        oauth_codes::insert(&self.pool, new).await
    }

    async fn consume_oauth_code(&self, code_hash: &str) -> Result<Option<OAuthCode>, StoreError> {
        oauth_codes::consume(&self.pool, code_hash).await
    }

    async fn upsert_reindex_job(&self, job: ReindexJob) -> Result<(), StoreError> {
        reindex_jobs::upsert(&self.pool, job).await
    }

    async fn get_reindex_job(&self, job_id: uuid::Uuid) -> Result<Option<ReindexJob>, StoreError> {
        reindex_jobs::get(&self.pool, job_id).await
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
    async fn list_api_tokens_for_member(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<ApiToken>, StoreError> {
        tokens::list_for_member(&self.pool, workspace_id, member_id).await
    }
    async fn get_workspace_mention_webhook_id(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WebhookSubscriptionId>, StoreError> {
        workspaces::get_mention_webhook_id(&self.pool, workspace_id).await
    }
    async fn set_workspace_mention_webhook_id(
        &self,
        workspace_id: WorkspaceId,
        webhook_id: Option<WebhookSubscriptionId>,
    ) -> Result<(), StoreError> {
        workspaces::set_mention_webhook_id(&self.pool, workspace_id, webhook_id).await
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

    async fn min_delivery_cursor(&self) -> Result<Option<i64>, StoreError> {
        retention::min_delivery_cursor(&self.pool).await
    }

    async fn prune_events(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        max_id: i64,
        limit: i64,
    ) -> Result<u64, StoreError> {
        retention::prune_events(&self.pool, cutoff, max_id, limit).await
    }

    async fn prune_audit(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError> {
        retention::prune_audit(&self.pool, cutoff, limit).await
    }

    async fn prune_deliveries(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError> {
        retention::prune_deliveries(&self.pool, cutoff, limit).await
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

    async fn list_enabled_webhook_subscriptions_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WebhookSubscriptionWithSecret>, StoreError> {
        let rows = webhooks::list_enabled_for_workspace(&self.pool, workspace_id).await?;
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

    async fn list_webhook_deliveries(
        &self,
        workspace_id: WorkspaceId,
        filter: crate::AutomationDeliveryFilter,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, StoreError> {
        webhooks::list_deliveries_for_workspace(&self.pool, workspace_id, filter, limit).await
    }

    async fn get_webhook_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<WebhookDelivery, StoreError> {
        webhooks::get_delivery(&self.pool, delivery_id, workspace_id).await
    }

    async fn replay_webhook_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<WebhookDelivery, StoreError> {
        webhooks::replay_delivery(&self.pool, delivery_id, workspace_id).await
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

    async fn upsert_a2a_push_config(
        &self,
        workspace_id: WorkspaceId,
        push_url: &str,
    ) -> Result<(), StoreError> {
        a2a::upsert_push_config(&self.pool, workspace_id, push_url).await
    }

    async fn get_a2a_push_config(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<String>, StoreError> {
        a2a::get_push_config(&self.pool, workspace_id).await
    }

    async fn upsert_a2a_task(
        &self,
        workspace_id: WorkspaceId,
        task_id: &str,
        task_json: serde_json::Value,
    ) -> Result<(), StoreError> {
        a2a::upsert_task(&self.pool, workspace_id, task_id, task_json).await
    }

    async fn get_a2a_task(&self, task_id: &str) -> Result<Option<serde_json::Value>, StoreError> {
        a2a::get_task(&self.pool, task_id).await
    }

    async fn get_a2a_task_workspace(
        &self,
        task_id: &str,
    ) -> Result<Option<WorkspaceId>, StoreError> {
        a2a::get_task_workspace(&self.pool, task_id).await
    }
}
