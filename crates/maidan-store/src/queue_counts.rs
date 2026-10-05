//! A task queue's counts, [`QueueDepth`](maidan_types::QueueDepth) and
//! [`ChannelOccupancy`](maidan_types::ChannelOccupancy), for one channel or a
//! whole workspace, as one query that both backends and both scopes build.
//!
//! The workspace scope is the channel scope without its channel predicate, so
//! a workspace's counts are the sum of its channels' for the same reader. Both
//! count only the threads the reader may read, by `claim_next`'s read rule
//! ([`claim_next::readable_by`]): a private channel's threads for its members,
//! a DM's for its participants. Without a reader (a caller that bypasses auth)
//! every thread in scope counts. The shared `__dm__` channel passes the route's
//! channel check for every member, so before the read rule applied here its
//! counts were everyone's DMs.

use maidan_types::{ChannelId, WorkspaceId};

use crate::claim_next;

/// Which threads a count covers.
#[derive(Debug, Clone, Copy)]
pub(crate) enum QueueScope {
    Channel(ChannelId),
    Workspace(WorkspaceId),
}

impl QueueScope {
    /// The id the scope's placeholder is bound to.
    pub(crate) fn id(self) -> uuid::Uuid {
        match self {
            Self::Channel(id) => id.0,
            Self::Workspace(id) => id.0,
        }
    }
}

/// The parts of the query each backend spells its own way.
pub(crate) struct QueueSql<'a> {
    /// Placeholder bound to [`QueueScope::id`].
    pub scope: &'a str,
    /// Placeholder bound to the reader's member id, or NULL for no reader.
    pub reader: &'a str,
    /// Placeholder bound to [`maidan_types::DM_CHANNEL_NAME`].
    pub dm_channel: &'a str,
    /// The current time, compared with a lease deadline.
    pub now: &'a str,
}

/// `SELECT` of one row with `open_count`, `ready_count`, `assigned_count`,
/// `blocked_count` and `unclaimable_count`: the [`QueueDepth`](maidan_types::QueueDepth)
/// partition of the open threads in `scope`.
pub(crate) fn queue_depth_select(scope: QueueScope, sql: &QueueSql<'_>) -> String {
    let held = held(sql.now);
    let available = available(sql.now);
    format!(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN {held} THEN 1 ELSE 0 END), 0) AS assigned_count,
             COALESCE(SUM(CASE WHEN {available} AND NOT {PARKED}
                       AND NOT {BLOCKED} AND NOT {WAITING}
                     THEN 1 ELSE 0 END), 0) AS ready_count,
             COALESCE(SUM(CASE WHEN {available} AND NOT {PARKED}
                       AND ({BLOCKED} OR {WAITING})
                     THEN 1 ELSE 0 END), 0) AS blocked_count,
             COALESCE(SUM(CASE WHEN {available} AND {PARKED}
                     THEN 1 ELSE 0 END), 0) AS unclaimable_count
         {}",
        open_threads(scope, sql)
    )
}

/// `SELECT` of one row with `open_count`, `queued_count`, `claimed_count`,
/// `working_count` and `blocked_count`: the
/// [`ChannelOccupancy`](maidan_types::ChannelOccupancy) partition of the open
/// threads in `scope`.
pub(crate) fn occupancy_select(scope: QueueScope, sql: &QueueSql<'_>) -> String {
    let held = held(sql.now);
    let available = available(sql.now);
    format!(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN {held} AND cand.work_started_at IS NULL
                     THEN 1 ELSE 0 END), 0) AS claimed_count,
             COALESCE(SUM(CASE WHEN {held} AND cand.work_started_at IS NOT NULL
                     THEN 1 ELSE 0 END), 0) AS working_count,
             COALESCE(SUM(CASE WHEN {available} AND NOT {BLOCKED} AND NOT {WAITING}
                     THEN 1 ELSE 0 END), 0) AS queued_count,
             COALESCE(SUM(CASE WHEN {available} AND ({BLOCKED} OR {WAITING})
                     THEN 1 ELSE 0 END), 0) AS blocked_count
         {}",
        open_threads(scope, sql)
    )
}

/// `FROM … WHERE` of the open, live threads in `scope` the reader may read,
/// aliased `cand` as [`claim_next::readable_by`] expects, with their channel
/// `ch`.
fn open_threads(scope: QueueScope, sql: &QueueSql<'_>) -> String {
    let QueueSql {
        scope: scope_id,
        reader,
        dm_channel,
        ..
    } = sql;
    let in_scope = match scope {
        QueueScope::Channel(_) => format!("cand.channel_id = {scope_id}"),
        QueueScope::Workspace(_) => format!("ch.workspace_id = {scope_id}"),
    };
    let readable = claim_next::readable_by(reader, dm_channel);
    format!(
        "FROM maidan_threads cand
         JOIN maidan_channels ch ON ch.id = cand.channel_id
         WHERE {in_scope} AND cand.state = 'open' AND cand.tombstoned_at IS NULL
           AND ({reader} IS NULL OR ({readable}))"
    )
}

/// Held on a live lease, or held with no lease at all.
fn held(now: &str) -> String {
    format!(
        "(cand.assignee_id IS NOT NULL
          AND (cand.assignment_expires_at IS NULL OR cand.assignment_expires_at >= {now}))"
    )
}

/// Unheld, or held on a lease that lapsed: what `claim_next` may take over.
fn available(now: &str) -> String {
    format!(
        "(cand.assignee_id IS NULL
          OR (cand.assignment_expires_at IS NOT NULL AND cand.assignment_expires_at < {now}))"
    )
}

const PARKED: &str =
    "EXISTS (SELECT 1 FROM maidan_thread_unclaimable u WHERE u.thread_id = cand.id)";
const BLOCKED: &str = "EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = cand.id)";
const WAITING: &str = "EXISTS (SELECT 1 FROM maidan_thread_dependencies d
                        JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                        WHERE d.thread_id = cand.id AND dep.state NOT IN ('closed', 'archived'))";
