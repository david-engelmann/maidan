//! Postgres implementation of [`crate::store::Store`].

mod a2a;
mod approval_gates;
mod apps;
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
mod priorities;
mod purge_workspace;
mod push_subscriptions;
mod reactions;
mod recipes;
mod refs;
mod reindex_jobs;
pub mod replication;
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_types::*;
use sqlx::PgPool;

use crate::a2a::{A2aPushConfigRow, A2aTaskQuery, A2aTaskRow, A2aTaskWrite, PendingGateQuery};
use crate::claim_next::ClaimScope;
use crate::error::StoreError;
use crate::store::*;

tokio::task_local! {
    /// The read-consistency scope for the current request. Present only inside
    /// a [`with_read_consistency`] scope (set by the server for GET/HEAD
    /// requests); `Some(lsn)` carries the client's causality token, `None`
    /// means the request has no causality requirement. Absent (not in scope —
    /// mutation handlers, background workers) routes reads to the primary.
    static READ_CONSISTENCY: Option<Lsn>;
}

/// Run `fut` with the request's read-consistency scope so `PostgresStore`'s
/// reads can route to a replica. The server wraps GET/HEAD handling in this
/// (with the parsed `Maidan-Consistency-Token`, or `None`); everything outside
/// a scope reads from the primary.
pub async fn with_read_consistency<F>(token: Option<Lsn>, fut: F) -> F::Output
where
    F: std::future::Future,
{
    READ_CONSISTENCY.scope(token, fut).await
}

#[derive(Debug, Clone)]
pub struct PostgresStore {
    pool: PgPool,
    /// Read pool for LSN-token read routing. Defaults to a clone of the writer
    /// `pool`; a real read-replica is supplied via
    /// [`PostgresStore::with_replica_reader`].
    reader: PgPool,
    /// Whether `reader` is a genuine replica (else it aliases `pool`). When false,
    /// [`PostgresStore::read_pool`] always returns the primary.
    has_replica: bool,
    /// The replica's last-known replay LSN as a raw `u64`, refreshed by a
    /// background poller. `read_pool` compares a request's causality token
    /// against this — a cheap atomic load, no per-read query. A stale value is
    /// only ever *behind* the true replay position, so it can only route to the
    /// primary unnecessarily, never serve a stale read.
    replica_replay: Arc<AtomicU64>,
    /// How many reads `read_pool` sent to the replica vs the primary, for the
    /// `maidan_replica_reads_total` metric. Counted only when a replica is
    /// configured (a single-pool store leaves it at zero).
    read_routing: Arc<ReadRoutingMetrics>,
    replica_health: Arc<ReplicaHealth>,
    /// Wraps the per-message content keys (crypto-shredding).
    keys: Arc<ContentKeyring>,
}

/// Cumulative read-routing outcomes. The server snapshots this into
/// `maidan_replica_reads_total{outcome}`; the store stays metrics-agnostic (the
/// `HydrateStats` pattern).
#[derive(Debug, Default)]
pub struct ReadRoutingMetrics {
    primary: AtomicU64,
    replica: AtomicU64,
    /// Replica lag in WAL bytes (primary write LSN − replica replay LSN),
    /// refreshed by the poller. `0` when caught up / not yet sampled.
    replica_lag_bytes: AtomicU64,
}

impl ReadRoutingMetrics {
    /// `(primary, replica)` cumulative read counts.
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.primary.load(Ordering::Relaxed),
            self.replica.load(Ordering::Relaxed),
        )
    }

    /// Current replica lag in WAL bytes, for `maidan_replica_lag_bytes`.
    pub fn lag_bytes(&self) -> u64 {
        self.replica_lag_bytes.load(Ordering::Relaxed)
    }
}

/// How often the background poller refreshes the cached replica replay LSN.
const REPLICA_LSN_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// A replica not successfully polled for this long is treated as down.
const REPLICA_STALE_AFTER: Duration = Duration::from_secs(2);

/// A replica more than this many WAL bytes behind the primary stops serving
/// reads that carry no consistency token. A token read is safe at any lag —
/// the cached replay LSN proves it has the write — but a read with no token is
/// served whatever the replica holds, and past this it is serving the past.
pub const REPLICA_MAX_LAG_BYTES: u64 = 64 * 1024 * 1024;

/// Whether the replica may serve reads. Before this, no-token reads went to the
/// replica however far behind it was, and a poller that could no longer reach
/// it left the cache stale and reads still routed there.
#[derive(Debug, Default)]
pub struct ReplicaHealth {
    /// Unix millis of the last successful poll; `0` before the first.
    last_good_poll_ms: AtomicU64,
    lagging: std::sync::atomic::AtomicBool,
}

impl ReplicaHealth {
    /// Record a successful poll, with the lag when the poller could measure it.
    pub fn record_good_poll(&self, lag_bytes: Option<u64>) {
        self.last_good_poll_ms
            .store(unix_millis(), Ordering::Relaxed);
        if let Some(lag) = lag_bytes {
            self.lagging
                .store(lag > REPLICA_MAX_LAG_BYTES, Ordering::Relaxed);
        }
    }

    /// Polled recently, and not lagging past [`REPLICA_MAX_LAG_BYTES`].
    pub fn is_healthy(&self) -> bool {
        let last = self.last_good_poll_ms.load(Ordering::Relaxed);
        let fresh = last != 0
            && unix_millis().saturating_sub(last) <= REPLICA_STALE_AFTER.as_millis() as u64;
        fresh && !self.lagging.load(Ordering::Relaxed)
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl PostgresStore {
    /// Single-pool store: reads and writes both use `pool` (reader aliases it).
    /// `keys` wraps and unwraps message content keys; there is no default.
    pub fn new(pool: PgPool, keys: Arc<ContentKeyring>) -> Self {
        Self {
            reader: pool.clone(),
            pool,
            has_replica: false,
            replica_replay: Arc::new(AtomicU64::new(0)),
            read_routing: Arc::new(ReadRoutingMetrics::default()),
            replica_health: Arc::new(ReplicaHealth::default()),
            keys,
        }
    }

    /// Single-pool store under the public development KEK, for tests only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(pool: PgPool) -> Self {
        Self::new(pool, crate::test_support::dev_keys())
    }

    /// Store with a distinct read-replica pool. Writes use `pool`;
    /// token-eligible reads route to `reader`. Spawns a background poller that
    /// keeps the replica's replay LSN cached for cheap per-read routing
    /// decisions.
    pub fn with_replica_reader(pool: PgPool, reader: PgPool, keys: Arc<ContentKeyring>) -> Self {
        let replica_replay = Arc::new(AtomicU64::new(0));
        let read_routing = Arc::new(ReadRoutingMetrics::default());
        let replica_health = Arc::new(ReplicaHealth::default());
        spawn_replica_lsn_poller(
            pool.clone(),
            reader.clone(),
            replica_replay.clone(),
            read_routing.clone(),
            replica_health.clone(),
        );
        Self {
            pool,
            reader,
            has_replica: true,
            replica_replay,
            read_routing,
            replica_health,
            keys,
        }
    }

    pub fn content_keys(&self) -> &Arc<ContentKeyring> {
        &self.keys
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn reader(&self) -> &PgPool {
        &self.reader
    }

    /// Read-routing counters for the `maidan_replica_reads_total` metric.
    pub fn read_routing_metrics(&self) -> Arc<ReadRoutingMetrics> {
        self.read_routing.clone()
    }

    /// The pool a read should use, honoring the current request's
    /// read-consistency scope:
    /// - no replica, or not inside a [`with_read_consistency`] scope (mutation
    ///   handlers, background workers) → the **primary** (safe default);
    /// - in scope with no token → the **replica** (no causality requirement);
    /// - in scope with a token → the **replica** iff its cached replay LSN has
    ///   reached the token, else the **primary** (read-your-writes).
    fn read_pool(&self) -> &PgPool {
        let cached = Lsn(self.replica_replay.load(Ordering::Relaxed));
        let decision = route_now(self.has_replica, self.replica_health.is_healthy(), cached);
        if self.has_replica {
            let counter = match decision {
                RouteDecision::Replica => &self.read_routing.replica,
                RouteDecision::Primary => &self.read_routing.primary,
            };
            counter.fetch_add(1, Ordering::Relaxed);
        }
        match decision {
            RouteDecision::Replica => &self.reader,
            RouteDecision::Primary => &self.pool,
        }
    }

    /// WAL position of the primary. `write_lsn` is shared with SQLite, which
    /// has no such position and returns `None`.
    async fn current_wal_lsn(&self) -> Result<Option<Lsn>, StoreError> {
        Ok(Some(replication::current_wal_lsn(&self.pool).await?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteDecision {
    Primary,
    Replica,
}

/// Read the current request's read-consistency scope from the task-local and apply
/// [`route_decision`]. Shared by [`PostgresStore::read_pool`] and exposed as a bool
/// via [`replica_route`] for read pools outside this struct.
fn route_now(has_replica: bool, replica_healthy: bool, cached_replay: Lsn) -> RouteDecision {
    let scope = READ_CONSISTENCY.try_with(|t| *t).ok();
    route_decision(has_replica && replica_healthy, scope, cached_replay)
}

/// Whether a read keyed on the **current request's** consistency scope should
/// go to a replica (`true`) or the primary (`false`) — the same task-local +
/// routing logic [`PostgresStore::read_pool`] uses, exposed so another read
/// pool (maidan-search's `PostgresSearch`) can honor the same
/// `Maidan-Consistency-Token` without duplicating the decision or re-reading
/// the task-local. `cached_replay` is that pool's own cached replica replay
/// LSN.
pub fn replica_route(has_replica: bool, replica_healthy: bool, cached_replay: Lsn) -> bool {
    matches!(
        route_now(has_replica, replica_healthy, cached_replay),
        RouteDecision::Replica
    )
}

/// Pure read-routing decision, factored out for unit testing:
/// - no replica, or not inside a read-consistency scope (`scope == None`) → primary;
/// - in scope with no token (`Some(None)`) → replica (no causality need);
/// - in scope with a token (`Some(Some(t))`) → replica iff the cached replay LSN has
///   reached `t`, else primary (read-your-writes). A cached LSN is only ever behind
///   the true replay position, so `cached >= t` guarantees the replica is caught up.
fn route_decision(
    has_replica: bool,
    scope: Option<Option<Lsn>>,
    cached_replay: Lsn,
) -> RouteDecision {
    if !has_replica {
        return RouteDecision::Primary;
    }
    match scope {
        None => RouteDecision::Primary,
        Some(None) => RouteDecision::Replica,
        Some(Some(token)) => {
            if cached_replay >= token {
                RouteDecision::Replica
            } else {
                RouteDecision::Primary
            }
        }
    }
}

/// Poll the replica's `pg_last_wal_replay_lsn()` into `replay_cache` on a fixed
/// cadence, so [`PostgresStore::read_pool`] can decide primary-vs-replica
/// without a per-read query, and refresh the replica-lag gauge from the
/// primary's current write LSN minus the replay position. A poll error /
/// non-standby result leaves the cache unchanged low → reads route to the
/// primary until the next good poll (fail-safe).
fn spawn_replica_lsn_poller(
    primary: PgPool,
    reader: PgPool,
    replay_cache: Arc<AtomicU64>,
    metrics: Arc<ReadRoutingMetrics>,
    health: Arc<ReplicaHealth>,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REPLICA_LSN_POLL_INTERVAL);
        loop {
            tick.tick().await;
            if let Ok(Some(replay)) = replication::replica_replay_lsn(&reader).await {
                replay_cache.store(replay.0, Ordering::Relaxed);
                let lag = match replication::current_wal_lsn(&primary).await {
                    Ok(current) => {
                        let lag = current.0.saturating_sub(replay.0);
                        metrics.replica_lag_bytes.store(lag, Ordering::Relaxed);
                        Some(lag)
                    }
                    Err(_) => None,
                };
                health.record_good_poll(lag);
            }
        }
    });
}

store_delegations!(PostgresStore, MetaStore);
store_delegations!(PostgresStore, WorkspaceStore);
store_delegations!(PostgresStore, MemberStore);
store_delegations!(PostgresStore, SkillStore);
store_delegations!(PostgresStore, ThreadResultStore);
store_delegations!(PostgresStore, ThreadSteerStore);
store_delegations!(PostgresStore, ThreadLineageStore);
store_delegations!(PostgresStore, BudgetStore);
store_delegations!(PostgresStore, UsageLedgerStore);
store_delegations!(PostgresStore, ApprovalGateStore);
store_delegations!(PostgresStore, GlossaryStore);
store_delegations!(PostgresStore, NotificationStore);
store_delegations!(PostgresStore, FollowStore);
store_delegations!(PostgresStore, MailStore);
store_delegations!(PostgresStore, EgressStore);
store_delegations!(PostgresStore, ProjectorLinkStore);
store_delegations!(PostgresStore, PresenceDigestStore);
store_delegations!(PostgresStore, SessionStore);
store_delegations!(PostgresStore, ChannelStore);
store_delegations!(PostgresStore, DmStore);
store_delegations!(PostgresStore, ThreadStore);
store_delegations!(PostgresStore, TaskScheduleStore);
store_delegations!(PostgresStore, RecipeStore);
store_delegations!(PostgresStore, SecretStore);
store_delegations!(PostgresStore, SpawnBudgetStore);
store_delegations!(PostgresStore, ReviewStore);
store_delegations!(PostgresStore, LandGateStore);
store_delegations!(PostgresStore, MemoryBlockStore);
store_delegations!(PostgresStore, MemberFreezeStore);
store_delegations!(PostgresStore, AssignmentStore);
store_delegations!(PostgresStore, ThreadDepStore);
store_delegations!(PostgresStore, MessageStore);
store_delegations!(PostgresStore, MentionInboxStore);
store_delegations!(PostgresStore, SocialStore);
store_delegations!(PostgresStore, ReferenceStore);
store_delegations!(PostgresStore, ArtifactMetaStore);
store_delegations!(PostgresStore, EventStore);
store_delegations!(PostgresStore, IntegrityStore);
store_delegations!(PostgresStore, AppStore);
store_delegations!(PostgresStore, McpSubscriptionStore);
store_delegations!(PostgresStore, IdempotencyStore);
store_delegations!(PostgresStore, OAuthCodeStore);
store_delegations!(PostgresStore, ReindexStore);
store_delegations!(PostgresStore, TokenStore);
store_delegations!(PostgresStore, ShareTicketStore);
store_delegations!(PostgresStore, DelegationGrantStore);
store_delegations!(PostgresStore, PeerStore);
store_delegations!(PostgresStore, DeliveryCursorStore);
store_delegations!(PostgresStore, WebhookStore);
store_delegations!(PostgresStore, AutomationStore);
store_delegations!(PostgresStore, SlashCommandStore);
store_delegations!(PostgresStore, FsmHookStore);
store_delegations!(PostgresStore, A2aStore);
store_delegations!(PostgresStore, GovernanceAuditStore);

#[cfg(test)]
mod route_tests {
    use super::{
        replica_route, route_decision, ReplicaHealth, RouteDecision, READ_CONSISTENCY,
        REPLICA_MAX_LAG_BYTES,
    };
    use maidan_types::Lsn;

    #[test]
    fn no_replica_always_primary() {
        for scope in [None, Some(None), Some(Some(Lsn(10)))] {
            assert_eq!(
                route_decision(false, scope, Lsn(u64::MAX)),
                RouteDecision::Primary
            );
        }
    }

    #[test]
    fn outside_a_read_scope_uses_primary() {
        // Mutation handlers / background workers are not in a read scope.
        assert_eq!(
            route_decision(true, None, Lsn(u64::MAX)),
            RouteDecision::Primary
        );
    }

    #[test]
    fn in_scope_without_a_token_uses_replica() {
        assert_eq!(
            route_decision(true, Some(None), Lsn(0)),
            RouteDecision::Replica
        );
    }

    #[test]
    fn token_routes_to_replica_only_once_caught_up() {
        let token = Lsn(100);
        // Replica behind the token -> primary (read-your-writes).
        assert_eq!(
            route_decision(true, Some(Some(token)), Lsn(99)),
            RouteDecision::Primary
        );
        // Replica exactly at / past the token -> replica.
        assert_eq!(
            route_decision(true, Some(Some(token)), Lsn(100)),
            RouteDecision::Replica
        );
        assert_eq!(
            route_decision(true, Some(Some(token)), Lsn(101)),
            RouteDecision::Replica
        );
    }

    #[test]
    fn a_replica_never_polled_is_not_healthy() {
        assert!(!ReplicaHealth::default().is_healthy());
    }

    #[test]
    fn a_polled_replica_within_the_lag_bound_is_healthy() {
        let health = ReplicaHealth::default();
        health.record_good_poll(Some(REPLICA_MAX_LAG_BYTES));
        assert!(health.is_healthy());
        // A poll that could not measure lag keeps the last lag verdict.
        health.record_good_poll(None);
        assert!(health.is_healthy());
    }

    #[test]
    fn a_replica_past_the_lag_bound_stops_serving() {
        let health = ReplicaHealth::default();
        health.record_good_poll(Some(REPLICA_MAX_LAG_BYTES + 1));
        assert!(!health.is_healthy());
        health.record_good_poll(Some(0));
        assert!(health.is_healthy(), "recovering clears it");
    }

    /// An unhealthy replica serves nothing, including reads whose token it has
    /// already replayed: a replica that cannot be polled may not be reachable.
    #[test]
    fn an_unhealthy_replica_routes_every_read_to_the_primary() {
        // Inside a read scope — outside one, every read is primary regardless.
        for scope in [None, Some(Lsn(10))] {
            READ_CONSISTENCY.sync_scope(scope, || {
                assert!(replica_route(true, true, Lsn(u64::MAX)), "healthy serves");
                assert!(
                    !replica_route(true, false, Lsn(u64::MAX)),
                    "unhealthy serves nothing, token or not"
                );
            });
        }
    }
}
