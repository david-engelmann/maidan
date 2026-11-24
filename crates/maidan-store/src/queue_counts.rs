//! A task queue's counts, [`QueueDepth`](maidan_types::QueueDepth) and
//! [`ChannelOccupancy`](maidan_types::ChannelOccupancy), for one channel or a
//! whole workspace, as one query that both backends and both scopes build.
//!
//! The workspace scope is the channel scope without its channel predicate, so
//! a workspace's counts are the sum of its channels' for the same reader. Both
//! count only the threads the reader may read, by `claim_next`'s read rule
//! ([`claim_next::readable_by`]): a private channel's threads for its members,
//! a DM's for its participants. Without a reader every thread in scope counts.
//! The shared `__dm__` channel passes the route's channel check for every
//! member, so before the read rule applied here its counts were everyone's DMs.
//!
//! Read for a claimer ([`maidan_types::QueueClaimer`]), the reader is also the
//! claimer, and every count narrows to the threads `claim_next` could hand it
//! ([`claim_next::takes`]), so its `ready` is what `claim_next` would hand it.
//! A thread held warm for a sibling still counts: that hold lasts minutes.

use maidan_types::{ChannelId, WorkspaceId};

use crate::claim_next;
use crate::claimability::{self, ThreadSql};

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
    /// Placeholder bound to the reader's member id. Required: there is no
    /// reader-less count.
    pub reader: &'a str,
    /// Placeholder bound to [`maidan_types::DM_CHANNEL_NAME`].
    pub dm_channel: &'a str,
    /// The current time, compared with a lease deadline.
    pub now: &'a str,
    /// Placeholder bound to a normalized repository, or NULL for any work:
    /// [`claim_next::served_by`]'s `repo`.
    pub repo: &'a str,
    /// Placeholder bound to an engine, or NULL for any work:
    /// [`claim_next::served_by`]'s `engine`.
    pub engine: &'a str,
    /// Placeholder bound to a kind of work, or NULL for any kind:
    /// [`claim_next::served_by`]'s `kind`.
    pub kind: &'a str,
    /// [`ThreadSql::lapsed_worked_secs`].
    pub lapsed_worked_secs: &'a str,
    /// [`ThreadSql::requeued_after_result`].
    pub requeued_after_result: &'a str,
    /// [`claim_next::ClaimSql::later_than`].
    pub later_than: &'a str,
    /// Placeholder bound to the app's `now`, as
    /// [`claim_next::ClaimSql::clock`]. Only a count read as a claimer
    /// names it, so only that read binds it.
    pub clock: &'a str,
}

impl<'a> QueueSql<'a> {
    fn thread(&self) -> ThreadSql<'a> {
        ThreadSql {
            now: self.now,
            lapsed_worked_secs: self.lapsed_worked_secs,
            requeued_after_result: self.requeued_after_result,
        }
    }
}

/// `SELECT` of one row with `open_count`, `ready_count`, `assigned_count`,
/// `blocked_count` and `unclaimable_count`: the [`QueueDepth`](maidan_types::QueueDepth)
/// partition of the open threads in `scope`, by [`claimability`]. `ready` is
/// exactly what `claim_next` would hand out to a claimer with no restriction.
/// `unclaimable` is withheld work (parked, out of budget, finished), and
/// `blocked` is stalled work (a block, a dependency, a gate, a footprint
/// conflict). With `as_claimer`, only the threads the reader could be handed
/// count, as the module says.
pub(crate) fn queue_depth_select(
    scope: QueueScope,
    sql: &QueueSql<'_>,
    as_claimer: bool,
) -> String {
    let t = sql.thread();
    let held = claimability::held(sql.now);
    let available = claimability::available(sql.now);
    let withheld = claimability::withheld(&t);
    let stalled = claimability::stalled(sql.now);
    format!(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN {held} THEN 1 ELSE 0 END), 0) AS assigned_count,
             COALESCE(SUM(CASE WHEN {available} AND NOT {withheld} AND NOT {stalled}
                     THEN 1 ELSE 0 END), 0) AS ready_count,
             COALESCE(SUM(CASE WHEN {available} AND NOT {withheld} AND {stalled}
                     THEN 1 ELSE 0 END), 0) AS blocked_count,
             COALESCE(SUM(CASE WHEN {available} AND {withheld}
                     THEN 1 ELSE 0 END), 0) AS unclaimable_count
         {}",
        open_threads(scope, sql, as_claimer)
    )
}

/// `SELECT` of one row with `open_count`, `queued_count`, `claimed_count`,
/// `working_count` and `blocked_count`: the
/// [`ChannelOccupancy`](maidan_types::ChannelOccupancy) partition of the open
/// threads in `scope`. `queued` is queue depth's `ready`, and every other
/// unheld thread is `blocked`.
pub(crate) fn occupancy_select(scope: QueueScope, sql: &QueueSql<'_>) -> String {
    format!(
        "SELECT {} {}",
        occupancy_columns(&sql.thread()),
        open_threads(scope, sql, false)
    )
}

/// The placeholders [`run_occupancy_select`] binds.
pub(crate) struct RunSql<'a> {
    /// Placeholder bound to the workspace id.
    pub workspace: &'a str,
    /// Placeholder bound to the normalized parent run id.
    pub parent_run_id: &'a str,
    /// Placeholder bound to the reader's member id. Required, as in
    /// [`QueueSql::reader`]: a count of a run's threads includes only those
    /// the reader may read.
    pub reader: &'a str,
    /// Placeholder bound to [`maidan_types::DM_CHANNEL_NAME`].
    pub dm_channel: &'a str,
    /// The rest, as [`ThreadSql`].
    pub thread: ThreadSql<'a>,
}

/// `SELECT` of the [`RunOccupancy`](maidan_types::RunOccupancy) counts of the
/// non-terminal threads homed under one parent run that the reader may read:
/// the same partition as [`occupancy_select`], so the two views of one thread
/// cannot disagree. A
/// thread that is not `open` (in review, say) is never handed out, so unheld
/// it counts `blocked`.
pub(crate) fn run_occupancy_select(sql: &RunSql<'_>) -> String {
    let RunSql {
        workspace,
        parent_run_id,
        reader,
        dm_channel,
        thread,
    } = sql;
    let readable = claim_next::readable_by(reader, dm_channel);
    format!(
        "SELECT {}
         FROM maidan_threads cand
         JOIN maidan_thread_lineage l ON l.thread_id = cand.id
         JOIN maidan_channels ch ON ch.id = cand.channel_id
         WHERE ch.workspace_id = {workspace}
           AND l.parent_run_id = {parent_run_id}
           AND cand.state NOT IN ('closed', 'archived')
           AND cand.tombstoned_at IS NULL
           AND {readable}",
        occupancy_columns(thread)
    )
}

/// The occupancy columns over threads aliased `cand`.
fn occupancy_columns(t: &ThreadSql<'_>) -> String {
    let held = claimability::held(t.now);
    let available = claimability::available(t.now);
    let withheld = claimability::withheld(t);
    let stalled = claimability::stalled(t.now);
    let queued =
        format!("({available} AND cand.state = 'open' AND NOT {withheld} AND NOT {stalled})");
    format!(
        "COUNT(*) AS open_count,
         COALESCE(SUM(CASE WHEN {held} AND cand.work_started_at IS NULL
                 THEN 1 ELSE 0 END), 0) AS claimed_count,
         COALESCE(SUM(CASE WHEN {held} AND cand.work_started_at IS NOT NULL
                 THEN 1 ELSE 0 END), 0) AS working_count,
         COALESCE(SUM(CASE WHEN {queued} THEN 1 ELSE 0 END), 0) AS queued_count,
         COALESCE(SUM(CASE WHEN {available} AND NOT {queued}
                 THEN 1 ELSE 0 END), 0) AS blocked_count"
    )
}

/// `FROM … WHERE` of the open, live threads in `scope` the reader may read and
/// the repo, engine and kind filters admit, and with `as_claimer` the reader
/// could be handed, aliased `cand` as [`claim_next::readable_by`] expects,
/// with their channel `ch`.
fn open_threads(scope: QueueScope, sql: &QueueSql<'_>, as_claimer: bool) -> String {
    let QueueSql {
        scope: scope_id,
        reader,
        dm_channel,
        repo,
        engine,
        kind,
        later_than,
        clock,
        ..
    } = sql;
    let in_scope = match scope {
        QueueScope::Channel(_) => format!("cand.channel_id = {scope_id}"),
        QueueScope::Workspace(_) => format!("ch.workspace_id = {scope_id}"),
    };
    let readable = claim_next::readable_by(reader, dm_channel);
    let served = claim_next::served_by(repo, engine, kind);
    let taken = if as_claimer {
        format!("AND {}", claim_next::takes(reader, clock, later_than))
    } else {
        String::new()
    };
    format!(
        "FROM maidan_threads cand
         JOIN maidan_channels ch ON ch.id = cand.channel_id
         WHERE {in_scope} AND cand.state = 'open' AND cand.tombstoned_at IS NULL
           AND ({readable})
           AND {served} {taken}"
    )
}
