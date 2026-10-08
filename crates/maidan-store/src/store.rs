use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maidan_types::*;

use crate::a2a::{A2aPushConfigRow, A2aTaskQuery, A2aTaskRow, A2aTaskWrite, PendingGateQuery};
use crate::error::StoreError;

/// Backend-agnostic Maidan storage interface.
///
/// Implementations live in submodules (`postgres`, `sqlite`). Methods are
/// minimal CRUD plus the few list/query operations the server needs in
/// Cluster A. Richer queries (search, threading rollups) arrive in later
/// clusters via dedicated traits and crates.
#[async_trait]
pub trait MetaStore: Send + Sync {
    async fn health_check(&self) -> Result<(), StoreError>;

    /// The primary's current WAL write position — the read-replica causality
    /// token stamped on a write. `Some` on Postgres (`pg_current_wal_lsn()`),
    /// `None` on SQLite (no streaming replication, so no token).
    async fn write_lsn(&self) -> Result<Option<Lsn>, StoreError>;
}

/// Input for [`WorkspaceStore::provision_workspace`].
pub struct NewProvisionedTenant {
    pub name: String,
    pub admin_handle: String,
    pub token_hash: String,
    pub token_label: Option<String>,
    pub capabilities: Vec<String>,
}

/// What [`WorkspaceStore::provision_workspace`] committed.
pub struct ProvisionedTenant {
    pub workspace: Workspace,
    pub member: Member,
    pub token: ApiToken,
    pub events: Vec<StoredEvent>,
}

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    /// Insert a whole workspace content graph — members, channels, threads,
    /// messages, edits, pins, references — with explicit ids, state, and
    /// timestamps preserved, in one transaction. The inverse of the export. Id
    /// remapping (fresh workspace vs same-id restore) and the already-exists
    /// guard are the caller's job.
    async fn import_workspace(&self, import: &WorkspaceImport) -> Result<(), StoreError>;

    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace, StoreError>;
    /// Create a workspace and append its `WorkspaceCreated` event atomically.
    async fn create_workspace_with_event(
        &self,
        new: NewWorkspace,
    ) -> Result<(Workspace, StoredEvent), StoreError>;

    /// Create a workspace, its first human admin, and the admin token in
    /// one transaction, with the token audit row in the same transaction.
    /// The two domain events are returned for the caller to publish.
    async fn provision_workspace(
        &self,
        new: NewProvisionedTenant,
        audit: crate::AuditFor<ApiToken>,
    ) -> Result<ProvisionedTenant, StoreError>;
    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace, StoreError>;
    /// Set the workspace display name. The id is unchanged. `NotFound` when
    /// the workspace does not exist.
    async fn rename_workspace(&self, id: WorkspaceId, name: &str) -> Result<Workspace, StoreError>;
    async fn count_workspaces(&self) -> Result<i64, StoreError>;
    /// Live per-workspace usage counts (members/channels/threads/messages,
    /// excluding tombstoned rows) for metering.
    async fn workspace_usage(&self, id: WorkspaceId) -> Result<WorkspaceUsage, StoreError>;

    /// Place a legal hold on a workspace, for one matter. While any hold
    /// stands, the workspace's events are exempt from retention pruning, audit
    /// pruning is frozen, purge/erase is refused, and withdrawn messages keep
    /// their words. A second matter is a second hold.
    async fn place_legal_hold(
        &self,
        workspace_id: WorkspaceId,
        reason: &str,
        placed_by: Option<MemberId>,
    ) -> Result<LegalHold, StoreError>;
    /// Lift one of the workspace's holds. `true` when that hold existed. What
    /// the holds kept is disposed of when the last one is lifted.
    async fn lift_legal_hold(
        &self,
        workspace_id: WorkspaceId,
        hold_id: maidan_types::LegalHoldId,
    ) -> Result<bool, StoreError>;

    // D-A: the audited forms request handlers use (see `create_api_token_audited`).

    /// [`Self::import_workspace`] with its audit row in the same transaction.
    /// With `replace_existing`, the named workspace is erased first in that
    /// transaction (refused under legal hold), so a failed import leaves it
    /// intact.
    async fn import_workspace_audited(
        &self,
        import: &WorkspaceImport,
        replace_existing: bool,
        audit: NewAuditEvent,
    ) -> Result<(), StoreError>;
    /// [`Self::place_legal_hold`] with its audit row in the same transaction.
    async fn place_legal_hold_audited(
        &self,
        workspace_id: WorkspaceId,
        reason: &str,
        placed_by: Option<MemberId>,
        audit: crate::AuditFor<LegalHold>,
    ) -> Result<LegalHold, StoreError>;
    /// [`Self::lift_legal_hold`] with its audit row in the same transaction;
    /// nothing is recorded when there was no hold.
    async fn lift_legal_hold_audited(
        &self,
        workspace_id: WorkspaceId,
        hold_id: maidan_types::LegalHoldId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// What the workspace's legal hold kept of messages withdrawn while it
    /// held: their last words and earlier versions. The audit row is written
    /// first, in the same transaction; a read that cannot be recorded returns
    /// nothing.
    async fn read_preserved_messages_audited(
        &self,
        workspace_id: WorkspaceId,
        audit: NewAuditEvent,
    ) -> Result<Vec<maidan_types::PreservedMessage>, StoreError>;
    /// The workspace's legal holds, newest first; empty when it is not held.
    async fn list_workspace_legal_holds(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<LegalHold>, StoreError>;
    /// Every active legal hold, newest first — the operator view.
    async fn list_legal_holds(&self) -> Result<Vec<LegalHold>, StoreError>;

    /// Set (or clear) the workspace's WIP limit: the max concurrent *live*
    /// claims any one member may hold. `Some(n)` upserts (`0` freezes
    /// claiming); `None` removes the cap (unlimited).
    async fn set_wip_limit(
        &self,
        workspace_id: WorkspaceId,
        limit: Option<i64>,
    ) -> Result<(), StoreError>;
    /// The workspace's WIP limit, or `None` if unset (unlimited).
    async fn get_wip_limit(&self, workspace_id: WorkspaceId) -> Result<Option<i64>, StoreError>;

    /// The workspace's delegation policy: how long a grant may live (D-B).
    /// The default ceiling when it has set none.
    async fn get_delegation_policy(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<DelegationPolicy, StoreError>;
    /// Set the grant ceiling in days (1–3650), or restore the default with
    /// `None`. Applies to grants issued afterwards.
    async fn set_delegation_policy(
        &self,
        workspace_id: WorkspaceId,
        max_grant_days: Option<i64>,
    ) -> Result<DelegationPolicy, StoreError>;
    /// [`Self::set_delegation_policy`] with its audit row in the same
    /// transaction (D-A).
    async fn set_delegation_policy_audited(
        &self,
        workspace_id: WorkspaceId,
        max_grant_days: Option<i64>,
        audit: crate::AuditFor<DelegationPolicy>,
    ) -> Result<DelegationPolicy, StoreError>;

    /// The retention the workspace set for its messages, events and finished
    /// deliveries; all `None` when it has set none.
    async fn get_retention_policy(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<RetentionDays, StoreError>;
    /// Replace the workspace's retention, with its audit row in the same
    /// transaction (D-A). `InvalidInput` for a value outside
    /// 1..=[`MAX_RETENTION_DAYS`] or longer than `instance` keeps that kind of
    /// row; all `None` clears it. There is no unaudited form.
    async fn set_retention_policy_audited(
        &self,
        workspace_id: WorkspaceId,
        days: RetentionDays,
        instance: RetentionDays,
        audit: crate::AuditFor<RetentionDays>,
    ) -> Result<RetentionDays, StoreError>;
    /// Every workspace that has set a retention, for the sweeper.
    async fn list_retention_policies(
        &self,
    ) -> Result<Vec<(WorkspaceId, RetentionDays)>, StoreError>;

    /// Set or rename a workspace handle. The workspace id is unchanged. Invalid
    /// syntax is [`StoreError::InvalidInput`]; a handle owned by another
    /// workspace is [`StoreError::Conflict`].
    ///
    /// **A handle is a display label, not an address**. Nothing
    /// resolves a handle *to* a workspace, deliberately — see the ADR in
    /// `docs/Decisions.md`. Uniqueness is enforced by the table's constraint,
    /// not by a reverse lookup.
    async fn set_workspace_handle(
        &self,
        workspace_id: WorkspaceId,
        handle: &str,
    ) -> Result<WorkspaceHandle, StoreError>;
    /// The workspace's current handle, or `None`.
    async fn get_workspace_handle(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceHandle>, StoreError>;
}

#[async_trait]
pub trait MemberStore: Send + Sync {
    async fn create_member(&self, new: NewMember) -> Result<Member, StoreError>;
    /// Create a member and append its `MemberJoined` event atomically.
    async fn create_member_with_event(
        &self,
        new: NewMember,
    ) -> Result<(Member, StoredEvent), StoreError>;
    async fn get_member(&self, id: MemberId) -> Result<Member, StoreError>;
    /// The member `id` names, if it is one of `workspace_id`'s. A member of
    /// another workspace is `NotFound`, as an id that names no member is, so
    /// a caller cannot learn that the id is a member somewhere else.
    async fn get_member_in(
        &self,
        workspace_id: WorkspaceId,
        id: MemberId,
    ) -> Result<Member, StoreError> {
        let member = self.get_member(id).await?;
        if member.workspace_id == workspace_id {
            Ok(member)
        } else {
            Err(StoreError::NotFound)
        }
    }
    async fn get_member_by_handle(
        &self,
        workspace_id: WorkspaceId,
        handle: &str,
    ) -> Result<Member, StoreError>;
    async fn list_members(&self, workspace_id: WorkspaceId) -> Result<Vec<Member>, StoreError>;

    /// Create the SCIM provisioning link for a member.
    async fn create_scim_user(
        &self,
        member_id: MemberId,
        workspace_id: WorkspaceId,
        external_id: Option<&str>,
        active: bool,
    ) -> Result<ScimUser, StoreError>;
    /// The member's SCIM link, or `None` if the member isn't SCIM-provisioned.
    async fn get_scim_user(&self, member_id: MemberId) -> Result<Option<ScimUser>, StoreError>;
    /// Every SCIM-provisioned user in a workspace — the SCIM list.
    async fn list_scim_users(&self, workspace_id: WorkspaceId)
        -> Result<Vec<ScimUser>, StoreError>;
    /// Update a SCIM link's `external_id` + `active` and bump `updated_at`.
    /// `None` when the member has no SCIM link.
    async fn update_scim_user(
        &self,
        member_id: MemberId,
        external_id: Option<&str>,
        active: bool,
    ) -> Result<Option<ScimUser>, StoreError>;
    /// Remove a member's SCIM link — `true` when one existed.
    async fn delete_scim_user(&self, member_id: MemberId) -> Result<bool, StoreError>;
    /// A SCIM group and its members, or `None` when the workspace has no such
    /// group.
    async fn get_scim_group(
        &self,
        workspace_id: WorkspaceId,
        id: ScimGroupId,
    ) -> Result<Option<ScimGroup>, StoreError>;
    /// Every SCIM group in a workspace with its members, oldest first.
    async fn list_scim_groups(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<ScimGroup>, StoreError>;
}

#[async_trait]
pub trait SkillStore: Send + Sync {
    /// Capability registry: the free-form skill tags a member declares. `add`
    /// is idempotent and rejects an empty skill; `remove` returns `true` when a
    /// row was deleted; `list` is ordered by skill. Skill routing reads these.
    /// No worker/routes yet — a zero-blast-radius foundation.
    async fn add_member_skill(&self, member_id: MemberId, skill: &str) -> Result<(), StoreError>;
    async fn remove_member_skill(
        &self,
        member_id: MemberId,
        skill: &str,
    ) -> Result<bool, StoreError>;
    async fn list_member_skills(&self, member_id: MemberId)
        -> Result<Vec<MemberSkill>, StoreError>;

    /// Has `member_id` ever held `thread_id`?
    ///
    /// The durable form of the separation-of-duties question. Both governance
    /// gates used to test the thread's **live** `assignee_id`, which a release
    /// sets to NULL — so doing the work and then releasing made the exclusion
    /// vacuous. This answers "ever", and is never cleared by release or
    /// unassign.
    async fn has_worked_thread(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<bool, StoreError>;

    /// Everyone who has ever held `thread_id`, oldest first.
    async fn list_thread_workers(&self, thread_id: ThreadId) -> Result<Vec<MemberId>, StoreError>;
    /// Skills a task (thread) requires. `add` idempotent + empty- reject;
    /// `remove` conditional; `list` ordered by skill. Skill routing:
    /// `claim_next` only takes a task whose required skills the claimer holds.
    async fn add_thread_required_skill(
        &self,
        thread_id: ThreadId,
        skill: &str,
    ) -> Result<(), StoreError>;
    async fn remove_thread_required_skill(
        &self,
        thread_id: ThreadId,
        skill: &str,
    ) -> Result<bool, StoreError>;
    async fn list_thread_required_skills(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadRequiredSkill>, StoreError>;
}

#[async_trait]
pub trait ThreadResultStore: Send + Sync {
    /// A task's structured result: `set` upserts (a re-set overwrites), `get`
    /// returns `None` until one is produced.
    async fn set_thread_result(
        &self,
        thread_id: ThreadId,
        produced_by: MemberId,
        result: &serde_json::Value,
    ) -> Result<ThreadResult, StoreError>;
    async fn get_thread_result(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadResult>, StoreError>;
    /// Workspace-scoped list of thread results. When `result_kind` is `Some`,
    /// exact-match on the namespaced string extracted from the payload (e.g.
    /// `example.review.result/1`) — not a closed enum. `None` (or empty /
    /// whitespace) returns every non-tombstoned result in the workspace. Newest
    /// first. `limit` is clamped `1..=500`.
    async fn list_thread_results(
        &self,
        workspace_id: WorkspaceId,
        result_kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ThreadResult>, StoreError>;
    /// Closed/archived, non-tombstoned thread results in `channel_id`, newest
    /// first. `exclude_thread_id` drops the claimer's own thread so the pack
    /// lists *other* in-channel decisions. `limit` is clamped `1..=50`. The
    /// store does **not** interpret `result_kind` — that is a namespaced string
    /// the pack assembler reads, not a closed enum.
    async fn list_channel_closed_results(
        &self,
        channel_id: ChannelId,
        exclude_thread_id: Option<ThreadId>,
        limit: i64,
    ) -> Result<Vec<ChannelClosedResult>, StoreError>;
}

#[async_trait]
pub trait ThreadSteerStore: Send + Sync {
    /// A thread's persisted steer: `set` upserts (latest wins), `get` returns
    /// `None` until one is set. A durable instruction that survives
    /// claims/handoffs — distinct from a handoff note.
    async fn set_thread_steer(
        &self,
        thread_id: ThreadId,
        steered_by: MemberId,
        steer: &str,
    ) -> Result<ThreadSteer, StoreError>;
    async fn get_thread_steer(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadSteer>, StoreError>;
}

#[async_trait]
pub trait ThreadLineageStore: Send + Sync {
    /// Home a producer's `run_id` on a thread as `parent_run_id`. Upserts; a
    /// re-set overwrites. The value is the producer's string after
    /// [`maidan_types::normalize_parent_run_id`] — Maidan does not mint an id.
    /// Empty / whitespace / over-long → `InvalidInput`.
    async fn set_thread_lineage(
        &self,
        thread_id: ThreadId,
        parent_run_id: &str,
    ) -> Result<ThreadLineage, StoreError>;
    /// A thread's lineage, or `None` until one is set.
    async fn get_thread_lineage(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadLineage>, StoreError>;
    /// Drop a thread's lineage. `true` when a row existed.
    async fn clear_thread_lineage(&self, thread_id: ThreadId) -> Result<bool, StoreError>;
    /// Non-tombstoned threads in `workspace_id` that share `parent_run_id`,
    /// oldest first. Nested children that were given the same producer value
    /// are included; F7 mute is not consulted.
    async fn list_threads_for_run(
        &self,
        workspace_id: WorkspaceId,
        parent_run_id: &str,
    ) -> Result<Vec<Thread>, StoreError>;
    /// Nested occupancy for a producer run: the two-clocks partition of every
    /// **open** thread in the workspace that shares `parent_run_id`. Empty /
    /// unknown run → zeros, not an error. F7 mute stays orthogonal (a muted
    /// nested thread still counts).
    async fn run_occupancy(
        &self,
        workspace_id: WorkspaceId,
        parent_run_id: &str,
    ) -> Result<RunOccupancy, StoreError>;
}

#[async_trait]
pub trait BudgetStore: Send + Sync {
    /// Set (upsert) a thread's budget maxima. Accumulated usage is preserved —
    /// only the `max_*` dimensions are touched.
    async fn set_thread_budget(
        &self,
        thread_id: ThreadId,
        limits: BudgetLimits,
    ) -> Result<ThreadBudget, StoreError>;

    /// Change only the dimensions a patch names.
    ///
    /// [`Self::set_thread_budget`] replaces the whole envelope, so raising one
    /// cap through it cleared the others — and a cleared cap is a run that
    /// should have been stopped and was not. This applies the patch to what is
    /// stored, inside one transaction, so two orchestrators adjusting different
    /// dimensions cannot clobber each other the way a read-modify-write would.
    ///
    /// Accumulated usage is untouched, as with a full set.
    async fn patch_thread_budget(
        &self,
        thread_id: ThreadId,
        patch: BudgetPatch,
    ) -> Result<ThreadBudget, StoreError>;
    /// A thread's budget, or `None` until one is set / usage is first reported.
    async fn get_thread_budget(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadBudget>, StoreError>;
    /// Accumulate reported usage onto a thread's budget, creating the row (no
    /// maxima) when the thread has none yet. Returns the new totals.
    /// Enforcement (stop-the-run on exceed) is a route/tool concern layered on
    /// top.
    async fn add_thread_usage(
        &self,
        thread_id: ThreadId,
        delta: UsageDelta,
    ) -> Result<ThreadBudget, StoreError>;
    /// Report usage and enforce the budget — the "stop the run" path.
    /// Accumulates `delta`; if the thread is now over budget AND has an active
    /// claim, atomically releases the claim, appends a `ClaimFailed` event, and
    /// records a DLQ entry. Returns the new totals + whether the run stopped,
    /// plus the `ClaimFailed` event to publish (`None` when not stopped).
    async fn report_thread_usage(
        &self,
        thread_id: ThreadId,
        delta: UsageDelta,
    ) -> Result<(UsageReport, Option<StoredEvent>), StoreError>;
    /// Record a dead-lettered agent run — a run stopped for exceeding its
    /// budget. `id`/`failed_at` are assigned by the store.
    async fn record_dlq_entry(&self, new: &NewDlqEntry) -> Result<DlqEntry, StoreError>;
    /// A channel's dead-lettered runs, newest first.
    async fn list_channel_dlq(
        &self,
        channel_id: ChannelId,
        limit: i64,
    ) -> Result<Vec<DlqEntry>, StoreError>;
}

#[async_trait]
pub trait UsageLedgerStore: Send + Sync {
    /// Accept one claim-fenced usage heartbeat. Exact retries return the
    /// original ledger outcome with no events; conflicting retries fail.
    async fn report_accounted_usage(
        &self,
        new: &NewUsageLedgerEntry,
    ) -> Result<(UsageLedgerEntry, Vec<StoredEvent>), StoreError>;
    /// One completed idempotent report, or `None` when the id is unknown.
    async fn get_usage_ledger_entry(
        &self,
        usage_report_id: uuid::Uuid,
    ) -> Result<Option<UsageLedgerEntry>, StoreError>;
    /// Completed reports for a thread, newest first.
    async fn list_thread_usage_ledger(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<UsageLedgerEntry>, StoreError>;
    /// Spend, cache shape, and cost per completed task. A thread or member
    /// outside the named workspace is `NotFound`.
    async fn usage_rollup(&self, query: UsageRollupQuery) -> Result<UsageRollup, StoreError>;
}

#[async_trait]
pub trait ApprovalGateStore: Send + Sync {
    /// A durable human-approval gate: `create` opens a `Pending` gate,
    /// `resolve` compare-and-sets it to accept/decline/cancel (a second answer
    /// is a no-op → `None`), `list_pending` is the queryable outstanding-gate
    /// list. No worker/routes yet — a zero-blast-radius foundation.
    async fn create_approval_gate(
        &self,
        gate: &NewApprovalGate,
    ) -> Result<ApprovalGate, StoreError>;
    /// Open a gate and append its `ApprovalRequested` event atomically.
    async fn create_approval_gate_with_event(
        &self,
        gate: &NewApprovalGate,
    ) -> Result<(ApprovalGate, StoredEvent), StoreError>;
    async fn get_approval_gate(
        &self,
        id: ApprovalGateId,
    ) -> Result<Option<ApprovalGate>, StoreError>;
    async fn list_pending_approval_gates(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<ApprovalGate>, StoreError>;
    /// A keyset page of the pending gates, newest first (see
    /// [`PendingGateQuery`]).
    async fn page_pending_approval_gates(
        &self,
        workspace_id: WorkspaceId,
        query: PendingGateQuery,
    ) -> Result<Vec<ApprovalGate>, StoreError>;
    /// How many pending gates match `query` (its `before` and `limit` are
    /// ignored), `readable_by` included.
    async fn count_pending_approval_gates(
        &self,
        workspace_id: WorkspaceId,
        query: PendingGateQuery,
    ) -> Result<i64, StoreError>;
    async fn resolve_approval_gate(
        &self,
        id: ApprovalGateId,
        resolved_by: MemberId,
        state: ApprovalGateState,
        content: Option<&serde_json::Value>,
    ) -> Result<Option<ApprovalGate>, StoreError>;
}

#[async_trait]
pub trait GlossaryStore: Send + Sync {
    /// A workspace's shared glossary: `set` upserts a canonical `term ->
    /// definition` (+ aliases) keyed on `(workspace_id, term)`, `get` fetches
    /// one, `list` returns all (ordered by term), `delete` removes one. The
    /// anti-drift pin — the target of the `defines` reference relation. No
    /// routes/tools yet — a zero-blast-radius foundation.
    async fn set_glossary_term(&self, new: NewGlossaryTerm) -> Result<GlossaryTerm, StoreError>;
    async fn get_glossary_term(
        &self,
        workspace_id: WorkspaceId,
        term: &str,
    ) -> Result<Option<GlossaryTerm>, StoreError>;
    async fn list_glossary_terms(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<GlossaryTerm>, StoreError>;
    async fn delete_glossary_term(
        &self,
        workspace_id: WorkspaceId,
        term: &str,
    ) -> Result<bool, StoreError>;
}

#[async_trait]
pub trait NotificationStore: Send + Sync {
    /// Per-recipient notifications: `create` inserts one row for a recipient,
    /// `list_for_member` returns newest-first (optionally unread-only),
    /// `mark_read` stamps `read_at` (idempotent), `unread_count` is the badge,
    /// `mark_all_read` clears the badge. No router/routes/worker yet — a
    /// zero-blast-radius foundation.
    async fn create_notification(&self, new: NewNotification) -> Result<Notification, StoreError>;
    /// Insert a notification unless one already exists for `(member_id,
    /// source_log_id)` — the router's idempotent write across event replays and
    /// server replicas. `None` = a row already existed (deduped).
    async fn create_notification_if_absent(
        &self,
        new: NewNotification,
    ) -> Result<Option<Notification>, StoreError>;
    /// Insert many per-recipient notifications in one round trip — the batch
    /// form of
    /// [`create_notification_if_absent`](Self::create_notification_if_absent)
    /// for the `MessagePosted` fan-out. Each row is idempotent on `(member_id,
    /// source_log_id)` (`ON CONFLICT DO NOTHING`); the returned vec is the
    /// subset that was actually inserted (deduped rows are omitted), so the
    /// caller meters
    /// + emails exactly the new notifications. Empty input returns empty with no
    /// query. Postgres inserts via `UNNEST` arrays; SQLite chunks a multi-row
    /// `VALUES` under the 999-parameter limit.
    async fn create_notifications_batch(
        &self,
        rows: &[NewNotification],
    ) -> Result<Vec<Notification>, StoreError>;
    async fn list_notifications(
        &self,
        member_id: MemberId,
        unread_only: bool,
        limit: i64,
    ) -> Result<Vec<Notification>, StoreError>;
    /// Mark one notification read, scoped to its recipient — a member can only
    /// mark their own. `false` when no `(member_id, id)` row exists.
    async fn mark_notification_read(
        &self,
        member_id: MemberId,
        id: NotificationId,
    ) -> Result<bool, StoreError>;
    /// Snooze one notification until `until` — recipient-scoped; it drops out
    /// of the default inbox + unread count until the snooze lapses. Returns
    /// whether the `(member_id, id)` row exists.
    async fn snooze_notification(
        &self,
        member_id: MemberId,
        id: NotificationId,
        until: DateTime<Utc>,
    ) -> Result<bool, StoreError>;
    async fn mark_all_notifications_read(&self, member_id: MemberId) -> Result<u64, StoreError>;
    async fn unread_notification_count(&self, member_id: MemberId) -> Result<i64, StoreError>;

    /// Per-member notification preferences: `set` upserts a mute flag for one
    /// event kind, `list` returns a member's prefs, `is_muted` answers the
    /// router's "should I suppress this?" (absent row = not muted). No router
    /// change or routes yet — a zero-blast-radius foundation.
    async fn set_notification_pref(
        &self,
        member_id: MemberId,
        kind: EventKind,
        muted: bool,
    ) -> Result<NotificationPref, StoreError>;
    async fn list_notification_prefs(
        &self,
        member_id: MemberId,
    ) -> Result<Vec<NotificationPref>, StoreError>;
    async fn is_notification_muted(
        &self,
        member_id: MemberId,
        kind: EventKind,
    ) -> Result<bool, StoreError>;

    /// Which of `members` have muted `kind` — the batch form of
    /// [`is_notification_muted`](Self::is_notification_muted), so a fan-out
    /// checks mutes in one query instead of one per recipient.
    async fn filter_muted_members(
        &self,
        kind: EventKind,
        members: &[MemberId],
    ) -> Result<Vec<MemberId>, StoreError>;

    /// Register (upsert) a member's Web Push subscription. Keyed on
    /// `(member_id, endpoint)` — re-subscribing the same device refreshes its
    /// keys.
    async fn add_push_subscription(
        &self,
        new: NewPushSubscription,
    ) -> Result<PushSubscription, StoreError>;
    /// A member's Web Push subscriptions — the router's delivery targets when
    /// the member has no live WebSocket.
    async fn list_push_subscriptions(
        &self,
        member_id: MemberId,
    ) -> Result<Vec<PushSubscription>, StoreError>;
    /// Remove one of a member's push subscriptions — recipient-scoped
    /// (`member_id` + `id`); `true` when a row was removed.
    async fn delete_push_subscription(
        &self,
        member_id: MemberId,
        id: PushSubscriptionId,
    ) -> Result<bool, StoreError>;

    /// Queue one failed web push for a later retry. The row is due at
    /// `next_attempt_at` and already counts `attempts` tries.
    async fn enqueue_web_push(&self, new: NewWebPushOutbox) -> Result<WebPushOutboxId, StoreError>;
    /// Lease the oldest due pending web push, bumping `attempts` and pushing
    /// `next_attempt_at` forward by `lease_secs` so a crashed worker retries it.
    async fn claim_next_due_web_push(
        &self,
        now: DateTime<Utc>,
        lease_secs: i64,
    ) -> Result<Option<WebPushOutbox>, StoreError>;
    async fn mark_web_push_delivered(&self, id: WebPushOutboxId) -> Result<(), StoreError>;
    /// Reschedule (`retry_at = Some`) or dead-letter (`None`).
    async fn mark_web_push_failed(
        &self,
        id: WebPushOutboxId,
        error: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), StoreError>;
    /// Hand a claimed push back unsent, due again at `until`, and give the
    /// claim attempt back. Used when the retry budget holds the send.
    async fn defer_web_push(
        &self,
        id: WebPushOutboxId,
        until: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}

#[async_trait]
pub trait FollowStore: Send + Sync {
    /// Subscription / follows: a member follows a channel, thread, or member to be
    /// notified of activity there even without a mention. `follow_*` is
    /// idempotent; `unfollow_*` returns `true` when a row was removed; `list_*`
    /// is a member's follows; `*_followers` is the router's fan-out set. No
    /// router change or routes yet — a zero-blast-radius foundation.
    async fn follow_channel(
        &self,
        member_id: MemberId,
        channel_id: ChannelId,
    ) -> Result<(), StoreError>;
    async fn unfollow_channel(
        &self,
        member_id: MemberId,
        channel_id: ChannelId,
    ) -> Result<bool, StoreError>;
    async fn list_channel_follows(
        &self,
        member_id: MemberId,
    ) -> Result<Vec<ChannelFollow>, StoreError>;
    async fn channel_followers(&self, channel_id: ChannelId) -> Result<Vec<MemberId>, StoreError>;
    async fn follow_thread(
        &self,
        member_id: MemberId,
        thread_id: ThreadId,
    ) -> Result<(), StoreError>;
    async fn unfollow_thread(
        &self,
        member_id: MemberId,
        thread_id: ThreadId,
    ) -> Result<bool, StoreError>;
    async fn list_thread_follows(
        &self,
        member_id: MemberId,
    ) -> Result<Vec<ThreadFollow>, StoreError>;
    async fn thread_followers(&self, thread_id: ThreadId) -> Result<Vec<MemberId>, StoreError>;
    async fn follow_member(
        &self,
        follower_id: MemberId,
        followed_id: MemberId,
    ) -> Result<(), StoreError>;
    async fn unfollow_member(
        &self,
        follower_id: MemberId,
        followed_id: MemberId,
    ) -> Result<bool, StoreError>;
    async fn list_member_follows(
        &self,
        follower_id: MemberId,
    ) -> Result<Vec<MemberFollow>, StoreError>;
    async fn member_followers(&self, followed_id: MemberId) -> Result<Vec<MemberId>, StoreError>;

    /// Mute a specific thread for a member. Idempotent; the notification router
    /// suppresses notifications about a muted thread.
    async fn mute_thread(&self, member_id: MemberId, thread_id: ThreadId)
        -> Result<(), StoreError>;
    /// Unmute a thread. `true` if it was muted.
    async fn unmute_thread(
        &self,
        member_id: MemberId,
        thread_id: ThreadId,
    ) -> Result<bool, StoreError>;
    /// Whether `member_id` has muted `thread_id`.
    async fn is_thread_muted(
        &self,
        member_id: MemberId,
        thread_id: ThreadId,
    ) -> Result<bool, StoreError>;
    /// Members who have muted `thread_id` — the router subtracts them from a
    /// `MessagePosted` fan-out in one batch query.
    async fn thread_muters(&self, thread_id: ThreadId) -> Result<Vec<MemberId>, StoreError>;
    /// Mute a whole channel for a member. Idempotent; the router suppresses the
    /// channel's firehose, but a mention breaks through.
    async fn mute_channel(
        &self,
        member_id: MemberId,
        channel_id: ChannelId,
    ) -> Result<(), StoreError>;
    /// Unmute a channel. `true` if it was muted.
    async fn unmute_channel(
        &self,
        member_id: MemberId,
        channel_id: ChannelId,
    ) -> Result<bool, StoreError>;
    /// Whether `member_id` has muted `channel_id`.
    async fn is_channel_muted(
        &self,
        member_id: MemberId,
        channel_id: ChannelId,
    ) -> Result<bool, StoreError>;
    /// Members who have muted `channel_id` — the router subtracts them from a
    /// `MessagePosted` fan-out in one batch query.
    async fn channel_muters(&self, channel_id: ChannelId) -> Result<Vec<MemberId>, StoreError>;
}

#[async_trait]
pub trait MailStore: Send + Sync {
    /// A member's delivery email address: where email notifications go. `set`
    /// upserts, `get` returns `None` when unset, `delete` removes it. A
    /// separate table, so the shared member row-mapping is untouched. No
    /// delivery wiring yet — a zero-blast-radius foundation.
    async fn set_member_email(
        &self,
        member_id: MemberId,
        email: &str,
    ) -> Result<MemberEmail, StoreError>;
    async fn get_member_email(
        &self,
        member_id: MemberId,
    ) -> Result<Option<MemberEmail>, StoreError>;
    async fn delete_member_email(&self, member_id: MemberId) -> Result<bool, StoreError>;

    /// Durable mail outbox: notification emails are enqueued and delivered by a
    /// retry/backoff worker instead of a best-effort send. `enqueue_mail`
    /// queues one; `claim_next_due_mail` atomically leases the oldest due
    /// `pending` row (bumps `attempts`, pushes `next_attempt_at` forward by
    /// `lease_secs` so a crashed worker's row is retried);
    /// `mark_mail_delivered` finishes it; `mark_mail_failed` reschedules
    /// (`retry_at = Some`) or dead-letters (`None`); `count_dead_mail` is the
    /// DLQ depth. `enqueue_mail` returns `None`, queueing nothing, when the
    /// mail is about a message that has been withdrawn (its content key is
    /// shredded); see [`NewMailOutbox::source_log_id`].
    async fn enqueue_mail(&self, new: NewMailOutbox) -> Result<Option<MailOutboxId>, StoreError>;
    async fn claim_next_due_mail(
        &self,
        now: DateTime<Utc>,
        lease_secs: i64,
    ) -> Result<Option<MailOutbox>, StoreError>;
    async fn mark_mail_delivered(&self, id: MailOutboxId) -> Result<(), StoreError>;
    async fn mark_mail_failed(
        &self,
        id: MailOutboxId,
        error: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), StoreError>;
    /// Hand a claimed entry back unsent, due again at `until`: the claim's
    /// attempt is given back, because the retry budget held the send back and
    /// nothing failed. A deferral never moves an entry toward the DLQ.
    async fn defer_mail(&self, id: MailOutboxId, until: DateTime<Utc>) -> Result<(), StoreError>;
    async fn count_dead_mail(&self) -> Result<i64, StoreError>;
    /// Dead-lettered entries for the operator DLQ view, newest first.
    /// Dead-lettered mail for the operator DLQ. `scope` is the caller's
    /// workspace; `None` is the `operator:global` view and the only way to see
    /// rows with a `NULL` workspace (pre-398.3, or tenant-less mail). The rows
    /// carry recipient addresses, subjects and bodies, so an unscoped query
    /// behind the per-workspace `token:admin` was a cross-tenant read.
    async fn list_dead_mail(
        &self,
        scope: Option<WorkspaceId>,
        limit: i64,
    ) -> Result<Vec<DeadMail>, StoreError>;
    /// Requeue a dead entry (`pending`, due now, `attempts` reset); returns whether
    /// a dead row was actually requeued.
    /// Requeue a dead entry, scoped like [`Self::list_dead_mail`] — another
    /// tenant's id is a no-op, not a re-send of their mail.
    async fn requeue_dead_mail(
        &self,
        scope: Option<WorkspaceId>,
        id: MailOutboxId,
    ) -> Result<bool, StoreError>;
}

#[async_trait]
pub trait EgressStore: Send + Sync {
    /// Durable projector egress: a message bound for an external surface is
    /// enqueued and delivered by a retry/backoff worker, instead of the
    /// projectors' best-effort inline post where a transient failure dropped
    /// it.
    ///
    /// `enqueue_egress` queues one and returns `None` when `(source_log_id,
    /// target)` is already queued — the router that enqueues runs on every
    /// replica, so the dedup is what makes N replicas send once (the lesson).
    /// `claim_next_due_egress` atomically leases the oldest due `pending` row
    /// (bumps `attempts`, pushes `next_attempt_at` forward by `lease_secs` so a
    /// crashed worker's row is retried); `mark_egress_delivered` finishes it;
    /// `mark_egress_failed` reschedules (`retry_at = Some`) or dead-letters
    /// (`None`); `count_dead_egress` is the DLQ depth. No worker/wiring yet — a
    /// zero-blast-radius foundation.
    async fn enqueue_egress(
        &self,
        new: NewEgressOutbox,
    ) -> Result<Option<EgressOutboxId>, StoreError>;
    async fn claim_next_due_egress(
        &self,
        now: DateTime<Utc>,
        lease_secs: i64,
    ) -> Result<Option<EgressOutbox>, StoreError>;
    async fn mark_egress_delivered(&self, id: EgressOutboxId) -> Result<(), StoreError>;
    async fn mark_egress_failed(
        &self,
        id: EgressOutboxId,
        error: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), StoreError>;
    /// Hand a claimed delivery back unsent, due again at `until`: the claim's
    /// attempt is given back, because the retry budget held the post back and
    /// nothing failed. A deferral never moves a delivery toward the DLQ.
    async fn defer_egress(
        &self,
        id: EgressOutboxId,
        until: DateTime<Utc>,
    ) -> Result<(), StoreError>;
    async fn count_dead_egress(&self) -> Result<i64, StoreError>;
    /// Dead-lettered deliveries for the operator DLQ view, newest first —
    /// **scoped to one workspace**.
    ///
    /// The scope is a parameter rather than a route-level filter because the
    /// rows name other tenants' Slack channel ids and GitHub repositories, and
    /// `token:admin` is per-workspace. A global query behind a per-workspace
    /// capability is a cross-tenant read.
    async fn list_dead_egress(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<DeadEgress>, StoreError>;
    /// Requeue a dead delivery (`pending`, due now, `attempts` reset); returns
    /// whether a dead row was actually requeued. Scoped like the list, so a
    /// guessed id from another tenant is a no-op rather than a re-send into
    /// their channel.
    async fn requeue_dead_egress(
        &self,
        workspace_id: WorkspaceId,
        id: EgressOutboxId,
    ) -> Result<bool, StoreError>;

    /// The egress trust boundary: a per-workspace allowlist of the destinations
    /// Maidan may deliver to. A result's `deliver_to` is written by an
    /// *agent*, while the connector credentials are operator-held and reach many
    /// repositories and channels — so `deliver_to` **selects** and this
    /// allowlist
    /// **authorizes**, and an empty allowlist authorizes nothing (the
    /// secret-broker fail-safe).
    ///
    /// `allow_egress_target` is idempotent (re-blessing keeps the original
    /// entry) and rejects a selector that is not an id — a Slack `#name` or a
    /// GitHub `owner/name#123` — because an allowlist keyed on a mutable name
    /// is not an allowlist and the authorization grain on GitHub is the
    /// *repository*. `revoke_egress_target` is workspace-scoped so one admin
    /// cannot revoke another workspace's entry by guessing an id.
    /// `is_egress_target_allowed` takes the allowlist grain, i.e.
    /// `EgressTarget::allowlist_selector()`.
    async fn allow_egress_target(
        &self,
        new: NewEgressTarget,
    ) -> Result<AllowedEgressTarget, StoreError>;
    async fn list_egress_targets(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<AllowedEgressTarget>, StoreError>;
    async fn revoke_egress_target(
        &self,
        workspace_id: WorkspaceId,
        id: EgressTargetId,
    ) -> Result<bool, StoreError>;
    async fn is_egress_target_allowed(
        &self,
        workspace_id: WorkspaceId,
        surface: EgressSurface,
        selector: &str,
    ) -> Result<bool, StoreError>;

    /// Result-delivery state — one row per `(thread, target)`. Distinct from
    /// the egress outbox, which is transport: this is *intent and identity*,
    /// and it is what makes an always-on every-replica router safe.
    ///
    /// `arm_result_delivery` is the contended write: it returns the row only
    /// when
    /// *this* caller won the right to deliver `revision`. `None` means another
    /// replica already armed this exact revision, or the row has seen one at
    /// least as new — which is the dedup (the lesson). A won row keeps its
    /// `external_ref`, so a re-review edits the object the first delivery
    /// created instead of leaving a second comment.
    ///
    /// `mark_result_delivered` records the handle to edit next time and the
    /// revision that actually landed; `mark_result_delivery_failed` leaves both
    /// alone, because whatever was delivered before is still out there and
    /// still editable. `mark_result_delivery_skipped` records a target we
    /// deliberately did not deliver to — an unknown surface, or one the
    /// workspace has not blessed — which is a **normal outcome**, not an error,
    /// and is recorded so the producer can read it rather than being dropped.
    async fn arm_result_delivery(
        &self,
        thread_id: ThreadId,
        target: &EgressTarget,
        revision: DateTime<Utc>,
    ) -> Result<Option<ResultDelivery>, StoreError>;
    /// Arm a skip whose destination this build cannot form an [`EgressTarget`]
    /// for — an unknown `surface`, or a known one with unusable detail (a Slack
    /// `#name`, a repo with no owner). The skip still has to be a row, because
    /// "we skipped your target" and "we lost it" are different answers; the
    /// producer reads it back from `list_result_deliveries`. Same contended
    /// write as [`Self::arm_result_delivery`]: `None` is the dedup.
    async fn arm_unroutable_result_delivery(
        &self,
        thread_id: ThreadId,
        surface: &str,
        selector: &str,
        revision: DateTime<Utc>,
    ) -> Result<Option<ResultDelivery>, StoreError>;
    async fn mark_result_delivered(
        &self,
        id: ResultDeliveryId,
        external_ref: Option<&str>,
        revision: DateTime<Utc>,
    ) -> Result<(), StoreError>;
    async fn mark_result_delivery_failed(
        &self,
        id: ResultDeliveryId,
        error: &str,
    ) -> Result<(), StoreError>;
    async fn mark_result_delivery_skipped(
        &self,
        id: ResultDeliveryId,
        reason: &str,
    ) -> Result<(), StoreError>;
    async fn get_result_delivery(
        &self,
        thread_id: ThreadId,
        target: &EgressTarget,
    ) -> Result<Option<ResultDelivery>, StoreError>;
    async fn list_result_deliveries(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ResultDelivery>, StoreError>;
    /// The delivery-status API's point read: this id on this thread, or `None`.
    /// Thread-scoped so a guessed UUID cannot read another thread's row.
    async fn get_result_delivery_by_id(
        &self,
        thread_id: ThreadId,
        id: ResultDeliveryId,
    ) -> Result<Option<ResultDelivery>, StoreError>;
    /// Operator replay: reopen as `pending` and clear `last_error` without
    /// touching `armed_revision` or `external_ref`. Arming is "is this a new
    /// result?"; replay is "try this result again". `None` if `(thread_id, id)`
    /// does not exist.
    async fn prepare_result_delivery_replay(
        &self,
        thread_id: ThreadId,
        id: ResultDeliveryId,
    ) -> Result<Option<ResultDelivery>, StoreError>;
}

#[async_trait]
pub trait ProjectorLinkStore: Send + Sync {
    /// Slack projector channel links: map a Slack channel to the Maidan
    /// channel/thread it projects into, and the member inbound messages post
    /// as. `link` upserts (one per Slack channel); `get` resolves the ingress
    /// target; `list` is the workspace's links; `unlink` removes one.
    async fn link_slack_channel(
        &self,
        new: NewSlackChannelLink,
    ) -> Result<SlackChannelLink, StoreError>;
    async fn get_slack_channel_link(
        &self,
        slack_channel_id: &str,
    ) -> Result<Option<SlackChannelLink>, StoreError>;
    /// Resolve the Slack link for a Maidan thread — the egress reverse lookup.
    async fn get_slack_channel_link_by_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<SlackChannelLink>, StoreError>;
    async fn list_slack_channel_links(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SlackChannelLink>, StoreError>;
    async fn unlink_slack_channel(&self, slack_channel_id: &str) -> Result<bool, StoreError>;
    /// Turn egress to a Slack channel off after an auth/config-class failure —
    /// a retry can't fix a revoked token or a deleted channel. Idempotent (an
    /// already-disabled link keeps its original timestamp); returns whether
    /// this call did the disabling, so only the first failure announces it.
    /// Re-linking clears the flag; ingress is unaffected.
    async fn disable_slack_channel_link(&self, slack_channel_id: &str) -> Result<bool, StoreError>;

    /// GitHub projector issue/PR links: map a GitHub issue/PR (`repo`,
    /// `issue_number`) to the Maidan channel/thread it projects into. `link`
    /// upserts; `get` resolves ingress; `get_by_thread` is the egress reverse
    /// lookup; `list` is the workspace's links; `unlink` removes one.
    async fn link_github_issue(
        &self,
        new: NewGithubIssueLink,
    ) -> Result<GithubIssueLink, StoreError>;
    async fn get_github_issue_link(
        &self,
        repo: &str,
        issue_number: i64,
    ) -> Result<Option<GithubIssueLink>, StoreError>;
    async fn get_github_issue_link_by_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<GithubIssueLink>, StoreError>;
    async fn list_github_issue_links(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<GithubIssueLink>, StoreError>;
    async fn unlink_github_issue(&self, repo: &str, issue_number: i64) -> Result<bool, StoreError>;
    /// The GitHub twin of
    /// [`disable_slack_channel_link`](Self::disable_slack_channel_link).
    async fn disable_github_issue_link(
        &self,
        repo: &str,
        issue_number: i64,
    ) -> Result<bool, StoreError>;
}

#[async_trait]
pub trait PresenceDigestStore: Send + Sync {
    /// Durable per-member last-seen: `touch` upserts `now()` (called on
    /// presence registration), `get` returns the instant or `None`. A
    /// cross-replica signal for presence-aware email routing. No wiring yet — a
    /// zero-blast-radius foundation.
    async fn touch_member_last_seen(&self, member_id: MemberId) -> Result<(), StoreError>;
    async fn get_member_last_seen(
        &self,
        member_id: MemberId,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, StoreError>;

    /// Email digest data model: a per-member delivery mode (`set`/`get`,
    /// default `Immediate` when unset), a digest watermark
    /// (`set_last_digest_at`, advanced after a digest is sent), and the
    /// sweeper's enumeration (`members_due_for_digest` — digest-mode members
    /// with an address and unread notifications since their last digest). No
    /// worker/routes yet — a zero-blast-radius foundation.
    async fn set_delivery_mode(
        &self,
        member_id: MemberId,
        mode: EmailDeliveryMode,
    ) -> Result<(), StoreError>;
    async fn get_delivery_mode(&self, member_id: MemberId)
        -> Result<EmailDeliveryMode, StoreError>;
    async fn set_last_digest_at(
        &self,
        member_id: MemberId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError>;
    async fn members_due_for_digest(&self, limit: i64) -> Result<Vec<DigestDue>, StoreError>;
    /// Task results ("decisions") produced by someone else after `since`, in a
    /// channel or thread the member follows — the "buried decisions" the digest
    /// surfaces, and a queryable read. Newest first.
    async fn buried_decisions_for_member(
        &self,
        member_id: MemberId,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<BuriedDecision>, StoreError>;
    /// Compose unread lifecycle notification rows after `since` into
    /// per-channel result/gate/stuck counts. This deliberately reads the
    /// notification delivery layer rather than an analytics projection.
    async fn manager_digest_for_member(
        &self,
        member_id: MemberId,
        since: DateTime<Utc>,
    ) -> Result<ManagerDigest, StoreError>;
}

#[async_trait]
pub trait SessionStore: Send + Sync {
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

    // D-A: a browser session is a credential, so signing in and signing out
    // write their audit row in the session write's transaction. Request
    // handlers use these; `authority_changes_are_audited_in_their_transaction`
    // fails if one calls the unaudited form above.

    /// [`Self::create_session`] with its audit row in the same transaction.
    async fn create_session_audited(
        &self,
        new: NewMaidanSession,
        audit: crate::AuditFor<MaidanSession>,
    ) -> Result<MaidanSession, StoreError>;
    /// Delete a session with its audit row in the same transaction, returning
    /// what was deleted. `NotFound`, with nothing written, if it is gone.
    async fn delete_session_audited(
        &self,
        id: SessionId,
        audit: crate::AuditFor<MaidanSession>,
    ) -> Result<MaidanSession, StoreError>;
    /// Delete a session if, and only if, it has expired: housekeeping for a
    /// credential that already grants nothing, so it is not audited.
    async fn delete_expired_session(&self, id: SessionId) -> Result<(), StoreError>;
}

#[async_trait]
pub trait ChannelStore: Send + Sync {
    async fn create_channel(&self, new: NewChannel) -> Result<Channel, StoreError>;

    /// Insert a channel and append its `ChannelCreated` event **atomically** in
    /// one transaction. Returns the channel and the durable event; the caller
    /// notifies the bus with the returned event.
    async fn create_channel_with_event(
        &self,
        new: NewChannel,
    ) -> Result<(Channel, StoredEvent), StoreError>;
    async fn get_channel(&self, id: ChannelId) -> Result<Channel, StoreError>;
    async fn list_channels(&self, workspace_id: WorkspaceId) -> Result<Vec<Channel>, StoreError>;

    /// Per-channel membership. Public channels are open to the workspace and
    /// need no rows; these gate private channels. `add` is an idempotent upsert
    /// of the role.
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
}

#[async_trait]
pub trait DmStore: Send + Sync {
    /// `NotFound` unless both members belong to `workspace_id`.
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
}

#[async_trait]
pub trait ThreadStore: Send + Sync {
    async fn create_thread(&self, new: NewThread) -> Result<Thread, StoreError>;

    /// Insert a thread and append its `ThreadCreated` event **atomically** in
    /// one transaction.
    async fn create_thread_with_event(
        &self,
        new: NewThread,
    ) -> Result<(Thread, StoredEvent), StoreError>;
    async fn get_thread(&self, id: ThreadId) -> Result<Thread, StoreError>;
    async fn list_threads(&self, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError>;

    /// All threads across a workspace's channels, in one query. Avoids the
    /// per-channel `list_threads` N+1 when assembling workspace context.
    /// Ordered by `created_at DESC`.
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

    /// One keyset page of a channel's **live** threads, ordered `(created_at,
    /// id)` ascending. `after` is an exclusive cursor (the prior page's last
    /// thread id); `None` starts from the beginning. The channel-scoped twin of
    /// [`page_threads_for_workspace`](Self::page_threads_for_workspace) —
    /// bounds the previously-unbounded [`list_threads`](Self::list_threads)
    /// (kept for internal full-list callers).
    async fn page_threads_for_channel(
        &self,
        channel_id: ChannelId,
        after: Option<ThreadId>,
        limit: i64,
    ) -> Result<Vec<Thread>, StoreError>;

    /// A parent thread's child threads, collapsed with a message count each.
    /// Oldest first; tombstoned children excluded.
    async fn child_thread_summaries(
        &self,
        parent_id: ThreadId,
    ) -> Result<Vec<ChildThreadSummary>, StoreError>;

    /// A channel's threads ordered by last activity — most-recently bumped
    /// first. A post bumps its thread's `updated_at`, floating it here.
    async fn list_recently_active_threads(
        &self,
        channel_id: ChannelId,
        limit: i64,
    ) -> Result<Vec<Thread>, StoreError>;

    async fn transition_thread(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
        action: maidan_fsm::ThreadAction,
    ) -> Result<ThreadTransitionResult, StoreError>;
    /// Transition a thread's state and append its `ThreadStateChanged` event
    /// atomically.
    async fn transition_thread_with_event(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
        action: maidan_fsm::ThreadAction,
    ) -> Result<(ThreadTransitionResult, StoredEvent), StoreError>;

    async fn list_thread_transitions(
        &self,
        thread_id: ThreadId,
        limit: i64,
    ) -> Result<Vec<ThreadTransition>, StoreError>;

    /// Point-in-time task-queue depth for a channel: counts of its open task
    /// threads partitioned into ready / assigned / blocked, using the same
    /// claimability predicate as `claim_next`. One aggregate query.
    /// `readable_by` counts only the threads that member may read, by
    /// `claim_next`'s read rule (on the `__dm__` channel, only its own DMs);
    /// `None` counts every thread, for a caller that bypasses auth.
    async fn channel_queue_depth(
        &self,
        channel_id: ChannelId,
        readable_by: Option<MemberId>,
    ) -> Result<QueueDepth, StoreError>;

    /// [`channel_queue_depth`](Self::channel_queue_depth) across every channel
    /// of `workspace_id`: the sum of its channels' counts for the same reader.
    /// A reader from another workspace counts nothing.
    async fn workspace_queue_depth(
        &self,
        workspace_id: WorkspaceId,
        readable_by: Option<MemberId>,
    ) -> Result<QueueDepth, StoreError>;

    /// Channel occupancy: the two-clocks refinement of `channel_queue_depth` —
    /// the held threads split into `claimed` (not yet acknowledged) and
    /// `working` (acknowledged), so an orchestrator sees how much held work is
    /// actually underway. One aggregate query. `readable_by` as for
    /// [`channel_queue_depth`](Self::channel_queue_depth).
    async fn channel_occupancy(
        &self,
        channel_id: ChannelId,
        readable_by: Option<MemberId>,
    ) -> Result<ChannelOccupancy, StoreError>;

    /// [`channel_occupancy`](Self::channel_occupancy) across every channel of
    /// `workspace_id`, as [`workspace_queue_depth`](Self::workspace_queue_depth)
    /// is to the channel depth.
    async fn workspace_occupancy(
        &self,
        workspace_id: WorkspaceId,
        readable_by: Option<MemberId>,
    ) -> Result<ChannelOccupancy, StoreError>;

    /// The thread's version: the number of writes to its messages, result,
    /// title and description, and linked artifacts. The database bumps it, so
    /// every write path moves it. 0 before any. `NotFound` for no such thread.
    async fn thread_version(&self, thread_id: ThreadId) -> Result<i64, StoreError>;
    /// Link an artifact the thread's workspace holds to the thread. `NotFound`
    /// when the workspace holds no artifact with that hash. Returns the link
    /// and whether it is new.
    async fn link_thread_artifact(
        &self,
        thread_id: ThreadId,
        sha256: &str,
        linked_by: MemberId,
    ) -> Result<(ThreadArtifact, bool), StoreError>;
    /// Unlink an artifact from a thread; `false` when it was not linked.
    async fn unlink_thread_artifact(
        &self,
        thread_id: ThreadId,
        sha256: &str,
    ) -> Result<bool, StoreError>;
    /// The artifacts linked to a thread, in the order they were linked.
    async fn list_thread_artifacts(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadArtifact>, StoreError>;
    /// The thread's latest review packet: what its current review was handed,
    /// recorded by `start_review`. `None` before any.
    async fn latest_review_packet(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ReviewPacket>, StoreError>;
}

#[async_trait]
pub trait TaskScheduleStore: Send + Sync {
    /// Scheduled / recurring task foundation. CRUD plus the sweeper's due-scan
    /// (`due_task_schedules`). No worker or routes yet — a zero-blast-radius
    /// foundation.
    async fn create_task_schedule(&self, new: NewTaskSchedule) -> Result<TaskSchedule, StoreError>;
    async fn get_task_schedule(&self, id: TaskScheduleId) -> Result<TaskSchedule, StoreError>;
    async fn list_task_schedules(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<TaskSchedule>, StoreError>;
    async fn delete_task_schedule(&self, id: TaskScheduleId) -> Result<bool, StoreError>;
    /// Active schedules whose `next_run_at` has arrived (`<= now`), oldest first,
    /// bounded by `limit` — the sweeper's batch read.
    async fn due_task_schedules(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<TaskSchedule>, StoreError>;
    /// Atomically claim the oldest due active schedule and advance it: a
    /// recurring schedule re-arms `next_run_at = now + interval_secs`
    /// (fire-once-per-tick, no catch-up storm); a one-shot deactivates. Returns
    /// the advanced row, or `None` when nothing is due. Postgres uses `FOR
    /// UPDATE SKIP LOCKED` so concurrent replicas claim distinct schedules;
    /// SQLite serializes writers. The caller creates the task thread *after*
    /// the claim commits (at-most-once on crash — a missed firing, never a
    /// double).
    async fn claim_next_due_schedule(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Option<TaskSchedule>, StoreError>;
    /// Pause (`false`) or resume (`true`) a schedule. `NotFound` if the id
    /// doesn't exist. Resuming does not re-arm `next_run_at`.
    async fn set_task_schedule_active(
        &self,
        id: TaskScheduleId,
        active: bool,
    ) -> Result<TaskSchedule, StoreError>;
}

#[async_trait]
pub trait RecipeStore: Send + Sync {
    /// Recipe blueprint CRUD. A recipe is a reusable thread-type; instantiation
    /// (`instantiate_recipe`) builds a parent + DAG children from it.
    /// Zero-blast-radius foundation — no routes yet.
    async fn create_recipe(&self, new: NewRecipe) -> Result<Recipe, StoreError>;
    async fn get_recipe(&self, id: RecipeId) -> Result<Recipe, StoreError>;
    async fn list_recipes(&self, workspace_id: WorkspaceId) -> Result<Vec<Recipe>, StoreError>;
    async fn delete_recipe(&self, id: RecipeId) -> Result<bool, StoreError>;
    /// Instantiate a recipe: in one transaction, create the parent thread + a
    /// child thread per `spec.children`, wire the readiness DAG (each child
    /// depends on its `depends_on` siblings; the parent depends on every child
    /// so it lands last), attach each child's required skills, and record a
    /// `maidan_recipe_runs` row freezing the recipe bytes + `params`
    /// (copy-on-fire). Returns the run plus every `ThreadCreated` event (the
    /// caller publishes them). `InvalidInput` if `params` fails the spec's
    /// required-param check.
    async fn instantiate_recipe(
        &self,
        recipe_id: RecipeId,
        params: serde_json::Value,
        actor: MemberId,
    ) -> Result<(RecipeRun, Vec<StoredEvent>), StoreError>;
    async fn get_recipe_run(&self, id: RecipeRunId) -> Result<RecipeRun, StoreError>;
    /// The most recent run of a recipe, or `None` if never instantiated — the
    /// dedup basis for the scheduler's `ScheduleSkipped`.
    async fn latest_recipe_run(&self, recipe_id: RecipeId)
        -> Result<Option<RecipeRun>, StoreError>;
}

#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Named-secret store. The value is stored AEAD-encrypted (the route layer
    /// holds the key); `get_secret_ciphertext` is the only read that returns
    /// it, and `list_secrets` returns metadata only. `create_secret` upserts on
    /// `(workspace_id, name)` — re-creating a name rotates its value.
    async fn create_secret(&self, new: NewSecret) -> Result<Secret, StoreError>;
    async fn get_secret_ciphertext(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<Option<String>, StoreError>;
    async fn list_secrets(&self, workspace_id: WorkspaceId) -> Result<Vec<Secret>, StoreError>;
    async fn delete_secret(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<bool, StoreError>;

    /// The hosts a workspace trusts with its secret values: the egress broker
    /// substitutes a `secret://` ref only on a delivery to a host listed here
    /// for the sending workspace, and an empty list substitutes nothing.
    /// `allow_secret_egress_host` is idempotent and refuses a host that is not
    /// a bare lowercase-able name (`normalize_secret_egress_host`);
    /// `revoke_secret_egress_host` is scoped to the workspace. Both reads stay
    /// on the primary: a revoked host must stop receiving values at once, and
    /// a lagging replica would still report it listed.
    async fn allow_secret_egress_host(
        &self,
        new: NewSecretEgressHost,
    ) -> Result<SecretEgressHost, StoreError>;
    async fn list_secret_egress_hosts(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SecretEgressHost>, StoreError>;
    async fn revoke_secret_egress_host(
        &self,
        workspace_id: WorkspaceId,
        host: &str,
    ) -> Result<bool, StoreError>;
    async fn is_secret_egress_host_allowed(
        &self,
        workspace_id: WorkspaceId,
        host: &str,
    ) -> Result<bool, StoreError>;
}

#[async_trait]
pub trait MemberFreezeStore: Send + Sync {
    /// Member-freeze kill-switch. `freeze_member` records the freeze, drops
    /// the member's active leases (charging each claim's worked wall time)
    /// and appends `MemberFrozen` in one tx,
    /// returning the freeze, the number of threads released and the event to
    /// publish; `claim_next` refuses a frozen member. Re-freezing refreshes the
    /// record and appends another event.
    async fn freeze_member(
        &self,
        member_id: MemberId,
        frozen_by: MemberId,
        reason: Option<&str>,
    ) -> Result<(MemberFreeze, u64, StoredEvent), StoreError>;
    /// Lift a freeze, appending `MemberUnfrozen` with it. `None` when the
    /// member was not frozen.
    async fn unfreeze_member(
        &self,
        member_id: MemberId,
        unfrozen_by: MemberId,
    ) -> Result<Option<StoredEvent>, StoreError>;
    async fn is_member_frozen(&self, member_id: MemberId) -> Result<bool, StoreError>;
    async fn get_member_freeze(
        &self,
        member_id: MemberId,
    ) -> Result<Option<MemberFreeze>, StoreError>;
    async fn list_frozen_members(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<MemberFreeze>, StoreError>;
}

#[async_trait]
pub trait MemoryBlockStore: Send + Sync {
    /// Attachable labeled memory blocks. A block is a Letta-shaped `{label,
    /// description, limit, read_only, value}` object a thread can attach to (a
    /// "room object"), letting a parent watch a child's result block without a
    /// nested runtime.
    ///
    /// `create_memory_block` is concurrent-safe on `(workspace_id, label)` — a
    /// racing create returns the existing block rather than erroring, so two
    /// racers converge on one block. `set_memory_block_value` is a full rewrite
    /// (last-writer-wins) that returns `InvalidInput` for a read-only block or
    /// a value over the block's char limit, and `NotFound` for an unknown
    /// block. `attach_memory_block` / `detach_memory_block` are idempotent and
    /// report whether they changed anything.
    async fn create_memory_block(&self, new: NewMemoryBlock) -> Result<MemoryBlock, StoreError>;
    async fn get_memory_block(&self, id: MemoryBlockId) -> Result<Option<MemoryBlock>, StoreError>;
    async fn get_memory_block_by_label(
        &self,
        workspace_id: WorkspaceId,
        label: &str,
    ) -> Result<Option<MemoryBlock>, StoreError>;
    async fn list_memory_blocks(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<MemoryBlock>, StoreError>;
    async fn set_memory_block_value(
        &self,
        id: MemoryBlockId,
        value: &str,
    ) -> Result<MemoryBlock, StoreError>;
    async fn delete_memory_block(&self, id: MemoryBlockId) -> Result<bool, StoreError>;
    async fn attach_memory_block(
        &self,
        thread_id: ThreadId,
        block_id: MemoryBlockId,
    ) -> Result<bool, StoreError>;
    async fn detach_memory_block(
        &self,
        thread_id: ThreadId,
        block_id: MemoryBlockId,
    ) -> Result<bool, StoreError>;
    async fn list_thread_memory_blocks(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<MemoryBlock>, StoreError>;
}

#[async_trait]
pub trait ReviewStore: Send + Sync {
    /// Required reviewers. `set_review_requirement` upserts the required
    /// approval count `k`; `add_reviewer` names the reviewer set (empty = open
    /// review — any qualifying member); `submit_review` upserts a reviewer's
    /// decision (re-submitting changes it). `review_status` counts the DISTINCT
    /// **qualifying** approvals — decision = approve, the reviewer is neither
    /// the thread's owner nor its assignee (separation of duties), and, when a
    /// named set exists, is in it — which the FSM close-gate (375.2) reads.
    async fn set_review_requirement(
        &self,
        thread_id: ThreadId,
        required_count: i64,
    ) -> Result<ThreadReviewRequirement, StoreError>;
    async fn get_review_requirement(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadReviewRequirement>, StoreError>;
    async fn clear_review_requirement(&self, thread_id: ThreadId) -> Result<bool, StoreError>;
    async fn add_reviewer(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<bool, StoreError>;
    async fn remove_reviewer(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<bool, StoreError>;
    async fn list_reviewers(&self, thread_id: ThreadId) -> Result<Vec<MemberId>, StoreError>;
    /// Record `reviewer_id`'s review and append its `ReviewSubmitted` event, in
    /// one transaction. A `request_changes` review that sends an `in_review`
    /// thread back for rework (from its owner or a reviewer whose approval
    /// would count) also reopens it, dismisses the approvals given to the
    /// version it replaces, and appends a `ThreadStateChanged`. The caller
    /// publishes both.
    async fn submit_review(
        &self,
        thread_id: ThreadId,
        reviewer_id: MemberId,
        decision: ReviewDecision,
        note: Option<&str>,
    ) -> Result<ReviewSubmission, StoreError>;
    async fn list_reviews(&self, thread_id: ThreadId) -> Result<Vec<ThreadReview>, StoreError>;
    /// Every review verdict on the thread, oldest first. `submit_review`
    /// appends one per submission, in its transaction; the history is never
    /// updated, so a re-submission or a dismissal leaves earlier verdicts as
    /// they were.
    async fn list_review_history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ReviewVerdict>, StoreError>;
    async fn review_status(&self, thread_id: ThreadId) -> Result<ReviewStatus, StoreError>;
    /// If `result` is a reviewed `example.review.result/1` with any `critical`
    /// finding **and** `reviewer_id` has declared the `review` skill, upsert a
    /// `request_changes` decision and, when the thread has no requirement yet,
    /// set `k = 1` so the close-gate refuses `closed` until a qualifying human
    /// approve. The verdict and that `k` commit in one transaction. `None` =
    /// nothing to apply (wrong shape, no critical, reviewer not review-skilled,
    /// or the reviewer's standing verdict already covers the thread's stored
    /// result). An existing `k` is left alone.
    async fn apply_critical_review_decision(
        &self,
        thread_id: ThreadId,
        reviewer_id: MemberId,
        result: &serde_json::Value,
    ) -> Result<Option<ReviewSubmission>, StoreError>;
}

#[async_trait]
pub trait LandGateStore: Send + Sync {
    /// Land-gate pointer. Presence of a row arms the close-gate.
    /// `require_land_gate` inserts a pending row (no pointer yet) so `closed`
    /// refuses until a qualifying green pass arrives; an existing pointer is
    /// left alone. `set_land_gate_pointer` upserts `{kind:"land_gate", status,
    /// artifact_sha?, land}` from a land-gate-skilled recorder.
    /// `get_land_gate_standing` is total (no row → not required, vacuous
    /// green). `clear_land_gate` deletes the row. The FSM close-gate (385.2)
    /// reads this standing in-tx.
    async fn require_land_gate(&self, thread_id: ThreadId) -> Result<LandGateStanding, StoreError>;
    async fn set_land_gate_pointer(
        &self,
        thread_id: ThreadId,
        recorded_by: MemberId,
        status: LandGateStatus,
        artifact_sha: Option<&str>,
        land: Option<LandColor>,
    ) -> Result<LandGateStanding, StoreError>;
    async fn get_land_gate_standing(
        &self,
        thread_id: ThreadId,
    ) -> Result<LandGateStanding, StoreError>;
    async fn clear_land_gate(&self, thread_id: ThreadId) -> Result<bool, StoreError>;
    /// Every land-gate verdict recorded on the thread, oldest first.
    /// `set_land_gate_pointer` appends one per recorded pointer, in its
    /// transaction; a later pointer or clearing the gate leaves them in place.
    async fn list_land_gate_history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<LandGateVerdict>, StoreError>;
}

#[async_trait]
pub trait SpawnBudgetStore: Send + Sync {
    /// Spawn budget. `set_spawn_budget` upserts the workspace's caps (each
    /// `None` = unlimited on that axis; all-`None` clears the row);
    /// `get_spawn_budget` is `None` when unset. `count_active_children` is a
    /// parent's non-tombstoned direct children (the fan-out so far);
    /// `thread_depth` is the nesting depth (a root thread is 1) via an ancestor
    /// walk; `count_thread_tool_uses` counts the tool-use blocks recorded
    /// across a thread's messages. These back the spawn-time gate.
    async fn set_spawn_budget(
        &self,
        workspace_id: WorkspaceId,
        max_children: Option<i64>,
        max_depth: Option<i64>,
        max_tools: Option<i64>,
    ) -> Result<Option<SpawnBudget>, StoreError>;
    async fn get_spawn_budget(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<SpawnBudget>, StoreError>;
    async fn count_active_children(&self, parent_thread_id: ThreadId) -> Result<i64, StoreError>;
    async fn thread_depth(&self, thread_id: ThreadId) -> Result<i64, StoreError>;
    async fn count_thread_tool_uses(&self, thread_id: ThreadId) -> Result<i64, StoreError>;
}

#[async_trait]
pub trait AssignmentStore: Send + Sync {
    /// Set a thread's assignee unconditionally (assign / handoff). The
    /// assignment has no lease: an earlier holder's deadline is cleared.
    /// `NotFound` if the thread doesn't exist or the assignee is not a member
    /// of its workspace.
    async fn assign_thread(
        &self,
        thread_id: ThreadId,
        assignee_id: MemberId,
    ) -> Result<Thread, StoreError>;
    /// Set (or clear, with `None`) a thread's durable owner — the accountable
    /// party, distinct from the assignee/claimer. `NotFound` if the thread is
    /// absent or tombstoned, or the owner is not a member of its workspace. Orthogonal to assignment; does not touch the claim
    /// lease or working clock.
    async fn set_thread_owner(
        &self,
        thread_id: ThreadId,
        owner_id: Option<MemberId>,
    ) -> Result<Thread, StoreError>;
    /// Rename a thread — titled threads become editable. `NotFound` if the
    /// thread is absent or tombstoned. Touches only `title`; a rename is
    /// metadata, not activity, so it does not bump the activity-sort key.
    async fn set_thread_title(
        &self,
        thread_id: ThreadId,
        title: Option<String>,
    ) -> Result<Thread, StoreError>;
    /// Assign a thread and append its `ThreadAssignmentChanged` event
    /// atomically. Captures the previous assignee in the same tx. `NotFound`
    /// if the assignee is not a member of the thread's workspace.
    async fn assign_thread_with_event(
        &self,
        thread_id: ThreadId,
        assignee_id: MemberId,
        actor_id: MemberId,
        note: Option<String>,
    ) -> Result<(Thread, StoredEvent), StoreError>;

    /// Atomically claim an unassigned thread for `member_id`. The
    /// compare-and-set (`WHERE assignee_id IS NULL`) makes concurrent claims
    /// race-safe: exactly one wins. The claim has no lease. `claimed` is
    /// `false` when it was already assigned; `NotFound` only when the thread
    /// doesn't exist.
    async fn claim_thread(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<ThreadClaimResult, StoreError>;
    /// Claim a thread, appending `ThreadAssignmentChanged` atomically **iff**
    /// the CAS actually claimed. `(result, event)`.
    async fn claim_thread_with_event(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
    ) -> Result<(ThreadClaimResult, Option<StoredEvent>), StoreError>;

    /// Clear a thread's assignee. `NotFound` if it doesn't exist.
    async fn unassign_thread(&self, thread_id: ThreadId) -> Result<Thread, StoreError>;
    /// Clear a thread's assignee and append its `ThreadAssignmentChanged` event
    /// atomically.
    async fn unassign_thread_with_event(
        &self,
        thread_id: ThreadId,
        actor_id: MemberId,
    ) -> Result<(Thread, StoredEvent), StoreError>;

    /// Threads in `workspace_id` assigned to `member_id` — the agent's work
    /// queue. Live threads only, oldest first.
    async fn list_assigned_threads(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<Thread>, StoreError>;
    /// Threads under review that name `member_id` as a reviewer and lack that
    /// member's approval, oldest first: the reviews waiting on them.
    async fn list_review_requests(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
    ) -> Result<Vec<Thread>, StoreError>;
    /// Threads under review in `workspace_id` that name no reviewer, so no
    /// review request reaches anyone: those `member_id` owns, plus, when
    /// `include_ownerless`, those with no owner. Oldest first, by when review
    /// began. Access is the caller's to filter.
    async fn list_unassigned_reviews(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        include_ownerless: bool,
    ) -> Result<Vec<Thread>, StoreError>;

    /// Atomically claim the oldest claimable live thread in `channel_id` for
    /// `member_id` — the "pull the next task" primitive. Claimable = unassigned
    /// **or** its lease has expired,
    /// **and** every task-dependency is terminal (a task blocked by an
    /// unfinished dependency is skipped), **and** it has no
    /// explicit [`BlockedReason`] row. `lease_secs` sets a lease deadline
    /// (`None` = durable, no lease). `None` return when there is no claimable
    /// *ready* work. Concurrent claimers get distinct threads. A thread goes
    /// only to a member who may read it, by `maidan_auth::authorize_thread`'s
    /// rule: a `__dm__` thread to its DM's participants, a private channel's
    /// to its members, and nothing outside the member's workspace.
    async fn claim_next_thread(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        lease_secs: Option<i64>,
    ) -> Result<Option<Thread>, StoreError>;
    /// Claim the next thread, appending its events atomically **iff** a thread
    /// was claimed. The returned events are the reclaim's
    /// `ThreadAssignmentChanged`, preceded by a `ClaimExpired` when the claim
    /// took over an expired lease (the previous holder's claim lapsed), or a
    /// `ClaimFailed` when charging that claim's worked time put the thread
    /// over budget (see [`reap_expired_claims`](Self::reap_expired_claims)).
    /// Empty vec when nothing was claimed.
    async fn claim_next_thread_with_event(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        lease_secs: Option<i64>,
    ) -> Result<(Option<Thread>, Vec<StoredEvent>), StoreError>;
    /// [`claim_next_thread_with_event`](Self::claim_next_thread_with_event)
    /// across every channel of `workspace_id` that `member_id` may read: the
    /// same filters, order, lease and events, without the channel predicate.
    /// A `workspace_id` other than the member's own finds nothing.
    async fn claim_next_workspace_thread_with_event(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        lease_secs: Option<i64>,
    ) -> Result<(Option<Thread>, Vec<StoredEvent>), StoreError>;

    /// Return up to `limit` claims whose lease lapsed before `now` to the
    /// queue, appending a `ClaimExpired` for each dead holder in the same
    /// transaction — the eager twin of the reclaim inside
    /// [`claim_next_thread_with_event`](Self::claim_next_thread_with_event).
    /// Only open, live threads are reaped, the ones `claim_next` could take.
    /// The holder, lease, fencing token and working clock are cleared, so the
    /// dead holder's token is fenced from then on. In the same transaction the
    /// time each claim worked (acknowledgement to deadline; nothing if never
    /// acknowledged) is charged to its thread's `used_wall_secs`, and a claim
    /// that leaves its thread over budget gets `ClaimFailed` and a DLQ entry,
    /// as a usage report would, instead of `ClaimExpired`. The reclaim inside
    /// `claim_next` charges the same way. Concurrent reapers take
    /// distinct threads (`FOR UPDATE SKIP LOCKED` on Postgres; SQLite
    /// serializes writers). Returns the appended events, oldest deadline
    /// first.
    async fn reap_expired_claims(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Report up to `limit` leased claims taken before `claimed_before` whose
    /// holder has not acknowledged them, appending a `ClaimUnacknowledged`
    /// for each in the same transaction. Only open, live threads whose lease
    /// is still running at `now` count (a lapsed one is the reaper's), and
    /// each claim is reported once: the claim's fencing token is recorded,
    /// and a new claim mints a new one. The claim itself is not changed.
    /// Concurrent callers report distinct claims (`FOR UPDATE SKIP LOCKED` on
    /// Postgres; a guarded update on SQLite). Returns the appended events,
    /// oldest claim first.
    async fn report_unacknowledged_claims(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        claimed_before: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Extend a claimed thread's lease (heartbeat), only for the current
    /// assignee holding the matching fencing token. `NotFound` if the thread is
    /// gone, the caller isn't the holder, or `lease_id` doesn't match the
    /// thread's current claim — so a stale holder whose claim was reclaimed by
    /// the next owner can no longer extend a lease it no longer holds.
    async fn renew_claim(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
        lease_id: ClaimLeaseId,
        lease_secs: i64,
    ) -> Result<Thread, StoreError>;

    /// Stamp the working clock: the current claim-holder acknowledges and
    /// begins work. Fenced by `(member_id, lease_id)`. Idempotent (the first
    /// start time wins). `NotFound` if the caller isn't the holder or the token
    /// is stale.
    async fn acknowledge_claim(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
        lease_id: ClaimLeaseId,
    ) -> Result<Thread, StoreError>;

    /// Release a claim (graceful handoff): the current holder returns the
    /// thread to the queue immediately (e.g. a clean shutdown) rather than
    /// letting the lease lapse. Fenced by `(member_id, lease_id)`. `NotFound`
    /// if the caller isn't the holder or the token is stale.
    async fn release_claim(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
        lease_id: ClaimLeaseId,
    ) -> Result<Thread, StoreError>;

    /// Release a claim and append its `ThreadAssignmentChanged` event
    /// atomically. The previous assignee is the caller.
    async fn release_claim_with_event(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
        lease_id: ClaimLeaseId,
    ) -> Result<(Thread, StoredEvent), StoreError>;

    /// Count a member's *live* claims: threads assigned to them that are
    /// non-terminal, non-tombstoned, and whose lease has not expired (a
    /// durable, no-lease assignment counts; an expired lease does not — it is a
    /// reclaimable ghost, not live work). This is the quantity a WIP limit
    /// caps.
    async fn count_live_claims(&self, member_id: MemberId) -> Result<i64, StoreError>;

    /// Park a thread from dispatch: while marked, `claim_next` skips it and an
    /// explicit claim is refused. Upserts the reason/actor.
    async fn mark_thread_unclaimable(
        &self,
        thread_id: ThreadId,
        reason: &str,
        marked_by: MemberId,
    ) -> Result<ThreadUnclaimable, StoreError>;
    /// Clear a thread's dispatch park. `true` if it was parked.
    async fn mark_thread_claimable(&self, thread_id: ThreadId) -> Result<bool, StoreError>;
    /// The thread's dispatch park, or `None` if it is claimable.
    async fn get_thread_unclaimable(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadUnclaimable>, StoreError>;
    /// The parked (unclaimable) threads in a channel, newest first.
    async fn list_unclaimable_threads(
        &self,
        channel_id: ChannelId,
    ) -> Result<Vec<ThreadUnclaimable>, StoreError>;

    /// Set (upsert) an explicit dispatch block. Presence of the row parks the
    /// thread from `claim_next` (enforced in
    /// 386.2). One block per thread; re-setting replaces the reason/actor.
    /// Emits `ThreadBlocked`.
    async fn set_thread_block(
        &self,
        thread_id: ThreadId,
        reason: BlockedReason,
        set_by: MemberId,
        note: Option<String>,
    ) -> Result<(ThreadBlock, StoredEvent), StoreError>;
    /// Clear a thread's explicit block. Returns the cleared row; `None` if it
    /// was not blocked (idempotent). Prefer [`clear_thread_block_with_event`]
    /// to emit `BlockedResolved`.
    async fn clear_thread_block(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadBlock>, StoreError>;
    /// Clear a thread's explicit block and append `BlockedResolved` atomically.
    /// `None` event when the thread was not blocked.
    async fn clear_thread_block_with_event(
        &self,
        thread_id: ThreadId,
        resolved_by: MemberId,
    ) -> Result<(Option<ThreadBlock>, Option<StoredEvent>), StoreError>;
    /// The thread's explicit block, or `None` if unblocked.
    async fn get_thread_block(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadBlock>, StoreError>;
    /// Explicitly blocked threads in a channel, newest first.
    async fn list_blocked_threads(
        &self,
        channel_id: ChannelId,
    ) -> Result<Vec<ThreadBlock>, StoreError>;
    /// Explicit blocks for exactly these threads, newest first. For attaching
    /// `Thread.block` to one page of threads without reading the channel's
    /// whole block list.
    async fn list_blocks_for_threads(
        &self,
        thread_ids: &[ThreadId],
    ) -> Result<Vec<ThreadBlock>, StoreError>;

    /// Declare (or supersede) the agent's self-reported status on a thread.
    /// The declaration is by the claim holder or owner; `stalled` is refused
    /// (system-computed only). Appends `StatusDeclared` atomically.
    async fn declare_thread_status(
        &self,
        thread_id: ThreadId,
        status: DeclaredStatus,
        note: String,
        declared_by: MemberId,
    ) -> Result<(ThreadStatusDeclaration, StoredEvent), StoreError>;
    /// Clear a thread's status declaration. Used when a human responds —
    /// the declaration is superseded by human activity. Returns the cleared
    /// declaration; `None` if there was none (idempotent).
    async fn clear_thread_status(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadStatusDeclaration>, StoreError>;
    /// The thread's active status declaration, or `None` if cleared.
    async fn get_thread_status(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadStatusDeclaration>, StoreError>;
    /// Active status declarations for threads in a channel. For the board's
    /// thread list: the chip renders from `Thread.status`, which the store's
    /// thread rows leave unset, so the route attaches these in one read —
    /// the same pattern as `list_blocked_threads`.
    async fn list_thread_statuses_for_channel(
        &self,
        channel_id: ChannelId,
    ) -> Result<Vec<ThreadStatusDeclaration>, StoreError>;
    /// Threads blocked with `human` or `gate` reason in a workspace, with
    /// their blocks. For the waiting inbox: these need a human (owner or
    /// admin) to unblock. Returns (thread_id, title, owner_id, block).
    /// Newest first.
    async fn list_human_gate_blocked_threads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<(ThreadId, Option<String>, Option<MemberId>, ThreadBlock)>, StoreError>;
    /// Threads whose agent declared `needs_input` in a workspace, with their
    /// declarations. A person's own `needs_input` is not an agent's question,
    /// so only a declaration by an agent member is listed. For the waiting inbox: an agent's question waits on a
    /// human until someone answers in the thread. Returns (thread_id, title,
    /// owner_id, declaration), oldest question first.
    async fn list_threads_needing_input(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<
        Vec<(
            ThreadId,
            Option<String>,
            Option<MemberId>,
            ThreadStatusDeclaration,
        )>,
        StoreError,
    >;

    /// Set (upsert) a thread's wait timer: the thread is waiting until
    /// `wait_until`, escalating via `on_timeout` on lapse. Re-setting resets
    /// `fired_at` (a fresh timer). One wait per thread.
    async fn set_thread_wait(
        &self,
        thread_id: ThreadId,
        wait_until: chrono::DateTime<chrono::Utc>,
        on_timeout: EscalationPolicy,
        reason: Option<&str>,
        created_by: MemberId,
    ) -> Result<ThreadWait, StoreError>;
    /// Cancel a thread's wait — the awaited thing happened. `true` when a wait
    /// existed.
    async fn cancel_thread_wait(&self, thread_id: ThreadId) -> Result<bool, StoreError>;
    /// The thread's wait, or `None`.
    async fn get_thread_wait(&self, thread_id: ThreadId) -> Result<Option<ThreadWait>, StoreError>;
    /// Atomically claim the oldest **due** un-fired wait (`wait_until <= now`)
    /// and stamp `fired_at = now`, returning it — the sweeper's fire-once
    /// primitive. `FOR UPDATE SKIP LOCKED` on Postgres so concurrent replicas
    /// never double-fire one wait; SQLite serializes writers. `None` when
    /// nothing is due.
    async fn claim_next_due_wait(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<ThreadWait>, StoreError>;

    /// Set (upsert) a thread's dispatch priority. Higher = more urgent; the
    /// default (no row) is `0`. `claim_next` orders by an effective rank that
    /// ages this base priority up the longer a thread has waited, so priority
    /// jumps the queue without starving long-waiting tasks.
    async fn set_thread_priority(
        &self,
        thread_id: ThreadId,
        priority: i64,
        set_by: MemberId,
    ) -> Result<ThreadPriority, StoreError>;
    /// The thread's dispatch priority record, or `None` (= the default 0).
    async fn get_thread_priority(
        &self,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadPriority>, StoreError>;
}

#[async_trait]
pub trait ThreadDepStore: Send + Sync {
    /// Add a task-dependency edge: `thread_id` depends on `depends_on`.
    /// Idempotent; a self-dependency is rejected; both threads must exist.
    async fn add_thread_dependency(
        &self,
        thread_id: ThreadId,
        depends_on: ThreadId,
    ) -> Result<(), StoreError>;
    /// Remove a dependency edge; `true` when a row was deleted.
    async fn remove_thread_dependency(
        &self,
        thread_id: ThreadId,
        depends_on: ThreadId,
    ) -> Result<bool, StoreError>;
    /// Edges `thread_id` depends on — what this task is blocked by.
    async fn list_thread_dependencies(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadDependency>, StoreError>;
    /// Edges that depend on `thread_id` — what this task blocks.
    async fn list_thread_dependents(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadDependency>, StoreError>;
    /// Whether every dependency of `thread_id` is terminal (closed/archived) —
    /// the task is ready to run. A task with no dependencies is ready.
    async fn thread_dependencies_satisfied(&self, thread_id: ThreadId) -> Result<bool, StoreError>;
    /// Non-terminal dependents of `thread_id` whose dependencies are now *all*
    /// terminal — i.e. the tasks that just became ready because `thread_id`
    /// reached a terminal state. Callers invoke this right after transitioning
    /// `thread_id` into a terminal state to emit `ThreadReady` for each result.
    async fn newly_ready_dependents(&self, thread_id: ThreadId) -> Result<Vec<Thread>, StoreError>;
}

#[async_trait]
pub trait MessageStore: Send + Sync {
    async fn post_message(&self, new: NewMessage) -> Result<Message, StoreError>;
    /// Insert a message and append its `MessagePosted` event atomically. For
    /// the DM / group-DM post paths, which do no post-insert slash edit.
    /// `dm_conversation_id` is `Some` for a 1:1 DM, `None` otherwise.
    async fn post_message_with_event(
        &self,
        new: NewMessage,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<(Message, StoredEvent), StoreError>;
    /// Edit a just-posted message and append its `MessagePosted` event
    /// (reflecting the edited message) atomically — the regular post path's
    /// slash-command finalization, where the event must carry the post-edit
    /// message.
    async fn edit_message_with_posted_event(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<(Message, StoredEvent), StoreError>;
    async fn edit_message(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
    ) -> Result<Message, StoreError>;
    /// Edit a message and append its `MessageEdited` event atomically.
    async fn edit_message_with_event(
        &self,
        id: MessageId,
        editor_id: MemberId,
        edit: EditMessage,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<(Message, StoredEvent), StoreError>;
    async fn list_message_edits(
        &self,
        message_id: MessageId,
        limit: i64,
    ) -> Result<Vec<MessageEdit>, StoreError>;

    /// Edits for many messages in one windowed query, at most `limit_per` per
    /// message (newest-last, like `list_message_edits`). Avoids the per-message
    /// edit N+1 in thread-context assembly. Returns a flat list the caller
    /// groups by `message_id`.
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
    /// Tombstone a message and append its `MessageTombstoned` event atomically.
    async fn tombstone_message_with_event(
        &self,
        id: MessageId,
        dm_conversation_id: Option<DmConversationId>,
    ) -> Result<StoredEvent, StoreError>;
    /// Hard-delete a tombstoned message (GDPR erasure). Fails if not
    /// tombstoned; `Conflict` under legal hold.
    async fn purge_message(&self, id: MessageId) -> Result<(), StoreError>;
    /// Tombstone then hard-delete all messages in a workspace (GDPR erasure),
    /// in one transaction. `Conflict` under legal hold.
    async fn purge_workspace_messages(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspacePurgeResult, StoreError>;
    /// Deep purge then delete the workspace row and CASCADE-owned data, in
    /// one transaction. `Conflict` under legal hold.
    async fn erase_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceEraseResult, StoreError>;

    // D-A: the audited forms request handlers use.

    /// [`Self::purge_message`] with its audit row in the same transaction.
    async fn purge_message_audited(
        &self,
        id: MessageId,
        audit: NewAuditEvent,
    ) -> Result<(), StoreError>;
    /// [`Self::purge_workspace_messages`] with its audit row in the same
    /// transaction.
    async fn purge_workspace_messages_audited(
        &self,
        workspace_id: WorkspaceId,
        audit: crate::AuditFor<WorkspacePurgeResult>,
    ) -> Result<WorkspacePurgeResult, StoreError>;
    /// [`Self::erase_workspace`] with its audit row in the same transaction.
    async fn erase_workspace_audited(
        &self,
        workspace_id: WorkspaceId,
        audit: crate::AuditFor<WorkspaceEraseResult>,
    ) -> Result<WorkspaceEraseResult, StoreError>;
}

#[async_trait]
pub trait MentionInboxStore: Send + Sync {
    /// Record a mention without appending its event: the `@handle` router
    /// publishes `MentionRecorded` itself. `NotFound` unless `member_id` is a
    /// member of the message's workspace, as for the evented form.
    async fn record_mention(
        &self,
        message_id: MessageId,
        member_id: MemberId,
    ) -> Result<(), StoreError>;
    /// Record a mention and append its `MentionRecorded` event atomically.
    /// `member_id` is the mentioned party; `NotFound` unless it is a member of
    /// the message's workspace.
    async fn record_mention_with_event(
        &self,
        message_id: MessageId,
        member_id: MemberId,
    ) -> Result<StoredEvent, StoreError>;
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
}

#[async_trait]
pub trait SocialStore: Send + Sync {
    async fn cast_vote(&self, new: NewVote) -> Result<(), StoreError>;
    /// Cast a vote and append its events atomically: a `VoteRetracted` for the
    /// opposing verdict it replaced, if any, then its `VoteCast`.
    async fn cast_vote_with_event(&self, new: NewVote) -> Result<Vec<StoredEvent>, StoreError>;
    /// Take back the member's vote of `kind`, appending `VoteRetracted`
    /// atomically **iff** a row was removed. `(removed, event)`.
    async fn retract_vote_with_event(
        &self,
        message_id: MessageId,
        member_id: MemberId,
        kind: VoteKind,
    ) -> Result<(bool, Option<StoredEvent>), StoreError>;
    async fn list_votes_for_message(&self, message_id: MessageId) -> Result<Vec<Vote>, StoreError>;

    async fn add_reaction(&self, new: NewReaction) -> Result<(), StoreError>;
    /// Add a reaction and append its `ReactionAdded` event atomically.
    async fn add_reaction_with_event(&self, new: NewReaction) -> Result<StoredEvent, StoreError>;
    async fn remove_reaction(
        &self,
        message_id: MessageId,
        member_id: MemberId,
        emoji: &str,
    ) -> Result<bool, StoreError>;
    /// Remove a reaction, appending `ReactionRemoved` atomically **iff** a row
    /// was removed. `(removed, event)`.
    async fn remove_reaction_with_event(
        &self,
        message_id: MessageId,
        member_id: MemberId,
        emoji: &str,
    ) -> Result<(bool, Option<StoredEvent>), StoreError>;
    async fn list_reactions_for_message(
        &self,
        message_id: MessageId,
    ) -> Result<Vec<Reaction>, StoreError>;

    async fn pin_message(&self, new: NewPin) -> Result<(), StoreError>;
    /// Pin a message and append its `MessagePinned` event atomically.
    async fn pin_message_with_event(&self, new: NewPin) -> Result<StoredEvent, StoreError>;
    async fn unpin_message(
        &self,
        thread_id: ThreadId,
        message_id: MessageId,
    ) -> Result<bool, StoreError>;
    /// Unpin a message, appending `MessageUnpinned` atomically **iff** a row
    /// was removed. `member_id` is the actor. `(removed, event)`.
    async fn unpin_message_with_event(
        &self,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
    ) -> Result<(bool, Option<StoredEvent>), StoreError>;
    async fn list_pins_for_thread(&self, thread_id: ThreadId) -> Result<Vec<Pin>, StoreError>;
}

#[async_trait]
pub trait ReferenceStore: Send + Sync {
    async fn add_reference(&self, new: NewReference) -> Result<Reference, StoreError>;
    /// Add a reference and append its `ReferenceAdded` event atomically.
    async fn add_reference_with_event(
        &self,
        new: NewReference,
    ) -> Result<(Reference, StoredEvent), StoreError>;
    async fn list_references_from(
        &self,
        src_kind: RefSide,
        src_id: uuid::Uuid,
    ) -> Result<Vec<Reference>, StoreError>;

    /// References pointing AT one target — the reverse edge.
    async fn list_references_to(
        &self,
        dst_kind: RefSide,
        dst_id: uuid::Uuid,
    ) -> Result<Vec<Reference>, StoreError>;

    /// References from many sources of one kind in a single query. Avoids the
    /// per-message `list_references_from` N+1. Returns a flat list the caller
    /// groups by `src_id`; ordered by `src_id` then `created_at ASC`.
    async fn list_references_from_many(
        &self,
        src_kind: RefSide,
        src_ids: &[uuid::Uuid],
    ) -> Result<Vec<Reference>, StoreError>;
}

/// Tombstone explorer, message backlink index, and EventKind census. Reads only
/// — no new table.
#[async_trait]
pub trait IntegrityStore: Send + Sync {
    /// Tombstoned messages in `workspace_id`, newest first. Optional channel /
    /// thread narrowing. `include_purged` reconstructs hard-deleted rows from
    /// `MessageTombstoned` events (`retained = false`).
    async fn list_tombstones(
        &self,
        workspace_id: WorkspaceId,
        channel_id: Option<ChannelId>,
        thread_id: Option<ThreadId>,
        include_purged: bool,
        limit: i64,
    ) -> Result<Vec<TombstoneRecord>, StoreError>;

    /// Incoming pointers at `message_id` (`RelationKind` reverse edges + pins
    /// + reactions + votes). `NotFound` if the message row is gone.
    async fn list_message_backlinks(
        &self,
        message_id: MessageId,
    ) -> Result<MessageBacklinks, StoreError>;

    /// `EventKind` counts for `workspace_id`, optionally narrowed to a channel
    /// or thread. `deny_channels` drops those channels' events (private-channel
    /// pre-filter); `channel_id IS NULL` workspace-level events stay.
    async fn event_kind_census(
        &self,
        workspace_id: WorkspaceId,
        channel_id: Option<ChannelId>,
        thread_id: Option<ThreadId>,
        deny_channels: &[ChannelId],
    ) -> Result<KindCensus, StoreError>;
}

#[async_trait]
pub trait ArtifactMetaStore: Send + Sync {
    async fn upsert_artifact(&self, new: NewArtifact) -> Result<Artifact, StoreError>;
    /// Upsert an artifact, optionally record its per-workspace access ref, and
    /// append its `ArtifactUpserted` event — all atomically. `ref_workspace` is
    /// `Some` when the caller would record a ref (non-bypass).
    async fn upsert_artifact_with_event(
        &self,
        new: NewArtifact,
        ref_workspace: Option<WorkspaceId>,
    ) -> Result<(Artifact, StoredEvent), StoreError>;
    async fn get_artifact_by_sha(&self, sha256: &str) -> Result<Artifact, StoreError>;
    /// The artifact as `workspace_id` sees it: shared content with that
    /// workspace's own `kind`, `mime_type`, `uploaded_by` and `created_at`.
    /// `NotFound` when the workspace has no access ref. Every read on behalf
    /// of a workspace uses this, never [`Self::get_artifact_by_sha`], whose
    /// metadata may be another tenant's.
    async fn get_artifact_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        sha256: &str,
    ) -> Result<Artifact, StoreError>;

    /// Record that `workspace_id` may access the artifact `sha256`. Idempotent.
    /// Written on upload; enforced on fetch by [`Self::artifact_ref_exists`].
    async fn record_artifact_ref(
        &self,
        workspace_id: WorkspaceId,
        sha256: &str,
    ) -> Result<(), StoreError>;

    /// Whether `workspace_id` has an access link to the artifact `sha256` — the
    /// per-tenant gate over the deduped blob store.
    async fn artifact_ref_exists(
        &self,
        workspace_id: WorkspaceId,
        sha256: &str,
    ) -> Result<bool, StoreError>;

    /// Erase `workspace_id`'s reference to `sha256`, with its audit row in the
    /// same transaction. The artifact row goes with the last reference; the
    /// caller then deletes the blob. `NotFound` without a reference,
    /// `Conflict` under a legal hold.
    async fn erase_artifact_audited(
        &self,
        workspace_id: WorkspaceId,
        sha256: &str,
        audit: crate::AuditFor<ArtifactErasure>,
    ) -> Result<ArtifactErasure, StoreError>;

    /// Delete the bytes of `sha256` if no artifact row holds it. The check
    /// and a lease on the sha commit together; `delete` then runs outside any
    /// transaction, bounded by [`crate::BLOB_DELETE_TIMEOUT`], and every
    /// artifact upsert of the sha waits while the lease is live. An upload of
    /// the same bytes therefore either commits its row first (the bytes stay)
    /// or after the delete, and then puts them back
    /// (`maidan_artifacts::restore_if_reaped`). A delete that overruns is
    /// reported as failed and its lease left to lapse, since it may still
    /// land. Called after a last-reference erase or a workspace purge orphaned
    /// the sha.
    async fn reap_artifact_blob(
        &self,
        sha256: &str,
        delete: crate::BlobDelete<'_>,
    ) -> Result<crate::BlobReap, StoreError>;
}

#[async_trait]
pub trait EventStore: Send + Sync {
    async fn append_audit(&self, new: NewAuditEvent) -> Result<AuditEvent, StoreError>;
    async fn list_audit(&self, limit: i64) -> Result<Vec<AuditEvent>, StoreError>;
    /// The rows stamped with `workspace_id` when they were written, newest
    /// first. Instance-level rows appear only in [`EventStore::list_audit`].
    async fn list_audit_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<AuditEvent>, StoreError>;

    async fn append_event(&self, event: &Event) -> Result<StoredEvent, StoreError>;
    /// Append an event that arrived from federation peer `origin`. A message's
    /// words are sealed under a key scoped to `(origin, message)`, so only that
    /// peer's tombstone can shred them; an event the origin already shredded
    /// (sealed, no key) is stored as ciphertext under a shredded key row.
    async fn append_federated_event(
        &self,
        event: &Event,
        origin: PeerId,
    ) -> Result<StoredEvent, StoreError>;
    /// Re-wrap up to `limit` content keys still wrapped by a previous KEK under
    /// the primary one. Returns how many were re-wrapped; `0` means done.
    async fn rewrap_content_keys(&self, limit: i64) -> Result<u64, StoreError>;
    /// Live content keys not yet wrapped by the primary KEK.
    async fn content_keys_needing_rewrap(&self) -> Result<u64, StoreError>;
    async fn get_stored_event(&self, log_id: i64) -> Result<StoredEvent, StoreError>;
    /// A thread's events with `id <= through_id`, in `id` order — the immutable
    /// substrate for as-of context replay.
    async fn list_thread_events_through(
        &self,
        thread_id: ThreadId,
        through_id: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;
    async fn list_events_after(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Like [`Store::list_events_after`] but only rows inserted at or before
    /// `stable_before` (the at-least-once reconcile read).
    async fn list_events_after_stable(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
        stable_before: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Lowest retained event `id` in `workspace_id` (`None` if the workspace
    /// has no rows). The floor a `CursorTooOld` check compares against.
    async fn min_event_id(&self, workspace_id: WorkspaceId) -> Result<Option<i64>, StoreError>;

    /// Every workspace that has at least one event, so a chain verifier knows
    /// what there is to verify.
    ///
    /// Derived from the log rather than the workspaces table on purpose: a
    /// workspace with no events has no chain, and verifying it would report a
    /// vacuous pass that is indistinguishable from a real one.
    async fn workspace_ids_with_events(&self) -> Result<Vec<WorkspaceId>, StoreError>;

    /// Highest event-log `id` across all workspaces (`0` when empty). The
    /// `Maidan-Room-LSN` value — the room head a client compares to last-seen
    /// `log_id`. **Not** a Postgres WAL [`maidan_types::Lsn`]
    /// (`Maidan-Consistency-Token`).
    async fn max_event_id(&self) -> Result<i64, StoreError>;

    /// The oldest event with `id > after_id` whose kind is in `kinds`, as
    /// `(id, inserted_at)`; `None` when there is none. `inserted_at` is the
    /// store's own clock, not the caller-supplied `occurred_at`. Readiness uses
    /// this to tell a projector that is behind from one with nothing to do.
    async fn oldest_event_after_of_kinds(
        &self,
        after_id: i64,
        kinds: &[EventKind],
    ) -> Result<Option<(i64, DateTime<Utc>)>, StoreError>;

    /// Where a tap projector last finished. `0` = never run.
    ///
    /// The search tap re-walked the whole log from genesis on every start,
    /// resubscribe and `Lagged` — re-embedding all history each time, and
    /// livelocking on a busy instance because the bus is not drained during a
    /// backfill.
    async fn tap_cursor(&self, surface: &str) -> Result<i64, StoreError>;

    /// Advance a tap's resume point. Monotonic — a lower value is ignored, so a
    /// slower replica cannot drag the cursor backwards.
    async fn set_tap_cursor(&self, surface: &str, last_event_id: i64) -> Result<(), StoreError>;

    /// Forget a tap's resume point so the next backfill re-walks from genesis.
    /// The rebuild path: resuming past a detected break would preserve exactly
    /// the divergence that was detected.
    async fn clear_tap_cursor(&self, surface: &str) -> Result<(), StoreError>;

    /// Cross-workspace `id > after_id` page, in `id` order. Internal bus
    /// consumers (indexer, webhook, notification router, FSM hooks) resume from
    /// this after `RecvError::Lagged` instead of dropping.
    async fn list_events_after_global(
        &self,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Verify the retained hash chain for `workspace_id`. Empty workspace is
    /// ok. Fail-closed report: `ok == false` on a break.
    async fn verify_event_chain(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<maidan_types::ChainVerifyReport, StoreError>;

    /// Oldest retained event link in `workspace_id`.
    async fn workspace_event_floor(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<maidan_types::EventLink>, StoreError>;

    /// Newest retained event link in `workspace_id`.
    async fn workspace_event_head(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<maidan_types::EventLink>, StoreError>;

    /// Latest event in `workspace_id` with `id <= lsn` — the catch-up
    /// predecessor when `after_lsn` may sit in another tenant's id gap.
    async fn workspace_event_at_or_before(
        &self,
        workspace_id: WorkspaceId,
        lsn: i64,
    ) -> Result<Option<maidan_types::EventLink>, StoreError>;

    /// Fail loud when `after_id` points into a pruned gap. `after_id <= 0`
    /// (fresh subscriber) is never too old. Default impl; both backends
    /// inherit it.
    async fn ensure_cursor_fresh(
        &self,
        workspace_id: WorkspaceId,
        after_id: i64,
    ) -> Result<(), StoreError> {
        if after_id <= 0 {
            return Ok(());
        }
        let Some(oldest_id) = self.min_event_id(workspace_id).await? else {
            return Ok(());
        };
        if maidan_types::cursor_is_too_old(after_id, Some(oldest_id)) {
            return Err(StoreError::cursor_too_old(after_id, oldest_id));
        }
        Ok(())
    }
}

#[async_trait]
pub trait AppStore: Send + Sync {
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
}

/// Idempotency keys for retried writes. See [`crate::idempotency`].
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Reserve `new.key` for this caller, or report what already holds it: a
    /// live reservation, or a finished request's stored response. A
    /// reservation that lapsed at `locked_until` without completing is taken
    /// over by a retry of the same request (same fingerprint). Clears the
    /// requested key if it expired, then a bounded batch of other expired
    /// keys, on the way in.
    async fn reserve_idempotency_key(
        &self,
        new: &crate::idempotency::NewIdempotencyKey,
    ) -> Result<crate::idempotency::IdempotencyReservation, StoreError>;

    /// Store the response of the request holding the key under `lease`.
    async fn complete_idempotency_key(
        &self,
        workspace_id: WorkspaceId,
        actor_id: MemberId,
        key: &str,
        lease: &str,
        response: &crate::idempotency::StoredResponse,
    ) -> Result<(), StoreError>;

    /// Give up a reservation without a response to keep (the request failed
    /// in a way a retry should repeat), so a retry runs again.
    async fn release_idempotency_key(
        &self,
        workspace_id: WorkspaceId,
        actor_id: MemberId,
        key: &str,
        lease: &str,
    ) -> Result<(), StoreError>;
}

/// Stateless MCP resource subscriptions, shared by every replica. See
/// [`crate::mcp_subscriptions`].
#[async_trait]
pub trait McpSubscriptionStore: Send + Sync {
    /// Subscribe `new.subscriber` to `new.uri`, and move every live
    /// subscription of that subscriber to `new.expires_at`. `false`, with
    /// nothing added, when the subscriber already watches `limit` other
    /// resources. The subscriber's lapsed rows are cleared first, so they do
    /// not count.
    async fn subscribe_mcp_resource(
        &self,
        new: &crate::mcp_subscriptions::NewMcpSubscription,
        limit: usize,
    ) -> Result<bool, StoreError>;

    /// Whether `subscriber` was watching `uri`.
    async fn unsubscribe_mcp_resource(
        &self,
        subscriber: &str,
        uri: &str,
    ) -> Result<bool, StoreError>;

    /// The live subscriptions, among `subscribers`, to any of `uris` that an
    /// update in `workspace_id` may reach: those taken in that workspace, and
    /// an auth-disabled caller's, which belong to none.
    async fn mcp_resource_watchers(
        &self,
        workspace_id: WorkspaceId,
        uris: &[String],
        subscribers: &[String],
        now: DateTime<Utc>,
    ) -> Result<Vec<crate::mcp_subscriptions::McpSubscriptionWatch>, StoreError>;

    /// Move the live subscriptions of `subscribers` to `expires_at`: they
    /// have an open listener. A subscription already lapsed stays lapsed.
    async fn extend_mcp_resource_subscriptions(
        &self,
        subscribers: &[String],
        now: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<u64, StoreError>;

    /// Delete every subscription lapsed by `now`.
    async fn reap_mcp_resource_subscriptions(&self, now: DateTime<Utc>) -> Result<u64, StoreError>;
}

#[async_trait]
pub trait OAuthCodeStore: Send + Sync {
    /// Persist a one-time OAuth authorization code. The plaintext code is never
    /// stored — `code_hash` is its SHA-256 digest.
    async fn insert_oauth_code(&self, new: NewOAuthCode) -> Result<(), StoreError>;

    /// Atomically consume (delete) a non-expired authorization code by hash.
    /// Returns `None` if the code is unknown, already consumed, or expired —
    /// guaranteeing single use across replicas.
    async fn consume_oauth_code(&self, code_hash: &str) -> Result<Option<OAuthCode>, StoreError>;
}

#[async_trait]
pub trait ReindexStore: Send + Sync {
    /// Insert or update an embedding reindex job. Keyed by `job_id`, so the
    /// start record and later status updates upsert the row, making job status
    /// visible on any replica.
    async fn upsert_reindex_job(&self, job: ReindexJob) -> Result<(), StoreError>;

    /// Fetch a reindex job by id, or `None` if unknown.
    async fn get_reindex_job(&self, job_id: uuid::Uuid) -> Result<Option<ReindexJob>, StoreError>;
}

#[async_trait]
pub trait TokenStore: Send + Sync {
    async fn create_api_token(&self, new: NewApiToken) -> Result<ApiToken, StoreError>;
    async fn get_api_token(&self, id: ApiTokenId) -> Result<ApiToken, StoreError>;
    async fn get_active_api_token_by_hash(&self, token_hash: &str) -> Result<ApiToken, StoreError>;
    /// Mint a token that records the token it was derived from, so
    /// [`Self::revoke_api_token`] can reach it.
    ///
    /// Separate from `create_api_token` rather than a field on `NewApiToken`:
    /// that struct is built at 109 sites, 100 of them tests, and only the
    /// attenuation path has a parent.
    async fn create_attenuated_api_token(
        &self,
        new: NewApiToken,
        parent_token_id: ApiTokenId,
    ) -> Result<ApiToken, StoreError>;
    /// Mint a short-lived token for a grant's subject. The store atomically
    /// verifies that the grant is live, belongs to the delegate/workspace, and
    /// outlives the token. `parent_token_id` links bearer exchanges into the
    /// existing revocation tree; sessions pass `None`.
    async fn create_delegated_api_token(
        &self,
        new: NewApiToken,
        grant_id: DelegationGrantId,
        delegate_id: MemberId,
        parent_token_id: Option<ApiTokenId>,
    ) -> Result<ApiToken, StoreError>;

    /// Revoke a token **and every token derived from it**.
    ///
    /// A derived token inherits the parent's app installation and quotas, because re-issuing was otherwise a way to shed a bound.
    /// Revocation is the ultimate limit and was the dimension still leaking.
    async fn revoke_api_token(&self, id: ApiTokenId) -> Result<ApiToken, StoreError>;

    // D-A (2026-09-23): changing authority writes its audit row inside the
    // change's own transaction, so a failed write aborts the change. Request
    // handlers use these; `authority_changes_are_audited_in_their_transaction`
    // fails if one calls the unaudited form above.

    /// [`Self::create_api_token`] with its audit row in the same transaction.
    async fn create_api_token_audited(
        &self,
        new: NewApiToken,
        audit: crate::AuditFor<ApiToken>,
    ) -> Result<ApiToken, StoreError>;
    /// [`Self::create_attenuated_api_token`] with its audit row in the same
    /// transaction.
    async fn create_attenuated_api_token_audited(
        &self,
        new: NewApiToken,
        parent_token_id: ApiTokenId,
        audit: crate::AuditFor<ApiToken>,
    ) -> Result<ApiToken, StoreError>;
    /// [`Self::create_delegated_api_token`] with its audit row in the same
    /// transaction.
    async fn create_delegated_api_token_audited(
        &self,
        new: NewApiToken,
        grant_id: DelegationGrantId,
        delegate_id: MemberId,
        parent_token_id: Option<ApiTokenId>,
        audit: crate::AuditFor<ApiToken>,
    ) -> Result<ApiToken, StoreError>;
    /// [`Self::revoke_api_token`] with its audit row in the same transaction.
    async fn revoke_api_token_audited(
        &self,
        id: ApiTokenId,
        audit: crate::AuditFor<ApiToken>,
    ) -> Result<ApiToken, StoreError>;
    /// Replace a live token's secret with `token_hash`: the successor keeps the
    /// old token's member, capabilities, label, expiry, app installation,
    /// parent and quotas, and inherits its derived tokens; the old token is
    /// revoked. `audit` is written for the successor in the same transaction.
    /// There is no unaudited form. A delegated token is a `Conflict`.
    async fn rotate_api_token_audited(
        &self,
        id: ApiTokenId,
        token_hash: &str,
        audit: crate::AuditFor<ApiToken>,
    ) -> Result<ApiToken, StoreError>;
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
}

#[async_trait]
pub trait ShareTicketStore: Send + Sync {
    /// Atomically create the ticket and its explicit artifact allowlist. The
    /// channel, owner, creator, and every artifact ref must belong to the same
    /// workspace. The plaintext secret never crosses this interface.
    async fn create_share_ticket(&self, new: NewShareTicket) -> Result<ShareTicket, StoreError>;
    async fn get_share_ticket(&self, id: ShareTicketId) -> Result<ShareTicket, StoreError>;
    async fn list_share_tickets(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<ShareTicket>, StoreError>;
    /// Resolve only an active ticket. Invalid, expired, and revoked hashes all
    /// return `NotFound`, so the consumer boundary does not disclose state.
    async fn resolve_share_ticket(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<ShareTicket, StoreError>;
    /// Revoke an active ticket in its workspace. Returns whether this call
    /// changed state.
    async fn revoke_share_ticket(
        &self,
        workspace_id: WorkspaceId,
        id: ShareTicketId,
    ) -> Result<bool, StoreError>;
    async fn list_share_ticket_artifacts(
        &self,
        id: ShareTicketId,
    ) -> Result<Vec<String>, StoreError>;
    /// Check the allowlist and ticket liveness in one query, closing the race
    /// between initial ticket resolution and an artifact fetch.
    async fn share_ticket_allows_artifact(
        &self,
        id: ShareTicketId,
        sha256: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, StoreError>;

    // D-A: the audited forms request handlers use (see `create_api_token_audited`).

    /// [`Self::create_share_ticket`] with its audit row in the same transaction.
    async fn create_share_ticket_audited(
        &self,
        new: NewShareTicket,
        audit: crate::AuditFor<ShareTicket>,
    ) -> Result<ShareTicket, StoreError>;
    /// [`Self::revoke_share_ticket`] with its audit row in the same transaction;
    /// nothing is recorded when there was no live ticket to revoke.
    async fn revoke_share_ticket_audited(
        &self,
        workspace_id: WorkspaceId,
        id: ShareTicketId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
}

#[async_trait]
pub trait DelegationGrantStore: Send + Sync {
    /// Persist a reviewable grant after validating its members, capability
    /// subset, purpose, and expiry. This storage foundation does not itself
    /// authorize requests or mint delegated credentials.
    async fn create_delegation_grant(
        &self,
        new: NewDelegationGrant,
    ) -> Result<DelegationGrant, StoreError>;
    async fn get_delegation_grant(
        &self,
        id: DelegationGrantId,
    ) -> Result<DelegationGrant, StoreError>;
    async fn list_delegation_grants(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<DelegationGrant>, StoreError>;
    /// Revoke a grant in its workspace. Returns whether this call changed it.
    async fn revoke_delegation_grant(
        &self,
        workspace_id: WorkspaceId,
        id: DelegationGrantId,
    ) -> Result<bool, StoreError>;

    // D-A: the audited forms request handlers use (see `create_api_token_audited`).

    /// [`Self::create_delegation_grant`] with its audit row in the same
    /// transaction.
    async fn create_delegation_grant_audited(
        &self,
        new: NewDelegationGrant,
        audit: crate::AuditFor<DelegationGrant>,
    ) -> Result<DelegationGrant, StoreError>;
    /// [`Self::revoke_delegation_grant`] with its audit row in the same
    /// transaction (written even when the grant was already revoked).
    async fn revoke_delegation_grant_audited(
        &self,
        workspace_id: WorkspaceId,
        id: DelegationGrantId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
}

#[async_trait]
pub trait PeerStore: Send + Sync {
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
        origin: &maidan_types::EventLink,
    ) -> Result<bool, StoreError>;
    /// Record the last origin link **verified** from this peer, whether or not
    /// the event was kept. Monotonic.
    async fn record_federated_verified_link(
        &self,
        peer_id: PeerId,
        link: &maidan_types::EventLink,
    ) -> Result<(), StoreError>;
    /// The last origin link verified from this peer, falling back to the last
    /// ingested one for a peer old enough to have none. This is what
    /// sequential ingest verify compares against — the *ingested* link is not
    /// it, because a policy-refused event still advances the chain.
    async fn last_federated_verified_link(
        &self,
        peer_id: PeerId,
    ) -> Result<Option<maidan_types::EventLink>, StoreError>;
    /// Last origin [`maidan_types::EventLink`] accepted from this peer. `None`
    /// if this peer has never ingested a hashed envelope.
    async fn last_federated_origin_link(
        &self,
        peer_id: PeerId,
    ) -> Result<Option<maidan_types::EventLink>, StoreError>;
    async fn is_federated_local_event(&self, local_event_id: i64) -> Result<bool, StoreError>;
}

#[async_trait]
pub trait DeliveryCursorStore: Send + Sync {
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

    /// Lowest `last_delivered_log_id` across the at-least-once delivery cursors
    /// that have advanced since `advanced_since`, or `None` when none has. An
    /// event at or below it has reached every live durable consumer and is
    /// safe to prune — the retention floor for the event log.
    ///
    /// A cursor that has not moved since then does not count. Pass the
    /// retention cutoff: a consumer that has not advanced in longer than the
    /// retention window is behind like any consumer offline that long, and
    /// gets `CursorTooOld` (must refetch) when it returns. Counting it would let
    /// one abandoned consumer id stop pruning forever. A consumer that is idle
    /// because it is caught up sits at the head, so excluding it prunes nothing
    /// it still needs.
    async fn min_delivery_cursor(
        &self,
        advanced_since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<i64>, StoreError>;

    /// Delete up to `limit` oldest event-log rows with `id <= max_id` **and**
    /// `occurred_at < cutoff`. The `max_id` floor (see
    /// [`Store::min_delivery_cursor`]) keeps events a lagging at-least-once
    /// consumer still needs. Returns the row count deleted; the caller loops
    /// until it is below `limit`.
    async fn prune_events(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        max_id: i64,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// Delete up to `limit` oldest audit rows with `occurred_at < cutoff`. A
    /// workspace under legal hold keeps its own rows; instance-level rows
    /// (no workspace) are never under a hold.
    async fn prune_audit(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// Delete up to `limit` read notifications with `created_at < cutoff`.
    /// Unread rows stay, and so does any row with a snooze set. A workspace
    /// under legal hold keeps its own rows.
    async fn prune_notifications(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// Delete up to `limit` of the oldest **finished** rows older than `cutoff`
    /// from each delivery table: delivered or quarantined webhook and
    /// automation deliveries, published outbox rows, delivered egress and
    /// mail, and dead-lettered agent runs. Pending rows are never pruned, nor
    /// are egress and mail dead letters (they wait for an operator), nor any
    /// row of a held workspace.
    async fn prune_deliveries(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// [`Self::prune_events`] for one workspace's own retention: up to `limit`
    /// of its events older than `cutoff`, floored at its own consumers'
    /// delivery cursors that advanced since `cutoff`. Nothing while the
    /// workspace is held.
    async fn prune_workspace_events(
        &self,
        workspace_id: WorkspaceId,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// Erase up to `limit` of one workspace's messages posted before `cutoff`,
    /// with their embeddings, references and content keys. Nothing while the
    /// workspace is held.
    async fn prune_workspace_messages(
        &self,
        workspace_id: WorkspaceId,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// Erase up to `limit` messages posted before `cutoff` in every workspace
    /// that is not under a legal hold. A held workspace keeps its messages.
    /// One page; the caller loops until a short page.
    async fn prune_messages(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;

    /// [`Self::prune_deliveries`] for one workspace's rows in every delivery
    /// table. Nothing while the workspace is held.
    async fn prune_workspace_deliveries(
        &self,
        workspace_id: WorkspaceId,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<u64, StoreError>;
}

#[async_trait]
pub trait WebhookStore: Send + Sync {
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
    /// (avoids scanning every workspace's subscriptions).
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
    /// Move a pending delivery's next attempt to `next_attempt_at` without
    /// recording an attempt or an error: the retry budget held it back, and it
    /// did not fail.
    async fn defer_webhook_delivery(
        &self,
        delivery_id: i64,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), StoreError>;
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
}

#[async_trait]
pub trait AutomationStore: Send + Sync {
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
    /// Move a pending delivery's next attempt to `next_attempt_at` without
    /// recording an attempt or an error: the retry budget held it back, and it
    /// did not fail.
    async fn defer_automation_delivery(
        &self,
        delivery_id: i64,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), StoreError>;
    async fn quarantine_automation_delivery(&self, delivery_id: i64) -> Result<(), StoreError>;
    async fn replay_automation_delivery(
        &self,
        delivery_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<AutomationDelivery, StoreError>;
}

#[async_trait]
pub trait SlashCommandStore: Send + Sync {
    async fn create_slash_command(&self, new: NewSlashCommand) -> Result<SlashCommand, StoreError>;
    async fn list_slash_commands(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SlashCommand>, StoreError>;
    async fn revoke_slash_command(
        &self,
        workspace_id: WorkspaceId,
        id: SlashCommandId,
    ) -> Result<SlashCommand, StoreError>;
    async fn get_slash_command(
        &self,
        id: SlashCommandId,
    ) -> Result<SlashCommandWithSecret, StoreError>;
    async fn get_slash_command_by_name(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<SlashCommandWithSecret, StoreError>;
}

#[async_trait]
pub trait FsmHookStore: Send + Sync {
    async fn create_fsm_hook(&self, new: NewFsmHook) -> Result<FsmHook, StoreError>;
    async fn list_fsm_hooks(&self, workspace_id: WorkspaceId) -> Result<Vec<FsmHook>, StoreError>;
    async fn revoke_fsm_hook(
        &self,
        workspace_id: WorkspaceId,
        id: FsmHookId,
    ) -> Result<FsmHook, StoreError>;
    async fn get_fsm_hook(&self, id: FsmHookId) -> Result<FsmHookWithSecret, StoreError>;
    async fn list_matching_fsm_hooks(
        &self,
        workspace_id: WorkspaceId,
        from_state: ThreadState,
        to_state: ThreadState,
    ) -> Result<Vec<FsmHookWithSecret>, StoreError>;
}

#[async_trait]
pub trait A2aStore: Send + Sync {
    async fn upsert_a2a_push_config(
        &self,
        workspace_id: WorkspaceId,
        push_url: &str,
    ) -> Result<(), StoreError>;
    async fn get_a2a_push_config(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<String>, StoreError>;

    /// Insert or replace a task. The row's `updated_at` is the task's
    /// status timestamp, so list order matches what the task reports.
    async fn upsert_a2a_task(&self, task: A2aTaskWrite<'_>) -> Result<(), StoreError>;
    async fn get_a2a_task(&self, task_id: &str) -> Result<Option<A2aTaskRow>, StoreError>;
    /// A workspace's tasks matching `query`, newest status first (ties by id
    /// descending), at most `query.limit`. `readable_by` filters in the
    /// query, so a page is `limit` readable tasks or the last of them.
    async fn list_a2a_tasks(
        &self,
        workspace_id: WorkspaceId,
        query: A2aTaskQuery<'_>,
    ) -> Result<Vec<A2aTaskRow>, StoreError>;
    /// How many of a workspace's tasks match `query`'s filters,
    /// `readable_by` included (`before` and `limit` are ignored).
    async fn count_a2a_tasks(
        &self,
        workspace_id: WorkspaceId,
        query: A2aTaskQuery<'_>,
    ) -> Result<i64, StoreError>;

    /// The thread a client-chosen A2A `contextId` names in a workspace.
    async fn get_a2a_context_thread(
        &self,
        workspace_id: WorkspaceId,
        context_id: &str,
    ) -> Result<Option<ThreadId>, StoreError>;
    /// Bind a client-chosen `contextId` to a thread. If a concurrent request
    /// bound it first, that binding wins and its thread is returned.
    async fn bind_a2a_context(
        &self,
        workspace_id: WorkspaceId,
        context_id: &str,
        thread_id: ThreadId,
    ) -> Result<ThreadId, StoreError>;

    /// Per-task A2A push notification configs (many per task, each with a
    /// stable `config_id`). Upsert by `(task_id, config_id)`.
    async fn upsert_a2a_task_push_config(
        &self,
        config: &A2aPushConfigRow,
    ) -> Result<(), StoreError>;
    async fn get_a2a_task_push_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> Result<Option<A2aPushConfigRow>, StoreError>;
    /// A task's configs, oldest first.
    async fn list_a2a_task_push_configs(
        &self,
        task_id: &str,
    ) -> Result<Vec<A2aPushConfigRow>, StoreError>;
    /// A keyset page of a task's configs in `config_id` order: at most
    /// `limit` whose id sorts after `after`.
    async fn page_a2a_task_push_configs(
        &self,
        task_id: &str,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<A2aPushConfigRow>, StoreError>;
    /// Returns `true` if a config was removed, `false` if none matched.
    async fn delete_a2a_task_push_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> Result<bool, StoreError>;
}

/// The full storage surface: the union of every domain sub-trait above.
///
/// Split from a single 258-method trait into cohesive sub-traits: `Store` is
/// now a marker super-trait, so `dyn Store` / `Arc<dyn Store>` still expose
/// every method, while a caller that needs only one concern can bound on the
/// narrower sub-trait (e.g. `impl ThreadStore`). The blanket impl means any
/// backend that implements all the sub-traits is automatically a `Store` — no
/// per-backend `impl Store` block.
/// D-A for governance and membership (413.4): the audited forms of the calls
/// that decide who may act, approve, or receive. Each writes its audit row in
/// the change's own transaction, so a failed write aborts the change.
#[async_trait]
pub trait GovernanceAuditStore: Send + Sync {
    /// Store a workspace secret (ciphertext only; the record names it).
    async fn create_secret_audited(
        &self,
        new: NewSecret,
        audit: crate::AuditFor<Secret>,
    ) -> Result<Secret, StoreError>;
    /// `true` when the secret existed; nothing is recorded otherwise.
    async fn delete_secret_audited(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// Trust a host with the workspace's secret values.
    async fn allow_secret_egress_host_audited(
        &self,
        new: NewSecretEgressHost,
        audit: crate::AuditFor<SecretEgressHost>,
    ) -> Result<SecretEgressHost, StoreError>;
    /// `true` when the host was listed; nothing is recorded otherwise.
    async fn revoke_secret_egress_host_audited(
        &self,
        workspace_id: WorkspaceId,
        host: &str,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// Create a SCIM user: the member and its link, together.
    async fn scim_provision_audited(
        &self,
        new: NewMember,
        external_id: Option<&str>,
        active: bool,
        audit: crate::AuditFor<(Member, ScimUser)>,
    ) -> Result<(Member, ScimUser), StoreError>;
    /// Update a SCIM user: rename it when `user_name` is given, and set its
    /// link's `external_id` and `active`. Deactivating revokes the member's
    /// live tokens and releases their claims (charging each claim's worked
    /// wall time) in the same transaction, each revocation recorded. `None` when the
    /// workspace has no such SCIM user; a `user_name` another member of the
    /// workspace holds is a [`StoreError::Conflict`].
    async fn scim_update_user_audited(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        user_name: Option<&str>,
        external_id: Option<&str>,
        active: bool,
        audit: NewAuditEvent,
    ) -> Result<Option<ScimUser>, StoreError>;
    /// Create a SCIM group with its members. A member that is not a SCIM user
    /// of the group's workspace is a [`StoreError::InvalidInput`], and nothing
    /// is written.
    async fn scim_create_group_audited(
        &self,
        new: NewScimGroup,
        audit: crate::AuditFor<ScimGroup>,
    ) -> Result<ScimGroup, StoreError>;
    /// Apply a change to a SCIM group, membership operations in order. `None`
    /// when the workspace has no such group; a member to add that is not a
    /// SCIM user of the workspace is a [`StoreError::InvalidInput`], and
    /// nothing is written.
    async fn scim_update_group_audited(
        &self,
        workspace_id: WorkspaceId,
        id: ScimGroupId,
        change: ScimGroupChange,
        audit: crate::AuditFor<ScimGroupWrite>,
    ) -> Result<Option<ScimGroupWrite>, StoreError>;
    /// Delete a SCIM group and its memberships. `false` when the workspace
    /// has no such group; nothing is recorded then.
    async fn scim_delete_group_audited(
        &self,
        workspace_id: WorkspaceId,
        id: ScimGroupId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// Revoke the member's live tokens and remove its SCIM link, together.
    /// `false` when there was no link.
    async fn scim_deprovision_audited(
        &self,
        workspace_id: WorkspaceId,
        member_id: MemberId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// Freeze a member and release their claims; the `MemberFrozen` event
    /// commits with the audit row.
    async fn freeze_member_audited(
        &self,
        member_id: MemberId,
        frozen_by: MemberId,
        reason: Option<&str>,
        audit: crate::AuditFor<(MemberFreeze, u64)>,
    ) -> Result<(MemberFreeze, u64, StoredEvent), StoreError>;
    /// The `MemberUnfrozen` event when the member was frozen; nothing is
    /// recorded or appended otherwise.
    async fn unfreeze_member_audited(
        &self,
        member_id: MemberId,
        unfrozen_by: MemberId,
        audit: NewAuditEvent,
    ) -> Result<Option<StoredEvent>, StoreError>;
    /// Add or re-role a channel member.
    async fn add_channel_member_audited(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        role: ChannelMemberRole,
        audit: crate::AuditFor<ChannelMember>,
    ) -> Result<ChannelMember, StoreError>;
    /// Remove a channel member.
    async fn remove_channel_member_audited(
        &self,
        channel_id: ChannelId,
        member_id: MemberId,
        audit: NewAuditEvent,
    ) -> Result<(), StoreError>;
    /// Set a thread's review requirement, returning the count it replaced. Without
    /// `allow_lower`, a write that would lower it is refused (`Conflict`) — decided
    /// in the write's transaction, so a concurrent change cannot turn a raise
    /// into a lowering.
    async fn set_review_requirement_audited(
        &self,
        thread_id: ThreadId,
        required_count: i64,
        allow_lower: bool,
        audit: crate::AuditFor<(i64, ThreadReviewRequirement)>,
    ) -> Result<(i64, ThreadReviewRequirement), StoreError>;
    /// `true` when a requirement existed; nothing is recorded otherwise.
    async fn clear_review_requirement_audited(
        &self,
        thread_id: ThreadId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// `true` when the member was a reviewer; nothing is recorded otherwise.
    async fn remove_reviewer_audited(
        &self,
        thread_id: ThreadId,
        member_id: MemberId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// `true` when a gate existed; nothing is recorded otherwise.
    async fn clear_land_gate_audited(
        &self,
        thread_id: ThreadId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// Grant a skill a gate reads as approval authority. Routing tags, which are
    /// personal state, use [`SkillStore::add_member_skill`].
    async fn grant_governance_skill_audited(
        &self,
        member_id: MemberId,
        skill: &str,
        audit: NewAuditEvent,
    ) -> Result<(), StoreError>;
    /// Allow a delivery destination.
    async fn allow_egress_target_audited(
        &self,
        new: NewEgressTarget,
        audit: crate::AuditFor<AllowedEgressTarget>,
    ) -> Result<AllowedEgressTarget, StoreError>;
    /// `true` when the target existed; nothing is recorded otherwise.
    async fn revoke_egress_target_audited(
        &self,
        workspace_id: WorkspaceId,
        id: EgressTargetId,
        audit: NewAuditEvent,
    ) -> Result<bool, StoreError>;
    /// Revoke an app installation and every token minted under it, together.
    async fn revoke_app_installation_audited(
        &self,
        id: AppInstallationId,
        audit: crate::AuditFor<AppInstallation>,
    ) -> Result<AppInstallation, StoreError>;
    /// Install `app_id` in `workspace_id` with `granted_capabilities`. The bot
    /// member of the app's latest revoked installation in that workspace is
    /// reused (same id, handle and history), so revoke-and-reinstall is how an
    /// installation's grants change; with none, an `app:<slug>` agent member is
    /// created in the same transaction. `Conflict` while an installation of
    /// the app is active; `NotFound` when the app is not in the workspace.
    async fn install_app_audited(
        &self,
        workspace_id: WorkspaceId,
        app_id: AppId,
        granted_capabilities: Vec<String>,
        audit: crate::AuditFor<crate::InstalledApp>,
    ) -> Result<crate::InstalledApp, StoreError>;
}

pub trait Store:
    MetaStore
    + WorkspaceStore
    + MemberStore
    + SkillStore
    + ThreadResultStore
    + ThreadSteerStore
    + ThreadLineageStore
    + BudgetStore
    + UsageLedgerStore
    + ApprovalGateStore
    + GlossaryStore
    + NotificationStore
    + FollowStore
    + MailStore
    + EgressStore
    + ProjectorLinkStore
    + PresenceDigestStore
    + SessionStore
    + ChannelStore
    + DmStore
    + ThreadStore
    + TaskScheduleStore
    + RecipeStore
    + SecretStore
    + MemberFreezeStore
    + MemoryBlockStore
    + ReviewStore
    + LandGateStore
    + SpawnBudgetStore
    + AssignmentStore
    + ThreadDepStore
    + MessageStore
    + MentionInboxStore
    + SocialStore
    + ReferenceStore
    + ArtifactMetaStore
    + EventStore
    + IntegrityStore
    + AppStore
    + OAuthCodeStore
    + IdempotencyStore
    + McpSubscriptionStore
    + ReindexStore
    + TokenStore
    + ShareTicketStore
    + DelegationGrantStore
    + PeerStore
    + DeliveryCursorStore
    + WebhookStore
    + AutomationStore
    + SlashCommandStore
    + FsmHookStore
    + A2aStore
    + GovernanceAuditStore
    + Send
    + Sync
{
}

impl<
        T: MetaStore
            + WorkspaceStore
            + MemberStore
            + SkillStore
            + ThreadResultStore
            + ThreadSteerStore
            + ThreadLineageStore
            + BudgetStore
            + UsageLedgerStore
            + ApprovalGateStore
            + GlossaryStore
            + NotificationStore
            + FollowStore
            + MailStore
            + EgressStore
            + ProjectorLinkStore
            + PresenceDigestStore
            + SessionStore
            + ChannelStore
            + DmStore
            + ThreadStore
            + TaskScheduleStore
            + RecipeStore
            + SecretStore
            + MemberFreezeStore
            + MemoryBlockStore
            + ReviewStore
            + LandGateStore
            + SpawnBudgetStore
            + AssignmentStore
            + ThreadDepStore
            + MessageStore
            + MentionInboxStore
            + SocialStore
            + ReferenceStore
            + ArtifactMetaStore
            + EventStore
            + IntegrityStore
            + AppStore
            + OAuthCodeStore
            + IdempotencyStore
            + McpSubscriptionStore
            + ReindexStore
            + TokenStore
            + ShareTicketStore
            + DelegationGrantStore
            + PeerStore
            + DeliveryCursorStore
            + WebhookStore
            + AutomationStore
            + SlashCommandStore
            + FsmHookStore
            + A2aStore
            + GovernanceAuditStore
            + Send
            + Sync,
    > Store for T
{
}
