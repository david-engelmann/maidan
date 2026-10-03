//! SQLite implementation of [`crate::store::Store`]. Mirrors the Postgres
//! impl 1:1, with SQLite-flavored SQL (`?` placeholders, TEXT-encoded
//! UUIDs and timestamps, no `RETURNING` for the audit table's
//! AUTOINCREMENT id — we read it back via `last_insert_rowid()`).

mod a2a;
mod approval_gates;
pub mod apps;
mod artifacts;
mod audit;
mod automation_deliveries;
mod blocks;
mod budget;
mod channel_members;
mod channels;
mod content_keys;
mod data_audited;
mod delegation_grants;
pub mod delivery_cursor;
mod dlq;
mod dm;
mod egress_outbox;
mod egress_targets;
mod email_digest;
mod erase_workspace;
pub mod events;
mod explorer;
mod follows;
mod fsm_hooks;
mod github_links;
mod glossary;
mod governance_audited;
mod group_dm;
mod idempotency;
mod import;
mod inbox;
mod land_gate;
mod legal_hold;
mod mail_outbox;
mod mcp_subscriptions;
mod member_emails;
mod member_freezes;
mod member_last_seen;
mod member_skills;
mod members;
mod memory_blocks;
mod mentions;
mod message_edits;
mod messages;
mod notification_prefs;
mod notifications;
mod oauth_codes;
mod oidc;
pub mod outbox;
mod peers;
mod pins;
mod pragmas;
mod priorities;
mod purge_workspace;
mod push_subscriptions;
mod reactions;
mod recipes;
mod refs;
mod reindex_jobs;
mod result_deliveries;
mod retention;
mod retention_policy;
mod reviews;
mod scim_audited;
mod scim_groups;
mod scim_users;
mod secret_egress_hosts;
mod secrets;
mod sessions;
mod share_tickets;
mod slack_links;
mod slash_commands;
mod spawn;
mod tap_cursor;
mod task_schedules;
mod thread_deps;
mod thread_lineage;
mod thread_results;
mod thread_skills;
mod thread_steer;
mod thread_transitions;
mod thread_workers;
mod threads;
mod token_quotas;
mod tokens;
mod unclaimable;
mod usage_ledger;
mod votes;
mod waits;
mod web_push_outbox;
mod webhooks;
mod wip;
mod workspace_handles;
mod workspaces;

use chrono::{DateTime, Utc};
use maidan_types::*;
use sqlx::SqlitePool;

use std::sync::Arc;

use crate::a2a::{A2aPushConfigRow, A2aTaskQuery, A2aTaskRow, A2aTaskWrite, PendingGateQuery};
use crate::claim_next::ClaimScope;
use crate::error::StoreError;
use crate::store::*;

pub use pragmas::{configure_pool, configure_pool_with};

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
    /// Wraps the per-message content keys (crypto-shredding).
    keys: Arc<ContentKeyring>,
}

impl SqliteStore {
    /// Store that wraps and unwraps message content keys with `keys`. There is
    /// no default: a caller that means the public development KEK says so.
    pub fn new(pool: SqlitePool, keys: Arc<ContentKeyring>) -> Self {
        Self { pool, keys }
    }

    /// Store under the public development KEK, for tests only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(pool: SqlitePool) -> Self {
        Self::new(pool, crate::test_support::dev_keys())
    }

    pub fn content_keys(&self) -> &Arc<ContentKeyring> {
        &self.keys
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// SQLite has one pool. The shared delegations call this where Postgres
    /// would send the read to a replica.
    fn read_pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// SQLite has no WAL position a replica could catch up to.
    async fn current_wal_lsn(&self) -> Result<Option<Lsn>, StoreError> {
        Ok(None)
    }
}

store_delegations!(SqliteStore, MetaStore);
store_delegations!(SqliteStore, WorkspaceStore);
store_delegations!(SqliteStore, MemberStore);
store_delegations!(SqliteStore, SkillStore);
store_delegations!(SqliteStore, ThreadResultStore);
store_delegations!(SqliteStore, ThreadSteerStore);
store_delegations!(SqliteStore, ThreadLineageStore);
store_delegations!(SqliteStore, BudgetStore);
store_delegations!(SqliteStore, UsageLedgerStore);
store_delegations!(SqliteStore, ApprovalGateStore);
store_delegations!(SqliteStore, GlossaryStore);
store_delegations!(SqliteStore, NotificationStore);
store_delegations!(SqliteStore, FollowStore);
store_delegations!(SqliteStore, MailStore);
store_delegations!(SqliteStore, EgressStore);
store_delegations!(SqliteStore, ProjectorLinkStore);
store_delegations!(SqliteStore, PresenceDigestStore);
store_delegations!(SqliteStore, SessionStore);
store_delegations!(SqliteStore, ChannelStore);
store_delegations!(SqliteStore, DmStore);
store_delegations!(SqliteStore, ThreadStore);
store_delegations!(SqliteStore, TaskScheduleStore);
store_delegations!(SqliteStore, RecipeStore);
store_delegations!(SqliteStore, SecretStore);
store_delegations!(SqliteStore, SpawnBudgetStore);
store_delegations!(SqliteStore, ReviewStore);
store_delegations!(SqliteStore, LandGateStore);
store_delegations!(SqliteStore, MemoryBlockStore);
store_delegations!(SqliteStore, MemberFreezeStore);
store_delegations!(SqliteStore, AssignmentStore);
store_delegations!(SqliteStore, ThreadDepStore);
store_delegations!(SqliteStore, MessageStore);
store_delegations!(SqliteStore, MentionInboxStore);
store_delegations!(SqliteStore, SocialStore);
store_delegations!(SqliteStore, ReferenceStore);
store_delegations!(SqliteStore, ArtifactMetaStore);
store_delegations!(SqliteStore, EventStore);
store_delegations!(SqliteStore, IntegrityStore);
store_delegations!(SqliteStore, AppStore);
store_delegations!(SqliteStore, McpSubscriptionStore);
store_delegations!(SqliteStore, IdempotencyStore);
store_delegations!(SqliteStore, OAuthCodeStore);
store_delegations!(SqliteStore, ReindexStore);
store_delegations!(SqliteStore, TokenStore);
store_delegations!(SqliteStore, ShareTicketStore);
store_delegations!(SqliteStore, DelegationGrantStore);
store_delegations!(SqliteStore, PeerStore);
store_delegations!(SqliteStore, DeliveryCursorStore);
store_delegations!(SqliteStore, WebhookStore);
store_delegations!(SqliteStore, AutomationStore);
store_delegations!(SqliteStore, SlashCommandStore);
store_delegations!(SqliteStore, FsmHookStore);
store_delegations!(SqliteStore, A2aStore);
store_delegations!(SqliteStore, GovernanceAuditStore);
